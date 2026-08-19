// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! The plugin entry-point descriptor and the native callback signatures it
//! carries.

use std::ffi::{CStr, c_char};

use luminate_core::control;
use luminate_core::shm_frame::ShmFrameHeader;

use crate::abi::{PluginBus, ProbeHintKind};
use crate::apply::PluginApplyResult;
use crate::deadline::PluginRequestContext;
use crate::settings::PluginSettingDescriptor;
use crate::setup::PluginSetupWorkflowDescriptor;

/// NUL-terminated name of the symbol the daemon looks up to get the plugin descriptor.
pub const PLUGIN_DESCRIPTOR_SYMBOL_NAME: &CStr = c"LUMINATE_PLUGIN_DESCRIPTOR";

/// NUL-terminated name of the symbol the daemon looks up to get the plugin ABI version.
pub const ABI_VERSION_SYMBOL_NAME: &CStr = c"LUMINATE_PLUGIN_ABI_VERSION";

/// An informational vendor/product ID pair a plugin claims to support.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginVendorId {
    pub vendor: u32,
    pub product: u32,
}

/// One discovery hint, e.g. a specific USB VID:PID string for
/// `ProbeHintKind::UsbVidPid`. `value` must be NUL-terminated and, like the
/// rest of `PluginDescriptor`, valid for the lifetime of the loaded plugin.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PluginProbeHint {
    pub kind: ProbeHintKind,
    pub value: *const c_char,
}

// SAFETY: these FFI descriptor records are immutable metadata blocks whose raw
// pointers refer to static storage owned by the plugin image.
unsafe impl Sync for PluginProbeHint {}

/// The single record a plugin exposes to the daemon via its entry point.
/// Every pointer field must reference `static` storage that outlives the
/// loaded plugin: the daemon may read this descriptor at any point while
/// the host process that loaded it is still running, and there is no
/// `dlclose` or ABI teardown callback to signal that it should stop. A
/// plugin unload or reload terminates the whole host process instead of
/// unloading the image from a still-running one; see the daemon's
/// `PluginManager::unload_plugin`/`reload_plugin`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PluginDescriptor {
    /// NUL-terminated, must be ASCII. Used in logs and for name-based
    /// config lookup (`lib{name}.so` / `{name}.so`).
    pub name: *const c_char,
    /// NUL-terminated version string; informational only.
    pub version: *const c_char,

    /// Resolves logical-ID and exclusive hardware-claim conflicts: highest
    /// priority wins, first-loaded is the tie-breaker.
    pub priority: i32,
    /// Policy recommendation only; daemon/user configuration remains authoritative.
    /// Stored as a raw byte so an unknown value cannot create an invalid enum.
    pub recommended_reconciliation: u8,

    pub buses: *const PluginBus,
    pub bus_count: usize,

    pub vendors: *const PluginVendorId,
    pub vendor_count: usize,

    pub probe_hints: *const PluginProbeHint,
    pub probe_hint_count: usize,

    /// Static schema which can be inspected without invoking lifecycle
    /// callbacks. Constructors and static initialization used to produce this
    /// metadata must have no externally visible side effects.
    pub settings: *const PluginSettingDescriptor,
    pub setting_count: usize,

    /// Static setup workflow metadata. A null pointer with a zero count means
    /// this plugin has no setup facility.
    pub setup_workflows: *const PluginSetupWorkflowDescriptor,
    pub setup_workflow_count: usize,

    /// Called once after `abi_version` is validated and before `probe`. The
    /// plugin may retain the supplied daemon callbacks for its lifetime.
    /// Plugin logging must use the supplied callback and maximum level because
    /// a plugin `cdylib` has its own `tracing` subscriber state.
    pub init: PluginInitFn,
    /// Called once at load time to distinguish unsupported, dormant, and
    /// ready providers. `None` means ready.
    pub probe: Option<PluginProbeFn>,
    /// Called once after probing accepts the plugin and before its initial
    /// topology is fetched. Discovery and polling threads should start here.
    pub start: Option<PluginStartFn>,
    /// Called when the daemon needs this plugin's hardware re-enumerated
    /// (on resume from suspend, on a device-change notification, or on an
    /// explicit operator request), immediately before `topology_cbor` is
    /// re-pulled.
    ///
    /// `None` is correct for a plugin whose `topology_cbor` already
    /// enumerates hardware afresh on every call, which is the common case:
    /// the daemon's re-pull is then sufficient on its own. Implement this
    /// when `topology_cbor` would instead answer from a cache that a
    /// background discovery thread owns. Without it, such a plugin keeps
    /// reporting its pre-suspend view until its own poll interval catches
    /// up, which is exactly the staleness a rescan exists to clear.
    pub rescan: Option<PluginRescanFn>,
    /// Called once at load time to fetch this plugin's topology. `None`
    /// means the plugin contributes no devices.
    pub topology_cbor: Option<PluginTopologyCborFn>,
    /// Called once per mutation targeting one of this plugin's devices.
    /// `None` means mutations against this plugin's devices are unsupported.
    pub apply_update_cbor: Option<PluginApplyUpdateFn>,
    /// Called instead of `apply_update_cbor`, once per batch, when the
    /// daemon has multiple updates to apply together (e.g. startup replay
    /// of persisted state) and the plugin opts in by providing this. `None`
    /// means the daemon falls back to calling `apply_update_cbor` once per
    /// update in the batch, in order. Batching is only a plugin-side
    /// optimization, never required for correctness.
    pub apply_batch_cbor: Option<PluginApplyBatchFn>,
    /// Optional bulk snapshot callback. `None` means the plugin has no live
    /// state-readback mechanism, regardless of any persistence support.
    pub read_state_cbor: Option<PluginReadStateFn>,
    /// Optional streamed-frame callback. `None` means any target advertising
    /// `FrameUploadCapability` is a contract violation the host rejects at
    /// topology-validation time.
    pub frame_upload_cbor: Option<PluginFrameUploadFn>,

    /// Begins a shared-memory frame stream. `None` is correct unless a
    /// target advertises `ShmFrameCapability`; the three shared-memory
    /// fields are validated as an all-or-nothing group at load time, the
    /// same way `frame_upload_cbor` is.
    pub shm_stream_begin: Option<PluginShmStreamBeginFn>,
    /// Applies one shared-memory frame sample. `None` is correct alongside
    /// `shm_stream_begin: None`.
    pub shm_frame_apply: Option<PluginShmFrameApplyFn>,
    /// Ends a shared-memory frame stream. `None` is correct alongside
    /// `shm_stream_begin: None`.
    pub shm_stream_end: Option<PluginShmStreamEndFn>,

    /// Executes one isolated setup step. `None` is required when
    /// `setup_workflow_count` is zero.
    pub setup_cbor: Option<PluginSetupFn>,
}

// SAFETY: the descriptor is immutable plugin metadata whose pointers refer to
// static storage for the lifetime of the loaded plugin.
unsafe impl Sync for PluginDescriptor {}

/// Signature of the plugin ABI version constant. The signature of this value
/// is immutable and cannot change.
pub type PluginAbiVersion = u32;

/// Signature of `PluginDescriptor::init`. `max_level` is the daemon's
/// currently effective log verbosity (resolved once from its own logging
/// configuration), so the plugin can install a subscriber that matches it
/// instead of resolving verbosity itself. It is a `PluginLogLevel` ABI byte
/// (decode with [`crate::abi::PluginLogLevel::from_abi`]) rather than the enum
/// itself, so a mismatched build can't hand across an out-of-range
/// discriminant. `configuration_cbor` points to `configuration_len` host-owned
/// CBOR bytes that are valid only for this call. The declaration
/// macro stores an owned parsed copy before invoking the plugin's optional
/// Rust initializer; plugins read it through the [`crate::configuration`]
/// module.
pub type PluginInitFn = unsafe extern "C" fn(
    log: PluginLogFn,
    max_level: u8,
    notify_topology_changed: PluginNotifyFn,
    configuration_cbor: *const u8,
    configuration_len: usize,
);

/// Signature of the topology-dirty notification passed to [`PluginInitFn`].
///
/// A plugin may call this from any thread after initialization. `plugin_name`
/// must be NUL-terminated and valid for the duration of the call. The callback
/// carries no topology payload: it only tells the daemon to re-pull the
/// plugin's authoritative [`PluginTopologyCborFn`] snapshot.
pub type PluginNotifyFn = unsafe extern "C" fn(plugin_name: *const c_char);

/// Signature of the logging handle passed to `PluginInitFn`. `plugin_name`
/// and `message` must both be NUL-terminated and valid only for the
/// duration of the call; the daemon-side implementation must not panic or
/// unwind, since this is called directly from plugin code across the FFI
/// boundary. `level` is a `PluginLogLevel` ABI byte (decode with
/// [`crate::abi::PluginLogLevel::from_abi`]) rather than the enum, so a
/// plugin built against a different revision can't hand across an
/// out-of-range discriminant.
pub type PluginLogFn =
    unsafe extern "C" fn(plugin_name: *const c_char, level: u8, message: *const c_char);

/// Signature of `PluginDescriptor::probe`. The result is a validated
/// [`crate::abi::ProbeOutcome`] ABI byte.
pub type PluginProbeFn = unsafe extern "C" fn() -> u8;

/// Signature of `PluginDescriptor::start`.
pub type PluginStartFn = unsafe extern "C" fn();

/// Signature of `PluginDescriptor::rescan`. `reason` is a
/// [`crate::abi::RescanReason`] ABI byte (decode with
/// [`crate::abi::RescanReason::from_abi`]) rather than the enum itself, so a
/// mismatched build cannot hand across an out-of-range discriminant.
///
/// The daemon calls this before re-pulling `topology_cbor`, and does not call
/// the two concurrently. It is a request to discard any cached view of the
/// hardware, not to report anything: whatever the plugin discovers surfaces
/// through the following `topology_cbor` call as usual.
pub type PluginRescanFn = unsafe extern "C" fn(reason: u8);

/// Signature of `PluginDescriptor::topology_cbor`.
///
/// The daemon invokes this callback serially. It copies or decodes all reported
/// bytes before invoking the callback again, and does not read the pointer after
/// that next invocation begins. The returned pointer must reference a CBOR
/// encoding of `Vec<DeviceDescriptor>` with the reported length and remain
/// valid until then. Static plugins can build it once in a `OnceLock<Vec<u8>>`;
/// dynamic plugins may replace a cached snapshot between calls because the
/// daemon's serialized invocation contract ensures no earlier pointer is still
/// in use.
pub type PluginTopologyCborFn = unsafe extern "C" fn(length: *mut usize) -> *const u8;

/// Signature of one isolated setup step.
///
/// The request is a CBOR [`crate::PluginSetupRequest`]. The returned pointer
/// references a CBOR `Result<PluginSetupStep, String>` and remains valid
/// until the next setup call in the same process.
pub type PluginSetupFn = unsafe extern "C" fn(
    request_cbor: *const u8,
    request_len: usize,
    response_len: *mut usize,
) -> *const u8;

/// Maximum CBOR snapshot accepted from a native plugin in one read operation.
pub const PLUGIN_STATE_CBOR_CAPACITY: usize = 1024 * 1024;

/// Encodes an optional plugin policy recommendation for the native ABI.
#[must_use]
pub const fn reconciliation_policy_to_abi(policy: Option<control::ReconciliationPolicy>) -> u8 {
    match policy {
        None => 0,
        Some(control::ReconciliationPolicy::Restore) => 1,
        Some(control::ReconciliationPolicy::Adopt) => 2,
        Some(control::ReconciliationPolicy::Leave) => 3,
    }
}

/// Decodes an untrusted plugin policy recommendation.
#[must_use]
pub const fn reconciliation_policy_from_abi(
    value: u8,
) -> Option<Option<control::ReconciliationPolicy>> {
    match value {
        0 => Some(None),
        1 => Some(Some(control::ReconciliationPolicy::Restore)),
        2 => Some(Some(control::ReconciliationPolicy::Adopt)),
        3 => Some(Some(control::ReconciliationPolicy::Leave)),
        _ => None,
    }
}

/// Signature of `PluginDescriptor::apply_update_cbor`. The input is a
/// length-delimited CBOR encoding of one [`crate::PluginUpdate`]. `result`
/// points to one caller-initialized writable result slot. A nonzero return
/// means the slot was written and may be decoded; zero means the callback
/// could not process the ABI envelope and the host ignores the slot.
pub type PluginApplyUpdateFn = unsafe extern "C" fn(
    context: PluginRequestContext,
    update_cbor: *const u8,
    update_len: usize,
    result: *mut PluginApplyResult,
) -> u8;

/// Signature of `PluginDescriptor::apply_batch_cbor`. The input is a
/// length-delimited CBOR encoding of one `PluginUpdateBatch`, valid only for
/// the duration of the call. `results` points to a caller-allocated buffer
/// of exactly `results_len` [`PluginApplyResult`] slots
/// (`results_len == updates.len()`); the plugin must write one structured
/// entry per update, in the same order, before returning. The overall
/// `u8` return value signals whether the call could be processed at all
/// (`0` on e.g. a CBOR decode failure, in which case the daemon must not trust
/// `results` and instead treats every entry in the batch as failed; decode
/// with [`crate::abi::abi_bool`]). Must not panic across the FFI boundary.
pub type PluginApplyBatchFn = unsafe extern "C" fn(
    context: PluginRequestContext,
    batch_cbor: *const u8,
    batch_len: usize,
    results: *mut PluginApplyResult,
    results_len: usize,
) -> u8;

/// Signature of `PluginDescriptor::read_state_cbor`. The callback performs at
/// most one hardware snapshot and writes its CBOR response into caller-owned
/// memory. It returns the required byte length; a value larger than
/// `output_capacity` is rejected without a second, potentially inconsistent,
/// hardware read.
pub type PluginReadStateFn = unsafe extern "C" fn(
    context: PluginRequestContext,
    request_cbor: *const u8,
    request_len: usize,
    output: *mut u8,
    output_capacity: usize,
) -> usize;

/// Signature of `PluginDescriptor::frame_upload_cbor`. The input is a
/// length-delimited CBOR encoding of one [`crate::PluginFrameUpload`] (target
/// plus frame envelope, mirroring how [`PluginApplyUpdateFn`] bundles its
/// target inside [`crate::PluginUpdate`]). Stream ownership,
/// generation/sequence ordering, and rate limiting are all host
/// responsibilities, already resolved and validated before this callback is
/// invoked, not this callback's. `result` points to one caller-initialized
/// writable result slot; the return value has the same meaning as
/// [`PluginApplyUpdateFn`]'s.
pub type PluginFrameUploadFn = unsafe extern "C" fn(
    context: PluginRequestContext,
    frame_upload_cbor: *const u8,
    frame_upload_len: usize,
    result: *mut PluginApplyResult,
) -> u8;

/// Signature of `PluginDescriptor::shm_stream_begin`. `target_cbor` is a
/// length-delimited CBOR encoding of one `PluginTarget`; `pixel_format` is a
/// [`luminate_core::shm_frame::ShmPixelFormat`] discriminant, already
/// validated by the host against this target's advertised
/// `ShmFrameCapability` before this call. On success the callback writes an
/// opaque, plugin-minted, nonzero handle to `*handle_out`; that same value
/// is echoed back verbatim on every subsequent `shm_frame_apply`/
/// `shm_stream_end` call for this stream, so the plugin may use it however
/// it likes (for example, a boxed pointer cast to `u64`) to recover its own
/// per-stream state. `result` points to one caller-initialized writable
/// result slot; the return value has the same meaning as
/// [`PluginApplyUpdateFn`]'s.
pub type PluginShmStreamBeginFn = unsafe extern "C" fn(
    context: PluginRequestContext,
    target_cbor: *const u8,
    target_len: usize,
    pixel_format: u32,
    pixel_count: u32,
    generation: u32,
    handle_out: *mut u64,
    result: *mut PluginApplyResult,
) -> u8;

/// Signature of `PluginDescriptor::shm_frame_apply`. `handle` is the value
/// [`PluginShmStreamBeginFn`] wrote for this stream. `header` and the
/// `pixel_count * pixel_format.bytes_per_pixel()` bytes at `pixels` are a
/// borrowed view into the shared-memory segment for the duration of this
/// call only; the callback must not retain either pointer past its return.
///
/// The host runs each stream's `shm_stream_begin`, `shm_frame_apply`, and
/// `shm_stream_end` calls on a thread dedicated to that stream. A given
/// `handle` is therefore used by at most one thread at a time.
///
/// Different streams may run concurrently with each other and with ordinary
/// commands such as `apply_update_cbor`. Plugins must synchronize state shared
/// across streams or callback families. This concurrency is why
/// `LuminatePlugin` requires `Sync`.
///
/// `result` points to one caller-initialized writable result slot; the return
/// value has the same meaning as [`PluginApplyUpdateFn`]'s.
pub type PluginShmFrameApplyFn = unsafe extern "C" fn(
    context: PluginRequestContext,
    handle: u64,
    header: ShmFrameHeader,
    pixels: *const u8,
    pixels_len: usize,
    result: *mut PluginApplyResult,
) -> u8;

/// Signature of `PluginDescriptor::shm_stream_end`. `handle` is the value
/// [`PluginShmStreamBeginFn`] wrote for this stream; the plugin should
/// release whatever per-stream state it associated with `handle` and treat
/// it as invalid afterwards. Infallible and best-effort by design, mirroring
/// the daemon's own idempotent stream-teardown semantics: there is no
/// result slot, and the host does not retry a failed cleanup.
pub type PluginShmStreamEndFn =
    unsafe extern "C" fn(context: PluginRequestContext, handle: u64, generation: u32);

#[cfg(test)]
#[path = "descriptor_tests.rs"]
mod tests;
