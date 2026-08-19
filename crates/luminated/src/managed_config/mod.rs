// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Strict, revisioned persistence for daemon-managed desired configuration.
//!
//! `managed.toml` is the machine-wide desired state an administrator sets and
//! the daemon rewrites. This file holds the stored shape itself; the work of
//! validating, merging, and persisting it lives in sibling modules:
//!
//! - `schema`: what a plugin declares its settings may contain
//! - `patch`: staging and committing one management transaction
//! - `preferences`: merging daemon preferences across layers
//! - `settings`: resolving and checking a plugin's settings table
//! - `persistence`: reading and atomically rewriting the file

use std::collections::{HashMap, HashSet};
use std::result::Result as StdResult;

use anyhow::{Context as _, Result};
use luminate_core::capability::CctEmulation;
use luminate_core::control::ReconciliationPolicy;
use luminate_core::device::DeviceId;
use luminate_protocol::{ManagedPlugin, ManagementChangeSet, ManagementPatch, ManagementSnapshot};
use serde::{Deserialize, Serialize};

use patch::{apply_mutation, validate_managed_plugin_settings};
use preferences::{daemon_preferences_from_config, daemon_preferences_from_managed};

use crate::device_config::DaemonConfig;

mod patch;
mod persistence;
mod preferences;
mod schema;
mod settings;

pub use patch::{PrepareManagementPatchError, PreparedManagementPatch};
pub use persistence::load;
pub use preferences::{merge_daemon_preferences, plugin_is_enabled};
pub use schema::{ManageablePlugin, PluginActivationInputs, PluginSettingSchema};
pub use settings::merge_plugin_settings;

/// Machine-wide desired configuration owned and rewritten by the daemon.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ManagedConfig {
    /// Monotonically increasing optimistic-concurrency revision.
    pub revision: u64,

    /// Safe runtime daemon preferences.
    pub daemon: ManagedDaemonConfig,

    /// Desired activation and settings by canonical plugin name.
    pub plugins: Vec<ManagedPluginConfig>,
}

/// Runtime-safe daemon preference overrides.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ManagedDaemonConfig {
    pub default_unsupported_policy: Option<luminate_protocol::UnsupportedPolicy>,
    pub reconciliation_policy: Option<ReconciliationPolicy>,
    pub device_reconciliation: HashMap<DeviceId, ReconciliationPolicy>,
    pub cct_emulation: Option<CctEmulation>,
    pub prefer_shm: Option<bool>,
    pub prefer_client_shm: Option<bool>,
}

/// Desired state for one installed plugin.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ManagedPluginConfig {
    pub name: String,
    pub enabled: Option<bool>,
    pub reconciliation: Option<ReconciliationPolicy>,
    pub settings: toml::Table,
}

impl ManagedConfig {
    /// Builds the management-facing snapshot from the stored layer, global
    /// policy, and current plugin catalogue.
    #[must_use]
    pub fn snapshot(
        &self,
        global: &DaemonConfig,
        plugins: Vec<ManagedPlugin>,
    ) -> ManagementSnapshot {
        let effective = merge_daemon_preferences(global, &self.daemon);

        ManagementSnapshot {
            revision: self.revision,
            desired_daemon: daemon_preferences_from_managed(&self.daemon),
            effective_daemon: daemon_preferences_from_config(&effective),
            locked_daemon_settings: global.management.locked_daemon_settings.clone(),
            plugins,
        }
    }

    /// Validates invariants independent of plugin metadata.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty or duplicate canonical plugin name.
    pub fn validate(&self) -> Result<()> {
        let mut device_ids: Vec<_> = self.daemon.device_reconciliation.keys().collect();
        device_ids.sort_by_key(|device| device.as_str());
        for device in device_ids {
            anyhow::ensure!(
                !device.as_str().trim().is_empty(),
                "daemon.device_reconciliation keys must not be empty"
            );
        }

        let mut names = HashSet::new();
        for (index, plugin) in self.plugins.iter().enumerate() {
            anyhow::ensure!(
                !plugin.name.trim().is_empty(),
                "plugins[{index}].name must not be empty"
            );
            anyhow::ensure!(
                names.insert(plugin.name.as_str()),
                "duplicate managed plugin entry for {}",
                plugin.name
            );
        }
        Ok(())
    }

    /// Validates an optimistic-concurrency patch against installed metadata.
    ///
    /// The returned transaction does not alter this configuration or write to
    /// disk until [`PreparedManagementPatch::commit`] succeeds.
    ///
    /// # Errors
    ///
    /// Returns an error when the expected revision is stale, the revision is
    /// exhausted, a plugin or setting is unknown, or a setting value violates
    /// its schema.
    pub fn prepare_patch(
        &self,
        patch: ManagementPatch,
        plugins: &[ManageablePlugin],
    ) -> StdResult<PreparedManagementPatch, PrepareManagementPatchError> {
        if patch.expected_revision != self.revision {
            return Err(PrepareManagementPatchError::Conflict {
                expected_revision: patch.expected_revision,
                current_revision: self.revision,
            });
        }

        let revision = self
            .revision
            .checked_add(1)
            .context("managed configuration revision is exhausted")
            .map_err(PrepareManagementPatchError::Invalid)?;
        let mut candidate = self.clone();
        let mut changes = Vec::with_capacity(patch.mutations.len());

        for mutation in patch.mutations {
            apply_mutation(&mut candidate, mutation, plugins, &mut changes)
                .map_err(PrepareManagementPatchError::Invalid)?;
        }

        candidate.revision = revision;
        candidate
            .validate()
            .map_err(PrepareManagementPatchError::Invalid)?;
        validate_managed_plugin_settings(&candidate, plugins)
            .map_err(PrepareManagementPatchError::Invalid)?;

        Ok(PreparedManagementPatch {
            config: candidate,
            changes: ManagementChangeSet { revision, changes },
        })
    }
}

#[cfg(test)]
mod tests;
