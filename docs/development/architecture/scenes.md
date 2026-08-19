<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Daemon-managed scenes

Scenes are persistent, owner-controlled sparse snapshots of intended lighting
state. They store configured appearance, brightness, and emission without
copying observations, reconciliation status, physical-power guesses, or frame
stream contents.

Each scene has a UUID, an optimistic revision beginning at one, a name,
optional description, owner, and ordered bindings. Names are presentation;
identifiers are authoritative and names need not be unique.

## Bindings

A frozen binding always addresses its captured concrete target. A dynamic
collection-member binding records a collection ID, one concrete target that
was a member at capture time, and that target's individual state. Application
uses the intersection of captured targets and current transitive membership.
Members added later receive no invented state, and removed members are left
untouched.

Bindings must resolve and validate when a scene is created or replaced.
Withdrawing a device later does not delete an otherwise valid scene. A
collection referenced by a dynamic binding cannot be deleted until the scene
is replaced or deleted.

## Sparse state and emission

Omitted facets remain unchanged. An empty target state is invalid, and
`Effect::Off` is never stored as an appearance because it controls emission.

`Dark` may accompany an appearance, representing a configured-but-dark target.
`Emitting` requires an appearance and cannot accompany brightness zero. During
application the daemon writes every appearance in stable target order, then
brightness, then emission. Dark emission uses `Effect::Off`. Because the
current mutation vocabulary has no independent “resume emission” operation,
emitting re-applies the required appearance in the final phase.

The daemon resolves dynamic membership, authorizes leaves, and validates the
complete authorized operation set before the first hardware write. Physical
hardware is not atomic: a later failure is reported with the existing partial
mutation error and its successfully applied targets.

## Capture

Capture composes the durable adopted baseline with desired overlays. It does
not read observations or capture active frame contents. A selected target with
no representable intended facet makes capture fail rather than causing the
daemon to guess.

## Persistence and downgrade

Scenes are stored in the crash-safe whole-file `state.json` snapshot beginning
with persistence version 10. Persistence version 11 refuses collections or
scenes whose owners are legacy UID/SID identities, because ownership must be a
daemon principal. When upgrading such a state, stop the daemon and run
`luminated state reset-owned-objects --confirm`; the command writes a backup,
clears only collections and scenes, and preserves target state and the adopted
physical baseline.

Persistence version 13 validates the complete snapshot through one path before
either restoring it or replacing the durable file. A successful save is
therefore guaranteed to remain inside the loader's accepted domain. The limits
are 4 MiB for the complete pretty-printed JSON document, 100,000 target
entries, 10,000 collections, 100,000 direct collection members, 10,000 scenes,
100,000 scene bindings, 100,000 appearance-slot values across all persisted
domains, and 100,000 adopted facets. A collection or scene may occupy at most
256 KiB when serialized, and each collection or scene name or description may
occupy at most 64 KiB of UTF-8.

The same validation checks collection key/ID parity, missing references,
cycles, duplicate membership, scene key/ID parity, scene structure, dynamic
scene references, and duplicate appearance-slot identifiers. A rejected live
candidate names the exhausted dimension and leaves the previous live and
durable snapshot authoritative. At startup, a malformed external or legacy
file is retained beside `state.json` with a `.corrupt` suffix and the daemon
starts with empty state. Inspect or restore that retained file only while the
daemon is stopped.

A daemon predating scenes ignores the unknown field while reading, but may
discard it if that older daemon subsequently rewrites the state file after a
downgrade.

## Compatibility versions

The scene interface began at client protocol 19 and event protocol 5.
Daemon-managed transitions advanced those to client protocol 20 and event
protocol 6. The metadata-free ping advances the client protocol to 21 and the
C ABI/SONAME to 18. Policy-provider ABI 9 and persistence version 13 remain
current. Hardware plugin and host-supervisor ABIs are unchanged.

Schedules are not part of this version. Daemon-managed transitions, including
cubic easing, directed encoded hue, and optional `OKLab` colour interpolation,
add explicit application options rather than overloading effect periods. Their
client interface is documented in the [Rust client guide](../rust-client.md#transitions).
