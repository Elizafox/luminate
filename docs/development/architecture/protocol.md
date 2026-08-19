<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Internal protocol: `luminated` ↔ `libluminate`

This document describes the Unix-socket wire protocol between `luminated` and
`libluminate`. It is an internal interface that may change with the daemon. The
stable public interfaces are the Rust and C APIs exposed by `libluminate`.

This is not a hardware protocol document. For HID-level wire formats, see
`docs/design/hardware/alienware/`.

Audience: `luminated` and `libluminate` contributors. External clients
should use the `libluminate` Rust API or generated C header at
`crates/libluminate/luminate.h`. Wire shapes, error codes, and compatibility
rules may change without a deprecation period. Such changes require a
`PROTOCOL_ABI_VERSION` bump.

## Transport and framing

Both sides use the framing implementation in
`crates/luminate-protocol/src/framing.rs`.

Each frame is:

```
u32 length (big-endian, via tokio's write_u32/read_u32)
<length> bytes of CBOR-encoded payload (ciborium)
```

- `receive` reads and validates the length prefix before allocating the
  payload buffer, rejecting anything over `MAX_FRAME_LEN` (8 MiB) as
  `FramingError::FrameTooLarge`. This bounds worst-case per-connection
  allocation from an untrusted length prefix.
- The connection is a single `UnixStream`. Every request and response carries
  a connection-local `u64` request ID so several calls may be in flight (see
  "Concurrency and connection state" below).

## Connection lifecycle

1. Client opens a `UnixStream` to the daemon's socket
   (`luminate_platform::default_path::default_socket_path`, or an explicit path via
   `Client::connect_path`).
2. Client sends `ClientHello` (its handshake message).
3. Daemon reads the hello frame under a 5-second timeout
   (`HANDSHAKE_TIMEOUT` in `luminated::daemon`); an idle peer that never
   sends one has its connection task dropped rather than held forever.
4. Daemon replies with `DaemonHello`.
5. If `DaemonHello.compatibility` is `Incompatible`, the daemon closes the
   connection immediately after sending the hello. No request loop starts.
   The client surfaces this as `Error::IncompatibleDaemon`.
6. If `Compatible`, both sides enter a request/response loop: the client sends
   `RequestMessage` frames and the daemon returns one `ResponseMessage` with
   the same ID for each request.
   An established connection may remain idle for up to 30 minutes between
   requests. Once the first byte of a request frame arrives, the remainder must
   arrive within the framing I/O timeout, so a partial-frame peer cannot occupy
   a connection slot until that longer idle deadline.
7. The loop ends when the client disconnects. The daemon's read side treats an
   `UnexpectedEof` as a clean disconnect (return `Ok(())`, not an error); any
   other framing error tears down the connection task with a logged warning.

### Handshake payloads

`ClientHello` (`crates/luminate-protocol/src/handshake.rs`):

```rust
struct ClientHello {
    protocol_abi_version: u32,
    client_name: String,
    client_version: String,
}
```

`libluminate` always sends its own crate name/version
(`env!("CARGO_PKG_NAME")`/`CARGO_PKG_VERSION`) and the `PROTOCOL_ABI_VERSION`
it was built against. The daemon logs `client_name`/`client_version`/
`protocol_abi_version` on every connection but does not otherwise act on the
name/version strings. They exist for daemon-side observability once more than
the CLI speaks the protocol.

`DaemonHello`:

```rust
struct DaemonHello {
    compatibility: Compatibility,
    protocol_abi_version: u32,
    daemon_version: String,
}

enum Compatibility {
    Compatible,
    Incompatible { supported_protocol_abi_version: u32, reason: Option<String> },
}
```

### Compatibility model

The daemon requires an exact `protocol_abi_version` match
(`hello.protocol_abi_version == PROTOCOL_ABI_VERSION`, in
`luminated::daemon::handle_connection`). This is a hard gate, not
semver-style negotiation. `PROTOCOL_ABI_VERSION` (`luminate-protocol`) is
bumped whenever a change could make an old client and a new daemon (or vice
versa) misinterpret each other's messages: added/removed/reordered
enum variants or fields carried over the wire, a framing change, or any
non-purely-additive change. See the doc comment on
`luminate_protocol::PROTOCOL_ABI_VERSION` for the authoritative bump rule and
current value.

There is no partial-compatibility mode: a mismatched client gets exactly one
`DaemonHello` telling it the daemon's supported version and an optional
human-readable `reason`, then the connection closes.

## Requests and responses

Here, a **facet** means one independently reportable part of state (appearance,
brightness, local emission, physical power, or the daemon-derived effective
appearance combining the first two). An **observation** is timestamped
knowledge of a hardware facet, with confidence and freshness; it is deliberately
separate from a desired command. `Adopt` stores confirmed observations in an
**adopted baseline** below explicit desired overlays, while `Restore` composes
and writes those layers. See
[`state-reconciliation.md`](state-reconciliation.md) for the longer vocabulary
and invariants.

`RequestMessage` wraps each `Request` with its correlation ID:

```rust
struct RequestMessage { id: u64, request: Request }
```

`Request` (`crates/luminate-protocol/src/message.rs`):

```rust
enum Request {
    Ping,
    ServerInfo,
    ListDevices,
    GetDevice { id: DeviceId },
    GetState { device: DeviceId },
    GetCollectionState { collection: CollectionId },
    RefreshState { device: DeviceId },
    SetEffect(SetEffectRequest),       // { selector: Selector, effect: Effect, on_unsupported }
    SetBrightness(SetBrightnessRequest), // { target: Selector, value: u32, on_unsupported }
    RestoreAppearance { target: Selector },
    ClearTarget { target: Selector },
    SaveCurrent { target: Selector },

    // Collection CRUD and standing state.
    CreateCollection { .. },
    DestroyCollection { id: CollectionId },
    AddCollectionMember { id: CollectionId, member: CollectionMember },
    RemoveCollectionMember { id: CollectionId, member: CollectionMember },
    ListCollections,
    GetCollection { id: CollectionId },

    // Frame streaming: see "Frame streaming" below.
    BeginFrameStream { target: TargetId },
    UploadFrame { target: TargetId, envelope: FrameEnvelope },
    EndFrameStream { target: TargetId, generation: u32 },
}

enum Selector {
    Target(TargetId),
    Collection(CollectionId),
    Targets(Vec<TargetId>),
}
```

> Collection request/response shapes and the `CollectionApplied` best-effort
> response are documented here. Authorization semantics are documented in
> [`client-authorization.md`](client-authorization.md).

`Selector::Targets` carries a concrete, already resolved leaf set. A
multi-principal front end uses it after authorizing a collection snapshot so a
concurrent membership change cannot add a target between its decision and
daemon execution. The daemon still applies its own process-principal policy to
every supplied leaf and returns `CollectionApplied`.

`ResponseMessage` echoes the ID and wraps the operation response:

```rust
struct ResponseMessage { id: u64, response: Response }

struct Response { status: ResponseStatus }

enum ResponseStatus {
    ServerInfo(ServerInfo),
    Devices(Vec<Device>),
    Device(Option<Box<Device>>),
    State(Option<Box<DeviceStateStatus>>),
    Ack,

    // Frame streaming: see "Frame streaming" below.
    FrameStreamStarted { generation: u32 },
    FrameAck { sequence: u64, dropped: bool },

    // Collections.
    CollectionCreated { id: CollectionId },
    Collections(Vec<Collection>),
    CollectionInfo(Option<Box<Collection>>),
    CollectionApplied { applied: Vec<TargetId>, denied: Vec<TargetId> },

    Error { code: ErrorCode, message: String },
}
```

Every request has exactly one legal response shape (`Ping` → `Ack`,
`ServerInfo` → `ResponseStatus::ServerInfo`, the mutation requests → `Ack` or
`Error`, etc.); `libluminate::Client` methods match narrowly on the expected
variant and treat every other non-`Error` shape as a protocol violation
(`Error::Protocol`, via `unexpected_response`) rather than silently
coercing it.

`Device(Option<Box<Device>>)` is `Box`ed (not `Device` inline) purely to
keep `ResponseStatus`'s largest variant under clippy's `large_enum_variant`
threshold as `Device` grew `category`/`notes`/`warnings` fields. No wire or
semantic significance beyond that.

### Request → daemon-side behaviour

`luminated::daemon::dispatch::dispatch_request` authorizes each `Request`,
then hands it to the module under `daemon/dispatch/` that owns its category:
`inspection` (read-only queries), `admin` (rescans and plugin unload/reload),
`management` (managed configuration), `appearance` (the mutations below),
`collections`, and `streaming` (frame streams). Every handler receives a
`DispatchContext` carrying the connection's dependencies alongside the
management configuration read for that request, so a committed management
patch takes effect on the next request rather than the next connection.

The variants map onto `DaemonState`/`PluginManager` as follows:

- `Ping` — returns only `Ack` on the existing primary connection. It creates
  no session or connection, reads no daemon state or metadata, and bypasses
  resource authorization because it authorizes no action. A per-connection
  token bucket permits a burst of four requests and replenishes one token per
  second; excess requests return `RateLimited` with retry guidance.
- `ServerInfo` — no state lock, returns daemon name/version/ABI version
  directly.
- `ListDevices`/`GetDevice` — read the locked topology, no plugin call.
- `GetState` — returns observed facets, confidence/freshness, reachability,
  reconciliation, adoption status, and the latest diagnostic separately from
  desired replay state. `RefreshState` runs the plugin's validated bulk
  snapshot path first and reports `UnsupportedCapability` when the device
  advertises no readable facets.
- `GetCollectionState` — synthesizes configured and effective appearance
  across a collection's transitive membership. Homogeneous members return
  their shared value, heterogeneous members return `Mixed`, and insufficient
  constituent knowledge remains unknown.
- `SetEffect`/`SetBrightness`/`ClearTarget`/`SaveCurrent` — go
  through `mutate`, which:
  1. Acquires every affected device sequencer in stable device-ID order and
     reserves a generation before hardware I/O. Locks `DaemonState` briefly to
     validate the target plus its advertised colour, brightness, effect, scope,
     channel, and value constraints
     (`ensure_target_state_known` additionally applies to `SaveCurrent`)
     before touching the plugin, so unsupported or malformed input never
     reaches plugin FFI.
  2. Sends the operation to the owning plugin's supervised child host. Calls
     are serialized per plugin and have a hard deadline; different plugins can
     make progress independently.
  3. On success, appends/compacts the in-memory desired overlays, publishes
     `Assumed` observations for projected facets, and persists them. A global
     commit coordinator orders whole-state saves while unrelated devices may
     still perform hardware I/O concurrently.
  4. Runs blocking mutation work outside the async reactor. The global state
     lock is held only for validation, commit, and persistence snapshots—not
     while waiting for plugin IPC—so topology reads remain responsive.
     If persisting the updated state fails after a successful hardware
     mutation, the daemon logs the error and returns a client-visible `Internal`
     error rather than an `Ack`. The failure is surfaced, not swallowed. The
     hardware change already took effect and the applied overlay stays in the
     in-memory state, which is now dirty relative to the on-disk file;
     persistence is retried on the next successful mutation. So a client that
     receives `Ack` knows the change is durably saved, while an `Internal` error
     here means "applied to hardware but not yet persisted."

State reconciliation uses `Restore`, `Adopt`, or `Leave`. Readback values cross
the plugin-host trust boundary as a bounded `PluginStateSnapshot`; the daemon
rejects a malformed snapshot before publishing any item. Exact facets can be
staged into the separate adopted physical baseline. The adopted delta is merged
into the latest whole-state snapshot under the same commit coordinator and is
published as durable only after the atomic save succeeds.

`SaveCurrent` additionally short-circuits to a trivial `Ack` (no plugin FFI
call at all) when the target's `PersistenceCapability` requirement is
`Required`. Write-through hardware has no "unsaved" state to save. The
[plugin authoring guide](../plugin-authoring.md#persistence) documents the
full persistence contract, including explicit `SaveCurrent` handling for
devices that can commit volatile state.

- `SetEffect`/`SetBrightness`/`ClearTarget`/`SaveCurrent` accept a
  selector: a concrete target, or a collection. A collection selector resolves
  to its (transitive) leaf targets; unlike a concrete target, a
  collection-targeted write authorizes each leaf's owning device
  independently and applies best-effort to the authorized subset, returning
  `ResponseStatus::CollectionApplied { applied, denied }` instead of `Ack`.
  `on_unsupported` (`None` uses the daemon's `default_unsupported_policy`,
  which defaults to `Skip`) governs a leaf that can't apply the state: `Skip`
  applies to the capable ones and leaves the rest untouched, `Reject` fails
  the whole command up front unless every resolved leaf can apply it.

### Frame streaming

A target that advertises `FrameUploadCapability` accepts a synchronous
ack-per-frame stream over the same request/response socket used for every
other mutation — there is no separate streaming channel. `BeginFrameStream`
returns a `generation` the caller echoes in every subsequent `UploadFrame`/
`EndFrameStream` on that stream. `DaemonState` rejects a `BeginFrameStream`
if the target has no frame-upload capability (`Unsupported`), already has an
active stream (`Conflict`), or has an active hardware effect not marked
`concurrent_with_streaming` (`Conflict`). `UploadFrame` rejects a mismatched
generation or a non-increasing `sequence` (`Conflict`/`InvalidArgument`
respectively — both protect against reordered or duplicate requests, not
just malicious clients), and silently drops (not errors) a frame that
arrives faster than the target's declared `max_rate_hz`: the client gets
`FrameAck { dropped: true }` and the frame never reaches the plugin.
`EndFrameStream` is idempotent — ending an inactive or already-superseded
stream is not an error, since disconnect cleanup can race an explicit end.

A stream is also bound to the authorization-relevant topology generation at
`BeginFrameStream`. Replacing that topology terminates the old daemon-side
stream. Its next upload returns `Conflict`; the client must refresh topology
and begin a new stream rather than assuming a same-ID device has the same
provider, attachment, collection membership, or capabilities.

Frame streaming deliberately bypasses `mutate`/the persisting commit
coordinator entirely: persisting a full state-file snapshot per frame would
be unworkable at streaming rates. Streamed frames update only lightweight
in-memory stream bookkeeping (generation, sequence, rate-limit timestamp) —
never `target_state`, never the on-disk state file, never `GetState`. A
connection tracks every stream it has started and ends them all when the
connection closes, regardless of whether it closed cleanly or via I/O error.

`FrameEnvelope`'s `generation`/`sequence` fields are deliberately
delivery-mechanism-agnostic: this synchronous request/response transport is
sufficient for every bundled plugin's actual hardware path today (all far
slower than a local Unix-socket round trip), which is why the same
envelope shape carries over unmodified into the fast path described next.

A target may additionally advertise a zero-copy shared-memory fast path
(`FrameUploadCapability::shm`, a `ShmFrameCapability`) alongside the
ordinary path above. The daemon negotiates it transparently — there is no
separate client-visible request, and `BeginFrameStream`'s response never
changes shape either way — and falls back to the socket path silently for
any frame or stream the fast path can't handle. The client ↔ daemon socket
protocol is unmodified either way. The fast path only replaces the
daemon ↔ plugin-host pipe hop, a separate trust domain not covered by this
document. See
`docs/development/architecture/shm-frame-streaming-phase1.md` for the full
design; `docs/development/plugin-authoring.md`'s "Frame streaming" section
covers the plugin-facing contract.

### Error codes

```rust
enum ErrorCode {
    DaemonUnavailable,
    PermissionDenied,
    NotFound,
    Unsupported,
    UnknownState,
    InvalidArgument,
    Internal,
    Io,
    Unavailable,
    RateLimited,
    PartialMutation,
    // A frame stream or hardware effect already owns the target in a way
    // incompatible with the requested operation (e.g. starting a stream
    // where a non-concurrent effect is active, or starting a second stream
    // on a target that already has one).
    Conflict,
}
```

Managed configuration uses `Conflict` for optimistic-concurrency failures. A
client reads `GetManagement`, submits `PatchManagement` with the snapshot's
revision, and retries from a fresh snapshot if another transaction committed
first. Successful patches return a redacted change set only after the new
revision is durable.

Daemon side: `DaemonError` (`crates/luminated/src/error.rs`) is a typed
`thiserror` enum carried through `state.rs`/`plugins.rs`; each variant maps
structurally to one `ErrorCode` via `DaemonError::error_code()`, with no
string-matching. `map_error` (`luminated::daemon`) builds the wire response, and
`ResponseStatus::Error` carries one structured `OperationError`: a stable
`ErrorCode`, a human-readable message, optional `retry_after_ms` guidance, and
the concrete targets already changed by a partial fan-out. Consumers must use
the structured fields for control flow and treat the message as a diagnostic.

Client side: `libluminate::Error` (`crates/libluminate/src/error.rs`) has
one variant per `ErrorCode` (`Error::from_error_code`) plus client-local
variants that never come from the wire: `Error::Protocol` (framing/CBOR
failure or an unexpected response shape), `Error::ConnectionPoisoned` (see
below), and `Error::IncompatibleDaemon` (handshake-time only).

Hardware-facing mutation failures preserve temporary absence and rate limiting
as `Unavailable` and `RateLimited` respectively. A provider-supplied minimum
retry delay crosses the plugin ABI and consumer protocol without being embedded
in text. A collection fan-out that changes hardware before a later failure is
`PartialMutation`, with the committed successful targets in `applied_targets`.
Frontends can therefore offer appropriate recovery guidance without parsing a
plugin diagnostic or treating a temporarily offline light as an
undifferentiated I/O failure.

`RestoreAppearance` reapplies each selected target's retained `Appearance`
observation. Stale observations are accepted deliberately: this operation asks
for the last-known configuration, not a claim about current hardware. Groups
resolve all canonical members before the first write. A uniform appearance is
coalesced through the group target when its capabilities can express it;
otherwise members are restored individually. A collection restores each
authorized leaf's own appearance and returns `CollectionApplied`, including
any denied leaves. Missing or unusable appearance state anywhere in the
authorized selection fails with `UnknownState` before any write. A later
member failure is `PartialMutation` and reports the targets already changed.

Protocol ABI 11 added `RestoreAppearance`. This changes the serialized
`Request` enum and is intentionally incompatible with protocol ABI 10.

Protocol ABI 12 changed `RestoreAppearance::target` from `TargetId` to
`Selector`, adding collection restoration. This is intentionally incompatible
with protocol ABI 11.

Protocol ABI 15 added `Selector::Targets` for explicit resolved-leaf
mutations. This is intentionally incompatible with protocol ABI 14.

Protocol ABI 19 adds persistent scene list, lookup, authoring, capture,
revisioned replacement/deletion, and immediate application requests. Dynamic
bindings carry captured targets rather than a rule for synthesizing future
members. `SceneApplied` reports the authorized targets applied and denied.
The event protocol ABI is 5 so `ScenesChanged` can invalidate cached scene
lists independently of topology and state.

`DaemonUnavailable` is unusual: it's a real `ErrorCode` variant, but in
practice `libluminate` produces it locally from a connection-level
`std::io::Error` (`NotFound`/`ConnectionRefused`) rather than receiving it
over the wire. The daemon has no request-handling path that returns it,
since if the daemon can respond at all, it's available by definition.

## Concurrency and connection state

- **Several requests may be in flight per `Client`.** A background writer owns
  the socket's write half and serializes complete request frames. A background
  reader owns the read half and routes each response through a pending-request
  map keyed by its ID.
- **Cancellation is local to one request.** Dropping or timing out a request
  future removes its pending response sender. The background tasks still
  finish any queued write and read complete response frames, so a late response
  is safely ignored and other requests continue normally. A framing or socket
  failure closes all pending requests because the connection itself is no
  longer usable.
- **One `luminated` connection task per client**, spawned into a `JoinSet`
  in the accept loop. The daemon admits at most 128 primary connections
  globally and 16 from one Unix peer UID; both permits return to their pools
  when the task ends. On `SIGTERM`/`SIGINT`, the daemon stops accepting new
  connections and gives in-flight ones up to 10 seconds
  (`SHUTDOWN_DRAIN_TIMEOUT`) to finish before exiting anyway.
- The daemon's global `DaemonState` is one `Arc<Mutex<DaemonState>>` shared
  across all connections. Mutation requests from different clients serialize
  against each other at the daemon, independent of the per-`Client`
  stream-level serialization above.

## What is deliberately not in this protocol yet

- **No targeted topology fetch below `GetDevice`.** There is no
  surface/element/group-scoped request. A client that wants one surface
  still calls `GetDevice` and filters client-side. Whether this should become
  a first-class wire operation remains an API investigation rather than a
  protocol commitment.
- **No event payload snapshots.** The dedicated event socket publishes bounded
  topology and state dirty bits. Clients still fetch authoritative data over
  the primary socket.
- **No parallel daemon execution within one connection.** Request IDs let one
  `Client` safely have several calls in flight and make cancellation local, but
  the daemon still dispatches frames from that connection in receive order. A
  client that needs operations to execute concurrently uses multiple
  connections.

## Event subscription

Plugins can report topology changes after daemon startup. LIFX discovery was
the first user of this path, but it applies equally to hot-plugged USB or HID
devices.

### Plugin → daemon

The `init` callback receives a topology notification function:
`init(log, max_level, notify_topology_changed: PluginNotifyFn)`, where
`PluginNotifyFn` follows the same self-identifying convention as the
existing `PluginLogFn` (`extern "C" fn(plugin_name: *const c_char)`,
not an opaque context pointer). A plugin calls it from its discovery thread
when its device set changes. The callback carries no topology data; it only
marks the plugin dirty.

The daemon waits 300 ms to coalesce a burst of notifications, then pulls a
complete snapshot from `topology_cbor()`. It validates the graph through the
same path used at startup, diffs it against the previous snapshot, and rebuilds
ownership atomically. An absent stable device ID means that device was
withdrawn. This full-snapshot design avoids transient dangling group references
that per-device announce and withdraw calls could create.

Plugins remain responsible for discovery hysteresis. Packet loss or a brief
network failure should not produce a topology notification. Address changes
also leave topology untouched because network devices use stable hardware IDs,
not IP addresses, as their identity.

### Race-free subscription

A client can miss a change if it fetches `ListDevices` before opening the event
socket. The safe order is:

1. Open the event socket and complete the subscription handshake.
2. Fetch the device baseline on the primary socket.

The daemon registers the subscriber before acknowledging the handshake. A
change that happens earlier appears in the baseline; a later change produces an
event. A redundant event is possible but harmless. `libluminate` hides this
ordering in `subscribe_with_baseline()` and the corresponding C helpers.

### Withdrawn-device state

A withdrawn device keeps its persisted state. The daemon cannot distinguish a
brief outage from permanent removal. If the device returns, reconciliation
applies the retained state according to the normal authority and write-through
rules before publishing the topology event. Operators can permanently remove
that retained data with `luminatectl purge-withdrawn --device ID`. The daemon
rejects active devices, so this administrative operation cannot silently clear
live desired state. It removes desired overlays, adopted baselines, and
related runtime diagnostics, then atomically persists the new snapshot.

### Hardware rescanning

`Request::Rescan` asks the daemon to re-enumerate every plugin's hardware and
reconcile whatever changed, the same work it performs on resume from suspend.
It names no target and is classified as daemon administration, alongside
`PurgeWithdrawnDevice`.

It is acknowledged as soon as the rescan is scheduled, not once it finishes:
re-enumerating network hardware can outlast any reasonable request timeout, and
the daemon's topology coordinator owns that work end to end. Clients that need
to know what changed watch the resulting `TopologyChanged`/`StateChanged`
events, exactly as they would for a hotplug.

Exposed as `luminatectl rescan` and `Client::rescan()`. `SIGUSR1` triggers the
same work for callers that cannot speak the protocol. See
[`suspend-resume.md`](suspend-resume.md).

### Plugin configuration and product-capability lookup

Each explicit `[[plugins]]` entry may carry a plugin-defined `[plugins.config]`
table. The daemon encodes it directly as CBOR, sends those opaque bytes to the
isolated child over a bounded bootstrap frame, and the plugin ABI copies it during `init`
before the plugin's Rust startup hook or `probe` runs. The object is immutable
for one daemon configuration and is resent unchanged whenever a failed host is
restarted. Autoloaded plugins receive an empty object. Configuration values are
not put in command-line arguments, environment variables, logs, or plugin
metadata.

Product capability tables, such as the LIFX registry, remain embedded in each
plugin so ordinary LAN control does not depend on internet access. A plugin
that later offers an opt-in registry refresh should expose that switch through
its typed configuration schema and retain embedded conservative behaviour when
the refresh is disabled or fails.

### Daemon → client

Events use a second socket because they are an unsolicited stream rather than
responses to control requests. By default, the event socket's
path is the primary socket path plus `.events`; daemon config may set
`event_socket_path`, and `LUMINATED_EVENT_SOCKET_PATH` overrides it at runtime.
A client opens it, sends a small subscribe handshake instead of the primary
socket's `ClientHello`/`DaemonHello`, then reads a stream of `Event` frames with
no requests from the client. Existing clients never open this socket.

An event is a prompt to refresh, not a topology diff or state snapshot:

```rust
enum Event {
    ResyncRequired,
    TopologyChanged { devices: Vec<DeviceId> },
    StateChanged { devices: Vec<DeviceId> },
    ConfigurationChanged { changes: ManagementChangeSet },
    ShmStreamEnded { target: TargetId, generation: u32 },
}
```

A subscribed client calls `ListDevices`, `GetDevice`, or `GetState` again on
its normal connection for authoritative data; `Event` only tells it when to
bother. `StateChanged` covers all committed client-visible state changes,
including desired-state mutations, hardware refreshes, reconciliation,
adoption, clearing state, and collection-derived state changes. The daemon
publishes it only after the mutation's durable commit, so a woken client cannot
fetch the previous value.

`ConfigurationChanged` identifies changed daemon preference names, plugin
activation, and plugin setting keys. It never carries setting values, whether
or not the setting is sensitive. Clients use `GetManagement` for the
authoritative redacted snapshot.

`ShmStreamEnded` tells a client-published shared-memory stream to stop after
the daemon has ended that stream. Because the event socket is a separate
connection with no primary-connection identifier, this event cannot currently
be targeted to only the primary connection that opened the stream.

Before delivery, the daemon projects topology and state device lists through
the subscriber's `Observe` authorization. It suppresses an event when every
named device is denied. Empty full-refresh markers require resource-free
`Observe`, configuration events require resource-free `ManagePlugins`, and
stream-ended events require `Control` for their target device.

`TransitionsChanged` carries only dirty transition identifiers. Transition
snapshots remain authoritative on the primary socket and are retained only in
daemon memory.

The event socket has its own `EVENT_PROTOCOL_VERSION` (currently 8),
`SubscribeHello`/`SubscribeAck` compatibility handshake, and the same
length-prefixed CBOR framing as the primary socket. A lagged subscriber
receives one `ResyncRequired` event and must fetch every authoritative baseline
again before trusting later incremental invalidations. The marker contains no
resource information and bypasses authorization and voluntary event filters.

## D-Bus effect representation

The optional `luminate-dbus` companion projects the canonical client model
through additive versioned interfaces. Legacy `Manager1`, `Target2`, and type
marker interfaces remain available. `Manager2`, `Target3`, and the typed `*2`
interfaces provide complete topology, capability, state, collection,
transition, setup, administration, and ordinary-frame semantics. The
[D-Bus consumer guide](../dbus.md) is the public contract; the
[parity matrix](../dbus-parity.md) records every operation and deliberate
transport exception.

Effect dictionaries remain shared by legacy and complete interfaces.
`EffectDescriptors` is an array of advertised hardware-effect descriptors.
Each descriptor is `(id, name, parameters)`, and each parameter is
`(kind, minimum, maximum, step, choices)`. Numeric bounds are zero when they do
not apply. A choice is `(id, display_name)`. Parameter kinds are `colour`,
`speed`, `direction`, `duration-ms`, `brightness`, and `choice`.

`SetEffect` accepts an `a{sv}` request dictionary. Keys use PascalCase:

- `Kind` is required and is one of `off`, `static`, `breathe`, `pulse`,
  `strobe`, `scanner`, `morph`, `spectrum`, `rainbow`, or `hardware`.
- `Colours` is an array of `(red, green, blue)` byte triples. Static and the
  single-colour portable effects require one entry; morph requires at least
  one.
- `PeriodMs` is required by every animated portable effect.
- A `hardware` request requires `HardwareId` and may carry `Colours`, `Speed`,
  `Direction`, `DurationMs`, `Brightness`, and `Choice` according to the
  advertised descriptor. Direction IDs are `forward`, `reverse`, `clockwise`,
  `counter-clockwise`, `inward`, `outward`, and `random`.

Appearance-slotted surfaces additionally expose `AppearanceSlotUpdatePolicy`
and ordered `AppearanceSlotDescriptors` properties. `SetAppearanceSlots`
accepts an array of `(slot_id, effect_dictionary)` pairs and submits it as one
logical mutation; it never expands the request into sequential `SetEffect`
calls. State reports an `appearance-slots` facet with the number of known
values and whether the observation is complete. Scene binding dictionaries use
`AppearanceSlots` with the same ordered `(slot_id, effect_dictionary)` shape.

The appearance-slot addition changes the client request and topology/state
payloads, so protocol ABI 27 replaces 26. The corresponding plugin CBOR
topology, observation, and update payloads change plugin ABI 11 from 10. No new
event variant or event payload was added: existing topology and state dirty
events still prompt authoritative refetches, so event protocol 7 is unchanged.

Device topology responses now serialize the ordered `physical_tags` attached
to a device as a whole, so protocol ABI 28 replaces 27. Surface physical tags
remain attached to their individual surfaces. The corresponding
`DeviceDescriptor` topology CBOR gains the device-level field, so plugin ABI 12
replaces 11; plugins must be rebuilt against the matching daemon API. Events
continue to identify affected devices by `DeviceId` rather than embedding
`Device` values, so event protocol 7 is unchanged.

The daemon's persisted state contains target identifiers, desired state, the
adopted physical baseline, collections, scenes, and related ownership metadata
rather than complete `Device` topology values. Device physical tags therefore
require no persistence-version bump or migration. The D-Bus object model
likewise projects complete topology and physical tags through additive
versioned interfaces without changing the retained legacy paths or members.

Element topology values now serialize their own ordered `physical_tags`, so
protocol ABI 29 replaces 28. The corresponding `ElementDescriptor` topology
CBOR field means plugin ABI 13 replaces 12; every native plugin must be rebuilt.
The tags remain scoped to their device, surface, or element and are not
inherited. Events still carry dirty identifiers rather than complete topology,
so event protocol 7 remains unchanged. Persisted daemon state does not contain
complete topology descriptors, so this change requires no persistence-version
bump or migration.

Event tickets are now issued through an authenticated
`Request::IssueEventTicket` immediately before a client opens an event
connection. They are no longer unconditional `SessionMetadata`, so ordinary
control sessions do not retain an unused 30-second ticket. This changes the
authentication response and request/response vocabulary, so protocol ABI 30
replaces 29. Event framing and payloads are unchanged, so event protocol 7 is
unchanged.

Unknown keys, effect kinds, direction IDs, missing required fields, and fields
that do not belong to the selected effect kind return
`org.luminate.Error.InvalidArgument`. Authorization is checked before the
request is converted and sent through `libluminate`; the daemon remains the
authority for target capability and descriptor-bound validation.

## D-Bus plugin management representation

`org.luminate.Luminate1.Manager1.GetManagement` returns the authoritative
management snapshot as `a{sv}`. `PatchManagement` accepts an expected revision
(`t`) and an array of mutation dictionaries (`aa{sv}`), and returns the
committed revision plus redacted change records. A change record has signature
`(ssasb)`: kind, plugin name, changed keys, and whether a plugin setting is
sensitive. The `ConfigurationChanged` signal carries the same revision and
record array after the daemon publishes a durable configuration event.

Snapshot dictionaries use PascalCase keys. Their top-level fields are
`Revision`, `DesiredDaemon`, `EffectiveDaemon`, `LockedDaemonSettings`, and
`Plugins`. Plugin dictionaries contain `Name`, `Version`, `Required`,
`DesiredEnabled`, `EffectiveEnabled`, `DesiredReconciliation`,
`EffectiveReconciliation`, `Runtime`, `ActivationLocked`, `Schema`,
`DesiredSettings`, `EffectiveSettings`, and `LockedSettings`. Optional values
are dictionaries containing `Present` and, when present, `Value`. Reported
setting values similarly contain a `State` of `unset`, `visible`, or
`redacted`; only `visible` includes `Value`.

Mutation dictionaries have a `Kind` and reject unknown fields:

- `set-daemon-preferences` carries `Preferences`.
- `set-plugin-enabled` carries `Plugin`, `HasEnabled`, and `Enabled` when
  present.
- `set-plugin-reconciliation` carries `Plugin`, `HasReconciliation`, and
  `Reconciliation` when present.
- `set-plugin-setting` carries `Plugin`, `Key`, and a native D-Bus `Value`.
- `clear-plugin-setting` carries `Plugin` and `Key`.

Setting values use D-Bus Boolean, signed 64-bit integer, double, string, array,
and string-keyed dictionary values. Arrays may contain variants. Tables use
variant values. Non-finite doubles and other D-Bus types are rejected.
Sensitive setting values are accepted only by `PatchManagement`; snapshots,
method replies, signals, diagnostics, and change records never contain them.
The bridge performs its normal group or Polkit authorization before reading,
converting, or forwarding management data, and the daemon independently
authorizes both reads and writes as `ManagePlugins`.

## D-Bus scene representation

`Manager1` exposes scene list/get/create/capture/replace/recapture/delete/apply
methods and the `ScenesChanged` signal. A scene record contains its ID,
revision, name, optional-description flag and value, owner kind/value, and
ordered bindings. Each binding contains an empty collection ID for frozen
targets or its dynamic collection ID, a canonical target ID, and an `a{sv}`
sparse state. `Appearance` uses the same effect dictionary accepted by
`Target2.SetEffect`; `Brightness` and `Emission` are present only when captured.
Application returns canonical applied and denied target lists.

## D-Bus error bodies

`Manager1.GetAccessPolicy` and `ReplaceAccessPolicy` use a strict versioned
`a{sv}` document. Its keys are `SchemaVersion`, `Revision`, `Roles`, and
`Bindings`; roles contain typed rules, operation names, resource constraints,
and quota records. Unknown or incorrectly typed fields are rejected before
daemon dispatch. Token create, list, rotate, and revoke use typed metadata
records. Create and rotate return the display-once secret as native `ay`.
Unix caller groups carried by bridge attestations are canonical decimal GID
strings.

### Error mapping

D-Bus errors normally carry one human-readable string. Two error names have
additional typed fields so clients do not need to parse diagnostics:

- `org.luminate.Error.RateLimited` carries `(sbt)`: the message, whether retry
  guidance is present, and the minimum retry delay in milliseconds. The delay
  is zero when the presence flag is false.
- `org.luminate.Error.PartialMutation` carries `(sas)`: the message and the
  canonical IDs of targets whose mutations were committed before the failure.

Canonical target IDs use `device:<id>`, followed where applicable by
`/surface:<id>/element:<id>` or `/group:<id>`. Error messages remain intended
for people and clients must not parse them for control flow.
