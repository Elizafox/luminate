// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Installed plugin inspection and effective startup-state resolution.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use luminate_core::control::ReconciliationPolicy;
use luminate_plugin_api::PluginBus;
use luminate_protocol::{
    ManagedPlugin, PluginRuntimeState as ReportedPluginRuntimeState, PluginSetupWorkflow,
    ReportedSettingValue, SettingValue,
};

use super::discovery::{PluginCandidate, discover_candidates};
use crate::device_config::{DaemonConfig, ManagedPluginActivation, PluginActivationMode};
use crate::managed_config::{
    ManagedConfig, PluginActivationInputs, PluginSettingSchema, merge_plugin_settings,
    plugin_is_enabled,
};
use crate::plugin_host::{self, InspectedPlugin, InspectedPluginSetting};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PluginRuntimeState {
    Inactive,
    Loading,
    Loaded,
    Failed(String),
}

#[derive(Debug, Clone)]
pub(super) struct PluginCatalogueEntry {
    pub(super) name: String,
    pub(super) version: String,
    pub(super) path: PathBuf,
    pub(super) required: bool,
    activation_inputs: PluginActivationInputs,
    global_activation: ManagedPluginActivation,
    global_reconciliation: Option<ReconciliationPolicy>,
    pub(super) desired_enabled: Option<bool>,
    pub(super) effective_enabled: bool,
    pub(super) effective_reconciliation: Option<ReconciliationPolicy>,
    pub(super) configuration_cbor: Vec<u8>,
    pub(super) global_settings: toml::Table,
    pub(super) locked_settings: Vec<String>,
    pub(super) validation_schema: Vec<PluginSettingSchema>,
    pub(super) setup_workflows: Vec<PluginSetupWorkflow>,
    pub(super) runtime: PluginRuntimeState,
    pub(super) management: ManagedPlugin,
}

impl PluginCatalogueEntry {
    pub(super) fn set_runtime(&mut self, runtime: PluginRuntimeState) {
        self.management.runtime = reported_runtime_state(&runtime);
        self.runtime = runtime;
    }

    pub(super) fn apply_managed(
        &mut self,
        global: &DaemonConfig,
        managed: &ManagedConfig,
    ) -> Result<()> {
        let managed_plugin = managed
            .plugins
            .iter()
            .find(|plugin| plugin.name == self.name);
        let empty = toml::Table::new();
        let settings = merge_plugin_settings(
            &self.global_settings,
            managed_plugin.map_or(&empty, |plugin| &plugin.settings),
            &self.locked_settings,
            &self.validation_schema,
        )
        .with_context(|| format!("resolving settings for plugin {}", self.name))?;

        self.desired_enabled = managed_plugin.and_then(|plugin| plugin.enabled);
        self.effective_enabled = plugin_is_enabled(
            global.plugin_activation_mode(),
            self.global_activation,
            managed_plugin,
            self.activation_inputs,
        );
        self.configuration_cbor = encode_configuration(&settings)?;

        self.management.desired_enabled = self.desired_enabled;
        self.effective_reconciliation = managed_plugin
            .and_then(|plugin| plugin.reconciliation)
            .or(self.global_reconciliation);
        self.management.desired_reconciliation =
            managed_plugin.and_then(|plugin| plugin.reconciliation);
        self.management.effective_reconciliation = self.effective_reconciliation;
        self.management.effective_enabled = self.effective_enabled;
        self.management.desired_settings = reported_settings(
            managed_plugin.map_or(&empty, |plugin| &plugin.settings),
            &self.validation_schema,
        )?;
        self.management.effective_settings = reported_settings(&settings, &self.validation_schema)?;

        Ok(())
    }
}

pub(super) fn build_catalogue(
    global: &DaemonConfig,
    managed: &ManagedConfig,
) -> Result<Vec<PluginCatalogueEntry>> {
    let candidates = discover_candidates(global)?;
    let mut names = HashSet::new();
    let mut entries = Vec::new();

    for candidate in candidates {
        let inspected = match plugin_host::inspect(&candidate.path) {
            Ok(inspected) => inspected,
            Err(error) if candidate.required => {
                return Err(error).with_context(|| {
                    format!(
                        "required plugin could not be inspected: {}",
                        candidate.path.display()
                    )
                });
            }
            Err(error) => {
                tracing::warn!(
                    path = %candidate.path.display(),
                    error = %error,
                    "skipping plugin whose static metadata could not be inspected"
                );
                continue;
            }
        };

        if !names.insert(inspected.name.clone()) {
            if candidate.required {
                anyhow::bail!(
                    "required plugin has duplicate installed name {}: {}",
                    inspected.name,
                    candidate.path.display()
                );
            }
            tracing::warn!(
                path = %candidate.path.display(),
                plugin = %inspected.name,
                "ignoring lower-precedence plugin with duplicate installed name"
            );
            continue;
        }

        let required = candidate.required;
        match resolve_candidate(global, managed, candidate.clone(), inspected.clone()) {
            Ok(entry) => entries.push(entry),
            Err(error) if required => return Err(error),
            Err(error) => {
                tracing::warn!(
                    plugin = %inspected.name,
                    error = %error,
                    "optional plugin configuration is invalid"
                );
                entries.push(resolve_invalid_optional_candidate(
                    global, managed, candidate, &inspected, &error,
                )?);
            }
        }
    }

    Ok(entries)
}

fn resolve_invalid_optional_candidate(
    global: &DaemonConfig,
    managed: &ManagedConfig,
    candidate: PluginCandidate,
    inspected: &InspectedPlugin,
    error: &anyhow::Error,
) -> Result<PluginCatalogueEntry> {
    let global_settings = candidate.configuration.clone();
    let managed_settings = managed
        .plugins
        .iter()
        .find(|plugin| plugin.name == inspected.name)
        .map(|plugin| plugin.settings.clone());
    let mut relaxed = inspected.clone();
    for setting in &mut relaxed.settings {
        setting.required = false;
    }
    let mut entry =
        if let Ok(entry) = resolve_candidate(global, managed, candidate.clone(), relaxed.clone()) {
            entry
        } else {
            let mut sanitized_candidate = candidate;
            sanitized_candidate.configuration.clear();
            let mut sanitized_managed = managed.clone();
            if let Some(plugin) = sanitized_managed
                .plugins
                .iter_mut()
                .find(|plugin| plugin.name == inspected.name)
            {
                plugin.settings.clear();
            }
            resolve_candidate(global, &sanitized_managed, sanitized_candidate, relaxed)?
        };
    entry.global_settings = global_settings;
    if let Some(managed_settings) = managed_settings {
        entry.management.desired_settings =
            reported_settings(&managed_settings, &entry.validation_schema)?;
    }
    for (schema, setting) in entry.validation_schema.iter_mut().zip(&inspected.settings) {
        schema.required = setting.required;
    }
    for (schema, setting) in entry.management.schema.iter_mut().zip(&inspected.settings) {
        schema.required = setting.required;
    }
    if entry.effective_enabled {
        entry.set_runtime(PluginRuntimeState::Failed(format!("{error:#}")));
    }
    Ok(entry)
}

fn resolve_candidate(
    global: &DaemonConfig,
    managed: &ManagedConfig,
    candidate: PluginCandidate,
    inspected: InspectedPlugin,
) -> Result<PluginCatalogueEntry> {
    let managed_plugin = managed
        .plugins
        .iter()
        .find(|plugin| plugin.name == inspected.name);
    let automatically_selected =
        automatically_selected(global.plugin_activation_mode(), &inspected.buses);
    let effective_enabled = plugin_is_enabled(
        global.plugin_activation_mode(),
        candidate.activation,
        managed_plugin,
        PluginActivationInputs {
            explicit: candidate.explicit,
            required: candidate.required,
            automatically_selected,
        },
    );
    let validation_schema = inspected
        .settings
        .iter()
        .map(|setting| PluginSettingSchema {
            key: setting.key.clone(),
            kind: setting.kind,
            default: setting.default.clone(),
            required: setting.required,
            sensitive: setting.sensitive,
            minimum: setting.minimum,
            maximum: setting.maximum,
            constraints: setting.constraints.clone(),
        })
        .collect::<Vec<_>>();
    let empty = toml::Table::new();
    let settings = merge_plugin_settings(
        &candidate.configuration,
        managed_plugin.map_or(&empty, |plugin| &plugin.settings),
        &candidate.locked_settings,
        &validation_schema,
    )
    .with_context(|| format!("resolving settings for plugin {}", inspected.name))?;
    let desired_settings = reported_settings(
        managed_plugin.map_or(&empty, |plugin| &plugin.settings),
        &validation_schema,
    )?;
    let effective_settings = reported_settings(&settings, &validation_schema)?;
    let reported_schema = inspected
        .settings
        .iter()
        .map(reported_setting_schema)
        .collect::<Result<Vec<_>>>()?;
    let locked_settings = candidate.locked_settings.clone();
    let activation_locked = candidate.activation != ManagedPluginActivation::Managed;
    let desired_reconciliation = managed_plugin.and_then(|plugin| plugin.reconciliation);
    let effective_reconciliation = desired_reconciliation.or(candidate.reconciliation);
    let configuration_cbor = encode_configuration(&settings)?;
    let runtime = if effective_enabled {
        PluginRuntimeState::Loading
    } else {
        PluginRuntimeState::Inactive
    };

    Ok(PluginCatalogueEntry {
        name: inspected.name.clone(),
        version: inspected.version.clone(),
        path: candidate.path,
        required: candidate.required,
        activation_inputs: PluginActivationInputs {
            explicit: candidate.explicit,
            required: candidate.required,
            automatically_selected,
        },
        global_activation: candidate.activation,
        global_reconciliation: candidate.reconciliation,
        desired_enabled: managed_plugin.and_then(|plugin| plugin.enabled),
        effective_enabled,
        effective_reconciliation,
        configuration_cbor,
        global_settings: candidate.configuration,
        locked_settings: locked_settings.clone(),
        validation_schema,
        setup_workflows: inspected.setup_workflows,
        runtime: runtime.clone(),
        management: ManagedPlugin {
            name: inspected.name,
            version: inspected.version,
            required: candidate.required,
            desired_enabled: managed_plugin.and_then(|plugin| plugin.enabled),
            desired_reconciliation,
            effective_reconciliation,
            effective_enabled,
            runtime: reported_runtime_state(&runtime),
            activation_locked,
            schema: reported_schema,
            desired_settings,
            effective_settings,
            locked_settings,
        },
    })
}

fn encode_configuration(settings: &toml::Table) -> Result<Vec<u8>> {
    let mut configuration_cbor = Vec::new();
    ciborium::into_writer(settings, &mut configuration_cbor)
        .context("serializing effective plugin settings as CBOR")?;
    Ok(configuration_cbor)
}

fn reported_setting_schema(
    setting: &InspectedPluginSetting,
) -> Result<luminate_protocol::PluginSettingSchema> {
    Ok(luminate_protocol::PluginSettingSchema {
        key: setting.key.clone(),
        label: setting.label.clone(),
        description: setting.description.clone(),
        kind: match setting.kind {
            luminate_plugin_api::PluginSettingKind::Boolean => {
                luminate_protocol::PluginSettingKind::Boolean
            }
            luminate_plugin_api::PluginSettingKind::Integer => {
                luminate_protocol::PluginSettingKind::Integer
            }
            luminate_plugin_api::PluginSettingKind::Number => {
                luminate_protocol::PluginSettingKind::Number
            }
            luminate_plugin_api::PluginSettingKind::String => {
                luminate_protocol::PluginSettingKind::String
            }
            luminate_plugin_api::PluginSettingKind::Enumeration => {
                luminate_protocol::PluginSettingKind::Enumeration
            }
            luminate_plugin_api::PluginSettingKind::Array => {
                luminate_protocol::PluginSettingKind::Array
            }
        },
        default: reported_value(setting.default.as_ref(), setting.sensitive)?,
        required: setting.required,
        sensitive: setting.sensitive,
        apply_mode: match setting.apply_mode {
            luminate_plugin_api::PluginSettingApplyMode::RestartRequired => {
                luminate_protocol::PluginSettingApplyMode::RestartRequired
            }
        },
        minimum: setting.minimum,
        maximum: setting.maximum,
        constraints: reported_value(setting.constraints.as_ref(), setting.sensitive)?,
    })
}

fn reported_settings(
    settings: &toml::Table,
    schema: &[PluginSettingSchema],
) -> Result<BTreeMap<String, ReportedSettingValue>> {
    let mut flattened = BTreeMap::new();
    flatten_settings(None, settings, &mut flattened)?;

    schema
        .iter()
        .map(|setting| {
            reported_value(flattened.get(&setting.key), setting.sensitive)
                .map(|value| (setting.key.clone(), value))
        })
        .collect()
}

fn flatten_settings(
    prefix: Option<&str>,
    table: &toml::Table,
    values: &mut BTreeMap<String, toml::Value>,
) -> Result<()> {
    for (key, value) in table {
        let key = prefix.map_or_else(|| key.clone(), |prefix| format!("{prefix}.{key}"));
        if let toml::Value::Table(child) = value {
            flatten_settings(Some(&key), child, values)?;
        } else {
            anyhow::ensure!(
                values.insert(key.clone(), value.clone()).is_none(),
                "duplicate plugin setting {key}"
            );
        }
    }
    Ok(())
}

fn reported_value(value: Option<&toml::Value>, sensitive: bool) -> Result<ReportedSettingValue> {
    match value {
        None => Ok(ReportedSettingValue::Unset),
        Some(_) if sensitive => Ok(ReportedSettingValue::Redacted),
        Some(value) => Ok(ReportedSettingValue::Visible(setting_value(value)?)),
    }
}

fn setting_value(value: &toml::Value) -> Result<SettingValue> {
    Ok(match value {
        toml::Value::String(value) => SettingValue::String(value.clone()),
        toml::Value::Integer(value) => SettingValue::Integer(*value),
        toml::Value::Float(value) => SettingValue::Number(*value),
        toml::Value::Boolean(value) => SettingValue::Boolean(*value),
        toml::Value::Datetime(_) => {
            anyhow::bail!("plugin setting metadata contains an unsupported datetime value")
        }
        toml::Value::Array(values) => SettingValue::Array(
            values
                .iter()
                .map(setting_value)
                .collect::<Result<Vec<_>>>()?,
        ),
        toml::Value::Table(values) => SettingValue::Table(
            values
                .iter()
                .map(|(key, value)| setting_value(value).map(|value| (key.clone(), value)))
                .collect::<Result<BTreeMap<_, _>>>()?,
        ),
    })
}

pub(super) fn reported_runtime_state(runtime: &PluginRuntimeState) -> ReportedPluginRuntimeState {
    match runtime {
        PluginRuntimeState::Inactive => ReportedPluginRuntimeState::Inactive,
        PluginRuntimeState::Loading => ReportedPluginRuntimeState::Loading,
        PluginRuntimeState::Loaded => ReportedPluginRuntimeState::Loaded,
        PluginRuntimeState::Failed(diagnostic) => ReportedPluginRuntimeState::Failed {
            diagnostic: diagnostic.clone(),
        },
    }
}

fn automatically_selected(mode: PluginActivationMode, buses: &[PluginBus]) -> bool {
    match mode {
        PluginActivationMode::Explicit => false,
        PluginActivationMode::HostAttached => {
            // An empty, unknown, or network bus set does not prove that the
            // plugin manages hardware attached to this host.
            !buses.is_empty()
                && buses
                    .iter()
                    .all(|bus| !matches!(bus, PluginBus::Unknown | PluginBus::Network))
        }
        PluginActivationMode::All => true,
    }
}

#[cfg(test)]
#[path = "catalogue_tests.rs"]
mod tests;
