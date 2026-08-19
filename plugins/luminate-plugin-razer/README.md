<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Luminate Razer plugin

`luminate-plugin-razer` is an experimental native lighting plugin derived from
OpenRazer protocol evidence. It does not require OpenRazer at build time or
runtime.

The current partially validated profile is the original full-size Razer
BlackWidow V4 Pro (`1532:028d`). Whole-device static colour, off, spectrum,
wave, wheel, reactive, breathing, starlight, brightness, and complete 8×23
frame upload are exposed. The US-ANSI key, command-dial, side-light, and
wrist-rest elements are named and individually addressable through a complete
shadow frame. Key coordinates are corroborated by OpenRGB, side-light ordering
was validated on the attached unit, and wrist-rest ordering remains presumed
because the test wrist rest is temporarily unavailable. Random-colour effect
forms stay hidden until their hardware validation is complete. Ordinary effect
and brightness writes use the live-validated volatile path. `SaveCurrent`
explicitly commits the current firmware effect and brightness after Luminate
has established a persistable state in the current plugin session.
Element-only batches are coalesced into one complete frame upload. Sustained
live transport testing supports the advertised conservative 60 Hz frame-rate
ceiling; frame rows are not presented atomically.

Razer keyboard devices publish `shape:keyboard` and `form:standalone`. The
mapped BlackWidow V4 Pro additionally publishes `layout:us-ansi`; provisional
profiles omit a layout tag because their regional arrangement is not known.
Each discovered keyboard is identified by its non-empty HID serial when one is
available, with the exact HID path as the fallback physical identity and
control route. Conflicting duplicate identities are omitted. Writes always
reopen the selected path; they never select the first matching model during an
operation. Multiple distinguishable keyboards of the same model therefore
have independent target IDs and frame/persistence shadows.

The restart-required `enable_untested_devices` setting defaults to `true` and
gates imported `Untested` profiles before discovery or hardware access. The
wired BlackWidow V4 (`1532:0287`), V4 75% (`1532:02a5`), V4 Mini HyperSpeed
(`1532:02b9`), and V4 Tenkeyless HyperSpeed (`1532:02d7`) profiles are enabled
provisionally. They expose inherited matrix coordinates, volatile lighting,
and only the firmware effects declared by OpenRazer. Their topology, HID
transport, and visible results have not been tested on physical units, and
their descriptors and first-discovery warning ask users to report results.
Persistence remains disabled for them. Disabling the setting does not hide the
`PartiallyValidated` BlackWidow V4 Pro.

Additional provisional wired profiles cover the BlackWidow V3 Mini HyperSpeed
and Huntsman Mini Analog. OpenRazer and OpenRGB agree on their exact interface-3
HID identity, transaction, and matrix dimensions. They have the same untested
coordinate topology and volatile-only safety limits. The Huntsman does not
advertise starlight.

The wired Huntsman V2, Huntsman V2 Tenkeyless, Huntsman V2 Analog, Huntsman V3
Pro, Huntsman V3 Pro Tenkeyless, DeathStalker V2, DeathStalker V2 Pro, and
DeathStalker V2 Pro Tenkeyless are also provisional. Their interface-3
transport is corroborated across OpenRazer and OpenRGB. Luminate uses
OpenRGB's corrected or capture-backed matrix sizes where the catalogues differ.
The Huntsman V2 Analog exposes a combined 9×22 coordinate frame for its 6×22
keyboard and 3×22 underglow zones; the physical occupancy and zone shape have
not been validated. All of these profiles are wired-only, volatile-only, and
require `enable_untested_devices`.

```toml
[[plugins]]
name = "luminate-plugin-razer"

[plugins.config]
enable_untested_devices = false
```

On Linux, the packaged udev rule grants the daemon access only to interface 3
of these exact imported products. It does not grant access to the keyboard,
mouse, media, or other input interfaces. Do not run Synapse, OpenRazer,
OpenRGB, or another lighting controller against the same keyboard while
Luminate owns it.

Build the plugin with:

```sh
cargo build -p luminate-plugin-razer
```

The ignored hardware tests require a dedicated `1532:028d` unit and intentionally
change its lighting:

```sh
cargo test -p luminate-plugin-razer \
  queries_attached_blackwidow_v4_pro_identity -- --ignored
cargo test -p luminate-plugin-razer \
  exercises_attached_blackwidow_v4_pro_static_effect -- --ignored
cargo test -p luminate-plugin-razer \
  validates_attached_blackwidow_v4_pro_power_cycle -- --ignored
cargo test -p luminate-plugin-razer \
  validates_attached_blackwidow_v4_pro_volatile_storage -- --ignored
```

These tests never log the device serial. The static-effect test shows an
off/red/green/blue/off sequence, restores the original brightness, and selects
spectrum after its visual checks. The storage tests each wait up to 60 seconds
for a physical disconnect and reconnect. They validate persistent and volatile
brightness separately; the persistent test restores brightness and spectrum
after a successful reconnect.
The reviewed upstream provenance, imported profiles, deferred families, and
refresh procedure are recorded in the
[profile import manifest](../../docs/design/hardware/razer/profile-import-manifest.md).
The [BlackWidow V4 Pro protocol specification](../../docs/design/hardware/razer/blackwidow-v4-pro-rgb-hid-protocol-spec.md)
records packet and live-validation evidence.
