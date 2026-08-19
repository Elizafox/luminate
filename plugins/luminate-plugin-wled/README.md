<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# WLED plugin

Controls WLED controllers through the local JSON HTTP API. Controllers are
discovered through `_wled._tcp.local` mDNS and may also be listed explicitly in
the plugin's daemon configuration:

```toml
[[plugins]]
name = "luminate-plugin-wled"
[plugins.config]
mdns = true
endpoints = ["192.0.2.10", "wled-office.local:8080"]
physical_tags = ["shape:planar"]
```

`endpoints` is the only explicit-discovery source. Each entry must be an HTTP
host or `host:port` without a path. At most 256 entries may be configured. Set
`endpoints = []` to use mDNS discovery only, or also set `mdns = false` to
disable all discovery.

`physical_tags` is an optional open list of presentation hints applied to each
WLED `segments` surface. WLED does not generally identify how a controller is
mounted, so configure tags such as `shape:cylinder` or
`layout:wrapped-horizontal` when they describe the installation. Unknown tags
are preserved for clients and ignored by consumers that do not recognise them.
They intentionally remain on the surface rather than the device: the tags
describe the user-configured LED installation, not the WLED controller.

Implemented operations:

- controller and segment power, 0–255 brightness, and RGB colour
- firmware-defined segments as elements on a linear `segments` surface
- WLED's complete reported firmware-effect list through the `wled-effect`
  choice, plus portable mappings for Breathe, Pulse, Scanner, Spectrum, and
  Rainbow when the installed firmware reports a matching effect
- bulk JSON state readback for appearance, brightness, emission, and
  controller physical power
- dynamic topology updates when controllers, segments, names, or firmware
  effects change

The plugin recommends `Adopt` reconciliation because WLED controllers are
commonly shared with the web UI, automations, and other local clients.
Appearance readback is best-effort: WLED reports the active effect and primary
colour, but that is not a complete reconstruction of palette, intensity, and
every effect-specific behaviour.

Realtime pixel protocols are deliberately not implemented. They require a
shared Luminate streaming contract for frame shape, timing, backpressure,
ownership, disconnect cleanup, and reconciliation before a WLED-specific UDP
transport can be added safely.

For local use:

```sh
cargo build -p luminated -p luminate-cli -p luminate-plugin-wled
LUMINATED_CONFIG=docs/development/config/wled-plugin.local.toml \
  cargo run -p luminated
cargo run -p luminate-cli -- \
  --socket-path /tmp/luminated-wled/luminated.sock list
```

The WLED JSON API is plain HTTP and usually unauthenticated. Use it only on a
trusted LAN or behind suitable network isolation. This implementation is
covered with protocol, topology, mutation-payload, and state-decoding tests but
still requires validation against physical WLED hardware.
