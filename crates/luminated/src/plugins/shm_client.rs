// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Daemon-side registry for the client → daemon leg of the zero-copy
//! shared-memory frame fast path.
//!
//! This is the role-reversed counterpart to
//! `crate::plugins::shm::ShmPublisherRegistry`. There, the daemon
//! publishes and the plugin host subscribes; here, a same-UID client
//! publishes and the daemon subscribes. The daemon still creates (rather
//! than merely opening) both iceoryx2 services, since it remains the
//! resource owner and the endpoint responsible for capability
//! negotiation. Clients only ever attach to names the daemon has already
//! negotiated and handed out.
//!
//! Each active stream owns a dedicated OS thread, which in turn owns that
//! stream's `Subscriber` and `Listener`. This deliberately follows
//! `crate::plugin_host::shm::ShmRuntime`'s established per-stream-thread
//! design and inherits its soundness argument; see that module's
//! documentation for why a shared `WaitSet` is unsuitable here.
//!
//! Every sample received here is treated as hostile input. Even a same-UID
//! client may be buggy, compromised, or crash mid-write. Every sample is
//! fully validated (`ShmClientFrameHeader::parse`, followed by generation
//! and nonce checks) before it reaches
//! [`crate::plugins::PluginManager::apply_one_frame`]. Invalid samples are
//! dropped and logged (subject to rate limiting); they never panic the
//! stream thread.

#![allow(
    unsafe_code,
    reason = "none of this module uses unsafe directly, but it links iceoryx2, which the workspace lint config treats as unsafe-adjacent FFI"
)]

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use iceoryx2::port::listener::Listener;
use iceoryx2::port::subscriber::Subscriber;
use iceoryx2::prelude::*;
use tokio::sync::Mutex as AsyncMutex;

use luminate_core::capability::ShmFrameCapability;
use luminate_core::frame::{FrameEnvelope, FramePayload};
use luminate_core::shm_frame::{SHM_CLIENT_FRAME_HEADER_LEN, ShmClientFrameHeader, ShmPixelFormat};
use luminate_core::target::TargetId;

use crate::state::DaemonState;

use super::PluginManager;

/// How often a stream thread wakes up even without a notification, purely
/// to check whether it has been asked to stop. Mirrors
/// `plugin_host::shm::STOP_POLL_INTERVAL`.
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Minimum spacing between consecutive "rejected a malformed/stale frame"
/// log lines for one stream. A buggy or malicious same-uid client
/// publishing garbage at frame rate must not be able to use logging itself
/// as a denial-of-service vector.
const REJECTED_FRAME_LOG_INTERVAL: Duration = Duration::from_secs(1);

/// Source for [`ClientShmStreamReady::stream_nonce`].
///
/// A process-local monotonic counter is sufficient: the nonce only
/// distinguishes successive negotiations for the same target. It does not
/// need to be unpredictable because the transport is already restricted to
/// the daemon's UID.
///
/// Starts at 1 so that zero can be reserved as an unmistakably invalid
/// nonce.
static NEXT_STREAM_NONCE: AtomicU64 = AtomicU64::new(1);

struct ActiveClientStream {
    generation: u32,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

/// A successfully negotiated client-published stream, ready to hand back to
/// the client as [`luminate_protocol::ResponseStatus::ShmFrameStreamReady`].
pub struct ClientShmStreamReady {
    pub service_name: String,
    pub event_service_name: String,
    pub pixel_format: ShmPixelFormat,
    pub stream_nonce: u64,
    pub segment_bytes: u32,
}

/// Per-daemon registry of active client-published shared-memory streams,
/// keyed by rendered [`TargetId`]. Owned by
/// [`crate::plugins::PluginManager`], alongside its daemon ↔ plugin-host
/// counterpart.
#[derive(Default)]
pub(super) struct ShmClientSubscriberRegistry {
    streams: Mutex<HashMap<String, ActiveClientStream>>,

    // Fields drop in declaration order. Active stream threads must release
    // their ports before iceoryx2 removes the node directory.
    node: OnceLock<Node<ipc_threadsafe::Service>>,
}

impl ShmClientSubscriberRegistry {
    fn node(&self) -> Result<&Node<ipc_threadsafe::Service>> {
        if let Some(node) = self.node.get() {
            return Ok(node);
        }
        let node = luminate_host_supervisor::create_node()?;
        Ok(self.node.get_or_init(|| node))
    }

    /// Negotiates and begins a client-published stream for `target`.
    ///
    /// The caller is responsible for performing the capability and policy
    /// checks documented on `Request::BeginShmFrameStream` (same-UID gate,
    /// `prefer_client_shm`, target capability, and ensuring the target is not
    /// already streaming). This method assumes those checks have already
    /// succeeded; it only creates the iceoryx2 services and spawns the stream
    /// thread.
    ///
    /// The spawned thread retains `state` and `plugin_manager` for the
    /// lifetime of the stream, invoking
    /// [`crate::plugins::PluginManager::apply_one_frame`] for every valid
    /// sample it receives.
    ///
    /// # Errors
    ///
    /// Returns an error only if the daemon fails to create its iceoryx2 node,
    /// services, or the stream thread itself. Client-controlled failures are
    /// never reported here, as they have already been filtered out by the
    /// caller.
    pub(super) fn begin(
        &self,
        target: &TargetId,
        capability: &ShmFrameCapability,
        generation: u32,
        plugin_manager: Arc<PluginManager>,
        state: Arc<AsyncMutex<DaemonState>>,
    ) -> Result<Option<ClientShmStreamReady>> {
        let Some(format) = capability.pixel_formats.first().copied() else {
            tracing::warn!("client shared-memory frame capability advertises no pixel formats");
            return Ok(None);
        };
        let Some(pixel_count) = capability.shape.pixel_count() else {
            tracing::warn!("client shared-memory frame pixel count overflowed");
            return Ok(None);
        };
        let Some(segment_bytes) = segment_bytes(format, pixel_count) else {
            tracing::warn!("client shared-memory frame segment size overflowed");
            return Ok(None);
        };

        let key = stream_key(target);
        let names = luminate_host_supervisor::client_service_names(&key)
            .context("deriving client shared-memory service names")?;
        let node = self.node()?;

        let pubsub = node
            .service_builder(&names.publish_subscribe)
            .publish_subscribe::<[u8]>()
            .max_publishers(1)
            .max_subscribers(1)
            .subscriber_max_buffer_size(1)
            .history_size(0)
            .enable_safe_overflow(true)
            .open_or_create()
            .context("creating client shared-memory segment")?;
        let subscriber = pubsub
            .subscriber_builder()
            .create()
            .context("creating client shared-memory subscriber")?;

        let event = node
            .service_builder(&names.event)
            .event()
            .max_notifiers(1)
            .max_listeners(1)
            .open_or_create()
            .context("creating client shared-memory event channel")?;
        let listener = event
            .listener_builder()
            .create()
            .context("creating client shared-memory listener")?;

        let stream_nonce = NEXT_STREAM_NONCE.fetch_add(1, Ordering::Relaxed);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_target = target.clone();
        let thread = thread::Builder::new()
            .name("luminate-shm-client-stream".to_owned())
            .spawn(move || {
                let context = StreamContext {
                    target: &thread_target,
                    generation,
                    stream_nonce,
                    plugin_manager: &plugin_manager,
                    state: &state,
                };
                stream_loop(&subscriber, &listener, &context, &thread_stop);
            })
            .context("spawning client shared-memory stream thread")?;

        #[allow(
            clippy::expect_used,
            clippy::unwrap_in_result,
            reason = "Acceptable to panic on poisoned locks"
        )]
        self.streams
            .lock()
            .expect("client shared-memory stream registry lock poisoned")
            .insert(
                key,
                ActiveClientStream {
                    generation,
                    stop,
                    thread,
                },
            );

        tracing::info!(
            ?target,
            generation,
            segment_bytes,
            "client-published shared-memory frame stream active"
        );

        Ok(Some(ClientShmStreamReady {
            service_name: names.publish_subscribe.to_string(),
            event_service_name: names.event.to_string(),
            pixel_format: format,
            stream_nonce,
            segment_bytes,
        }))
    }

    /// Ends the client-published stream for `target` if `generation` matches
    /// the currently active stream.
    ///
    /// Like `DaemonState::end_frame_stream`, this operation is idempotent and
    /// does nothing if no matching stream exists.
    ///
    /// Returns `true` if a stream was actually torn down, so the caller can
    /// emit the appropriate lifecycle event.
    pub(super) fn end(&self, target: &TargetId, generation: u32) -> bool {
        let key = stream_key(target);
        #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
        let removed = {
            let mut streams = self
                .streams
                .lock()
                .expect("client shared-memory stream registry lock poisoned");
            let current_matches = streams
                .get(&key)
                .is_some_and(|stream| stream.generation == generation);
            if current_matches {
                streams.remove(&key)
            } else {
                None
            }
        };
        if let Some(stream) = removed {
            stop_and_join(stream);
            true
        } else {
            false
        }
    }

    /// Force-ends the client-published stream for `target`, regardless of
    /// generation.
    ///
    /// Used during connection-drop cleanup, mirroring
    /// `ShmPublisherRegistry::force_end`'s unconditional-by-target semantics.
    ///
    /// Returns the generation of the stream that was torn down, or `None` if
    /// no stream was active. The caller uses this to emit the corresponding
    /// `ShmStreamEnded` lifecycle event.
    pub(super) fn force_end(&self, target: &TargetId) -> Option<u32> {
        let key = stream_key(target);
        #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
        let removed = self
            .streams
            .lock()
            .expect("client shared-memory stream registry lock poisoned")
            .remove(&key);
        removed.map(|stream| {
            let generation = stream.generation;
            stop_and_join(stream);
            generation
        })
    }
}

/// Stops a stream's dedicated thread and joins it.
///
/// No plugin-facing teardown is required, unlike
/// `plugin_host::shm::teardown`.
fn stop_and_join(stream: ActiveClientStream) {
    stream.stop.store(true, Ordering::Release);
    if stream.thread.join().is_err() {
        tracing::warn!("client shared-memory stream thread panicked");
    }
}

/// Renders `target` into the canonical string representation for the
/// client → daemon shared-memory path.
///
/// The resulting string is used both by
/// [`luminate_host_supervisor::client_service_names`] to derive iceoryx2
/// service names and as the key in the in-memory registry.
///
/// This intentionally reuses
/// [`super::plugin_target_from_target_id`] so the client and
/// daemon ↔ plugin-host paths render targets consistently. The client-side
/// namespace keeps the two legs' derived service names distinct.
fn stream_key(target: &TargetId) -> String {
    super::plugin_target_from_target_id(target).to_string()
}

/// Total bytes one client-published sample needs: the fixed
/// [`SHM_CLIENT_FRAME_HEADER_LEN`]-byte header plus the packed pixel
/// buffer. `None` if that would overflow `u32`.
fn segment_bytes(format: ShmPixelFormat, pixel_count: u32) -> Option<u32> {
    let per_pixel = u32::try_from(format.bytes_per_pixel()).ok()?;
    let payload = pixel_count.checked_mul(per_pixel)?;
    let header = u32::try_from(SHM_CLIENT_FRAME_HEADER_LEN).ok()?;
    header.checked_add(payload)
}

/// Borrowed state shared by every iteration of [`stream_loop`].
#[derive(Clone, Copy)]
struct StreamContext<'a> {
    target: &'a TargetId,
    generation: u32,
    stream_nonce: u64,

    plugin_manager: &'a Arc<PluginManager>,
    state: &'a Arc<AsyncMutex<DaemonState>>,
}

/// Runs one client-published stream's dedicated thread: waits for
/// notifications (or the poll interval, to re-check `stop`), drains the
/// subscriber to the latest sample, validates it, then forwards valid
/// frames through the ordinary frame-application path.
fn stream_loop(
    subscriber: &Subscriber<ipc_threadsafe::Service, [u8], ()>,
    listener: &Listener<ipc_threadsafe::Service>,
    context: &StreamContext<'_>,
    stop: &AtomicBool,
) {
    let StreamContext {
        target,
        generation,
        stream_nonce,
        plugin_manager,
        state,
    } = *context;
    let mut last_rejected_log = Instant::now()
        .checked_sub(REJECTED_FRAME_LOG_INTERVAL)
        .unwrap_or_else(Instant::now);

    while !stop.load(Ordering::Acquire) {
        if let Err(error) = listener.timed_wait(|_event_id| {}, STOP_POLL_INTERVAL) {
            tracing::warn!(%error, "client shared-memory stream listener wait failed");
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
                    tracing::warn!(%error, "client shared-memory stream subscriber receive failed");
                    break;
                }
            }
        }
        let Some(sample) = latest else {
            continue;
        };

        let sample_bytes: &[u8] = sample.payload();
        let pixel_bytes = sample_bytes
            .get(SHM_CLIENT_FRAME_HEADER_LEN..)
            .unwrap_or(&[]);
        let (header, format) = match ShmClientFrameHeader::parse(sample_bytes, pixel_bytes) {
            Ok(parsed) => parsed,
            Err(error) => {
                log_rejected_frame(&mut last_rejected_log, target, &error);
                continue;
            }
        };

        if header.generation != generation {
            log_rejected_frame(
                &mut last_rejected_log,
                target,
                &"stale generation on a client-published sample",
            );
            continue;
        }
        if header.stream_nonce != stream_nonce {
            log_rejected_frame(
                &mut last_rejected_log,
                target,
                &"stream nonce mismatch on a client-published sample",
            );
            continue;
        }

        let per_pixel = format.bytes_per_pixel();
        let colours = pixel_bytes
            .chunks(per_pixel)
            .map(|chunk| format.unpack(chunk))
            .collect();

        let envelope = FrameEnvelope {
            generation: header.generation,
            sequence: header.sequence,
            payload: FramePayload::Full(colours),
            commit: header.commit(),
        };

        if let Err(error) = plugin_manager.apply_one_frame(state, target, &envelope) {
            tracing::warn!(%error, "applying a client-published shared-memory frame failed");
        }
    }
}

/// Logs a rejected client-published sample, rate-limited to at most once per
/// [`REJECTED_FRAME_LOG_INTERVAL`] for this stream (see that constant's docs
/// for why).
fn log_rejected_frame(last_logged: &mut Instant, target: &TargetId, reason: &dyn fmt::Display) {
    let now = Instant::now();
    if now.duration_since(*last_logged) >= REJECTED_FRAME_LOG_INTERVAL {
        tracing::warn!(
            ?target,
            %reason,
            "rejected a client-published shared-memory frame"
        );
        *last_logged = now;
    }
}

#[cfg(test)]
#[path = "shm_client_tests.rs"]
mod tests;
