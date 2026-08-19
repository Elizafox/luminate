<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Philips Hue plugin

Controls lights through the local Philips Hue Bridge API v2. The plugin uses
authenticated HTTPS, binds the configured bridge ID to the bridge certificate,
and does not use Hue cloud services or a Hue account.

The initial implementation supports:

- one configured square Hue Bridge with API v2 support;
- mDNS discovery or an explicit bridge endpoint;
- authenticated enumeration and dynamic topology updates;
- on/off, 0–100 brightness, RGB colour, and colour temperature when advertised
  by each light;
- bounded live readback for appearance, brightness, and emission; and
- an `Adopt` reconciliation recommendation for lights shared with Hue
  applications, switches, and automations.

Appearance and brightness readback are best-effort. Hue reports CIE xy and
fractional brightness, while Luminate represents RGB and integer brightness.
Emission readback is exact. Rooms, zones, scenes, gradients, firmware effects,
Entertainment API streaming, sensors, and multiple simultaneously configured
bridges are not implemented.

## Bridge setup

The plugin advertises the `push-link` setup workflow through libluminate. It
discovers local bridges, asks the user to select one when necessary, requests
the physical link-button action, creates an application key over
certificate-bound HTTPS, and verifies the key before luminated commits it. The
generated key is not returned to the setup client.

List the available workflow and then run push-link setup interactively:

```sh
luminatectl plugin setup luminate-plugin-philips-hue
luminatectl plugin setup luminate-plugin-philips-hue push-link
```

The command guides bridge selection and waits for confirmation after you press
the bridge's link button. Add `--json` for JSON workflow discovery and a
JSON-lines session interface suitable for another frontend.

Manual configuration remains available. Create a local application key using
Philips Hue's documented push-link procedure and keep it private: it grants
local control through the bridge. Record the bridge's canonical 16-digit ID,
normally printed on the bridge or returned by its discovery API.

Configure an explicit plugin entry:

```toml
[[plugins]]
name = "luminate-plugin-philips-hue"
required = true

[plugins.config]
bridge_id = "001788fffe123456"
application_key = "replace-with-the-local-application-key"
mdns = true
```

`bridge_id` and `application_key` are required. The application key is marked
sensitive and redacted from diagnostics. With mDNS enabled, Luminate discovers
`_hue._tcp.local` and accepts only the configured bridge identity.

When multicast discovery is unavailable, disable it and provide a hostname or
IP address. The endpoint controls routing only; it does not weaken certificate
identity checks.

```toml
[plugins.config]
bridge_id = "001788fffe123456"
application_key = "replace-with-the-local-application-key"
mdns = false
endpoint = "192.0.2.20:443"
```

## Local development

Copy the supplied example and replace its bridge ID and application key before
running it:

```sh
cargo build -p luminated -p luminate-cli -p luminate-plugin-philips-hue
cp docs/development/config/hue-plugin.local.toml /tmp/hue-plugin.local.toml
$EDITOR /tmp/hue-plugin.local.toml
LUMINATED_CONFIG=/tmp/hue-plugin.local.toml cargo run -p luminated
cargo run -p luminate-cli -- \
  --socket-path /tmp/luminated-hue/luminated.sock list
```

The example contains a placeholder key only. Do not commit real application
keys, bridge resource snapshots, or household metadata.

The implementation follows the official Hue API and colour-conversion
guidance and has deterministic protocol tests. Physical validation covers
mDNS discovery and push-link setup on a BSB002 bridge, enumeration of LCA007
and LCD006 lights, and control/readback on an LCA007 colour lamp. Broader
product and network coverage remains experimental.
