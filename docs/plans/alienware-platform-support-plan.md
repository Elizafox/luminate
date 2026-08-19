<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Alienware and Dell laptop platform support plan

Status: implementation in progress

## Goal

Extend `luminate-plugin-alienware` to support the additional laptop AW-ELC
platforms that OpenRGB identifies explicitly, without applying the tested
Alienware m16 R2 topology or its power-button semantics to different hardware.

The first import covers these OpenRGB profiles:

| Platform ID | OpenRGB model | USB product | Zones |
| ----------- | ------------- | ----------- | ----: |
| `0x0c01` | Dell G5 SE 5505 | `187c:0550` | 4 |
| `0x0a01` | Dell G7 15 7500 | `187c:0550` | 16 |
| `0x0e03` | Dell G15 5511 | `187c:0550` | 4 |
| `0x0e0a` | Dell G15 5530 | `187c:0550` | 4 |

OpenRGB registers both `187c:0550` and `187c:0551` for its common AW-ELC
driver, but its named platform table currently contains the four profiles
above. Luminate's existing `187c:0551` m16 R2 support remains a separate,
hardware-validated profile. This work does not claim that every device sharing
either USB ID is safe to control.

## Reference provenance

The import is based on OpenRGB master commit
`5e6d627f519a791487e766aeee51eed7bf7129ef`, recorded on 2026-08-16:

- `Controllers/AlienwareController/AlienwareControllerDetect.cpp` registers
  `187c:0550` and `187c:0551`.
- `Controllers/AlienwareController/AlienwareController.cpp` contains the four
  platform-specific zone-count quirks and zone-name tables.
- The same controller implementation corroborates the existing AW-ELC report
  family: configuration query `0x20`, user animation `0x21`, zone selection
  `0x23`, actions `0x24`, direct colour `0x27`, and reset `0x28`.

OpenRGB is a research reference, not a build input. Do not copy its C++ source
or comments into the project. Re-express the independently relevant protocol
facts in Luminate's existing GPL-3.0-or-later implementation and cite the
pinned revision in hardware documentation.

## Safety and support policy

Recognition and validation are different states:

- The m16 R2 profile remains `HardwareValidated` and retains its current
  trackpad-ring, rear-logo, and firmware-managed power-button topology.
- Imported OpenRGB profiles are `OpenRgbCorroborated` until exercised through
  Luminate on their named hardware.
- An unknown platform ID, an unexpected zone count, a malformed configuration
  response, or more than one plausible AW-ELC controller must fail closed. It
  must not publish writable topology or inherit the m16 R2 profile.
- Imported profiles must publish a clear descriptor warning and a rate-limited
  process warning stating that the profile is externally corroborated but not
  hardware-validated in Luminate.
- No destructive protocol command is added. In particular, opcode `0xff`
  (`ERASE_FLASH`) remains undocumented as an operation and unreachable.
- Do not infer keyboard layouts, physical geometry, power conditions, or
  persistence semantics that OpenRGB's profile table does not establish.

The imported profiles are enabled by default because they have explicit model,
platform, zone-count, and working-protocol evidence upstream. If review prefers
an opt-in gate like the provisional Razer profiles, add that as a separate
policy decision before implementation; it is not assumed here.

## Proposed design

### 1. Typed controller identity and profile registry

Add an internal AW-ELC identity model containing:

- exact USB VID/PID;
- firmware-reported platform ID;
- firmware-reported zone count; and
- a resolved `AwElcProfile` registry entry.

Each profile owns its model name, validation status, exact expected zone count,
and zone declarations. A zone declaration contains a stable surface ID, a
human name, the firmware zone ID, conservative physical tags, and its supported
operation class. Keep the registry declarative and exhaustively tested.

The existing m16 R2 profile uses zones `0x00`, `0x02`, and `0x04`; zone `0x04`
retains its special `PowerButton` operation class. The imported profiles use
ordinary live-zone operation only:

- four-zone profiles: `left`, `middle`, `right`, and `numpad`;
- G7 15 7500: `left`, `centre-left`, `centre-right`, `right`, followed by
  `light-bar-1` through `light-bar-12`.

Use Oxford English for Luminate-owned identifiers and display names, hence
`centre-left` and `centre-right`, while preserving OpenRGB's source terminology
in the provenance table where quoted.

Do not represent the four named keyboard regions as individual keys. Publish
them as opaque lighting surfaces with conservative `form:laptop` and
`shape:keyboard-zone` tags. Publish the twelve G7 light-bar segments as opaque
surfaces with `form:laptop-light-bar` and `shape:segment` tags. These tags are
descriptive only and do not assert coordinates or ordering beyond the
firmware/OpenRGB zone order.

### 2. Configuration query

Add a small read-only query beside the existing AW-ELC packet encoder:

1. Open the exact candidate VID/PID and vendor usage.
2. Send `03:20:02` (`REPORT_CONFIG`) using the existing hidapi placeholder
   framing.
3. Read and validate the complete feature report.
4. Parse the big-endian platform ID and reported zone count from the documented
   response offsets.
5. Resolve the tuple through the profile registry.

Return a typed error for short reports, wrong report markers, unknown platform
IDs, and zone-count mismatches. Discovery may log these errors but must preserve
the uncertainty and withhold the device.

OpenRGB carries zone-count quirks for all four imported profiles. Luminate
should treat the registry count as the expected safe bound and the raw firmware
count as diagnostic evidence. A known platform with OpenRGB's documented
misreported raw count may resolve only through an explicit per-profile quirk;
never silently clamp arbitrary counts. Capture the exact raw values in user
hardware reports before adding such a quirk if OpenRGB does not record them.

### 3. Discovery and routing

Replace the `aw_elc: bool` discovery field with an optional resolved controller
identity. Enumerate both `187c:0550` and `187c:0551`, query each candidate, and
cache the resolved identity along with the keyboard layout.

The current public device ID `alienware-aw-elc` remains unchanged for the m16
R2 and is reused for a single resolved laptop AW-ELC controller. This avoids a
persisted-state and CLI compatibility break. The descriptor model, hardware
claim, and update routing use the cached exact PID and profile instead of the
current global `AW_ELC_PID` constant.

If multiple supported AW-ELC candidates are present, publish none and emit an
actionable diagnostic rather than selecting by enumeration order. Supporting
multiple simultaneous controllers would require stable per-instance device IDs
and a persisted-state migration and is outside this laptop-focused change.

`apply` and batch routing need access to the latest cached identity. Reject an
AW-ELC update as unavailable if the cache has no resolved controller. Continue
to rely on the daemon's topology rescan path for unplug, resume, and hotplug
refresh. Avoid reopening a different PID based only on the target's generic
device ID.

### 4. Profile-driven topology

Change `topology::aw_elc_device()` to accept a resolved profile and exact
hardware identity.

For the m16 R2, preserve current surface IDs, groups, capability declarations,
appearance slots, notes, and persistence behaviour byte-for-byte where
possible.

For imported profiles:

- expose each declared zone as an opaque surface;
- expose an `all` topology group over every surface;
- offer static colour, off, portable morph, and the existing fixed-cadence
  AW-ELC hardware effects through the live-animation path;
- do not expose appearance slots or power-button operations;
- initially advertise volatile state only and no `SaveCurrent`, because the
  imported model table does not establish the saved-slot identity, boot restore
  behaviour, or cross-zone persistence contract on these laptops; and
- retain `Restore` reconciliation and assumed readback because the protocol has
  no exact live-state query.

The topology notes identify the upstream profile and pinned revision. Warnings
make the unvalidated-in-Luminate status visible to clients.

### 5. Generalize AW-ELC zone handling

Replace the fixed `Zone` enum's live variants with a profile-resolved live zone
ID. Keep the m16 R2 power-button variant distinct so invalid combinations remain
difficult to represent.

Target resolution receives the active profile and maps device, surface, and
group targets to its declared zones. It must reject surface IDs absent from the
active profile before opening hardware. Packet construction remains bounded by
the 34-byte hidapi buffer and must validate that selected-zone lists fit.

Generalize the live-zone shadow key to include enough controller/profile
identity that a rescan from one supported controller to another cannot reuse
stale animation records. Profile changes and disappearance invalidate the
shadow. Preserve the existing m16 R2 power-button shadow and write throttling
only for that profile.

Batch updates continue to coalesce only when their resolved operation classes
and profile permit it. Imported live zones may share one animation transaction;
they must never enter the m16 R2 power-slot batching path.

### 6. Packaging and metadata

Add `187c:0550` to:

- plugin vendor/product metadata and HID probe hints;
- the narrowly scoped Alienware udev rule template; and
- packaging documentation that lists granted hardware IDs.

Keep the existing `187c:0551` rule. No dependency or plugin ABI change is
expected. The topology changes only for newly recognized hardware, while the
existing device ID and m16 R2 descriptor remain compatible.

### 7. Documentation

Update:

- `docs/development/plugins/alienware-plugin.md` with the profile table,
  validation states, diagnostics, and hardware-report request;
- `docs/design/hardware/alienware/alienware-aw-elc-rgb-hid-protocol-spec.md`
  with the read-only configuration query, response offsets, provenance, and
  boundaries between corroborated and hardware-validated facts;
- `docs/development/packaging.md` for the additional udev ID; and
- `docs/TODO.md` to replace the generic additional-laptop item with specific
  remaining validation and unknown-platform work.

Provide a privacy-conscious reporting command or trace recipe that captures
VID/PID, platform ID, raw zone count, and profile resolution without logging
HID paths, serial numbers, usernames, or unrelated USB devices.

## Tests

Add deterministic fake-transport and fake-enumerator coverage for:

- parsing valid `REPORT_CONFIG` responses for each supported platform;
- short, malformed, and wrong-marker responses;
- unknown platform IDs and mismatched zone counts failing closed;
- discovery of each PID and exact cached PID routing;
- disappearance replacing rather than layering on cached discovery;
- multiple candidates failing closed independent of enumeration order;
- unchanged m16 R2 topology and hardware claim;
- exact surface IDs, firmware zone IDs, group membership, validation warnings,
  and capabilities for all four imported profiles;
- target resolution rejecting a surface from another profile before HID open;
- static, morph, and hardware-effect packets selecting the expected imported
  zone IDs;
- imported profiles rejecting appearance slots, power-state operations, and
  `SaveCurrent` without writes;
- m16 R2 power-button and save-current regression coverage;
- live shadow invalidation across profile/controller changes;
- batch partitioning between generic live zones and the m16 R2 power button;
- plugin metadata and probe hints containing both product IDs; and
- udev/template checks containing both narrowly scoped IDs.

Run affected tests early:

```sh
cargo test -p luminate-plugin-alienware
```

Before completion, run the full contribution workflow:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Also run relevant packaging checks that cover generated package roots and udev
rules. Record any check that cannot run because it requires attached hardware,
host integration, or elevated privileges.

## Hardware validation and promotion

Imported profiles remain externally corroborated after code completion. For
each physical model, collect and record:

1. exact VID/PID, platform ID, and raw zone count;
2. topology enumeration and surface names;
3. one-at-a-time static red/green/blue/off identification for every zone;
4. multi-zone selection and batch behaviour;
5. each advertised hardware effect and morph boundary;
6. controller response to the maximum zone selection without command spam;
7. unplug/reconnect, daemon restart, and suspend/resume behaviour;
8. coexistence warning behaviour with AWCC/OpenRGB or similar tools; and
9. restoration of a neutral or pre-test state.

Promote a profile to `HardwareValidated` only after its topology and every
advertised operation pass on that model. Persistence can be designed and added
later only after its saved-slot and boot-restore semantics are captured.

## Compatibility, security, and remaining risks

- Reusing `alienware-aw-elc` preserves existing selectors and persisted m16 R2
  state. A user moving the same state file between laptop models could still
  hold surface IDs absent from the new profile; normal topology validation must
  reject them rather than remap them.
- Descriptor topology is compatibility-sensitive. Existing m16 R2 IDs and
  semantics must not change during the generalization.
- The new udev rule grants the daemon account raw access to `187c:0550`, but no
  client gains direct access. Unknown profiles remain unwritable.
- OpenRGB has reports of controllers becoming unresponsive when spammed and of
  unknown high-zone-count platforms exposing unsafe assumptions. Retain
  Luminate's bounded transactions and pacing, validate every count before
  constructing a report, and never use an unknown platform as a generic
  fallback.
- The imported zone names prove logical ordering, not physical geometry. Avoid
  coordinate maps and per-key claims until hardware evidence exists.
- Supporting the older AlienFX families (`187c:05xx` other than `0550/0551`),
  other Darfon keyboards, regional per-key layouts, desktops, and unknown
  `0551` platforms is explicitly out of scope. They may use different protocol
  generations or topology semantics.

## Implementation sequence

- [x] Review and approve profile policy, default enablement, surface naming,
      single-controller behaviour, and conservative capability scope.
- [x] Add typed AW-ELC identities, validation status, and profile registry.
- [x] Implement and test the read-only configuration query.
- [x] Refactor discovery/cache/routing around the resolved controller.
- [x] Make topology and target resolution profile-driven while preserving the
      m16 R2 descriptor.
- [x] Generalize live-zone application, shadow identity, and batching.
- [x] Add metadata, udev, packaging, and documentation updates.
- [x] Run focused, full-workspace, and applicable packaging checks.
- [ ] Request model-specific hardware validation; promote profiles separately
      as evidence arrives.
