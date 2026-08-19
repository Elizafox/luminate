// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Routing already-validated updates to the plugin that owns each device.
//!
//! Nothing here decides whether an update is legal; `DaemonState` has already
//! settled that. This layer's job is finding the owner, chunking calls to
//! respect the plugin-host message limits, and translating an
//! [`ApplyOutcome`] back into the daemon's error vocabulary.

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;

use tokio::sync::Mutex as AsyncMutex;

use luminate_core::appearance_slot::AppearanceSlotValue;
use luminate_core::device::DeviceId;
use luminate_core::effect::Effect;
use luminate_core::frame::FrameEnvelope;
use luminate_core::target::TargetId;
use luminate_plugin_api::{
    PluginFrameUpload, PluginReadRequest, PluginStateSnapshot, PluginUpdate, PluginUpdateOperation,
};

use super::id::LoadedPluginId;
use super::{LoadedPlugin, PluginManager, plugin_target_from_target_id};
use crate::error::DaemonError;
use crate::plugin_host::{ApplyOutcome, MAX_PLUGIN_BATCH_UPDATES, MAX_PLUGIN_READ_TARGETS};
use crate::state::DaemonState;

impl PluginManager {
    pub fn read_state(
        &self,
        device: &DeviceId,
        request: PluginReadRequest,
    ) -> Result<PluginStateSnapshot, DaemonError> {
        let plugin = self.plugin_for_device(device.as_str())?;
        let PluginReadRequest { targets } = request;
        let mut snapshot = PluginStateSnapshot::default();
        for targets in targets.chunks(MAX_PLUGIN_READ_TARGETS) {
            let part = plugin
                .host
                .read_state(PluginReadRequest {
                    targets: targets.to_vec(),
                })
                .map_err(|error| DaemonError::PluginIo {
                    plugin: plugin.metadata.name.clone(),
                    device: device.as_str().to_owned(),
                    diagnostic: format!("state read failed: {error:#}"),
                })?;
            snapshot.observations.extend(part.observations);
            snapshot.errors.extend(part.errors);
        }
        Ok(snapshot)
    }

    pub fn apply_effect(&self, target: &TargetId, effect: &Effect) -> Result<(), DaemonError> {
        self.apply_update(&PluginUpdate {
            target: plugin_target_from_target_id(target),
            operation: PluginUpdateOperation::SetEffect {
                effect: effect.clone(),
            },
        })
    }

    pub fn apply_brightness(&self, target: &TargetId, value: u32) -> Result<(), DaemonError> {
        self.apply_update(&PluginUpdate {
            target: plugin_target_from_target_id(target),
            operation: PluginUpdateOperation::SetBrightness { value },
        })
    }

    pub fn apply_appearance_slots(
        &self,
        target: &TargetId,
        values: &[AppearanceSlotValue],
    ) -> Result<(), DaemonError> {
        self.apply_update(&PluginUpdate {
            target: plugin_target_from_target_id(target),
            operation: PluginUpdateOperation::SetAppearanceSlots {
                values: values.to_vec(),
            },
        })
    }

    pub fn apply_clear(&self, target: &TargetId) -> Result<(), DaemonError> {
        self.apply_update(&PluginUpdate {
            target: plugin_target_from_target_id(target),
            operation: PluginUpdateOperation::Clear,
        })
    }

    pub fn apply_save_current(&self, target: &TargetId) -> Result<(), DaemonError> {
        self.apply_update(&PluginUpdate {
            target: plugin_target_from_target_id(target),
            operation: PluginUpdateOperation::SaveCurrent,
        })
    }

    fn apply_update(&self, update: &PluginUpdate) -> Result<(), DaemonError> {
        let device_id = update.target.device_id();
        let plugin = self.plugin_for_device(device_id)?;
        let outcome = plugin.host.apply(update.clone()).map_err(|error| {
            DaemonError::Internal(format!(
                "plugin host call failed for {}: {error:#}",
                plugin.metadata.name
            ))
        })?;
        apply_outcome_to_result(&plugin.metadata.name, update, outcome)
    }

    /// Forwards one already-validated frame to its owning plugin. Stream
    /// ownership, generation/sequence ordering, and rate limiting are all
    /// resolved by the caller (`DaemonState::record_frame_upload`) before
    /// this is ever called.
    pub fn apply_frame(
        &self,
        target: &TargetId,
        envelope: &FrameEnvelope,
    ) -> Result<(), DaemonError> {
        let device_id = target.device_id().as_str();
        let plugin = self.plugin_for_device(device_id)?;
        let frame_upload = PluginFrameUpload {
            target: plugin_target_from_target_id(target),
            envelope: envelope.clone(),
        };
        let outcome = plugin.host.upload_frame(frame_upload).map_err(|error| {
            DaemonError::Internal(format!(
                "plugin host call failed for {}: {error:#}",
                plugin.metadata.name
            ))
        })?;
        apply_outcome_error(&plugin.metadata.name, device_id, outcome)
    }

    /// Applies one already-validated frame to `target`, reusing the same
    /// generation, sequence, rate-limiting, and shared-memory-fast-path with
    /// pipe-fallback logic as the ordinary `UploadFrame` request handler.
    ///
    /// This is a synchronous method rather than an `async fn` so it can be
    /// called both from `UploadFrame`'s blocking mutation job
    /// (`daemon::dispatch`) and from the dedicated OS thread serving
    /// client-published shared-memory streams
    /// (`crate::plugins::shm_client::ShmClientSubscriberRegistry`). Both
    /// callers may safely use `state.blocking_lock()`: neither runs on a Tokio
    /// reactor thread.
    ///
    /// Returns whether the frame was forwarded to the plugin. `false` means
    /// the frame was intentionally dropped (for example, by rate limiting),
    /// not that an error occurred.
    ///
    /// # Errors
    ///
    /// Returns an error if the owning plugin rejects the frame or the
    /// plugin-host call itself fails.
    pub fn apply_one_frame(
        &self,
        state: &Arc<AsyncMutex<DaemonState>>,
        target: &TargetId,
        envelope: &FrameEnvelope,
    ) -> Result<bool, DaemonError> {
        let should_forward = state
            .blocking_lock()
            .record_frame_upload(target, envelope)?;
        if !should_forward {
            return Ok(false);
        }

        if self.try_apply_shm_frame(target, envelope) {
            return Ok(true);
        }

        self.apply_frame(target, envelope)?;
        Ok(true)
    }

    /// Applies many updates, grouping them by owning plugin so a plugin that
    /// implements `apply_batch_cbor` is called once per plugin rather than
    /// once per update.
    ///
    /// The motivating case is daemon startup replay, which would otherwise
    /// perform one independent hardware transaction for every persisted
    /// target. Plugins that do not implement `apply_batch_cbor` transparently
    /// fall back to the ordinary per-update dispatch in the same order, making
    /// batching purely a plugin-side optimization.
    ///
    /// Results are returned in the same order as `updates`. Updates remain
    /// independent: there is no cross-item atomicity.
    pub fn apply_batch(
        &self,
        updates: &[(TargetId, PluginUpdateOperation)],
    ) -> Vec<Result<(), DaemonError>> {
        self.apply_batch_inner(updates, false)
    }

    /// Applies ordered updates concurrently across their owning plugins.
    ///
    /// Each plugin receives its updates in input order and remains responsible
    /// for serializing access to its host. Only distinct plugin hosts run in
    /// parallel, which preserves backend ordering assumptions while allowing a
    /// multi-plugin aggregate operation to make progress concurrently.
    pub fn apply_batch_parallel(
        &self,
        updates: &[(TargetId, PluginUpdateOperation)],
    ) -> Vec<Result<(), DaemonError>> {
        self.apply_batch_inner(updates, true)
    }

    fn apply_batch_inner(
        &self,
        updates: &[(TargetId, PluginUpdateOperation)],
        parallel: bool,
    ) -> Vec<Result<(), DaemonError>> {
        #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
        let (by_plugin, mut results) = {
            let topology = self.topology.lock().expect("plugin topology lock poisoned");
            group_updates_by_owner(&topology.owner_by_device, updates)
        };

        let loaded = self.loaded_snapshot();
        let mut by_plugin = by_plugin.into_iter().collect::<Vec<_>>();
        by_plugin.sort_by_key(|(plugin_id, _)| *plugin_id);

        let mut batches = Vec::with_capacity(by_plugin.len());
        for (plugin_id, indices) in by_plugin {
            let Some(plugin) = loaded.iter().find(|plugin| plugin.id == plugin_id) else {
                fill_results(&mut results, &indices, || {
                    Err(DaemonError::Internal(format!(
                        "plugin owner {plugin_id:?} is no longer loaded"
                    )))
                });
                continue;
            };

            batches.push((plugin.clone(), indices));
        }

        if parallel && batches.len() > 1 {
            thread::scope(|scope| {
                let mut workers = Vec::with_capacity(batches.len());
                for (plugin, indices) in batches {
                    let worker_indices = indices.clone();
                    let plugin_name = plugin.metadata.name.clone();
                    match thread::Builder::new()
                        .name(format!("scene-apply-{plugin_name}"))
                        .spawn_scoped(scope, move || {
                            apply_plugin_batch(&plugin, &worker_indices, updates)
                        }) {
                        Ok(worker) => workers.push((indices, worker)),
                        Err(error) => fill_results(&mut results, &indices, || {
                            Err(DaemonError::Internal(format!(
                                "failed to start scene worker for {plugin_name}: {error}"
                            )))
                        }),
                    }
                }

                for (indices, worker) in workers {
                    match worker.join() {
                        Ok(outcomes) => store_batch_outcomes(&mut results, &indices, outcomes),
                        Err(_) => fill_results(&mut results, &indices, || {
                            Err(DaemonError::Internal(
                                "scene plugin worker panicked".to_owned(),
                            ))
                        }),
                    }
                }
            });
        } else {
            for (plugin, indices) in batches {
                let outcomes = apply_plugin_batch(&plugin, &indices, updates);
                store_batch_outcomes(&mut results, &indices, outcomes);
            }
        }

        results
            .into_iter()
            .map(|result| {
                result.unwrap_or_else(|| {
                    Err(DaemonError::Internal(
                        "batch result missing for an index".to_owned(),
                    ))
                })
            })
            .collect()
    }
}

fn apply_plugin_batch(
    plugin: &LoadedPlugin,
    indices: &[usize],
    updates: &[(TargetId, PluginUpdateOperation)],
) -> Vec<Result<(), DaemonError>> {
    let plugin_updates = indices
        .iter()
        .filter_map(|&index| {
            updates.get(index).map(|(target, operation)| PluginUpdate {
                target: plugin_target_from_target_id(target),
                operation: operation.clone(),
            })
        })
        .collect::<Vec<_>>();

    let mut outcomes = Vec::with_capacity(plugin_updates.len());
    for chunk in plugin_updates.chunks(MAX_PLUGIN_BATCH_UPDATES) {
        let chunk_outcomes = match plugin.host.apply_batch(chunk.to_vec()) {
            Ok(chunk_outcomes) if chunk_outcomes.len() == chunk.len() => chunk
                .iter()
                .zip(chunk_outcomes)
                .map(|(update, outcome)| {
                    apply_outcome_to_result(&plugin.metadata.name, update, outcome)
                })
                .collect::<Vec<_>>(),
            Ok(_) => chunk
                .iter()
                .map(|_| {
                    Err(DaemonError::Internal(format!(
                        "plugin host returned an invalid batch response for {}",
                        plugin.metadata.name
                    )))
                })
                .collect(),
            Err(error) => {
                let message = format!(
                    "plugin host batch call failed for {}: {error:#}",
                    plugin.metadata.name
                );
                chunk
                    .iter()
                    .map(|_| Err(DaemonError::Internal(message.clone())))
                    .collect()
            }
        };
        outcomes.extend(chunk_outcomes);
    }
    outcomes
}

fn store_batch_outcomes(
    results: &mut [Option<Result<(), DaemonError>>],
    indices: &[usize],
    outcomes: Vec<Result<(), DaemonError>>,
) {
    for (&index, outcome) in indices.iter().zip(outcomes) {
        if let Some(slot) = results.get_mut(index) {
            *slot = Some(outcome);
        }
    }
}

/// Fills every `results[index]` named by `indices` with a fresh error built
/// by `error`, using `.get_mut` throughout to stay clear of direct indexing.
pub(super) fn fill_results(
    results: &mut [Option<Result<(), DaemonError>>],
    indices: &[usize],
    mut error: impl FnMut() -> Result<(), DaemonError>,
) {
    for &index in indices {
        if let Some(slot) = results.get_mut(index) {
            *slot = Some(error());
        }
    }
}

pub(super) fn apply_outcome_to_result(
    plugin_name: &str,
    update: &PluginUpdate,
    outcome: ApplyOutcome,
) -> Result<(), DaemonError> {
    match outcome {
        ApplyOutcome::Applied => {
            tracing::info!(
                plugin = %plugin_name,
                target = %update.target,
                operation = %update.operation.name(),
                "plugin update applied"
            );
            Ok(())
        }
        outcome @ (ApplyOutcome::Unsupported(_)
        | ApplyOutcome::InvalidArgument(_)
        | ApplyOutcome::Io(_)
        | ApplyOutcome::Unavailable(_)
        | ApplyOutcome::RateLimited { .. }
        | ApplyOutcome::Internal(_)) => {
            apply_outcome_error(plugin_name, update.target.device_id(), outcome)
        }
    }
}

pub(super) fn apply_outcome_error(
    plugin_name: &str,
    device: &str,
    outcome: ApplyOutcome,
) -> Result<(), DaemonError> {
    let plugin = plugin_name.to_owned();
    let device = device.to_owned();
    Err(match outcome {
        ApplyOutcome::Applied => return Ok(()),
        ApplyOutcome::Unsupported(diagnostic) => DaemonError::PluginUnsupportedUpdate {
            plugin,
            device,
            diagnostic,
        },
        ApplyOutcome::InvalidArgument(diagnostic) => DaemonError::PluginInvalidUpdate {
            plugin,
            device,
            diagnostic,
        },
        ApplyOutcome::Io(diagnostic) => DaemonError::PluginIo {
            plugin,
            device,
            diagnostic,
        },
        ApplyOutcome::Unavailable(diagnostic) => DaemonError::PluginUnavailable {
            plugin,
            device,
            diagnostic,
        },
        ApplyOutcome::RateLimited {
            diagnostic,
            retry_after_ms,
        } => DaemonError::PluginRateLimited {
            plugin,
            device,
            diagnostic,
            retry_after_ms,
        },
        ApplyOutcome::Internal(diagnostic) => DaemonError::PluginInternal {
            plugin,
            device,
            diagnostic,
        },
    })
}

/// Per-index outcome slots alongside the owner → indices grouping;
/// `group_updates_by_owner`'s return shape.
pub(super) type BatchGrouping = (
    HashMap<LoadedPluginId, Vec<usize>>,
    Vec<Option<Result<(), DaemonError>>>,
);

/// Groups `updates` by owning plugin id, recording an immediate
/// `DeviceUnowned` result for any update whose device has no owner. Pure
/// grouping logic split out from `PluginManager::apply_batch` so it's
/// unit-testable against a plain `owner_by_device` map, without needing a
/// real loaded plugin/`Library`.
pub(super) fn group_updates_by_owner(
    owner_by_device: &HashMap<String, LoadedPluginId>,
    updates: &[(TargetId, PluginUpdateOperation)],
) -> BatchGrouping {
    let mut results: Vec<Option<Result<(), DaemonError>>> = updates.iter().map(|_| None).collect();
    let mut by_plugin: HashMap<LoadedPluginId, Vec<usize>> = HashMap::new();

    for (index, (target, _)) in updates.iter().enumerate() {
        let plugin_target = plugin_target_from_target_id(target);
        let device_id = plugin_target.device_id();
        match owner_by_device.get(device_id) {
            Some(&plugin_id) => by_plugin.entry(plugin_id).or_default().push(index),
            None => {
                if let Some(slot) = results.get_mut(index) {
                    *slot = Some(Err(DaemonError::DeviceUnowned(device_id.to_owned())));
                }
            }
        }
    }

    (by_plugin, results)
}
