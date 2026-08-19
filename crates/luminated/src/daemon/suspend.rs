// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Suspend quiescing and resume reconciliation.
//!
//! What the daemon does when the machine goes down and comes back, kept
//! separate from *how* it finds out (see [`luminate_platform::power`] for
//! the platform sources). Suspend quiesces; resume asks for a rescan and lets
//! the existing topology-coordinator path reconcile whatever moved.

use super::executor::MutationExecutor;
use super::{Arc, Event, HashSet, PluginManager, RescanReason, RescanRequester, mem, sync};

/// Prepares the daemon for system suspend.
///
/// Three things happen, in order:
///
/// 1. End every active frame stream and notify its client. Continuing to
///    publish into a suspending machine is at best wasted work and at worst
///    leaves a client writing into a shared-memory segment that nobody will
///    read again. The daemon cannot revoke a client's mapping, so this event
///    is the only way clients learn to stop.
/// 2. Mark all observations stale. Hardware may be power-cycled or reset
///    while the machine is suspended, so pre-suspend observations cease to
///    be evidence of current hardware state the moment suspend begins.
/// 3. Preserve desired state. Suspend does not change user intent; it only
///    invalidates our knowledge of the hardware.
///
/// This function deliberately does not drive hardware dark. That is a policy
/// decision about what the user wants; its sole responsibility is to stop
/// claiming facts that are about to become unreliable.
pub(super) async fn quiesce_for_suspend(
    mutations: &MutationExecutor,
    plugin_manager: &Arc<PluginManager>,
) {
    let streaming = mutations.state.lock().await.streaming_targets();
    if !streaming.is_empty() {
        let devices = streaming
            .iter()
            .map(|target| target.device_id().clone())
            .collect::<HashSet<_>>();
        mutations
            .state
            .lock()
            .await
            .end_all_frame_streams(&streaming);

        let manager = Arc::clone(plugin_manager);
        let targets = streaming.clone();
        let ended_client_streams = Arc::new(sync::Mutex::new(Vec::new()));
        let ended_in_job = Arc::clone(&ended_client_streams);

        // A target is streamed by exactly one publisher: the daemon or a client.
        // `streaming_targets` records only that streaming exists, not which side
        // owns it, so both registries are swept. The inactive registry naturally
        // becomes a no-op.
        let _ = mutations
            .execute_hardware_only(
                devices.iter().cloned().collect(),
                Box::new(move || {
                    manager.end_all_shm_streams(&targets);
                    let ended = manager.end_all_shm_client_streams(&targets);
                    *ended_in_job.lock().expect("lock poisoned") = ended;
                    Ok(())
                }),
            )
            .await;

        for (target, generation) in
            mem::take(&mut *ended_client_streams.lock().expect("lock poisoned"))
        {
            let _ = mutations
                .events
                .send(Event::ShmStreamEnded { target, generation });
        }

        let _ = mutations.events.send(Event::StateChanged {
            devices: devices.into_iter().collect(),
        });

        tracing::info!(
            streams = streaming.len(),
            "ended active frame streams before suspend"
        );
    }

    let marked = mutations.state.lock().await.mark_observations_stale();
    tracing::info!(
        observations = marked,
        "quiesced for suspend; observations are no longer current"
    );
}

/// Reacts to the machine resuming from suspend.
///
/// Requests a rescan and returns. Everything else happens in the topology
/// coordinator, which re-enumerates every plugin and reconciles devices
/// under their existing policies.
///
/// This reuses the same path as daemon startup and device hotplug. Resume is
/// another point where Luminate regains control of hardware whose state it
/// can no longer trust, not a separate reconciliation mode.
pub(super) fn reconcile_after_resume(rescans: &RescanRequester) {
    if rescans.request(RescanReason::Resume) {
        tracing::info!("resumed from suspend; rescanning hardware");
    } else {
        tracing::warn!("resumed from suspend but the topology coordinator is gone");
    }
}

#[cfg(test)]
#[path = "suspend_tests.rs"]
mod tests;
