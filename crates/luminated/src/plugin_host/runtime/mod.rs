// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Child-side native plugin loading, ABI adaptation, and request execution.

#![allow(
    unsafe_code,
    reason = "only the isolated plugin-host runtime loads and calls native plugin ABI symbols"
)]

use std::env;
use std::ffi::{self, CStr};
use std::io;
#[cfg(test)]
use std::io::Write;
use std::mem::ManuallyDrop;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::slice;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context as _, Result};
use libloading::Library;
use serde::Serialize;

#[cfg(test)]
use luminate_core::capability::{
    CapabilitySet, EffectParameter, HardwareEffectsCapability, StateReadbackCapability,
};
use luminate_host_supervisor::sync_io::{read_frame, write_frame};
use luminate_host_supervisor::{Compatibility, HostHello, SupervisorHello};
use luminate_plugin_api::{
    ABI_VERSION_SYMBOL_NAME, DeviceDescriptor, PLUGIN_ABI_VERSION, PLUGIN_DESCRIPTOR_SYMBOL_NAME,
    PLUGIN_STATE_CBOR_CAPACITY, PluginAbiVersion, PluginApplyBatchFn, PluginApplyResult,
    PluginApplyStatus, PluginApplyUpdateFn, PluginBus, PluginDescriptor, PluginFrameUpload,
    PluginFrameUploadFn, PluginLogLevel, PluginProbeHint, PluginReadRequest, PluginReadStateFn,
    PluginRequestContext, PluginRescanFn, PluginShmFrameApplyFn, PluginShmStreamBeginFn,
    PluginShmStreamEndFn, PluginStateSnapshot, PluginTopologyCborFn, PluginUpdate,
    PluginUpdateBatch, ProbeHintKind, ProbeOutcome, RescanReason, abi_bool,
    reconciliation_policy_from_abi,
};

use crate::normalize;

use super::protocol::{
    ApplyOutcome, BeginShmStreamRequest, HostBootstrap, HostCommand, HostMessage, HostMetadata,
    HostReady, HostRequest, HostResponse, ShmStreamOutcome, WireMetadata,
};
use super::shm::{ShmCallbacks, ShmRuntime};
use super::validation::{PluginCallbacks, validate_hardware_claims, validate_topology_contract};
use super::{
    HOST_LOG_LEVEL_ENV, HOST_MODE_ARGUMENT, MAX_PLUGIN_BATCH_UPDATES, MAX_PLUGIN_READ_TARGETS,
};

/// Same cap the shared frame primitive enforces
/// ([`luminate_host_supervisor::MAX_FRAME_LEN`]), as a `usize` for
/// comparison against FFI-reported sizes (which are always `usize`).
const MAX_HOST_FRAME_SIZE: usize = luminate_host_supervisor::MAX_FRAME_LEN as usize;

static HOST_OUTPUT: OnceLock<Mutex<io::Stdout>> = OnceLock::new();

#[cfg(test)]
use super::supervisor::{
    ExitReporting, HostConnection, HostedPlugin, PendingRequests, current_max_plugin_log_level,
    fail_pending, read_host_messages, terminate_child,
};
#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::process::{Child, Command, Stdio};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(test)]
use std::sync::{Arc, mpsc};

fn host_send(message: &HostMessage) {
    let output = HOST_OUTPUT.get_or_init(|| Mutex::new(io::stdout()));
    #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
    let _ = write_frame(
        &mut *output.lock().expect("plugin host stdout lock poisoned"),
        message,
    );
}

unsafe extern "C" fn host_log_callback(
    plugin_name: *const ffi::c_char,
    level: u8,
    message: *const ffi::c_char,
) {
    // `host_send` locks `HOST_OUTPUT` and now panics on poison; a panic must
    // not unwind back across this `extern "C"` frame into the calling plugin.
    if catch_unwind(AssertUnwindSafe(|| {
        host_send(&HostMessage::Log {
            plugin: read_c_str_lossy(plugin_name),
            level,
            message: read_c_str_lossy(message),
        });
    }))
    .is_err()
    {
        tracing::error!("plugin-host log callback panicked");
    }
}

unsafe extern "C" fn host_topology_changed_callback(_plugin_name: *const ffi::c_char) {
    // See `host_log_callback`: contain the panic before it reaches the ABI boundary.
    if catch_unwind(AssertUnwindSafe(|| {
        host_send(&HostMessage::TopologyChanged);
    }))
    .is_err()
    {
        tracing::error!("plugin-host topology-changed callback panicked");
    }
}

pub fn invocation_from_args() -> Result<Option<(PathBuf, PluginLogLevel)>> {
    let mut arguments = env::args_os().skip(1);
    if arguments.next().as_deref() != Some(ffi::OsStr::new(HOST_MODE_ARGUMENT)) {
        return Ok(None);
    }
    let path = arguments
        .next()
        .context("--plugin-host requires a plugin path")?;
    anyhow::ensure!(
        arguments.next().is_none(),
        "unexpected plugin-host argument"
    );
    let level = env::var(HOST_LOG_LEVEL_ENV)
        .context("plugin host log level was not provided")?
        .parse::<u8>()
        .context("plugin host log level was invalid")?;
    let level = PluginLogLevel::from_abi(level).context("plugin host log level was unknown")?;
    Ok(Some((PathBuf::from(path), level)))
}

pub fn run(path: &Path, max_log_level: PluginLogLevel) -> Result<()> {
    let mut input = io::stdin();

    let hello: SupervisorHello =
        read_frame(&mut input).context("reading plugin-host supervisor hello")?;
    let compatibility = Compatibility::check(hello.protocol_version);
    write_frame(
        &mut io::stdout(),
        &HostHello {
            compatibility: compatibility.clone(),
            protocol_version: luminate_host_supervisor::HOST_SUPERVISOR_PROTOCOL_VERSION,
            host_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
    )
    .context("sending plugin host hello")?;
    if !matches!(compatibility, Compatibility::Compatible) {
        return Ok(());
    }

    let bootstrap: HostBootstrap =
        read_frame(&mut input).context("reading plugin-host bootstrap")?;
    let native = match NativePlugin::load_with_configuration(
        path,
        max_log_level,
        &bootstrap.configuration_cbor,
    ) {
        Ok(native) => native,
        Err(error) => {
            host_send(&HostMessage::Ready(Err(format!("{error:#}"))));
            return Ok(());
        }
    };
    let descriptors = match native.pull_topology() {
        Ok(descriptors) => descriptors,
        Err(error) => {
            host_send(&HostMessage::Ready(Err(format!("{error:#}"))));
            return Ok(());
        }
    };
    host_send(&HostMessage::Ready(Ok(HostReady {
        metadata: WireMetadata::from(&native.metadata),
        descriptors,
    })));

    loop {
        let request: HostRequest = read_frame(&mut input)?;
        let should_stop = matches!(request.command, HostCommand::Shutdown);
        let result = native
            .handle(
                request.command,
                PluginRequestContext::new(Duration::from_millis(request.timeout_millis)),
            )
            .map_err(|error| format!("{error:#}"));
        host_send(&HostMessage::Response {
            id: request.id,
            result,
        });
        if should_stop {
            return Ok(());
        }
    }
}

pub(super) struct NativePlugin {
    pub(super) metadata: HostMetadata,
    pub(super) topology_cbor: Option<PluginTopologyCborFn>,
    pub(super) rescan: Option<PluginRescanFn>,
    pub(super) apply_update_cbor: Option<PluginApplyUpdateFn>,
    pub(super) apply_batch_cbor: Option<PluginApplyBatchFn>,
    pub(super) read_state_cbor: Option<PluginReadStateFn>,
    pub(super) frame_upload_cbor: Option<PluginFrameUploadFn>,
    pub(super) shm_stream_begin: Option<PluginShmStreamBeginFn>,
    pub(super) shm_frame_apply: Option<PluginShmFrameApplyFn>,
    pub(super) shm_stream_end: Option<PluginShmStreamEndFn>,
    pub(super) shm: ShmRuntime,
    pub(super) _library: ManuallyDrop<Library>,
}

impl NativePlugin {
    #[allow(
        clippy::multiple_unsafe_ops_per_block,
        reason = "the isolated host validates related native ABI symbols while the image is pinned"
    )]
    fn load_with_configuration(
        path: &Path,
        max_log_level: PluginLogLevel,
        configuration_cbor: &[u8],
    ) -> Result<Self> {
        // SAFETY: this code runs only in the disposable plugin-host child. ABI
        // compatibility is checked before the descriptor is dereferenced, and
        // the library is pinned until child-process exit after `init` runs.
        unsafe {
            let library =
                Library::new(path).with_context(|| format!("failed to open {}", path.display()))?;
            let abi_version = library
                .get::<*const PluginAbiVersion>(ABI_VERSION_SYMBOL_NAME)
                .with_context(|| format!("missing ABI version symbol in {}", path.display()))?;
            ensure_abi_compatible(**abi_version)?;
            let descriptor = &**library
                .get::<*const PluginDescriptor>(PLUGIN_DESCRIPTOR_SYMBOL_NAME)
                .with_context(|| {
                    format!("missing plugin descriptor symbol in {}", path.display())
                })?;

            (descriptor.init)(
                host_log_callback,
                max_log_level.to_abi(),
                host_topology_changed_callback,
                configuration_cbor.as_ptr(),
                configuration_cbor.len(),
            );
            let validated = (|| {
                let probe_outcome = if let Some(probe) = descriptor.probe {
                    ProbeOutcome::from_abi(probe())
                        .context("plugin returned an unknown probe outcome")?
                } else {
                    ProbeOutcome::Ready
                };
                anyhow::ensure!(
                    probe_outcome != ProbeOutcome::Unsupported,
                    "plugin probe reported this system as unsupported"
                );
                let mut metadata = descriptor_to_metadata(descriptor)?;
                metadata.probe_outcome = probe_outcome;
                anyhow::ensure!(
                    metadata.name.is_ascii(),
                    "plugin name must be ASCII for now: {}",
                    metadata.name
                );
                let shm_fields = [
                    descriptor.shm_stream_begin.is_some(),
                    descriptor.shm_frame_apply.is_some(),
                    descriptor.shm_stream_end.is_some(),
                ];
                anyhow::ensure!(
                    shm_fields.iter().all(|present| *present)
                        || shm_fields.iter().all(|present| !present),
                    "plugin must provide all of shm_stream_begin/shm_frame_apply/shm_stream_end \
                     or none of them"
                );
                Ok(metadata)
            })();
            let metadata = match validated {
                Ok(metadata) => metadata,
                Err(error) => {
                    let _pinned_library = ManuallyDrop::new(library);
                    return Err(error);
                }
            };
            if let Some(start) = descriptor.start {
                start();
            }
            Ok(Self {
                metadata,
                topology_cbor: descriptor.topology_cbor,
                rescan: descriptor.rescan,
                apply_update_cbor: descriptor.apply_update_cbor,
                apply_batch_cbor: descriptor.apply_batch_cbor,
                read_state_cbor: descriptor.read_state_cbor,
                frame_upload_cbor: descriptor.frame_upload_cbor,
                shm_stream_begin: descriptor.shm_stream_begin,
                shm_frame_apply: descriptor.shm_frame_apply,
                shm_stream_end: descriptor.shm_stream_end,
                shm: ShmRuntime::default(),
                _library: ManuallyDrop::new(library),
            })
        }
    }

    /// Whether this plugin implements the shared-memory streaming
    /// callbacks. Load-time validation (`load_with_configuration`) already
    /// guarantees all three or none are present, so checking one stands in
    /// for all three.
    fn has_shm_frame(&self) -> bool {
        self.shm_stream_begin.is_some()
    }

    fn pull_topology(&self) -> Result<Vec<DeviceDescriptor>> {
        let Some(topology_cbor) = self.topology_cbor else {
            return Ok(Vec::new());
        };
        let mut length = 0;
        // SAFETY: the callback comes from the pinned plugin descriptor. This
        // host invokes it on its single command-processing thread and decodes
        // the returned bytes synchronously, so they are consumed before the
        // ABI permits another topology call to invalidate the pointer.
        let pointer = unsafe { topology_cbor(&raw mut length) };
        if pointer.is_null() {
            return Ok(Vec::new());
        }
        anyhow::ensure!(
            length <= MAX_HOST_FRAME_SIZE,
            "plugin topology exceeds size limit"
        );
        // SAFETY: the plugin ABI requires the returned allocation to contain `length` readable bytes.
        let encoded = unsafe { slice::from_raw_parts(pointer, length) };
        let descriptors: Vec<DeviceDescriptor> = ciborium::from_reader(encoded)?;
        validate_hardware_claims(&descriptors)?;
        normalize::normalize_devices(&descriptors).with_context(|| {
            format!("plugin {} produced an invalid topology", self.metadata.name)
        })?;
        validate_topology_contract(
            &descriptors,
            PluginCallbacks {
                apply: self.apply_update_cbor.is_some(),
                read_state: self.read_state_cbor.is_some(),
                frame_upload: self.frame_upload_cbor.is_some(),
                shm_frame: self.has_shm_frame(),
            },
        )
        .with_context(|| {
            format!(
                "plugin {} produced topology incompatible with its callbacks",
                self.metadata.name
            )
        })?;
        Ok(descriptors)
    }

    /// Asks the plugin to discard any cached view of its hardware, so the
    /// `pull_topology` that follows sees freshly enumerated state.
    ///
    /// A plugin without the callback needs nothing: its `topology_cbor`
    /// already enumerates afresh on every call, so the daemon's re-pull is
    /// the whole rescan.
    fn rescan(&self, reason: RescanReason) {
        let Some(rescan) = self.rescan else {
            tracing::debug!(
                plugin = %self.metadata.name,
                "plugin has no rescan callback; the topology re-pull is the whole rescan"
            );
            return;
        };
        tracing::debug!(plugin = %self.metadata.name, reason = ?reason, "asking plugin to re-enumerate");
        // SAFETY: the plugin ABI requires this callback to accept the reason
        // byte and return without taking ownership of anything.
        unsafe { rescan(reason.to_abi()) };
    }

    fn handle(&self, command: HostCommand, context: PluginRequestContext) -> Result<HostResponse> {
        match command {
            HostCommand::Topology => Ok(HostResponse::Topology(self.pull_topology()?)),
            HostCommand::Rescan { reason } => {
                // An unknown byte still means "re-enumerate": the daemon is
                // about to re-pull topology either way, so refusing to
                // invalidate the plugin's cache because a newer daemon named
                // a reason this build doesn't know would preserve exactly the
                // staleness a rescan exists to clear.
                self.rescan(RescanReason::from_abi(reason).unwrap_or(RescanReason::Operator));
                Ok(HostResponse::Rescan)
            }
            HostCommand::Apply(update) => Ok(HostResponse::Apply(self.apply(context, &update)?)),
            HostCommand::ApplyBatch(updates) => {
                anyhow::ensure!(
                    updates.len() <= MAX_PLUGIN_BATCH_UPDATES,
                    "plugin batch contains {} updates, limit is {MAX_PLUGIN_BATCH_UPDATES}",
                    updates.len()
                );
                Ok(HostResponse::Batch(self.apply_batch(context, &updates)?))
            }
            HostCommand::ReadState(request) => {
                anyhow::ensure!(
                    request.targets.len() <= MAX_PLUGIN_READ_TARGETS,
                    "plugin read contains {} targets, limit is {MAX_PLUGIN_READ_TARGETS}",
                    request.targets.len()
                );
                Ok(HostResponse::State(self.read_state(context, &request)?))
            }
            HostCommand::UploadFrame(frame_upload) => Ok(HostResponse::Frame(
                self.upload_frame(context, &frame_upload)?,
            )),
            HostCommand::BeginShmStream(request) => Ok(HostResponse::ShmStream(
                self.begin_shm_stream(context, &request)?,
            )),
            HostCommand::EndShmStream { target, generation } => Ok(HostResponse::ShmStream(
                self.shm.end(&target, generation, context),
            )),
            HostCommand::ResetShmStream { target, generation } => {
                Ok(HostResponse::ShmStream(self.shm.reset(&target, generation)))
            }
            HostCommand::Shutdown => {
                self.shm.end_all(context);
                Ok(HostResponse::Shutdown)
            }
        }
    }

    fn begin_shm_stream(
        &self,
        context: PluginRequestContext,
        request: &BeginShmStreamRequest,
    ) -> Result<ShmStreamOutcome> {
        let (Some(begin), Some(apply), Some(end)) = (
            self.shm_stream_begin,
            self.shm_frame_apply,
            self.shm_stream_end,
        ) else {
            return Ok(ShmStreamOutcome::Unsupported(
                "plugin does not implement shared-memory frame streaming".to_owned(),
            ));
        };
        self.shm.begin(
            &self.metadata.name,
            request,
            ShmCallbacks { begin, apply, end },
            context,
        )
    }

    fn apply(&self, context: PluginRequestContext, update: &PluginUpdate) -> Result<ApplyOutcome> {
        let Some(callback) = self.apply_update_cbor else {
            return Ok(ApplyOutcome::Unsupported(
                "plugin exposes no mutation callback".to_owned(),
            ));
        };
        let payload = encode_cbor(update)?;
        let mut result = PluginApplyResult::internal("plugin did not write an apply result");
        // SAFETY: callback belongs to the pinned image and payload is a live,
        // NUL-terminated string and result is writable for the duration of this
        // call.
        let processed = abi_bool(unsafe {
            callback(context, payload.as_ptr(), payload.len(), &raw mut result)
        });
        finish_apply_call(
            processed,
            &result,
            "plugin could not process the apply envelope",
        )
    }

    fn upload_frame(
        &self,
        context: PluginRequestContext,
        frame_upload: &PluginFrameUpload,
    ) -> Result<ApplyOutcome> {
        let Some(callback) = self.frame_upload_cbor else {
            return Ok(ApplyOutcome::Unsupported(
                "plugin exposes no frame-upload callback".to_owned(),
            ));
        };
        let payload = encode_cbor(frame_upload)?;
        let mut result = PluginApplyResult::internal("plugin did not write a frame result");
        // SAFETY: callback belongs to the pinned image and payload is a live
        // buffer and result is writable for the duration of this call.
        let processed = abi_bool(unsafe {
            callback(context, payload.as_ptr(), payload.len(), &raw mut result)
        });
        finish_apply_call(
            processed,
            &result,
            "plugin could not process the frame envelope",
        )
    }

    fn apply_batch(
        &self,
        context: PluginRequestContext,
        updates: &[PluginUpdate],
    ) -> Result<Vec<ApplyOutcome>> {
        let Some(callback) = self.apply_batch_cbor else {
            return updates
                .iter()
                .map(|update| self.apply(context, update))
                .collect();
        };
        let mut payload = Vec::new();
        ciborium::into_writer(
            &PluginUpdateBatch {
                updates: updates.to_vec(),
            },
            &mut payload,
        )?;
        let mut results =
            vec![PluginApplyResult::internal("plugin did not write a batch result"); updates.len()];
        // SAFETY: callback belongs to the pinned image; input and result buffers
        // remain valid and have the ABI-required lengths for the entire call.
        let processed = abi_bool(unsafe {
            callback(
                context,
                payload.as_ptr(),
                payload.len(),
                results.as_mut_ptr(),
                results.len(),
            )
        });
        if !processed {
            return Ok(updates
                .iter()
                .map(|_| {
                    ApplyOutcome::Internal("plugin could not process the batch envelope".to_owned())
                })
                .collect());
        }
        results.iter().map(decode_apply_result).collect()
    }

    fn read_state(
        &self,
        context: PluginRequestContext,
        request: &PluginReadRequest,
    ) -> Result<PluginStateSnapshot> {
        let callback = self
            .read_state_cbor
            .context("plugin exposes no state-readback callback")?;
        let mut payload = Vec::new();
        ciborium::into_writer(request, &mut payload)?;
        let mut output = vec![0_u8; PLUGIN_STATE_CBOR_CAPACITY];
        // SAFETY: callback belongs to the pinned image; both buffers remain
        // valid for the call and the plugin may write at most the stated size.
        let length = unsafe {
            callback(
                context,
                payload.as_ptr(),
                payload.len(),
                output.as_mut_ptr(),
                output.len(),
            )
        };
        anyhow::ensure!(
            length != usize::MAX,
            "plugin could not encode state snapshot"
        );
        anyhow::ensure!(
            length <= output.len(),
            "plugin state snapshot requires {length} bytes, limit is {}",
            output.len()
        );
        let encoded = output
            .get(..length)
            .context("plugin state snapshot length exceeded its buffer")?;
        ciborium::from_reader(encoded).context("decoding plugin state snapshot")
    }
}

pub(super) fn encode_cbor(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    ciborium::into_writer(value, &mut payload)?;
    Ok(payload)
}

fn finish_apply_call(
    processed: bool,
    result: &PluginApplyResult,
    failure: &str,
) -> Result<ApplyOutcome> {
    if processed {
        decode_apply_result(result)
    } else {
        Ok(ApplyOutcome::Internal(failure.to_owned()))
    }
}

pub(super) fn decode_apply_result(result: &PluginApplyResult) -> Result<ApplyOutcome> {
    let (status, diagnostic) = result
        .decode()
        .map_err(|error| anyhow::anyhow!("plugin returned malformed apply result: {error}"))?;
    let diagnostic = diagnostic.to_owned();
    Ok(match status {
        PluginApplyStatus::Applied => ApplyOutcome::Applied,
        PluginApplyStatus::Unsupported => ApplyOutcome::Unsupported(diagnostic),
        PluginApplyStatus::InvalidArgument => ApplyOutcome::InvalidArgument(diagnostic),
        PluginApplyStatus::Io => ApplyOutcome::Io(diagnostic),
        PluginApplyStatus::Unavailable => ApplyOutcome::Unavailable(diagnostic),
        PluginApplyStatus::RateLimited => ApplyOutcome::RateLimited {
            diagnostic,
            retry_after_ms: result.retry_after_ms(),
        },
        PluginApplyStatus::Internal => ApplyOutcome::Internal(diagnostic),
    })
}

pub(crate) fn ensure_abi_compatible(abi_version: u32) -> Result<()> {
    anyhow::ensure!(
        abi_version == PLUGIN_ABI_VERSION,
        "plugin ABI mismatch: plugin={abi_version}, daemon={PLUGIN_ABI_VERSION}"
    );
    Ok(())
}

fn descriptor_to_metadata(descriptor: &PluginDescriptor) -> Result<HostMetadata> {
    Ok(HostMetadata {
        name: read_c_string(descriptor.name, "plugin name")?,
        version: read_c_string(descriptor.version, "plugin version")?,
        priority: descriptor.priority,
        recommended_reconciliation: reconciliation_policy_from_abi(
            descriptor.recommended_reconciliation,
        )
        .context("plugin returned unknown reconciliation recommendation")?,
        probe_outcome: ProbeOutcome::Ready,
        buses: read_bus_slice(descriptor.buses, descriptor.bus_count),
        vendors: read_slice_copy(descriptor.vendors, descriptor.vendor_count),
        probe_hints: read_probe_hints(descriptor.probe_hints, descriptor.probe_hint_count)?,
    })
}

fn read_c_string(pointer: *const ffi::c_char, field: &str) -> Result<String> {
    anyhow::ensure!(!pointer.is_null(), "{field} pointer was null");
    // SAFETY: pointer was checked and the plugin ABI requires a C string.
    Ok(unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .with_context(|| format!("{field} was not valid UTF-8"))?
        .to_owned())
}

fn read_c_str_lossy(pointer: *const ffi::c_char) -> String {
    if pointer.is_null() {
        return "<null>".to_owned();
    }
    // SAFETY: the plugin callback ABI requires a non-null pointer to a C string.
    String::from_utf8_lossy(unsafe { CStr::from_ptr(pointer) }.to_bytes()).into_owned()
}

fn read_slice_copy<T: Copy>(pointer: *const T, length: usize) -> Vec<T> {
    if pointer.is_null() || length == 0 {
        return Vec::new();
    }
    // SAFETY: plugin descriptor arrays are contiguous and live with the image.
    unsafe { slice::from_raw_parts(pointer, length) }.to_vec()
}

pub(crate) fn read_bus_slice(pointer: *const PluginBus, length: usize) -> Vec<PluginBus> {
    if pointer.is_null() || length == 0 {
        return Vec::new();
    }
    // SAFETY: PluginBus is repr(u32); reading raw codes avoids invalid enums.
    unsafe { slice::from_raw_parts(pointer.cast::<u32>(), length) }
        .iter()
        .map(|&code| PluginBus::from_abi(code).unwrap_or(PluginBus::Unknown))
        .collect()
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct RawProbeHint {
    pub kind: u32,
    pub value: *const ffi::c_char,
}

const _: () = {
    assert!(
        size_of::<RawProbeHint>() == size_of::<PluginProbeHint>(),
        "raw probe-hint mirror must preserve ABI size"
    );
    assert!(
        align_of::<RawProbeHint>() == align_of::<PluginProbeHint>(),
        "raw probe-hint mirror must preserve ABI alignment"
    );
};

pub(crate) fn read_probe_hints(
    pointer: *const PluginProbeHint,
    length: usize,
) -> Result<Vec<String>> {
    if pointer.is_null() || length == 0 {
        return Ok(Vec::new());
    }
    // SAFETY: RawProbeHint mirrors the repr(C) layout while keeping kind scalar.
    unsafe { slice::from_raw_parts(pointer.cast::<RawProbeHint>(), length) }
        .iter()
        .map(|hint| {
            let _known_kind = ProbeHintKind::from_abi(hint.kind);
            if hint.value.is_null() {
                Ok(String::new())
            } else {
                // SAFETY: non-null values are C strings under the plugin ABI.
                Ok(unsafe { CStr::from_ptr(hint.value) }
                    .to_str()
                    .context("probe hint was not valid UTF-8")?
                    .to_owned())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
