<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# D-Bus consumer guide

The optional `luminate-dbus` companion presents the local Luminate daemon as a
native D-Bus service. Its well-known name is `org.luminate.Luminate1`, and its
root object is `/org/luminate/Luminate1`. It normally runs on the system bus;
the integration suite uses an isolated session bus.

The bridge does not replace daemon authorization. It first checks the calling
process against its configured group or optional Polkit action, then opens a
short-lived actor-bound daemon session. The daemon independently applies its
policy to every operation. Read-only inspection still requires the applicable
daemon permission. Mutating and administrative methods also require the
bridge's local authorization check.

## Discovery and interface versions

The root implements `org.freedesktop.DBus.ObjectManager`,
`org.luminate.Luminate1.Manager1`, and `org.luminate.Luminate1.Manager2`.
Targets beneath `/org/luminate/Luminate1/devices` implement:

- `org.luminate.Target2`, the retained legacy control and partial state view;
- `org.luminate.Target3`, complete capabilities, structured state, and newer
  direct controls;
- the legacy type marker (`Device1`, `Surface1`, `Group1`, or `Element1`);
- its complete versioned type interface (`Device2`, `Surface2`, `Group2`, or
  `Element2`).

Interfaces are additive and versioned. Consumers should inspect the interfaces
present on each object and use the highest version they understand. Existing
members are not silently retyped. The exact current member signatures are
protected by `crates/luminate-dbus/testdata/introspection-contract.xml`.

```sh
busctl --system tree org.luminate.Luminate1
busctl --system introspect org.luminate.Luminate1 /org/luminate/Luminate1
gdbus introspect --system \
  --dest org.luminate.Luminate1 \
  --object-path /org/luminate/Luminate1
```

ObjectManager is the authoritative topology baseline. A topology event is
emitted only after the managed-object tree has been updated. Child objects are
removed before their parents.

## Identifiers and native values

Method arguments use canonical target IDs:

```text
device:<device>
device:<device>/surface:<surface>
device:<device>/surface:<surface>/element:<element>
device:<device>/group:<group>
```

Object-path components are an implementation-safe encoding of these IDs; do
not construct paths by guessing that encoding. Discover paths with
ObjectManager or use the `Path` field in a device snapshot.

Structured models use `a{sv}` dictionaries with PascalCase keys. Variant
selection is explicit through `Kind`; optional fields are accompanied by a
`Has...` Boolean where absence would otherwise be ambiguous. Input dictionaries
reject unknown keys, missing variant fields, incompatible optional fields,
unknown enum strings, invalid identifiers, and invalid numeric values.

The principal schemas are:

- device snapshots: `Device` plus an ordered `Targets` array; every target has
  `Path`, `Identifier`, `Type`, `Name`, `Capabilities`, and `Topology`;
- selectors: `Kind=target` with `Target`, `Kind=collection` with `Collection`,
  or `Kind=targets` with `Targets`;
- outcomes: ordered canonical `Applied` and `Denied` arrays;
- state: observations with target, facet, typed value, confidence, source,
  timestamp and staleness, plus reachability, reconciliation, adoption, and
  optional latest diagnostics;
- transition status: `Id`, `Targets`, `ElapsedMs`, `DurationMs`, `HasOutcome`,
  and an optional typed `Outcome`;
- frame upload: generation, sequence, `Kind=full` with `Colours` or
  `Kind=partial` with indexed `Pixels`, and `Commit`;
- setup sessions: identity, plugin, workflow, generation, and a typed `State`;
- session information: authenticated subject, verified groups, sanitized
  authentication source, non-secret credential ID, and optional expiry.

Capabilities and topology retain the complete libluminate model, including
scope, physical tags, geometry, hierarchy, frame metadata, hardware-effect
descriptors, appearance slots, persistence, power domains, and readback.
`Device2.PhysicalTags`, `Surface2.PhysicalTags`, and `Element2.PhysicalTags`
retain provider order at their respective scopes. Tags are not inherited.
The retained `Element1` marker remains unchanged.

## Inspection and control examples

Check bridge/daemon availability and list complete device snapshots:

```sh
busctl --system call org.luminate.Luminate1 /org/luminate/Luminate1 \
  org.luminate.Luminate1.Manager2 Ping
busctl --system call org.luminate.Luminate1 /org/luminate/Luminate1 \
  org.luminate.Luminate1.Manager2 ListDevices
```

Read one device and its structured state:

```sh
gdbus call --system --dest org.luminate.Luminate1 \
  --object-path /org/luminate/Luminate1 \
  --method org.luminate.Luminate1.Manager2.GetDevice demo-keyboard
gdbus call --system --dest org.luminate.Luminate1 \
  --object-path /org/luminate/Luminate1 \
  --method org.luminate.Luminate1.Manager2.GetDeviceState demo-keyboard
```

Set brightness through a selector and inspect the applied/denied outcome:

```sh
gdbus call --system --dest org.luminate.Luminate1 \
  --object-path /org/luminate/Luminate1 \
  --method org.luminate.Luminate1.Manager2.SetBrightnessSelector \
  "{'Kind': <'target'>, 'Target': <'device:demo-keyboard'>}" \
  80 false ''
```

Create and inspect a collection:

```sh
gdbus call --system --dest org.luminate.Luminate1 \
  --object-path /org/luminate/Luminate1 \
  --method org.luminate.Luminate1.Manager2.CreateCollection \
  "{'Name': <'Desk'>, 'Members': <[{'Kind': <'target'>, 'Target': <'device:demo-keyboard'>}]>}"
gdbus call --system --dest org.luminate.Luminate1 \
  --object-path /org/luminate/Luminate1 \
  --method org.luminate.Luminate1.Manager2.ListCollections
```

List scenes and inspect the complete management view:

```sh
gdbus call --system --dest org.luminate.Luminate1 \
  --object-path /org/luminate/Luminate1 \
  --method org.luminate.Luminate1.Manager1.ListScenes
gdbus call --system --dest org.luminate.Luminate1 \
  --object-path /org/luminate/Luminate1 \
  --method org.luminate.Luminate1.Manager1.GetManagement
```

Create a short transition to an explicit state. The returned `Id` can be
passed to `GetTransition`, `WaitTransition`, or `AbortTransition`:

```sh
gdbus call --system --dest org.luminate.Luminate1 \
  --object-path /org/luminate/Luminate1 \
  --method org.luminate.Luminate1.Manager2.CreateCurrentToStatesTransition \
  "[{'Target': <'device:demo-keyboard'>, 'Brightness': <uint32 40>}]" \
  "{'DurationMs': <uint64 250>, 'Function': <'linear'>, 'ColourInterpolation': <'encoded'>}"
gdbus call --system --dest org.luminate.Luminate1 \
  --object-path /org/luminate/Luminate1 \
  --method org.luminate.Luminate1.Manager2.GetTransition TRANSITION_ID
```

Transition, frame, setup, scene, and management requests can contain nested
native dictionaries. Use introspection for the outer signature and the schemas
above for each dictionary. `Manager1` retains the typed scene and management
methods; `Manager2` owns complete topology, control, transition, frame, setup,
and attestation operations.

## Events and refetch rules

`Manager2` emits `TopologyChanged`, `StateChanged`, `TransitionsChanged`,
`ShmStreamEnded`, `ConfigurationChanged`, and `ScenesChanged`. Dirty signals do
not contain authoritative snapshots. Refetch after receiving them:

- topology: ObjectManager or `ListDevices`;
- state: `GetDeviceState`, `Target3.State`, or `GetCollectionState`;
- transitions: `GetTransition`;
- configuration: `Manager1.GetManagement`;
- scenes: `Manager1.ListScenes`.

An empty topology, state, or transition ID array is a full-refresh marker.
Subscribe before fetching the baseline, then process signals queued during the
fetch. `ShmStreamEnded` carries the canonical target and generation even though
shared-memory negotiation itself is not exposed through D-Bus.

If the daemon event subscription falls behind, the companion refetches the
complete topology and emits empty `TopologyChanged` and `StateChanged` arrays.

## Errors and sensitive values

Stable error names use the `org.luminate.Error.*` namespace. Most carry a
human-readable diagnostic which must not be parsed. `RateLimited` additionally
carries retry presence and delay; `PartialMutation` carries canonical targets
already committed. Capability rejection, stale generations/revisions,
not-found resources, authorization failures, and daemon unavailability remain
distinct errors.

Sensitive settings are accepted only in authorized mutation requests and are
redacted from snapshots, signals, and diagnostics. Token and attestation
creation return a display-once secret as `ay`; list methods return metadata
only. `SessionInformation` never returns a credential or reusable
authentication material.

## Deliberate transport exceptions

D-Bus activation and sender identity replace libluminate's socket paths,
builders, event tickets, and subscription-socket methods. Shared-memory stream
negotiation remains local to libluminate because its handles and lifetime are
tied to one daemon connection. Use ordinary `BeginFrameStream`, `UploadFrame`,
and `EndFrameStream` through D-Bus, or use libluminate directly for the
shared-memory fast path.

The checked operation-by-operation inventory is the
[D-Bus parity matrix](dbus-parity.md).
