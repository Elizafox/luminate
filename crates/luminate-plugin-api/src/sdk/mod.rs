// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Safe Rust facade and native-ABI adapters for plugin authors.
//!
//! Plugin implementations provide typed Rust methods. The export macro owns
//! the raw pointers, CBOR envelopes, stable topology storage, request-context
//! installation, and panic containment required by the native ABI.

use std::ffi::CStr;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::slice;
use std::sync::{Mutex, OnceLock};

use luminate_core::shm_frame::ShmFrameHeader;

use crate::apply::decode_update_batch;
use crate::{
    PluginApplyResult, PluginError, PluginReadError, PluginReadRequest, PluginRequestContext,
    PluginStateSnapshot, PluginUpdate, ProbeOutcome, RescanReason, configuration,
    dispatch_frame_upload_cbor, dispatch_shm_frame, dispatch_shm_stream_begin,
    dispatch_update_batch_cbor, dispatch_update_cbor, logging, notification,
};

mod complete_shadow;
mod export;
mod traits;

pub use complete_shadow::{
    CompleteShadow, CompleteShadowError, stage_frame_updates, stage_map_updates,
};

pub use traits::{
    BatchPlugin, FrameStreamingPlugin, LuminatePlugin, ReadablePlugin, RescanPlugin, SetupPlugin,
    ShmFrameStreamingPlugin, StartPlugin,
};

pub use crate::dynamic_registry::{
    DiscoveryPacer, DynamicDeviceRegistry, RegistryExpiry, RegistryPoisonError, RegistryRefresh,
};

fn plugin<P: LuminatePlugin>(
    instance: &OnceLock<Result<P, PluginError>>,
) -> Result<&P, PluginError> {
    match instance.get() {
        Some(Ok(plugin)) => Ok(plugin),
        Some(Err(error)) => Err(error.clone()),
        None => Err(PluginError::Internal(
            "plugin callback ran before initialization".to_owned(),
        )),
    }
}

fn panic_diagnostic() -> PluginError {
    PluginError::Internal("plugin callback panicked".to_owned())
}

/// Executes one typed setup step and stores its CBOR result.
///
/// # Safety
///
/// `request_cbor` must identify `request_len` readable bytes. When non-null,
/// `response_len` must point to a writable `usize`. Calls using the same cache
/// must be serialized and the caller must copy the returned bytes before the
/// next call.
#[doc(hidden)]
pub unsafe fn setup<P: SetupPlugin>(
    cache: &OnceLock<Mutex<Vec<u8>>>,
    request_cbor: *const u8,
    request_len: usize,
    response_len: *mut usize,
) -> *const u8 {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        if request_cbor.is_null() || request_len > crate::PLUGIN_SETUP_CBOR_CAPACITY {
            return Err("invalid setup request buffer".to_owned());
        }
        // SAFETY: the callback contract supplies this readable region.
        let payload = unsafe { slice::from_raw_parts(request_cbor, request_len) };
        let request = crate::decode_cbor(payload)?;
        P::setup(request).map_err(|error| error.to_string())
    }))
    .unwrap_or_else(|_| Err("plugin setup callback panicked".to_owned()));
    let encoded = crate::encode_cbor(&outcome).unwrap_or_else(|error| {
        crate::encode_cbor(&Result::<crate::PluginSetupStep, String>::Err(error))
            .unwrap_or_default()
    });
    let Ok(mut storage) = cache.get_or_init(|| Mutex::new(Vec::new())).lock() else {
        return ptr::null();
    };
    *storage = encoded;
    if storage.len() > crate::PLUGIN_SETUP_CBOR_CAPACITY {
        return ptr::null();
    }
    if !response_len.is_null() {
        // SAFETY: the callback contract supplies a writable length slot.
        unsafe { response_len.write(storage.len()) };
    }
    storage.as_ptr()
}

/// Converts one plugin outcome to its ABI result, shared by every call site
/// that applies a single update or frame.
fn outcome_to_result(outcome: Result<(), PluginError>) -> PluginApplyResult {
    outcome.map_or_else(PluginError::into_apply_result, |()| {
        PluginApplyResult::applied()
    })
}

/// Applies one update through the plugin's typed `apply` method, shared by
/// the single-update and ordered-batch dispatch paths.
fn apply_one<P: LuminatePlugin>(
    instance: &OnceLock<Result<P, PluginError>>,
    context: PluginRequestContext,
    update: &PluginUpdate,
) -> PluginApplyResult {
    outcome_to_result(plugin(instance).and_then(|plugin| plugin.apply(&context, update)))
}

/// Initializes the process-lifetime safe plugin instance.
#[doc(hidden)]
pub unsafe fn initialize<P: LuminatePlugin>(
    instance: &OnceLock<Result<P, PluginError>>,
    name: &'static CStr,
    log: crate::PluginLogFn,
    max_level: u8,
    notify_topology_changed: crate::PluginNotifyFn,
    configuration_cbor: *const u8,
    configuration_len: usize,
) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let max_level =
            crate::PluginLogLevel::from_abi(max_level).unwrap_or(crate::PluginLogLevel::Info);
        logging::init(name, log, max_level);
        notification::init(name, notify_topology_changed);
        // SAFETY: forwarded from the descriptor initialization contract and
        // copied before the constructor runs.
        unsafe { configuration::init(configuration_cbor, configuration_len) };
        P::new()
    }))
    .unwrap_or_else(|_| Err(panic_diagnostic()));

    if instance.set(result).is_err() {
        tracing::error!("plugin instance was initialized more than once");
    }
}

/// Runs the typed probe with panic containment.
#[doc(hidden)]
pub fn probe<P: LuminatePlugin>(instance: &OnceLock<Result<P, PluginError>>) -> u8 {
    catch_unwind(AssertUnwindSafe(|| {
        plugin(instance).map_or(ProbeOutcome::Unsupported, LuminatePlugin::probe)
    }))
    .unwrap_or(ProbeOutcome::Unsupported)
    .to_abi()
}

/// Starts an accepted dynamic provider with panic containment.
#[doc(hidden)]
pub fn start<P: StartPlugin>(instance: &OnceLock<Result<P, PluginError>>) {
    if catch_unwind(AssertUnwindSafe(|| {
        plugin(instance).map(StartPlugin::start)
    }))
    .is_err()
    {
        tracing::error!("safe plugin start callback panicked");
    }
}

/// Runs the typed rescan with panic containment.
///
/// An unknown `reason` byte is treated as [`RescanReason::Operator`] rather
/// than skipped: the daemon is about to re-pull topology either way, so
/// dropping the cache-invalidation half because a newer daemon named a reason
/// this plugin build doesn't know would leave exactly the stale topology a
/// rescan exists to clear.
#[doc(hidden)]
pub fn rescan<P: RescanPlugin>(instance: &OnceLock<Result<P, PluginError>>, reason: u8) {
    let reason = RescanReason::from_abi(reason).unwrap_or(RescanReason::Operator);
    if catch_unwind(AssertUnwindSafe(|| {
        plugin(instance).map(|plugin| plugin.rescan(reason))
    }))
    .is_err()
    {
        tracing::error!("safe plugin rescan callback panicked");
    }
}

/// Serializes one authoritative topology into storage that remains stable until
/// the next serialized topology call.
///
/// # Safety
///
/// When non-null, `length` must point to a writable `usize` for the duration
/// of this call. Calls using the same `cache` must not overlap, and the caller
/// must finish reading the returned bytes before calling this function again:
/// the next call may replace the allocation after releasing the cache lock.
#[doc(hidden)]
pub unsafe fn topology<P: LuminatePlugin>(
    instance: &OnceLock<Result<P, PluginError>>,
    cache: &OnceLock<Mutex<Vec<u8>>>,
    length: *mut usize,
) -> *const u8 {
    let devices = catch_unwind(AssertUnwindSafe(|| plugin(instance)?.topology()))
        .unwrap_or_else(|_| Err(panic_diagnostic()))
        .unwrap_or_else(|error| {
            tracing::error!(error = %error, "safe plugin failed to provide topology");
            Vec::new()
        });
    let cache = cache.get_or_init(|| Mutex::new(crate::topology_cbor(&[])));
    // The lock must stay inside `catch_unwind`: a poisoned-lock panic here
    // would otherwise unwind straight through the `extern "C"` topology
    // callback that calls this function.
    #[allow(
        clippy::expect_used,
        reason = "a poisoned topology-cache lock means another thread already \
                  panicked mid-update; continuing risks handing the host a \
                  half-written CBOR buffer, so propagate instead of recovering"
    )]
    catch_unwind(AssertUnwindSafe(|| {
        let mut cache = cache.lock().expect("plugin topology cache lock poisoned");
        *cache = crate::topology_cbor(&devices);
        if !length.is_null() {
            // SAFETY: the host supplies a writable length slot for this call.
            unsafe { length.write(cache.len()) };
        }
        // The pointer escapes the lock deliberately. PluginTopologyCborFn's
        // serialized-call contract keeps it valid until the next call replaces
        // this allocation.
        cache.as_ptr()
    }))
    .unwrap_or_else(|_| {
        tracing::error!("plugin topology cache update panicked");
        ptr::null()
    })
}

/// Decodes and applies one update through the safe facade.
#[doc(hidden)]
pub unsafe fn apply<P: LuminatePlugin>(
    instance: &OnceLock<Result<P, PluginError>>,
    context: PluginRequestContext,
    update_cbor: *const u8,
    update_len: usize,
    result: *mut PluginApplyResult,
) -> u8 {
    catch_unwind(AssertUnwindSafe(|| {
        crate::with_request_context(context, || {
            // SAFETY: forwarded from the descriptor callback contract; the
            // shared decoder validates null pointers and the CBOR envelope.
            unsafe {
                dispatch_update_cbor(update_cbor, update_len, result, |update| {
                    apply_one(instance, context, update)
                })
            }
        })
    }))
    .unwrap_or_else(|_| {
        // SAFETY: the shared writer handles a null result pointer.
        unsafe { crate::write_apply_result(result, panic_diagnostic().into_apply_result()) }
    })
}

/// Decodes and applies one streamed frame through the safe facade.
#[doc(hidden)]
pub unsafe fn upload_frame<P: FrameStreamingPlugin>(
    instance: &OnceLock<Result<P, PluginError>>,
    context: PluginRequestContext,
    frame_upload_cbor: *const u8,
    frame_upload_len: usize,
    result: *mut PluginApplyResult,
) -> u8 {
    catch_unwind(AssertUnwindSafe(|| {
        crate::with_request_context(context, || {
            // SAFETY: forwarded from the descriptor callback contract; the
            // shared decoder validates null pointers and the CBOR envelope.
            unsafe {
                dispatch_frame_upload_cbor(
                    frame_upload_cbor,
                    frame_upload_len,
                    result,
                    |frame_upload| {
                        outcome_to_result(plugin(instance).and_then(|plugin| {
                            plugin.upload_frame(
                                &context,
                                &frame_upload.target,
                                &frame_upload.envelope,
                            )
                        }))
                    },
                )
            }
        })
    }))
    .unwrap_or_else(|_| {
        // SAFETY: the shared writer handles a null result pointer.
        unsafe { crate::write_apply_result(result, panic_diagnostic().into_apply_result()) }
    })
}

/// Decodes the target, begins a shared-memory stream through the safe
/// facade, and mints the opaque handle later `shm_frame`/`shm_stream_end`
/// calls echo back.
#[doc(hidden)]
#[allow(
    clippy::too_many_arguments,
    reason = "mirrors the raw ABI signature it adapts (PluginShmStreamBeginFn), which the plugin ABI's compatibility contract fixes"
)]
pub unsafe fn shm_stream_begin<P: ShmFrameStreamingPlugin>(
    instance: &OnceLock<Result<P, PluginError>>,
    context: PluginRequestContext,
    target_cbor: *const u8,
    target_len: usize,
    pixel_format: u32,
    pixel_count: u32,
    generation: u32,
    handle_out: *mut u64,
    result: *mut PluginApplyResult,
) -> u8 {
    catch_unwind(AssertUnwindSafe(|| {
        crate::with_request_context(context, || {
            // SAFETY: forwarded from the descriptor callback contract; the
            // shared decoder validates null pointers and the CBOR envelope.
            unsafe {
                dispatch_shm_stream_begin(
                    target_cbor,
                    target_len,
                    pixel_format,
                    pixel_count,
                    generation,
                    handle_out,
                    result,
                    |target, format, pixel_count, generation| {
                        let stream = plugin(instance)
                            .and_then(|plugin| {
                                plugin.shm_stream_begin(
                                    &context,
                                    target,
                                    format,
                                    pixel_count,
                                    generation,
                                )
                            })
                            .map_err(|error| Box::new(error.into_apply_result()))?;
                        // A boxed pointer is deliberately the whole handle:
                        // both ends of this cast are the same trust domain
                        // (this plugin's own generated glue), unlike the
                        // plugin.so -> daemon direction, which is the one
                        // that has to defend against adversarial input.
                        let raw = Box::into_raw(Box::new(stream));
                        Ok(raw as usize as u64)
                    },
                )
            }
        })
    }))
    .unwrap_or_else(|_| {
        // SAFETY: the shared writer handles a null result pointer.
        unsafe { crate::write_apply_result(result, panic_diagnostic().into_apply_result()) }
    })
}

/// Applies one shared-memory frame sample through the safe facade.
///
/// # Safety
///
/// `handle` must be a value this plugin's [`shm_stream_begin`] returned, not
/// yet passed to [`shm_stream_end`], and not currently in use by any other
/// concurrent call; the host's serialized-per-plugin shared-memory call
/// ordering (see [`crate::PluginShmFrameApplyFn`]) is what makes the
/// resulting mutable reborrow sound.
#[doc(hidden)]
pub unsafe fn shm_frame<P: ShmFrameStreamingPlugin>(
    instance: &OnceLock<Result<P, PluginError>>,
    context: PluginRequestContext,
    handle: u64,
    header: ShmFrameHeader,
    pixels: *const u8,
    pixels_len: usize,
    result: *mut PluginApplyResult,
) -> u8 {
    if handle == 0 {
        tracing::warn!("received an shm frame for handle 0 (no active stream)");
        // SAFETY: the shared writer handles a null result pointer.
        return unsafe {
            crate::write_apply_result(
                result,
                PluginApplyResult::invalid_argument(
                    "no active shared-memory stream for this handle",
                ),
            )
        };
    }

    catch_unwind(AssertUnwindSafe(|| {
        crate::with_request_context(context, || {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "handle was minted by this same process's own Box::into_raw in shm_stream_begin and round-tripped verbatim through u64 storage, never crossing a narrower pointer width"
            )]
            // SAFETY: upheld by this function's own safety contract.
            let stream = unsafe { &mut *(handle as usize as *mut P::Stream) };
            // SAFETY: forwarded from the descriptor callback contract; the
            // shared adapter validates the pixel buffer's null/length pairing.
            unsafe {
                dispatch_shm_frame(header, pixels, pixels_len, result, |header, pixels| {
                    outcome_to_result(
                        plugin(instance)
                            .and_then(|plugin| plugin.shm_frame(&context, stream, header, pixels)),
                    )
                })
            }
        })
    }))
    .unwrap_or_else(|_| {
        // SAFETY: the shared writer handles a null result pointer.
        unsafe { crate::write_apply_result(result, panic_diagnostic().into_apply_result()) }
    })
}

/// Ends a shared-memory stream through the safe facade, reclaiming the
/// boxed state [`shm_stream_begin`] allocated for `handle`.
///
/// # Safety
///
/// `handle` must be a value this plugin's [`shm_stream_begin`] returned and
/// must not be used again (by `shm_frame` or a second `shm_stream_end`)
/// after this call, since it frees the boxed stream state.
#[doc(hidden)]
pub unsafe fn shm_stream_end<P: ShmFrameStreamingPlugin>(
    instance: &OnceLock<Result<P, PluginError>>,
    context: PluginRequestContext,
    handle: u64,
    generation: u32,
) {
    if handle == 0 {
        return;
    }

    let panicked = catch_unwind(AssertUnwindSafe(|| {
        crate::with_request_context(context, || {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "handle was minted by this same process's own Box::into_raw in shm_stream_begin and round-tripped verbatim through u64 storage, never crossing a narrower pointer width"
            )]
            // SAFETY: the handle was minted by `Box::into_raw` in
            // `shm_stream_begin` and is reclaimed here exactly once.
            let stream = unsafe { Box::from_raw(handle as usize as *mut P::Stream) };
            if let Ok(plugin) = plugin(instance) {
                plugin.shm_stream_end(*stream, generation);
            }
        });
    }))
    .is_err();

    if panicked {
        tracing::error!("safe plugin shm_stream_end callback panicked");
    }
}

/// Applies a batch in order through a plugin's ordinary typed update method.
#[doc(hidden)]
pub unsafe fn apply_batch<P: LuminatePlugin>(
    instance: &OnceLock<Result<P, PluginError>>,
    context: PluginRequestContext,
    batch_cbor: *const u8,
    batch_len: usize,
    results: *mut PluginApplyResult,
    results_len: usize,
) -> u8 {
    catch_unwind(AssertUnwindSafe(|| {
        crate::with_request_context(context, || {
            // SAFETY: forwarded from the descriptor callback contract; the
            // shared decoder validates the buffer and batch length.
            unsafe {
                dispatch_update_batch_cbor(batch_cbor, batch_len, results, results_len, |update| {
                    apply_one(instance, context, update)
                })
            }
        })
    }))
    .unwrap_or(0)
}

/// Decodes and applies a batch through a plugin's native coalescing method.
#[doc(hidden)]
pub unsafe fn apply_native_batch<P: BatchPlugin>(
    instance: &OnceLock<Result<P, PluginError>>,
    context: PluginRequestContext,
    batch_cbor: *const u8,
    batch_len: usize,
    results: *mut PluginApplyResult,
    results_len: usize,
) -> u8 {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: forwarded from the descriptor callback contract.
        let Some(batch) =
            (unsafe { decode_update_batch(batch_cbor, batch_len, results, results_len) })
        else {
            return 0;
        };

        let outcomes = crate::with_request_context(context, || {
            plugin(instance).map(|plugin| plugin.apply_batch(&context, &batch.updates))
        });
        let outcomes = match outcomes {
            Ok(outcomes) if outcomes.len() == results_len => outcomes,
            Ok(outcomes) => {
                tracing::error!(
                    expected = results_len,
                    actual = outcomes.len(),
                    "safe plugin returned the wrong number of batch results"
                );
                return 0;
            }
            Err(error) => vec![Err(error); results_len],
        };
        // SAFETY: the caller guarantees this buffer and its length matches the
        // decoded update count.
        let slots = unsafe { slice::from_raw_parts_mut(results, results_len) };
        for (slot, outcome) in slots.iter_mut().zip(outcomes) {
            *slot = outcome_to_result(outcome);
        }
        1
    }))
    .unwrap_or(0)
}

/// Decodes a bounded read request and writes one typed snapshot.
#[doc(hidden)]
pub unsafe fn read_state<P: ReadablePlugin>(
    instance: &OnceLock<Result<P, PluginError>>,
    context: PluginRequestContext,
    request_cbor: *const u8,
    request_len: usize,
    output: *mut u8,
    output_capacity: usize,
) -> usize {
    catch_unwind(AssertUnwindSafe(|| {
        if request_cbor.is_null() {
            tracing::warn!("received null state-read request");
            return usize::MAX;
        }
        // SAFETY: upheld by the descriptor callback contract; null was checked.
        let payload = unsafe { slice::from_raw_parts(request_cbor, request_len) };
        let request: PluginReadRequest = match ciborium::from_reader(payload) {
            Ok(request) => request,
            Err(error) => {
                tracing::warn!(error = %error, "failed to parse state-read request");
                return usize::MAX;
            }
        };
        let snapshot = crate::with_request_context(context, || {
            let plugin = plugin(instance)?;
            plugin.read_state(&context, &request)
        })
        .unwrap_or_else(|error| PluginStateSnapshot {
            observations: Vec::new(),
            errors: request
                .targets
                .iter()
                .map(|target| PluginReadError {
                    target: target.target.clone(),
                    diagnostic: error.to_string(),
                })
                .collect(),
        });
        // SAFETY: forwarded from the descriptor callback contract; the writer
        // copies only when the supplied capacity is sufficient.
        unsafe { crate::write_state_snapshot(output, output_capacity, &snapshot) }
    }))
    .unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests;
