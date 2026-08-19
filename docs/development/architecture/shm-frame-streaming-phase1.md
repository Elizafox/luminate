<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Zero-copy SHM frame streaming — Phase 1 (daemon ↔ plugin-host)

> Status: landed. This document is the permanent record of the design;
> Status: landed. This document is the permanent phase-one design record.

## Why this exists

The ordinary frame-streaming path (`BeginFrameStream`/`UploadFrame`/
`EndFrameStream`, see `docs/development/architecture/protocol.md`'s "Frame
streaming" section) is a synchronous request/response round trip per frame,
CBOR-encoded twice: once over the client ↔ daemon Unix socket, again over the
daemon ↔ plugin-host pipe. That is fine for the ~5-10 Hz every bundled plugin
needs today.

This work is anticipatory: future hardware doing genuine per-pixel
animation at real framerates will need faster delivery, and the two extra
serialize/copy hops plus the round-trip-per-frame shape become the
bottleneck at that point. Rather than retrofit under pressure later, this
phase builds the fast path now, as a second, opt-in API alongside the
existing one — not a replacement. Every plugin that only implements
`FrameStreamingPlugin` keeps working exactly as before, with zero
behavioural change.

This phase covers only the daemon ↔ plugin-host hop. That hop is a single trust
domain (`luminated` spawns and fully controls its plugin-host child), so it is
the lower-risk place to prove out the SHM machinery, pixel format, and lifecycle
model. The client ↔ daemon hop crosses a real Unix-uid trust boundary and is
where the client-observable framerate ceiling actually lives. That work is
deferred to a future phase.
Phase 1 does not reduce client-observable frame latency; it removes the
redundant CBOR re-encode/decode and pipe hop between the daemon and the
plugin-host process, and builds the segment/lifecycle/crash-safety model a
future client-facing phase can reuse.

The system is Linux-centric today, but that is not intended to be permanent. A
hand-rolled `memfd_create` + `SCM_RIGHTS` fd-passing scheme would work but is
Linux/BSD-specific by construction. It would actively foreclose that future
rather than leave it unaddressed. The transport is
[iceoryx2](https://github.com/eclipse-iceoryx/iceoryx2) (MIT OR Apache-2.0),
which supports Linux/Windows/macOS/FreeBSD/QNX and handles the crash and stale
resource recovery a hand-rolled SHM scheme would otherwise require. Only
`luminated` and `luminate-host-supervisor` depend on it; no plugin `.so` or
`libluminate` links it.

## Authorization and lifecycle model

Once a stream is negotiated (capability advertised, plugin accepted), the
grant is sticky and irrevocable by default — no idle timeout, no lease
expiry, no heartbeat-required-or-die semantics. It persists until one of
exactly two things happens:

1. An explicit control-plane message ends it (`EndShmStream`), from either
   side.
2. A party is detected as dead (process crash) — not a revocation, but a
   hard force-majeure cleanup trigger, since a stream cannot survive the
   literal death of its counterparty.

Other settled behavioural decisions:

- **Ack semantics are fire-and-forget.** `FrameAck` confirms the daemon
  published the sample, not that the plugin applied it. A persistent apply
  failure is only visible via plugin-host logs, not per-frame `FrameAck` —
  a deliberate, real regression in per-frame error observability versus the
  ordinary pipe path, accepted in exchange for keeping the full throughput
  win.
- **Partial frames against an SHM-backed stream silently fall back to the
  pipe**, per-frame. The fast path is full-frame-only: iceoryx2 needs a
  fixed max payload size per service.
- **Activation is fully automatic and transparent**, gated only by plugin
  capability advertisement plus the daemon's `prefer_shm` config knob
  (default `true`, see `DaemonConfig::prefer_shm`) — not any client-visible
  toggle. This phase does not touch the client protocol at all.
- **Any SHM setup failure, including the plugin declining, falls back to
  the pipe silently.** `BeginFrameStream`'s client-visible contract never
  regresses because SHM plumbing did not work in some environment.

## Wire format

`crates/luminate-core/src/shm_frame.rs` (dependency-light on purpose, so a
future client-facing phase can reuse it from `libluminate` without linking
iceoryx2 into the client or into plugin `.so`s):

```rust
#[repr(u32)]
pub enum ShmPixelFormat {
    Rgb8 = 0,   // 3 bytes/pixel
    Rgbw8 = 1,  // 4 bytes/pixel
    Mono8 = 2,  // 1 byte/pixel
    Rgbx8 = 3,  // 4 bytes/pixel, 1 unused padding byte (alignment-friendly)
}

#[repr(C)]
pub struct ShmFrameHeader {
    pub sequence: u64,
    pub generation: u32,
    pub pixel_count: u32,
    pub pixel_format: u32,   // ShmPixelFormat discriminant
    pub header_version: u16, // independent of PLUGIN_ABI_VERSION
    pub flags: u8,           // bit 0 = commit
    pub reserved: u8,
}
// 24 bytes, 8-byte aligned, const-asserted.
```

`ShmFrameHeader` mirrors `FrameEnvelope`'s `generation`/`sequence`/`commit`
fields so the fast path shares the ordinary path's staleness/ordering
vocabulary. It is followed immediately by `pixel_count *
pixel_format.bytes_per_pixel()` tightly packed pixel bytes.

`ShmFrameHeader::to_bytes()`/`from_bytes()` do explicit little-endian
field-by-field (de)serialization rather than a `repr(C)` pointer-cast
reinterpretation: an iceoryx2 `[u8]` payload buffer is only guaranteed byte
alignment, not the struct's natural 8-byte alignment, so reading it back
through a pointer cast would risk unaligned-access undefined behaviour.

Deferred: a packed 1-bit-per-LED format for genuinely
binary (non-dimmable) matrices. Every format above does per-pixel
`pack`/`unpack` into a fixed byte stride; a 1-bit format has no addressable
per-pixel byte slice and needs bulk buffer-level (de)serialization instead
— a real API shape change, not just a new enum variant. Deferred until a
real binary-matrix consumer exists.

## Capability negotiation

`crates/luminate-core/src/capability.rs`:

```rust
pub enum ShmFrameShape {
    Linear { pixel_count: u32 },
    Matrix { width: u32, height: u32 },
}

pub struct ShmFrameCapability {
    pub pixel_formats: Vec<ShmPixelFormat>, // preference order
    pub shape: ShmFrameShape,
    pub max_rate_hz: Option<u16>,           // overrides FrameUploadCapability::max_rate_hz if Some
}

pub struct FrameUploadCapability {
    // ...existing fields...
    pub shm: Option<ShmFrameCapability>, // None = sync-path-only
}
```

`validation.rs` checks `shape`'s pixel count is internally consistent
(`Matrix`'s `width * height` doesn't overflow and is nonzero) and that
`capabilities.frame_upload.shm.is_some()` implies the plugin exposes all
three shared-memory callbacks, both at plugin load time.

## ABI/protocol version impact

| Constant | Bumped? | Why |
|---|---|---|
| `PLUGIN_ABI_VERSION` (2 → 3) | Yes | `PluginDescriptor` gained 3 new `Option<fn pointer>` fields (a `repr(C)` layout change) and the `FrameUploadCapability`/`ShmFrameCapability` CBOR shape crossing `topology_cbor` changed. |
| `HOST_SUPERVISOR_PROTOCOL_VERSION` | No | Covers only the `SupervisorHello`/`HostHello` handshake. The `HostCommand`/`HostResponse` enum family is implicitly version-locked by the re-exec-same-binary invariant (`env::current_exe()`), not by this constant. |
| `PROTOCOL_ABI_VERSION` | No | Client ↔ daemon wire is untouched in Phase 1. |
| `POLICY_ABI_VERSION`, `LUMINATE_C_ABI_VERSION` | No | Unrelated stacks. |

## iceoryx2 topology and lifecycle

`luminated` is the Publisher + Notifier. The plugin-host child
is the Subscriber + Listener.

Each active stream has a dedicated OS thread rather than a shared `WaitSet`. The
original design considered one shared `WaitSet` per plugin-host process,
multiplexing every active stream's `Listener` on one thread. That turned
out to be unsound to implement in safe Rust:
`WaitSet::attach_notification` returns a `WaitSetGuard<'waitset,
'attachment, Service>` that borrows both the `WaitSet` and the `Listener`
it attaches. Streams begin and end dynamically, so the plugin-host would
need to store a growing/shrinking collection of owned `Listener`s and the
guards borrowing them together — a self-referential struct safe Rust
cannot express, not even via `Box`, since the guard's lifetime is tied to
the specific borrow, not the address.

Instead, `crates/luminated/src/plugin_host/shm.rs`'s `ShmRuntime` spawns
one dedicated thread per stream on `BeginShmStream`, and that thread owns
its `Subscriber`/`Listener` outright, calling `Listener::timed_wait_all()`
directly in a loop (a 250 ms poll interval, re-checking an `Arc<AtomicBool>`
stop flag on each wake) — no `WaitSet` at all on this path. This is simpler
than the shared-`WaitSet` design, not just a workaround: frame streams are
inherently few per process, so the efficiency case for multiplexing many
listeners behind one wait primitive does not apply strongly here.
`Listener::timed_wait_all()` alone already wraps the fastest native wait
primitive per platform, which is the portability property that mattered;
`WaitSet` remains available for a future phase with many concurrent
streams per process, without a transport change.

Consequently, different streams' `shm_frame` calls can run concurrently with
each other (each on its own dedicated thread) and with an in-flight ordinary
command (`apply`/`read_state`/etc., which stay on the plugin-host's single
command-processing thread). A given stream's handle is still only ever touched
by that one stream's thread, so
`ShmFrameStreamingPlugin` implementations that keep state entirely within
`Self::Stream` need no extra synchronization; implementations sharing state
across streams via `&self` do. See `PluginShmFrameApplyFn`'s and
`ShmFrameStreamingPlugin`'s doc comments for the exact contract.

The implementation uses `ipc_threadsafe::Service`, not `ipc::Service`.
iceoryx2's default `ipc::Service` marker type uses `Rc`-based internal reference
counting for performance, which is not `Send`. It cannot move a
`Subscriber`/`Listener` from the thread that creates them (the plugin-host's
command thread, during `BeginShmStream`) to the dedicated stream thread that
then owns them. `ipc_threadsafe::Service` uses `Arc` and mutex-protected internal
state instead, at a small performance cost, and is used on both the plugin-host
and daemon sides for consistency. The daemon's registry also needs `Send`/`Sync`
because it is called from `tokio::task::spawn_blocking` jobs on a multi-threaded
runtime.

`crates/luminate-host-supervisor/src/shm.rs`'s
`service_names(plugin_name, target_path)` derives two deterministic
iceoryx2 service names (one publish-subscribe, one event) from a plugin
name and a rendered target string, independently on both sides — no name
crosses the wire. Both inputs are percent-escaped before joining so an
embedded `/` (for example from `PluginTarget`'s own `Display` rendering)
can never collide with the joiner's own `/`.

The publish-subscribe service's `initial_max_slice_len`
is `size_of::<ShmFrameHeader>() + shape.pixel_count() *
format.bytes_per_pixel()`, computed once from the negotiated
`ShmFrameCapability` and echoed back to the daemon in
`ShmStreamOutcome::Ready { segment_bytes }`. Subscriber buffer size 1 with
`enable_safe_overflow(true)` — "latest wins, drop older", matching the
ordinary path's own rate-limit-drop posture rather than queuing.

The begin sequence is triggered when `Request::BeginFrameStream` succeeds and
the target's capability advertises `shm` (gated by `prefer_shm`):

1. `DaemonState::begin_frame_stream` runs exactly as today, unmodified.
2. The daemon picks the first pixel format it recognizes from the
   capability's preference list and sends `HostCommand::BeginShmStream`
   over the *existing pipe* — negotiation is one-shot; high-frequency
   payloads move to SHM, not the handshake.
3. The plugin-host calls the plugin's `shm_stream_begin`; on acceptance it
   opens both iceoryx2 services and spawns the stream's dedicated thread,
   replying `Ready { segment_bytes }` or a typed decline/failure.
4. The daemon opens the same two services, creates its Publisher+Notifier,
   and records the stream in `crates/luminated/src/plugins/shm.rs`'s
   `ShmPublisherRegistry`. **Any failure at any step falls back to the
   pipe for that stream silently** — `BeginFrameStream`'s client response
   never regresses.

`Request::UploadFrame` still runs
`DaemonState::record_frame_upload` unmodified. If the target is
SHM-backed, `PluginManager::try_apply_shm_frame` loans a sample, packs the
header and every `Colour`, sends, and notifies — returning success
immediately (fire-and-forget). A `false` return (no active stream, a
`Partial` envelope, or delivery failure) falls through to the ordinary
`apply_frame` pipe call for that one frame.

`Request::EndFrameStream` (unmodified wire shape) additionally
triggers `HostCommand::EndShmStream` when SHM-backed, gated by a cheap
local `has_active_shm_stream` check so the common (never-SHM) case pays no
extra plugin-host round trip. Both sides drop their
Subscriber/Publisher/service handles symmetrically. Idempotent and
generation-gated, mirroring `DaemonState::end_frame_stream`. Connection-drop
cleanup (`end_all_frame_streams`) gets a matching `end_all_shm_streams`,
which force-ends by target without a generation to match against (there is
none to have at disconnect time).

`ResetShmStream` is fully plumbed at the protocol and plugin-host levels
(`ShmRuntime::reset` bumps the locally-remembered generation without
touching iceoryx2 objects, mirroring how the ordinary path already treats a
generation bump as a lightweight "reset your buffering state" signal). **No
daemon-side trigger exists yet** — there is currently no wedge-detection
heuristic that would call it. `HostedPlugin::reset_shm_stream` carries a
narrow `#[allow(dead_code)]` pointing back to this section rather than
being deleted, since it is a real part of the protocol contract shared with
the plugin host, just not yet exercised by any caller.

## Crash and respawn handling

- **Daemon dies while the plugin-host holds a Subscriber:** already handled
  by the existing supervised-child architecture — the plugin-host's stdin
  read loop hits EOF and the process exits. Not an iceoryx2 concern.
- **luminated itself restarts** (e.g. respawned by systemd) with an
  orphaned segment still on disk: iceoryx2's `open_or_create()` recognizes
  and cleans up a previous instance's leftover resources, rather than
  erroring — this is iceoryx2's documented self-healing behaviour, not
  something Phase 1 implements itself.
- **Plugin-host dies and respawns while the daemon holds a Publisher:**
  detected via the existing `HostedPlugin`/`HostConnection` respawn
  mechanism (`HostedPlugin::call`). Phase 1 adds a `connection_epoch:
  AtomicU64` to `HostedPlugin`, bumped on every respawn. Each
  `ActiveShmStream` records the epoch it began under;
  `ShmPublisherRegistry::apply` checks the current epoch against it before
  sending and drops the stale local registry entry on mismatch rather than
  publishing into an orphaned segment nobody is listening to any more.
  This closes a real correctness gap that a fire-and-forget transport would
  otherwise hide silently: `sample.send()`/`notifier.notify()` do not error
  just because there is no subscriber, so without the epoch check a
  post-respawn frame would report success while going nowhere.
  Phase 1 does not implement transparent
  re-negotiation of a fresh SHM segment after a respawn. Today, once a
  stream's epoch goes stale, that stream safely falls back to
  the pipe for the rest of its life — correct, but short of the ideal
  where the daemon re-negotiates automatically and the client never
  notices at all. A future phase can add that without changing the wire
  contract; the epoch field already provides the signal it would need.

## Plugin-facing API

`crates/luminate-plugin-api/src/descriptor.rs` adds three ABI fn-pointer
types (`PluginShmStreamBeginFn`/`PluginShmFrameApplyFn`/
`PluginShmStreamEndFn`) and matching `Option<...>` fields on
`PluginDescriptor`, validated all-or-nothing at load time. The ABI and
safe-trait `shm_stream_begin` signatures carry a flat `pixel_count`, not
the full `ShmFrameShape` — the plugin already knows its own shape (it
advertised it via `topology_cbor`), so `shape` is negotiation-time-only
metadata for the daemon side.

The `handle` a plugin mints in `shm_stream_begin` is a raw `Box<P::Stream>`
pointer cast to `u64`, minted/dereferenced/reclaimed across the three
calls — chosen over a `Mutex<HashMap<u64, Box<Stream>>>` registry because
this handle only ever flows plugin-host → plugin.so, already the same trust
domain (unlike plugin.so → daemon, the direction that actually needs to
defend against adversarial input).

`crates/luminate-plugin-api/src/sdk/traits.rs` adds `ShmFrameStreamingPlugin:
FrameStreamingPlugin` (a plugin may implement both, or only
`FrameStreamingPlugin`) and `sdk/export.rs` extends `luminate_export_plugin!`
with a
mandatory `shm_frame: none|native` argument, mirroring `frame_upload`'s
existing `none|native` slot. This is a mechanical, one-line change to
every plugin crate's macro invocation regardless of whether it implements
the trait; only `luminate-plugin-demo-display` sets `shm_frame: native`.

## Testing

- **Pure unit tests:** `ShmPixelFormat::pack`/`unpack` round trips and
  rejection cases; `ShmFrameHeader` byte-layout/size/align const
  assertions and explicit little-endian (de)serialization round trips;
  `service_names()` collision/escaping edge cases; `validation.rs`
  contract tests for the all-or-nothing callback check and shape
  validation.
- **In-process iceoryx2 integration tests**
  (`crates/luminated/src/plugin_host/shm.rs`'s `tests` module): a real
  `ShmRuntime` negotiates a stream against synthetic ABI callbacks, and the
  test itself acts as the daemon side — opening the same two services by
  name, publishing a real sample through real shared memory, and polling
  (bounded, not a wall-clock assumption) for the dedicated stream thread to
  observe it. This exercises the actual per-stream-thread mechanism end to
  end, not a mock. A newly-created publisher connecting to an
  already-existing subscriber for the first time can race iceoryx2's
  connection establishment, so the test retries publishing rather than
  assuming the first sample lands — the same tolerance a real multi-frame
  stream already has for free.
- **Plugin-host dispatch tests** (`runtime/tests.rs`): drive
  `NativePlugin::handle` directly through `HostCommand::BeginShmStream`/
  `EndShmStream`/`Shutdown`, including confirming `Shutdown` ends every
  active stream gracefully via `ShmRuntime::end_all`.
- **`luminate-plugin-demo-display`'s own unit tests:** topology/capability
  shape, both frame-streaming paths' validation, and the
  begin/apply/end lifecycle directly against the trait methods.
- **Manual end-to-end smoke test:** run `luminated` with
  `luminate-plugin-demo-display` loaded, drive `BeginFrameStream`/
  `UploadFrame` × 5/`EndFrameStream` via `libluminate::Client`, and confirm
  from two independent signals that the fast path was actually used, not
  silently falling back: the daemon's own `shared-memory frame stream
  active` log line reports `segment_bytes=792` (24-byte header + 256 × 3
  RGB8 bytes, exactly as expected for the 16×16 matrix), and the plugin's
  `shm_stream_end` log reports `frames_applied=5` from a counter that only
  increments inside `ShmFrameStreamingPlugin::shm_frame`, never the
  ordinary CBOR path.
- **Deliberately deferred:** a real-subprocess SIGKILL-mid-stream test
  (kill a real plugin-host child while an SHM stream is active and assert
  detection/cleanup). The connection-epoch mechanism above is the
  correctness property such a test would validate, and it is covered by
  code-level reasoning and the existing `HostedPlugin` respawn tests
  (which exercise the *general* respawn-and-restart mechanism, just not
  specifically while an SHM stream is active) rather than a new
  purpose-built harness. Worth adding if this path sees real production
  traffic.

## Affected crates

| Crate | Changes | ABI |
| --- | --- | --- |
| `luminate-core` | `shm_frame.rs` (new); `ShmFrameShape`/`ShmFrameCapability` and `FrameUploadCapability::shm` in `capability.rs` | Both (see version table above) |
| `luminate-plugin-api` | Bump `PLUGIN_ABI_VERSION` 2 → 3 | Plugin 2 → 3 |
| `luminate-host-supervisor` | New `iceoryx2` dependency; `shm.rs` service-naming helper | None |
| `luminate-plugin-api` | `PluginShmStreamBeginFn`/`PluginShmFrameApplyFn`/`PluginShmStreamEndFn` and matching `PluginDescriptor` fields; `shm.rs` raw ABI dispatch helpers; `ShmFrameStreamingPlugin` trait and macro support in `sdk/` | Plugin 2 → 3 |
| `luminated` | New `iceoryx2` dependency; `plugin_host/shm.rs` (child-side `ShmRuntime`, new); `plugins/shm.rs` (daemon-side `ShmPublisherRegistry`, new); `plugin_host/protocol.rs`, `plugin_host/runtime/mod.rs`, `plugin_host/supervisor.rs`, `plugin_host/validation.rs`, `plugins/streams.rs`, `daemon/dispatch/streaming.rs`, `daemon/connection.rs`, `daemon/listener.rs`, `device_config.rs` (`prefer_shm`) | None (see version table) |
| `luminate-plugin-demo-display` | New crate; the only bundled plugin implementing `ShmFrameStreamingPlugin` | N/A |
| Every other plugin crate | Mechanical `shm_frame: none` in `luminate_export_plugin!` | None |

`crates/luminated/src/state/frame_stream.rs` and the client ↔ daemon wire
protocol (`luminate-protocol`) are deliberately untouched — see the scope
boundary at the top of this document.

## Decisions and deferred work

- **Per-stream dedicated threads, not a shared `WaitSet`:** the safe-Rust
  soundness argument above, not a performance decision — see "iceoryx2
  topology and lifecycle".
- **`ipc_threadsafe::Service` everywhere:** required on the plugin-host
  side by the per-stream-thread design; used on the daemon side too for
  consistency and because its registry is called from
  `spawn_blocking` jobs on a multi-threaded runtime regardless.
- **Fire-and-forget delivery:** the deliberate throughput/observability
  trade-off described above. A persistent plugin-side apply failure is
  only visible via logs.
- **Connection-epoch gating, not transparent re-negotiation:** closes the
  silent-data-loss gap after a plugin-host respawn without the added
  complexity of automatic segment re-creation. See "Crash and respawn
  handling".
- **`ResetShmStream` has no caller yet:** plumbed for protocol completeness
  and future use, not exercised by any current heuristic.
- **Client ↔ daemon SHM streaming:** out of scope for this phase entirely.
  The wire-format and lifecycle model here are designed to be reusable by
  that future phase, but no client-facing work has started.
- **1-bit-per-LED pixel format:** deferred until a real binary-matrix
  consumer exists; would need a bulk buffer-level (de)serialization path
  alongside `ShmPixelFormat`'s existing per-pixel one.
