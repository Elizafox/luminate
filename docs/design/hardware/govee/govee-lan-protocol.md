---
# SPDX-License-Identifier: CC-BY-SA-4.0
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford
title: Govee LAN API — research notes
author:
  - Elizabeth Ashford
date: 2026-07-12
updated: 2026-07-27
status: >-
  Whole-device control, firmware scenes, and H6022 matrix frame streaming
  have shipped. Whole-device control, scenes, and static matrix uploads are
  verified on the owned H6022.
---

# Govee LAN API

These notes originally scoped a possible `luminate-plugin-govee`; the crate
now exists at `plugins/luminate-plugin-govee`. They collect the details
needed to understand the plugin's design rather than attempting to specify
the full protocol. Govee does not publish a versioned LAN protocol reference,
so the information comes from third-party documentation, packet captures from
one H6022, and — for segment colour and firmware scenes specifically — the
user's own hardware-verified reverse-engineering repo, `govee-encrypted-ble`
(see its `docs/protocol-extensions.md` for the full raw-frame command set).

## Sources

- <https://github.com/wez/govee2mqtt/blob/main/docs/LAN.md> documents ports and
  multicast behaviour, but not message formats.
- <https://github.com/wez/govee-lan-hass> is a Home Assistant integration for
  the same protocol.
- <https://gist.github.com/mtwilliams5/08ae4782063b57a9b430069044f443f6>
  includes literal JSON message examples.
- <https://app-h5.govee.com/user-manual/wlan-guide> is Govee's LAN Control
  guide. It is client-rendered and was read in the app.
- The user's own `govee-encrypted-ble` repo (BLE encryption and protocol
  research, hardware-verified against the owned H6022) documents the raw
  20-byte command frames that the plugin's `ptreal` module builds and sends
  over the `ptReal` passthrough — see "Segments and scenes via `ptReal`"
  below.

There is no official schema or product registry, and behaviour may vary by
firmware.

## Protocol outline

The API sends JSON over UDP and uses three fixed ports:

- Send discovery to multicast address `239.255.255.250:4001`.
- Send commands to port 4003 at the device's IP address.
- Listen for every reply on local port 4002.

Replies do not return to the sending socket's ephemeral port. In a live test on
2026-07-12, an H6022 ignored a `devStatus` request until a separate listener
was bound to port 4002.

### Discovery

Send:

```json
{"msg":{"cmd":"scan","data":{"account_topic":"reserve"}}}
```

Devices reply on port 4002 with these fields:

```json
{"msg":{"cmd":"scan","data":{"ip","device","sku","bleVersionHard","bleVersionSoft","wifiVersionHard","wifiVersionSoft"}}}
```

Discovery does not report whether LAN Control is enabled. That setting is a
per-device toggle in the Govee Home app. The only practical check is whether
the device answers `scan`.

### State and control

Send `devStatus` to port 4003:

```json
{"msg":{"cmd":"devStatus","data":{}}}
```

The reply contains:

```json
{"msg":{"cmd":"devStatus","data":{"onOff","brightness","color":{"r","g","b"},"colorTemInKelvin"}}}
```

Known control commands are:

- `turn` with `{"value":0|1}`;
- `brightness` with `{"value":0-100}`; and
- `colorwc` with `{"color":{"r","g","b"},"colorTemInKelvin"}`.

Discovery, `devStatus`, and `colorwc` have been tested on the owned H6022.
Plain `turn` and `brightness` writes still need testing on that unit.
The H6022's native CCT range is 2700-6500 K. The plugin sends an RGB
approximation for requested temperatures outside that range, so clients can
still request warmer or cooler appearances without relying on unsupported
firmware values.

Unknown SKUs advertise RGB colour and brightness, but not native CCT. The
presence of `colorTemInKelvin` in the shared wire format only means an
unsupported device can ignore the field safely; it does not prove that the
requested temperature was applied. Luminate therefore uses RGB CCT emulation
until a model-specific native range has been verified.

## Scene and pattern limits

The LAN API does not expose the active scene. During a live test, the H6022 was
visibly running its built-in Aurora pattern, but `devStatus` returned:

```text
onOff: 1
brightness: 100
color: { r: 0, g: 0, b: 0 }
colorTemInKelvin: 2700
```

This describes the lamp's plain colour register, not the visible pattern.
None of the documented LAN commands refers to scenes.

Govee's cloud Developer API has a `dynamic_scene` capability with
`lightScene` and `diyScene` instances. Its documentation says an empty value
means the instance cannot be queried, so the cloud API also treats scene state
as write-only. Neither supported API can answer which pattern is running.

The H6022 appears to use two kinds of pattern:

- a small set of firmware patterns; and
- app-driven patterns, where the app likely generates a colour sequence and
  sends updates over BLE or the cloud.

The second kind may have no persistent scene ID on the lamp. Packet captures
of the app are still needed to confirm how both kinds are driven. In
particular, it is not yet known whether firmware patterns are readable or
whether any pattern can be selected through this JSON API.

## Client-driven effects

Rapid `colorwc` writes work well enough for simple client-timed effects. A
manual hue-cycle test ran smoothly at both 20 updates per second (50 ms
intervals) and 50 updates per second (20 ms intervals), with no visible lag or
stutter.

This was an ad hoc test rather than a daemon load test, but it supports whole-
device effects similar to Luminate's `Breathe` and `Pulse`. The client must
schedule each colour change because this API has no equivalent to LIFX
`SetWaveform`.

The app's fire and rainbow effects are more sophisticated. Other Govee
reverse-engineering work, including `wez/govee2mqtt` issue 105 and
`wez/govee-lan-hass` issue 42, describes a separate binary protocol with
roughly 20-byte packets, a `0x33` header, and per-segment bitmasks. It travels
over Govee's cloud/MQTT channel and over BLE — **and, as it turns out, over
this same LAN JSON API too**: see the next section. The note below this
paragraph, from the original research pass, was wrong on that specific point
and is kept for the record rather than silently deleted:

> No one has found a way to use that protocol through the JSON UDP API.

## Scenes and matrix effects via `ptReal`

The above turned out to be solvable: Govee's LAN API has an undocumented
`ptReal` command that carries the exact same raw 20-byte frames as BLE,
Base64-encoded inside JSON, with no encryption at all:

```json
{"msg":{"cmd":"ptReal","data":{"command":["<base64 frame>", "<base64 frame>", …]}}}
```

`plugins/luminate-plugin-govee/src/ptreal.rs` builds the scene frames. The
confirmed scene sub-mode (under `33 05 <sub_mode>`, hardware-verified against
the H6022 per `govee-encrypted-ble/docs/protocol-extensions.md`) is:

- `0x04` — firmware scene select: `<code> 00 01`. Twelve scene codes are
  confirmed by name: Reading, Night Light, Fire, Snow flake, Ocean, Forest,
  Joyful, Wave, Starry Sky, White Light, Rainbow Drawing, and Rainbow Striped.
  The two Rainbow variants were verified on the owned H6022: `0x16` produces
  the drawn-looking animation and `0x2a` produces the striped animation.

The plugin also currently sends `33 05 0b <R> <G> <B> <maskLow> <maskHigh>`
for a 15-element `"segments"` surface. Live testing on 2026-07-27 disproved
that model for the H6022: the command did not select static bands and instead
left a stuttering purple, black, and rainbow pattern resembling previously
uploaded DIY state. The 15-segment command was derived from
`dreamcolorlightv1`, a different Govee product family, and had not actually
been exercised by the research client's command-line path. The H6022 segment
surface must therefore be removed rather than repaired by guessing at other
bytes.

Scene *readback* is still unsupported — nothing above changes that; see
"Scene and pattern limits" above, which remains accurate.

## H6022 DIY/graffiti matrix

The H6022 is a cylindrical 12-column by 11-row RGBIC matrix, for 132 cells.
Cell indices are row-major (`index = row * 12 + column`): columns wrap around
the cylinder and rows run from top to bottom. Exact captured four-corner
frames and newly encoded moving-ring frames were both reproduced over LAN on
2026-07-27.

Matrix data uses an `0xa3` multipart upload followed by the
`33 05 0a 20 03 5a` commit frame. Each multipart frame carries 17 payload
bytes and retains the ordinary XOR checksum. The payload describes a
background colour, colour-to-cell-index groups, and layer behaviour. See the
research repository's `docs/protocol-extensions.md` for the byte-level
layout.

The layer's speed byte is also its static-image control:

- speed `0` holds the uploaded matrix image steady;
- nonzero speed animates it according to the action byte; and
- duration `0` does not disable animation.

These semantics were verified directly on the owned H6022. A four-corner
image using action `down`, speed `0`, and duration `0xffff` remained static.
The same image with speed `100` cascaded down. Action `0` with nonzero speed
flashed every coloured cell.

## Plugin scope

`luminate-plugin-govee` is implemented in two phases:

- **Phase 0** (shipped): whole-device power, brightness, static colour, and
  colour temperature, over the documented `turn`/`brightness`/`colorwc` JSON
  commands. `SaveCurrent` is a no-op (Govee LAN lights persist state in
  firmware already); `Clear` turns the device off.
- **Phase 1** (shipped): firmware scene selection and the H6022's
  hardware-verified 12×11 static matrix upload work over `ptReal`. The matrix
  uses a shadow framebuffer because the LAN API cannot read cells back. BLE,
  firmware-driven animated matrix effects, and music mode remain out of scope.
- **Phase 2** (shipped in software): the H6022 matrix surface accepts
  client-timed effects through Luminate's frame-streaming API. Each accepted
  frame is a complete 132-pixel, row-major RGB image encoded through the same
  DIY upload path as static cell updates. The plugin advertises a conservative
  10 Hz ceiling; sustained streaming at that rate still needs hardware
  verification. The surface also exposes the device's RGB, colour-temperature,
  brightness, emission, persistence, and firmware-scene controls because it is
  the H6022's sole light-emitting surface. Individual cells remain RGB-only:
  the protocol does not provide per-cell CCT, brightness, power, or scenes.

Frame uploads are full-frame only and target the `"matrix"` surface rather
than the whole device. Each upload is atomic from Luminate's perspective:
the complete DIY image and its commit command travel in one `ptReal` datagram.
The ordinary UDP transport retransmits that datagram once after 50 ms, and
Govee provides no acknowledgement that either copy was applied. A successful
frame therefore means “submitted”, just like other plugin writes.

Readback still needs the same caveat as before: `devStatus` does not reflect
an app-driven pattern, and the plugin does not implement `ReadablePlugin`.
The daemon must not treat any LAN reply as an exact account of visible
output. This follows the unknown-state rules in
`docs/development/architecture/protocol.md`.

## Owned hardware

The owned Govee H6022 table lamp has LAN Control enabled and was reachable at
`10.0.0.218` during testing. The address is assigned by DHCP and may change.
Discovery, status reads, and rapid `colorwc` writes were validated on
2026-07-12. On 2026-07-27, the plugin's end-to-end path was validated for
power, brightness, RGB, CCT, and all 12 named firmware scenes, including both
Rainbow variants. The invalid 15-segment walking-colour test failed and led
to the hardware mockups that established the matrix protocol. On 2026-07-27,
an isolated daemon automatically discovered the lamp and successfully
submitted four-corner cell updates followed by a whole-device colour and
single-cell invalidation check. Visual confirmation remains because the
protocol provides no acknowledgement or matrix readback.

## Status and next steps

All three phases described above have shipped in
`plugins/luminate-plugin-govee`. Whole-device control, named scenes, static
matrix mockups, and animated matrix research uploads have been verified
against the owned H6022. The plugin's static matrix path has been exercised
end-to-end through UDP submission; its physical result still needs visual
confirmation. Sustained client-timed frame streaming also needs physical
verification. BLE, firmware-driven animated matrix effects, and music mode
remain future work, not currently scheduled.
