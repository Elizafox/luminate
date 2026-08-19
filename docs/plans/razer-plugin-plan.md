<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Native Razer plugin port plan

Status: implementation in progress

## Current checklist

- [ ] Finish the BlackWidow V4 Pro physical map, including wrist-rest ordering
      and remaining auxiliary-zone boundaries.
- [ ] Complete live effect, brightness/readback, persistence, streaming-rate,
      unplug/reconnect, restart, suspend/resume, hotplug, and input-coexistence
      validation; promote the profile only when the full checklist passes.
- [ ] Continue importing safely representable lighting profiles in protocol
      families, with exact selectors, fixtures, honest validation status, and a
      recorded blocker for each omitted family.
- [ ] Add the remaining transport families only when bounded: single-zone and
      classic matrix, mice/accessories, wireless/receiver routing, ARGB, and
      legacy reports.
- [ ] Finish narrow device permissions, package integration, sanitized user
      reporting instructions, and the upstream refresh/drift workflow.
- [ ] Define migration/alias handling before replacing published coordinate-only
      topology with named physical elements.

## Implementation resume point

The second provisional keyboard import cross-checks OpenRazer's report index,
transaction, dimensions, response behaviour, and firmware-effect declarations
against OpenRGB's exact interface-3 HID detectors and device tables. The wired
BlackWidow V3 Mini HyperSpeed and Huntsman Mini Analog agree across those facts
and are now imported. Both remain `Untested`, coordinate-only, volatile, and
gated by `enable_untested_devices`; the Huntsman omits starlight because
OpenRazer does not declare it. The profile schema now distinguishes the three
V4 no-response custom-frame exceptions from the response-bearing custom path
used by the other imported keyboards. This also corrects the earlier
provisional V4 Mini and V4 Tenkeyless response policy.

The wired Huntsman V2/V3 and DeathStalker V2 profiles are now imported as
provisional devices. Research found that their apparent transaction-ID
disagreement is not a routing contradiction: `0x1f`, `0x3f`, and `0x9f` all
select device 7 in the low three bits and differ only in the response-correlated
request tag. The dimension differences are resolved in favour of OpenRGB's
physically tested or capture-backed topology: 6×17 for the Huntsman V2 TKL,
9×22 for the Huntsman V2 Analog's combined keyboard and underglow frame, and
6×19 for the Huntsman V3 Pro TKL. Exact occupancy and the Analog's underglow
shape remain untested, so these profiles publish coordinate-only topology and
remain gated by `enable_untested_devices`.

The first provisional family import adds four wired BlackWidow V4 profiles:
the V4, V4 75%, V4 Mini HyperSpeed, and V4 Tenkeyless HyperSpeed. OpenRazer's
pinned driver places each on report/interface index 3 with transaction `0x1f`,
the 600 μs BlackWidow Chroma delay, extended-matrix commands, and bounded
matrix dimensions. Luminate publishes every coordinate because occupancy and
physical naming are unverified, marks each descriptor `Untested`, warns once
per model per process, and honours `enable_untested_devices` before opening or
publishing it. Volatile effects, brightness, and complete frames are exposed;
wheel is limited to the two profiles for which upstream declares it, and
persistence remains disabled. These profiles have deterministic registry and
topology coverage but no physical-hardware validation.

Commit `9ebe7d2` created the workspace crate and initial typed report. The
first implementation milestone completes the shared report and transport
core and establishes a partially validated BlackWidow V4 Pro profile. It adds
typed identity, brightness, effects, and frame commands; strict response
correlation and status errors;
a mockable `hidapi` feature-report transport; and a three-attempt, 10 ms busy
retry used only by the currently idempotent command set. Deterministic tests
cover the common envelope, upstream-derived command bytes, response rejection,
leading HID report ID, and transport execution.

The crate now exports a plugin descriptor and publishes one conservative,
partially validated BlackWidow V4 Pro profile. It recognizes exact
`1532:028d` interface 3, advertises only the visually validated hardware
effects, whole-device brightness, and complete 8×23 frame upload. It
deliberately withholds untested argument forms and selectable profiles.
Read-only and write-enabled plugin-host conformance tests pass with the
attached unit. Linux packaging
installs the plugin and a udev rule restricted to the exact product and
interface. Resume with the remaining hardware validation and profile data
before broad profile import; do not widen advertised capabilities ahead of
validation.

The BlackWidow V4 Pro is dedicated test hardware. The user has authorized
reversible lighting changes and high-confidence persistent changes, while
asking that hardware-destructive operations remain excluded. A live `hidapi`
feature-report test has opened exact interface 3 and successfully queried the
protocol serial twice with stable results and firmware version `1.0`. Both
responses passed length, report ID, envelope, CRC, transaction, command,
remaining-packet, and status validation. The test does not log the serial.
Brightness, static colour, off, and spectrum restoration have passed protocol
and visual tests. The device returned `Busy` on the first brightness write,
which motivated the bounded idempotent retry. A deliberate sequence visibly
transitioned to solid green, turned fully off, and returned to cycling colours.
Exact auxiliary-zone coverage still needs closer observation. Persistence has
not been exercised.

The no-response custom path has also passed a full-frame visual test. All eight
rows and 23 columns were set to cyan and the user observed uniform cyan across
all visible lighting before successful spectrum restoration. This establishes
safe complete-frame upload for the known matrix bounds, but it does not yet map
physical occupancy or identify each coordinate that feeds keys and auxiliary
zones.

A later 120-frame live transport run sustained 162.1 complete 8×23 frames per
second. The plugin now advertises a conservative 60 Hz maximum while retaining
the truthful non-atomic, immediate-row-write model. Brightness endpoints `0`
and `255` both read back exactly and the pre-test value was restored. Two
independent HID close-and-reopen cycles returned the same protocol identity.
Physical disconnect and reconnect preserved that identity as well. Brightness
and a static effect written with `VARSTORE` survived a power cycle; `NOSTORE`
brightness and static changes reverted to the persistent state. The keyboard
briefly displayed an older cyan custom frame during one reconnect before the
persisted spectrum effect took authority. Ordinary plugin updates now use
`NOSTORE`, and explicit `SaveCurrent` persistence excludes custom frames.
`VARSTORE` is the older snapshot-style persistence mechanism, not the newer
and incompletely decoded selectable-profile protocol. The plugin therefore
advertises current-state persistence without profile enumeration or selection.
After a full Linux system suspend and resume, the control interface reopened,
the protocol identity query passed, and write-enabled plugin-host conformance
completed successfully through the volatile command path.

The remaining inherited firmware effects have received an initial visual pass:
wave moved left-to-right and right-to-left; wheel moved clockwise and
anticlockwise; pressing Tab in reactive mode illuminated it red; single-colour
breathing faded red or blue to black; dual breathing alternated red, black,
blue, black; and two-colour starlight distributed colours across individual
keys. The plugin advertises only these tested argument forms. Random-colour
forms and untested boundaries remain hidden.

A row-isolation custom-frame sweep mapped rows 0–5 to the six physical key
rows and row 6 to keyboard side lighting. Row 7 appeared to control the wrist
rest. A detached-wrist-rest repeat produced no visible row 7 output, supporting
that identification. Exact attached wrist-rest ordering remains presumed
because the wrist rest is temporarily unavailable.

An OpenRGB research checkout at commit
`bcfaa7e8a740a414b96cd65c397e3dfc`, supplemented by current commit
`02d0e16a72f9b007e16e60ec2a68852fb81242f0` for newer profiles, provides an
independent device-table cross-check. Live coordinate tests with the wrist rest
detached confirmed `r0-c0` as unoccupied, `r0-c1` as the command dial, and
`r0-c2` as Escape.
They also completely mapped row 6: `c0`–`c8` are the left side from top to
bottom, `c9`–`c17` are the right side from bottom to top, and `c18`–`c22` are
unoccupied. Row 7 produced no visible output with the wrist rest detached, as
expected; attached ordering still needs validation.

The plugin now publishes the corroborated US-ANSI key map, live-validated
command dial and side-light segments, and presumed wrist-rest segments as
individually addressable elements. Element writes update a complete frame
shadow initialized to black, upload the bounded 8×23 frame, and invalidate the
shadow before transport so a partial failure cannot leave uncertain state
trusted. Firmware effects invalidate the shadow; brightness-only changes do
not. Element-only batches coalesce into one complete frame upload; mixed
batches retain ordered single-update behaviour.

The disposable OpenRazer research clone was created at
`/tmp/luminate-openrazer-reference` and pinned to the revision recorded below.
Re-clone that exact revision if temporary storage has been cleared. The clone
is not a build input and must not be added to the repository.

The worktree contained unrelated modified documentation, configuration, and
packaging files before this work began. Preserve them. Razer work currently
consists of this plan, the workspace membership and lockfile entries, and
`plugins/luminate-plugin-razer/`.

## Goal

Add one official `luminate-plugin-razer` that ports the lighting-relevant
protocol knowledge and device profiles from OpenRazer into native, idiomatic
Luminate abstractions. The original full-size US-ANSI Razer BlackWidow V4 Pro
(`1532:028d`) will be the first fully mapped and hardware-validated device and
the conformance target for the shared implementation.

The port should make OpenRazer-recognized lighting devices available by
default. Profiles without Luminate hardware validation will be visibly marked
untested and will emit a rate-limited warning asking users to report results.
Recognition is not the same as validation: documentation and diagnostics must
not describe imported profiles as fully supported until their topology and
advertised operations have passed device-specific tests.

The plugin must not depend on OpenRazer at build time or runtime. It will use
Luminate's plugin API, state model, topology, batching, and streaming paths,
plus the workspace's existing `hidapi` dependency.

The plugin as a whole is experimental. Its device IDs, topology, effect
descriptors, profile catalogue, configuration, and hardware behaviour may
change as testing replaces inherited assumptions with direct evidence. This
status must be visible in plugin metadata, user documentation, and startup
diagnostics rather than existing only in this plan.

Broad device coverage is a primary objective, not a distant follow-up. Port as
many lighting-capable profiles as can be represented safely by the shared
transport and capability model. The BlackWidow V4 Pro provides real-hardware
validation for those shared paths and its own profile; it does not artificially
limit the plugin to closely related keyboards.

## Porting doctrine

OpenRazer supplies protocol evidence and a mature catalogue of device quirks.
It does not define the new plugin's architecture. The result should look like a
Luminate plugin whose hardware facts happen to come from OpenRazer, not like a
kernel driver or sysfs API mechanically translated into Rust.

In particular:

- model Luminate capabilities and topology first, then map each device's proven
  commands onto them;
- preserve desired, observed, mixed, and unknown state semantics rather than
  reproducing OpenRazer's imperative setters;
- use Luminate batches, complete shadow frames, reconciliation, and streaming
  ownership instead of exposing packet-oriented operations to clients;
- share typed protocol families when their wire behaviour is genuinely the
  same, while keeping narrow profile quirks explicit;
- keep profile data declarative and validated rather than recreating sprawling
  product-ID switch statements;
- expose the broadest truthful control scope for each imported profile, even
  when a named physical map is unavailable;
- prefer a safe coordinate or zone topology over omitting a usable device, but
  never invent key names, occupancy, readback, persistence, or capabilities;
  and
- treat every imported profile as normal production code: no panics, unchecked
  reports, weakened validation, or second-class "experimental" code path.

The implementation may deliberately differ from OpenRazer wherever Luminate's
state model, safety rules, cross-platform transport, or user-facing topology
requires it. Such differences should be documented when they affect observable
hardware behaviour.

## Scope

The port covers lighting-related behaviour that maps truthfully onto Luminate:

- USB/HID discovery and safe selection of vendor control interfaces;
- serial and firmware queries needed for identity and diagnostics;
- LED and matrix brightness, including readback where trustworthy;
- static, off, and verified firmware effects;
- addressable matrix/zone writes and direct frame streaming;
- device, surface, group, and element targeting where a verified physical map
  exists;
- volatile versus persistent storage selection;
- wired and wireless transport variants where `hidapi` can provide the
  required semantics;
- hotplug, reconnect, and suspend/resume recovery;
- narrow device permissions, packaging, tests, and protocol documentation.

OpenRazer features outside Luminate's lighting domain remain out of scope:

- macros and key remapping;
- command-dial input behaviour;
- polling rate, DPI, scroll mode, game mode, and performance settings;
- battery charging policy and wireless pairing;
- input-event remapping or kernel-driver quirks;
- OpenRazer's sysfs, D-Bus, daemon, Python client, and kernel-module APIs.

Input-only features may be used during manual coexistence tests, but the plugin
must never claim, intercept, or modify standard keyboard, mouse, consumer
control, or vendor input reports unrelated to lighting.

## Decisions

- Implement one generic Razer plugin, not one plugin per product.
- Port protocol concepts and device knowledge into Rust rather than wrapping,
  invoking, or wholesale-translating OpenRazer.
- Import known lighting profiles in reviewable families. Do not generate a
  giant opaque table directly from upstream conditionals.
- Aim to import every lighting-capable profile whose transport and operations
  can be bounded and represented truthfully. Defer only profiles with a concrete
  transport, identity, safety, or topology blocker, and record that blocker.
- Enable untested profiles by default. On first discovery of each untested
  physical unit per plugin process, log a warning containing the model,
  VID:PID, profile status, and a concise request for results. Do not repeat the
  warning on every frame or topology pull.
- Expose a Boolean plugin setting named `enable_untested_devices`, defaulting to
  `true`. When `false`, exclude `Untested` profiles from discovery before
  opening or probing hardware. The setting does not suppress
  `PartiallyValidated` or `Validated` profiles.
- Add an untested note to each affected device descriptor so status remains
  visible outside logs. Do not weaken capability validation merely because a
  profile is untested.
- Never send commands based only on the Razer VID. Require an exact imported
  product profile and a positively selected control interface.
- Reject an incomplete or internally inconsistent profile at startup rather
  than publishing speculative topology.
- Use the BlackWidow V4 Pro as the first `validated` profile. Initially imported
  devices will be `untested` unless existing evidence can be reproduced on
  physical hardware during this work.
- Keep product validation state explicit data, not comments or model-name
  conventions. Candidate states are `Untested`, `PartiallyValidated`, and
  `Validated`, with concise evidence notes kept in permanent hardware docs.
- Do not add dependencies. Reconsider only if `hidapi` demonstrably cannot
  reproduce a required transport; discuss that architecture change before
  implementation.
- Preserve the existing dirty worktree and avoid changes to unrelated modified
  documentation and configuration files.

## Upstream source and provenance

OpenRazer is the approved reference implementation. The development clone is
pinned at commit `6820f9da169d354bc7e6e93a0aa8683a6bb75792` from 2026-07-05.
It has no Git submodules; the kernel drivers, daemon, Python library, tests, and
capture tooling are present in one repository.

Relevant source areas are:

- `driver/razercommon.c` and `.h`: transport, 90-byte report envelope, status,
  CRC, and response matching;
- `driver/razerchromacommon.c` and `.h`: shared command builders;
- `driver/razerkbd_driver.c`: keyboard profiles and per-product quirks;
- `driver/razermouse_driver.c`: mouse and receiver profiles;
- `driver/razeraccessory_driver.c`: mats, docks, stands, and accessories;
- `daemon/openrazer_daemon/hardware/`: model names, capability declarations,
  matrix dimensions, and client-facing constraints;
- `daemon/tests/` and `pylib/tests/`: behavioural expectations useful when
  reconstructing focused Luminate tests; and
- `scripts/wireshark/`: capture and protocol-analysis aids.

The relevant driver files are `GPL-2.0-or-later`, compatible with the new
plugin's `GPL-3.0-or-later` licence. Any Luminate source containing a substantial
adaptation must preserve the appropriate upstream copyright attribution in
addition to the normal project SPDX header. Permanent protocol documentation
must name the pinned upstream revision and distinguish:

- facts derived from OpenRazer;
- facts confirmed through captures or live hardware;
- Luminate-specific design and safety decisions; and
- unresolved assumptions inherited from an untested profile.

Rebuild typed packet construction and data profiles around documented fields.
Do not carry over Linux kernel control flow, sysfs parsing, large product-ID
switches, or unrelated features. Third-party sources may supplement gaps only
after their licences and provenance are checked.

The upstream clone under `/tmp` is a disposable research checkout, not a
vendored source or build input. Record the revision in documentation and tests
so future refreshes are deliberate and reviewable.

## Architectural shape

Create `plugins/luminate-plugin-razer/` with responsibilities along these
lines:

```text
plugins/luminate-plugin-razer/
├── Cargo.toml
├── README.md
├── build.rs
├── packaging/udev/60-luminate-razer.rules.in
└── src/
    ├── devices/
    │   ├── keyboards.rs
    │   ├── mice.rs
    │   ├── accessories.rs
    │   └── layouts/
    ├── protocol/
    │   ├── commands.rs
    │   ├── report.rs
    │   └── response.rs
    ├── discovery.rs
    ├── lib.rs
    ├── topology.rs
    └── transport.rs
```

This is a responsibility map, not a demand for empty modules. Split profile
families further when doing so clarifies meaningful protocol differences.

### Report and command model

Represent the common 90-byte report as a validated type with:

- status;
- transaction ID;
- remaining-packet count;
- protocol type;
- bounded data length;
- command class and command ID/direction;
- an 80-byte argument area;
- XOR CRC; and
- reserved byte.

Packet builders should be pure where possible. Typed command inputs should
represent LED target, storage mode, colour count, effect speed, direction,
matrix row/span, and response policy. Avoid passing loose bytes beyond the
serialization boundary.

Each command declares whether a response is required, optional, or intentionally
absent, along with timing and retry policy. The BlackWidow V4 Pro's custom-mode
and custom-frame operations are known no-response commands; this exception must
not become a transport-wide default.

Validate response length, status, CRC, transaction ID, command class/ID, and
remaining-packet semantics before reporting success. Decode busy, failure,
timeout, and unsupported statuses separately. Retry only commands proven both
idempotent and transient, with a strict bound.

### Device profiles

A profile should be declarative wherever the differences are genuinely data:

- VID and PID;
- stable model slug and user-facing name;
- validation status;
- device category;
- expected control-interface selector;
- request/response report indices and transport variant;
- transaction ID and response timing class;
- serial/firmware query variants;
- lighting targets and matrix dimensions;
- supported command families and argument constraints;
- per-command response exceptions;
- volatile/persistent storage semantics;
- physical layout/map availability; and
- documented quirks.

Use narrow Rust behaviour implementations where products truly differ. Do not
weaken the profile schema into arbitrary callbacks merely to mimic an upstream
switch statement.

Construct profiles through validated `const` data where practical and run a
complete registry validation in tests. Invalid combinations should be hard to
represent; examples include streaming without matrix dimensions, a mapped
layout with mismatched dimensions, or persistence without an explicit storage
policy.

### Validation state and warnings

Validation status affects diagnostics and support claims, not packet safety.
An untested profile still needs an exact VID:PID, a known transport, internally
valid capabilities, and a narrowly selected control interface.

When an untested unit is discovered, emit one warning per physical identity per
plugin process. Suggested meaning:

```text
Razer <model> (<vid>:<pid>) uses an untested Luminate profile; lighting is
enabled, but hardware reports are welcome
```

Include a device note with the same status and a stable documentation/reporting
location. Avoid warnings on every retry, frame, rescan, or daemon reconciliation.
If a profile has only partial lighting evidence, advertise only that subset and
mark it `PartiallyValidated`; status must never enable speculative capabilities.

The `enable_untested_devices` setting is a discovery and hardware-access gate,
not merely a presentation filter. Turning it off must prevent opening, probing,
publishing, or writing an untested profile. Apply it through the ordinary
Luminate plugin settings schema and restart-required configuration lifecycle.

### Topology tiers

Profile evidence determines topology precision:

1. A validated physical map exposes named elements, accurate geometry, groups,
   and frame ordering.
2. Known matrix dimensions without a physical map expose a matrix surface with
   stable coordinate elements such as `r0-c0`, excluding only cells known to be
   absent. Notes must explain that physical names and occupancy are unverified.
3. Zone-only hardware exposes documented zones.
4. Hardware with only whole-device effects exposes only device-level control.

Do not assign guessed physical key names. Imported matrix dimensions do not by
themselves prove occupancy. If writing an unknown coordinate is potentially
unsafe or affects non-lighting hardware, fall back to the broadest proven scope
rather than exposing a raw matrix.

All stable IDs should include a model slug and, when available, a trustworthy
serial-derived suffix so multiple identical devices remain distinct. Preserve
uncertainty when a serial is absent or unreliable; do not silently merge units.

### State, batching, and streaming

The daemon remains authoritative for desired state. The plugin keeps only
ephemeral protocol state, such as:

- open-path/interface identity;
- full-frame shadows needed for partial matrix writes;
- the most recent trustworthy brightness observation;
- per-process untested-warning suppression; and
- reconnect generations used to invalidate stale state.

Group and element mutations should update a complete shadow and coalesce into
the fewest safe matrix spans per batch. A shadow miss initializes every known
position before applying partial changes so stale firmware colours cannot leak
into the result. Unknown occupancy must not be silently initialized as though
it were a real LED.

Invalidate relevant shadows after firmware effects, reconnect, resume,
uncertain transport failure, storage-mode change, and any operation whose
matrix side effects are unknown. Reject a wrong frame size before opening
hardware. Define deterministic ordering between brightness, frame writes,
custom-mode activation, and firmware effects.

Firmware effects should use vendor hardware-effect descriptors with exact
discrete schemas unless a faithful portable-effect mapping exists. Software
effects implemented by OpenRazer's daemon, such as ripple, are not firmware
effects; Luminate may provide them through its own rendering and streaming
path.

## Transport investigation

OpenRazer's Linux kernel drivers send USB HID class control requests and often
read a response after a device-specific delay. The port must prove which of
these behaviours `hidapi` can express on each platform before promising broad
cross-platform support.

For the BlackWidow V4 Pro, first establish:

- the HID collection/interface used for lighting;
- whether `hidapi` feature-report calls reproduce report index `0x03` request
  and response traffic;
- whether the leading report-ID byte is part of the userspace buffer;
- the required delay and busy behaviour;
- the effect of OpenRazer or a generic kernel HID driver already binding the
  device; and
- whether normal input remains usable while the lighting path is open.

If `hidapi` cannot implement the required control transfers, stop and discuss
the transport architecture before adding a USB dependency or platform-specific
FFI. Do not silently reduce support to Linux or detach input drivers.

Profiles using old-device reports, 320-byte ARGB reports, receiver routing,
wireless transaction IDs, or transport-specific response paths should remain
separate variants with their own fixtures and test evidence.

## Import strategy

Port incrementally even though untested profiles are enabled by default:

### Phase 1: inventory and normalize upstream

- Extract a reviewable inventory of every OpenRazer lighting-capable VID:PID,
  category, matrix dimension, command family, transport parameters, and quirk.
- Exclude profiles whose OpenRazer capabilities are entirely outside lighting.
- Group devices by actual protocol behaviour rather than marketing family.
- Identify contradictory or incomplete upstream declarations and leave them
  out with a recorded reason instead of guessing.
- Record the upstream commit and an import manifest mapping each Luminate
  profile to its source locations.

Use a development-only extraction/checking script if it reduces transcription
errors, but generated output must not become an opaque primary source. Either
commit a readable reviewed manifest as source data or hand-maintain typed
profiles with a drift checker.

### Phase 2: shared report and transport core

- Implement report encoding, CRC, response validation, timing, response policy,
  and a mockable transport.
- Add upstream-derived packet fixtures for shared commands.
- Prove safe non-mutating identity/firmware queries on the BlackWidow V4 Pro.
- Document platform behaviour and halt if transport requires an unapproved
  dependency or architecture change.

### Phase 3: BlackWidow V4 Pro validated profile

- Implement PID `0x028d`, transaction ID `0x1f`, report/response index `0x03`,
  the known no-response custom-frame quirk, and the 8×23 matrix.
- Capture the HID descriptors and every relevant request/response on the local
  unit.
- Sweep every matrix coordinate and map US-ANSI keys, macro/media/command
  controls, side/underglow, and attached wrist-rest lighting.
- Validate off, static, spectrum, wave, wheel, reactive, breathing, starlight,
  brightness, custom frames, and volatile/persistent behaviour.
- Publish the detailed protocol and physical map in
  `docs/design/hardware/razer/blackwidow-v4-pro-rgb-hid-protocol-spec.md`.
- Mark this profile `Validated` only after the complete live checklist passes.

The agreed meaning of fully featured for this keyboard is all verified lighting:
every addressable element, command-dial lighting, side/underglow and wrist-rest
zones, brightness, firmware effects, streaming, safe persistence, discovery,
hotplug, packaging, tests, and documentation. Non-lighting keyboard settings
remain out of scope.

### Phase 4: first shared device families

- Import wired extended-matrix keyboards closest to the validated V4 Pro
  protocol.
- Import simpler single-zone and classic-matrix devices.
- Add mice and accessories only after their transport and topology models are
  represented cleanly.
- Add wireless/receiver profiles after routing and identity semantics have
  focused tests.
- Add ARGB and old-device report paths last unless available hardware moves
  them earlier.

Each family lands with registry validation, packet fixtures, topology tests,
packaging IDs, and untested warnings. A family may be split across focused
changes. Continue through all safely representable OpenRazer lighting profiles;
do not stop at the V4 Pro's nearest relatives merely because the shared core is
already demonstrated. Profiles requiring unresolved raw-USB, kernel-only,
receiver-routing, or unsafe identity behaviour remain documented deferrals
rather than guessed implementations.

### Phase 5: packaging and user reporting

- Generate or maintain narrow udev rules from the reviewed profile registry.
  Grant the Luminate daemon access only to known lighting control nodes, not all
  Razer devices or every interface on a composite device.
- Add plugin artifacts and udev assets to applicable Debian, RPM, and other
  package manifests and staging scripts.
- Document how users identify their profile status, collect sanitized logs,
  run opt-in reversible tests, and report results.
- Document the plugin's experimental compatibility policy and the
  `enable_untested_devices` opt-out.
- Ensure diagnostics never collect raw keyboard input reports and redact
  serial numbers by default.

### Phase 6: upstream refresh process

- Add a documented maintainer workflow that compares the pinned OpenRazer
  revision against a newer chosen revision.
- Produce a reviewable diff of new devices, capability changes, transport
  quirks, and removed support.
- Never automatically promote validation status or overwrite Luminate physical
  maps.
- Re-run licence and attribution review for changed upstream files.
- Treat profile removals, ID changes, topology changes, and capability changes
  as compatibility-sensitive Luminate changes.

## BlackWidow V4 Pro starting facts

- Product: original full-size US-ANSI Razer BlackWidow V4 Pro.
- USB identity: `1532:028d`.
- OpenRazer matrix dimensions: 8 rows by 23 columns.
- Extended-matrix transaction ID: `0x1f`.
- Request and response report indices: `0x03`.
- Timing class: BlackWidow Chroma wait interval.
- Common report: fixed 90 bytes with XOR CRC over the command body.
- Reported firmware capabilities: off, static, spectrum, wave, wheel, reactive,
  breathing, starlight, brightness read/write, custom-frame upload, and
  custom-mode activation.
- Wave and wheel direction values: `0x01` and `0x02`.
- Custom-frame upload and custom-mode activation deliberately use a no-response
  path for this model.

Attached wrist-rest ordering and concurrent input testing still require local
validation. The wrist rest is currently unavailable. The newer
selectable-profile protocol remains out of scope until it is sufficiently
decoded to model safely.

### Local USB enumeration captured on 2026-08-03

Read-only host enumeration confirmed the attached keyboard at
`Bus 003 Device 009: ID 1532:028d`. Bus and device numbers are ephemeral and
must not be used as identity or configuration.

The unit reports USB 2.0 at high speed, device revision `1.01`, one
configuration, five HID interfaces, and no USB descriptor serial (`iSerial` is
zero):

| Interface | Subclass | Protocol | Interrupt endpoint | Maximum packet | Report descriptor length |
| --------- | -------- | -------- | ------------------ | -------------- | ------------------------ |
| 0         | boot     | keyboard | `0x81`             | 8 bytes        | 61 bytes                 |
| 1         | none     | keyboard | `0x82`             | 22 bytes       | 177 bytes                |
| 2         | boot     | mouse    | `0x83`             | 8 bytes        | 88 bytes                 |
| 3         | none     | none     | `0x84`             | 8 bytes        | 22 bytes                 |
| 4         | none     | keyboard | `0x85`             | 2 bytes        | 348 bytes                |

Interface 3 is the leading lighting-control candidate because it is the only
non-input-protocol interface and matches OpenRazer's report/response index
`0x03`. This is evidence for investigation, not yet permission to assume the
mapping: capture its HID report descriptor and confirm a non-mutating protocol
query before sending lighting commands.

Because the USB descriptor has no serial, query the Razer protocol serial and
validate its stability before using it in a device ID. If that query is absent,
unsupported, or returns a non-unique placeholder, preserve the identity
uncertainty and document the multiple-identical-device limitation.

The host does not have OpenRazer installed and does not run OpenRGB or Synapse.
No competing lighting controller is expected during validation. Sandbox-level
USB access is unavailable, but explicit host-level read-only enumeration has
been approved and works. Obtain separate explicit access before the first
reversible protocol query or lighting write; do not treat descriptor access as
authorization for mutations.

## Testing strategy

### Shared deterministic tests

- exact request bytes and CRC for every ported command family;
- report data-length boundaries and reserved-byte handling;
- response status, length, CRC, transaction, and command correlation;
- required/optional/no-response command policy;
- bounded retry and timing selection;
- storage, LED target, speed, direction, colour-count, row, and span validation;
- profile registry uniqueness and internal consistency;
- precise mapping from profile capabilities to topology descriptors;
- validation-status notes and one-warning-per-unit behaviour;
- discovery rejection of unknown VID:PIDs and wrong HID collections;
- multiple-device identity and detach/reattach using fake enumeration;
- shadow initialization, batch coalescing, invalidation, and recovery;
- exact frame ordering and rejection before hardware access; and
- errors that preserve useful context without panics or unchecked indexing.

### Profile fixtures

Every imported profile family needs minimal, documented packet fixtures for
its claimed command shapes. Fixtures derived from captures must be sanitized
and legally redistributable. Tests should validate observable protocol
behaviour rather than reproduce OpenRazer's control flow.

### Hardware tests

Hardware tests remain ignored or separately invoked so normal workspace tests
stay hermetic. Each must declare the target VID:PID, expected visible effect,
whether the write is volatile, and how state is restored.

The BlackWidow V4 Pro live suite must cover:

- safe identity and firmware queries;
- every occupied coordinate and physical topology name;
- every firmware effect and argument boundary;
- brightness endpoints and readback after effects/custom mode;
- partial batches and complete frames;
- sustained frame rate and report latency;
- wrong-sized frames and malformed-operation rejection;
- clear/off and restoration;
- unplug during a write, reconnect, daemon restart, and suspend/resume;
- wrist rest attached and detached; and
- uninterrupted keyboard, macro, media, and command-dial input while loaded and
  while streaming.

Untested users should have a safe opt-in validation command or documented test
sequence. Recognition alone must not mark a profile validated; reports need the
device identity, plugin version, operations exercised, observable results, and
sanitized diagnostics.

## Required project verification

Run focused checks throughout development, then the full contribution workflow:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Also run:

- focused tests for `luminate-plugin-razer`;
- plugin-host conformance with and without attached Razer hardware;
- profile-registry and upstream-drift checks;
- relevant package staging and smoke tests for each changed format;
- repository licence/SPDX checks;
- the documented BlackWidow V4 Pro hardware suite; and
- the complete manual coordinate/effect checklist.

Report every unavailable platform, packaging, or hardware check with the exact
unrun command and requirement. Fake transport tests do not establish hardware
support.

## Compatibility, safety, and security

- New device, surface, group, element, and hardware-effect IDs are externally
  visible. Review naming before release; do not rename imported coordinate IDs
  casually after users have persisted state for them.
- The plugin is explicitly experimental, so its IDs, topology, effects,
  profiles, and settings are not yet compatibility commitments. Still update
  related configuration, persisted-state handling, tests, migrations, and
  documentation coherently when they change; experimental status is not a
  reason to create gratuitous churn.
- A generic plugin should not require a plugin ABI or public protocol version
  bump. Reassess if implementation needs a new capability or serialized type.
- Profiles may change from coordinate-only to physically mapped topology after
  validation. Define a migration/alias policy before making such an untested
  topology broadly available, because persisted coordinate targets may already
  exist.
- HID responses are untrusted. Validate all lengths and values before indexing
  or allocation.
- Never enumerate by Razer VID alone and then probe unknown products.
- Never log raw input reports. Select only the vendor lighting collection and
  redact trustworthy serials in ordinary diagnostics.
- Composite-device udev permissions can expose input interfaces. Rules must be
  constrained by known product and interface/usage evidence.
- Persistent storage writes may cause wear. Default to volatile storage and
  expose persistence only per profile after exact semantics are verified.
- Concurrent lighting controllers can race. Luminate owns hardware while
  active; document that Synapse, OpenRazer, OpenRGB, and similar lighting
  controllers should not control the same unit concurrently.
- An untested warning is not a safety waiver. A profile that lacks an exact
  transport match or bounded command schema must not be imported.

## Completion criteria

The initial generic Razer port is complete when:

- `luminate-plugin-razer` has no OpenRazer dependency and uses native Luminate
  transport, topology, capability, state, batching, and streaming paths;
- the upstream provenance and refresh workflow are documented;
- the shared report/command core has deterministic byte-level tests;
- imported profiles are exact, internally validated, enabled by default, and
  clearly marked with honest validation status;
- setting `enable_untested_devices = false` prevents all discovery and hardware
  access for untested profiles without hiding more strongly validated devices;
- untested profiles warn once per unit and ask for reports without flooding
  logs;
- discovery never writes to an unknown product or unrelated HID interface;
- the BlackWidow V4 Pro profile is completely mapped and live-validated without
  disturbing normal input;
- every capability advertised for that keyboard passes packet tests and a
  physical-device check;
- every OpenRazer lighting profile that the implemented transports and
  capability model can safely represent has been imported or has a specific
  documented blocker;
- packaging installs the plugin and narrowly scoped device permissions;
- relevant full-workspace, conformance, packaging, and hardware checks pass;
  and
- remaining family/platform uncertainty is documented precisely.

The broader port remains iterative after that milestone. A device becomes
`Validated` only through its own hardware evidence, even when it shares a
protocol family with the BlackWidow V4 Pro.

## Open questions requiring investigation

- Can `hidapi` reproduce every relevant OpenRazer HID class control transfer on
  Linux, Windows, and macOS, including non-zero report indices and 90-byte
  buffers?
- Which profiles require a kernel-driver-only behaviour, interface detachment,
  raw USB access, old-device reports, ARGB reports, or receiver routing that
  cannot initially be ported safely?
- Which HID usage/interface selectors can be encoded per profile without
  relying on unstable device paths?
- How many imported devices have matrix dimensions but no reliable occupancy or
  physical map?
- What migration or cleanup should accompany stable-ID and topology changes
  when an untested coordinate profile later receives a named physical map,
  given the plugin's experimental compatibility policy?
- Where should the stable reporting instructions live, and what sanitized
  diagnostic bundle is sufficient to promote a profile?

Answer transport behaviour before broad profile import. Define topology-change
cleanup before promoting the first coordinate-only profile to a named physical
map. The other questions can be resolved incrementally without weakening the
safety rules above.
