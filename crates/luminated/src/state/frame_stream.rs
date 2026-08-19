// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Frame-stream lifecycle: starting, uploading to, and ending a raw pixel
//! stream on a target, independent of ordinary `target_state` mutation.

use std::time::{Duration, Instant};

use luminate_core::effect::Effect;
use luminate_core::frame::FrameEnvelope;
use luminate_core::target::TargetId;

use crate::error::DaemonError;

use super::DaemonState;
use super::target_state::TargetState;

#[derive(Debug, Clone, Copy)]
pub(super) struct FrameStream {
    generation: u32,
    topology_generation: u64,
    next_sequence: u64,
    last_forwarded_at: Option<Instant>,
}

impl DaemonState {
    /// Starts a frame stream on `target`.
    ///
    /// On success, returns the stream's generation token. The caller must
    /// echo this value in every subsequent `UploadFrame` and
    /// `EndFrameStream` request for the lifetime of the stream.
    ///
    /// Fails if `target` does not advertise frame-upload capability, already
    /// has an active frame stream, or has an active hardware effect that is
    /// not marked `concurrent_with_streaming`.
    pub fn begin_frame_stream(&mut self, target: &TargetId) -> Result<u32, DaemonError> {
        let capabilities = self.capabilities_for_target_or_error(target)?;
        if capabilities.frame_upload.is_none() {
            return Err(DaemonError::UnsupportedCapability {
                target: target.clone(),
                reason: "target does not advertise frame-upload capability".to_owned(),
            });
        }
        if self.frame_streams.contains_key(target) {
            return Err(DaemonError::Conflict {
                target: target.clone(),
                reason: "a frame stream is already active on this target".to_owned(),
            });
        }
        let effect_active = self.target_state.iter().any(|entry| {
            self.target_covers(&entry.target, target)
                && matches!(
                    entry.state,
                    TargetState::Effect(
                        Effect::Breathe { .. }
                            | Effect::Pulse { .. }
                            | Effect::Strobe { .. }
                            | Effect::Scanner { .. }
                            | Effect::Morph { .. }
                            | Effect::Spectrum { .. }
                            | Effect::Rainbow { .. }
                            | Effect::Hardware { .. }
                    )
                )
        });
        let effect_concurrent = capabilities
            .hardware_effects
            .as_ref()
            .is_some_and(|effects| effects.concurrent_with_streaming);
        if effect_active && !effect_concurrent {
            return Err(DaemonError::Conflict {
                target: target.clone(),
                reason: "an active hardware effect is not marked concurrent with streaming"
                    .to_owned(),
            });
        }

        let generation = self
            .frame_streams
            .values()
            .map(|stream| stream.generation)
            .max()
            .unwrap_or(0)
            .wrapping_add(1);

        self.frame_streams.insert(
            target.clone(),
            FrameStream {
                generation,
                topology_generation: self.topology_generation(),
                next_sequence: 0,
                last_forwarded_at: None,
            },
        );
        Ok(generation)
    }

    /// Validates an uploaded frame against its stream's generation,
    /// sequence number, and (if the target declares one) rate limit.
    ///
    /// Returns whether the frame should be forwarded to the plugin. A return
    /// value of `false` indicates the frame was accepted but suppressed by
    /// rate limiting, rather than rejected or applied.
    pub fn record_frame_upload(
        &mut self,
        target: &TargetId,
        envelope: &FrameEnvelope,
    ) -> Result<bool, DaemonError> {
        let max_rate_hz = self
            .capabilities_for_target(target)
            .and_then(|capabilities| capabilities.frame_upload.as_ref())
            .and_then(|frame_upload| frame_upload.max_rate_hz);

        let topology_generation = self.topology_generation();
        let stream = self
            .frame_streams
            .get_mut(target)
            .ok_or_else(|| DaemonError::Conflict {
                target: target.clone(),
                reason: "no active frame stream on this target".to_owned(),
            })?;

        if stream.topology_generation != topology_generation {
            self.frame_streams.remove(target);
            return Err(DaemonError::AuthorizationConflict {
                reason: "device topology changed after the frame stream was authorized; begin a new stream"
                    .to_owned(),
            });
        }

        if envelope.generation != stream.generation {
            return Err(DaemonError::Conflict {
                target: target.clone(),
                reason: "frame generation does not match the active stream".to_owned(),
            });
        }
        if envelope.sequence < stream.next_sequence {
            return Err(DaemonError::InvalidArgument {
                target: target.clone(),
                reason: format!(
                    "frame sequence {} is stale; the active stream already accepted sequence {}",
                    envelope.sequence,
                    stream.next_sequence.saturating_sub(1)
                ),
            });
        }

        stream.next_sequence = envelope.sequence.saturating_add(1);

        let now = Instant::now();
        let should_forward = match (max_rate_hz, stream.last_forwarded_at) {
            (Some(max_rate_hz), Some(last_forwarded_at)) => {
                let min_interval = Duration::from_secs_f64(1.0 / f64::from(max_rate_hz.max(1)));
                now.duration_since(last_forwarded_at) >= min_interval
            }
            _ => true,
        };
        if should_forward {
            stream.last_forwarded_at = Some(now);
        }
        Ok(should_forward)
    }

    /// Returns the topology generation bound to the matching active stream.
    pub(crate) fn frame_stream_topology_generation(
        &mut self,
        target: &TargetId,
        generation: u32,
    ) -> Result<u64, DaemonError> {
        let stream = self
            .frame_streams
            .get(target)
            .ok_or_else(|| DaemonError::Conflict {
                target: target.clone(),
                reason: "no active frame stream on this target".to_owned(),
            })?;
        if stream.generation != generation {
            return Err(DaemonError::Conflict {
                target: target.clone(),
                reason: "frame generation does not match the active stream".to_owned(),
            });
        }
        if stream.topology_generation != self.topology_generation() {
            self.frame_streams.remove(target);
            return Err(DaemonError::AuthorizationConflict {
                reason: "device topology changed after the frame stream was authorized; begin a new stream"
                    .to_owned(),
            });
        }
        Ok(stream.topology_generation)
    }

    /// Ends the frame stream on `target`. Ending a stream that isn't active,
    /// or whose `generation` doesn't match, is idempotent rather than an
    /// error: disconnect cleanup can race an explicit `EndFrameStream`.
    pub fn end_frame_stream(&mut self, target: &TargetId, generation: u32) {
        if self
            .frame_streams
            .get(target)
            .is_some_and(|stream| stream.generation == generation)
        {
            self.frame_streams.remove(target);
        }
    }

    /// Ends every active frame stream owned by a closed connection.
    pub fn end_all_frame_streams(&mut self, targets: &[TargetId]) {
        for target in targets {
            self.frame_streams.remove(target);
        }
    }

    /// Every target with an active frame stream, sorted for determinism.
    ///
    /// Connection teardown knows which targets one client owned; a
    /// daemon-wide event such as suspend does not, and needs the whole set.
    #[must_use]
    pub fn streaming_targets(&self) -> Vec<TargetId> {
        let mut targets = self.frame_streams.keys().cloned().collect::<Vec<_>>();
        // `TargetId` is not `Ord`; its debug form is stable and total, which
        // is all that's needed. The order only has to be reproducible so
        // teardown and its logs don't shuffle between runs.
        targets.sort_by_key(|target| format!("{target:?}"));
        targets
    }

    /// Whether `target` is currently covered by an active frame stream,
    /// including a stream owning a broader target (for example a
    /// device-scope stream covering one of the device's elements).
    #[must_use]
    pub(super) fn is_frame_streaming(&self, target: &TargetId) -> bool {
        self.frame_streams
            .keys()
            .any(|owner| self.target_covers(owner, target))
    }

    pub(crate) fn ensure_not_frame_streaming(&self, target: &TargetId) -> Result<(), DaemonError> {
        if self.is_frame_streaming(target) {
            return Err(DaemonError::Conflict {
                target: target.clone(),
                reason: "a frame stream is active on this target".to_owned(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "frame_stream_tests.rs"]
mod tests;
