<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Plugin setup API plan

Status: in progress. The generic session facility, Philips Hue push-link
workflow, and `luminatectl` frontend are implemented. Richer interaction kinds
remain pending. Philips Hue push-link setup is validated on physical hardware.

## Purpose

Provide a generic, typed setup facility for plugins through `luminated` and the
Rust and C `libluminate` APIs. A plugin has no setup workflows unless it
explicitly declares them. Vendor protocols, generated credentials, and device
verification remain inside the plugin-host boundary; client applications see
only typed interactions and sanitized results.

The first consumer is expected to be Philips Hue push-link pairing, but no
public type or daemon behaviour may depend on Hue concepts.

## Architectural boundaries

- Plugins declare zero or more setup workflows through static, inspectable
  metadata. An absent declaration is equivalent to an empty list.
- `luminated` owns authorization, session identity and lifetime, concurrency,
  sensitive transport, configuration validation, and atomic persistence.
- A short-lived setup host owns vendor discovery, pairing, verification, and
  construction of a constrained settings result. It does not write managed
  configuration itself.
- `libluminate` exposes the same typed model to Rust and C consumers.
  `luminatectl` is a presentation layer rather than the owner of setup logic.
- Plugin-generated secrets never appear in client-visible snapshots, events,
  diagnostics, or completion results. User-supplied secret input travels only
  inward through explicitly sensitive fields.

## Public vocabulary

Each workflow has a plugin-local stable ID, human-readable label and
description, and a typed kind. Initial kinds distinguish provisioning new
hardware, repairing an existing configuration, discovery, importing
configuration, and factory provisioning. Unknown future kinds require an
explicit compatibility strategy before they are added to a serialized enum.

Interactive setup is a daemon-owned session with a monotonically increasing
generation. Its eventual state vocabulary is deliberately bounded:

- starting and progress;
- choice, structured input, and confirmation requests;
- physical and external action requests;
- applying configuration and verification; and
- completed, failed, or cancelled terminal states.

Each response names both the session and the interaction generation so a late
or repeated response cannot answer a different prompt.

## Security and authority

Setup initially requires the existing `ManagePlugins` operation because a
successful workflow changes managed plugin settings or activation. A distinct
least-privilege provisioning operation should be added only when delegating
pairing without general plugin management is a concrete requirement.

The daemon validates every setup result against the inspected schema and lets
it mutate only settings owned by the originating plugin. Global locks remain
authoritative. Plugin paths, daemon preferences, other plugins, and global
configuration are outside the setup host's authority.

Setup sessions use unpredictable identifiers, are associated with their
initiating authenticated actor, are bounded by inactivity and workflow
deadlines, and are not persisted across daemon restarts. A disconnected client
does not implicitly cancel a session; explicit cancellation and expiry govern
cleanup.

## Configuration concurrency

Setup must not hold the managed-configuration lock while a user performs a
physical action. At session start, the daemon records the relevant plugin
settings and locks. On completion it may rebase over unrelated revisions, but
must report a recoverable conflict if a touched setting or activation choice
changed. A generated credential may remain only in the in-memory session for a
short, bounded commit retry, after which it is discarded.

## Implementation sequence

- [x] Add workflow descriptor wire types and a request that lists workflows for
      one installed plugin. Add Rust and C `libluminate` accessors. Existing
      plugins return an empty list by default. Bump the control protocol version
      and test authorization, unknown plugins, serialization, and ownership.
- [x] Extend the plugin ABI with optional, statically inspectable workflow
      descriptors. Validate all strings, IDs, counts, and discriminants in the
      disposable inspection process. Preserve an absent descriptor as no setup.
- [x] Add daemon-owned session IDs, generations, lifecycle storage, typed state
      snapshots, get/cancel operations, actor ownership, expiry, and
      authorization. Add matching Rust and C APIs without callbacks.
- [x] Add the isolated setup-host protocol and optional plugin entry points.
      Choice and physical-action interactions are implemented without creating an
      arbitrary remote-UI language. Progress, structured input, and external
      action remain future additions driven by concrete plugin needs.
- [ ] Add progress, structured-input, and external-action interactions when a
      concrete plugin needs them.
- [x] Add a structurally separate plugin-generated settings result. Validate and
      atomically persist the result, reconcile the plugin host, rescan, and return
      only a sanitized completion summary.
- [ ] Add sensitive inward input.
- [x] Add `luminatectl plugin setup` as an interactive frontend plus stable JSON
      output for workflow discovery and session state changes.
- [x] Implement Philips Hue discovery, bridge selection, certificate-bound
      push-link pairing, credential verification, and sanitized completion in the
      Hue plugin.
- [x] Validate Philips Hue setup on physical hardware before changing support
      claims.

## Compatibility

Workflow discovery changes the daemon control protocol and therefore increments
`PROTOCOL_ABI_VERSION`. The Rust and C client additions are additive. Plugin ABI
changes wait for the second slice and will receive their own coordinated
version update. Every serialized enum, numeric C constant, generated header,
and public fallible API must receive compatibility and documentation review.

## Verification

Each slice receives deterministic protocol round-trip tests, daemon dispatch
and authorization tests, Rust client tests, C accessor and consumer tests, and
generated-header verification. Before a slice is considered complete, run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Run the libluminate and luminated coverage workflows when the environment has
their required tools. No ordinary test may require network access or hardware.
