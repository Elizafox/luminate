// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;
use crate::device_config::{ManagedPluginActivation, PluginManagementConfig};
use crate::managed_config::ManagedPluginConfig;
use crate::plugin_host::InspectedPluginSetting;

fn candidate(required: bool, explicit: bool, configuration: toml::Table) -> PluginCandidate {
    PluginCandidate {
        path: PathBuf::from("/plugins/example.so"),
        explicit,
        required,
        activation: ManagedPluginActivation::Managed,
        reconciliation: None,
        locked_settings: Vec::new(),
        configuration,
    }
}

fn inspected() -> InspectedPlugin {
    InspectedPlugin {
        name: "example".to_owned(),
        version: "1.0".to_owned(),
        buses: vec![PluginBus::Network],
        settings: Vec::new(),
        setup_workflows: Vec::new(),
    }
}

fn inspected_with_required_setting() -> InspectedPlugin {
    let mut plugin = inspected();
    plugin.settings.push(InspectedPluginSetting {
        key: "credential".to_owned(),
        label: "Credential".to_owned(),
        description: "Required credential.".to_owned(),
        kind: luminate_plugin_api::PluginSettingKind::String,
        default: None,
        required: true,
        sensitive: true,
        apply_mode: luminate_plugin_api::PluginSettingApplyMode::RestartRequired,
        minimum: None,
        maximum: None,
        constraints: None,
    });
    plugin
}

#[test]
fn host_attached_requires_only_known_local_buses() {
    assert!(automatically_selected(
        PluginActivationMode::HostAttached,
        &[PluginBus::Usb, PluginBus::I2c]
    ));
    assert!(!automatically_selected(
        PluginActivationMode::HostAttached,
        &[]
    ));
    assert!(!automatically_selected(
        PluginActivationMode::HostAttached,
        &[PluginBus::Usb, PluginBus::Network]
    ));
    assert!(!automatically_selected(
        PluginActivationMode::HostAttached,
        &[PluginBus::Unknown]
    ));
}

#[test]
fn all_selects_plugins_without_bus_metadata() {
    assert!(automatically_selected(PluginActivationMode::All, &[]));
}

#[test]
fn managed_enable_selects_an_installed_plugin_in_explicit_mode() {
    let global = DaemonConfig {
        plugin_management: PluginManagementConfig {
            activation: PluginActivationMode::Explicit,
        },
        ..DaemonConfig::default()
    };
    let managed = ManagedConfig {
        plugins: vec![ManagedPluginConfig {
            name: "example".to_owned(),
            enabled: Some(true),
            reconciliation: None,
            settings: toml::Table::new(),
        }],
        ..ManagedConfig::default()
    };

    let entry = resolve_candidate(
        &global,
        &managed,
        candidate(false, false, toml::Table::new()),
        inspected(),
    )
    .expect("resolve managed activation");

    assert!(entry.effective_enabled);
    assert_eq!(entry.desired_enabled, Some(true));
    assert_eq!(entry.runtime, PluginRuntimeState::Loading);
    assert!(entry.setup_workflows.is_empty());
}

#[test]
fn applying_managed_config_refreshes_the_management_view() {
    let global = DaemonConfig {
        plugin_management: PluginManagementConfig {
            activation: PluginActivationMode::Explicit,
        },
        ..DaemonConfig::default()
    };
    let mut entry = resolve_candidate(
        &global,
        &ManagedConfig::default(),
        candidate(false, false, toml::Table::new()),
        inspected(),
    )
    .expect("resolve installed plugin");
    let managed = ManagedConfig {
        plugins: vec![ManagedPluginConfig {
            name: "example".to_owned(),
            enabled: Some(true),
            reconciliation: None,
            settings: toml::Table::new(),
        }],
        ..ManagedConfig::default()
    };

    entry
        .apply_managed(&global, &managed)
        .expect("apply managed configuration");

    assert_eq!(entry.desired_enabled, Some(true));
    assert!(entry.effective_enabled);
    assert_eq!(entry.management.desired_enabled, Some(true));
    assert!(entry.management.effective_enabled);
    assert_eq!(entry.runtime, PluginRuntimeState::Inactive);
    assert_eq!(
        entry.management.runtime,
        ReportedPluginRuntimeState::Inactive
    );
}

#[test]
fn inspected_setup_workflows_are_advertised_by_the_catalogue() {
    let mut metadata = inspected();
    metadata.setup_workflows.push(PluginSetupWorkflow::new(
        "example",
        "pair",
        "Pair hardware",
        "Connect nearby hardware.",
        luminate_protocol::PluginSetupWorkflowKind::Provision,
    ));
    let entry = resolve_candidate(
        &DaemonConfig::default(),
        &ManagedConfig::default(),
        candidate(false, true, toml::Table::new()),
        metadata,
    )
    .expect("resolve plugin with setup workflow");

    assert_eq!(entry.setup_workflows.len(), 1);
    assert_eq!(entry.setup_workflows[0].id, "pair");
}

#[test]
fn enabled_optional_plugin_with_missing_settings_remains_failed_in_catalogue() {
    let global = DaemonConfig::default();
    let candidate = candidate(false, true, toml::Table::new());
    let inspected = inspected_with_required_setting();
    let error = resolve_candidate(
        &global,
        &ManagedConfig::default(),
        candidate.clone(),
        inspected.clone(),
    )
    .expect_err("required setting is missing");

    let entry = resolve_invalid_optional_candidate(
        &global,
        &ManagedConfig::default(),
        candidate,
        &inspected,
        &error,
    )
    .expect("retain optional plugin");

    assert!(entry.effective_enabled);
    assert!(matches!(entry.runtime, PluginRuntimeState::Failed(_)));
    assert!(entry.validation_schema[0].required);
    assert!(entry.management.schema[0].required);
}

#[test]
fn enabled_optional_plugin_with_invalid_settings_remains_failed_in_catalogue() {
    let global = DaemonConfig::default();
    let candidate = candidate(
        false,
        true,
        toml::toml! {
            credential = false
        },
    );
    let inspected = inspected_with_required_setting();
    let error = resolve_candidate(
        &global,
        &ManagedConfig::default(),
        candidate.clone(),
        inspected.clone(),
    )
    .expect_err("credential has the wrong type");

    let entry = resolve_invalid_optional_candidate(
        &global,
        &ManagedConfig::default(),
        candidate,
        &inspected,
        &error,
    )
    .expect("retain optional plugin");

    assert!(entry.effective_enabled);
    assert!(matches!(entry.runtime, PluginRuntimeState::Failed(_)));
    assert_eq!(
        entry.global_settings["credential"],
        toml::Value::Boolean(false)
    );
}

#[test]
fn inactive_plugin_may_await_setup_for_required_settings() {
    let global = DaemonConfig {
        plugin_management: PluginManagementConfig {
            activation: PluginActivationMode::Explicit,
        },
        ..DaemonConfig::default()
    };
    let candidate = candidate(false, false, toml::Table::new());
    let inspected = inspected_with_required_setting();
    let error = resolve_candidate(
        &global,
        &ManagedConfig::default(),
        candidate.clone(),
        inspected.clone(),
    )
    .expect_err("required setting is missing");

    let entry = resolve_invalid_optional_candidate(
        &global,
        &ManagedConfig::default(),
        candidate,
        &inspected,
        &error,
    )
    .expect("retain inactive plugin for setup");

    assert!(!entry.effective_enabled);
    assert_eq!(entry.runtime, PluginRuntimeState::Inactive);
}

#[test]
fn required_plugin_with_missing_settings_is_rejected() {
    let error = resolve_candidate(
        &DaemonConfig::default(),
        &ManagedConfig::default(),
        candidate(true, true, toml::Table::new()),
        inspected_with_required_setting(),
    )
    .expect_err("required plugin configuration must be complete");

    assert!(format!("{error:#}").contains("required plugin setting credential has no value"));
}

#[test]
fn managed_reconciliation_overrides_the_global_plugin_policy() {
    let global = DaemonConfig::default();
    let mut candidate = candidate(false, true, toml::Table::new());
    candidate.reconciliation = Some(ReconciliationPolicy::Leave);
    let managed = ManagedConfig {
        plugins: vec![ManagedPluginConfig {
            name: "example".to_owned(),
            reconciliation: Some(ReconciliationPolicy::Restore),
            ..ManagedPluginConfig::default()
        }],
        ..ManagedConfig::default()
    };

    let entry = resolve_candidate(&global, &managed, candidate, inspected())
        .expect("resolve reconciliation policy");

    assert_eq!(
        entry.effective_reconciliation,
        Some(ReconciliationPolicy::Restore)
    );
    assert_eq!(
        entry.management.desired_reconciliation,
        Some(ReconciliationPolicy::Restore)
    );
}

#[test]
fn management_view_redacts_sensitive_schema_and_setting_values() {
    let global = DaemonConfig {
        plugin_management: PluginManagementConfig {
            activation: PluginActivationMode::Explicit,
        },
        ..DaemonConfig::default()
    };
    let managed = ManagedConfig {
        plugins: vec![ManagedPluginConfig {
            name: "example".to_owned(),
            enabled: None,
            reconciliation: None,
            settings: toml::toml! {
                [credentials]
                token = "managed-secret"
            },
        }],
        ..ManagedConfig::default()
    };
    let mut inspected = inspected();
    inspected.settings.push(InspectedPluginSetting {
        key: "credentials.token".to_owned(),
        label: "Token".to_owned(),
        description: "Authentication token.".to_owned(),
        kind: luminate_plugin_api::PluginSettingKind::String,
        default: Some(toml::Value::String("default-secret".to_owned())),
        required: true,
        sensitive: true,
        apply_mode: luminate_plugin_api::PluginSettingApplyMode::RestartRequired,
        minimum: None,
        maximum: None,
        constraints: Some(toml::Value::Array(vec![toml::Value::String(
            "secret-constraint".to_owned(),
        )])),
    });
    let mut candidate = candidate(
        false,
        false,
        toml::toml! {
            [credentials]
            token = "global-secret"
        },
    );
    candidate.locked_settings = vec!["credentials.token".to_owned()];

    let entry = resolve_candidate(&global, &managed, candidate, inspected).expect("resolve plugin");
    let view = entry.management;

    assert_eq!(view.schema[0].default, ReportedSettingValue::Redacted);
    assert_eq!(view.schema[0].constraints, ReportedSettingValue::Redacted);
    assert_eq!(
        view.desired_settings["credentials.token"],
        ReportedSettingValue::Redacted
    );
    assert_eq!(
        view.effective_settings["credentials.token"],
        ReportedSettingValue::Redacted
    );
    assert_eq!(view.locked_settings, vec!["credentials.token"]);
}

#[test]
fn required_plugin_remains_enabled_despite_both_disable_layers() {
    let global = DaemonConfig {
        plugin_management: PluginManagementConfig {
            activation: PluginActivationMode::Explicit,
        },
        ..DaemonConfig::default()
    };
    let managed = ManagedConfig {
        plugins: vec![ManagedPluginConfig {
            name: "example".to_owned(),
            enabled: Some(false),
            reconciliation: None,
            settings: toml::Table::new(),
        }],
        ..ManagedConfig::default()
    };

    let mut candidate = candidate(true, true, toml::Table::new());
    candidate.activation = ManagedPluginActivation::Disabled;
    let entry = resolve_candidate(&global, &managed, candidate, inspected())
        .expect("resolve required activation");

    assert!(entry.effective_enabled);
}
