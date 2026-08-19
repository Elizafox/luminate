// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Runtime topology lifecycle: rescans, re-pulls, unloads, and reloads.
//!
//! Every method here ends in the same commit sequence: rebuild ownership
//! across all remaining plugins, normalize the aggregate, diff it against the
//! previous devices, then write both halves of the topology under one lock.
//! Ownership is always rebuilt from scratch rather than patched, so a
//! lower-priority contender automatically inherits a device when the previous
//! winner withdraws it. The old snapshot stays active if any step fails.

use anyhow::{Context as _, Result};

use luminate_core::device::DeviceId;
use luminate_core::target::TargetId;
use luminate_plugin_api::{DeviceDescriptor, RescanReason};

use super::catalogue::PluginRuntimeState;
use super::id::LoadedPluginId;
use super::loading::warn_if_adopt_has_no_exact_readback;
use super::ownership::{
    arbitrate_ownership_by_id, changed_device_ids, log_plugin_ownership_summary, owned_descriptors,
};
use super::{LoadedPlugin, LoadedPluginMetadata, PluginManager, TopologyReconcile};
use crate::error::DaemonError;
use crate::normalize;

use std::sync::Arc;

impl PluginManager {
    /// Asks every loaded plugin to discard any cached view of its hardware,
    /// before their topologies are re-pulled.
    ///
    /// Best-effort by design. A plugin whose host is wedged or crashed must
    /// not stop the others from renumerating, and the topology re-pull that
    /// follows is what actually publishes any change. A failure here means
    /// that one plugin may report a stale topology, but does not prevent the
    /// other plugins from rescanning.
    pub fn rescan_all(&self, reason: RescanReason) {
        for plugin in self.loaded_snapshot() {
            if let Err(error) = plugin.host.rescan(reason) {
                tracing::warn!(
                    plugin = %plugin.metadata.name,
                    error = %error,
                    "plugin rescan failed; its topology re-pull may report stale hardware"
                );
            }
        }
    }

    /// Re-pulls and atomically commits one plugin's complete topology.
    ///
    /// Ownership is rebuilt for every loaded plugin so a lower-priority
    /// contender automatically becomes owner when the previous winner
    /// withdraws a device. The old snapshot remains active if pulling,
    /// validation, or aggregate normalization fails.
    pub fn reconcile_plugin_topology(&self, plugin_name: &str) -> Result<TopologyReconcile> {
        let loaded = self.loaded_snapshot();
        let matches = loaded
            .iter()
            .filter(|plugin| plugin.metadata.name == plugin_name)
            .collect::<Vec<_>>();
        anyhow::ensure!(
            !matches.is_empty(),
            "unknown plugin notification source: {plugin_name}"
        );
        anyhow::ensure!(
            matches.len() == 1,
            "plugin notification source is ambiguous: {plugin_name}"
        );
        let plugin = matches
            .into_iter()
            .next()
            .context("validated plugin match disappeared")?;
        let plugin_id = plugin.id;
        let descriptors = plugin.host.topology()?;

        normalize::normalize_devices(&descriptors)
            .with_context(|| format!("plugin {plugin_name} produced an invalid topology"))?;

        warn_if_adopt_has_no_exact_readback(&plugin.metadata, &descriptors);

        #[allow(
            clippy::expect_used,
            clippy::unwrap_in_result,
            reason = "Acceptable to panic on poisoned locks"
        )]
        let mut topology = self.topology.lock().expect("plugin topology lock poisoned");
        let previous_descriptors = owned_descriptors(
            &topology.owner_by_device,
            topology
                .descriptors_by_plugin
                .iter()
                .map(|(id, descriptors)| (*id, descriptors.as_slice())),
        );
        let previous_devices = normalize::normalize_devices(&previous_descriptors)?;

        let mut candidate_descriptors = topology.descriptors_by_plugin.clone();
        let Some(slot) = candidate_descriptors
            .iter_mut()
            .find(|(id, _)| *id == plugin_id)
        else {
            anyhow::bail!("plugin topology entry missing for {plugin_name}");
        };
        slot.1 = descriptors;

        let plugins_for_arbitration = arbitration_inputs(&candidate_descriptors, &loaded);
        let candidate_owners = arbitrate_ownership_by_id(&plugins_for_arbitration);
        let aggregate = owned_descriptors(
            &candidate_owners,
            candidate_descriptors
                .iter()
                .map(|(id, descriptors)| (*id, descriptors.as_slice())),
        );
        let devices = normalize::normalize_devices(&aggregate)?;
        let changed_devices = changed_device_ids(&previous_devices, &devices);

        topology.descriptors_by_plugin = candidate_descriptors;
        topology.owner_by_device = candidate_owners;
        log_plugin_ownership_summary(
            plugins_for_arbitration
                .iter()
                .map(|(id, metadata, descriptors)| {
                    (metadata.name.as_str(), *id, descriptors.as_slice())
                }),
            &topology.owner_by_device,
        );

        Ok(TopologyReconcile {
            devices,
            changed_devices,
        })
    }

    /// Unloads a loaded plugin by name: terminates its host process and
    /// removes it from the loaded set.
    ///
    /// Devices the plugin owned are treated exactly like an ordinary
    /// device withdrawal: the caller commits the returned topology via
    /// [`crate::state::DaemonState::replace_devices_preserving_withdrawn_state`],
    /// which retains `target_state`/observations/`adopted_baseline` for
    /// anything that disappears rather than deleting it, so a later reload
    /// (or an explicit `PurgeWithdrawnDevice`) can decide its fate. There is
    /// no reappearance reconciliation here, since nothing reappears on an
    /// unload.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::PluginNotFound`] if no loaded plugin has this
    /// name.
    #[allow(
        clippy::expect_used,
        clippy::unwrap_in_result,
        reason = "Acceptable to panic on poisoned locks"
    )]
    pub fn unload_plugin(&self, name: &str) -> Result<TopologyReconcile, DaemonError> {
        let removed = {
            let mut loaded = self.loaded.write().expect("plugin loaded lock poisoned");
            let position = loaded
                .iter()
                .position(|plugin| plugin.metadata.name == name)
                .ok_or_else(|| DaemonError::PluginNotFound(name.to_owned()))?;
            loaded.remove(position)
        };
        // `removed`'s only strong reference (baring any transient clone an
        // in-flight call might still hold) is now this local; dropping it at
        // the end of this function shuts its host process down via
        // `HostedPlugin`'s `Drop` impl.

        let remaining = self
            .loaded
            .read()
            .expect("plugin loaded lock poisoned")
            .clone();

        let mut topology = self.topology.lock().expect("plugin topology lock poisoned");
        let previous_descriptors = owned_descriptors(
            &topology.owner_by_device,
            topology
                .descriptors_by_plugin
                .iter()
                .map(|(id, descriptors)| (*id, descriptors.as_slice())),
        );
        let previous_devices =
            normalize::normalize_devices(&previous_descriptors).map_err(|error| {
                DaemonError::Internal(format!("previous topology became invalid: {error:#}"))
            })?;

        let unloaded_descriptors = topology
            .descriptors_by_plugin
            .iter()
            .find(|(id, _)| *id == removed.id)
            .map(|(_, descriptors)| descriptors.clone())
            .unwrap_or_default();
        let candidate_descriptors = topology
            .descriptors_by_plugin
            .iter()
            .filter(|(id, _)| *id != removed.id)
            .cloned()
            .collect::<Vec<_>>();

        let plugins_for_arbitration = arbitration_inputs(&candidate_descriptors, &remaining);
        let candidate_owners = arbitrate_ownership_by_id(&plugins_for_arbitration);
        let aggregate = owned_descriptors(
            &candidate_owners,
            candidate_descriptors
                .iter()
                .map(|(id, descriptors)| (*id, descriptors.as_slice())),
        );
        let devices = normalize::normalize_devices(&aggregate).map_err(|error| {
            DaemonError::Internal(format!(
                "post-unload aggregate topology is invalid: {error:#}"
            ))
        })?;
        let changed_devices = changed_device_ids(&previous_devices, &devices);

        topology.descriptors_by_plugin = candidate_descriptors;
        topology.owner_by_device = candidate_owners;
        log_plugin_ownership_summary(
            plugins_for_arbitration
                .iter()
                .map(|(id, metadata, descriptors)| {
                    (metadata.name.as_str(), *id, descriptors.as_slice())
                }),
            &topology.owner_by_device,
        );
        drop(topology);

        let unloaded_targets = unloaded_descriptors
            .iter()
            .map(|descriptor| TargetId::Device(DeviceId::new(descriptor.id.clone())))
            .collect::<Vec<_>>();
        self.end_all_shm_streams(&unloaded_targets);
        let _ = self.end_all_shm_client_streams(&unloaded_targets);
        self.set_runtime_state(name, PluginRuntimeState::Inactive);

        tracing::info!(plugin = %name, "plugin unloaded");
        Ok(TopologyReconcile {
            devices,
            changed_devices,
        })
    }

    /// Reloads a loaded plugin by name: terminates its host process and
    /// starts a fresh one at the same path and configuration, returning the
    /// resulting topology reconciliation for the caller to commit
    /// immediately.
    ///
    /// The caller should also schedule a follow-up rescan (see
    /// [`super::RescanRequester`]) so devices the reloaded plugin still owns
    /// get a full reappearance-reconciliation pass, the same way resume-from-
    /// suspend restores hardware state even when the topology comes back
    /// unchanged (a plugin can reload identically but still need its
    /// hardware state re-driven).
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::PluginNotFound`] if no loaded plugin has this
    /// name, or [`DaemonError::PluginReloadFailed`] if the replacement host
    /// failed to start or reported different identity metadata. Either way,
    /// the plugin's old host process has already been shut down as part of
    /// the attempt; on failure the plugin is left unloaded rather than
    /// restored to its prior state, and the topology reflects that removal,
    /// though this method reports the error rather than that topology
    /// change. It is corrected by the next rescan.
    #[allow(
        clippy::expect_used,
        clippy::unwrap_in_result,
        reason = "Acceptable to panic on poisoned locks"
    )]
    pub fn reload_plugin(&self, name: &str) -> Result<TopologyReconcile, DaemonError> {
        let old_plugin = self
            .loaded_snapshot()
            .into_iter()
            .find(|plugin| plugin.metadata.name == name)
            .ok_or_else(|| DaemonError::PluginNotFound(name.to_owned()))?;
        let old_id = old_plugin.id;

        let reload_outcome = old_plugin.host.force_reload().and_then(|descriptors| {
            normalize::normalize_devices(&descriptors)?;
            Ok(descriptors)
        });

        self.loaded
            .write()
            .expect("plugin loaded lock poisoned")
            .retain(|plugin| plugin.id != old_id);

        let descriptors = reload_outcome.map_err(|error| {
            let diagnostic = format!("{error:#}");
            self.set_runtime_state(name, PluginRuntimeState::Failed(diagnostic.clone()));
            DaemonError::PluginReloadFailed {
                plugin: name.to_owned(),
                diagnostic,
            }
        })?;

        let new_plugin = LoadedPlugin {
            id: LoadedPluginId::new(),
            metadata: old_plugin.metadata.clone(),
            host: Arc::clone(&old_plugin.host),
        };
        let new_id = new_plugin.id;

        self.loaded
            .write()
            .expect("plugin loaded lock poisoned")
            .push(new_plugin.clone());
        self.set_runtime_state(name, PluginRuntimeState::Loaded);

        let remaining = self
            .loaded
            .read()
            .expect("plugin loaded lock poisoned")
            .clone();

        let mut topology = self.topology.lock().expect("plugin topology lock poisoned");
        let previous_descriptors = owned_descriptors(
            &topology.owner_by_device,
            topology
                .descriptors_by_plugin
                .iter()
                .map(|(id, descriptors)| (*id, descriptors.as_slice())),
        );
        let previous_devices =
            normalize::normalize_devices(&previous_descriptors).map_err(|error| {
                DaemonError::Internal(format!("previous topology became invalid: {error:#}"))
            })?;

        let mut candidate_descriptors = topology
            .descriptors_by_plugin
            .iter()
            .filter(|(id, _)| *id != old_id)
            .cloned()
            .collect::<Vec<_>>();
        candidate_descriptors.push((new_id, descriptors));

        let plugins_for_arbitration = arbitration_inputs(&candidate_descriptors, &remaining);
        let candidate_owners = arbitrate_ownership_by_id(&plugins_for_arbitration);
        let aggregate = owned_descriptors(
            &candidate_owners,
            candidate_descriptors
                .iter()
                .map(|(id, descriptors)| (*id, descriptors.as_slice())),
        );
        let devices = normalize::normalize_devices(&aggregate).map_err(|error| {
            DaemonError::Internal(format!(
                "post-reload aggregate topology is invalid: {error:#}"
            ))
        })?;
        let changed_devices = changed_device_ids(&previous_devices, &devices);

        topology.descriptors_by_plugin = candidate_descriptors;
        topology.owner_by_device = candidate_owners;
        log_plugin_ownership_summary(
            plugins_for_arbitration
                .iter()
                .map(|(id, metadata, descriptors)| {
                    (metadata.name.as_str(), *id, descriptors.as_slice())
                }),
            &topology.owner_by_device,
        );

        tracing::info!(plugin = %name, "plugin reloaded");
        Ok(TopologyReconcile {
            devices,
            changed_devices,
        })
    }

    /// Reloads every currently loaded plugin at its existing path and
    /// configuration, logging and continuing past any individual failure.
    ///
    /// This is the config/plugin-reload story `SIGHUP` triggers: it does
    /// not re-read `luminated`'s configuration file for added or removed
    /// plugin entries, only restarts what's already loaded.
    pub fn reload_all(&self) -> Vec<(String, Result<TopologyReconcile, DaemonError>)> {
        let names = self.plugin_names();
        names
            .into_iter()
            .map(|name| {
                let result = self.reload_plugin(&name);
                if let Err(error) = &result {
                    tracing::warn!(plugin = %name, error = %error, "plugin reload failed");
                }
                (name, result)
            })
            .collect()
    }
}

/// Pairs each candidate plugin's descriptors back up with its live metadata,
/// dropping entries whose plugin is no longer loaded.
///
/// [`arbitrate_ownership_by_id`] needs metadata (for priority) alongside the
/// descriptors, but the topology state stores only descriptors keyed by id.
/// All three commit paths rebuild that pairing identically.
fn arbitration_inputs(
    candidate_descriptors: &[(LoadedPluginId, Vec<DeviceDescriptor>)],
    loaded: &[LoadedPlugin],
) -> Vec<(LoadedPluginId, LoadedPluginMetadata, Vec<DeviceDescriptor>)> {
    candidate_descriptors
        .iter()
        .filter_map(|(id, descriptors)| {
            loaded
                .iter()
                .find(|plugin| plugin.id == *id)
                .map(|plugin| (*id, plugin.metadata.clone(), descriptors.clone()))
        })
        .collect()
}
