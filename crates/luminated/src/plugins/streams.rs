// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Shared-memory frame transport, in both directions.
//!
//! Two independent fast paths meet here. The daemon → plugin leg (`shm`) is an
//! opportunistic upgrade on top of an ordinary frame stream, and the
//! client → daemon leg (`shm_client`) is an explicit opt-in negotiated by
//! `BeginShmFrameStream`. Both are strictly optional: every failure leaves the
//! stream on the ordinary pipe transport, which is always available, so the
//! methods here report success as a plain `bool` rather than an error.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex as AsyncMutex;

use luminate_core::capability::ShmFrameCapability;
use luminate_core::frame::FrameEnvelope;
use luminate_core::target::TargetId;

use super::shm_client::ClientShmStreamReady;
use super::{PluginManager, plugin_target_from_target_id};
use crate::state::DaemonState;

impl PluginManager {
    /// Attempts to negotiate the zero-copy shared-memory fast path for
    /// `target` on top of an already-active frame stream.
    ///
    /// Returns whether negotiation succeeded. Failure is non-fatal: the stream
    /// remains on the ordinary pipe transport, which is already the default
    /// and fully functional.
    #[must_use]
    pub fn begin_shm_stream(
        &self,
        target: &TargetId,
        capability: &ShmFrameCapability,
        generation: u32,
    ) -> bool {
        let Ok(plugin) = self.plugin_for_device(target.device_id().as_str()) else {
            return false;
        };
        self.shm.begin(
            &plugin.metadata.name,
            &plugin.host,
            &plugin_target_from_target_id(target),
            capability,
            generation,
        )
    }

    /// Attempts to deliver `envelope` via the shared-memory fast path.
    ///
    /// Returns whether the fast path was used.
    ///
    /// `false` is not an error; it means the caller should deliver the
    /// frame via [`Self::apply_frame`] instead.
    #[must_use]
    pub fn try_apply_shm_frame(&self, target: &TargetId, envelope: &FrameEnvelope) -> bool {
        let Ok(plugin) = self.plugin_for_device(target.device_id().as_str()) else {
            return false;
        };
        self.shm.apply(
            &plugin_target_from_target_id(target),
            envelope,
            plugin.host.connection_epoch(),
        )
    }

    /// Returns whether `target` currently has an active shared-memory stream.
    ///
    /// This is a cheap local check (no plugin-host IPC), allowing callers to
    /// skip [`Self::end_shm_stream`]'s plugin-host round trip when the stream
    /// was never upgraded to shared memory.
    #[must_use]
    pub fn has_active_shm_stream(&self, target: &TargetId) -> bool {
        self.shm
            .has_active_stream(&plugin_target_from_target_id(target))
    }

    /// Ends the shared-memory stream for `target`, if one is active.
    ///
    /// This is a no-op if the stream was never upgraded to shared memory (the
    /// common case), so callers may invoke it unconditionally.
    pub fn end_shm_stream(&self, target: &TargetId, generation: u32) {
        if let Ok(plugin) = self.plugin_for_device(target.device_id().as_str()) {
            self.shm.end(
                &plugin.host,
                &plugin_target_from_target_id(target),
                generation,
            );
        }
    }

    /// Force-ends every active shared-memory stream among `targets`,
    /// regardless of generation.
    ///
    /// Connection teardown revokes all streams owned by the disconnecting
    /// client, so there is no specific generation to match against. This is
    /// used alongside `DaemonState::end_all_frame_streams`.
    pub fn end_all_shm_streams(&self, targets: &[TargetId]) {
        for target in targets {
            if let Ok(plugin) = self.plugin_for_device(target.device_id().as_str()) {
                self.shm
                    .force_end(&plugin.host, &plugin_target_from_target_id(target));
            }
        }
    }

    /// Negotiates and begins a client-published shared-memory stream on
    /// `target`.
    ///
    /// The caller is responsible for enforcing every capability and policy
    /// precondition documented on `Request::BeginShmFrameStream` (same-UID,
    /// `prefer_client_shm`, target capability, and exclusive stream
    /// ownership). This method assumes those checks have already succeeded and
    /// only establishes the daemon-side shared-memory transport and its
    /// dedicated subscriber thread.
    ///
    /// `plugin_manager` is conventionally `self` behind its own `Arc`. The
    /// spawned thread retains this handle so it can call
    /// [`Self::apply_one_frame`] for the lifetime of the stream without
    /// borrowing `self`.
    ///
    /// # Errors
    ///
    /// Returns an error only if daemon-side setup fails (for example, creating
    /// the iceoryx2 node, service, or subscriber thread). Client-controlled
    /// rejection reasons are handled before this method is called.
    pub fn begin_shm_client_stream(
        &self,
        plugin_manager: &Arc<PluginManager>,
        state: &Arc<AsyncMutex<DaemonState>>,
        target: &TargetId,
        capability: &ShmFrameCapability,
        generation: u32,
    ) -> Result<Option<ClientShmStreamReady>> {
        self.shm_client.begin(
            target,
            capability,
            generation,
            Arc::clone(plugin_manager),
            Arc::clone(state),
        )
    }

    /// Ends the client-published shared-memory stream for `target`, provided
    /// `generation` still identifies the active stream.
    ///
    /// Returns whether a stream was actually torn down, allowing the caller to
    /// emit a stream-ended lifecycle event only when appropriate.
    pub fn end_shm_client_stream(&self, target: &TargetId, generation: u32) -> bool {
        self.shm_client.end(target, generation)
    }

    /// Force-ends every active client-published shared-memory stream among
    /// `targets`, regardless of generation.
    ///
    /// Connection teardown revokes every stream owned by the disconnecting
    /// client, so there is no specific generation to match against.
    ///
    /// Returns the `(target, generation)` pairs whose streams were actually
    /// torn down, allowing the caller to emit one stream-ended lifecycle event
    /// for each.
    pub fn end_all_shm_client_streams(&self, targets: &[TargetId]) -> Vec<(TargetId, u32)> {
        targets
            .iter()
            .filter_map(|target| {
                self.shm_client
                    .force_end(target)
                    .map(|generation| (target.clone(), generation))
            })
            .collect()
    }
}
