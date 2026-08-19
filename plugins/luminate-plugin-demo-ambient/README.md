<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Demo ambient panel plugin

`luminate-plugin-demo-ambient` is a virtual ambient-light plugin. It models a
single glow panel whose defining trait is trust: the hardware reports its current
state reliably, and that live reading is the source of truth after a restart.

The fictional hardware is the `Nimue Glowpane` from `Avalon Lightworks`, a
lake-mirror that always shows the world as it truly is rather than as it was last
remembered.

It models:

- one device: `demo-ambient-panel`
- one surface: `panel`
- exact appearance/brightness/emission readback on the panel surface
- an `Adopt` reconciliation recommendation for shared hardware
- virtual shadow state for mutations
- batched update handling

The plugin accepts every probe. It does not touch hardware.

## Why this plugin exists

The demo system plugin shows many device and capability shapes. This plugin is
intentionally narrower: it isolates one readback model so its effect on the
daemon is easy to observe. It previously lived inside the demo-system plugin and
was split out into its own crate alongside the demo keyboard.

## Capabilities and behaviour

The device deliberately splits its capabilities across two scopes:

- `ambient_device_capabilities()` advertises appearance, brightness, and
  emission as `BestEffort` readback.
- `panel_surface_capabilities()` advertises the same facets as `Exact`, plus
  optional readable persistence. This is the trustworthy physical scope.

The plugin recommends `Adopt`, so absent a user override the daemon reads the
panel after startup or reappearance and durably rebases confirmed facets below
explicit desired overlays. Authority is policy, not capability: configuring
`Restore` or `Leave` changes the direction without changing the readback facts.
`SaveCurrent` is rejected because persistence has no explicit commit.

## Update handling

After the plugin is loaded, probed, and its topology accepted, the daemon uses
three callbacks for `demo-ambient-panel`:

- the generated update callback decodes and applies one CBOR `PluginUpdate`
- the generated batch callback decodes an ordered CBOR `PluginUpdateBatch` and writes
  one structured result for each entry
- the generated read-state callback returns a bounded CBOR `PluginStateSnapshot`
  from the virtual hardware shadow

Both mutation paths call `apply_update`, so their validation and results stay
consistent.
`apply_update` validates that the target is the device or its `panel` surface,
rejects unsupported `SaveCurrent`, logs the operation, and updates an in-memory
hardware shadow. The shadow is observable through the read callback but is not
daemon desired state or durable device storage.

## Generated ABI boundary

`luminate_export_plugin!` exports the ABI version and descriptor symbols the
daemon discovers in the `cdylib`. The SDK adapter owns topology CBOR storage,
input decoding, mutation and snapshot buffers, and panic containment; this
plugin implements only typed Rust methods.

See the
[`plugin-authoring guide`](../../docs/development/plugin-authoring.md) for the
author-facing contract.
