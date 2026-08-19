<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# LIFX LAN plugin

Supports plain LIFX colour bulbs and linear-multizone LIFX Z strips and Beams
over the LAN. The plugin loads with no light online, discovers devices in a
background thread, and notifies the daemon when the stable device topology
changes.

Supported operations:

- device-scoped RGB colour and independent brightness
- `Off` and `Static`
- firmware `Breathe` (`SINE`) and `Pulse` (`PULSE`) waveforms
- a dynamic `zones` surface for Z/Beam devices, with 1 to 255 device-reported
  elements and per-zone RGB, brightness, static, and off control
- device-level physical tags curated from the product registry: supported Z
  products report `shape:flexible-strip`, Beam products report
  `shape:modular-light-bar`, and known bulb families report their established
  A19, BR30, GU10, PAR38, downlight, or candle form
- firmware `MOVE` effects for linear-device `Scanner`, `Spectrum`, and
  `Rainbow`
- acknowledged UDP writes with response correlation and retries
- bounded live snapshots of rendered colour, brightness, emission, and
  device-level physical power using `GetColor`/`LightState`
- an `Adopt` reconciliation recommendation, suitable for bulbs shared with
  wall switches, vendor applications, and other controllers

Rendered colour readback is `BestEffort` because LIFX does not reconstruct the
original active waveform/effect parameters. Brightness, emission, and physical
power are exact and therefore eligible for durable adoption. Zones advertise
local emission but not independent physical power; their writes name the
broader device power domain explicitly.

The embedded product table contains plain-colour and known linear-multizone
products from the official
[`LIFX/products`](https://github.com/LIFX/products) registry captured on
2026-07-12. Matrix, chain, relay, button, and unknown products are deliberately
excluded until their topology shapes are implemented.

The owned A19 and BR30 have been live-validated. Z and Beam support is
implemented from the published protocol and covered by packet/topology tests,
but remains explicitly untested on physical hardware until those devices are
installed. The linear `zones` surface does not repeat Z or Beam device tags: it
describes addressable topology, while the tag describes the complete fixture.
Zone counts are re-read during discovery; changes cause a topology update,
subject to the current legacy multizone limit of 255 zones.

For local use:

```bash
cargo build -p luminated -p luminate-cli -p luminate-plugin-lifx
LUMINATED_CONFIG=docs/development/config/lifx-plugin.local.toml cargo run -p luminated
cargo run -p luminate-cli -- --socket-path /tmp/luminated-lifx/luminated.sock list
```

The daemon account needs permission to broadcast UDP and reach bulbs on their
advertised LAN service ports. No cloud connection or LIFX account is used.
Discovery defaults to `255.255.255.255:56700`. A plugin entry may set
`config.discovery_address` to an IPv4 socket address when discovery must be
directed through a specific endpoint, for example in an isolated test network:

```toml
[[plugins]]
name = "luminate-plugin-lifx"

[plugins.config]
discovery_address = "127.0.0.1:56700"
```

The LIFX LAN protocol does not authenticate devices or packets. Replies are
correlated to the advertised UDP endpoint as well as their source, sequence,
target, and message type, which rejects matching packets from unrelated
endpoints. This does not protect against source-address spoofing or a poisoned
discovery response; operate the plugin on a trusted LAN or enforce network
isolation appropriate for the devices.
