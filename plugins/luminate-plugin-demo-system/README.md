<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Demo system plugin

This plugin is the reference smoke-test plugin for `luminated`.

## Why this exists

This plugin does three jobs:

- Proves the loadable plugin path works end to end
- Provides a readable example for authors writing their own plugins
- Stress-tests the topology and update paths with a deliberately varied fake system

## What it models

The demo system exposes a fake controller plus several fake devices:

- Keyboard
- Mouse
- Power button
- CPU cooler fan
- Two RAM sticks
- Power supply
- Case lights
- A firmware-effect-only status LED
- An RGBW strip
- A tunable-white fan
- An individually addressable strip
- A streaming keypad
- A gaming mousepad
- A monitor

The topology intentionally mixes different lighting styles:

- RGB zone lighting
- Individual key-style elements (`G1`, `G2`, `G3`, `Turbo`)
- Linear strip/ring surfaces
- A fixed-colour style power button
- A monochrome two-level logo light
- A grid-addressed keypad (`SurfaceKind::Matrix`/`ElementGeometry::MatrixCell`) with HSV colour
  and a real parameterized hardware effect
- Scattered, non-grid perimeter LEDs (`SurfaceKind::Sparse2d`/`ElementGeometry::Point`) with HSL
  colour
- A multi-surface device (monitor backlight + logo badge) with a five-channel amber/UV backlight

This is deliberate. The plugin is meant to exercise different capability shapes,
target forms, and group references rather than present one uniform RGB-only model.

## Lifecycle: shared object to hardware update

The important boundary is between the daemon and the plugin `cdylib`. They are
compiled separately and communicate through the C-compatible types in
`luminate-plugin-api`; ordinary Rust values must not cross that boundary.

At startup, the daemon:

1. Opens the plugin `.so` and finds the ABI-version and descriptor symbols
   emitted by `luminate_export_plugin!`.
2. Rejects the plugin unless its ABI version exactly matches the daemon. The ABI
   is intentionally version-locked, so plugins should be rebuilt with each
   Luminate release.
3. Calls the macro-generated initialization function. It installs logging,
   configuration, and topology-notification bridges.
4. Calls `DemoSystem::probe`. A real plugin reports `Unsupported`, `Dormant`, or
   `Ready`; this synthetic plugin is always ready.
5. Calls the post-acceptance `start` hook, when present.
6. Calls the generated topology adapter and decodes its returned
   `Vec<DeviceDescriptor>`. The daemon validates IDs, references, and
   capabilities before merging accepted devices into its topology. When
   plugins claim the same logical ID or an exclusive physical hardware claim,
   descriptor priority determines ownership.
7. Routes later mutations through the generated adapter to
   `DemoSystem::apply`; the adapter preserves ordered batch results.
8. Uses the plugin's `Restore` recommendation when choosing startup and
   reappearance reconciliation, unless user configuration overrides it.

The daemon validates a request against the declared target and capabilities
before calling the plugin. The plugin remains responsible for device-specific
validation and transport errors. Mutation callbacks return structured status
and a bounded diagnostic; their outer ABI byte only says whether the result
envelope was written.

## Reading the source

`src/lib.rs` is organized in the same order as the concepts above:

- `NAME`, `VERSION`, `BUSES`, `VENDORS`, and `HINTS` are static descriptor
  metadata retained by the daemon for the lifetime of the loaded library.
- `DemoSystem` implements the typed construction, probe, topology, and update
  contract.
- `demo_topology` and the `*_device` functions author the device tree.
- the `*_capabilities` helpers show how support varies by target scope, colour
  encoding, persistence policy, state readback, and hardware effects.
- `log_update` is where a real plugin would translate normalized operations to
  USB, HID, I2C, sysfs, or a vendor SDK.
- `luminate_export_plugin!` exports the symbols and generated ABI adapter that
  make the crate discoverable.

## ABI memory rules

Variable-sized data crosses the generated ABI adapter as CBOR rather than Rust
collections. Plugin authors do not manage these pointers or buffers directly.

- topology vectors are serialized into stable adapter-owned storage
- updates are decoded before `DemoSystem::apply` runs
- batch result slots are written in input order
- panics are contained and become bounded internal failures

## Proof-of-life behaviour

The plugin logs when:

- it is probed and accepted by the daemon
- lighting updates are applied

That makes it useful as a development harness: if the daemon, socket path, and
plugin callback wiring are working, you should see log messages from this plugin.

The topology payload is intentionally synthetic and human-readable. It is a
teaching aid, not a model for efficient real hardware transport. The demo also
keeps a small in-memory shadow map so accepted updates have observable internal
state; that map is not daemon state, hardware readback, or durable persistence.

## Guidance for plugin authors

Use this plugin as a starting point when you want to understand:

- How to publish plugin metadata
- How to describe devices/surfaces/elements/groups
- How the daemon normalizes topology before exposing it to consumers
- How target updates flow back into the plugin

When writing a real hardware plugin:

- Keep plugin metadata static and simple
- Make probing cheap, side-effect-free, and honest about absent hardware
- Keep the topology pointer alive for the lifetime required by the ABI
- Ensure declared capabilities match what update handling really accepts
- Prefer structured authored topology data over giant hand-written match trees
- Use the batch callback to reduce hardware transactions when that is useful;
  otherwise preserve the same per-update semantics as the single callback
- Log probe and unusual hardware/update behaviour clearly
- Keep transport quirks inside the plugin, not in the daemon

For the full contract, configuration, failure handling, and capability rules,
see [`docs/development/plugin-authoring.md`](../../docs/development/plugin-authoring.md).
