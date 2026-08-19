// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Serialized mutation and hardware-job execution.

use super::transition::TransitionRegistry;
use super::{
    Arc, DaemonError, DaemonState, Effect, Event, EventPublisher, HashMap, HashSet, Mutex, Path,
    PluginManager, TargetId, TargetState, device, panic, persistence, sync, task,
};
use crate::operator_event::OperatorEvent;

/// A mutation closure. It receives the shared state handle so it can
/// lock only for validation/commit and release it while waiting on an isolated
/// plugin host or filesystem I/O.
pub(super) type MutationJob<T = ()> =
    Box<dyn FnOnce(&Arc<Mutex<DaemonState>>) -> Result<T, DaemonError> + Send>;

/// Hardware work that needs per-device ordering without state persistence.
pub(super) type SequencedJob<T = ()> = Box<dyn FnOnce() -> Result<T, DaemonError> + Send>;

#[derive(Clone)]
pub(super) struct MutationExecutor {
    pub(super) state: Arc<Mutex<DaemonState>>,
    pub(super) state_path: Arc<Path>,
    pub(super) commit: Arc<sync::Mutex<()>>,
    pub(super) device_sequencers: Arc<sync::Mutex<HashMap<device::DeviceId, Arc<sync::Mutex<()>>>>>,
    pub(super) events: EventPublisher,
    pub(super) operator_events: bool,
    pub(super) transitions: Arc<TransitionRegistry>,
}

impl MutationExecutor {
    pub(super) async fn device_ids(&self) -> Vec<device::DeviceId> {
        self.state
            .lock()
            .await
            .devices()
            .into_iter()
            .map(|device| device.id)
            .collect()
    }

    pub(super) fn sequencers_for_new_devices(
        &self,
        known: &HashSet<device::DeviceId>,
        changed: &[device::DeviceId],
    ) -> Vec<Arc<sync::Mutex<()>>> {
        let mut added = changed
            .iter()
            .filter(|device| !known.contains(*device))
            .cloned()
            .collect::<Vec<_>>();
        normalize_devices(&mut added);

        let mut sequencers = self.device_sequencers.lock().expect("lock poisoned");
        added
            .into_iter()
            .map(|device| {
                Arc::clone(
                    sequencers
                        .entry(device)
                        .or_insert_with(|| Arc::new(sync::Mutex::new(()))),
                )
            })
            .collect()
    }

    pub(super) async fn execute_authorized<T: Send + 'static>(
        &self,
        devices: Vec<device::DeviceId>,
        topology_generation: u64,
        job: MutationJob<T>,
    ) -> Result<T, DaemonError> {
        self.execute_inner(devices, Some(topology_generation), job, true)
            .await
    }

    /// Runs a mutation through generation reservation and persistence, but
    /// leaves event publication to the caller.
    pub(super) async fn execute_without_notification<T: Send + 'static>(
        &self,
        devices: Vec<device::DeviceId>,
        job: MutationJob<T>,
    ) -> Result<T, DaemonError> {
        self.execute_inner(devices, None, job, false).await
    }

    /// Sequences hardware work against other mutations for the same devices.
    ///
    /// This does not reserve state generations, persist daemon state, or
    /// publish change events.
    pub(super) async fn execute_hardware_only(
        &self,
        devices: Vec<device::DeviceId>,
        job: SequencedJob,
    ) -> Result<(), DaemonError> {
        let sequencers = Arc::clone(&self.device_sequencers);

        task::spawn_blocking(move || run_sequenced_job(&sequencers, devices, job))
            .await
            .map_err(|error| DaemonError::Internal(format!("hardware task failed: {error}")))?
    }

    /// Sequences hardware work and rejects a stale authorization decision
    /// before routing to a plugin.
    pub(super) async fn execute_hardware_only_authorized(
        &self,
        devices: Vec<device::DeviceId>,
        topology_generation: u64,
        job: SequencedJob,
    ) -> Result<(), DaemonError> {
        let sequencers = Arc::clone(&self.device_sequencers);
        let state = Arc::clone(&self.state);

        task::spawn_blocking(move || {
            run_sequenced_job(
                &sequencers,
                devices,
                Box::new(move || {
                    if !state
                        .blocking_lock()
                        .topology_generation_is(topology_generation)
                    {
                        return Err(DaemonError::AuthorizationConflict {
                            reason:
                                "device topology changed after authorization; retry the request"
                                    .to_owned(),
                        });
                    }
                    job()
                }),
            )
        })
        .await
        .map_err(|error| DaemonError::Internal(format!("hardware task failed: {error}")))?
    }

    async fn execute_inner<T: Send + 'static>(
        &self,
        mut devices: Vec<device::DeviceId>,
        topology_generation: Option<u64>,
        job: MutationJob<T>,
        notify: bool,
    ) -> Result<T, DaemonError> {
        let state = Arc::clone(&self.state);
        let state_path = Arc::clone(&self.state_path);
        let commit = Arc::clone(&self.commit);
        let sequencers = Arc::clone(&self.device_sequencers);
        let events = self.events.clone();
        let operator_events = self.operator_events;

        normalize_devices(&mut devices);

        task::spawn_blocking(move || {
            let device_ids = devices.clone();
            run_normalized_sequenced_job(
                &sequencers,
                devices,
                Box::new(move || {
                    {
                        // Reserve generations before hardware I/O while retaining the
                        // device sequencers. Publication can then reject stale work.
                        let _commit = commit.lock().expect("lock poisoned");
                        let mut daemon_state = state.blocking_lock();
                        if topology_generation.is_some_and(|generation| {
                            !daemon_state.topology_generation_is(generation)
                        }) {
                            return Err(DaemonError::AuthorizationConflict {
                                reason:
                                    "device topology changed after authorization; retry the request"
                                        .to_owned(),
                            });
                        }
                        for device in &device_ids {
                            daemon_state.reserve_generation(device.clone());
                        }
                    }
                    let outcome =
                        run_mutation_job(&state, &state_path, &commit, job, operator_events);
                    if notify
                        && !device_ids.is_empty()
                        && (outcome.is_ok()
                            || matches!(outcome, Err(DaemonError::PartialMutation { .. })))
                    {
                        let _ = events.send(Event::StateChanged {
                            devices: device_ids,
                        });
                    }
                    outcome
                }),
            )
        })
        .await
        .map_err(|error| DaemonError::Internal(format!("mutation task failed: {error}")))?
    }

    /// Removes sequencers for devices that are no longer active and have no
    /// in-flight users. A later operation can safely recreate an idle entry.
    pub(super) fn prune_device_sequencers(&self, active: &HashSet<device::DeviceId>) {
        self.device_sequencers
            .lock()
            .expect("lock poisoned")
            .retain(|device, sequencer| {
                active.contains(device) || Arc::strong_count(sequencer) > 1
            });
    }
}

fn normalize_devices(devices: &mut Vec<device::DeviceId>) {
    devices.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    devices.dedup();
}

fn run_sequenced_job(
    sequencers: &sync::Mutex<HashMap<device::DeviceId, Arc<sync::Mutex<()>>>>,
    mut devices: Vec<device::DeviceId>,
    job: SequencedJob,
) -> Result<(), DaemonError> {
    normalize_devices(&mut devices);
    run_normalized_sequenced_job(sequencers, devices, job)
}

fn run_normalized_sequenced_job<T>(
    sequencers: &sync::Mutex<HashMap<device::DeviceId, Arc<sync::Mutex<()>>>>,
    devices: Vec<device::DeviceId>,
    job: SequencedJob<T>,
) -> Result<T, DaemonError> {
    let locks = {
        let mut known = sequencers.lock().expect("lock poisoned");
        devices
            .into_iter()
            .map(|device| {
                Arc::clone(
                    known
                        .entry(device)
                        .or_insert_with(|| Arc::new(sync::Mutex::new(()))),
                )
            })
            .collect::<Vec<_>>()
    };
    // Stable device-ID ordering avoids deadlock for multi-device operations.
    let _guards = locks
        .iter()
        .map(|lock| lock.lock().expect("lock poisoned"))
        .collect::<Vec<_>>();

    job()
}

/// Applies a target-scoped state to hardware through the owning plugin.
///
/// The target may be a whole device or any sub-device leaf (surface,
/// element, or device-scoped group), matching the kinds of targets a
/// collection may contain.
pub(super) fn apply_target_state(
    daemon_state: &Arc<Mutex<DaemonState>>,
    plugin_manager: &PluginManager,
    target: &TargetId,
    state: &TargetState,
) -> Result<(), DaemonError> {
    match state {
        TargetState::Effect(Effect::Static { colour }) => {
            let resolved = daemon_state
                .blocking_lock()
                .resolve_colour(target, colour)?;
            plugin_manager.apply_effect(target, &Effect::Static { colour: resolved })
        }
        TargetState::Effect(effect) => plugin_manager.apply_effect(target, effect),
        TargetState::Brightness(value) => plugin_manager.apply_brightness(target, *value),
        TargetState::AppearanceSlots(values) => {
            plugin_manager.apply_appearance_slots(target, values)
        }
        TargetState::Clear => plugin_manager.apply_clear(target),
    }
}

fn run_mutation_job<T>(
    state: &Arc<Mutex<DaemonState>>,
    state_path: &Path,
    commit: &sync::Mutex<()>,
    job: MutationJob<T>,
    operator_events: bool,
) -> Result<T, DaemonError> {
    let outcome = panic::catch_unwind(panic::AssertUnwindSafe(|| job(state)));
    let outcome = outcome.unwrap_or_else(|_| {
        tracing::error!("mutation task panicked");
        Err(DaemonError::Internal("mutation task panicked".to_owned()))
    });
    if outcome.is_ok() || matches!(outcome, Err(DaemonError::PartialMutation { .. })) {
        // Every state-file write is a whole snapshot, so unrelated devices
        // still share this short commit/save coordinator.
        let _commit = commit.lock().expect("lock poisoned");
        let mut daemon_state = state.blocking_lock();
        let adopted = daemon_state.adopted_baseline_for_persistence();
        if let Err(error) = persistence::save_complete(
            state_path,
            daemon_state.target_states(),
            daemon_state.collections(),
            daemon_state.scenes(),
            &adopted,
        ) {
            drop(daemon_state);
            state.blocking_lock().finish_pending_adoption(false);
            tracing::error!(error = %error, "mutation applied but daemon state persistence failed");
            if operator_events {
                OperatorEvent::PersistedStateSaveFailed.emit(&format!(
                    "mutation applied but daemon state persistence failed: {error:#}"
                ));
            }
            return Err(DaemonError::Internal(format!(
                "mutation applied but failed to persist daemon state: {error}; \
                 daemon state is dirty and will retry persistence on the next mutation"
            )));
        }
        daemon_state.finish_pending_adoption(true);
    }
    outcome
}

#[cfg(test)]
#[path = "executor_tests.rs"]
mod tests;
