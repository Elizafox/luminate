<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Keychron plugin plan

## Status

Active. A generic experimental write surface is implemented with mocked
transport coverage. It discovers compatible Raw HID collections, publishes
firmware LED indices, applies per-key static colour or off, and offers
`SaveCurrent` only after the current session has established a complete
shadow. None of that surface has been validated on real hardware yet.

## Goal

Add offline host-side lighting control for modern Keychron keyboards running
stock QMK firmware. The plugin must use the keyboard's vendor-defined Raw HID
collection and must never claim or inspect ordinary keyboard, mouse, media, or
consumer-control reports.

Keychron's current public QMK firmware exposes a common command set over the
QMK Raw HID usage page `0xff60`, usage `0x61`. Command `0xa2` advertises the
`FEATURE_KEYCHRON_RGB` capability and command group `0xa8` provides a versioned
RGB protocol. The shared RGB implementation includes LED count and mapping,
per-key HSV state, mixed-effect regions, indicator configuration, and an
explicit EEPROM save command.

## Scope

The first experimental implementation is deliberately generic:

- enumerate Keychron vendor `0x3434` only;
- select only Raw HID usage `0xff60:0x61`;
- query the Keychron protocol version, supported features, RGB protocol
  version, and LED count before any write;
- reject short, mismatched, failed, or unsupported responses;
- keep the HID exchange behind an injectable transport and cover it with a
  mock device;
- publish numbered firmware LED indices without guessed key names or geometry;
- allow only static colour and off through the runtime per-key HSV commands;
- locally validate every colour range before it reaches firmware; and
- expose the explicit save command only after Luminate has established a
  complete session shadow.

No udev rules will be installed until an exact product ID and interface have
been validated. A vendor-wide access rule would expose unrelated Keychron
devices and collections.

## Architecture

`luminate-plugin-keychron` is an independent hardware plugin with these
internal layers:

1. `protocol` owns the 32-byte Raw HID packet shapes, command constants,
   response validation, and typed discovery results.
2. `transport` owns Raw HID request/response exchange. Production uses
   `hidapi`; tests inject a mock transport.
3. Plugin discovery enumerates candidate Raw HID collections, opens each exact
   path, and retains only devices that advertise the common Keychron RGB
   feature and a supported RGB protocol version.
4. The experimental topology uses a product-ID identity only when exactly one
   device with that product ID is attached. Indistinguishable duplicates are
   omitted rather than assigned enumeration-derived identities.
5. Model profiles will later translate firmware LED indices into stable key
   identities and physical geometry. Unknown layouts must preserve that
   uncertainty rather than guessing key names.

Stable device IDs must not depend on `/dev/hidrawN` or enumeration order. The
generic profile therefore uses `keychron-3434-<pid>` only for a unique attached
product. Exact profiles still require a validated identity strategy before
supporting multiple identical devices.

## Follow-up phases

### Hardware identity and topology

- [ ] Select one attached reference keyboard and record its model, revision,
      firmware version, VID/PID, interface number, usage page, usage, and report
      descriptor.
- [ ] Compare firmware LED count and row/column mapping with its public QMK
      `g_led_config`.
- [ ] Add an exact profile with stable key IDs and geometry.
- [ ] Add a narrowly matched udev rule for that product and Raw HID interface.

### Volatile lighting

- [ ] Validate per-key HSV get/set without issuing `RGB_SAVE`.
- [ ] Confirm changes disappear after a power cycle and do not otherwise write
      nonvolatile storage.
- [x] Implement generic per-key static colour and off with local range
      validation and complete-shadow guards.
- [ ] Implement brightness and per-key batching.
- [ ] Add readback using the protocol's get commands with the fidelity actually
      observed on hardware.

### Persistence and effects

- [x] Map Luminate `SaveCurrent` to `RGB_SAVE` only after the complete
      persistable state is known in the current plugin session.
- [ ] Validate the guarded `RGB_SAVE` path on real hardware.
- [ ] Exercise firmware effects and mixed regions individually and document their
      model-specific availability.
- [ ] Add frame streaming only if sustained hardware tests establish a truthful
      rate ceiling and acceptable update behaviour.

### Expansion

- [ ] Import closely related models only when their exact identity, collection,
      topology, firmware feature bits, and protocol version are corroborated. Treat
      legacy non-QMK boards, white-backlight variants, receivers, Bluetooth, and
      firmware predating the common protocol as separate compatibility families.

## Verification

Each phase requires deterministic protocol tests for success, unsupported
features, malformed lengths, mismatched command echoes, firmware failures, and
transport errors. Hardware tests must be ignored and explicitly gated because
they change visible lighting. Before completion, run the workspace formatting,
Clippy, and test workflow described in `CONTRIBUTING.md`.
