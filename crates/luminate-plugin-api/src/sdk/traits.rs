// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! The traits a plugin author implements.
//!
//! This is the whole safe surface of the SDK: plain Rust, no raw pointers,
//! no ABI awareness. `LuminatePlugin` is required; every other trait here is
//! an optional capability the export macro wires up only if implemented.

use luminate_core::frame::FrameEnvelope;
use luminate_core::shm_frame::{ShmFrameHeader, ShmPixelFormat};

use crate::{
    DeviceDescriptor, PluginError, PluginReadRequest, PluginRequestContext, PluginSetupRequest,
    PluginSetupStep, PluginStateSnapshot, PluginTarget, PluginUpdate, ProbeOutcome, RescanReason,
};

/// Optional setup implementation executed in a disposable host process.
pub trait SetupPlugin {
    /// Advances one setup workflow step.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the workflow, response, hardware, or
    /// transport cannot be handled safely.
    fn setup(request: PluginSetupRequest) -> Result<PluginSetupStep, PluginError>;
}

/// Core behaviour implemented by a safe Rust plugin.
pub trait LuminatePlugin: Send + Sync + Sized + 'static {
    /// Constructs the process-lifetime plugin instance from installed
    /// daemon configuration.
    ///
    /// # Errors
    ///
    /// Returns a typed error when configuration or process-local resources
    /// cannot be initialized. A failed construction makes probing reject the
    /// plugin.
    fn new() -> Result<Self, PluginError>;

    /// Determines whether this provider is unsupported, dormant, or ready.
    fn probe(&self) -> ProbeOutcome;

    /// Returns the plugin's complete authoritative topology snapshot.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the snapshot cannot be obtained safely.
    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError>;

    /// Applies one normalized update.
    ///
    /// # Errors
    ///
    /// Returns an error if the target or update is invalid, the operation is
    /// unsupported, hardware is unavailable, transport fails, or the plugin
    /// encounters an internal error.
    fn apply(
        &self,
        context: &PluginRequestContext,
        update: &PluginUpdate,
    ) -> Result<(), PluginError>;
}

/// Optional post-acceptance lifecycle implemented by dynamic providers.
pub trait StartPlugin: LuminatePlugin {
    /// Starts discovery or polling work after the host accepts the plugin.
    fn start(&self);
}

/// Optional hardware re-enumeration for plugins that cache their topology.
///
/// Implement this only when [`LuminatePlugin::topology`] answers from a cache
/// a background discovery thread owns. A plugin that enumerates hardware on
/// every `topology` call already re-enumerates when the daemon re-pulls, and
/// needs nothing here.
pub trait RescanPlugin: LuminatePlugin {
    /// Discards any cached view of the hardware so the `topology` call that
    /// immediately follows reports freshly discovered state.
    ///
    /// Called on resume from suspend, on a device-change notification, and on
    /// an explicit operator request; `reason` says which, so a plugin can be
    /// proportionate. It must not block for long: the daemon is waiting to
    /// re-pull topology, and on the resume path a user is waiting for their
    /// lighting to come back.
    fn rescan(&self, reason: RescanReason);
}

/// Optional native batch implementation for plugins that can coalesce work.
pub trait BatchPlugin: LuminatePlugin {
    /// Applies one ordered batch without implying cross-entry atomicity.
    ///
    /// The returned vector must contain exactly one result for each input
    /// update, in the same order.
    fn apply_batch(
        &self,
        context: &PluginRequestContext,
        updates: &[PluginUpdate],
    ) -> Vec<Result<(), PluginError>>;
}

/// Optional exact or best-effort hardware state readback.
pub trait ReadablePlugin: LuminatePlugin {
    /// Reads one bounded snapshot for the requested targets and facets.
    ///
    /// # Errors
    ///
    /// Returns a typed whole-request failure when no useful snapshot can be
    /// produced. Target-specific failures belong in the returned snapshot.
    fn read_state(
        &self,
        context: &PluginRequestContext,
        request: &PluginReadRequest,
    ) -> Result<PluginStateSnapshot, PluginError>;
}

/// Optional streamed-frame upload for targets that advertise
/// `FrameUploadCapability`. The host has already resolved stream ownership,
/// generation/sequence ordering, and rate limiting before this is called;
/// the plugin only needs to apply the frame it's given.
pub trait FrameStreamingPlugin: LuminatePlugin {
    /// Applies one frame envelope addressed to `target`.
    ///
    /// # Errors
    ///
    /// Returns an error if the target or frame is invalid, frame upload is
    /// unsupported, hardware is unavailable, transport fails, or the plugin
    /// encounters an internal error.
    fn upload_frame(
        &self,
        context: &PluginRequestContext,
        target: &PluginTarget,
        envelope: &FrameEnvelope,
    ) -> Result<(), PluginError>;
}

/// Optional opt-in, zero-copy shared-memory frame fast path, for a target
/// that additionally advertises `ShmFrameCapability` on top of the
/// always-available [`FrameStreamingPlugin`] path above. A plugin may
/// implement both (the host prefers shared memory when available and falls
/// back to `upload_frame` otherwise) or only `FrameStreamingPlugin`.
///
/// # Concurrency
///
/// See [`crate::PluginShmFrameApplyFn`]'s documentation: each active
/// stream's shared-memory callbacks run on a thread dedicated to that
/// stream, so a given `Self::Stream` is never touched by more than one
/// thread at a time, but different streams' callbacks (and an in-flight
/// ordinary command such as `apply`) can all run concurrently with each
/// other. Any state shared across streams (via `&self`, not
/// `Self::Stream`) must be synchronized accordingly.
pub trait ShmFrameStreamingPlugin: FrameStreamingPlugin {
    /// Per-stream state kept between `shm_stream_begin` and
    /// `shm_stream_end`. Must be `Send`: the value crosses from the thread
    /// that begins the stream to the (possibly different) thread that later
    /// applies frames to it and ends it.
    type Stream: Send;

    /// Begins a shared-memory stream for `target`, negotiated in `format`
    /// with `pixel_count` pixels.
    ///
    /// # Errors
    ///
    /// Returns an error if the stream parameters are invalid or the plugin
    /// cannot start the stream. The host then falls back to the ordinary
    /// [`FrameStreamingPlugin`] path instead of failing the client request.
    fn shm_stream_begin(
        &self,
        context: &PluginRequestContext,
        target: &PluginTarget,
        format: ShmPixelFormat,
        pixel_count: u32,
        generation: u32,
    ) -> Result<Self::Stream, PluginError>;

    /// Applies one shared-memory frame sample to `stream`.
    ///
    /// # Errors
    ///
    /// Returns an error if the frame is invalid or cannot be applied. The host
    /// does not retry or report individual fire-and-forget delivery failures
    /// to the client; persistent failures are visible in plugin-host logs.
    fn shm_frame(
        &self,
        context: &PluginRequestContext,
        stream: &mut Self::Stream,
        header: &ShmFrameHeader,
        pixels: &[u8],
    ) -> Result<(), PluginError>;

    /// Ends a shared-memory stream, releasing whatever `stream` holds.
    /// Infallible and best-effort, mirroring the daemon's own idempotent
    /// stream-teardown semantics.
    fn shm_stream_end(&self, stream: Self::Stream, generation: u32);
}
