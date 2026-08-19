// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::{
    Client, FrameAck, FrameEnvelope, Request, ResponseStatus, Result, ShmStreamReady, TargetId,
    unexpected_response,
};

impl Client {
    /// Start a frame stream on `target`, returning the generation to echo
    /// back in every [`Client::upload_frame`]/[`Client::end_frame_stream`]
    /// call for this stream. Rejected if `target` doesn't advertise
    /// frame-upload capability, already has an active stream, or has an
    /// active hardware effect that isn't marked concurrent with streaming.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed,
    /// [`Error::Conflict`] for
    /// the ownership conflicts above, [`Error::Unsupported`] if the target
    /// has no frame-upload capability, or another [`Error`] variant for an
    /// I/O/protocol failure.
    pub async fn begin_frame_stream(&self, target: TargetId) -> Result<u32> {
        match self
            .request(Request::BeginFrameStream { target })
            .await?
            .status
        {
            ResponseStatus::FrameStreamStarted { generation } => Ok(generation),
            other @ (ResponseStatus::ServerInfo(_)
            | ResponseStatus::Devices(_)
            | ResponseStatus::WithdrawnDevices(_)
            | ResponseStatus::Device(_)
            | ResponseStatus::State(_)
            | ResponseStatus::CollectionState(_)
            | ResponseStatus::EventTicket(_)
            | ResponseStatus::Ack
            | ResponseStatus::FrameAck { .. }
            | ResponseStatus::CollectionCreated { .. }
            | ResponseStatus::Collections(_)
            | ResponseStatus::CollectionInfo(_)
            | ResponseStatus::CollectionApplied { .. }
            | ResponseStatus::ShmFrameStreamReady { .. }
            | ResponseStatus::PluginSetupWorkflows(_)
            | ResponseStatus::ManagementSnapshot(_)
            | ResponseStatus::ManagementPatched(_)
            | ResponseStatus::AccessPolicy(_)
            | ResponseStatus::TokenCreated { .. }
            | ResponseStatus::Tokens(_)
            | ResponseStatus::AttestationCreated { .. }
            | ResponseStatus::Attestations(_)
            | ResponseStatus::Scene(_)
            | ResponseStatus::Scenes(_)
            | ResponseStatus::SceneInfo(_)
            | ResponseStatus::SceneApplied { .. }
            | ResponseStatus::Transition(_)
            | ResponseStatus::PluginSetupSession(_)
            | ResponseStatus::Error { .. }) => {
                Err(unexpected_response("frame stream started", &other))
            }
        }
    }

    /// Upload one frame to an active stream on `target`. `envelope.sequence`
    /// must be monotonically increasing within `envelope.generation`; a
    /// frame arriving faster than the target's declared `max_rate_hz` is
    /// accepted but silently rate-limited (`dropped: true` in the returned
    /// ack), not forwarded to the plugin.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed,
    /// [`Error::Conflict`] if the
    /// generation doesn't match the active stream, [`Error::InvalidArgument`]
    /// for a stale/non-increasing sequence, or another [`Error`] variant for
    /// an I/O/protocol failure.
    pub async fn upload_frame(
        &self,
        target: TargetId,
        envelope: FrameEnvelope,
    ) -> Result<FrameAck> {
        match self
            .request(Request::UploadFrame { target, envelope })
            .await?
            .status
        {
            ResponseStatus::FrameAck { sequence, dropped } => Ok(FrameAck { sequence, dropped }),
            other @ (ResponseStatus::ServerInfo(_)
            | ResponseStatus::Devices(_)
            | ResponseStatus::WithdrawnDevices(_)
            | ResponseStatus::Device(_)
            | ResponseStatus::State(_)
            | ResponseStatus::CollectionState(_)
            | ResponseStatus::EventTicket(_)
            | ResponseStatus::Ack
            | ResponseStatus::FrameStreamStarted { .. }
            | ResponseStatus::CollectionCreated { .. }
            | ResponseStatus::Collections(_)
            | ResponseStatus::CollectionInfo(_)
            | ResponseStatus::CollectionApplied { .. }
            | ResponseStatus::ShmFrameStreamReady { .. }
            | ResponseStatus::PluginSetupWorkflows(_)
            | ResponseStatus::ManagementSnapshot(_)
            | ResponseStatus::ManagementPatched(_)
            | ResponseStatus::AccessPolicy(_)
            | ResponseStatus::TokenCreated { .. }
            | ResponseStatus::Tokens(_)
            | ResponseStatus::AttestationCreated { .. }
            | ResponseStatus::Attestations(_)
            | ResponseStatus::Scene(_)
            | ResponseStatus::Scenes(_)
            | ResponseStatus::SceneInfo(_)
            | ResponseStatus::SceneApplied { .. }
            | ResponseStatus::Transition(_)
            | ResponseStatus::PluginSetupSession(_)
            | ResponseStatus::Error { .. }) => Err(unexpected_response("frame ack", &other)),
        }
    }

    /// End the frame stream on `target` started with generation `generation`.
    /// Ending a stream that isn't active, or whose generation doesn't match
    /// the caller's, is idempotent rather than an error.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed, or another [`Error`]
    /// variant for an I/O/protocol failure.
    pub async fn end_frame_stream(&self, target: TargetId, generation: u32) -> Result<()> {
        self.expect_ack(Request::EndFrameStream { target, generation })
            .await
    }

    /// Negotiates the client → daemon shared-memory frame-streaming fast path
    /// on `target`, as an explicit opt-in alternative to
    /// [`Self::begin_frame_stream`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] if the fast path isn't offered to this
    /// connection or target for any reason (uid mismatch, disabled by
    /// daemon config, no advertised capability, or the target is already
    /// streaming): every such case collapses to the same error, since the
    /// caller's uniform fallback is [`Self::begin_frame_stream`] regardless
    /// of which precondition failed. Returns [`Error::Unavailable`] if the
    /// connection has closed, or another [`Error`] variant for an
    /// I/O/protocol failure.
    pub async fn begin_shm_frame_stream(&self, target: TargetId) -> Result<ShmStreamReady> {
        match self
            .request(Request::BeginShmFrameStream { target })
            .await?
            .status
        {
            ResponseStatus::ShmFrameStreamReady {
                generation,
                service_name,
                event_service_name,
                pixel_format,
                stream_nonce,
                segment_bytes,
            } => Ok(ShmStreamReady {
                generation,
                service_name,
                event_service_name,
                pixel_format,
                stream_nonce,
                segment_bytes,
            }),
            other @ (ResponseStatus::ServerInfo(_)
            | ResponseStatus::Devices(_)
            | ResponseStatus::WithdrawnDevices(_)
            | ResponseStatus::Device(_)
            | ResponseStatus::State(_)
            | ResponseStatus::CollectionState(_)
            | ResponseStatus::EventTicket(_)
            | ResponseStatus::Ack
            | ResponseStatus::FrameStreamStarted { .. }
            | ResponseStatus::FrameAck { .. }
            | ResponseStatus::CollectionCreated { .. }
            | ResponseStatus::Collections(_)
            | ResponseStatus::CollectionInfo(_)
            | ResponseStatus::CollectionApplied { .. }
            | ResponseStatus::PluginSetupWorkflows(_)
            | ResponseStatus::ManagementSnapshot(_)
            | ResponseStatus::ManagementPatched(_)
            | ResponseStatus::AccessPolicy(_)
            | ResponseStatus::TokenCreated { .. }
            | ResponseStatus::Tokens(_)
            | ResponseStatus::AttestationCreated { .. }
            | ResponseStatus::Attestations(_)
            | ResponseStatus::Scene(_)
            | ResponseStatus::Scenes(_)
            | ResponseStatus::SceneInfo(_)
            | ResponseStatus::SceneApplied { .. }
            | ResponseStatus::Transition(_)
            | ResponseStatus::PluginSetupSession(_)
            | ResponseStatus::Error { .. }) => {
                Err(unexpected_response("shm frame stream ready", &other))
            }
        }
    }

    /// Ends the client-published shared-memory stream on `target` with the
    /// given `generation`. Ending a stream that isn't active, or whose
    /// generation doesn't match the caller's, is idempotent rather than an
    /// error, mirroring [`Self::end_frame_stream`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed, or
    /// another [`Error`] variant for an I/O/protocol failure.
    pub async fn end_shm_frame_stream(&self, target: TargetId, generation: u32) -> Result<()> {
        self.expect_ack(Request::EndShmFrameStream { target, generation })
            .await
    }
}
