---
# SPDX-License-Identifier: CC-BY-SA-4.0
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford
title: LIFX LAN Protocol — pointers and stub notes
author:
  - Elizabeth Ashford
date: 2026-07-10
updated: 2026-07-27
status: plain and linear phases implemented; A19/BR30 validated, Z/Beam untested
---

# LIFX LAN protocol

These are design and validation notes for `luminate-plugin-lifx`, not a copy of
the protocol specification. LIFX publishes the protocol; this file links to the
primary sources and records Luminate-specific decisions.

## Primary sources

- <https://lan.developer.lifx.com/docs> — official LAN protocol docs
- <https://github.com/LIFX/lifx-protocol-docs> — same docs, versioned in git
- <https://github.com/LIFX/public-protocol> — machine-readable protocol
  definition (message types/fields), useful if generating request/response
  structs instead of hand-writing them
- <https://lan.developer.lifx.com/docs/product-registry> — product ID →
  capability table (which products have colour, HEV, multizone, matrix, etc.)

## Protocol outline

- Transport is UDP to port 56700, not USB/HID — every bulb is discovered and
  addressed over the LAN, not enumerated as a local device.
- Discovery: broadcast `GetService` (packet 2, `tagged=1`), devices reply
  with `StateService` (packet 3). mDNS (`_lifx._udp`, firmware 4.110+) is
  the newer preferred alternative to broadcast discovery.
- Every message is a 36-byte header (Frame / Frame Address / Protocol
  Header, little-endian, `source`+`sequence` correlate requests to
  responses) followed by a type-specific payload.
- `res_required`/`ack_required` flags control whether a device replies with
  a `State*` message, an `Acknowledgement`, both, or neither.
- Relevant message pairs for a first-cut plugin: `GetVersion`/`StateVersion`
  (product id, for the capability table), `GetColor`/`LightState`,
  `SetColor`, `GetPower`/`StatePower`, `SetPower`, `GetLabel`/`StateLabel`.
  Multizone strips add `GetColorZones`/`StateMultiZone`; waveform effects
  (`SetWaveform`: SAW/SINE/HALF_SINE/TRIANGLE/PULSE) are the closest analog
  to this project's `hardware_effects` capability.

## Security boundary

The LAN protocol has no packet or device authentication. The plugin correlates
a response to the endpoint advertised during discovery and to the header's
source, sequence, target, and expected message type. Endpoint matching rejects
otherwise valid packets from an unrelated socket, but it cannot prevent source
address spoofing or a malicious discovery response from poisoning the cached
endpoint. LIFX devices should therefore be treated as trusted-LAN peers and
isolated at the network layer when that trust is inappropriate.

## Plugin ABI considerations

The earlier plugins were either synthetic or local USB/HID devices with
effectively synchronous callbacks. LIFX devices use asynchronous network I/O
and may appear or disappear while the daemon is running. This required dynamic
topology notifications rather than one-shot enumeration through `probe()`.

## Owned hardware and validation

- LIFX A19 (colour bulb) — on the LAN today, reachable and validated
  end-to-end: `GetService` broadcast discovery, `GetColor`/`LightState`,
  and `SetColor` with `ack_required` all round-tripped successfully
  against the real device (2026-07-12).
- LIFX BR30 (colour bulb, BR30 form factor) — live-tested successfully
  (2026-07-27). Uses the same plain-bulb path as the A19.
- LIFX Z (multizone light strip) — owned, not yet installed. Testable
  once it's up.
- LIFX Beam — owned, location/setup TBD. Testable once it's up.

The plain-bulb path is validated against the A19 and BR30. Z strip and Beam
support is implemented from the published spec and product registry without
live validation until they're installed.

## Staged scope

Device shapes, staged in this order:

1. **Plain bulb** (A19 and BR30; implemented and validated). One
   `DeviceDescriptor`, no child surfaces, and colour plus brightness
   capabilities. This mirrors the demo
   plugin's `controller_device()` pattern. Message set: `GetService`/
   `StateService` (discovery), `GetVersion`/`StateVersion` (product ID,
   for the capability-registry lookup), `GetColor`/`LightState`,
   `SetColor`, `GetPower`/`SetPower`, `GetLabel`.

2. **Linear multizone** (Z strip and Beam — implemented, untested). Both use
   LIFX's published linear multizone protocol; Beam is not a tile/device-chain
   product. A `Linear{length}` surface has one
   `Element` per zone with `ElementGeometry::Linear{position}` — same
   pattern already used for other addressable strip hardware in this
   workspace. Adds `GetColorZones`/`SetColorZones`/`StateMultiZone`.
   The zone count is read from `StateZone`/`StateMultiZone` on discovery and
   every later discovery refresh, so adding/removing extensions or Beam
   segments changes topology dynamically. The legacy-protocol phase limit is
   255 zones; counts outside `1..=255` are rejected while the last valid
   topology remains active. This is a strict superset of the bulb message set.

3. **Matrix/chain products** (Tile, Candle, Ceiling, etc. — not implemented or
   owned). These use `GetDeviceChain`/`StateDeviceChain`, `Set64`, and tile
   effects. They remain separate from the Z/Beam linear code path.

Waveform effects use `SetWaveform` with `SAW`, `SINE`, `HALF_SINE`, `TRIANGLE`,
or `PULSE`. They are independent of the product's physical shape.

## Effect mapping (`Effect` in `crates/luminate-core/src/effect.rs`)

`Effect` is the shared cross-hardware vocabulary: `Off`, `Static`, `Breathe`,
`Pulse`, `Strobe`, `Scanner`, `Morph`, `Spectrum`, and `Rainbow`. Each plugin
maps those requests to its hardware and advertises only the effects it
supports through `HardwareEffectsCapability`.

**Plain bulb (phase 1)** — only `Off`/`Static`/`Breathe`/`Pulse`/`Strobe` are
advertised:

- `Off` → `SetPower(false)` directly, not a waveform.
- `Static{colour}` → `SetColor`, then `SetPower(true)`.
- `Breathe{colour, period_ms}` → `SetWaveform` with `SINE`, transient,
  cycling indefinitely, then `SetPower(true)`.
- `Pulse{colour, period_ms}` → `SetWaveform` with `PULSE`, transient, then
  `SetPower(true)`.
- `Strobe{colour, period_ms}` → the same `SetWaveform` `PULSE` path as
  `Pulse`; the two share one wire mechanism and are distinguished only by
  the caller's chosen `period_ms` (short for a strobe, longer for a
  leisurely pulse).
- `Scanner`/`Morph`/`Spectrum`/`Rainbow` are **not advertised** on a
  plain bulb: `Scanner` and `Spectrum`/`Rainbow` imply spatial
  positioning a single bulb doesn't have; `Morph`'s arbitrary
  client-supplied colour list has no native single-shot LIFX primitive.
  `SetWaveform` interpolates from the current colour to one target colour per
  call. Chaining calls over UDP would add timing jitter and would not be atomic,
  so the plugin does not emulate this effect client-side.

**Linear multizone / Z strip and Beam (phase 2)** — adds `SetMultiZoneEffect`
`MOVE`, a firmware effect that shifts the current zone colours along the strip
at a configurable speed and direction. Once started, it runs on the device.
The initial zone colours determine which Luminate effect it represents:

- `Scanner{colour, period_ms}` → set a single lit zone surrounded by
  off, then `MOVE`.
- `Spectrum{period_ms}`/`Rainbow{period_ms}` → set a full hue gradient
  across zones with `SetColorZones`, then start `MOVE`.

Power is a separate LIFX state field, so every visible transition ends with
`SetPower(true)`. Before replacing a linear device's moving state with static
colour, brightness, zone data, clear/off, or another movement, the plugin first
sends `SetMultiZoneEffect` with effect type `OFF`. It then configures the new
state and restores power last; power-off stops `MOVE` before `SetPower(false)`.

**Matrix/chain products (future phase)** — would add `SetTileEffect`:

- `Morph{colours, period_ms}` → LIFX's own native `MORPH` tile effect
  takes a colour palette and smoothly blends across the tile array in
  firmware — a close conceptual match to `Effect::Morph`'s
  client-supplied colour list, and a real hardware-accelerated
  implementation instead of any client-side sequencing fallback.
- `FLAME` and `SKY` are LIFX-specific presets with no portable `Effect`
  equivalent. If exposed later, they should use the vendor-specific hardware
  effect path rather than new shared variants.

**HEV Clean cycle** (`SetHevCycle`) is a germicidal UV operation on LIFX Clean
bulbs, not a lighting effect. It has its own duration and remaining-time state
and is outside this plugin's current scope.

**`Strobe`** was added as a shared `Effect` variant (a fast blink rather than
`Breathe`'s slow fade), implemented on LIFX with a short-period `SetWaveform`
`PULSE` as above. The Alienware keyboard was checked for a comparable
fast-blink mode as a second backend before committing to the shared variant;
it has none (only `Breathe`, `Pulse`, Rainbow Wave, Sweeper, Spectrum, and
Morph), so `Strobe` is rejected there alongside the other portable animated
effects and remains LIFX/WLED-only for now. WLED maps it to its native
firmware `Strobe` effect.

## Plugin ABI implementation

`init()` starts a background `GetService` discovery loop and caches devices in
a `static Mutex<HashMap<...>>`. The cache expires devices that stop responding.
`topology_cbor()` returns a snapshot of that cache, and
`apply_update_cbor()` performs a blocking acknowledged UDP round trip to the
target's cached address. Plugin FFI calls run on a dedicated host thread rather
than the async reactor, so these blocking requests follow the same model as the
Alienware HID calls.

**Dynamic startup topology (resolved 2026-07-12):** dynamic topology support is
implemented as a general plugin feature rather than a LIFX-specific workaround.
The discovery thread calls `notification::topology_changed()` when its stable
device set changes; the daemon debounces, re-pulls `topology_cbor()`, updates
ownership/state, and publishes a client event. See `docs/development/architecture/protocol.md`'s
"Event subscription" section. The prerequisite is complete.

## Status

Plain-bulb and linear-multizone support is implemented in
`plugins/luminate-plugin-lifx` and included in the workspace and packaging.
The plain path was live-validated against the owned A19 on 2026-07-12:

- the plugin loaded with an empty startup topology, discovered product 93
  (`LIFX Color A19 1100lm`), and caused the daemon to reconcile the new
  `lifx-d073d57207c8` device without restart;
- the published device name combines `StateLabel` with the stable LIFX serial
  (`Living Room Lamp (d073d57207c8)`), so ordinary duplicate user labels remain
  globally unambiguous;
- two daemon/CLI brightness mutations each performed `GetColor` readback and
  an acknowledged `SetColor`, with the second restoring full brightness;
- packet encoding, discovery parsing, RGB → HSBK conversion, product filtering,
  and topology capabilities have focused unit tests.

Known BR30 products use this same plain path, and the owned BR30 was
live-tested successfully on 2026-07-27. Known Z and Beam products publish a
device-reported, dynamically refreshed `Linear` zone surface and support
per-zone writes plus firmware MOVE effects. Their packet formats, count bounds,
and topology have focused tests, but no physical Z or Beam has been exercised
yet. The topology itself carries this warning.

Matrix/chain, switch, and unknown product IDs remain deliberately absent until
their distinct topology shapes are implemented.
