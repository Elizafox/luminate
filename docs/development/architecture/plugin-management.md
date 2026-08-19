<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Plugin management architecture

Status: accepted implementation design

This note describes how Luminate discovers, configures, and reconciles plugins
at runtime. It focuses on authority, trust boundaries, and failure semantics.
The wire representation is documented in
[`protocol.md`](protocol.md), authorization in
[`client-authorization.md`](client-authorization.md), and deployment defaults
in [`../packaging.md`](../packaging.md).

## Why configuration is layered

Plugin management has two different authors:

- the global TOML is administrator-owned bootstrap policy; and
- the managed TOML is daemon-owned, machine-wide desired state changed through
  the management API.

The daemon reads but never rewrites the global file. It controls where plugins
may be discovered, the automatic activation mode, authorization policy, and
settings that are unsafe to delegate at runtime. Each global `[[plugins]]`
entry keeps that plugin's path, required and activation policy,
reconciliation policy, setting locks, and plugin-defined configuration
together. The managed file contains only the safe preferences, plugin
activation and reconciliation choices, and schema-declared plugin settings
exposed through the management API.

This separation keeps the daemon from turning delegated management access into
permission to load arbitrary code or replace administrator policy. In
particular, plugin paths, configuration and state paths, service access, and
authorization remain global-only.

A managed value remains stored when a global lock makes it ineffective. This is
intentional: changing global policy can reveal the previously requested value
without destroying it. Management snapshots therefore report desired and
effective values separately, along with the applicable locks.

## Startup authority

The effective paths are resolved after environment overrides have selected the
global configuration and state paths. Unless explicitly configured, the
managed file is `managed.toml` beside the effective state file. It must be
distinct from both the global configuration and state files.

A missing managed file means revision zero and no managed overrides. An
existing file is authoritative: failure to open, parse, or validate it prevents
startup. Silently ignoring it could unexpectedly reactivate a plugin or discard
an administrator's desired state.

The managed file is strictly parsed and limited in size. The daemon writes it
through an owner-only temporary file in the same private directory, syncs the
file, atomically renames it, and syncs the directory. Existing files are opened
without following symbolic links. These constraints make the persisted
revision the durable commit boundary.

## Installed plugin catalogue

The daemon builds a catalogue from candidates found only through
administrator-configured paths. Discovery precedence resolves duplicate paths
and names. A required candidate that cannot be inspected or conflicts with
another installed identity prevents startup; an unusable optional candidate is
reported and skipped.

Before loading a plugin into a long-lived host, the daemon starts a disposable
inspection process. That process opens one plugin image, checks its ABI and
static descriptor, returns validated identity, bus, and settings-schema
metadata, and exits. It does not invoke lifecycle, topology, or discovery
callbacks. Inspection limits the effect of malformed metadata and lets the
daemon validate configuration without first activating hardware code. It is
not a security sandbox for untrusted native code: administrators still control
which directories may supply plugin binaries.

Each catalogue entry keeps four related views:

- installed metadata: identity, version, path, buses, and settings schema;
- desired state: managed activation, reconciliation, and setting overrides;
- effective state: desired state merged with global policy and locks; and
- runtime state: inactive, loading, loaded, or failed.

Keeping these views distinct preserves useful failure information. A plugin can
be desired and effectively enabled while its host is failed, so the daemon can
report the problem and retry without changing the stored request.

## Activation resolution

Global policy selects one automatic activation mode:

- `explicit` selects explicit global entries and managed-enabled plugins;
- `host-attached` additionally selects plugins whose non-empty bus list
  contains only known, non-network buses; and
- `all` additionally selects every discovered plugin.

Any `Network` bus makes a plugin network-capable. Empty or unknown bus metadata
does not qualify for `host-attached`.

Per-plugin policy from the same global `[[plugins]]` entry and managed desired
state are then applied to that automatic selection. A required explicit plugin
is always enabled and keeps fail-startup semantics. A globally forced
activation choice overrides a managed choice; forcing a required plugin off is
invalid configuration.

When the global plugin-management section is absent, the activation mode
defaults to `host-attached`.

## Reconciliation and settings resolution

A managed plugin reconciliation value overrides the value in its global
`[[plugins]]` entry. The effective plugin value then overrides the daemon-wide
reconciliation default. Device-specific policy remains highest precedence,
followed by the plugin's recommendation and the daemon's `Leave` fallback when
no configured value applies.

Plugin settings resolve in increasing order of authority:

1. the inspected schema default;
2. the plugin's global configuration; and
3. its managed override, unless the dotted key is globally locked.

Only settings declared by the inspected schema are accepted. Values are checked
against their declared type, bounds, constraints, and required status before a
managed revision can commit. A lock on a parent dotted key also locks its
descendants. The initial runtime apply mode is restart-required, so an effective
setting change restarts a loaded plugin host.

The same layered approach applies to manageable daemon preferences. Global
locks may cover one preference or one entry in the device-reconciliation map.
The effective configuration is recomputed from the global and managed layers
rather than mutating the global configuration in place.

## Management transaction

All mutations in one patch form a revision-checked transaction:

```text
client snapshot
      |
      v
expected revision -> validate complete candidate -> persist managed.toml
                                                    |
                                                    v
                                      publish in-memory authority
                                                    |
                                                    v
                              recompute effective configuration
                                                    |
                                                    v
                                  reconcile plugin hosts and topology
                                                    |
                                                    v
                                      emit redacted change event
```

The management lock serializes snapshot validation, persistence, and the
in-memory authority update. A stale expected revision returns `Conflict`.
Unknown plugins or settings, invalid values, and an exhausted revision reject
the whole candidate without writing anything. Locked values may still be
stored, but remain dormant in the effective view. Every successful patch,
including one whose mutations do not alter a value, advances the monotonically
increasing revision once.

Persistence happens before runtime action. If persistence fails, neither the
in-memory authority nor runtime state changes. Once persistence succeeds, the
patch is committed even if a later host restart, activation, topology update,
or runtime-view refresh fails. Rolling back the file at that point would make a
temporary hardware or host failure erase durable intent. Instead, failures are
reported against runtime state and remain eligible for reconciliation.

Plugin host reconciliation uses the existing load and unload paths. Disabling a
plugin withdraws its topology while retaining the state needed for a possible
return. Enabling one validates and incorporates its topology, then schedules a
follow-up rescan. Restart-required settings unload and reload an active host.
Topology changes and the redacted configuration change are published through
the normal event mechanisms.

## Authorization and sensitive values

Reading and mutating management state require the dedicated `ManagePlugins`
policy operation, whether the request arrives through the daemon protocol or
D-Bus.

Sensitive plugin settings are write-only. The CLI accepts their values through
standard input, not command-line arguments. Management snapshots represent
sensitive desired and effective values as redacted, and change records carry
only the plugin, key, and sensitivity marker. Replies, events, diagnostics, and
logs must never reproduce the value.

Redaction is a cross-layer invariant rather than a presentation convention.
The protocol models reported values so a sensitive setting cannot accidentally
share the ordinary readable-value path, and D-Bus and CLI representations are
derived from that model.

## Setup workflows

Installed plugins expose no setup workflows by default. The control protocol
and Rust/C client APIs can enumerate typed workflow metadata for one plugin;
unknown plugin names remain distinct from known plugins with an empty workflow
list. Discovery uses `ManagePlugins`, since later setup sessions will commit
plugin settings or activation under the same authority.

Workflow descriptors and an optional setup callback are part of the plugin
ABI. Static inspection validates the descriptor before the daemon advertises
it. The callback runs one step in a disposable process with a hard deadline;
ordinary plugin hosts never receive setup credentials or own persistence.

The daemon owns unpredictable, actor-bound, expiring session IDs and a
monotonic interaction generation. It retains plugin-private continuation bytes
but returns only choices, physical-action instructions, and sanitized terminal
states to clients. A late response is rejected by generation. Cancellation or
expiry discards continuation state.

Successful setup returns a structurally separate settings map. The daemon
limits it to the originating plugin's inspected schema, checks touched settings
against their session-start values, and commits through the normal atomic
management path. Unrelated management revisions may be rebased; a change to a
touched setting fails safely. Generated credentials are never copied into
session snapshots, events, or completion diagnostics.
Once a completed plugin step atomically enters the applying state, cancellation
can no longer overtake its configuration transaction.

`luminatectl plugin setup PLUGIN` lists the advertised workflows. Supplying a
workflow ID runs it interactively. With `--json`, discovery returns an array and
execution emits one compact session object per state change. Choice and
physical-action responses are read as JSON lines:

```json
{"response":"choice","choice":"bridge-id"}
{"response":"confirmed"}
```

Workflow `kind` and session `state` values use lowercase snake case. Session
objects contain `session`, `plugin`, `workflow`, `generation`, and a `status`
object whose fields depend on `state`. This CLI representation is a stable
machine-readable contract separate from the protocol types' Serde encoding.

## Architectural consequences

- Administrator policy remains effective even when management clients are
  compromised.
- A management reply distinguishes durable intent from runtime convergence.
- Installed, desired, effective, and runtime states must not be collapsed into
  one Boolean.
- Static metadata may inform activation and validation, but only configured
  paths establish which native code is eligible.
- Optimistic concurrency protects administrators from silently overwriting
  changes based on stale snapshots.
- Secret safety must hold in every representation, including error and event
  paths.
