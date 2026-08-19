<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Demo keyboard plugin

`luminate-plugin-demo-keyboard` is a virtual keyboard plugin emulating a simple
keyboard.

It models:

- one keyboard device: `demo-keyboard-only` (distinct from the demo-system
  plugin's own `demo-keyboard`, so both can load side by side)
- one surface: `keys`
- per-key RGB elements for a small 75% style layout sample
- topology groups for `all`, `wasd`, `arrows`, and `function-row`
- virtual shadow state for mutations
- batched update handling
- a `Restore` reconciliation recommendation for daemon-owned volatile lighting

The plugin accepts every probe. It does not touch hardware.

## Why this plugin exists

The demo system plugin shows many device and capability shapes. This plugin is
intentionally narrower: it keeps the complete keyboard layout and update path
small enough to understand in one file. It is useful for testing topology
renderers, per-key targeting, named key groups, and batch dispatch without
requiring supported hardware.

## From layout data to targets

The `KEYS` table in `src/lib.rs` is the source of truth for the exposed keys.
Each `KeySpec` contains:

- a stable `id` used in protocol targets, such as `escape`, `w`, or `space`
- a display `name` suitable for labels
- normalized `x` and `y` coordinates used to build `ElementGeometry::Rect`

Coordinates and rectangle sizes are relative to the keyboard surface, not
pixels or physical units. Geometry tells a UI where to draw a key; it does not
identify or address that key. Array order remains useful as authored reading
order, while the stable ID is the contract used by updates and group members.

The resulting target hierarchy is:

```text
demo-keyboard-only              device
demo-keyboard-only/keys         surface
demo-keyboard-only/keys/w       element
demo-keyboard-only/group:wasd   group
```

Groups reference existing surface or element IDs. They do not copy elements or
create a second state store. The daemon resolves and validates those references
when loading the topology.

## Capabilities and scope

The helper `keyboard_capabilities(scope)` advertises RGB colour, brightness
from 0 through 100, and `static`/`rainbow` hardware effects. It is reused with a
different `CapabilityScope` at each addressable level:

- device scope controls the whole keyboard
- surface scope controls the `keys` surface
- element scope controls one key
- device scope on a group controls the selected cluster within this device

Capability metadata is enforced by the daemon before an update reaches the
plugin. It must still match what the plugin and hardware can actually perform.
This demo declares no persistence, state read-back, or frame upload support, so
`SaveCurrent` is rejected and its shadow state is neither durable nor readable
through the daemon.

## Update handling

After the plugin is loaded, probed, and its topology accepted, the daemon routes
mutations for `demo-keyboard-only` to one of two callbacks:

- the generated update callback decodes and applies one CBOR `PluginUpdate`
- the generated batch callback decodes an ordered CBOR `PluginUpdateBatch` and
  writes one structured result for each entry

Both paths call `apply_update`, so their validation and results stay consistent.
The demo batch callback processes entries individually because there is no real
transport. A hardware keyboard would commonly use the batch boundary to update
a shadow framebuffer and send one USB/HID transaction rather than one write per
key. Batch delivery does not imply that every entry must succeed.

`apply_update` validates that the target belongs to this plugin, checks surface
and key IDs, rejects unsupported persistence, logs the operation, and records a
string in an in-memory map. That map is only proof of callback execution:

- it stores the last operation for the exact addressed target
- group and surface updates are not expanded into individual keys
- it is lost when the daemon exits
- it does not implement hardware state readback

## Generated ABI boundary

`luminate_export_plugin!` exports the ABI version and descriptor symbols the
daemon discovers in the `cdylib`. The SDK adapter owns topology CBOR storage,
input decoding, native-batch result buffers, and panic containment; this plugin
implements only typed Rust methods.

See the broader [demo system plugin](../luminate-plugin-demo-system/README.md)
for the complete load lifecycle and [`docs/development/plugin-authoring.md`](../../docs/development/plugin-authoring.md)
for the author-facing plugin contract.
