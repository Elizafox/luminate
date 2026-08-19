// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Reconciling loaded plugin hosts with committed managed configuration.
//!
//! Everything here is driven by the catalogue rather than by hardware: once
//! an administrator's managed revision is committed, these methods work out
//! which plugins should now be running and move each one towards that state.
//! Individual failures are recorded against the catalogue entry and never
//! roll back the committed configuration, because the desired state is
//! already durable by the time reconciliation starts.

use anyhow::{Context as _, Result};

use luminate_protocol::{ManagedPlugin, PluginSetupWorkflow};

use super::catalogue::PluginRuntimeState;
use super::loading::{load_plugin, validate_startup_candidate};
use super::ownership::{changed_device_ids, owned_descriptors};
use super::{ManagedActivationReconcile, PluginManager, TopologyReconcile};
use crate::device_config::DaemonConfig;
use crate::managed_config::{ManageablePlugin, ManagedConfig};
use crate::normalize;

use std::collections::HashSet;

impl PluginManager {
    /// Returns setup workflows for one installed plugin.
    ///
    /// The plugin ABI does not expose workflow metadata yet, so every known
    /// plugin currently returns an empty list. Keeping this lookup on the
    /// catalogue establishes absence as the default without treating an
    /// unknown plugin as an empty-capability plugin.
    #[must_use]
    #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
    pub fn setup_workflows(&self, name: &str) -> Option<Vec<PluginSetupWorkflow>> {
        self.catalogue
            .read()
            .expect("plugin catalogue lock poisoned")
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.setup_workflows.clone())
    }

    /// Returns the current management-facing plugin catalogue in deterministic
    /// discovery order.
    #[must_use]
    #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
    pub fn management_plugins(&self) -> Vec<ManagedPlugin> {
        self.catalogue
            .read()
            .expect("plugin catalogue lock poisoned")
            .iter()
            .map(|entry| entry.management.clone())
            .collect()
    }

    /// Returns the installed metadata needed to validate a management patch.
    #[must_use]
    #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
    pub fn manageable_plugins(&self) -> Vec<ManageablePlugin> {
        self.catalogue
            .read()
            .expect("plugin catalogue lock poisoned")
            .iter()
            .map(|entry| ManageablePlugin {
                name: entry.name.clone(),
                global_settings: entry.global_settings.clone(),
                locked_settings: entry.locked_settings.clone(),
                schema: entry.validation_schema.clone(),
            })
            .collect()
    }

    /// Updates the management-facing desired and effective configuration after
    /// a managed revision has been committed.
    ///
    /// # Errors
    ///
    /// Returns an error if the committed configuration cannot be merged with
    /// the inspected plugin schemas.
    #[allow(
        clippy::expect_used,
        clippy::unwrap_in_result,
        reason = "Acceptable to panic on poisoned locks"
    )]
    pub fn apply_managed_config(
        &self,
        global: &DaemonConfig,
        managed: &ManagedConfig,
    ) -> Result<Vec<String>> {
        let mut configuration_changes = Vec::new();

        for entry in self
            .catalogue
            .write()
            .expect("plugin catalogue lock poisoned")
            .iter_mut()
        {
            let previous_configuration = entry.configuration_cbor.clone();
            entry.apply_managed(global, managed)?;
            if entry.configuration_cbor != previous_configuration {
                configuration_changes.push(entry.name.clone());
            }
        }

        Ok(configuration_changes)
    }

    /// Reconciles plugin hosts with committed managed activation and
    /// restart-required setting changes. Individual failures remain recorded
    /// in the catalogue and do not roll back the committed desired
    /// configuration.
    #[must_use]
    pub fn reconcile_managed_plugins(
        &self,
        configuration_changes: &[String],
    ) -> ManagedActivationReconcile {
        let actions = {
            #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
            let catalogue = self
                .catalogue
                .read()
                .expect("plugin catalogue lock poisoned");
            catalogue
                .iter()
                .filter_map(|entry| {
                    managed_plugin_action(
                        &entry.runtime,
                        entry.effective_enabled,
                        configuration_changes.contains(&entry.name),
                    )
                    .map(|action| (entry.name.clone(), action))
                })
                .collect::<Vec<_>>()
        };
        let mut latest = None;
        let mut changed = HashSet::new();
        let mut activated = false;

        for (name, action) in actions {
            let outcome = run_managed_plugin_action(
                action,
                || self.activate_plugin(&name),
                || self.unload_plugin(&name).map_err(anyhow::Error::from),
            );
            for reconciled in outcome.reconciliations {
                changed.extend(reconciled.changed_devices);
                latest = Some(reconciled.devices);
            }
            activated |= outcome.activated;

            if let Some(error) = outcome.error {
                let diagnostic = format!("{error:#}");
                self.set_runtime_state(&name, PluginRuntimeState::Failed(diagnostic.clone()));
                tracing::warn!(
                    plugin = %name,
                    ?action,
                    error = %diagnostic,
                    "managed plugin runtime reconciliation failed"
                );
            }
        }

        ManagedActivationReconcile {
            topology: latest.map(|devices| {
                let mut changed_devices = changed.into_iter().collect::<Vec<_>>();
                changed_devices.sort_by(|left, right| left.as_str().cmp(right.as_str()));
                TopologyReconcile {
                    devices,
                    changed_devices,
                }
            }),
            activated,
        }
    }

    #[allow(
        clippy::expect_used,
        clippy::unwrap_in_result,
        reason = "Acceptable to panic on poisoned locks"
    )]
    fn activate_plugin(&self, name: &str) -> Result<TopologyReconcile> {
        let (path, configuration) = self
            .catalogue
            .read()
            .expect("plugin catalogue lock poisoned")
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| (entry.path.clone(), entry.configuration_cbor.clone()))
            .with_context(|| format!("installed plugin not found: {name}"))?;
        self.set_runtime_state(name, PluginRuntimeState::Loading);

        let (plugin, descriptors) = load_plugin(&path, configuration, self.operator_events)?;
        anyhow::ensure!(
            plugin.metadata.name == name,
            "plugin identity changed after inspection: expected {name}, loaded {}",
            plugin.metadata.name
        );

        let mut loaded = self.loaded.write().expect("plugin loaded lock poisoned");
        anyhow::ensure!(
            loaded
                .iter()
                .all(|candidate| candidate.metadata.name != name),
            "plugin is already loaded: {name}"
        );
        let mut topology = self.topology.lock().expect("plugin topology lock poisoned");
        let previous_descriptors = owned_descriptors(
            &topology.owner_by_device,
            topology
                .descriptors_by_plugin
                .iter()
                .map(|(id, descriptors)| (*id, descriptors.as_slice())),
        );
        let previous_devices = normalize::normalize_devices(&previous_descriptors)
            .context("previous topology became invalid")?;
        let existing = topology
            .descriptors_by_plugin
            .iter()
            .filter_map(|(id, descriptors)| {
                loaded
                    .iter()
                    .find(|candidate| candidate.id == *id)
                    .map(|candidate| (*id, candidate.metadata.clone(), descriptors.as_slice()))
            })
            .collect::<Vec<_>>();
        let existing_ids = existing.iter().map(|(id, _, _)| *id).collect::<Vec<_>>();
        let existing_metadata = existing
            .iter()
            .map(|(_, metadata, _)| metadata.clone())
            .collect::<Vec<_>>();
        let existing_descriptors = existing
            .iter()
            .map(|(_, _, descriptors)| (*descriptors).to_vec())
            .collect::<Vec<_>>();
        let candidate_owners = validate_startup_candidate(
            &existing_metadata,
            &existing_ids,
            &existing_descriptors,
            plugin.id,
            &plugin.metadata,
            &descriptors,
        )?;
        let mut candidate_descriptors = topology.descriptors_by_plugin.clone();
        candidate_descriptors.push((plugin.id, descriptors));
        let aggregate = owned_descriptors(
            &candidate_owners,
            candidate_descriptors
                .iter()
                .map(|(id, descriptors)| (*id, descriptors.as_slice())),
        );
        let devices =
            normalize::normalize_devices(&aggregate).context("activated topology is invalid")?;
        let changed_devices = changed_device_ids(&previous_devices, &devices);

        topology.descriptors_by_plugin = candidate_descriptors;
        topology.owner_by_device = candidate_owners;
        loaded.push(plugin);
        drop(topology);
        drop(loaded);
        self.set_runtime_state(name, PluginRuntimeState::Loaded);
        tracing::info!(plugin = %name, "plugin activated from managed configuration");

        Ok(TopologyReconcile {
            devices,
            changed_devices,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ManagedPluginAction {
    Activate,
    Deactivate,
    Restart,
}

pub(super) struct ManagedPluginActionOutcome {
    pub(super) reconciliations: Vec<TopologyReconcile>,
    pub(super) activated: bool,
    pub(super) error: Option<anyhow::Error>,
}

pub(super) fn run_managed_plugin_action(
    action: ManagedPluginAction,
    activate: impl FnOnce() -> Result<TopologyReconcile>,
    deactivate: impl FnOnce() -> Result<TopologyReconcile>,
) -> ManagedPluginActionOutcome {
    let mut reconciliations = Vec::new();
    let result = match action {
        ManagedPluginAction::Activate => activate(),
        ManagedPluginAction::Deactivate => deactivate(),
        ManagedPluginAction::Restart => deactivate().and_then(|reconciled| {
            reconciliations.push(reconciled);
            activate()
        }),
    };
    let activated = result.is_ok()
        && matches!(
            action,
            ManagedPluginAction::Activate | ManagedPluginAction::Restart
        );
    let error = match result {
        Ok(reconciled) => {
            reconciliations.push(reconciled);
            None
        }
        Err(error) => Some(error),
    };

    ManagedPluginActionOutcome {
        reconciliations,
        activated,
        error,
    }
}

pub(super) fn managed_plugin_action(
    runtime: &PluginRuntimeState,
    effective_enabled: bool,
    configuration_changed: bool,
) -> Option<ManagedPluginAction> {
    match (runtime, effective_enabled, configuration_changed) {
        (PluginRuntimeState::Loaded, false, _) => Some(ManagedPluginAction::Deactivate),
        (PluginRuntimeState::Inactive | PluginRuntimeState::Failed(_), true, _) => {
            Some(ManagedPluginAction::Activate)
        }
        (PluginRuntimeState::Loaded, true, true) => Some(ManagedPluginAction::Restart),
        (PluginRuntimeState::Loading, _, _)
        | (PluginRuntimeState::Loaded, true, false)
        | (PluginRuntimeState::Inactive | PluginRuntimeState::Failed(_), false, _) => None,
    }
}
