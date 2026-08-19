<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Writing a `luminated` plugin

This guide covers the plugin contract, the daemon's assumptions, and the path
from a new crate to working hardware. Use
`plugins/luminate-plugin-demo-system` as the reference implementation while
reading it.

Contents: [What a plugin is](#what-a-plugin-is) ·
[Starting point](#starting-point) · [The ABI symbols](#the-abi-symbols) ·
[`PluginDescriptor` fields](#plugindescriptor-fields) · [Logging](#logging) ·
[Discovery, activation, and configuration](#discovery-activation-and-configuration) ·
[Failure handling](#failure-handling) · [Topology](#topology) ·
[Applying updates](#applying-updates) · [Batched updates](#batched-updates) ·
[Unload and reload](#unload-and-reload) ·
[Local dev/test loop](#local-devtest-loop)

## What a plugin is

A plugin is a `cdylib` shared object loaded into a dedicated child host that the
`luminated` daemon supervises. The image is never unloaded within a still-running
host; there is no `dlclose` or ABI teardown callback, so the only way to
release a plugin's resources is to end its host process, whether that's
triggered by daemon shutdown, a timeout, native failure, or an explicit
unload/reload (see [Unload and reload](#unload-and-reload)). A plugin owns a
slice of the topology (one or more `Device`s) and is responsible for:

- discovery and probing: decide whether the plugin should load on this system;
- topology: describe its devices, surfaces, elements, groups, and capabilities;
  and
- device operations: apply colour, brightness, effect, and clear requests to
  hardware.

The plugin ABI (`crates/luminate-plugin-api`) is intentionally not
stable across daemon versions, similar to a Linux kernel module. Plugins are
expected to be rebuilt against each `luminated` release, not to negotiate
compatibility with old daemons. The daemon checks this by comparing the
exported `LUMINATE_PLUGIN_ABI_VERSION` symbol against its own
`luminate_plugin_api::PLUGIN_ABI_VERSION` value and refuses to load anything that
doesn't match exactly.

The current plugin ABI is 13. ABI 13 added element-level `physical_tags` to the
topology CBOR representation, so plugins built for ABI 12 must be rebuilt.

## Reconciliation vocabulary

The state model uses a few precise terms:

- A **facet** is one independently meaningful part of lighting state:
  appearance (colour or effect), brightness, local emission (dark or emitting),
  or physical power. Splitting state this way lets hardware report brightness
  exactly even when it cannot reconstruct an active effect.
- Facets split along a **configured/instantaneous** line. **Appearance** and
  **brightness** report what the target is configured to show: its colour,
  effect, and level, independent of whether that configuration is currently
  visible. **Emission** and **physical power** report the instantaneous fact
  of whether it's currently visible, at the target and at the ancestor power
  domain respectively, and are the sole authority for on/off. A dimmer set to
  50% while off still reports brightness 50, the same way an off light still
  reports its last configured colour, so a consumer can show "red, at 50%,
  but off" instead of losing that configuration the moment power drops. `Off`
  is therefore never a valid observed appearance value: it is an instruction
  to stop emitting, not a configuration, so a plugin acting on it updates
  emission, not the target's configured appearance.
- Prefer deriving emission from a genuine power/enable signal when a target
  has one. When it does not have one (a target that is only ever "set to a
  colour," with no separate enable register), deriving `Dark` from the
  configured colour being black is an acceptable approximation, since "is
  this on or off" is worth answering even at the cost of conflating "off" with
  "deliberately configured to black." Only skip advertising emission entirely
  for a target with no meaningful on/off answer at all, for example one that
  is always emitting and can only be recoloured.
- An **observation** is the daemon's last information about what hardware is
  actually rendering at a canonical device, surface, or element target. It has
  a value, source, timestamp, confidence, and stale/fresh marker. It is not a
  command and does not by itself say what hardware should do next.
- **Confidence** says how the value is known: `Assumed` after a successful write
  without verification, `BestEffort` from limited readback, or `Confirmed` from
  exact readback. **Freshness** is separate: an old confirmed value can be stale.
- A **desired overlay** is an explicit Luminate command that should win when
  policy restores state. Broad and narrow overlays compose in replay order.
- The **adopted physical baseline** is confirmed hardware state durably saved
  beneath desired overlays. Adoption uses this separate layer so it does not
  accidentally create an individual override.
- A **reconciliation policy** chooses direction when control is gained or
  regained: `Restore` writes effective desired state, `Adopt` reads and rebases
  exact hardware facets, and `Leave` avoids writes and may read only for display.
- A **power domain** is the broader target whose physical power a subtarget
  operation depends on. A zone can be dark locally while its device remains on.
- A per-device **operation sequencer** prevents reads and writes from observing
  transitional hardware state. A **generation** is a monotonic epoch used to
  reject results captured before a newer mutation or topology change.

For the full composition and failure semantics, see
[`docs/development/architecture/state-reconciliation.md`](architecture/state-reconciliation.md).

## Starting point

All bundled plugins use the safe Rust authoring facade in
`luminate_plugin_api::sdk`. Implement `LuminatePlugin` for construction,
probing, topology, and individual updates. Add `StartPlugin`, `RescanPlugin`,
`BatchPlugin`, or `ReadablePlugin` only when the corresponding callback should
exist, then use `luminate_export_plugin!` to generate the raw ABI callbacks and
descriptor.

Use the demo bulb for ordinary ordered updates, the demo ambient panel for
state readback, the demo keyboard for native batch results, and the demo system
for broad topology coverage. LIFX and WLED demonstrate dynamic discovery and
readback; Linux LEDs demonstrates scan-based topology; Alienware demonstrates
native batching around one hardware transaction.

Dynamic network providers can use `DynamicDeviceRegistry` to retain stable
identities across missed discovery cycles, enforce a device cap, publish
deterministic snapshots, and notify the daemon once when topology fingerprints
change. LIFX and WLED are the reference implementations; expiry-at-boundary is
an explicit policy because their established TTL semantics differ.

Such a provider should also implement `RescanPlugin` (`rescan: native`), which
the daemon calls before re-pulling topology on resume from suspend, on a device
change, or on an operator request. Because `topology` answers from the registry
rather than from hardware, a re-pull alone would replay the pre-suspend view
until the next ordinary discovery cycle. Pace the discovery loop with
`DiscoveryPacer` instead of `thread::sleep` and have `rescan` wake it; do not
clear the registry, or every known device vanishes and reappears for no reason.
A plugin whose `topology` enumerates hardware on every call needs none of this
and should use `rescan: none`. See
[`architecture/suspend-resume.md`](architecture/suspend-resume.md).

Start by copying the closest demo plugin and:

1. Rename the package in `Cargo.toml` (`name = "luminate-plugin-your-thing"`,
   keep `crate-type = ["cdylib"]`), and add `tracing.workspace = true`
   alongside `luminate-plugin-api.workspace = true` if you want logging (see
   "Logging" below).
2. Change `NAME`/`VERSION` in `src/lib.rs`.
3. Implement `LuminatePlugin`; consume typed configuration in its fallible
   `new()` constructor, and keep probing cheap and side-effect free.
4. Replace the topology and device-building functions with your
   real topology.
5. Implement `StartPlugin` for discovery or polling work after acceptance.
6. Implement `BatchPlugin` only when native coalescing is useful,
   `ReadablePlugin` when hardware supports live state queries,
   `FrameStreamingPlugin` when a target advertises `FrameUploadCapability`,
   and additionally `ShmFrameStreamingPlugin` when a target also advertises
   `FrameUploadCapability::shm` (see "Frame streaming" below).
7. Invoke `luminate_export_plugin!` with explicit `start`, `rescan`, `batch`,
   `read_state`, `frame_upload`, and `shm_frame` modes and choose a
   `recommended_reconciliation` policy. Pass a static `settings` slice when
   the plugin accepts configuration; omitting it declares an empty schema.

## The ABI symbols

The daemon looks up two exported data symbols:
`LUMINATE_PLUGIN_ABI_VERSION` and `LUMINATE_PLUGIN_DESCRIPTOR`. The
first is a `PluginAbiVersion` scalar; the second is a `PluginDescriptor`.
Both, and everything the descriptor points to, must stay valid for the
lifetime of the loaded plugin. In practice this means `static` storage, like
the demo plugin's `NAME`, `VERSION`, `BUSES`, `VENDORS`, and `HINTS`.

Plugins use `luminate_export_plugin!` to emit those symbols. The generated
adapter owns CBOR decoding, result buffers, request-context installation,
stable topology storage, logging/configuration bridges, and panic containment.

## `PluginDescriptor` fields

| Field                              | Meaning                                                                                                                                                                                                                                                                                             |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `name`                             | NUL-terminated, must be ASCII. Used in logs and for name-based config lookup (see "Discovery, activation, and configuration" below).                                                                                                                                                                |
| `version`                          | NUL-terminated plugin version string, informational.                                                                                                                                                                                                                                                |
| `priority`                         | `i32`. Used to resolve logical-ID and exclusive hardware-claim conflicts: highest priority wins, first-loaded is the tie-breaker. Pick a priority that reflects how specific and authoritative this plugin is for the hardware it targets.                                                          |
| `recommended_reconciliation`       | Optional `Restore`, `Adopt`, or `Leave` recommendation. Use `Restore` for daemon-owned volatile peripherals, `Adopt` for shared independently controlled hardware, and `Leave` when neither direction is safe. User configuration always wins.                                                      |
| `buses` / `bus_count`              | Bus classification (`PluginBus::Usb`, `Hid`, `I2c`, `Platform`, `Network`, ...). In `host-attached` activation mode, only a non-empty list containing known non-network buses qualifies for automatic activation; any `Network` entry makes the plugin network-capable.                              |
| `vendors` / `vendor_count`         | Informational vendor/product ID pairs.                                                                                                                                                                                                                                                              |
| `probe_hints` / `probe_hint_count` | Optional hints (`ProbeHintKind::UsbVidPid`, `HidVidPid`, `DmiMatch`) intended to help future discovery tooling narrow which plugins to try; not currently consumed by the loader beyond being read into `LoadedPluginMetadata`.                                                                     |
| `settings` / `setting_count`       | Static, inspectable configuration schema. Each setting declares a dotted key, display text, value kind, optional default and constraints, whether it is required or sensitive, and how a change is applied. Pass an empty slice when the plugin accepts no configuration.                           |
| `init`                             | Required. Called once after ABI validation and before `probe`, with the daemon's log callback, effective log level, topology-change callback, and host-owned configuration CBOR. The macro-generated init installs these bridges and copies the configuration. Do not start discovery threads here. |
| `probe`                            | Optional in the ABI, but every real plugin should provide one. Returns `ProbeOutcome::Unsupported`, `Dormant`, or `Ready`; only `Unsupported` rejects the load.                                                                                                                                     |
| `start`                            | Optional. Called once after a non-unsupported probe and before the initial topology fetch. Start discovery and polling threads here so a rejected plugin creates no background work.                                                                                                                |
| `topology_cbor`                    | Optional in the ABI, but real plugins should provide one. Called at load and again after a dynamic plugin calls `notification::topology_changed()`. Each call returns the plugin's complete authoritative topology as length-delimited CBOR.                                                        |
| `apply_update_cbor`                | Optional in the ABI, but mutable devices need one. Called by the daemon every time a mutation targets one of this plugin's devices.                                                                                                                                                                 |
| `apply_batch_cbor`                 | Optional in the ABI. The safe export adapter supplies ordered per-update behaviour by default or delegates to `BatchPlugin` for native coalescing; see "Batched updates" below.                                                                                                                     |
| `read_state_cbor`                  | Optional bounded bulk snapshot callback. `None` is correct when live hardware state cannot be queried. See "State snapshots" below.                                                                                                                                                                 |
| `frame_upload_cbor`                | Optional streamed-frame callback. `None` is correct unless a target advertises `FrameUploadCapability`; see "Frame streaming" below.                                                                                                                                                                |
| `shm_stream_begin` / `shm_frame_apply` / `shm_stream_end` | Optional zero-copy shared-memory streaming callbacks, validated as an all-or-nothing group. `None` for all three is correct unless a target advertises `FrameUploadCapability::shm`; see "Frame streaming" below. |

If `probe`, `topology_cbor`, or `apply_update_cbor` are omitted from a
hand-written descriptor, the plugin can still load structurally, but it will
either opt out at load time or contribute no mutable devices. Leaving
`apply_batch_cbor` as `None` costs nothing beyond the batching optimization
itself; the daemon's fallback path is the same per-update behaviour. Leaving
`frame_upload_cbor` as `None` while a target advertises `FrameUploadCapability`
is a contract violation the host rejects at topology-validation time, exactly
like an unread readable capability without `read_state_cbor`. The same
is true of the three `shm_*` callbacks against
`FrameUploadCapability::shm`, checked together as one group rather than
individually.

## Frame streaming

A target that advertises `FrameUploadCapability` (`scope`, `update_mode`,
`max_rate_hz`, `atomic`, `buffering`) accepts streamed frames through the
client protocol's `BeginFrameStream`/`UploadFrame`/`EndFrameStream`
requests. The daemon owns the whole stream lifecycle: it validates that no
conflicting stream or non-concurrent hardware effect already owns the
target, tracks generation/sequence to reject stale or duplicate frames, and
enforces `max_rate_hz` by silently dropping frames that arrive too soon
(the client sees `FrameAck { dropped: true }`; the plugin is never called
for a dropped frame). A plugin only needs to apply the frames it actually
receives. Implement `FrameStreamingPlugin::upload_frame(&self, context,
target, envelope) -> Result<(), PluginError>` and set `frame_upload:
native` in `luminate_export_plugin!`. `target` is bundled with the frame
(mirroring how `PluginUpdate` bundles its target for `apply_update_cbor`),
since a plugin may own more than one device. `envelope.generation` changing
between calls is the plugin's only signal that a stream restarted; reset any
internal buffering state accordingly rather than assuming ordered delivery
across generations.

Frames are never persisted or reflected in `GetState`/read-state snapshots.
They are a hardware-only, best-effort path kept out of the daemon's normal
state-file write path, since persisting a full state snapshot per frame would
be unworkable at streaming rates. `FramePayload` is
`Full(Vec<Colour>)` (required when `update_mode` is `FullFrameOnly`) or
`Partial(Vec<(u32, Colour)>)` (index/colour pairs, valid when `update_mode`
is `Partial` or `Both`); pixel order and index meaning are plugin-defined.
See `luminate-plugin-wled` for a reference implementation: it advertises
`FrameUploadCapability` at device scope and maps a full frame onto its
existing `/json/state` HTTP transport via the `seg.i` field, one segment at
a time, sliced from the flat device-wide pixel array.

### The zero-copy shared-memory fast path

A target may additionally advertise `FrameUploadCapability::shm` (a
`ShmFrameCapability`: accepted pixel formats in preference order, a
`Linear`/`Matrix` shape, and an optional fast-path-specific rate ceiling).
Doing so is additive: the host still falls back to the ordinary
`FrameStreamingPlugin` path above for any frame or stream the fast path
can't handle (a `Partial` envelope, a negotiation failure, `prefer_shm`
disabled). Implement `FrameStreamingPlugin` regardless, then add
`ShmFrameStreamingPlugin` on top of it once you actually need the extra
throughput:

```rust
impl ShmFrameStreamingPlugin for MyPlugin {
    type Stream = MyStream; // per-stream state; must be Send

    fn shm_stream_begin(
        &self, context: &PluginRequestContext, target: &PluginTarget,
        format: ShmPixelFormat, pixel_count: u32, generation: u32,
    ) -> Result<Self::Stream, PluginError> { /* ... */ }

    fn shm_frame(
        &self, context: &PluginRequestContext, stream: &mut Self::Stream,
        header: &ShmFrameHeader, pixels: &[u8],
    ) -> Result<(), PluginError> { /* ... */ }

    fn shm_stream_end(&self, stream: Self::Stream, generation: u32);
}
```

and set `shm_frame: native` in `luminate_export_plugin!` (mandatory for
every plugin, mirroring `frame_upload`'s `none`/`native` slot; most
plugins pass `none`). `pixels` is `header.pixel_count *
format.bytes_per_pixel()` tightly packed bytes, borrowed for the duration
of the call only; do not retain the pointer past it.

Concurrency is different here. Every other callback in this table runs
serialized on the plugin-host's single command thread. `shm_frame` instead
runs on a thread dedicated to its own stream, with one thread per active
stream. It can therefore run concurrently with an in-flight
`apply`/`read_state`/etc. call and with another active stream's
`shm_frame` call. A given `Self::Stream` is never touched by more than one
thread at a time, so state kept entirely inside it needs no extra
synchronization; state shared across streams via `&self` does. See
`PluginShmFrameApplyFn`'s doc comment for the exact contract and
`docs/development/architecture/shm-frame-streaming-phase1.md` for the full
design, including why this differs from the CBOR path's threading model.
`luminate-plugin-demo-display` is the reference implementation: a 16×16
matrix that implements both paths side by side.

## Logging

Configure the logging bridge before calling `tracing::` macros; without it,
plugin logs have no subscriber. A plugin `cdylib` has its own `tracing` global
state and cannot use the daemon's subscriber directly. Before `probe`, the
daemon calls `PluginDescriptor::init` with a `PluginLogFn`, its effective log
level as a `u8`, a `PluginNotifyFn`, and the plugin's configuration CBOR.
`PluginLogFn` forwards messages into the daemon and is the route by which
plugin logs reach its subscriber.

`luminate_export_plugin!` installs the plugin-local `tracing` subscriber before
constructing the plugin. It tags events with the plugin name, filters them at
the daemon-provided level, and forwards them through the host callback. Plugin
code can use `tracing::info!`, `warn!`, and the other macros normally.

`max_level` is the daemon's own currently effective log verbosity, resolved
once and handed down at load time. Don't re-parse `RUST_LOG` yourself inside
the plugin; that risks drifting from what the daemon logs at.

You may call `PluginLogFn` directly with the following signature, though the
logging bridge is usually simpler:

```rust
unsafe extern "C" fn(
    plugin_name: *const c_char,
    level: u8,
    message: *const c_char,
)
```

## Discovery, activation, and configuration

The daemon discovers plugin candidates only in administrator-controlled
locations, both driven by `DaemonConfig`
(`crates/luminated/src/device_config.rs`):

- **Explicit**: a `[[plugins]]` entry with `path` (used as-is) or canonical
  plugin `name`. Name lookup checks `lib{name}.so` then `{name}.so` in each
  of `plugin_dirs`, in order, and also checks Cargo's underscore-normalized
  artefact name when the canonical name contains hyphens. `required = true`
  makes a missing/failing explicit plugin a fatal daemon startup error;
  `required = false` logs and skips it.
- **Directory discovery**: every `*.so` file directly inside each
  `plugin_dirs` entry is considered, in sorted order. An unreadable optional
  plugin directory is logged and skipped rather than aborting startup.

Default `plugin_dirs` come from
`luminate_platform::default_path::{default_plugin_dir_local,
default_plugin_dir_system}`
(normally `/usr/local/lib/luminate/plugins` and
`/usr/lib/luminate/plugins`). See `docs/development/plugins/demo-plugin.md`
for the local/dev override flow (`LUMINATED_CONFIG`, and pointing a config's
`[[plugins]]` entry straight at your `target/debug/lib*.so`).

The same physical file is only ever catalogued once: explicit and discovered
candidates are deduplicated by canonical path, and an explicit entry wins
the dedup (so its `required` flag applies even if the file also lives in an
enumerated directory).

Before activation, a disposable host opens each candidate and reads its ABI
version, identity, buses, and settings schema. It does not call `init`, `probe`,
`start`, `topology`, or any other lifecycle callback. Keep descriptor
initialization side-effect free and every descriptor pointer backed by static
storage. Inspection limits malformed metadata and avoids touching hardware
while building the catalogue, but loading native code is not a security
sandbox; only administrators may choose plugin paths.

Global policy chooses which catalogued plugins are activated:

- `explicit` selects explicit entries and plugins enabled through managed
  configuration;
- `host-attached` additionally selects plugins whose non-empty bus list
  contains only known non-network buses; and
- `all` additionally selects every discovered plugin.

Required explicit plugins are always enabled. Each `[[plugins]]` entry may set
`activation` to `managed`, `enabled`, or `disabled`; a forced value overrides
managed desired state.

An explicit entry keeps all administrator-owned per-plugin policy together.
Its optional managed reconciliation override replaces `reconciliation`; the
effective plugin policy in turn overrides the daemon-wide
`reconciliation_policy`. Device-specific policy remains highest precedence.

```toml
[[plugins]]
name = "luminate-plugin-your-thing"
required = true
activation = "enabled"
reconciliation = "Restore"
locked_settings = ["discovery"]

[plugins.config]
discovery = true
endpoints = ["192.0.2.10"]
```

Declare every accepted value in a static `PluginSettingDescriptor` slice and
pass it as `settings` to `luminate_export_plugin!`. Keys are dotted identifiers
whose segments contain ASCII letters, digits, `_`, or `-`. Kinds are boolean,
integer, number, string, enumeration, and array. Defaults and kind-specific
constraints are TOML expressions; numeric bounds are inclusive. Enumeration
constraints use `{ choices = [...] }`, while an array may use
`{ element_kind = "string" }` (or `boolean`, `integer`, or `number`). The
current and only supported apply mode is
`RestartRequired`: changing an effective setting restarts a loaded plugin host.
`PluginSettingDescriptor::string` is a convenient constructor for the common
restart-required string case. See `luminate-plugin-lifx` for a complete example.

A setting marked `required` must have a value after resolution. Mark secrets
such as tokens or passwords `sensitive`; management reads, events, diagnostics,
and catalogue output redact both their values and sensitive constraints.
Management clients supply sensitive values through write-only mutation
requests, and the CLI accepts them only on standard input. Plugin code receives
the resolved value normally and must still avoid logging or otherwise exposing
it.

Effective configuration resolves the schema default, then the explicit global
`config` table, then a managed override. An administrator may lock a dotted key
or a parent key globally; a locked managed override remains stored but dormant.
Only schema-declared values with the declared kind, bounds, and constraints are
accepted. A setting change is committed to the managed file before its host is
restarted.

The daemon encodes the resolved table as CBOR and sends those opaque bytes to
the supervised host over the private length-prefixed CBOR channel used for
commands. The host passes the bytes unchanged to the native plugin ABI. They
are not placed in process arguments or environment variables, and the same
immutable bytes are resent after a host restart. A plugin with an empty schema
receives an empty CBOR map. Deserialize a typed schema during construction with
`luminate_plugin_api::configuration::deserialize::<T>()`. Use Serde defaults
for optional settings and `deny_unknown_fields` as defence in depth. Return
`PluginError::InvalidArgument` when parsing or semantic validation fails so a
required plugin fails startup rather than silently using unintended values.

## Failure handling

A plugin can fail to load in several ways:

- a missing ABI-version symbol or descriptor symbol
- a null descriptor
- an ABI-version mismatch
- a `probe` that returns `Unsupported` or an unknown ABI value
- a non-ASCII `name`
- a panic during load (panics are not caught)

None of these is fatal to the daemon unless the plugin was configured with
`required = true`. Otherwise the daemon logs the failure at `warn` and
continues with whatever plugins did load. Write `probe()` to check platform and
configuration prerequisites cheaply. Return `Dormant` when the provider is
supported but hotpluggable hardware is absent, and `Unsupported` only when the
provider cannot operate on this system.

Once `init` has been called, the child pins the shared library until that child
exits, including when a later probe or validation step rejects it. Background
threads therefore cannot execute an unloaded image. Plugin callbacks never run
in the supervising daemon: a panic or native fault kills only the host, and an
IPC deadline kills a host whose callback blocks indefinitely. The daemon
reports the failed operation and may restart the host on later contact after
revalidating its identity. Plugin authors should still return ordinary errors
rather than relying on this containment boundary.

## Topology

`LuminatePlugin::topology` returns the complete typed
`Vec<DeviceDescriptor>`. The generated adapter serializes it into stable CBOR
storage for the host. When the complete snapshot changes, call
`luminate_plugin_api::notification::topology_changed()` from the discovery
thread. The daemon debounces the dirty bit, re-pulls the full graph, validates
it, and updates ownership atomically; do not send per-device deltas.

The topology hierarchy is:

- `DeviceDescriptor`: `id`, `name`, optional `vendor`/`model`, hardware
  `claims`, a list of `SurfaceDescriptor`, a list of `GroupDescriptor`, its own
  `CapabilitySet`,
  an optional `category`, open `physical_tags`, a required `host_attached`
  (all below), and `notes`/`warnings` (see below).

`DeviceDescriptor::id` is the logical identity exposed to clients and used for
persisted state. A `HardwareClaim` separately names the physical identity and
control domain reached by that device. Publish an `Exclusive` claim whenever a
second provider must not drive the same output. Two `Shared` claims may coexist;
an overlap involving an exclusive claim is resolved by plugin priority. Claims
are whole-device routing metadata, so a device that loses one contested claim
is omitted rather than partially merged. Keep claim strings stable,
non-empty, and normalized without surrounding whitespace.

Derive `DeviceDescriptor::id` from stable physical identity, such as a serial
number, a `UNIQ`/`PRODUCT` pair, or a vendor/product ID. Never derive it from an
enumeration-order path. USB and HID devices routinely re-enumerate across a
suspend/resume or a hub reset, so an ID built from `/dev/hidraw2` becomes a
different device the moment the kernel hands out `hidraw5`. The daemon then
correctly concludes that one device withdrew and an unrelated one arrived, and
the user's persisted state stays attached to the device that no longer exists.
Linux LEDs (`stable_device_identity`) and Alienware (fixed vendor/product IDs)
are the in-tree examples. See
[`architecture/suspend-resume.md`](architecture/suspend-resume.md).

- `SurfaceDescriptor`: a named region (`SurfaceKind::Opaque` for zoned
  panels, `Linear { length }` for a strip/ring, `Sparse2d { width, height }`
  for irregularly placed elements, `Matrix { rows, cols }` for a keyboard-
  style grid, `Zone` for a single-region surface like a button ring or
  badge), containing `ElementDescriptor`s, plus open `physical_tags` and
  `notes`/`warnings`.
- `ElementDescriptor`: the finest-grained addressable unit (a key, a zone,
  a logo, ...), with `ElementKind`, optional `ElementGeometry`, open
  `physical_tags`, and `notes`/`warnings`.
- `GroupDescriptor`: a named, possibly cross-surface collection
  (`GroupMemberDescriptor::Surface`/`Element`/`Group`) for convenience
  targeting, e.g. "all gamer keys" or "all case lighting," plus
  `notes`/`warnings`.

`notes: Vec<String>`/`warnings: Vec<String>` (on all four descriptor types)
are display text for a future GUI/CLI, e.g. "requires the vendor kernel
module" or "avoid a static colour for extended periods, this panel is prone
to burn-in." They are never parsed or matched on for behaviour; an empty
`Vec` (the common case) means none. The `luminate` CLI prints them as
`note: ...`/`warning: ...` lines under each entry.

`DeviceDescriptor.category: Option<DeviceCategory>` is a GUI presentation
hint (icon/silhouette selection), not a functional capability. Set it if you
know a reasonable classification; otherwise leave it `None` (a GUI falls back
to a generic icon). `DeviceCategory` is an open string newtype, not a closed
enum, so new peripheral kinds don't need a `luminate-core` change.
`luminate_core::device::device_category` has well-known constants
(`KEYBOARD`, `MOUSE`, `FAN`, `RAM`, `CASE_LIGHT`, `MONITOR`, `BUTTON`,
`GPU`, `MOTHERBOARD`, `COOLER`, `LED_STRIP`, `SPEAKER`, `MICROPHONE`,
`HEADSET`, `CONTROLLER`, `POWER_SUPPLY`) a GUI can match on; anything else
(e.g. `DeviceCategory::new("case-light-strip")` for a more specific case
light) is a valid, supported choice that renders with a generic icon
until/unless a GUI special-cases it.

`DeviceDescriptor.host_attached: bool` is a required, plugin-declared fact:
whether the device is physically attached to (part of) the machine running
`luminated`, independent of any user-facing grouping. It is the sole signal
the daemon uses to compute an ACL-facing resource's `host_attached`
(`luminated::authorization::Resource`), so state it explicitly and
accurately for every device. The Alienware plugin and the demo system
plugin set `true`; networked fixtures like the LIFX plugin set `false`. There
is no default and no user override, unlike the free-form, user-created
collections that clients can group devices into at runtime.

`DeviceDescriptor::physical_tags`, `SurfaceDescriptor::physical_tags`, and
`ElementDescriptor::physical_tags` are open sets of semantic presentation
hints. Device tags describe the physical fixture or peripheral as a whole.
Surface tags describe only one addressable region, installation, or mapping.
Element tags describe one individually addressable physical object within a
surface. Publish each fact at the narrowest truthful scope: a LIFX Beam is a
`shape:modular-light-bar` device, while a WLED controller's configured
`shape:cylinder` installation is a surface. Do not copy tags to descendants
merely for consumer convenience. The
[physical-tag reference](physical-tags.md) defines the standard vocabulary and
the rules for plugin-defined extensions.

Tags are not inherited. A consumer must not treat a device or surface tag as
though it appeared on every descendant. The same tag may independently appear
at more than one scope when the physical fact is genuinely true at each one.

Tags supplement `DeviceCategory` and logical `SurfaceKind`; they do not replace
either, advertise capabilities, authorize operations, or select protocol
behaviour. In particular, a linear addressable surface does not establish that
the device is a flexible strip. Use identity data, a hardware profile, or
explicit installation configuration as the authority. Do not derive physical
form from a user-controlled label or a loose model-name match. An empty list is
the correct representation when the form is unknown.

The daemon preserves tag order but rejects an empty tag, surrounding
whitespace, or a duplicate within either a device's or a surface's list. A
device and one of its surfaces may independently carry the same tag when the
fact is genuinely true at both scopes. Consumers must preserve unknown tags
and ignore them safely. Prefer reusable, vendor-neutral tags when their meaning
fits; namespace genuinely provider-specific forms (for example,
`govee:shape/hexagonal-tile`). Multiple tags are useful when they express
independent facts, such as `shape:cylinder` and
`layout:wrapped-horizontal` on one surface.

`ElementGeometry` (`Rect`/`Point`/`Linear`/`MatrixCell { row, col }`) is
optional per-element and only partially populated by the bundled plugins
today: most elements still leave it empty, but the demo plugin already uses
`Linear` and `MatrixCell` for some of its devices. `Rect`/`Point`/`Linear`
coordinates are normalized `[0, 1]` relative to the surface's own bounding
box, not pixels or physical units; `MatrixCell` is for `SurfaceKind::Matrix`
surfaces where discrete row/column indices are the natural fit. If you don't
set `geometry`, the surface's `elements` array order is taken as the intended
visual/physical order (reading order for a keyboard, wiring order for a
strip).

The daemon normalizes and validates all of this on load
(`crates/luminated/src/normalize.rs`): every ID must be unique at its scope,
every name must be unique within its scope, every group member reference
must resolve, and everything must trace back to a real device/surface/
element. A plugin that violates this fails to load the same way a bad ABI
version does.

`CapabilitySet` (`crates/luminate-core/src/capability.rs`) describes what each
level supports. Its fields are `colour`
(`Some(ColourCapability::rgb8())`,
`Some(ColourCapability::monochrome(bits))`, a custom channel list, or
`None` for hardware with no drivable colour output at all, such as a device
whose only lighting behaviour is a fixed built-in effect advertised through
`hardware_effects` below), `brightness` (`BrightnessCapability::None` or
`Independent { bits, maximum, scope }`), `frame_upload`, `hardware_effects`,
`persistence`, `state_readback`, `emission`, `off_is_wear_safe`,
`physical_power`, and `power_domain`. Set capabilities at whatever scope is
real: the demo plugin's power button and PSU badge use
`fixed_colour_capabilities()` (`ColourCapability::monochrome(1)`,
`BrightnessCapability::None`) because they're single-colour LEDs with no
brightness control, while the keyboard uses full RGB with per-surface
independent brightness. Look at
`plugins/luminate-plugin-demo-system/src/lib.rs`'s capability helper
functions for the range of shapes already exercised.

`hardware_effects: Option<HardwareEffectsCapability>` advertises the
firmware-driven animated effects a target can run on its own. Each supported
effect is one `HardwareEffectDescriptor` (`id`, display `name`, and the
`EffectParameter` bounds the daemon validates against); `None` means the
target has no built-in effects. This is the capability a `SetEffect` mutation
is checked against; see "Applying updates" below. An `id` can either match a
well-known typed `Effect` variant (e.g. `"breathe"`) or name a vendor-specific
effect with no portable equivalent (e.g. `"aw-sweeper"`, a Govee scene set),
which clients invoke through `Effect::Hardware { id, arguments }`; see below. `frame_upload:
Option<FrameUploadCapability>` advertises that a target can accept streamed
full or partial pixel frames (`FrameUpdateMode`, `BufferingMode`, frame rate,
atomicity). Leave it `None` unless the plugin also implements
`FrameStreamingPlugin` and sets `frame_upload: native`; see "Frame streaming"
above.

Capability metadata is enforced by the daemon before plugin FFI; it is not
advisory. Keep descriptors aligned with `apply_update_cbor`:

- Static `SetEffect` requires a colour model advertised by the target. Additive
  colours require the exact channel set; duplicate, missing, or extra channels
  are rejected, and every value must fit its channel's advertised bit width.
- `Independent { bits, maximum, scope }` must describe the real brightness
  input. `maximum` is inclusive, begins at zero, and must fit in `bits`; the
  daemon rejects larger values rather than relying on a plugin to clamp them.
- Capability scope must match the target level: element, surface, or device.
  Group mutations use device scope because groups select targets within one
  device.
- Animated `Effect` variants (`breathe`, `pulse`, `strobe`, `scanner`, `morph`,
  `spectrum`, `rainbow`) are accepted only if the target advertises them in
  its `hardware_effects` capability: each supported effect is one
  `HardwareEffectDescriptor` whose `id` equals the effect's canonical id (the
  strings just listed). A `SetEffect` naming an id no descriptor covers is
  rejected as `Unsupported`. Every typed animated descriptor must include an
  `EffectParameter::Duration` bound, and effects carrying colours (`breathe`,
  `pulse`, `strobe`, and `scanner` carry one; `morph` carries several) must
  also include a `Colour` bound; the daemon validates the effect's period
  against the duration range/step and its colour count against the colour
  bound. `Static` is validated against the target's colour capability, and
  `Off` is available when the target has colour output or explicitly
  advertises `off`.
- Vendor-specific effects that have no portable typed variant are advertised
  under any other `id` and invoked with `Effect::Hardware { id, arguments }`,
  where `arguments: EffectArguments` is a generic bag (`colours`, `speed`,
  `direction`, `duration_ms`, `brightness`, `choice`). The daemon validates the
  supplied arguments against the matched descriptor's declared `EffectParameter`
  list. Each declared parameter must be present and in range, and no argument
  is accepted for a parameter the descriptor didn't declare. This is where
  `EffectParameter::Speed`, `Direction`, and `Brightness` (unusable by the typed
  variants) apply, plus `EffectParameter::Choice { options }`, a pick-one-of-
  named parameter for grouping a large scene set (e.g. hundreds of bulb presets)
  into one effect whose invocation sets `arguments.choice` to the chosen option
  id. A plugin may equally advertise many zero-parameter descriptors instead;
  choose whichever fits the hardware. The demo system plugin's `demo-scene-show`
  and the Alienware keyboard's `aw-sweeper` are worked examples.
- `Clear` remains valid for every resolved target. Persistence metadata governs
  `SaveCurrent` separately as described below.

The plugin must still validate inputs as defense in depth, particularly any
hardware-specific constraint that the shared capability model cannot express.
An unsupported operation is returned to clients as `Unsupported`; a malformed
or out-of-range value is `InvalidArgument` and never reaches the plugin.

### Persistence

`PersistenceCapability::CurrentState`/`Profiles` carry a
`requirement: PersistenceRequirement`: `Optional` (the common case: the
device works fully without ever persisting, and an explicit save is opt-in
convenience) or `Required` (the device write-throughs every mutation to
non-volatile storage as a side effect of ordinary operation, so there's no
"unsaved" state to save). Get this right if you know it: the daemon skips
redundant startup-reconciliation writes and short-circuits `SaveCurrent` to a
trivial success for `Required` targets, since real flash/EEPROM writes
aren't free or infinite. If your plugin doesn't yet implement firmware
persistence (i.e. it rejects `SaveCurrent`), use `Optional`: `Required` is a
claim about how the hardware actually behaves, not a placeholder for
persistence you plan to add later.

`off_is_wear_safe` is independent of that requirement. Set it only when the
device's ordinary off operation does not consume wear-limited persistent
storage, even if other appearance updates are written through. The all-off
planner uses this flag to include such targets while continuing to skip
required-persistence targets whose off operation is not known to be safe. A
target such as a keyboard that represents off by writing black colour values
should leave this flag false.

### `state_readback` and reconciliation

`StateReadbackCapability::None` means live hardware state cannot be queried.
`Readable` lists each readable facet (`Appearance`, `Brightness`, `Emission`,
or `PhysicalPower`) with `Exact` or `BestEffort` fidelity, plus whether reading
disturbs output and whether the plugin can notify external changes. Advertise
only facets the callback actually returns at that target scope.

Readback is a hardware fact, not authority. Startup and reappearance use the
effective `Restore`, `Adopt`, or `Leave` policy selected from user overrides,
the plugin recommendation, and finally the daemon's safe `Leave` fallback.
Exact reads may be durably promoted into the adopted physical baseline;
best-effort values remain visible observations but are never silently adopted.

`emission` describes a meaningful local emitting/dark facet.
`physical_power` exists only where the target independently owns a power
domain. A zone that can go dark but cannot power itself off advertises emission
without physical power. `power_domain` points to the broader device or surface
that a visible subtarget update powers on.

## Applying updates

`LuminatePlugin::apply` receives one typed `PluginUpdate` and request context;
the generated adapter owns CBOR decoding and the ABI result slot. `target` tells you which
device/surface/element/group the update is for; `operation` is one of
`SetEffect { effect: Effect }`, `SetBrightness { value: u32 }`, `Clear`, or
`SaveCurrent`, where `Effect`
(`luminate_core::effect::Effect`) and `Colour`
(`luminate_core::colour::Colour`) are the same types the daemon itself uses
internally. See the demo plugins' typed `apply` methods for the full match
shape; the demo system only logs, but this is exactly where real hardware
I/O (HID reports, feature reports, etc.) belongs.

`SaveCurrent` is already part of the current protocol. The daemon short-
circuits it to success for targets whose persistence requirement is
`Required`, because those devices already write through every mutation to
non-volatile storage. For `Optional` targets, the daemon dispatches it to the
plugin; `PluginApplyResult::applied()` should mean the hardware's current
state is now durably saved to firmware, not just accepted into the daemon's
cache.

Choose the result code by cause: `unsupported` for a permanently unsupported
operation, `invalid_argument` for malformed or invalid input, `unavailable`
when the addressed hardware is temporarily absent or unreachable,
`rate_limited` when work was temporarily refused because of a rate limit, or
`rate_limited_after` when the provider can state a minimum retry delay,
`io` for another transport failure, and `internal` for a plugin defect or
invariant failure. Each accepts an optional UTF-8 diagnostic, truncated safely
to 256 bytes. The daemon uses only the structured code for behaviour and maps
it to the corresponding client error category; the diagnostic is human
context. Long error chains belong in plugin logs.

`PluginError::RateLimited` carries a `Duration`; its `into_apply_result()`
conversion preserves that guidance across the current plugin ABI so maintained
clients can schedule a retry without parsing the diagnostic.

`PluginError` is the shared typed vocabulary for preserving those causes inside
a plugin. It distinguishes invalid targets and arguments, unsupported work,
temporary unavailability, rate limits, transport I/O, and internal failures;
`into_apply_result()` preserves those categories at the ABI boundary. Prefer
carrying `PluginError` to the callback boundary instead of flattening different
causes into `String` or reporting every failure as I/O.

Treat every callback as fallible-but-uncatchable: a Rust panic across the FFI
boundary is undefined behavior, not a clean error. Report failures through the
result slot rather than panicking.

Every mutation and read callback also runs with a daemon-owned monotonic work
budget. Typed methods receive a `PluginRequestContext`; construct its deadline,
then use `remaining()` or `cap()` before each retry or blocking transport
operation. An expired deadline should produce a timeout diagnostic and stop new
work. The host still enforces a five-second hard limit and terminates a callback
that ignores its slightly shorter budget; the plugin-side deadline exists to
finish cleanly before that containment boundary is needed. Background
discovery threads have no request deadline and receive `None`.

## Batched updates

`apply_batch_cbor` is an optional alternative entry point the daemon calls
instead of `apply_update_cbor`, once per batch, whenever it has several
updates to apply at once. Today that is daemon-startup replay of persisted
state, one call per plugin instead of one call per persisted target.
Implementing it is only an optimization: it lets you coalesce hardware work
(for example, rebuild an on-device table once for the whole batch instead of
once per update). It is never required for correctness, because leaving it
`None` falls back to the same per-update behaviour.

The batch callback has this signature:

```rust
pub type PluginApplyBatchFn = unsafe extern "C" fn(
    context: PluginRequestContext,
    batch_cbor: *const u8,
    batch_len: usize,
    results: *mut PluginApplyResult,
    results_len: usize,
) -> u8;
```

It follows a caller-allocated-buffer convention rather than a new alloc/free
FFI protocol. The CBOR byte slice decodes to one
`PluginUpdateBatch { updates: Vec<PluginUpdate> }`, and `results` points to a
buffer of exactly `results_len` `PluginApplyResult`s
(`results_len == updates.len()`). Write one structured entry per update, in the
same order as `updates`. The function's own `u8` return value separately
signals whether the call could be processed at all. Return `0` on a CBOR decode
failure; the daemon will not trust `results` and will treat every entry in the
batch as an internal failure. The daemon initializes every slot defensively and
validates status bytes, flags, diagnostic lengths, and UTF-8 before use.

The daemon makes no atomicity guarantee across a batch: a plugin may accept
some entries and reject others freely. As with `apply_update_cbor`, never
panic across the FFI boundary; use the precise structured code for every
entry. The daemon caps batches at 64 entries and chunks larger independent
workloads; state reads are likewise chunked to at most 256 targets per
callback. A callback's deadline covers the complete chunk, not each entry.

See Alienware's `BatchPlugin` implementation for a real example: it partitions
batch entries by internal path, resolves each independently so one bad entry
doesn't fail its siblings, and issues one hardware transaction for the whole
group instead of one per entry. The demo plugins take the simpler route. For
example, `demo-bulb` implements `apply_batch_cbor` as a loop that runs each
entry through the same per-update path, which is all a plugin with no
per-target state worth coalescing needs. Plugins without useful coalescing
select `batch: default`.

## State snapshots

`ReadablePlugin::read_state` receives one `PluginReadRequest` containing canonical device,
surface, or element targets and the facets requested at each target. Groups and
collections are selection concepts and must never appear as observation targets.
Return one `PluginStateSnapshot` with values and optional per-target errors;
partial success is expected when one device is offline.

Return a typed `PluginStateSnapshot`; the generated adapter serializes it into
the bounded host buffer without repeating hardware I/O. The daemon checks target ownership,
active topology, advertised facet/fidelity, value bounds, duplicates, and
canonical scope before publishing anything.

Plugins report values, not confidence. The daemon assigns `Confirmed` to exact
readback and `BestEffort` to lower-fidelity readback, retains timestamps and
freshness, and keeps observations separate from desired commands and the
durable adopted baseline. Per-target read errors belong in the snapshot;
`PluginApplyResult` remains specific to mutation outcomes.

## Unload and reload

An operator (or the control protocol's `UnloadPlugin`/`ReloadPlugin`
requests, or `SIGHUP` for reloading everything) can end a plugin's host
process while the daemon keeps running. There is no ABI teardown callback
symmetric to `start`, and nothing in the ABI changes for this: from a
plugin's perspective, unload and reload are indistinguishable from the
daemon shutting the host down for any other reason. Reload means the
daemon starts a fresh host at the same path and configuration afterward and
runs `init`/`probe`/`start` again as if this were the first load; nothing
about the previous instance's in-memory state carries over, and none needs
to, since the daemon retains the last known topology and device state on
its own side across the gap (rather than deleting it) and reapplies it once
the reloaded plugin reports back which devices it can serve.

An unload treats every device the plugin owned as withdrawn, the same as if
the hardware had been unplugged: the daemon keeps that device's desired
state and last observations around rather than deleting them, in case a
future reload or reappearance restores it.

## Local dev/test loop

See `docs/development/plugins/demo-plugin.md` for the full walkthrough (build, point a local
config at your `target/debug/lib*.so`, run `luminated` with
`LUMINATED_CONFIG` set, and drive it with `luminate` CLI commands). The
short version:

```bash
cargo build -p luminated -p luminate-plugin-your-thing

cat > /tmp/your-plugin.local.toml <<'EOF'
socket_path = "/tmp/luminated-dev/luminated.sock"
state_path = "/tmp/luminated-dev/state.json"
# Optional: make collection requests without an explicit policy fail if any
# current member cannot apply the requested state. The default is "Skip".
# default_unsupported_policy = "Reject"

# Optional reconciliation overrides. Precedence is device, plugin, global,
# plugin recommendation, then the daemon's Leave fallback.
# reconciliation_policy = "Leave"
# [device_reconciliation]
# living_room_bulb = "Adopt"

[plugin_management]
activation = "explicit"

[[plugins]]
path = "/home/you/dev/luminate/target/debug/libluminate_plugin_your_thing.so"
required = true
# reconciliation = "Adopt"

# Optional plugin-defined settings, delivered privately during initialization.
# [plugins.config]
# discovery = true
EOF

LUMINATED_CONFIG=/tmp/your-plugin.local.toml cargo run -p luminated
```

For keyboard-specific local testing without real hardware, build
`luminate-plugin-demo-keyboard` and use
`docs/development/config/demo-keyboard-plugin.local.toml`.

Then use `luminatectl list --json`, `luminatectl set-effect --effect static`, and
`luminatectl state --device ID --refresh` against it from another terminal.
