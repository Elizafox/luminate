// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Child-side (plugin-host process) shared-memory frame stream management.
//!
//! One dedicated OS thread per active stream owns that stream's iceoryx2
//! `Subscriber` and `Listener` outright and calls `Listener::timed_wait_all`
//! directly to wake up, drain the latest published sample, and dispatch it
//! into the plugin's `shm_frame_apply` callback.
//!
//! This replaced an earlier design where every stream on one plugin-host
//! shared a single `WaitSet`: `WaitSet::attach_notification` returns a
//! `WaitSetGuard` that borrows both the `WaitSet` and the `Listener` it
//! attaches, so storing a dynamically growing/shrinking set of owned
//! `Listener`s alongside guards borrowing them is a self-referential struct
//! safe Rust can't express. Giving each stream sole ownership of its own
//! thread and ports sidesteps the problem entirely instead of working
//! around it with `unsafe`, and is simpler besides: one process rarely
//! juggles more than a handful of concurrent streams, so there is little to
//! be gained from multiplexing them behind one wait primitive.

#![allow(
    unsafe_code,
    reason = "dispatches into native plugin ABI function pointers from stream threads"
)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context as _, Result};
use iceoryx2::port::listener::Listener;
use iceoryx2::port::subscriber::Subscriber;
use iceoryx2::prelude::*;

use luminate_core::shm_frame::{ShmFrameHeader, ShmPixelFormat};
use luminate_plugin_api::{
    PluginApplyResult, PluginRequestContext, PluginShmFrameApplyFn, PluginShmStreamBeginFn,
    PluginShmStreamEndFn, PluginTarget, abi_bool,
};

use super::protocol::{ApplyOutcome, BeginShmStreamRequest, ShmStreamOutcome};
use super::runtime::{decode_apply_result, encode_cbor};

/// How often a stream thread wakes up even without a notification, purely
/// to check whether it has been asked to stop. Bounds `EndShmStream`
/// latency to roughly this long; short enough to feel immediate to a
/// client, long enough that idle streams cost nothing worth measuring.
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Work budget for each `shm_frame_apply` call.
///
/// Unlike ordinary command callbacks, shared-memory frame delivery has no
/// daemon request whose timeout it can inherit: it is driven by an
/// iceoryx2 notification rather than a pipe request.
///
/// This call is on the frame-delivery hot path, so the budget is
/// intentionally short. A plugin that cannot apply a single frame within
/// this budget is already failing the high-framerate contract that this
/// path exists to support.
const SHM_FRAME_APPLY_BUDGET: Duration = Duration::from_millis(250);

/// Callbacks a plugin exposes for shared-memory streaming, bundled together
/// because the load-time contract requires all three or none (see
/// `validation.rs`).
#[derive(Clone, Copy)]
pub(super) struct ShmCallbacks {
    pub(super) begin: PluginShmStreamBeginFn,
    pub(super) apply: PluginShmFrameApplyFn,
    pub(super) end: PluginShmStreamEndFn,
}

struct ActiveStream {
    generation: u32,
    handle: u64,
    end: PluginShmStreamEndFn,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

/// Per-plugin shared-memory stream state: a lazily created iceoryx2 `Node`
/// and the set of currently active streams, keyed by rendered
/// [`PluginTarget`]. `Begin`/`End`/`Reset` all run on the plugin-host's
/// single command-processing thread (see `runtime::run`'s request loop), so
/// a `RefCell` is enough here; the concurrency this module actually manages
/// is each active stream's own dedicated thread, not concurrent access to
/// this registry.
#[derive(Default)]
pub(super) struct ShmRuntime {
    streams: RefCell<HashMap<String, ActiveStream>>,

    // Fields drop in declaration order. Active stream threads must release
    // their ports before iceoryx2 removes the node directory.
    node: OnceLock<Node<ipc_threadsafe::Service>>,
}

impl ShmRuntime {
    fn node(&self) -> Result<&Node<ipc_threadsafe::Service>> {
        if let Some(node) = self.node.get() {
            return Ok(node);
        }
        let node = luminate_host_supervisor::create_node()?;
        Ok(self.node.get_or_init(|| node))
    }

    /// Negotiates and, on acceptance, begins a shared-memory stream.
    ///
    /// Every failure path returns `Ok` with a descriptive
    /// [`ShmStreamOutcome`] rather than `Err`: a plugin declining, or this
    /// host failing to stand up the shared-memory segment, are ordinary,
    /// expected outcomes the daemon falls back to the pipe for, not
    /// exceptional plugin-host failures.
    pub(super) fn begin(
        &self,
        plugin_name: &str,
        request: &BeginShmStreamRequest,
        callbacks: ShmCallbacks,
        context: PluginRequestContext,
    ) -> Result<ShmStreamOutcome> {
        let key = request.target.to_string();
        if self.streams.borrow().contains_key(&key) {
            return Ok(ShmStreamOutcome::Internal(
                "a shared-memory stream is already active for this target".to_owned(),
            ));
        }

        let Some(format) = ShmPixelFormat::from_abi(request.pixel_format) else {
            return Ok(ShmStreamOutcome::Unsupported(format!(
                "unrecognized pixel format {}",
                request.pixel_format
            )));
        };
        let Some(pixel_count) = request.shape.pixel_count() else {
            return Ok(ShmStreamOutcome::Internal(
                "frame pixel count overflowed".to_owned(),
            ));
        };
        let Some(segment_bytes) = segment_bytes(format, pixel_count) else {
            return Ok(ShmStreamOutcome::Internal(
                "frame segment size overflowed".to_owned(),
            ));
        };

        let names = match luminate_host_supervisor::service_names(plugin_name, &key) {
            Ok(names) => names,
            Err(error) => {
                return Ok(ShmStreamOutcome::Internal(format!(
                    "deriving shared-memory service names: {error}"
                )));
            }
        };
        let node = match self.node() {
            Ok(node) => node,
            Err(error) => return Ok(ShmStreamOutcome::Io(format!("{error:#}"))),
        };
        let (subscriber, listener) = match open_stream_ports(node, &names) {
            Ok(ports) => ports,
            Err(outcome) => return Ok(outcome),
        };

        let handle = match negotiate_begin(
            callbacks.begin,
            context,
            &request.target,
            format,
            pixel_count,
            request.generation,
        )? {
            NegotiateOutcome::Accepted(handle) => handle,
            NegotiateOutcome::Declined(outcome) => return Ok(outcome),
        };

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let apply = callbacks.apply;
        let thread = thread::Builder::new()
            .name("luminate-shm-stream".to_owned())
            .spawn(move || stream_loop(&subscriber, &listener, apply, handle, &thread_stop))
            .context("spawning shared-memory stream thread")?;

        self.streams.borrow_mut().insert(
            key,
            ActiveStream {
                generation: request.generation,
                handle,
                end: callbacks.end,
                stop,
                thread,
            },
        );

        Ok(ShmStreamOutcome::Ready { segment_bytes })
    }

    /// Ends the stream on `target`, if `generation` still matches the
    /// currently active one. Idempotent and generation-gated for the same
    /// reason `DaemonState::end_frame_stream` is: disconnect cleanup can
    /// race an explicit `EndShmStream`, and a stale end for an
    /// already-superseded stream must not tear down its replacement.
    pub(super) fn end(
        &self,
        target: &PluginTarget,
        generation: u32,
        context: PluginRequestContext,
    ) -> ShmStreamOutcome {
        let key = target.to_string();
        let current_matches = self
            .streams
            .borrow()
            .get(&key)
            .is_some_and(|stream| stream.generation == generation);
        if current_matches && let Some(stream) = self.streams.borrow_mut().remove(&key) {
            teardown(stream, context, generation);
        }
        ShmStreamOutcome::Acknowledged
    }

    /// Updates the locally remembered generation for the stream on
    /// `target`, without touching its iceoryx2 services or thread. Mirrors
    /// how the ordinary frame-upload path already treats a `generation`
    /// bump as a lightweight "reset your buffering state" signal rather
    /// than a full disconnect.
    pub(super) fn reset(&self, target: &PluginTarget, generation: u32) -> ShmStreamOutcome {
        if let Some(stream) = self.streams.borrow_mut().get_mut(&target.to_string()) {
            stream.generation = generation;
        }
        ShmStreamOutcome::Acknowledged
    }

    /// Tears down every active stream. Called when the plugin-host is
    /// shutting down, so the plugin gets a graceful `shm_stream_end` for
    /// each one rather than losing them silently to process exit.
    pub(super) fn end_all(&self, context: PluginRequestContext) {
        let keys: Vec<String> = self.streams.borrow().keys().cloned().collect();
        for key in keys {
            if let Some(stream) = self.streams.borrow_mut().remove(&key) {
                let generation = stream.generation;
                teardown(stream, context, generation);
            }
        }
    }
}

/// Opens (or creates) the publish-subscribe and event services for one
/// stream and creates this side's ports on them. Split out of
/// [`ShmRuntime::begin`] purely to keep that function short; every failure
/// here is an ordinary `ShmStreamOutcome` the caller returns as-is.
type StreamPorts = (
    Subscriber<ipc_threadsafe::Service, [u8], ()>,
    Listener<ipc_threadsafe::Service>,
);

fn open_stream_ports(
    node: &Node<ipc_threadsafe::Service>,
    names: &luminate_host_supervisor::ShmServiceNames,
) -> Result<StreamPorts, ShmStreamOutcome> {
    let pubsub = node
        .service_builder(&names.publish_subscribe)
        .publish_subscribe::<[u8]>()
        .max_publishers(1)
        .max_subscribers(1)
        .subscriber_max_buffer_size(1)
        .history_size(0)
        .enable_safe_overflow(true)
        .open_or_create()
        .map_err(|error| ShmStreamOutcome::Io(format!("opening shared-memory segment: {error}")))?;
    let subscriber = pubsub.subscriber_builder().create().map_err(|error| {
        ShmStreamOutcome::Io(format!("creating shared-memory subscriber: {error}"))
    })?;

    let event = node
        .service_builder(&names.event)
        .event()
        .max_notifiers(1)
        .max_listeners(1)
        .open_or_create()
        .map_err(|error| {
            ShmStreamOutcome::Io(format!("opening shared-memory event channel: {error}"))
        })?;
    let listener = event.listener_builder().create().map_err(|error| {
        ShmStreamOutcome::Io(format!("creating shared-memory listener: {error}"))
    })?;

    Ok((subscriber, listener))
}

/// Outcome of asking the plugin to accept a shared-memory stream.
enum NegotiateOutcome {
    /// The plugin accepted; carries its minted stream handle.
    Accepted(u64),

    /// The plugin declined, or the plugin-host itself couldn't process the
    /// request. Caller returns outcome as-is.
    Declined(ShmStreamOutcome),
}

/// Calls the plugin's `shm_stream_begin` and interprets the result. Split
/// out of [`ShmRuntime::begin`] purely to keep that function short.
///
/// # Errors
///
/// Only for a plugin-host bug (a malformed [`PluginApplyResult`]). An
/// ordinary decline is [`NegotiateOutcome::Declined`], not an `Err`.
#[allow(
    clippy::too_many_arguments,
    reason = "mirrors PluginShmStreamBeginFn's raw ABI signature, which the plugin ABI's compatibility contract fixes"
)]
fn negotiate_begin(
    begin_fn: PluginShmStreamBeginFn,
    context: PluginRequestContext,
    target: &PluginTarget,
    format: ShmPixelFormat,
    pixel_count: u32,
    generation: u32,
) -> Result<NegotiateOutcome> {
    let target_cbor = encode_cbor(target)?;

    let mut handle = 0_u64;
    let mut result = PluginApplyResult::internal("plugin did not write a shm-stream-begin result");
    // SAFETY: `begin_fn` belongs to the pinned plugin image; `target_cbor`
    // is a live buffer for the duration of this call; and `handle`/`result`
    // are writable stack locations for the duration of this call.
    let processed = abi_bool(unsafe {
        begin_fn(
            context,
            target_cbor.as_ptr(),
            target_cbor.len(),
            format.to_abi(),
            pixel_count,
            generation,
            &raw mut handle,
            &raw mut result,
        )
    });
    if !processed {
        return Ok(NegotiateOutcome::Declined(ShmStreamOutcome::Internal(
            "plugin could not process the shared-memory stream begin".to_owned(),
        )));
    }
    let outcome = decode_apply_result(&result)?;
    Ok(if matches!(outcome, ApplyOutcome::Applied) {
        NegotiateOutcome::Accepted(handle)
    } else {
        NegotiateOutcome::Declined(apply_outcome_to_stream_outcome(outcome))
    })
}

/// Stops a stream's dedicated thread, joins it, then calls the plugin's
/// `shm_stream_end`. Joining first guarantees `shm_stream_end` never runs
/// concurrently with a `shm_frame_apply` for the same handle.
fn teardown(stream: ActiveStream, context: PluginRequestContext, generation: u32) {
    stream.stop.store(true, Ordering::Release);
    if stream.thread.join().is_err() {
        tracing::warn!("shared-memory stream thread panicked");
    }
    // SAFETY: `stream.end` belongs to the pinned plugin image; the stream
    // thread has just been joined, so `stream.handle` is not being touched
    // by any other thread during this call.
    unsafe {
        (stream.end)(context, stream.handle, generation);
    }
}

/// Runs one shared-memory stream's dedicated subscriber thread.
///
/// Waits for a notification, periodically waking to observe `stop`, then
/// drains all currently available samples and applies only the newest one.
///
/// Dropping older queued samples is intentional. It matches the service's
/// `enable_safe_overflow` policy: frame delivery is latest-wins rather than
/// a lossless queue.
fn stream_loop(
    subscriber: &Subscriber<ipc_threadsafe::Service, [u8], ()>,
    listener: &Listener<ipc_threadsafe::Service>,
    apply: PluginShmFrameApplyFn,
    handle: u64,
    stop: &AtomicBool,
) {
    while !stop.load(Ordering::Acquire) {
        if let Err(error) = listener.timed_wait(|_event_id| {}, STOP_POLL_INTERVAL) {
            tracing::warn!(%error, "shared-memory stream listener wait failed");
            thread::sleep(STOP_POLL_INTERVAL);
            continue;
        }
        if stop.load(Ordering::Acquire) {
            break;
        }

        let mut latest = None;
        loop {
            match subscriber.receive() {
                Ok(Some(sample)) => latest = Some(sample),
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(%error, "shared-memory stream subscriber receive failed");
                    break;
                }
            }
        }
        let Some(sample) = latest else {
            continue;
        };

        let payload: &[u8] = sample.payload();
        let Some(header) = ShmFrameHeader::from_bytes(payload) else {
            tracing::warn!("shared-memory stream sample was too short for its header");
            continue;
        };
        let pixels = payload.get(24..).unwrap_or(&[]);

        let context = PluginRequestContext::new(SHM_FRAME_APPLY_BUDGET);
        let mut result =
            PluginApplyResult::internal("plugin did not write a shm-frame-apply result");
        // SAFETY: `apply` belongs to the pinned plugin image; `header` is
        // `Copy`; `pixels` is a live buffer for the duration of this call
        // (it borrows from `sample`, which outlives this call); and
        // `result` is a writable stack location for the duration of this
        // call.
        let processed = abi_bool(unsafe {
            apply(
                context,
                handle,
                header,
                pixels.as_ptr(),
                pixels.len(),
                &raw mut result,
            )
        });
        if !processed {
            tracing::warn!("plugin could not process a shared-memory frame");
            continue;
        }
        match decode_apply_result(&result) {
            Ok(ApplyOutcome::Applied) => {}
            Ok(outcome) => tracing::warn!(?outcome, "plugin rejected a shared-memory frame"),
            Err(error) => {
                tracing::warn!(%error, "plugin returned a malformed shared-memory apply result");
            }
        }
    }
}

/// Total bytes one sample needs: the fixed 24-byte header plus the packed
/// pixel buffer. `None` if that would overflow `u32`.
fn segment_bytes(format: ShmPixelFormat, pixel_count: u32) -> Option<u32> {
    let per_pixel = u32::try_from(format.bytes_per_pixel()).ok()?;
    let payload = pixel_count.checked_mul(per_pixel)?;
    let header = u32::try_from(size_of::<ShmFrameHeader>()).ok()?;
    header.checked_add(payload)
}

/// Maps a non-`Applied` [`ApplyOutcome`] from `shm_stream_begin` onto the
/// coarser [`ShmStreamOutcome`] the daemon reads to decide whether to fall
/// back to the pipe. Never called with `ApplyOutcome::Applied`, which the
/// caller already special-cases into `ShmStreamOutcome::Ready`.
fn apply_outcome_to_stream_outcome(outcome: ApplyOutcome) -> ShmStreamOutcome {
    match outcome {
        ApplyOutcome::Applied => ShmStreamOutcome::Internal(
            "unreachable: Applied is handled before this conversion".to_owned(),
        ),
        ApplyOutcome::Unsupported(message) | ApplyOutcome::InvalidArgument(message) => {
            ShmStreamOutcome::Unsupported(message)
        }
        ApplyOutcome::Io(message) | ApplyOutcome::Unavailable(message) => {
            ShmStreamOutcome::Io(message)
        }
        ApplyOutcome::RateLimited { diagnostic, .. } | ApplyOutcome::Internal(diagnostic) => {
            ShmStreamOutcome::Internal(diagnostic)
        }
    }
}

#[cfg(test)]
#[path = "shm_tests.rs"]
mod tests;
