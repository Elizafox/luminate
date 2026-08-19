<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Zero-copy SHM frame streaming — Phase 2 (client ↔ daemon hop)

> Status: landed. This document is the permanent record of the design for
> the client-facing follow-on to
> [`shm-frame-streaming-phase1.md`](shm-frame-streaming-phase1.md).

## Why this exists

[`shm-frame-streaming-phase1.md`](shm-frame-streaming-phase1.md) built a
zero-copy fast path for the daemon ↔ plugin-host hop only, explicitly
deferring the client ↔ daemon hop because it crosses a real Unix-uid trust
boundary: `luminated` is a system daemon and `libluminate` clients are
separate processes that may run as a different, less-trusted user. Phase 1
does not reduce client-observable frame latency at all — it removes the
redundant CBOR re-encode/decode and pipe hop between the daemon and its
plugin-host child. Phase 2 is where the client-observable framerate
ceiling actually lives.

## Uid scoping: same-uid fast path only, door left open for ACLs later

Testing iceoryx2 0.9.3 found no cross-uid access control at all. Verified
directly against the vendored source
(`iceoryx2-{cal,bb-posix,pal-configuration}-0.9.3`, available locally under
`~/.cargo/registry/src/index.crates.io-*/`) and confirmed against a live
`/tmp/iceoryx2`/`/dev/shm` state:

- Every resource iceoryx2 creates (payload/dynamic shm segments, the
  `.service`/`.node` static-config registry files, the event mechanism's
  `AF_UNIX` datagram socket) gets a hardcoded owner-only mode: shm
  segments `0700` (`iceoryx2-cal-0.9.3/src/dynamic_storage/posix_shared_memory.rs:76-82`),
  the service config file `0400`, registry directories `0750`
  (`iceoryx2-cal-0.9.3/src/static_storage/file.rs:67-80`), the event socket
  `0700` (`iceoryx2-cal-0.9.3/src/event/unix_datagram_socket.rs:34-38`).
- No `Config`/`ServiceBuilder`/`NodeBuilder` field anywhere in the public
  `iceoryx2` crate exposes a mode/uid/gid override. The only knob is
  `dev_permissions`, a compile-time Cargo feature that flips every
  resource in the process to `0777`/`0444` world-read-write — all or
  nothing, process-wide, not per-service. Not usable for a system daemon.
- No credential check on attach either: `SocketCred`/`SCM_CREDENTIALS`
  exists in `iceoryx2-bb-posix` but is never called from the service/port
  layer. Access is purely "can this uid pass the kernel's ordinary DAC
  check on the file/shm-object," nothing iceoryx2-specific layered on top.

A naive port of Phase 1's deterministic-name, no-permission-handling
approach to the client ↔ daemon leg would either silently never work
cross-uid (daemon and client can't open each other's owner-only resources)
or require `dev_permissions`, which would make the daemon's shm
world-writable to every local user. That would let any local user inject
frames into another user's stream, regressing from today's `0660` group-scoped
socket (`crates/luminated/src/daemon/mod.rs`'s `SOCKET_MODE`).

Phase 2 offers the SHM fast path only when the connecting
client's `SO_PEERCRED` uid (already captured today as `Principal.uid`, see
`crates/luminated/src/daemon/authorization.rs`) equals the daemon
process's own uid — i.e., a client that is, literally, the same OS user the
daemon runs as (the natural case for a per-user session daemon, and for
every local-dev workflow `docs/development/plugins/demo-plugin.md`
already documents). Any other uid — including today's already-supported
same-group-different-uid clients — gets `LUMINATE_STATUS_UNSUPPORTED` from
the new call and keeps using the existing `BeginFrameStream` path,
exactly as it does today. This is a strict superset of today's
capabilities: nobody who can drive a light now loses the ability to; some
callers just don't get the speed boost yet.

This keeps segment naming as simple as Phase 1's (deterministic,
target-derived) rather than needing unguessable per-connection tokens or
ACL provisioning: within the same uid there is no adversarial process to
defend the name against, so the only remaining concern is same-uid
resource contention (two client processes racing for the same target),
which the daemon already serializes today via `DaemonState::begin_frame_stream`
rejecting a second stream on an already-active target — not a new
mechanism.

The uid-equality check lives in exactly one place: a single predicate,
`Principal::is_same_user_as_daemon(&self) -> bool`, checked once at
`BeginShmFrameStream` time. A future increment adding real cross-uid
support (POSIX ACL grants applied per-connection to the specific client
uid, with their own new-dependency sign-off and revocation-on-end
lifecycle) only needs to widen that one predicate and the naming scheme
it gates; nothing else in this document's design changes shape when that
happens. Tracked as explicit deferred work below, not implemented now.

## Explicit new client call, not a transparent upgrade to `BeginFrameStream`

Phase 1's "fully automatic and transparent" activation worked because the
choice was entirely internal to processes the daemon already spawns and
controls — nothing outside the daemon's own trust domain had to do
anything differently. That doesn't carry over: in Phase 2 the client
process itself has to mmap a segment and write pixels into it, an
irreducibly different code path from "build a `Vec<Colour>` and call
`upload_frame_full`." Hiding that behind the existing calls would mean
either a silent internal copy inside `libluminate` (a real but partial
win — skips the socket round trip and CBOR encode, still copies once) or
false transparency.

`BeginFrameStream`/`UploadFrame`/`EndFrameStream` keep meaning exactly
what they mean today — the portable, always-available request/response
path, no new failure modes, no behavioural change. A new, explicit,
opt-in call (`BeginShmFrameStream`) exists for clients that want the fast
path. This also gives capability negotiation a clean, dedicated place to
live instead of being smuggled into `BeginFrameStream`'s existing response
shape.

## Role reversal: client is the publisher

Phase 1: daemon publishes, plugin-host subscribes. Phase 2 flips this for
the new leg, because the client is now the actual frame producer: client
publishes, daemon subscribes.

The daemon, not the client, continues to create the iceoryx2 services for the
client leg. It already owns capability negotiation and is the trusted party.

The topology retains two segments rather than one spanning all three
processes. A single segment from client to plugin-host would remove the daemon
from the data path, but the daemon must remain the validation and authorization
chokepoint (capability checks, hardware-effect concurrency, and rate limiting)
regardless of transport. It cannot rubber-stamp bytes it never inspects. The
actual shape is a second SHM leg (client → daemon) feeding into Phase 1's
existing daemon → plugin-host leg, with one small validating copy at the daemon
boundary. That copy is the price of keeping the daemon as a real mediator, not
a design compromise to fix later. See "Overhead of the relay copy" below for
why this is not a performance concern in practice.

## Revocation semantics: consumer-side disengagement, not memory reclamation

The daemon cannot literally claw back pages it has already let a client
mmap — there's no "your mapping vanishes now" primitive, and revocation
doesn't need one. "Grant revoked" means the consumer stops honouring the
segment, not that the producer's writes are prevented:

1. Daemon invalidates (bumps, or marks dead) the stream's generation/token.
2. The daemon (the subscribing side on this leg) stops accepting frames
   tagged with the old generation/token.
3. The daemon's subscriber/service for that stream is dropped.
4. The client is told the stream ended, via the lifecycle-event channel
   below — not left to infer it from silence.
5. Anything the client still writes after that lands in an abandoned
   segment nobody reads — a harmless write into memory that happens to
   still be mapped, not a security hole. A malicious client gains nothing
   by continuing to publish once steps 2–3 have happened; it can only
   ever talk to itself.

This composes directly with the `generation` field Phase 1 already carries
in `ShmFrameHeader` and with the stream nonce from the security section
below — no new mechanism required, just the explicit statement that
revocation is implemented as "the daemon stops listening and says so," not
as memory protection.

## Async lifecycle/health events, not per-frame acks

Today, on the sync path, the plugin-host dies when the daemon's pipe
closes — the stream dies for free, no extra mechanism needed. That
doesn't extend to the client leg: a client can keep holding its SHM
publisher and keep "successfully" sending into a segment nobody is
reading from anymore after the daemon dies or the grant is torn down (the
scenario the revocation semantics above make harmless, but the client
still needs to find out) — structurally the same problem Phase 1 solved on
the daemon → plugin-host leg with `connection_epoch`
(`crates/luminated/src/plugin_host/supervisor.rs`), just facing the other
direction across the new boundary.

The fast path itself stays fire-and-forget (per-frame acks would defeat
the point, as in Phase 1). A control-plane channel carries async
stream-lifecycle events (e.g. "grant revoked", "daemon-side stream torn
down") — not per-frame status — driving step 4 of the revocation sequence
above. This also partially addresses Phase 1's accepted observability
regression (a persistent plugin apply failure is currently only visible
via logs); a lifecycle-event channel is a natural place to eventually
surface a health signal for that too, without reintroducing per-frame
overhead. Transport: piggyback on the existing event-subscription
connection/`LUMINATE_EVENT_*` machinery (`crates/libluminate`'s
`luminate_event_subscription_next`) rather than inventing a third channel.

## Security: hostile-input handling, even though the uid boundary is closed for now

With the same-uid gate above, Phase 2 does not face an adversarial
different-uid process attaching to its segments — that channel is closed
by kernel DAC on iceoryx2's own hardcoded owner-only permissions. But the
segment is still a boundary worth defending rigorously, for two reasons
that have nothing to do with uid: a same-uid client process can be buggy,
compromised (e.g. a malicious plugin/extension running inside a trusted
user's own client app), or crash mid-write leaving a torn frame;
and this is the same discipline Phase 1 already applies to
plugin-host-produced frames even though that leg has no adversary either.

- Validate `header_version` before trusting anything else in the header;
  validate the payload length exactly matches `pixel_count *
bytes_per_pixel` before indexing; validate `pixel_format` is a known
  discriminant; reject unrecognized `flags` bits rather than silently
  ignoring them; validate `generation`/`sequence` against expected
  bounds; all arithmetic on client-controlled fields (`pixel_count *
bytes_per_pixel` in particular) must be checked/overflow-safe; a
  malformed frame must never be able to panic the stream thread;
  malformed-frame logging needs rate-limiting so a buggy or malicious
  same-uid client can't use it as a log-flood DoS vector.
- Phase 1's `ShmFrameHeader::to_bytes()`/`from_bytes()` design (explicit
  little-endian field-by-field encode/decode, deliberately not a
  `repr(C)` pointer-cast reinterpretation, chosen originally for the
  alignment-safety reason documented in the Phase 1 doc) keeps mattering
  here — keep the pattern, do not relax it for Phase 2.
- **A stream nonce/ID is added to the header.** Not a defense against an
  attacker guessing the service name (same-uid scoping already closes
  that channel) — it catches a stale-but-same-uid segment: a crashed and
  respawned client process reusing the same deterministic target-derived
  service name before the daemon has finished tearing down the previous
  stream's resources. Same category of bug Phase 1's `connection_epoch`
  catches on the daemon → plugin-host leg, applied here to the
  client → daemon leg.
- Segment/service naming stays deterministic (target-derived, mirroring
  `crates/luminate-host-supervisor/src/shm.rs`'s existing
  `service_names()` helper) precisely because the same-uid gate above
  means there is no name-guessing attacker to defend against. This is the
  one place the future cross-uid increment would have to change — see
  "Uid scoping" above.

## Overhead of the relay copy

The one memcpy at the daemon boundary (see "Role reversal" above) is not
expected to impose substantial overhead, quantified rather than assumed:

- **The copy itself.** A 10,000-pixel RGB8 frame — large for any bundled
  or plausible near-term target — is ~30 KB; copying it takes low
  single-digit microseconds on any modern CPU (memcpy throughput is
  easily several GB/s). At a stress-case 1 kHz framerate that's under 1%
  of a single core. Most real LED hardware tops out at a few hundred Hz
  regardless, due to protocol-level limits (e.g. WS2812 timing).
- **The two thread wake-ups either side of the relay** — the client's
  notify waking the daemon's stream thread, the daemon's notify waking the
  plugin-host's — are backed by iceoryx2's native futex/eventfd-class wait
  primitive and add tens of microseconds, not milliseconds. The
  `Listener::timed_wait_all` poll interval from Phase 1's design (e.g.
  250 ms) is only the upper bound for rechecking the shutdown flag when no
  notification arrives; an actual notify wakes the thread immediately.
- **Net:** total added latency through the relay is realistically tens of
  microseconds against a frame budget of ≥1000 μs even at an aggressive
  1 kHz target — negligible next to what it buys (per-frame authorization,
  matching the daemon's already-established policy model on the ordinary
  path).
- **Where this would start to matter instead:** a large number of
  concurrent high-framerate streams, since each active stream is its own
  OS thread on both legs — cost scales with concurrent-stream count, not
  framerate per stream. Phase 1's design already treats "many concurrent
  streams per process" as a non-goal for the per-stream-thread model;
  revisit that model specifically (not the relay-copy decision) if a
  future use case ever needs dozens+ of simultaneous high-rate streams per
  daemon.

## New wire and C ABI surface

New crate dependency, with its own sign-off distinct from Phase 1's:
`libluminate` adds a direct dependency on the ordinary Rust `iceoryx2`
crate (not `iceoryx2-ffi-c` — `libluminate` already owns its own
hand-rolled `extern "C"` boundary in `src/ffi.rs`, so it consumes iceoryx2
the same way `luminated` does, as a Rust crate, and re-exposes it through
that existing boundary). `cargo audit` is re-run against the pinned
version as part of this phase's own verification, not assumed carried
over from Phase 1's.

**`crates/luminate-protocol/src/message.rs`** (bumps `PROTOCOL_ABI_VERSION`
— the constant Phase 1's version table explicitly left untouched, and what
this phase triggers):

```rust
Request::BeginShmFrameStream { target: TargetId }
// -> ResponseStatus::ShmFrameStreamReady {
//        generation: u32,
//        service_name: String,  // deterministic, target-derived
//        pixel_format: ShmPixelFormat,
//        stream_nonce: u64,     // see security section
//        segment_bytes: u32,
//    }
// -> ResponseStatus::Unsupported (existing variant: uid mismatch, no
//        shm capability, prefer_client_shm disabled, or target already
//        streaming all collapse to this — same posture BeginFrameStream
//        already has for "target doesn't advertise capability")
```

`UploadFrame`/`EndFrameStream` wire messages are not reused for the SHM
leg (data doesn't cross the socket at all once negotiated); a matching
`Request::EndShmFrameStream { target, generation }` closes out the control
side, mirroring `EndFrameStream`'s idempotent posture.

**`crates/libluminate`** (bumps `LUMINATE_C_ABI_VERSION` — unlike Phase 1,
this phase adds new client-visible C surface):

- `luminate_client_begin_shm_frame_stream(client, target, *out_stream)` —
  opaque `LuminateShmFrameStream*` handle on success,
  `LUMINATE_STATUS_UNSUPPORTED` on any of the collapsed reasons above (the
  caller's fallback is to call the existing
  `luminate_client_begin_frame_stream` instead).
- `luminate_client_shm_upload_frame_full(stream, colours*, count, commit,
*out_ack)` — copy-in convenience function: packs the caller's buffer
  into a loaned iceoryx2 sample and sends. One copy (app buffer → shm
  segment), but skips the socket round trip and both CBOR encode/decodes
  `UploadFrame` costs today. Ships in Phase 2.
- `luminate_client_end_shm_frame_stream(stream)` — idempotent, symmetric
  with `luminate_client_end_frame_stream`.
- A new `LuminateEventKind` variant, `LUMINATE_EVENT_SHM_STREAM_ENDED`,
  delivered through the existing `luminate_event_subscription_next`
  machinery per the lifecycle-events decision above — no new event
  channel, one new discriminant.

A loan/commit pair is deferred from this phase:
(`luminate_client_shm_loan_frame` / `luminate_client_shm_commit_frame`)
letting an advanced caller write pixels directly into the shared segment
with zero copies on the client side. The copy-in convenience function
above already captures the socket-round-trip and CBOR-encode win, which is
where today's actual overhead lives; the loan-based API is a further,
separable optimization for callers generating pixels fast enough that even
one memcpy matters, consistent with Phase 1's own pattern of shipping the
smaller version first (e.g. deferring the 1-bit pixel format
until a real consumer needs it).

## Daemon-side implementation shape

Directly reuses Phase 1's established machinery rather than inventing new
patterns:

- **Role-reversed registry.** A new `ShmClientSubscriberRegistry`
  (`crates/luminated/src/plugins/shm.rs` or a sibling module), structurally
  the mirror of Phase 1's `ShmPublisherRegistry`: on `BeginShmFrameStream`,
  the daemon creates (not opens) the two iceoryx2 services — it remains
  the resource owner and trusted party, per "Role reversal" above — then
  spawns one dedicated OS thread per active client-fed stream, directly
  reusing the per-stream-thread pattern
  `crates/luminated/src/plugin_host/shm.rs`'s `ShmRuntime` already
  established in Phase 1 (`Listener::timed_wait_all()` in a loop, no
  shared `WaitSet`, same soundness argument applies unchanged).
- **Validate-then-relay, not a raw pass-through.** Each stream's dedicated
  thread decodes and validates a received frame (hostile-input checklist
  above), then calls into the same per-frame apply path
  `daemon/dispatch/streaming.rs`'s `UploadFrame` handler already uses today
  (`PluginManager::try_apply_shm_frame`, falling back to `apply_frame`) —
  likely requires extracting that one-frame-apply logic into a function
  callable from both the existing async dispatch handler and this new
  synchronous OS thread, rather than duplicating it. This is the "small
  validating copy at the daemon boundary" from "Role reversal" above, and
  it's also where the rate-limiting/generation checks that exist today for
  the ordinary path keep applying unchanged.
- **Config.** A new `DaemonConfig::prefer_client_shm: bool` (default
  `true`), separate from Phase 1's existing `prefer_shm` (left as-is —
  renaming it retroactively to disambiguate isn't worth the churn on an
  already-shipped field) — same ops-rollback posture Phase 1 established.
- **Connection-drop cleanup.** Extends
  `crates/luminated/src/daemon/connection.rs`'s existing `owned_streams`
  drain (already handles `end_all_frame_streams`/`end_all_shm_streams` on
  disconnect) with the new client-published streams — same code path, one
  more registry to sweep.

## Testing

- Unit tests: `Principal::is_same_user_as_daemon` predicate (uid match/mismatch,
  config-disabled, no-capability cases); hostile-input validation
  (truncated payload, bad `pixel_format`/`header_version`/`flags`,
  overflowing `pixel_count * bytes_per_pixel`) — every case must reject
  cleanly, never panic.
- Cross-process integration test: a real client subprocess (using
  `libluminate`'s new C API, or its Rust equivalent) and a real `luminated`
  instance running as the same test-runner uid — negotiate, stream N
  frames, confirm delivery via an independent counter (mirroring Phase 1's
  `frames_applied` pattern), then end and confirm cleanup. Same-uid is
  trivially true in CI (single runner uid), so this exercises the real
  fast path, not a mock.
- Out of scope for automated testing, with the same posture as
  Phase 1's precedent: a literal different-uid integration test (would
  need root/setuid tooling in CI). Covered instead by a direct unit test
  of the `is_same_user_as_daemon` predicate with a fabricated non-matching
  `Principal`.
- Manual end-to-end smoke test once the C API lands: drive
  `luminate_client_begin_shm_frame_stream`/`_shm_upload_frame_full`/
  `_end_shm_frame_stream` against `demo-led-display` and confirm two
  independent signals the fast path activated (daemon-side log line with
  `segment_bytes`, and the existing plugin-side `frames_applied` counter
  from Phase 1 — should increment even though these frames now arrive via
  a different daemon-side entry point).

## Affected crates

| Crate               | Changes                                                                                                                                                                                                                                                                                   | ABI                           |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------- |
| `luminate-protocol` | `Request::BeginShmFrameStream`/`EndShmFrameStream`, `ResponseStatus::ShmFrameStreamReady`                                                                                                                                                                                                 | `PROTOCOL_ABI_VERSION` bump   |
| `luminate-core`     | `ShmClientFrameHeader` (new sibling type to Phase 1's `ShmFrameHeader`, adding `stream_nonce`) and its hostile-input `parse` validation                                                                                                                                                   | None                          |
| `luminated`         | `Principal::is_same_user_as_daemon` predicate; `plugins/shm_client.rs`'s `ShmClientSubscriberRegistry` (new); shared one-frame-apply function (`PluginManager::apply_one_frame`); `daemon/dispatch/streaming.rs`, `daemon/connection.rs`, `device_config.rs` (`prefer_client_shm`) wiring | None                          |
| `libluminate`       | New `iceoryx2` dependency; `LuminateShmFrameStream` opaque handle and three new `extern "C"` functions; new `LuminateEventKind` variant                                                                                                                                                   | `LUMINATE_C_ABI_VERSION` bump |

`crates/luminate-core/src/shm_frame.rs` gains `ShmClientFrameHeader` as a
new sibling type, not a change to Phase 1's `ShmFrameHeader` — that type,
and the daemon ↔ plugin-host wire shape it describes, are both reused
unmodified. The client ↔ daemon leg needed one extra field
(`stream_nonce`, see "Security" below) that leg has no use for, so it gets
its own struct rather than growing `ShmFrameHeader`'s shape for a hop that
doesn't need it.

## Decisions and deferred work

- **Same-uid-only fast path, door left open for cross-uid ACLs later** —
  see "Uid scoping" above. The single predicate that gates this is the one
  integration point a future increment needs to touch.
- **Copy-in upload API ships now; loan/commit zero-copy API deferred** —
  see "New wire and C ABI surface" above.
- **Two-segment relay topology (client → daemon, daemon → plugin-host), not
  one segment spanning all three processes** — keeps the daemon a real
  validation chokepoint; the one small copy at the daemon boundary is the
  accepted price, not a compromise to fix later (see "Overhead of the
  relay copy").
- **Lifecycle events piggyback on the existing event-subscription
  channel** rather than a third transport — exact new `LuminateEventKind`
  variant(s) to be finalized during implementation; may also become the
  natural home for surfacing Phase 1's still-open observability gap
  (persistent plugin-apply failures currently visible only via logs), but
  that extension is not required for Phase 2 to land.
- **`ResetShmStream`-equivalent for the client leg:** not designed here;
  Phase 1 shipped `ResetShmStream` on the daemon ↔ plugin-host leg with no
  caller yet either. Revisit once real usage patterns exist.
