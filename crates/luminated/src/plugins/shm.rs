// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Daemon-side shared-memory frame stream registry: the publisher/notifier
//! side of the zero-copy fast path, mirrored by
//! `crate::plugin_host::shm`'s subscriber/listener side.
//!
//! Shared memory is an optimisation, not a requirement. If a stream is not
//! backed by shared memory (for example because the peer lacks the
//! capability, declined it, negotiation failed, or `prefer_shm` is
//! disabled), the ordinary pipe transport is still used.
//!
//! Consequently, operations here are best-effort. They report "couldn't
//! deliver via shared memory" rather than returning errors that every
//! caller would immediately translate into "fall back to the pipe."

#![allow(
    unsafe_code,
    reason = "none of this module uses unsafe directly, but it links iceoryx2, which the workspace lint config treats as unsafe-adjacent FFI"
)]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use anyhow::{Context as _, Result};
use iceoryx2::port::notifier::Notifier;
use iceoryx2::port::publisher::Publisher;
use iceoryx2::prelude::*;

use luminate_core::capability::ShmFrameCapability;
use luminate_core::colour::Colour;
use luminate_core::frame::{FrameEnvelope, FramePayload};
use luminate_core::shm_frame::{SHM_FRAME_HEADER_VERSION, ShmFrameHeader, ShmPixelFormat};
use luminate_plugin_api::PluginTarget;

use crate::plugin_host::{BeginShmStreamRequest, HostedPlugin, ShmStreamOutcome};

struct ActiveShmStream {
    generation: u32,

    /// Pixel format in use for this stream.
    format: ShmPixelFormat,

    /// Publisher for this stream.
    publisher: Publisher<ipc_threadsafe::Service, [u8], ()>,

    /// Notifier for this stream.
    notifier: Notifier<ipc_threadsafe::Service>,

    /// `HostedPlugin::connection_epoch()` when this stream was negotiated.
    ///
    /// If the plugin-host process has since respawned, this publisher and
    /// notifier refer to a shared-memory segment that no longer has a
    /// subscriber. Because `sample.send()` and `notifier.notify()` succeed
    /// even when nobody is listening, this epoch check prevents frames from
    /// being silently dropped while appearing to succeed.
    ///
    /// Shared-memory streams are not transparently renegotiated. An epoch
    /// mismatch makes the stream use ordinary pipe transport for the rest of
    /// its lifetime, like any other shared-memory setup failure.
    connection_epoch: u64,
}

/// Shared-memory publishing state for all loaded plugins: a lazily
/// created iceoryx2 `Node` and the active streams keyed by rendered
/// [`PluginTarget`].
///
/// This registry is owned by [`crate::plugins::PluginManager`]. One
/// instance is sufficient because shared-memory service names are already
/// namespaced by plugin name.
#[derive(Default)]
pub(super) struct ShmPublisherRegistry {
    node: OnceLock<Node<ipc_threadsafe::Service>>,
    streams: Mutex<HashMap<String, ActiveShmStream>>,
}

impl ShmPublisherRegistry {
    fn node(&self) -> Result<&Node<ipc_threadsafe::Service>> {
        if let Some(node) = self.node.get() {
            return Ok(node);
        }
        let node = luminate_host_supervisor::create_node()?;
        Ok(self.node.get_or_init(|| node))
    }

    /// Attempts to negotiate the shared-memory fast path for `target` on
    /// `host`. Returns whether it succeeded; every failure or decline logs
    /// and returns `false` rather than propagating an error, since the
    /// caller's fallback (the ordinary pipe path) is always available and
    /// already what happens by default.
    pub(super) fn begin(
        &self,
        plugin_name: &str,
        host: &HostedPlugin,
        target: &PluginTarget,
        capability: &ShmFrameCapability,
        generation: u32,
    ) -> bool {
        let Some(format) = capability.pixel_formats.first().copied() else {
            tracing::warn!(
                plugin = plugin_name,
                "shared-memory frame capability advertises no pixel formats"
            );
            return false;
        };
        let key = target.to_string();
        let request = BeginShmStreamRequest {
            target: target.clone(),
            generation,
            pixel_format: format.to_abi(),
            shape: capability.shape,
        };
        let outcome = match host.begin_shm_stream(request) {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::warn!(
                    plugin = plugin_name,
                    %error,
                    "shared-memory stream negotiation failed"
                );
                return false;
            }
        };
        let segment_bytes = match outcome {
            ShmStreamOutcome::Ready { segment_bytes } => segment_bytes,
            ShmStreamOutcome::Unsupported(reason) => {
                tracing::debug!(
                    plugin = plugin_name,
                    reason,
                    "plugin declined shared-memory streaming; falling back to the pipe"
                );
                return false;
            }
            ShmStreamOutcome::Io(reason) | ShmStreamOutcome::Internal(reason) => {
                tracing::warn!(
                    plugin = plugin_name,
                    reason,
                    "plugin host failed to begin a shared-memory stream; falling back to the pipe"
                );
                return false;
            }
            ShmStreamOutcome::Acknowledged => {
                tracing::warn!(
                    plugin = plugin_name,
                    "plugin host returned an unexpected acknowledgement for a stream begin"
                );
                return false;
            }
        };

        if let Err(error) = self.open_and_insert(
            plugin_name,
            &key,
            format,
            segment_bytes,
            generation,
            host.connection_epoch(),
        ) {
            tracing::warn!(
                plugin = plugin_name,
                %error,
                "opening the shared-memory segment failed; falling back to the pipe"
            );

            // The plugin-host already accepted and stood up its half; tell
            // it to tear that down again rather than leaving an orphaned
            // subscriber waiting for a publisher that will never appear.
            if let Err(error) = host.end_shm_stream(target.clone(), generation) {
                tracing::warn!(plugin = plugin_name, %error, "cleaning up an abandoned shared-memory stream begin failed");
            }
            return false;
        }

        tracing::info!(
            plugin = plugin_name,
            target = %target,
            generation,
            segment_bytes,
            "shared-memory frame stream active"
        );
        true
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "each parameter is an independent piece of state this stream's registry entry needs; bundling them would just move the count into a single-use struct"
    )]
    fn open_and_insert(
        &self,
        plugin_name: &str,
        key: &str,
        format: ShmPixelFormat,
        segment_bytes: u32,
        generation: u32,
        connection_epoch: u64,
    ) -> Result<()> {
        let names = luminate_host_supervisor::service_names(plugin_name, key)
            .context("deriving shared-memory service names")?;
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
            .context("opening shared-memory segment")?;
        let publisher = pubsub
            .publisher_builder()
            .initial_max_slice_len(usize::try_from(segment_bytes).unwrap_or(usize::MAX))
            .create()
            .context("creating shared-memory publisher")?;

        let event = node
            .service_builder(&names.event)
            .event()
            .max_notifiers(1)
            .max_listeners(1)
            .open_or_create()
            .context("opening shared-memory event channel")?;
        let notifier = event
            .notifier_builder()
            .create()
            .context("creating shared-memory notifier")?;

        #[allow(
            clippy::expect_used,
            clippy::unwrap_in_result,
            reason = "Acceptable to panic on poisoned locks"
        )]
        self.streams
            .lock()
            .expect("shared-memory stream registry lock poisoned")
            .insert(
                key.to_owned(),
                ActiveShmStream {
                    generation,
                    format,
                    publisher,
                    notifier,
                    connection_epoch,
                },
            );
        Ok(())
    }

    /// Attempts to deliver `envelope` over the shared-memory fast path.
    ///
    /// Returns `false` (never `Err`) if the caller should fall back to the
    /// ordinary pipe transport for this frame. This occurs when:
    ///
    /// - No active stream exists for `target`;
    /// - The envelope is `Partial` (the fast path only supports full frames);
    /// - `connection_epoch` no longer matches the epoch under which the stream
    ///   was established (the plugin host has since respawned; see
    ///   [`ActiveShmStream::connection_epoch`]); or
    /// - Delivery over the shared-memory transport itself fails.
    pub(super) fn apply(
        &self,
        target: &PluginTarget,
        envelope: &FrameEnvelope,
        connection_epoch: u64,
    ) -> bool {
        let FramePayload::Full(colours) = &envelope.payload else {
            return false;
        };
        let key = target.to_string();
        #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
        let mut streams = self
            .streams
            .lock()
            .expect("shared-memory stream registry lock poisoned");
        let Some(stream) = streams.get_mut(&key) else {
            return false;
        };
        if stream.generation != envelope.generation {
            return false;
        }
        if stream.connection_epoch != connection_epoch {
            // This stream belongs to a previous incarnation of the plugin host. Its
            // shared-memory segment no longer has a subscriber, so discard the stream
            // locally instead of pretending the send succeeded. The new process never
            // knows this stream existed.
            streams.remove(&key);
            return false;
        }
        match send_frame(stream, envelope, colours) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "shared-memory frame delivery failed; this frame will fall back to the pipe"
                );
                false
            }
        }
    }

    /// Returns whether `target` currently has an active shared-memory stream.
    ///
    /// This performs only a local registry lookup without plugin-host IPC, making
    /// it cheap enough to call from an async context. `EndFrameStream` uses
    /// it to avoid an unnecessary plugin-host round trip in [`Self::end`] for
    /// the common case where the stream was never shared-memory-backed.
    pub(super) fn has_active_stream(&self, target: &PluginTarget) -> bool {
        #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
        self.streams
            .lock()
            .expect("shared-memory stream registry lock poisoned")
            .contains_key(&target.to_string())
    }

    /// Tears down the shared-memory stream for `target`, if `generation`
    /// still matches the currently active one, telling the plugin host to
    /// tear down its own half symmetrically. This is a no-op if there is
    /// no matching active stream.
    pub(super) fn end(&self, host: &HostedPlugin, target: &PluginTarget, generation: u32) {
        let key = target.to_string();
        #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
        let removed = {
            let mut streams = self
                .streams
                .lock()
                .expect("shared-memory stream registry lock poisoned");
            let current_matches = streams
                .get(&key)
                .is_some_and(|stream| stream.generation == generation);
            if current_matches {
                streams.remove(&key);
            }
            current_matches
        };
        if removed && let Err(error) = host.end_shm_stream(target.clone(), generation) {
            tracing::warn!(%error, "ending a shared-memory stream on the plugin host failed");
        }
    }

    /// Force-ends the shared-memory stream for `target`, regardless of
    /// generation, and tells the plugin host to tear down its own half.
    ///
    /// Intended for connection-drop cleanup, where there is no specific
    /// generation to match against. This mirrors
    /// `DaemonState::end_all_frame_streams`, which likewise tears down streams
    /// unconditionally by target.
    ///
    /// A no-op if `target` has no active shared-memory stream.
    pub(super) fn force_end(&self, host: &HostedPlugin, target: &PluginTarget) {
        let key = target.to_string();
        #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
        let removed_generation = self
            .streams
            .lock()
            .expect("shared-memory stream registry lock poisoned")
            .remove(&key)
            .map(|stream| stream.generation);
        if let Some(generation) = removed_generation
            && let Err(error) = host.end_shm_stream(target.clone(), generation)
        {
            tracing::warn!(%error, "ending an abandoned shared-memory stream on the plugin host failed");
        }
    }
}

/// Loans a sample sized for `envelope`'s pixels, packs the header and
/// colour data into it, then publishes it with a notification.
///
/// The stream's negotiated pixel count and `colours.len()` must agree.
/// Violations are reported as
/// `ShmPixelFormatError::BufferLength`; the payload is never silently
/// truncated or padded.
fn send_frame(
    stream: &mut ActiveShmStream,
    envelope: &FrameEnvelope,
    colours: &[Colour],
) -> Result<()> {
    let per_pixel = stream.format.bytes_per_pixel();
    let pixel_count = u32::try_from(colours.len()).context("frame pixel count overflowed u32")?;
    let header = ShmFrameHeader {
        sequence: envelope.sequence,
        generation: envelope.generation,
        pixel_count,
        pixel_format: stream.format.to_abi(),
        header_version: SHM_FRAME_HEADER_VERSION,
        flags: 0,
        reserved: 0,
    }
    .with_commit(envelope.commit);

    let total_len = size_of::<ShmFrameHeader>()
        .checked_add(
            colours
                .len()
                .checked_mul(per_pixel)
                .context("frame payload size overflowed")?,
        )
        .context("frame payload size overflowed")?;
    let mut sample = stream
        .publisher
        .loan_slice(total_len)
        .context("loaning a shared-memory sample")?;
    let payload = sample.payload_mut();
    let (header_bytes, pixel_bytes) = payload
        .split_at_mut_checked(size_of::<ShmFrameHeader>())
        .context("loaned shared-memory sample was shorter than the frame header")?;
    header_bytes.copy_from_slice(&header.to_bytes());
    for (chunk, colour) in pixel_bytes.chunks_mut(per_pixel).zip(colours) {
        stream
            .format
            .pack(colour, chunk)
            .context("packing a pixel into the shared-memory buffer")?;
    }

    sample.send().context("sending a shared-memory sample")?;
    stream
        .notifier
        .notify()
        .context("notifying the shared-memory subscriber")?;
    Ok(())
}

#[cfg(test)]
#[path = "shm_tests.rs"]
mod tests;
