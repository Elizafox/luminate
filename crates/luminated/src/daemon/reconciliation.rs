// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Startup and topology-change device reconciliation.

use super::*;

pub(super) async fn run_startup_reconciliation(
    state: &Arc<Mutex<DaemonState>>,
    plugin_manager: &Arc<PluginManager>,
    state_path: &Arc<Path>,
    config: &DaemonConfig,
) -> anyhow::Result<()> {
    // Plugin reads are synchronous; keep them off the async runtime while
    // still completing reconciliation before clients are accepted.
    let state = Arc::clone(state);
    let plugin_manager = Arc::clone(plugin_manager);
    let state_path = Arc::clone(state_path);
    let config = config.clone();
    task::spawn_blocking(move || {
        startup_reconcile(&state, &plugin_manager, &state_path, &config);
    })
    .await
    .context("startup reconciliation task failed")
}

/// Reconciles one device at a time before clients can mutate it. Policy comes
/// from configuration/plugin recommendation; readback capability only says
/// what hardware can report, never which side is authoritative.
fn startup_reconcile(
    state: &Arc<Mutex<DaemonState>>,
    plugin_manager: &PluginManager,
    state_path: &Path,
    config: &DaemonConfig,
) {
    let devices = state.blocking_lock().devices();
    let mut adoption_candidates = Vec::new();
    for device in devices {
        let policy = effective_reconciliation_policy(config, plugin_manager, &device);
        let candidate = match reconcile_device(state, plugin_manager, &device.id, policy) {
            Ok(candidate) => candidate,
            Err(error) => {
                tracing::warn!(device = %device.id, error = %error, "startup reconciliation failed");
                continue;
            }
        };
        if !candidate.is_empty() {
            adoption_candidates.push((device.id, candidate));
        }
    }

    if adoption_candidates.is_empty() {
        return;
    }

    let combined = adoption_candidates
        .iter()
        .flat_map(|(_, candidate)| candidate.iter().cloned())
        .collect::<Vec<_>>();
    let mut daemon_state = state.blocking_lock();
    let merged = daemon_state.merged_adopted_baseline(&combined);
    match persistence::save_complete(
        state_path,
        daemon_state.target_states(),
        daemon_state.collections(),
        daemon_state.scenes(),
        &merged,
    ) {
        Ok(()) => {
            for (_, candidate) in &adoption_candidates {
                daemon_state.commit_adopted_baseline(candidate);
            }
        }
        Err(error) => {
            let diagnostic = format!("confirmed state could not be adopted durably: {error}");
            for (device, candidate) in &adoption_candidates {
                daemon_state.fail_adoption_persistence(device, candidate, diagnostic.clone());
            }
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "The restore/adopt/leave policies share sequencing state (verification, failure tracking) that would be awkward to split without threading it back together."
)]
pub(super) fn reconcile_device(
    state: &Arc<Mutex<DaemonState>>,
    plugin_manager: &PluginManager,
    device: &device::DeviceId,
    policy: ReconciliationPolicy,
) -> Result<Vec<state::AdoptedFacet>, DaemonError> {
    state.blocking_lock().begin_reconciliation(device);
    let mut verification = Vec::new();
    if policy == ReconciliationPolicy::Restore {
        let selected = HashSet::from([device]);
        let pending = {
            let state = state.blocking_lock();
            pending_cached_state(&state, Some(&selected))
        };
        verification.clone_from(&pending);
        let resolved_pending = {
            let state = state.blocking_lock();
            pending
                .iter()
                .map(|(target, operation)| {
                    (
                        target.clone(),
                        resolve_operation_colour(&state, target, operation.clone()),
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut failed = false;
        for ((target, operation), result) in pending
            .iter()
            .zip(plugin_manager.apply_batch(&resolved_pending))
        {
            match result {
                Ok(()) => state
                    .blocking_lock()
                    .note_successful_operation(target, operation),
                Err(error) => {
                    failed = true;
                    tracing::warn!(target = ?target, error = %error, "restore operation failed");
                }
            }
        }
        // Write-through targets were skipped by `pending_cached_state` above
        // because the hardware still durably holds them, but the in-memory
        // observation cache does not survive a daemon restart the way the
        // persisted target-state cache does. Re-establish the observation
        // without re-writing hardware, or a write-through target reads back
        // as unknown until its value is next changed.
        let write_through = {
            let state = state.blocking_lock();
            write_through_cached_state(&state, &selected)
        };
        {
            let mut state = state.blocking_lock();
            for (target, operation) in &write_through {
                state.note_successful_operation(target, operation);
            }
        }

        if failed {
            let diagnostic = "one or more restore operations failed".to_owned();
            state
                .blocking_lock()
                .fail_reconciliation(device, diagnostic.clone());
            return Err(DaemonError::Internal(diagnostic));
        }
    }

    let (request, generation) = {
        let state = state.blocking_lock();
        (state.read_request(device), state.generation(device))
    };
    if request.targets.is_empty() {
        if policy == ReconciliationPolicy::Adopt {
            let diagnostic = "adopt requested but the device has no exact readback".to_owned();
            state
                .blocking_lock()
                .fail_reconciliation(device, diagnostic.clone());
            return Err(DaemonError::Internal(diagnostic));
        }
        state.blocking_lock().complete_reconciliation(device);
        return Ok(Vec::new());
    }
    let snapshot = plugin_manager
        .read_state(device, request)
        .inspect_err(|error| {
            state
                .blocking_lock()
                .fail_reconciliation(device, error.to_string());
        })?;
    let result = state.blocking_lock().accept_snapshot(
        device,
        generation,
        snapshot,
        policy == ReconciliationPolicy::Adopt,
    );
    if let Err(error) = &result {
        state
            .blocking_lock()
            .fail_reconciliation(device, error.to_string());
    }
    if result.is_ok()
        && policy == ReconciliationPolicy::Restore
        && !state
            .blocking_lock()
            .confirmed_observations_match(&verification)
    {
        let diagnostic = "restore verification disagreed with desired state".to_owned();
        state
            .blocking_lock()
            .mark_reconciliation_drifted(device, diagnostic.clone());
        return Err(DaemonError::Internal(diagnostic));
    }
    result
}

pub(super) fn effective_reconciliation_policy(
    config: &DaemonConfig,
    plugin_manager: &PluginManager,
    device: &device::Device,
) -> ReconciliationPolicy {
    let device_policy = config.device_reconciliation.get(&device.id).copied();
    let plugin_policy = plugin_manager.configured_reconciliation(&device.id);
    select_reconciliation_policy(
        device_policy,
        plugin_policy,
        config.reconciliation_policy,
        plugin_manager.recommended_reconciliation(&device.id),
    )
}

fn select_reconciliation_policy(
    device: Option<ReconciliationPolicy>,
    plugin: Option<ReconciliationPolicy>,
    global: Option<ReconciliationPolicy>,
    recommendation: Option<ReconciliationPolicy>,
) -> ReconciliationPolicy {
    // Explicit configuration narrows from device outward; plugin advice is a default only.
    device
        .or(plugin)
        .or(global)
        .or(recommendation)
        .unwrap_or(ReconciliationPolicy::Leave)
}

fn pending_cached_state(
    state: &DaemonState,
    devices: Option<&HashSet<&device::DeviceId>>,
) -> Vec<(TargetId, luminate_plugin_api::PluginUpdateOperation)> {
    // Adopted facets are replayed first; explicit desired entries remain
    // above them in the existing authority model.
    let mut pending = state
        .adopted_baseline()
        .iter()
        .filter(|facet| {
            state.capabilities_for_target(&facet.target).is_some()
                && devices.is_none_or(|devices| devices.contains(facet.target.device_id()))
        })
        .filter_map(|facet| {
            let operation = match &facet.value {
                FacetValue::Appearance(AppearanceState::Static(colour)) => {
                    luminate_plugin_api::PluginUpdateOperation::SetEffect {
                        effect: Effect::Static {
                            colour: colour.clone(),
                        },
                    }
                }
                FacetValue::Appearance(AppearanceState::Effect(effect)) => {
                    luminate_plugin_api::PluginUpdateOperation::SetEffect {
                        effect: effect.clone(),
                    }
                }
                FacetValue::Brightness(value) => {
                    luminate_plugin_api::PluginUpdateOperation::SetBrightness { value: *value }
                }
                FacetValue::AppearanceSlots(slots) => {
                    luminate_plugin_api::PluginUpdateOperation::SetAppearanceSlots {
                        values: slots.values.clone(),
                    }
                }
                FacetValue::Emission(EmissionState::Dark)
                | FacetValue::PhysicalPower(PhysicalPowerState::Off) => {
                    luminate_plugin_api::PluginUpdateOperation::SetEffect {
                        effect: Effect::Off,
                    }
                }
                // `EffectiveAppearance` is daemon-synthesized only and never
                // part of an adopted baseline.
                FacetValue::Emission(EmissionState::Emitting)
                | FacetValue::PhysicalPower(PhysicalPowerState::On)
                | FacetValue::Appearance(AppearanceState::Mixed)
                | FacetValue::EffectiveAppearance(_) => return None,
            };
            Some((facet.target.clone(), operation))
        })
        .collect::<Vec<_>>();

    pending.extend(state
        .target_states()
        .iter()
        .filter(|entry| {
            let target = &entry.target;
            if state.capabilities_for_target(target).is_none()
                || devices.is_some_and(|devices| !devices.contains(target.device_id()))
            {
                return false;
            }
            // Write-through hardware already durably holds whatever was
            // last written, so re-pushing the cached value on every daemon
            // restart is a needless write (real flash/EEPROM writes are not
            // free and are often wear-limited), not a correctness fix.
            let persistence_is_required = persistence_requirement_for_target(state, target)
                == Some(PersistenceRequirement::Required);

            if persistence_is_required {
                tracing::debug!(target = ?target, "restore replay skipped: hardware persistence is required, cached state is already durable");
            }

            !persistence_is_required
        })
        .map(|entry| {
            (
                entry.target.clone(),
                operation_for_target_state(&entry.state),
            )
        })
        .collect::<Vec<_>>());
    pending
}

/// The write-through counterpart to `pending_cached_state`'s target-state
/// half: entries `pending_cached_state` skips resending to hardware because
/// their persistence is `Required`. The daemon still needs these recorded as
/// known state after a restart, since only the persisted target-state cache
/// (not the in-memory observation cache that `GetState` reads from) survives
/// process restarts.
fn write_through_cached_state(
    state: &DaemonState,
    devices: &HashSet<&device::DeviceId>,
) -> Vec<(TargetId, luminate_plugin_api::PluginUpdateOperation)> {
    state
        .target_states()
        .iter()
        .filter(|entry| {
            let target = &entry.target;
            state.capabilities_for_target(target).is_some()
                && devices.contains(target.device_id())
                && persistence_requirement_for_target(state, target)
                    == Some(PersistenceRequirement::Required)
        })
        .map(|entry| {
            (
                entry.target.clone(),
                operation_for_target_state(&entry.state),
            )
        })
        .collect()
}

/// Looks up whether `target`'s persistence capability is `Required`
/// (write-through: every mutation is already durable on the device) versus
/// `Optional`/absent. Shared by startup reconciliation's skip logic and
/// `SaveCurrent`'s dispatch.
pub(super) fn persistence_requirement_for_target(
    state: &DaemonState,
    target: &TargetId,
) -> Option<PersistenceRequirement> {
    match state.capabilities_for_target(target)?.persistence {
        PersistenceCapability::None => None,
        PersistenceCapability::CurrentState { requirement, .. }
        | PersistenceCapability::Profiles { requirement, .. } => Some(requirement),
    }
}

/// Resolves a replayed Static operation's payload the same way a live
/// colour request does, so a persisted `Cct` colour gets emulated to
/// `Additive` RGB before being sent to a plugin that lacks native CCT
/// support. Falls back to the original colour if resolution fails, so the
/// plugin's own rejection still surfaces as the restore-operation error.
fn resolve_operation_colour(
    state: &DaemonState,
    target: &TargetId,
    operation: luminate_plugin_api::PluginUpdateOperation,
) -> luminate_plugin_api::PluginUpdateOperation {
    match operation {
        luminate_plugin_api::PluginUpdateOperation::SetEffect {
            effect: Effect::Static { colour },
        } => {
            let resolved = state.resolve_colour(target, &colour).unwrap_or(colour);
            luminate_plugin_api::PluginUpdateOperation::SetEffect {
                effect: Effect::Static { colour: resolved },
            }
        }
        other @ (luminate_plugin_api::PluginUpdateOperation::SetEffect { .. }
        | luminate_plugin_api::PluginUpdateOperation::SetAppearanceSlots { .. }
        | luminate_plugin_api::PluginUpdateOperation::SetBrightness { .. }
        | luminate_plugin_api::PluginUpdateOperation::Clear
        | luminate_plugin_api::PluginUpdateOperation::SaveCurrent) => other,
    }
}

/// Converts persisted state into the plugin-facing operation that would
/// reproduce it, for startup reconciliation's batched replay.
fn operation_for_target_state(
    target_state: &TargetState,
) -> luminate_plugin_api::PluginUpdateOperation {
    match target_state {
        TargetState::Effect(effect) => luminate_plugin_api::PluginUpdateOperation::SetEffect {
            effect: effect.clone(),
        },
        TargetState::Brightness(value) => {
            luminate_plugin_api::PluginUpdateOperation::SetBrightness { value: *value }
        }
        TargetState::AppearanceSlots(values) => {
            luminate_plugin_api::PluginUpdateOperation::SetAppearanceSlots {
                values: values.clone(),
            }
        }
        TargetState::Clear => luminate_plugin_api::PluginUpdateOperation::Clear,
    }
}

#[cfg(test)]
#[path = "reconciliation_tests.rs"]
mod tests;
