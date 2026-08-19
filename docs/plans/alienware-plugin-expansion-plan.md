<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Alienware plugin expansion plan

Status: implementation in progress

## Goal

Grow `luminate-plugin-alienware` from its currently supported modern laptop
controllers into a family of narrowly identified AlienFX drivers. The first new
family is the legacy nine-byte HID protocol used by the `187c:0525` Alienware
M14x profile. Later work may add other laptops, monitors, mice, and external
keyboards when their identities, transports, topologies, and safe operations
are established independently.

Breadth is not a reason to weaken discovery. Luminate must not send commands to
an arbitrary Alienware HID collection merely because its vendor ID or report
length resembles a known protocol.

## Relationship to existing work

The [Alienware and Dell laptop platform support plan](alienware-platform-support-plan.md)
remains the implementation record for importing additional AW-ELC profiles.
Its completed code milestones and outstanding hardware-validation work are not
duplicated here. This plan is the umbrella for additional protocol families and
for shared policy that should apply across them.

The existing plugin already contains two drivers:

- the 64-byte `0xcc` Darfon keyboard protocol used by the hardware-validated
  Alienware m16 R2 keyboard; and
- the 34-byte AW-ELC protocol used by the m16 R2 chassis lighting and several
  externally corroborated Dell laptop profiles.

Those implementations should remain behaviourally stable while common
discovery or profile infrastructure is generalized.

## Reference provenance and licensing

The initial protocol-family survey uses AlienFX-Tools commit
`52713b238066d1343a492018ded546ff751cfcd4` from 2026-07-28. In particular:

- `AlienFX-SDK/AlienFX_SDK/AlienFX_SDK.cpp` contains its HID discovery,
  report-length classification, transaction sequencing, and operation routing;
- `AlienFX-SDK/AlienFX_SDK/alienfx-controls.h` contains protocol command
  templates for API v2 through v8; and
- `alienfx-gui/Mappings/devices.csv` contains user-facing device and zone
  mappings.

AlienFX-Tools is MIT-licensed. It may be used as a research reference and its
independently relevant protocol facts may be re-expressed in Luminate's
GPL-3.0-or-later Rust implementation. If substantial source expression is ever
copied rather than independently reimplemented, preserve the required MIT
copyright and permission notice in the resulting distribution. Prefer a clean
Rust design, explicit provenance, and protocol fixtures over transliterating
the C++ implementation.

The `187c:0525` topology is additionally corroborated by the GPL-3.0
`trackmastersteve/alienfx` M14xR3 controller profile. Record a pinned revision
before implementation. Its exact VID/PID and zone declarations are useful
profile evidence; its source must not be treated as licence-free data.

Neither upstream project is a build input. This work adds no dependency.

## Protocol-family inventory

AlienFX-Tools recognizes these relevant HID families:

| Family | Report shape | Example scope | Luminate direction |
| ------ | ------------ | ------------- | ------------------ |
| Legacy v2 | 9-byte HID output | M11x, M14x, M15x and some older laptops | Add exact laptop profiles, beginning with `187c:0525` |
| Legacy v3 | 12-byte HID output | Later pre-AW-ELC laptops | Defer until an exact profile and hardware reporter exist |
| AW-ELC/v4 | 34-byte HID report | Modern chassis and four-zone laptop lighting | Continue under the existing AW-ELC plan |
| Darfon/v5 | 64-byte `0xcc` feature report | Modern internal keyboards | Expand only through exact keyboard identities and layouts |
| Peripheral v6 | 65-byte interrupt report | Alienware monitors | Treat as a distinct monitor driver |
| Peripheral v7 | 65-byte interrupt report | Alienware mice | Treat as a distinct mouse driver |
| Peripheral v8 | 65-byte feature/interrupt report | External keyboards | Treat as a distinct keyboard driver |

AlienFX-Tools' historical documentation also mentions an eight-byte v1 family,
but the pinned SDK no longer detects or implements it distinctly. Do not add a
v1 label or compatibility claim without a separate primary implementation or
hardware capture that establishes its framing and operations.

Fan control, thermal monitoring, power profiles, overclocking, ACPI lighting,
and Windows LightFX emulation are outside Luminate's lighting-plugin scope.
They must not be pulled into this plan merely because AlienFX-Tools implements
them in the same repository.

## Support and validation policy

### Exact profiles, not generic probing

Each writable profile must own:

- exact USB vendor and product IDs;
- the expected HID usage page and usage when available;
- report kind and exact report length;
- a stable model name and validation status;
- explicit firmware light IDs and Luminate topology declarations;
- supported effects, brightness, persistence, and power-state semantics; and
- any required pacing, status polling, or reset precondition.

Report length may corroborate a profile after VID/PID matching, but must never
select a protocol by itself. Unknown Alienware product IDs should produce a
privacy-conscious diagnostic and no writable topology. Multiple matching
collections or controllers must fail closed unless the profile defines an
unambiguous selector and stable per-instance identity.

### Evidence levels

Use the existing Alienware validation vocabulary where it remains sufficient:

- `HardwareValidated`: every advertised operation and topology element has
  been exercised through Luminate on the named hardware;
- an upstream-specific corroborated state: exact identity, protocol, and
  topology are backed by a pinned independent implementation but are not yet
  tested through Luminate; and
- unknown/unsupported: evidence is insufficient for writable publication.

Before adding more upstream-specific enum variants, consider whether one
general `ExternallyCorroborated` state with structured provenance would avoid
encoding project names into the domain model. Changing the existing validation
type is an internal architectural decision, but descriptor warnings and notes
are user-visible and must remain clear and stable.

Externally corroborated profiles should be gated by an explicit plugin setting
unless there is strong exact-profile evidence and review approves default
enablement. Decide this once for legacy AlienFX profiles before implementing
the M14x milestone; do not let individual profiles acquire inconsistent policy
accidentally.

### Command safety

- Never expose or encode controller flash-erasure commands.
- Do not issue a reset as part of read-only discovery.
- Treat reset, persistence, power-state slots, and firmware animation commits
  as separately validated capabilities, not incidental setup steps.
- Bound every light mask, report offset, sequence number, and command length
  before opening hardware.
- Model device busy and malformed status replies explicitly. Retry only an
  operation proven idempotent, with a small fixed limit and required pacing.
- Invalidate shadow state after device disappearance, failed multi-report
  transactions, reset, or profile changes.
- Warn users not to run AWCC, OpenRGB, AlienFX-Tools, or another controller
  against the same device concurrently.

## Architecture

### Driver families

Keep one `luminate-plugin-alienware` crate, but route resolved hardware through
typed internal driver families. A family should own its transport framing,
status model, pacing, and operation lowering. A profile should own identity,
topology, and the capabilities valid for one model.

Avoid a single version-number switch accepting loosely related byte arrays.
Prefer types such as `LegacyAlienFxProfile`, `AwElcProfile`, and keyboard-layout
identities whose constructors validate the facts required by their transports.
Common colour conversion and HID-opening code can remain shared where the
semantics genuinely match.

The external device IDs must remain stable for existing hardware. Define the
M14x public device ID and the policy for additional simultaneous Alienware
controllers before publishing the first legacy topology. A model-specific ID
such as `alienware-m14x` is simple for the first device but does not scale to
multiple legacy profiles; a family ID such as `alienware-legacy` risks moving
persisted state between incompatible topologies. The preferred direction is a
stable profile-derived ID, with an explicit collision policy if two identical
controllers can be attached. This decision is compatibility-sensitive and
requires review before implementation.

### Profile registry

Use a declarative, exhaustively tested registry rather than importing
AlienFX-Tools' mutable user mappings. Each entry should cite its evidence and
contain only facts safe to publish. Do not infer regional keyboard geometry or
physical coordinates from a logical zone order.

Profile declarations should make invalid combinations difficult to represent:
for example, an ordinary live zone should not be able to carry power-button
slot semantics, and a volatile-only profile should not accidentally advertise
`SaveCurrent`.

If the registry becomes large, keep its primary source reviewable Rust or a
small checked source-data format with a generator and schema validation. Do not
introduce generation or a new serialization dependency for the initial M14x
profile.

### Discovery

Discovery should proceed from exact profile candidates:

1. enumerate a known VID/PID;
2. filter to the profile's expected HID collection;
3. verify non-mutating descriptor facts, including report shape;
4. perform a read-only identity/status query only when that protocol defines
   one and the query is known safe;
5. resolve exactly one profile and cache its complete routing identity; and
6. publish topology only after all required checks succeed.

Linux and Windows expose HID collections differently. Preserve the existing
usage-based cross-platform selection rule rather than relying on enumeration
order or on Linux hidraw exposing several collections through one node.

### Legacy v2 transport

Implement legacy nine-byte reports in a new module rather than extending the
34-byte AW-ELC encoder. The transport needs typed construction for the smallest
initial command set, sequence-number handling, required inter-command delays,
and strict status parsing. Tests should derive expected bytes from pinned
protocol evidence without copying upstream implementation structure.

The M14x first milestone should expose volatile static colour and off only
unless review of both references proves that another operation is an inherent,
non-persistent part of the same safe transaction. Brightness, blink, morph,
power-state slots, reset, and persistence should land as separate capability
increments after hardware validation.

## First milestone: `187c:0525` M14x

The initial profile is the device called `Alienware M14xR3` by
`trackmastersteve/alienfx`, identified by `187c:0525`. The user's contact has a
physical laptop with that controller and can potentially perform opt-in tests,
but its consumer-facing revision name is not yet independently known.

Before implementation, capture without serial numbers:

- DMI product name and product version;
- the HID report descriptor for the exact `187c:0525` collection;
- usage page, usage, input/output/feature report lengths, and product string;
- the current operating system and whether a known tool can still control it;
  and
- the pinned `trackmastersteve/alienfx` revision used for its profile and
  packet evidence.

Do not block offline packet and topology tests on hardware availability, but do
not promote the profile or broaden advertised capabilities without the capture.

The candidate topology must be transcribed into a review table before code is
written. Cross-check each logical light ID and power-state code between the two
reference implementations. Any disagreement remains unknown until hardware
identification; do not merge or rename zones to make the sources appear to
agree.

Initial implementation scope:

- exact, opt-in discovery for `187c:0525` and its expected HID collection;
- one profile-owned zone topology and an all-zones group;
- volatile static RGB and off using the legacy transaction sequence;
- `Restore` reconciliation with assumed state, unless a truthful readback is
  established;
- deterministic packet, error, discovery, topology, and routing tests;
- a narrow udev rule and packaging coverage for `187c:0525`; and
- a privacy-conscious diagnostic and hardware-validation procedure.

Explicitly deferred from the first write test:

- persistence and saved groups;
- reset and global enable/disable commands;
- AC/battery/sleep power-button slots;
- blink, morph, tempo, and brightness;
- automatic support for `187c:0521` or any other legacy PID; and
- generic user-configured zone scanning.

## Later milestones

### Additional legacy laptops

Add v2 and v3 profiles one at a time. Each profile requires exact identity,
descriptor evidence, a reviewed zone table, deterministic fixtures, and a
named hardware-validation owner or an explicit externally corroborated state.
Sharing a packet family does not establish shared light masks, power slots, or
persistence semantics.

### Darfon keyboards

Use AlienFX-Tools to cross-check protocol operations already implemented for
the m16 R2, but add models only through exact keyboard identities and regional
layout maps. A shared `0d62` vendor ID, `0xcc` usage, or 64-byte feature report
does not establish per-key topology.

### Monitors, mice, and external keyboards

Treat v6, v7, and v8 as separate driver projects under this umbrella. Although
their reports are all 65 bytes, they use different transports, vendors,
commands, topology types, and capability semantics. Each family should receive
its own focused plan before implementation. Prefer existing Luminate device
categories and frame abstractions rather than importing AlienFX-Tools' GUI
effects or user mapping model.

## Documentation and provenance

Add a legacy AlienFX hardware note under
`docs/design/hardware/alienware/` before the first implementation. It should
document report framing, status and sequence rules, safe command sequences,
known unknowns, and pinned sources. Keep inferred facts visibly distinct from
hardware observations.

Update the plugin-development guide, packaging documentation, and `docs/TODO.md`
with every supported identity and validation state. Descriptor notes should
tell users when a profile is externally corroborated and how to submit a safe
hardware report.

Record an upstream refresh procedure: compare later AlienFX-Tools revisions
against the pinned commit, review changed commands and mappings manually, and
never regenerate supported profiles automatically from upstream data.

## Testing and verification

For each new family or profile, add deterministic coverage for:

- exact positive and near-miss VID/PID/usage/report-shape discovery;
- ambiguous collections and multiple controllers failing closed;
- complete packet bytes, sequence progression, masks, colour quantization,
  padding, and pacing decisions;
- short writes, I/O failures, malformed status, busy status, and retry limits;
- exact topology IDs, groups, tags, notes, warnings, and capabilities;
- unsupported effects, persistence, power slots, and unknown targets producing
  errors before a hardware write;
- batch coalescing only where the profile and protocol permit it;
- shadow invalidation after partial failure, disappearance, and identity
  changes;
- unchanged m16 R2 keyboard and AW-ELC descriptors and routing; and
- plugin metadata, udev templates, and package-root contents.

Run the focused plugin tests during implementation:

```sh
cargo test -p luminate-plugin-alienware
```

Before completing each implementation milestone, run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Run the packaging smoke tests that cover the platforms and udev changes. Record
hardware, host-integration, network, privilege, and operating-system gaps
exactly rather than treating them as passing checks.

## Hardware-validation protocol

Hardware writes are opt-in and should progress from the narrowest reversible
operation:

1. confirm exact identity and descriptor without writes;
2. stop AWCC, OpenRGB, AlienFX, and other competing controllers;
3. record the pre-test visible state without reading or logging serial data;
4. set one known non-power zone to low-intensity red, green, blue, and off;
5. restore that zone, then identify remaining zones one at a time;
6. test the all-zones transaction and daemon restart behaviour;
7. test unplug/reconnect and suspend/resume only after ordinary writes are
   reliable; and
8. restore a neutral or user-selected state.

Reset, power-state programming, and persistence require separate explicit
authorization and a recovery procedure. A profile becomes
`HardwareValidated` only after every advertised operation and topology element
has passed on that exact model.

## Compatibility and security implications

- New topology IDs become compatibility-sensitive once published. Resolve the
  legacy public device-ID policy before implementation.
- Adding a udev rule grants the daemon raw write access to the exact controller.
  Keep product and, where udev permits, interface matching narrow; unknown
  devices must remain unpublished and unwritable.
- Persisted intended state may trigger writes after daemon restart. An opt-in
  profile must not replay state while disabled, and state from one profile must
  never be applied to another profile sharing a family.
- Protocol masks and zone IDs are firmware addresses, not harmless display
  metadata. Validate shifts and bounds so malformed profiles cannot select
  undocumented lights.
- Imported commands may contain destructive or persistent operations even when
  a reference GUI exposes them casually. Luminate should encode only commands
  reachable through reviewed capabilities.

## Decision points before implementation

- [x] Approve a stable public device-ID scheme for legacy profiles and
  simultaneous controllers.
- [x] Decide whether all externally corroborated Alienware profiles share one
  opt-in setting or use family-specific gates.
- [ ] Decide whether to generalize `ValidationStatus` to structured provenance
      before adding a second upstream source.
- [ ] Review the exact M14x zone and power-state table from both pinned sources.
- [ ] Review the M14x HID descriptor capture and select its exact collection.

## Implementation sequence

- [x] Survey AlienFX-Tools protocol families and establish the safety boundary.
- [x] Choose a family-driver plus exact-profile architecture for planning.
- [x] Pin and document the M14x profile sources and resolve their disagreements.
- [x] Resolve the compatibility-sensitive device-ID and opt-in policy decisions.
- [x] Add legacy protocol documentation and deterministic packet fixtures.
- [ ] Implement the typed v2 transport and exact M14x profile.
- [ ] Add discovery, topology, routing, configuration, and packaging support.
- [ ] Run focused, workspace, and packaging verification.
- [ ] Perform opt-in M14x hardware validation and promote capabilities
      independently as evidence arrives.
- [ ] Select any next exact profile only after reviewing the first milestone's
      design and validation results.
