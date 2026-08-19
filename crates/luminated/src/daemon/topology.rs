// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Debounced plugin topology reconciliation and event publication.

use luminate_core::device::DeviceId;

use super::executor::MutationExecutor;
use super::reconciliation::{effective_reconciliation_policy, reconcile_device};
use super::{
    Arc, DaemonError, Event, EventPublisher, HashSet, ManagementReadState, PluginManager,
    RescanReason, TOPOLOGY_DEBOUNCE, TopologyNotification, mpsc, panic, sync, task, time,
};

#[allow(
    clippy::too_many_lines,
    reason = "Topology replacement, reconciliation, durable commit, and event publication share one ordered transaction."
)]
pub(super) async fn run_topology_coordinator(
    mut notifications: mpsc::UnboundedReceiver<TopologyNotification>,
    mutations: MutationExecutor,
    plugin_manager: Arc<PluginManager>,
    management: Arc<ManagementReadState>,
    events: EventPublisher,
) {
    while let Some(first) = notifications.recv().await {
        let (batch, rescan_reason) =
            collect_topology_batch(first, &mut notifications, &plugin_manager).await;

        // Invalidate plugin-side caches before re-pulling, or a plugin that
        // serves topology from a discovery cache would just hand back the
        // same pre-suspend view. This blocks; each call is a round trip to a
        // plugin host.
        if let Some(reason) = rescan_reason {
            let manager = Arc::clone(&plugin_manager);
            if let Err(error) = task::spawn_blocking(move || manager.rescan_all(reason)).await {
                tracing::warn!(error = %error, "plugin rescan task failed; re-pulling topology anyway");
            }
        }

        for plugin_name in batch {
            let manager = Arc::clone(&plugin_manager);
            let management = Arc::clone(&management);
            let events = events.clone();
            let logged_name = plugin_name.clone();
            let changed_after_commit = Arc::new(sync::Mutex::new((Vec::new(), Vec::new())));
            let changed_from_job = Arc::clone(&changed_after_commit);
            let known_devices = mutations.device_ids().await;
            let known_device_set = known_devices.iter().cloned().collect::<HashSet<_>>();
            let topology_mutations = mutations.clone();
            let outcome = mutations
                .execute_without_notification(known_devices, Box::new(move |state| {
                    let reconciled =
                        manager
                            .reconcile_plugin_topology(&plugin_name)
                            .map_err(|error| {
                                DaemonError::Internal(format!(
                                    "topology reconciliation failed for {plugin_name}: {error:#}"
                                ))
                            })?;
                    let changed_devices = reconciled.changed_devices;

                    // A plugin-observed notification means "my device list may have
                    // changed", so there is nothing to reconcile when the topology is
                    // unchanged.
                    //
                    // A rescan means something different: hardware state may have been
                    // lost, even if the topology is identical. A keyboard can resume
                    // completely dark while enumerating exactly the same as before, so
                    // gating reconciliation on a topology diff would skip precisely the
                    // devices a resume exists to restore.
                    let owned = reconciled
                        .devices
                        .iter()
                        .filter(|device| {
                            manager.owner_name(&device.id).as_deref() == Some(plugin_name.as_str())
                        })
                        .map(|device| device.id.clone())
                        .collect::<Vec<_>>();
                    let reconciling = devices_to_reconcile(
                        &changed_devices,
                        &owned,
                        rescan_reason,
                    );
                    if reconciling.is_empty() {
                        tracing::debug!(
                            plugin = %plugin_name,
                            "topology notification produced no owned-device change"
                        );
                        return Ok(());
                    }
                    let new_locks = topology_mutations
                        .sequencers_for_new_devices(&known_device_set, &changed_devices);
                    // New devices are not visible to clients yet, so acquiring
                    // their sequencers here cannot invert an existing lock.
                    let _new_guards = new_locks
                        .iter()
                        .map(|lock| {
                            lock.lock()
                                .expect("lock poisoned")
                        })
                        .collect::<Vec<_>>();
                    let dropped_state = if changed_devices.is_empty() {
                        0
                    } else {
                        let mut state = state.blocking_lock();
                        state.replace_devices_preserving_withdrawn_state(reconciled.devices)
                    };

                    for device_id in &reconciling {
                        let Some(device) = state.blocking_lock().device(device_id) else {
                            continue;
                        };
                        let config = management.effective();
                        let policy = effective_reconciliation_policy(&config, &manager, &device);
                        match reconcile_device(state, &manager, device_id, policy) {
                            Ok(candidate) if !candidate.is_empty() => {
                                // The enclosing mutation job saves this staged
                                // delta before publishing it as durable.
                                state.blocking_lock().stage_adoption(candidate)?;
                            }
                            Ok(_) => {}
                            Err(error) => {
                                tracing::warn!(device = %device_id, error = %error, "device reappearance reconciliation failed");
                            }
                        }
                    }
                    tracing::info!(
                        plugin = %plugin_name,
                        changed_devices = ?changed_devices,
                        reconciled_devices = ?reconciling,
                        dropped_stale_state = dropped_state,
                        "plugin topology reconciled"
                    );
                    *changed_from_job
                        .lock()
                        .expect("lock poisoned") =
                        (changed_devices, reconciling);
                    Ok(())
                }))
                .await;
            match outcome {
                Ok(()) => {
                    let active = mutations
                        .device_ids()
                        .await
                        .into_iter()
                        .collect::<HashSet<_>>();
                    mutations.prune_device_sequencers(&active);
                    let (changed_devices, reconciled_devices) =
                        changed_after_commit.lock().expect("lock poisoned").clone();
                    publish_topology_changes(&events, changed_devices, reconciled_devices);
                }
                Err(error) => {
                    tracing::warn!(plugin = %logged_name, error = %error, "topology reconciliation rejected");
                }
            }
        }
    }
}

/// Publishes the observable effects of this reconciliation pass.
///
/// Topology and state are emitted as separate events because they evolve
/// independently. A reconciliation may update device state while leaving
/// the topology unchanged, so conflating the two would spuriously signal
/// a structural change.
///
/// Keeping them separate avoids unnecessary topology reloads when the
/// device list is byte-for-byte identical.
fn publish_topology_changes(
    events: &EventPublisher,
    changed_devices: Vec<DeviceId>,
    reconciled_devices: Vec<DeviceId>,
) {
    if !changed_devices.is_empty() {
        let _ = events.send(Event::TopologyChanged {
            devices: changed_devices,
        });
    }
    if !reconciled_devices.is_empty() {
        let _ = events.send(Event::StateChanged {
            devices: reconciled_devices,
        });
    }
}

/// Which devices this reconciliation pass should cover.
///
/// Notification-driven reconciliation and device-change rescans are
/// incremental: only devices that actually changed are revisited. Resume and
/// operator rescans are exhaustive, covering every device the plugin owns,
/// because they imply that hardware state may have changed in ways a topology
/// diff cannot observe.
///
/// A broad platform device-change signal carries no evidence that an
/// unchanged device lost state. Treating it as exhaustive would re-drive
/// unrelated devices for every Linux uevent. `changed` is included in the
/// exhaustive set so devices that have just disappeared still receive a final
/// reconciliation pass. The caller ignores any that are no longer present in
/// the topology.
fn devices_to_reconcile(
    changed: &[DeviceId],
    owned: &[DeviceId],
    rescan_reason: Option<RescanReason>,
) -> Vec<DeviceId> {
    if !matches!(
        rescan_reason,
        Some(RescanReason::Resume | RescanReason::Operator)
    ) {
        return changed.to_vec();
    }
    let mut devices = owned.to_vec();
    devices.extend(changed.iter().cloned());
    let mut seen = HashSet::new();
    devices.retain(|device| seen.insert(device.clone()));
    devices.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    devices
}

/// Collapses a burst of notifications into a single deduplicated, sorted
/// batch of plugins to re-pull.
///
/// A [`TopologyNotification::Rescan`] expands the batch to every loaded
/// plugin. Any `PluginObserved` notifications arriving in the same window
/// are therefore subsumed by that rescan rather than triggering a second
/// reconciliation pass.
///
/// This matches the common resume path, where a rescan is typically
/// accompanied by a burst of plugin-observed notifications as hardware
/// re-enumerates. Re-pulling the same plugin twice would only reconcile
/// the same devices twice.
async fn collect_topology_batch(
    first: TopologyNotification,
    notifications: &mut mpsc::UnboundedReceiver<TopologyNotification>,
    plugin_manager: &PluginManager,
) -> (Vec<String>, Option<RescanReason>) {
    let mut pending = HashSet::new();
    let mut rescan_reason = None;
    let mut absorb = |notification| match notification {
        TopologyNotification::PluginObserved(plugin_name) => {
            pending.insert(plugin_name);
        }
        TopologyNotification::Rescan(reason) => {
            rescan_reason = Some(merge_rescan_reason(rescan_reason, reason));
        }
    };

    absorb(first);
    let debounce = time::sleep(TOPOLOGY_DEBOUNCE);
    tokio::pin!(debounce);
    loop {
        tokio::select! {
            () = &mut debounce => break,
            notification = notifications.recv() => {
                let Some(notification) = notification else { break };
                absorb(notification);
            }
        }
    }

    if let Some(reason) = rescan_reason {
        let names = plugin_manager.plugin_names();
        tracing::info!(
            reason = ?reason,
            plugins = names.len(),
            "rescanning every loaded plugin's topology"
        );
        return (merge_rescan_batch(pending, Some(names)), Some(reason));
    }

    (merge_rescan_batch(pending, None), None)
}

/// Preserves the strongest reconciliation requirement in a debounced batch.
///
/// Resume must win over a nearby Linux device-change burst so the initial
/// post-suspend restoration remains exhaustive. An operator request also
/// remains exhaustive, while a device-change-only batch is incremental.
const fn merge_rescan_reason(
    current: Option<RescanReason>,
    incoming: RescanReason,
) -> RescanReason {
    match (current, incoming) {
        (Some(RescanReason::Resume), _) | (_, RescanReason::Resume) => RescanReason::Resume,
        (Some(RescanReason::Operator), _) | (_, RescanReason::Operator) => RescanReason::Operator,
        _ => RescanReason::DeviceChange,
    }
}

/// Produces the final re-pull batch by combining plugin-observed
/// notifications with the loaded-plugin set from a pending rescan,
/// then deduplicating and sorting the result into a deterministic order.
fn merge_rescan_batch(mut pending: HashSet<String>, rescan: Option<Vec<String>>) -> Vec<String> {
    if let Some(loaded) = rescan {
        pending.extend(loaded);
    }
    let mut pending = pending.into_iter().collect::<Vec<_>>();
    pending.sort();
    pending
}

#[cfg(test)]
#[path = "topology_tests.rs"]
mod tests;
