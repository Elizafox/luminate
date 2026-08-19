<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# State observation and reconciliation architecture

Status: accepted implementation design

This note records the design of the state observation and reconciliation
architecture for contributors. The core model and wire/plugin shapes are
implemented. Sections that describe future work say so explicitly.

> **Historical note.** An earlier revision of this design had a third
> desired-state layer above per-device overlays: standing per-*location*
> defaults. Locations were removed — persisted-state format v7 drops
> `location_overrides` and `location_defaults` — and user-created collections
> replaced location-based fan-out. Collections apply state to their members
> when asked and hold no standing default of their own; that is the settled
> design, not a missing piece. Desired state is therefore two layers: the
> adopted physical baseline beneath per-device overlays.

## Original problem

Before this design was implemented, Luminate persisted successful mutation
operations for startup replay while a target could advertise:

```rust
StateReadbackCapability::Readable {
    reliable,
    startup_authoritative,
}
```

The plugin ABI had no state-readback callback. The daemon interpreted
`startup_authoritative: true` only as permission to skip cached replay; it did
not read hardware or reconcile daemon state with hardware state. The current
facet/fidelity capability and snapshot callback replace those fields.

That design conflated three separate concepts:

1. Whether hardware can report its state.
2. What state Luminate wants the hardware to have.
3. Which side should win when the two differ.

A single global answer is inappropriate. A laptop keyboard is normally owned
by the local daemon and should restore the user's configuration after boot. A
household bulb may be shared with wall switches, vendor applications, and other
controllers; restarting Luminate should not visibly overwrite its current
state with an old cached value.

## Decision summary

The implemented design:

- models desired state, an adopted physical baseline, and observed hardware
  state as separate layers;
- makes a reconciliation decision when it gains or regains control of
  a device, but does not always perform a write;
- exposes the directional policies `restore`, `adopt`, and `leave`;
- makes `adopt` durably rebase each confirmed hardware facet into a distinct
  physical baseline beneath explicit desired overlays, rather than inserting
  ordinary `TargetStateEntry` values or keeping an ephemeral baseline;
- serializes generation reservation and every whole-state commit/save through a
  global coordinator so an older reconciliation snapshot cannot overwrite a
  newer persisted mutation;
- serializes every same-device hardware operation end-to-end, including client
  mutations, reconciliation reads, and verification, so no read can observe or
  publish a mutation's transitional hardware state;
- treats authority as configuration, not as a hardware capability;
- represents unknown state directly instead of presenting cached state as
  current hardware state;
- uses capability-scoped appearance, brightness, emission, and physical-power
  facets, with existing composite commands explicitly projected across them and
  facet replay defined over the existing command set (no new power operation);
- requires a callable, typed plugin readback path before a plugin advertises
  readback, with observations limited to canonical physical targets; and
- supports explicit runtime refresh because startup-only readback does not
  handle changes made by other controllers while the daemon is running; dirty
  notifications remain a future extension.

## Terminology and invariants

### Desired state

Desired state is explicit Luminate intent. The current ordered
`TargetStateEntry` collection is best understood as persisted desired overlays:
applying the entries in order reproduces appearance and brightness operations
across device, surface, element, and group scopes. Saved scenes are also
explicit higher-level intent, but capture composes this baseline and overlay
state into a separate sparse snapshot rather than changing reconciliation.

Desired state is not proof of current hardware state.

### Adopted physical baseline

The adopted physical baseline is a distinct durable layer containing confirmed
hardware facets promoted by `adopt`. It is keyed by canonical device, surface,
or element targets and sits beneath explicit desired state:

```text
adopted physical baseline
        -> device/surface/element desired overlays
```

`restore` resolves this composition using the existing desired-state authority
rules. An adopted baseline fills state for which no desired overlay applies; it
never overrides an explicit device mutation.

This layer must not be represented by an ordinary `TargetStateEntry`. Adopted
state is what hardware happened to be doing, and a desired overlay is what the
user asked for; storing the former as the latter would make the two
indistinguishable, so `restore` could no longer tell an explicit "set this red"
from a red it merely observed and adopted. Implementing adoption through
`set_effect`, `set_brightness`, `clear_target`, or their shared
insertion path would create exactly that accidental individual override.
Adoption therefore needs its own store and merge path — `DaemonState`'s
`adopted_baseline: Vec<AdoptedFacet>`, deliberately separate from
`target_state`.

### Observed state

Observed state is information about what hardware is currently rendering. It
has provenance, freshness, and a knowledge level. It may be partial: a device
might report power, brightness, and static colour while being unable to
reconstruct an active effect or its original parameters.

Observed state must not reuse `TargetState::Clear`. `Clear` is an instruction
to return control to a plugin default; it is not a physical state that can be
read from hardware.

### Required invariants

- A cached successful command must never be presented as confirmed hardware
  readback.
- Lack of a hardware readback mechanism is valid. A device or facet may remain
  `unknown` indefinitely; this is an honest knowledge state, not a daemon
  failure. After Luminate successfully writes a complete value it may become
  `assumed`, but only sufficiently faithful readback or a sound derivation from
  confirmed inputs may make it `confirmed`.
- Best-effort readback is knowledge, not `unknown`: retain it as
  `best_effort(value)` with readback provenance. It is neither an unverified
  Luminate write nor sufficiently faithful confirmation, so it cannot be
  promoted by `adopt`, satisfy exact verification, or silently supply a missing
  facet for an operation that requires current exact state.
- Unknown observed state must not erase desired state.
- A failed or timed-out refresh must not erase a prior observed value. It
  changes reachability/latest-attempt status and makes the retained value stale;
  a facet becomes `unknown` only when no usable earlier observation exists or
  that knowledge has been explicitly invalidated.
- A failed read must not silently cause a stale cached write under a
  hardware-preserving policy.
- Readback data must be validated against the active topology and advertised
  capabilities before it is committed.
- Partial readback must update only the facets it actually reports.
- A validated readback value may be published at its correct knowledge level
  before adoption persistence succeeds: sufficiently faithful values are
  `confirmed`, while lower-fidelity values are `best_effort`. Observation
  validity and adoption durability are independent statuses.
- Adoption is durable only after confirmed facets have been persisted into the
  separate adopted physical baseline.
- Reconciliation and client mutations for the same target must be ordered so a
  late startup operation cannot overwrite a newer client command. Every device
  has a mutation generation/epoch; reconciliation may publish or commit only if
  the generation is unchanged from the value captured when it began.
- Every operation that reads or changes one device's hardware joins the same
  per-device operation sequencer. A mutation holds it across validation,
  generation reservation, hardware apply, persisted commit, and observation
  update. Reconciliation holds it across generation capture, hardware read,
  observation publication, and any adoption commit. Generation checks remain
  necessary for topology/ownership invalidation, but cannot replace this
  in-flight-operation ordering.
- The generation check that gates observation publication is atomic with
  generation reservation: publication happens while holding the per-device
  sequencer and under the state-commit coordinator (or an observation lock
  ordered with it). A newer generation supersedes the older value as current;
  the older observation may remain as history but cannot be republished after a
  newer same-device operation.
- Generation validation and durable persistence form one ordered commit under a
  shared state-commit coordinator. No persisted mutation may commit between the
  final generation check and publication of the adopted snapshot, and an older
  reconciliation snapshot must never be renamed over a newer persisted state.
  Because the daemon writes one whole state snapshot, every operation that
  changes persisted state uses the same global coordinator even when devices do
  not overlap.
- Plugin readback may address device, surface, and element targets only. Groups
  (plugin-declared convenience collections) are selection/intent constructs, not
  physical observation identities; a plugin response containing one is
  malformed. User collections are daemon-side and cannot appear here at all,
  since `PluginTarget` has no variant able to name one.
- Plugin or device reappearance must use the same reconciliation path as
  initial daemon startup.

## Reconciliation policies

Do not expose a binary `sync` option: it does not say which direction data
flows. Use three explicit policies.

Every policy executes its hardware read/write phase under the affected
device's operation sequencer and retains it through the resulting observation
publication and persisted commit, if any. The individual policy steps below do
not repeat that rule except where `adopt` makes the ordering especially
important.

### `restore`

Desired Luminate state wins.

1. Resolve the effective state by composing the adopted physical baseline with
   the currently authoritative desired overlays.
2. Replay every resolved, actionable facet to hardware.
3. If no desired rule or adopted baseline provides any actionable state, read
   the hardware when possible; otherwise leave it untouched. With no earlier
   observation, its facets remain `unknown`. This read is observation-only and
   does not promote values into the adopted baseline under `restore`.
4. If readback is available, read after applying and verify the result.
5. Without readback, a successful plugin apply may produce an `assumed`
   observation, but never a `confirmed` one.
6. A failed apply reports reconciliation failure without erasing an earlier
   observation; retained values become stale when they can no longer be
   considered current.

If apply reports success but verification returns a different value, desired
state remains unchanged, the confirmed hardware value is published as the
observation, and reconciliation reports drift/failure. `Restore` must not
silently adopt a mismatching value.

This is the recommended default for host-attached, daemon-owned lighting such
as laptop keyboards and other peripherals. Users generally expect their local
profile to return after boot.

### `adopt`

Current hardware state wins.

1. Acquire the device's operation sequencer, wait for every earlier mutation or
   reconciliation operation to finish, and then capture the device generation.
2. Read the hardware while continuing to hold the device sequencer.
3. Do not replay stale desired state over it.
4. Validate the response envelope and every returned facet, classifying each
   usable value as `confirmed` or `best_effort` from its advertised fidelity.
5. Briefly acquire the global state-commit coordinator and, if the device
   generation is unchanged, publish each valid facet at that knowledge level.
   The check and publication are one atomic step with respect to generation
   reservation; then release the coordinator while retaining the device
   sequencer.
6. Build a per-device candidate delta from the `confirmed` facets only and tag
   it with the captured generation.
7. Acquire the global state-commit coordinator, recheck the generation, merge
   the delta into the latest transactional state snapshot, atomically persist
   that snapshot, publish the committed adopted baseline, and then release the
   coordinator. Only this ordered commit makes the rebase durable.
8. Track per-facet adoption status independently as pending, durable,
   ineligible because of insufficient fidelity, or degraded
   (`PersistenceFailed`). Retry a failed persistence operation while its
   captured generation remains current.
9. If the read fails, leave hardware untouched and do not fall back to a write.
   Preserve any previous last-known values as stale and report the failed
   attempt through reachability/read status; use `unknown` only for facets with
   no prior knowledge.
10. Release the device sequencer on every success or failure path only after
    publication/commit eligibility for this operation has been resolved.

This is the recommended default for shared smart-home devices such as network
bulbs. A daemon restart should not visibly change a room merely because an old
value exists in Luminate's cache.

Observation publication and durable adoption are separate transitions. A
hardware read remains a valid confirmed observation if writing the state file
fails. In that case the client may see `Confirmed(value)` together with
`adoption: PersistenceFailed(error)`. The durable adopted baseline remains the
previous version, the candidate is retained for a guarded retry, and a daemon
restart sees only the previously durable baseline.

Adoption is per facet. Facets absent from readback or reported only with
best-effort fidelity are not rebased. A failed or unavailable read attempt is
operation status and promotes no facets. Explicit Luminate intent remains in
the desired overlay layer and is not applied while `adopt` is in effect; it is
retained indefinitely rather than expired or rewritten, and the
policy-transition preview below is the deliberate safeguard against
resurrecting it unnoticed. For example, if hardware confirms blue but cannot
report brightness, blue replaces the adopted colour baseline while prior
desired brightness remains explicit but unconfirmed and the observed brightness
is `unknown` unless a prior observation exists.

A subsequent partial mutation may compose only with facets that are currently
confirmed or otherwise valid for that operation. A stale adopted baseline is not
silently treated as a current observation. If a plugin can set
brightness independently, unknown appearance is irrelevant. If the hardware
operation requires a complete state, the daemon or plugin must read the
missing facets or reject the mutation as unknown; it must not silently combine
confirmed adopted data with stale latent intent. A `best_effort` facet is
available for display and diagnostics but does not satisfy an exact missing
facet requirement.

Switching from `adopt` to `restore` resolves the adopted physical baseline
beneath all currently authoritative desired overlays. The
state API or policy-change tooling should preview that effective composition so
the transition cannot unexpectedly activate an invisible desired value.

Durable rebasing changes only the adopted physical baseline. It must not
rewrite saved profiles, scenes, or other explicit higher-level intent. Those
objects have their own authority semantics.

### `leave`

Do not change hardware and do not promote observed state into desired state.
Readback may still populate observations because a read is non-mutating, but a
read failure retains any prior last-known value as stale and marks the latest
attempt unavailable. The value is `unknown` only if no usable observation has
ever been established.

This is useful for safety-sensitive or advanced setups and as an honest
fallback for devices that cannot be read without being disturbed.

### Policy selection

Authority is user policy, not a physical capability. Consequently,
`startup_authoritative` should be removed or deprecated rather than moved into
a new readback capability unchanged. This has since happened: the field no
longer exists anywhere in the workspace, and the per-facet
`Exact`/`BestEffort` fidelity model replaced it.

The effective policy uses this precedence:

1. per-device user override;
2. plugin user override;
3. global user override;
4. plugin-recommended default; and
5. the daemon fallback: `leave`.

`effective_reconciliation_policy` (`daemon/reconciliation.rs`) resolves exactly
these five steps.

A plugin recommendation is useful because the plugin knows whether hardware is
normally host-attached and volatile or shared and independently controlled.
It remains a recommendation because users may operate the same hardware under
different ownership models.

The daemon fallback is deliberately `leave`: with no override and no
recommendation, the daemon neither writes nor promotes. Honest inaction beats
guessing wrong in either direction, and because in-tree plugins gain
recommendations in the same migration (step 8 below), the fallback fires only
for plugins that decline to recommend.

Suggested user-facing labels are:

- `restore`: "Restore my last Luminate state"
- `adopt`: "Use the device's current state"
- `leave`: "Leave the device unchanged"

If `adopt` is configured for a device without sufficiently faithful readback,
the daemon must not write. With best-effort readback it still reads and
publishes fresh `best_effort` observations; they remain available for display
but ineligible for adoption. With no readback at all it retains any prior
observation as stale and leaves never-observed facets unknown. Either way it
logs a clear diagnostic and must not silently reinterpret `adopt` as
`restore`.

## Observation model

Facet knowledge, read/reachability status, and adoption status are separate
axes. They must not be collapsed into one enum.

Facet knowledge is one of:

- `unknown`: no usable value has been established;
- `assumed(value)`: Luminate successfully sent a value but cannot verify it;
- `best_effort(value)`: a plugin readback reported a value, but its advertised
  fidelity is insufficient to treat it as exact current hardware state;
- `confirmed(value)`: validated, sufficiently faithful hardware readback, or a
  sound derivation from confirmed inputs, established the value at a recorded
  time.

A facet record should retain the value, source (`readback`, `successful_apply`,
or `derived` with its input provenance), and the time at which the knowledge
was established. Derivation must never increase confidence: a value derived
from `best_effort` input remains at most `best_effort`. Freshness is separate.
Best-effort values always retain readback provenance and their
fidelity/limitation metadata. After a later timeout, a previously confirmed or
best-effort value remains the last reported value, but it is stale rather than
current.

`best_effort` describes fidelity, not freshness. A freshly read best-effort
value is still not confirmed; an old confirmed value may be stale while
remaining the last value that was confirmed at its recorded time.

Device/read-operation status should independently represent:

- whether reconciliation is idle or in progress;
- latest read-attempt time and error;
- reachability, including unavailable; and
- reconciliation completion, drift, or failure.

Per-facet adoption status should independently represent at least pending,
durable, ineligible-fidelity, and persistence-failed/degraded states. A device
may aggregate these for display without discarding the facet detail. This
permits:

```text
facet knowledge:  Confirmed(red, confirmed_at=10:00)
latest attempt:   Failed(timeout, attempted_at=10:05)
reachability:     Unavailable
freshness:        Stale
adoption:         PersistenceFailed(...)
```

`reconciling` and `unavailable` are therefore not facet observation values. A
failed refresh changes operation/reachability status and freshness; it does not
erase the last-known facet value.

The durable adopted baseline should retain origin metadata such as
`adopted_readback` and the original confirmation time. A persisted adopted
value records that it was confirmed at a particular time; it starts as stale
last-known knowledge rather than a current confirmed observation after restart.

The model uses the capability-scoped facets defined below. A device may confirm
one facet while leaving another unknown.

Client APIs should expose observed state separately from desired/replay state.
Most user interfaces will display observed state as the current value and use
desired state and provenance for diagnostics, pending operations, or explicit
controller-oriented views.

Aggregate appearance is computed bottom-up from canonical constituent
observations. Elements feed surfaces; surfaces and elements feed groups;
surfaces feed devices; concrete targets and nested collections feed
collections. Equal known constituents produce their shared appearance.
Different known constituents produce `Mixed`. When the known values do not
prove a difference but some relevant constituent is unknown, the aggregate
remains unknown.

Successful writes flow in the other direction. A device, surface, or group
write updates the daemon's assumptions for every covered canonical descendant,
so a later element write can recompute its ancestors without leaving a stale
aggregate observation behind. Plugin snapshots remain canonical: plugins do
not report `Mixed`, and synthesized aggregate values are neither adopted nor
replayed to hardware.

### Facet schema decision

The observation and adopted-baseline schema contains four logical facets that
plugins report, plus one the daemon derives:

- `appearance`: a static colour or active effect description. This describes
  what the target is *configured* to show, independent of whether it is
  currently visible: an off light still reports its last configured colour or
  effect, the same way a dimmer set to 50% while off still reports brightness
  50 rather than 0. `Effect::Off` is not an observed appearance; it remains a
  composite command described below, and a plugin acting on it updates
  `emission`, not `appearance`.
- `brightness`: independently advertised brightness state, configured the same
  way as `appearance` above.
- `emission`: whether the target's lighting output is logically emitting or
  intentionally dark. This is the instantaneous on/off fact, and the sole
  authority for it — `appearance` and `brightness` never encode it. It
  describes target-local rendered output, not an electrical power switch and
  not instantaneous photometry during a dynamic effect. Prefer deriving it
  from a genuine power/enable signal when a target has one; when it doesn't, a
  target that is only ever "set to a colour" may derive `dark` from the
  configured colour being black, since "is this on or off" is worth answering
  even at the cost of conflating "off" with "deliberately configured to
  black." Skip advertising `emission` only for a target with no meaningful
  on/off answer at all.
- `physical_power`: whether an independently controllable/readable physical
  power domain is on or off.
- `effective_appearance` (daemon-derived, read-only): what a target actually
  looks like right now, combining `appearance` and `emission` — `off` when
  `emission=dark`, otherwise the target's `appearance`. This exists so
  consumers have one obvious facet to read for a colour swatch instead of
  joining `appearance` and `emission` themselves, which is what let a GUI
  bug show a light as "on and red" when it was actually off. Plugins never
  report it; the daemon synthesizes it per target from that target's
  `appearance` and `emission` observations, with confidence and freshness
  following the weaker of the two inputs. It is not part of desired state or
  adoption and does not appear in the command-projection table below.

Every facet is capability-scoped. `physical_power` exists only on canonical
targets where the hardware independently supports it, commonly a whole device.
It must not be synthesized on every surface or element. A surface or element
may expose `emission` without exposing `physical_power`; a LIFX zone can be
dark while its bulb's device-level power remains on. Unsupported facet absence
is different from supported-but-`unknown` knowledge.

Topology/capability metadata must identify the owning physical-power domain for
a target when commands at that target require a broader target to be powered.
For example, a LIFX zone element maps to the bulb device's physical-power
domain. A subtarget observation never reports that ancestor power value as if
it belonged to the subtarget.

Existing commands are composite intent and project onto facets in operation
order:

| Command                                                                        | Facet projection                                                                                                                                                                                                        |
| ------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Whole-target `Effect::Off` where that target owns independent physical power   | Set `physical_power=off` and derive `emission=dark`; do not invent a new appearance or brightness value.                                                                                                                |
| Subtarget `Effect::Off`, or off on a target without independent physical power | Set target-local `emission=dark` using the plugin's supported mechanism. If the hardware requires its owning power domain on to represent subtarget darkness, that broader `physical_power=on` side effect is explicit. |
| Static `SetEffect`                                                             | Set `appearance=static(colour)`, require the owning physical-power domain on, and derive emission as dark for a zero-output colour or emitting otherwise.                                                               |
| Animated or hardware `SetEffect`                                               | Set `appearance=effect`, require the owning physical-power domain on, and set logical emission to emitting even if an animation has momentary dark phases.                                                              |
| `SetBrightness`                                                                | Set `brightness`, require the owning physical-power domain on, derive `emission=dark` at zero, and otherwise preserve or derive emission from appearance rather than assuming nonzero brightness alone emits light.     |
| `Clear`                                                                        | Remains a reset command, not an observation value. Its plugin-defined result invalidates affected facet knowledge until known defaults or readback establish new values.                                                |

These mappings define semantic effects, including cross-scope power effects;
they do not require the daemon to pretend every device implements them through
the same hardware transaction. A successful unverified operation records the
projected facets as `assumed`. Readback may later confirm or disagree with each
facet independently.

The existing `TargetState::facet()`/`TargetStateFacet` compaction cannot remain
the sole projection model: it classifies `Effect::Off` with appearance and
models brightness and appearance as the only independent facets. Command
validation, ordered compaction/replay, observation comparison, and restore
composition must all use the new multi-facet projection while preserving
operation order and broader/narrower target coverage.

Replay of a resolved facet composition (the `restore` path) uses the existing
command set only; this design adds no power operation to the plugin ABI. The
inverse projection is:

- `appearance=static(colour)` replays as static `SetEffect`; `appearance=effect`
  replays as the corresponding `SetEffect` only when the stored description is
  complete enough to reissue the command — an adopted appearance that cannot
  be reissued is not actionable;
- `brightness` replays as `SetBrightness`;
- `physical_power=off` on a target owning an independent power domain replays
  as whole-target `Effect::Off`;
- `emission=dark` on a subtarget replays as subtarget `Effect::Off`; and
- `physical_power=on` or `emission=emitting` alone is not actionable: power-on
  is only ever the explicit side effect of replaying a colour, effect, or
  brightness command.

The consumer-facing `restore_appearance(target)` and
`restore_appearance_selector(selector)` helpers are a smaller, on-demand form
of appearance replay. They use the retained `Appearance` observation even when
that observation is stale, then reissue the ordinary colour or non-off effect
mutation. They do not infer an appearance from `Emission`,
`EffectiveAppearance`, desired-state overlays, or hardware power. An unknown
appearance therefore returns `UnknownState` without touching hardware.

For a group, the daemon first expands canonical members and resolves and
validates every member. Identical appearances may be sent once to the group
when its advertised capability can represent them faithfully. Heterogeneous
appearances, or uniform appearances unsupported at group scope, are applied
member by member. A failure after earlier member writes reports
`PartialMutation`; preflight failures produce no writes.

For a collection, the daemon first performs best-effort authorization per leaf,
then preflights every authorized leaf before writing. Each leaf retains its own
appearance; a collection does not establish a uniform appearance. Successful
requests return the authorized and denied leaves in `CollectionApplied`.

Power-affecting darkness replays last: the command replaying a resolved
`physical_power=off` or `emission=dark` is emitted after appearance/brightness
commands so their implicit power-on side effects cannot undo it. A plugin that
implements batch apply may coalesce the sequence to avoid visible intermediate
states. A composition that resolves only non-actionable facets replays nothing.

Derived values must retain derivation provenance. For example, confirmed
device-level `physical_power=off` can imply descendant `emission=dark` without
pretending each descendant was independently read. Conversely,
`physical_power=on` does not imply that any zone emits light.

This facet decision is a prerequisite for implementing the plugin readback
payload, consumer state protocol, command projection, or durable adopted
baseline. Exact Rust representation may vary, but it must preserve these scope
and composition semantics.

## Plugin API direction

Add an optional typed state-snapshot operation to the plugin path. The native
ABI carries validated, length-delimited CBOR, as topology and mutations do today.
Conceptually:

```text
read_state(requested targets/facets) -> observations or per-target errors
```

A bulk operation is preferable to forcing one hardware transaction per target.
The plugin must remain free to batch network or bus requests efficiently. The
response must support partial success because one device in a plugin may be
offline while others are available.

Plugin readback uses canonical physical observation targets only:

- device;
- surface; and
- element.

Groups are not valid plugin observation targets: they overlap, and they are
plugin-declared convenience collections rather than anything that uniquely
identifies a physical readback scope. Group desired state must be expanded or
projected by the daemon onto canonical physical targets before comparison with
observation. A plugin response containing a group target is malformed and the
affected snapshot must be rejected.

The same reasoning covers user collections, which are daemon-side selection
constructs. They are unreachable here in practice, because `PluginTarget`
cannot express one in the first place.

Capability metadata should describe hardware facts, for example:

- which facets can be read;
- whether the result represents rendered state exactly or only best effort;
- whether reads are expensive or rate-limited;
- whether performing a read visibly disturbs rendered state (hardware like this
  should normally advertise no readback; the flag lets `leave` and on-demand
  refresh avoid disturbing reads the user did not ask for); and
- whether the plugin can notify Luminate when externally caused state changes
  occur.

The implemented per-facet `Exact`/`BestEffort` fidelity avoids a single coarse
reliability bit. Policy such as "hardware wins" is not embedded in this
metadata.

Plugin-produced observations cross a trust boundary and require the same
defensive treatment as topology: size bounds, schema validation, target
ownership checks, capability validation, and rejection without partial daemon
state corruption when malformed. Validation also rejects observation targets
outside device/surface/element scope.

## Reconciliation lifecycle

Reconciliation is not merely a one-time daemon startup task. It should run when
Luminate gains or regains control:

- daemon startup;
- plugin-host startup or restart;
- device hotplug;
- network device discovery or reappearance; and
- resume from suspend when hardware state may have changed.

The resume trigger is implemented, and drives the same per-device path as the
others rather than a parallel one; suspend additionally marks every observation
stale, since hardware may be power-cycled or reset while the machine is down.
See [`suspend-resume.md`](suspend-resume.md).

Conceptually, each affected device has independent state transitions:

```text
reconciliation:  idle -> reconciling -> complete | drifted | failed
reachability:    unknown -> reachable | unavailable
facet knowledge: unknown -> assumed(value) | best_effort(value) | confirmed(value)
adoption:        pending -> durable | ineligible-fidelity | persistence-failed
```

For `restore`, reconciliation applies the desired replay and optionally verifies
it. For `adopt`, it performs readback and no stale write. For `leave`, it avoids
writes and may perform observation only.

Reconciliation must have bounded timeouts. An offline network bulb must not
prevent the entire daemon from becoming usable. State may be exposed as
`reconciling` while work continues.

Every device carries both an operation sequencer and a mutation
generation/epoch. Client mutations, reconciliation reads, on-demand refreshes,
and post-write verification all acquire the same sequencer before hardware I/O.
A reconciliation read therefore begins only after any earlier mutation has
finished its hardware and commit phases; a mutation arriving during a read waits
or safely cancels/invalidates that read before it may touch hardware. The two
operations must never execute concurrently against the same device.

After acquiring the device sequencer, a client mutation reserves its new
generation under the state-commit coordinator, releases the coordinator, and
performs hardware I/O while retaining the device sequencer. Reconciliation
captures the current generation only after acquiring that same sequencer. This
prevents a read from starting inside the interval between a mutation's
generation reservation and its completed commit.

Hardware reads and writes remain outside the coordinator. After I/O, every
operation that changes persisted state acquires the same global coordinator
while still holding its device sequencer, rechecks its device generation,
applies its delta to the latest in-memory transactional snapshot, atomically
saves that whole snapshot, publishes the committed in-memory state, and
releases the coordinator. A generation mismatch makes the result stale:
discard it and reschedule if policy still requires reconciliation.

The lock order is device sequencer first, global state-commit coordinator
second; code must never wait for a device sequencer while holding the global
coordinator. A multi-device operation acquires device sequencers in stable
device-ID order or is decomposed into independently sequenced per-device work.
This prevents deadlock while preserving hardware concurrency across unrelated
devices.

A slow reconciliation read can delay a same-device command, so every read is
bounded. An implementation may give queued user mutations priority or cancel a
cancel-safe reconciliation read, but the mutation may proceed only after the
read has stopped touching hardware and has lost publication/commit eligibility.
Responsiveness must not reintroduce concurrent same-device read/write I/O.

The coordinator is global because persistence is one whole state file. Two
non-overlapping per-device saves can still clobber each other if an older
snapshot completes its rename last. Per-device serialization and generation
checks remain useful, but neither replaces globally ordered commit-and-save.

The existing topology-reconciliation path should trigger state reconciliation
for newly appearing devices rather than introducing a separate special-case
startup implementation.

## Runtime consistency

Startup readback alone is insufficient for devices that can be changed by wall
switches, vendor applications, firmware, or other controllers.

The eventual architecture should provide:

- an explicit client refresh operation;
- a plugin-to-daemon state-dirty notification analogous to topology dirty-bit
  notification; and
- optional plugin polling where hardware cannot notify changes.

The notification should be a dirty bit, not a trusted state delta. On receipt,
the daemon calls the same validated snapshot path and clients refresh the
authoritative state API. This matches the existing topology-event philosophy.

Runtime observation is especially important for `adopt` devices. Until it is
implemented, documentation must make clear that observations are startup or
on-demand snapshots rather than continuously synchronized truth.

## Persistence implications

Persist explicit desired state as today and persist the adopted physical
baseline as a separate layer. A confirmed observation selected by `adopt` is
published after validation and generation checking; its validity does not
depend on a state-file write.

Durable adoption stores a per-device candidate delta with its captured
generation, not a prebuilt whole-state snapshot. It then uses the same global
state-commit coordinator and atomic persistence guarantees as every successful
mutation. Under the coordinator it rechecks the generation, merges the delta
into the latest snapshot, saves that snapshot, and only then replaces the
active adopted baseline and moves adoption status to `durable`. If persistence
fails:

- the confirmed runtime observation remains published;
- adoption becomes `PersistenceFailed` or otherwise degraded;
- the active durable baseline remains the previous version;
- the guarded delta is retained for retry while its device generation is still
  current, and retry merges it into a fresh latest snapshot under the same
  coordinator rather than reusing an old whole-state image; and
- a daemon restart sees only the previously durable baseline.

Persisting last-known observations beyond the adopted baseline may be useful
for diagnostics or an initial UI hint, but every value must retain its source
and timestamp and be labelled stale after restart. Persistence proves only
that a value was confirmed or assumed at an earlier time, not that it is
current now.

Only confirmed, sufficiently faithful facets may be promoted. Unknown facets
and `best_effort` values do not replace the durable adopted baseline. An
unavailable read attempt is operation status, not a value; it leaves the
existing adopted baseline and last-known observation intact. Repeated identical
reads should not rewrite the state file; runtime external-change updates should
be compared and debounced so adoption does not create needless persistence
churn.

`PersistenceCapability` is separate from state observation:

- device persistence describes what the hardware stores across its own power
  cycles;
- desired-state persistence describes what the daemon stores;
- adopted-baseline persistence stores confirmed physical facets below explicit
  desired intent without claiming they remain current;
- state readback describes what the plugin can observe; and
- reconciliation policy decides which source is acted upon.

A write-through persistent device may avoid redundant startup writes, but that
does not prove its current state. If it has no readback, the daemon has at most
an assumed last-known value.

## Expected device behaviour

### Laptop or locally attached lighting

Default to `restore`. Apply the complete cached profile after boot, plugin-host
restart, reattachment, or resume as appropriate. Verify through readback where
available. If hardware has no readback, expose successful application as
assumed; truthful confirmation is impossible without hardware support.

### Shared household bulb

Default to `adopt`. Read current state without visibly changing the bulb. If
the bulb is offline, report unavailable reachability while retaining any
last-known values as stale; facets with no earlier value remain unknown. Do not
push old desired state when it later appears unless the selected policy requires
that behaviour. A user who treats Luminate as the only controller may override
the policy to `restore`.

### Persistent but unreadable hardware

Do not claim knowledge merely because the last command was durable. The
effective policy decides whether Luminate replays desired state. Otherwise,
leave hardware untouched and expose an assumed or unknown value with its
provenance.

## Implementation map

The implementation followed these layers; this order remains useful when
reviewing or extending the feature:

1. Implement the decided capability-scoped appearance, brightness, emission,
   and physical-power facets plus command projection and power-domain metadata.
2. Introduce separate observed-state, adopted-baseline, and explicit desired
   layers without changing existing replay behaviour.
3. Introduce independent facet-knowledge, reachability/read, reconciliation,
   and adoption status plus per-device mutation generations, per-device
   end-to-end operation sequencers shared by mutations and reconciliation, and
   one global state-commit/persistence coordinator shared by every persisted
   mutation.
4. Add the canonical-target plugin snapshot callback and validation.
5. Add a client state-query API.
6. Add explicit `restore`, `adopt`, and `leave` policies.
7. Add transactional, per-facet durable rebasing for confirmed `adopt`
   observations, including provenance, persistence-failure status, and
   stale-after-restart handling.
8. Migrate plugin recommendations: laptop/peripheral plugins toward `restore`,
   shared smart-light plugins toward `adopt`.
9. Run the same reconciliation on dynamic device appearance and plugin-host
   restart.
10. Add explicit runtime refresh. Dirty-bit plugin notifications remain a
    future extension and must feed the same validation/sequencing path.

The plugin ABI is intentionally lockstep and unstable, so adding the callback
does not require preserving compatibility with independently versioned plugin
builds. The consumer protocol and C API do require deliberate versioning and
additive API design.

## Acceptance scenarios

Implementation is incomplete unless tests cover at least:

1. A laptop light with cached state starts under `restore`; replay succeeds and
   its state becomes assumed or confirmed according to readback support.
2. A shared bulb is changed externally while the daemon is stopped; `adopt`
   reads the new value, durably updates the adopted physical baseline, and never
   flashes the cached desired value.
3. Readback times out under `adopt`; hardware is not written, reachability is
   unavailable, a previous confirmed value remains as stale last-known state,
   and only never-observed facets remain unknown.
4. A partially readable device reports brightness but not its active effect;
   brightness becomes confirmed and is durably rebased into the adopted
   physical layer while appearance remains unknown and its prior desired intent
   is not applied under `adopt`.
5. A malformed or foreign-target plugin snapshot is rejected without changing
   observed state, the adopted baseline, or desired state.
6. A plugin snapshot containing a group observation target is rejected as
   malformed; group desired state is projected onto canonical physical targets
   by the daemon.
7. A client mutation races startup reconciliation. If reconciliation owns the
   device sequencer first, the mutation waits and then supersedes its result; if
   the mutation owns it first, reconciliation waits and reads the post-mutation
   hardware state. No late read, adoption commit, verification, or replay can
   cross the same-device operation boundary.
8. A device disappears and later returns; the selected reconciliation policy
   is applied on reappearance.
9. A write-through persistent device without readback is not described as
   confirmed merely because cached state exists.
10. Changing a device from `adopt` to `restore` composes the adopted physical
    baseline beneath explicit desired overlays, with the effective transition
    visible before application.
11. Adoption never writes an ordinary `TargetStateEntry`: an adopted facet
    lands in `adopted_baseline` only, leaving the device with no individual
    desired override it did not already have.
12. Broad and narrow desired overlays project onto canonical observation
    targets without corrupting desired-state ordering.
13. Persistence fails after successful readback; the confirmed observation is
    visible with degraded adoption status, the old durable baseline remains
    active, and restart exposes only that old baseline.
14. An adopted facet persisted on a prior run starts as stale/last-known, not
    currently confirmed, until new readback succeeds.
15. `restore` with no desired rule or adopted baseline reads when possible and
    otherwise leaves hardware untouched without inventing state.
16. `restore` apply succeeds but verification disagrees; desired state is
    retained, the confirmed hardware value is published, and reconciliation
    reports drift/failure.
17. A plugin returns a valid but insufficiently faithful value; the facet is
    published as `best_effort` with readback provenance and freshness, remains
    ineligible for durable adoption or exact verification with an explicit
    insufficient-fidelity adoption status, and is not misclassified as
    `unknown`, `assumed`, or `confirmed`.
18. A client mutation arrives after readback validation while adoption
    persistence is pending. The reconciliation operation retains the device
    sequencer through its ordered commit, so the mutation cannot reserve a
    generation or touch hardware until adoption releases it. The mutation then
    persists the newer snapshot; stale adoption can never be renamed over that
    committed mutation, and restart after both commits observes the mutation.
19. Whole-bulb `Effect::Off` on a target with independent power records
    device-level `physical_power=off` and derived dark emission without
    inventing or erasing an observed appearance/brightness value.
20. LIFX zone `Effect::Off` records zone-local dark emission and the explicit
    device-level `physical_power=on` side effect; it never invents a zone-level
    physical-power facet.
21. `SetBrightness(0)` may leave the physical power domain on while deriving
    dark emission. Nonzero brightness with unknown/dark appearance does not by
    itself become confirmed emitting output.
22. Unsupported physical power at a target is represented by absent capability,
    not by an `unknown` physical-power observation.
23. Two concurrent client mutations target the same device; hardware apply
    order and commit order agree, and the final desired state and `assumed`
    observations match the last hardware write.
24. A client mutation reserves generation N and begins slow hardware I/O.
    Reconciliation for that device waits on the operation sequencer, captures
    the generation only after the mutation commits, and reads the resulting
    hardware state. It cannot publish a pre-mutation or transitional read under
    generation N.

## Resolved implementation choices

- Startup reconciliation completes before the daemon begins accepting general
  client work. Reappearing devices reconcile under the same per-device
  sequencer before their topology event is published.
- Policy precedence is device, plugin configuration, global configuration,
  plugin recommendation, then `Leave`.
- The consumer wire exposes value, source, confidence, timestamp, freshness,
  reachability, reconciliation/adoption status, and the latest diagnostic.
  Rust returns typed state; the C API returns the same state through an owned
  opaque snapshot with borrowed, length-delimited accessors.

These choices preserve the central separation:
capability states what hardware can do, observation states what is known,
the adopted physical baseline records durably promoted confirmed hardware
facets, desired state records explicit Luminate intent, and policy decides how
the layers compose. `Adopt` must not collapse these models into ordinary desired
state entries.
