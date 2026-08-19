<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Daemon authentication and authorization

`luminated` is the sole authority for client identity, policy, ownership,
limits, revocation, and authorization audit. It listens only on local IPC.
HTTP, D-Bus, and other front ends may impose additional checks at their own
boundaries, but they cannot grant more access than the daemon permits.

## Trust boundaries

Unix installations retain the `luminate` group, a protected runtime directory,
and a `0660` socket. Windows service installations use the configured local
group and named-pipe DACL. These boundaries decide which processes may attempt
authentication; they do not replace authentication or policy evaluation.

The daemon captures kernel peer credentials once when accepting a connection.
It derives an immutable actor for audit and for authentication methods that
must be bound to a process identity. Callers cannot select their actor class.

The default `socket-trusted` mode preserves local compatibility: an ordinary
process admitted by the socket and authenticated directly as its peer receives
the full API. Registered front ends and non-peer subjects do not receive that
shortcut. In `managed` mode all ordinary sessions are policy-evaluated.

Intrinsic break glass is limited to the real root/daemon identity on Unix and
the service identity or enabled `BUILTIN\Administrators` membership on Windows.
It bypasses configurable policy, ownership, and subject limits, but not
protocol validation, global safety limits, hardware capabilities, or a
voluntary session scope. Helpers and plugins running as the daemon account
inherit that power, which is why packages use a dedicated service account.

Exact configured recovery principals may repair policy and authentication
state only when directly peer-authenticated. Group membership and delegated
authentication never confer recovery authority.

## Authentication and sessions

Every primary connection first negotiates the consumer protocol without
credentials, then performs one bounded authentication exchange. Supported
methods are:

- kernel peer credentials;
- daemon bearer tokens;
- actor-bound, one-use attestations minted by daemon administration; and
- configured supervised executable authentication providers.

The resulting session fixes the actor, canonical `PrincipalId`, verified
groups, sanitized authentication source and credential ID, lease, and optional
allow-only `SessionScope`. Effective access is the intersection of the actor
ceiling, subject policy, current credential validity, and voluntary scope.
Scope grants can only remove authority and cannot be broadened after connect.

Bearer secrets contain 256 bits from the operating system random source. They
are displayed once; daemon state stores only domain-separated SHA-256 hashes.
Comparison is constant-time. Expiry, revocation, and rotation close affected
active sessions. Credentials and provider continuations are distinct redacted,
zeroizing wire domains with independent bounds.

External authentication providers use independently versioned protocol v1 over
bounded stdin/stdout frames. Configuration fixes their public name, authority,
absolute executable path, private initialization file, required/optional startup
posture, and concurrent-session ceiling. Responses supply identity facts and
revalidation state, never roles or permissions. Provider exchanges use bounded
deadlines. Active sessions revalidate halfway through each provider lease and
close if revalidation fails or changes immutable identity facts. Invalid
exchanges, timeouts, crashes, and unavailable revalidation fail closed. A
required provider that cannot start prevents daemon startup; an optional
provider only loses its own authentication method.

Event connections do not repeat credentials. A primary client mints a
short-lived, one-use ticket carrying its subject, lease, scope, and policy
context. Consumption must come from the same kernel actor. Reuse, expiry, and
actor mismatch are rejected.

## Policy model

The built-in evaluator consumes one validated, revisioned `PolicyDocument`.
Principals contain only an authority and subject plus verified authentication
groups. Bindings assign materialized or administrator-defined roles. Roles may
inherit roles and contain stable-ID allow or deny rules over exhaustive
operations and optional resource constraints.

Matching denies override matching allows; otherwise any matching allow wins,
and no match denies. Device constraints cover stable device IDs, configured
provider instances, host attachment, and transitive collection membership.
Resource-constrained rules never match resource-free administration requests.
Presets are explicit viewer, user, operator, and administrator roles rather
than hidden evaluator behaviour.

Authorization roles do not carry resource quotas. The daemon configuration
exposes only the global connection and event-subscription ceilings enforced at
listener admission. Omitted values retain the daemon's conservative built-in
ceilings. The listener also applies a fixed per-kernel-actor connection cap.

Authorization occurs after selectors are resolved and before observation,
mutation, persistence, or hardware access. The daemon pins the topology
generation across policy evaluation, response capture, and hardware dispatch,
so stale decisions return a retryable conflict. Frame streams retain the
generation authorized when they begin; a topology replacement rejects further
uploads and terminates the daemon-side stream. Internally, event publications
carry their originating generation. A queued event from an older topology is
converted into a full-topology resynchronization signal rather than filtered
against replacement metadata. Collection and scene ownership stores the exact
canonical principal. Unknown ownership fails closed; administrator operations
are the explicit override.

Policy replacement validates a complete document, checks its expected
revision, records the required durable pre-mutation audit event, and atomically
persists it. A failed validation, audit write, comparison, or state write
leaves the active policy unchanged.

## Front ends

A registered front end is trusted to attest a caller identity, not to choose
permissions. D-Bus first applies its caller group or Polkit rule, requests a
one-use actor-bound attestation through an administrative daemon connection,
then opens an ordinary client for the attested subject. HTTP first
authenticates each opaque daemon bearer credential. When a bounded delegation
claim is present, it requires the registered HTTP actor and
`AdministerFrontend`, mints a group-bearing one-use attestation, and reconnects
as the delegated subject. The effective result at either boundary is always
the intersection of front-end checks and daemon authorization.

Packages run HTTP and D-Bus under distinct dedicated accounts in the client
group. Neither runs as root or as the daemon account. Registration and policy
must name those exact platform identities; sharing an identity between front
ends would collapse their actor ceilings and audit attribution.

Front-end registrations accept either a canonical platform identity or an
account reference resolved at daemon startup. Unix entries set exactly one of
`uid` or `user`; Windows entries set exactly one of `sid` or `account`.
Explicit Windows SIDs are validated and canonicalized in the same form as
captured named-pipe SIDs. Resolution failure prevents startup rather than
silently disabling the trust boundary. Only registered actors may create,
list, or revoke attestations, and daemon policy must independently grant
`AdministerFrontend`. Token operations instead require
`ManageAuthentication`.

## Persistence, audit, and disclosure

Policy, bearer-token hashes, and JSONL authorization audit records live in
private daemon state and use crash-safe replacement helpers. Administrative
mutations require a durable audit record before changing state. Ordinary
observe and control decisions use best-effort audit with loud drop diagnostics,
so an audit outage cannot silently masquerade as complete records. Intrinsic
break glass can recover administration during an audit outage.

For token, external-provider, and attestation sessions, the private audit JSON
records the authenticated subject's non-secret `credential_id`. Delegated
sessions additionally record the front-end principal and its non-secret
`credential_id`. These fields are additive to the private JSONL format. They
support revocation and incident attribution, and never contain bearer tokens,
attestation secrets, or event tickets.

Audit records include actor, subject, authentication source, credential ID,
operation, bounded resource summary, outcome, rule ID, and policy revision.
They never include presented credentials, provider continuations, private
provider initialization data, or sensitive managed settings. Denials and
front-end responses use caller-safe diagnostics and do not reveal hidden
resource identities.

Daemon persistence format 11 refuses legacy UID/SID-owned collections and
scenes rather than guessing a canonical identity. The offline, explicitly
confirmed `luminated state reset-owned-objects` recovery command backs up the
state, clears only those owned objects, preserves target and baseline state,
and writes format 11.

## Compatibility versions

This redesign established consumer protocol ABI 23, event protocol 7, C ABI
and SONAME 22, daemon persistence 12, and authentication-provider protocol 1.
The daemon protocol and C API report incompatible versions explicitly; no
authentication failure may fall back to a less restrictive mode.
