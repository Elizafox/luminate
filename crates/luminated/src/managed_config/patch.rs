// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Preparing a managed-configuration transaction.
//!
//! A patch is validated in full and staged as a [`PreparedManagementPatch`]
//! before anything is written, so a rejected patch leaves both the file and
//! the in-memory authority untouched. Committing is the only step that can
//! change either, and it changes both together.

/// A prepared managed-configuration transaction which has not been persisted.
use std::fmt;
use std::path::Path;

use anyhow::{Context as _, Result};
use luminate_protocol::{ManagementChange, ManagementChangeSet, ManagementMutation};
use thiserror::Error;

use super::persistence::save;
use super::preferences::{changed_daemon_preferences, managed_daemon_config};
use super::schema::ManageablePlugin;
use super::settings::{
    clear_dotted_setting, merge_plugin_settings, set_dotted_setting, setting_value_to_toml,
    validate_setting_value,
};
use super::{ManagedConfig, ManagedPluginConfig};

pub struct PreparedManagementPatch {
    pub(super) config: ManagedConfig,
    pub(super) changes: ManagementChangeSet,
}

/// Failure to prepare a managed-configuration transaction.
#[derive(Debug, Error)]
pub enum PrepareManagementPatchError {
    /// The caller prepared its patch from a stale snapshot.
    #[error("expected revision {expected_revision}, current revision is {current_revision}")]
    Conflict {
        expected_revision: u64,
        current_revision: u64,
    },
    /// The proposed configuration is invalid.
    #[error(transparent)]
    Invalid(anyhow::Error),
}

impl fmt::Debug for PreparedManagementPatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedManagementPatch")
            .field("revision", &self.config.revision)
            .field("changes", &self.changes)
            .finish()
    }
}

impl PreparedManagementPatch {
    /// Returns the fully validated candidate configuration.
    #[must_use]
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "inspecting a prepared patch before committing it has no production caller yet; the dispatch path commits directly"
        )
    )]
    pub fn config(&self) -> &ManagedConfig {
        &self.config
    }

    /// Persists the candidate atomically and makes it the in-memory authority.
    ///
    /// # Errors
    ///
    /// Returns an error without changing `current` when persistence fails.
    pub fn commit(self, path: &Path, current: &mut ManagedConfig) -> Result<ManagementChangeSet> {
        save(path, &self.config)?;
        *current = self.config;
        Ok(self.changes)
    }
}

pub(super) fn validate_managed_plugin_settings(
    config: &ManagedConfig,
    plugins: &[ManageablePlugin],
) -> Result<()> {
    for managed in &config.plugins {
        let plugin = require_plugin(plugins, &managed.name)?;
        merge_plugin_settings(
            &plugin.global_settings,
            &managed.settings,
            &plugin.locked_settings,
            &plugin.schema,
        )
        .with_context(|| format!("validating settings for plugin {}", managed.name))?;
    }
    Ok(())
}

pub(super) fn apply_mutation(
    config: &mut ManagedConfig,
    mutation: ManagementMutation,
    plugins: &[ManageablePlugin],
    changes: &mut Vec<ManagementChange>,
) -> Result<()> {
    match mutation {
        ManagementMutation::SetDaemonPreferences(preferences) => {
            let daemon = managed_daemon_config(preferences);
            let keys = changed_daemon_preferences(&config.daemon, &daemon);
            config.daemon = daemon;
            changes.push(ManagementChange::DaemonPreferencesChanged { keys });
        }
        ManagementMutation::SetPluginEnabled { plugin, enabled } => {
            require_plugin(plugins, &plugin)?;
            managed_plugin_mut(config, &plugin)?.enabled = enabled;
            changes.push(ManagementChange::PluginActivationChanged { plugin });
        }
        ManagementMutation::SetPluginReconciliation {
            plugin,
            reconciliation,
        } => {
            require_plugin(plugins, &plugin)?;
            managed_plugin_mut(config, &plugin)?.reconciliation = reconciliation;
            changes.push(ManagementChange::PluginReconciliationChanged { plugin });
        }
        ManagementMutation::SetPluginSetting { plugin, key, value } => {
            let metadata = require_plugin(plugins, &plugin)?;
            let setting = metadata
                .schema
                .iter()
                .find(|setting| setting.key == key)
                .with_context(|| format!("plugin {plugin} has no setting {key}"))?;
            let value = setting_value_to_toml(value.into_inner())?;
            validate_setting_value(setting, &value)?;
            let managed = managed_plugin_mut(config, &plugin)?;
            set_dotted_setting(&mut managed.settings, &key, value)?;
            changes.push(ManagementChange::PluginSettingChanged {
                plugin,
                key,
                sensitive: setting.sensitive,
            });
        }
        ManagementMutation::ClearPluginSetting { plugin, key } => {
            let metadata = require_plugin(plugins, &plugin)?;
            let setting = metadata
                .schema
                .iter()
                .find(|setting| setting.key == key)
                .with_context(|| format!("plugin {plugin} has no setting {key}"))?;
            let managed = managed_plugin_mut(config, &plugin)?;
            clear_dotted_setting(&mut managed.settings, &key);
            changes.push(ManagementChange::PluginSettingChanged {
                plugin,
                key,
                sensitive: setting.sensitive,
            });
        }
    }
    Ok(())
}

fn require_plugin<'a>(plugins: &'a [ManageablePlugin], name: &str) -> Result<&'a ManageablePlugin> {
    plugins
        .iter()
        .find(|plugin| plugin.name == name)
        .with_context(|| format!("plugin {name} is not installed"))
}

fn managed_plugin_mut<'a>(
    config: &'a mut ManagedConfig,
    name: &str,
) -> Result<&'a mut ManagedPluginConfig> {
    let index = config.plugins.iter().position(|plugin| plugin.name == name);
    if let Some(index) = index {
        return config
            .plugins
            .get_mut(index)
            .context("managed plugin index disappeared");
    }

    config.plugins.push(ManagedPluginConfig {
        name: name.to_owned(),
        ..ManagedPluginConfig::default()
    });
    config
        .plugins
        .last_mut()
        .context("managed plugin insertion did not create an entry")
}
