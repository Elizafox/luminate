<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Demo bulb plugin

`luminate-plugin-demo-bulb` is a virtual smart-bulb plugin. It models a single
light whose defining trait is its storage policy: every write goes straight to
hardware and is immediately durable, and that stored state cannot be read back.

The fictional hardware is the `Efreet Ember` from `Djinn Foundry`, a bottled
flame that remembers every wish the moment it is granted.

It models:

- one device: `demo-smart-bulb`
- no surfaces, elements, or groups: a single addressable RGB endpoint
- `PersistenceRequirement::Required` write-through persistence with no readback
- virtual shadow state for mutations
- batched update handling
- one-pixel, full-frame uploads through the ordinary CBOR path

The plugin accepts every probe. It does not touch hardware.

## Why this plugin exists

The demo system plugin shows many device and capability shapes. This plugin is
intentionally narrower: it isolates one persistence model so its effect on the
daemon is easy to observe. It previously lived inside the demo-system plugin and
was split out into its own crate alongside the demo keyboard.

## Capabilities and behaviour

`bulb_capabilities()` advertises RGB colour, device-scope brightness, and:

- `persistence: CurrentState { requirement: Required, explicit_commit: false,
  readback: false }`: every mutation is durable the instant it is written, and
  there is no separate save step.
- `state_readback: None`: the stored state cannot be queried.
- reconciliation recommendation: `Leave`, because neither an exact adoption
  read nor an unnecessary restore write is available.

Two consequences follow, and both are visible from the daemon:

- `SaveCurrent` is accepted as a durable no-op. Most demo devices reject it;
  the bulb instead logs that write-through persistence is already durable and
  returns success.
- Startup writes are skipped. The `Leave` recommendation preserves hardware,
  and `Required` persistence also makes cached replay redundant. See
  [the demo-plugin guide](../../docs/development/plugins/demo-plugin.md) for the
  exact startup-sync log lines.

## Update handling

After the plugin is loaded, probed, and its topology accepted, the daemon uses
three callbacks for `demo-smart-bulb`:

- the generated update callback decodes and applies one CBOR `PluginUpdate`
- the generated batch callback decodes an ordered CBOR `PluginUpdateBatch` and writes
  one structured result for each entry
- the generated frame-upload callback decodes a CBOR `FrameEnvelope` and passes
  its full-frame payload to `FrameStreamingPlugin::upload_frame`

Both mutation paths call `apply_update`, so their validation and results stay
consistent.
`apply_update` rejects any non-device target (the bulb has no surfaces, elements,
or groups), accepts `SaveCurrent`, logs the operation, and records a string in an
in-memory map that proves callback execution but is neither daemon state nor
durable device storage.

The frame path treats the bulb as a one-pixel output. A full frame containing
one colour is equivalent to a static-colour update; partial frames and any other
pixel count are rejected. This deliberately small example shows the ordinary
frame-upload contract without introducing a framebuffer or shared memory.

## Generated ABI boundary

`luminate_export_plugin!` exports the ABI version and descriptor symbols the
daemon discovers in the `cdylib`. The SDK adapter owns topology CBOR storage,
input decoding, ordered result buffers, frame buffers, and panic containment;
this plugin implements only typed Rust methods.

See the
[`plugin-authoring guide`](../../docs/development/plugin-authoring.md) for the
author-facing contract.
