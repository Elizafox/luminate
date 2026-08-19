// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Unit tests for managed-configuration merging and persistence.

use std::fs;
use std::io::Write as _;

use luminate_protocol::{ManagementChange, ManagementMutation, ManagementPatch, SettingValue};

use super::persistence::{load, save};
use super::preferences::{merge_daemon_preferences, plugin_is_enabled};
use super::schema::{ManageablePlugin, PluginActivationInputs, PluginSettingSchema};
use super::settings::merge_plugin_settings;
use crate::device_config::{DaemonConfig, ManagedPluginActivation, PluginActivationMode};

use super::*;
use crate::device_config::ManagementConfig;
use luminate_core::capability::CctEmulation;
use luminate_core::control::ReconciliationPolicy;
use luminate_core::device::DeviceId;
use luminate_platform::secure_storage::create_private_file;
use luminate_platform::test_support::TestDir;
use luminate_plugin_api::PluginSettingKind;
use luminate_protocol::UnsupportedPolicy;
use std::slice;

#[test]
fn missing_file_is_revision_zero() {
    let directory = TestDir::new("managed-config-missing");
    assert_eq!(
        load(&directory.path().join("managed.toml"))
            .expect("load missing managed config")
            .revision,
        0
    );
}

#[test]
fn invalid_existing_file_is_an_error() {
    let directory = TestDir::new("managed-config-invalid");
    let path = directory.path().join("managed.toml");
    let mut file = create_private_file(&path).expect("create invalid private managed config");
    file.write_all(b"revision = nope\n")
        .expect("write invalid managed config");
    assert!(load(&path).is_err());
}

#[test]
fn save_round_trips_and_replaces_atomically() {
    let directory = TestDir::new("managed-config-round-trip");
    let path = directory.path().join("managed.toml");
    let config = ManagedConfig {
        revision: 7,
        daemon: ManagedDaemonConfig {
            prefer_shm: Some(false),
            ..ManagedDaemonConfig::default()
        },
        plugins: vec![ManagedPluginConfig {
            name: "luminate-plugin-demo".to_owned(),
            enabled: Some(true),
            reconciliation: Some(ReconciliationPolicy::Restore),
            settings: toml::Table::new(),
        }],
    };
    save(&path, &config).expect("save managed config");
    assert_eq!(load(&path).expect("reload managed config"), config);
}

#[test]
fn daemon_preferences_merge_global_scoped_and_locked_values() {
    let device = DeviceId::new("desk");
    let locked_device = DeviceId::new("locked");
    let mut global = DaemonConfig {
        management: ManagementConfig {
            locked_daemon_settings: vec![
                "default_unsupported_policy".to_owned(),
                "device_reconciliation.locked".to_owned(),
                "prefer_shm".to_owned(),
            ],
            ..ManagementConfig::default()
        },
        default_unsupported_policy: UnsupportedPolicy::Skip,
        prefer_shm: true,
        ..DaemonConfig::default()
    };
    global
        .device_reconciliation
        .insert(locked_device.clone(), ReconciliationPolicy::Leave);

    let managed = ManagedDaemonConfig {
        default_unsupported_policy: Some(UnsupportedPolicy::Reject),
        reconciliation_policy: Some(ReconciliationPolicy::Restore),
        device_reconciliation: HashMap::from([
            (device.clone(), ReconciliationPolicy::Adopt),
            (locked_device.clone(), ReconciliationPolicy::Adopt),
        ]),
        cct_emulation: Some(CctEmulation::Disabled),
        prefer_shm: Some(false),
        prefer_client_shm: Some(false),
    };

    let effective = merge_daemon_preferences(&global, &managed);

    assert_eq!(
        effective.default_unsupported_policy,
        UnsupportedPolicy::Skip
    );
    assert_eq!(
        effective.reconciliation_policy,
        Some(ReconciliationPolicy::Restore)
    );
    assert_eq!(
        effective.device_reconciliation.get(&device),
        Some(&ReconciliationPolicy::Adopt)
    );
    assert_eq!(
        effective.device_reconciliation.get(&locked_device),
        Some(&ReconciliationPolicy::Leave)
    );
    assert_eq!(effective.cct_emulation, Some(CctEmulation::Disabled));
    assert!(effective.prefer_shm);
    assert!(!effective.prefer_client_shm);

    assert_eq!(
        managed.default_unsupported_policy,
        Some(UnsupportedPolicy::Reject)
    );
    assert_eq!(managed.prefer_shm, Some(false));
}

#[test]
fn activation_precedence_preserves_required_and_global_policy() {
    let managed_disabled = ManagedPluginConfig {
        enabled: Some(false),
        ..ManagedPluginConfig::default()
    };
    let ordinary = PluginActivationInputs {
        explicit: false,
        required: false,
        automatically_selected: false,
    };

    assert!(plugin_is_enabled(
        PluginActivationMode::Explicit,
        ManagedPluginActivation::Managed,
        Some(&managed_disabled),
        PluginActivationInputs {
            required: true,
            ..ordinary
        }
    ));
    assert!(plugin_is_enabled(
        PluginActivationMode::Explicit,
        ManagedPluginActivation::Enabled,
        Some(&managed_disabled),
        ordinary
    ));
    assert!(!plugin_is_enabled(
        PluginActivationMode::All,
        ManagedPluginActivation::Disabled,
        None,
        PluginActivationInputs {
            automatically_selected: true,
            ..ordinary
        }
    ));
}

#[test]
fn activation_uses_managed_state_then_global_defaults() {
    let enabled = ManagedPluginConfig {
        enabled: Some(true),
        ..ManagedPluginConfig::default()
    };
    let disabled = ManagedPluginConfig {
        enabled: Some(false),
        ..ManagedPluginConfig::default()
    };
    let ordinary = PluginActivationInputs {
        explicit: false,
        required: false,
        automatically_selected: false,
    };

    assert!(plugin_is_enabled(
        PluginActivationMode::Explicit,
        ManagedPluginActivation::Managed,
        Some(&enabled),
        ordinary
    ));
    assert!(!plugin_is_enabled(
        PluginActivationMode::All,
        ManagedPluginActivation::Managed,
        Some(&disabled),
        PluginActivationInputs {
            automatically_selected: true,
            ..ordinary
        }
    ));
    assert!(plugin_is_enabled(
        PluginActivationMode::Explicit,
        ManagedPluginActivation::Managed,
        None,
        PluginActivationInputs {
            explicit: true,
            ..ordinary
        }
    ));
    assert!(plugin_is_enabled(
        PluginActivationMode::HostAttached,
        ManagedPluginActivation::Managed,
        None,
        PluginActivationInputs {
            automatically_selected: true,
            ..ordinary
        }
    ));
}

#[test]
fn settings_merge_defaults_global_and_unlocked_managed_values() {
    let schema = [
        PluginSettingSchema {
            key: "connection.address".to_owned(),
            kind: PluginSettingKind::String,
            default: Some(toml::Value::String("broadcast".to_owned())),
            required: true,
            sensitive: false,
            minimum: None,
            maximum: None,
            constraints: None,
        },
        PluginSettingSchema {
            key: "connection.retries".to_owned(),
            kind: PluginSettingKind::Integer,
            default: Some(toml::Value::Integer(3)),
            required: false,
            sensitive: false,
            minimum: None,
            maximum: None,
            constraints: None,
        },
    ];
    let global = toml::Table::from_iter([(
        "connection".to_owned(),
        toml::Value::Table(toml::Table::from_iter([(
            "address".to_owned(),
            toml::Value::String("global".to_owned()),
        )])),
    )]);
    let managed = toml::Table::from_iter([(
        "connection".to_owned(),
        toml::Value::Table(toml::Table::from_iter([
            (
                "address".to_owned(),
                toml::Value::String("managed".to_owned()),
            ),
            ("retries".to_owned(), toml::Value::Integer(5)),
        ])),
    )]);

    let merged = merge_plugin_settings(
        &global,
        &managed,
        &["connection.address".to_owned()],
        &schema,
    )
    .expect("merge plugin settings");
    let connection = merged
        .get("connection")
        .and_then(toml::Value::as_table)
        .expect("connection table");

    assert_eq!(
        connection.get("address").and_then(toml::Value::as_str),
        Some("global")
    );
    assert_eq!(
        connection.get("retries").and_then(toml::Value::as_integer),
        Some(5)
    );
    assert_eq!(
        managed["connection"]["address"].as_str(),
        Some("managed"),
        "locked managed value remains stored"
    );
}

#[test]
fn settings_reject_undeclared_wrong_kind_and_missing_required_values() {
    let required = PluginSettingSchema {
        key: "address".to_owned(),
        kind: PluginSettingKind::String,
        default: None,
        required: true,
        sensitive: false,
        minimum: None,
        maximum: None,
        constraints: None,
    };
    let undeclared = toml::Table::from_iter([("mystery".to_owned(), toml::Value::Boolean(true))]);
    assert!(
        merge_plugin_settings(
            &undeclared,
            &toml::Table::new(),
            &[],
            slice::from_ref(&required),
        )
        .is_err()
    );

    let wrong_kind = toml::Table::from_iter([("address".to_owned(), toml::Value::Integer(42))]);
    assert!(
        merge_plugin_settings(
            &wrong_kind,
            &toml::Table::new(),
            &[],
            slice::from_ref(&required),
        )
        .is_err()
    );
    assert!(
        merge_plugin_settings(&toml::Table::new(), &toml::Table::new(), &[], &[required]).is_err()
    );
}

#[test]
fn managed_scoped_reconciliation_round_trips() {
    let config: ManagedConfig = toml::from_str(
        r#"
revision = 4

[daemon.device_reconciliation]
desk = "Adopt"

[[plugins]]
name = "luminate-plugin-lifx"
reconciliation = "Restore"
"#,
    )
    .expect("parse managed scoped preferences");

    assert_eq!(
        config
            .daemon
            .device_reconciliation
            .get(&DeviceId::new("desk")),
        Some(&ReconciliationPolicy::Adopt)
    );
    assert_eq!(
        config.plugins[0].reconciliation,
        Some(ReconciliationPolicy::Restore)
    );
    let serialized = toml::to_string(&config).expect("serialize managed config");
    assert!(serialized.contains("[daemon.device_reconciliation]"));
}

#[test]
fn removed_managed_plugin_reconciliation_map_is_rejected() {
    let error =
        toml::from_str::<ManagedConfig>("[daemon.plugin_reconciliation]\nexample = \"Restore\"\n")
            .unwrap_err();
    assert!(error.to_string().contains("plugin_reconciliation"));
}

fn transaction_schema() -> Vec<PluginSettingSchema> {
    vec![
        PluginSettingSchema {
            key: "credentials.token".to_owned(),
            kind: PluginSettingKind::String,
            default: None,
            required: true,
            sensitive: true,
            minimum: None,
            maximum: None,
            constraints: None,
        },
        PluginSettingSchema {
            key: "retries".to_owned(),
            kind: PluginSettingKind::Integer,
            default: Some(toml::Value::Integer(3)),
            required: false,
            sensitive: false,
            minimum: Some(1.0),
            maximum: Some(5.0),
            constraints: None,
        },
    ]
}

fn manageable_plugin(schema: &[PluginSettingSchema], global: &toml::Table) -> ManageablePlugin {
    ManageablePlugin {
        name: "example".to_owned(),
        global_settings: global.clone(),
        locked_settings: Vec::new(),
        schema: schema.to_vec(),
    }
}

#[test]
fn patch_is_revision_checked_and_atomic() {
    let config = ManagedConfig {
        revision: 4,
        ..ManagedConfig::default()
    };
    let schema = transaction_schema();
    let global = toml::Table::from_iter([(
        "credentials".to_owned(),
        toml::Value::Table(toml::Table::from_iter([(
            "token".to_owned(),
            toml::Value::String("global-token".to_owned()),
        )])),
    )]);
    let plugins = [manageable_plugin(&schema, &global)];

    let stale = config.prepare_patch(
        ManagementPatch {
            expected_revision: 3,
            mutations: Vec::new(),
        },
        &plugins,
    );
    assert!(matches!(
        stale,
        Err(PrepareManagementPatchError::Conflict {
            expected_revision: 3,
            current_revision: 4
        })
    ));

    let invalid = config.prepare_patch(
        ManagementPatch {
            expected_revision: 4,
            mutations: vec![
                ManagementMutation::SetPluginEnabled {
                    plugin: "example".to_owned(),
                    enabled: Some(true),
                },
                ManagementMutation::SetPluginSetting {
                    plugin: "example".to_owned(),
                    key: "retries".to_owned(),
                    value: luminate_protocol::WriteOnly::new(SettingValue::Integer(6)),
                },
            ],
        },
        &plugins,
    );
    assert!(invalid.is_err());
    assert_eq!(config.revision, 4);
    assert!(config.plugins.is_empty());
}

#[test]
fn patch_preserves_locked_values_as_dormant_desired_state() {
    let config = ManagedConfig::default();
    let schema = transaction_schema();
    let global = toml::Table::from_iter([(
        "credentials".to_owned(),
        toml::Value::Table(toml::Table::from_iter([(
            "token".to_owned(),
            toml::Value::String("administrator-token".to_owned()),
        )])),
    )]);
    let plugins = [ManageablePlugin {
        name: "example".to_owned(),
        global_settings: global.clone(),
        locked_settings: vec!["credentials".to_owned()],
        schema: schema.clone(),
    }];
    let prepared = config
        .prepare_patch(
            ManagementPatch {
                expected_revision: 0,
                mutations: vec![ManagementMutation::SetPluginSetting {
                    plugin: "example".to_owned(),
                    key: "credentials.token".to_owned(),
                    value: luminate_protocol::WriteOnly::new(SettingValue::String(
                        "managed-token".to_owned(),
                    )),
                }],
            },
            &plugins,
        )
        .expect("prepare locked setting patch");

    assert_eq!(
        prepared.config().plugins[0].settings["credentials"]["token"].as_str(),
        Some("managed-token")
    );
    let effective = merge_plugin_settings(
        &global,
        &prepared.config().plugins[0].settings,
        &plugins[0].locked_settings,
        &schema,
    )
    .expect("merge locked setting");
    assert_eq!(
        effective["credentials"]["token"].as_str(),
        Some("administrator-token")
    );
}

#[test]
fn patch_changes_are_redacted_and_preserve_request_order() {
    let config = ManagedConfig::default();
    let schema = transaction_schema();
    let global = toml::Table::from_iter([(
        "credentials".to_owned(),
        toml::Value::Table(toml::Table::from_iter([(
            "token".to_owned(),
            toml::Value::String("global-token".to_owned()),
        )])),
    )]);
    let plugins = [manageable_plugin(&schema, &global)];
    let prepared = config
        .prepare_patch(
            ManagementPatch {
                expected_revision: 0,
                mutations: vec![
                    ManagementMutation::SetPluginSetting {
                        plugin: "example".to_owned(),
                        key: "credentials.token".to_owned(),
                        value: luminate_protocol::WriteOnly::new(SettingValue::String(
                            "secret".to_owned(),
                        )),
                    },
                    ManagementMutation::SetPluginEnabled {
                        plugin: "example".to_owned(),
                        enabled: Some(true),
                    },
                ],
            },
            &plugins,
        )
        .expect("prepare redacted patch");

    assert_eq!(prepared.changes.revision, 1);
    assert!(matches!(
        prepared.changes.changes.as_slice(),
        [
            ManagementChange::PluginSettingChanged {
                plugin,
                key,
                sensitive: true
            },
            ManagementChange::PluginActivationChanged {
                plugin: activation_plugin
            }
        ] if plugin == "example"
            && key == "credentials.token"
            && activation_plugin == "example"
    ));
    assert!(!format!("{prepared:?}").contains("secret"));
}

#[test]
fn failed_persistence_does_not_change_in_memory_configuration() {
    let mut config = ManagedConfig::default();
    let schema = transaction_schema();
    let global = toml::Table::from_iter([(
        "credentials".to_owned(),
        toml::Value::Table(toml::Table::from_iter([(
            "token".to_owned(),
            toml::Value::String("global-token".to_owned()),
        )])),
    )]);
    let plugins = [manageable_plugin(&schema, &global)];
    let prepared = config
        .prepare_patch(
            ManagementPatch {
                expected_revision: 0,
                mutations: vec![ManagementMutation::SetPluginEnabled {
                    plugin: "example".to_owned(),
                    enabled: Some(true),
                }],
            },
            &plugins,
        )
        .expect("prepare patch");
    let directory = TestDir::new("managed-config-commit-failure");
    let non_directory = directory.path().join("ordinary-file");
    fs::write(&non_directory, "not a directory").expect("write ordinary file");
    let missing_parent = non_directory.join("managed.toml");

    assert!(prepared.commit(&missing_parent, &mut config).is_err());
    assert_eq!(config, ManagedConfig::default());
}
