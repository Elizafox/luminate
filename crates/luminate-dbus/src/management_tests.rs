// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

fn invalid_message(error: MethodError) -> String {
    let MethodError::InvalidArgument(message) = error else {
        panic!("expected invalid argument, got {error:?}");
    };
    message
}

fn empty_preferences() -> DaemonPreferences {
    DaemonPreferences {
        default_unsupported_policy: None,
        reconciliation_policy: None,
        device_reconciliation: Vec::new(),
        cct_emulation: None,
        prefer_shm: None,
        prefer_client_shm: None,
    }
}

#[test]
fn setting_mutation_preserves_native_value_without_exposing_it_in_debug() {
    let request = dictionary([
        (
            "Kind",
            owned("set-plugin-setting".to_owned()).expect("kind"),
        ),
        ("Plugin", owned("example".to_owned()).expect("plugin")),
        ("Key", owned("credentials.token".to_owned()).expect("key")),
        ("Value", owned("very-secret".to_owned()).expect("value")),
    ]);

    let patch = patch(7, vec![request]).expect("parse patch");
    assert_eq!(patch.expected_revision, 7);
    assert!(!format!("{patch:?}").contains("very-secret"));
    assert_eq!(
        patch.mutations,
        vec![ManagementMutation::SetPluginSetting {
            plugin: "example".to_owned(),
            key: "credentials.token".to_owned(),
            value: WriteOnly::new(SettingValue::String("very-secret".to_owned())),
        }]
    );
}

#[test]
fn mutation_parser_covers_every_patch_shape() {
    let preferences = dictionary([
        (
            "HasDefaultUnsupportedPolicy",
            owned(true).expect("presence"),
        ),
        (
            "DefaultUnsupportedPolicy",
            owned("reject".to_owned()).expect("unsupported policy"),
        ),
        ("HasReconciliationPolicy", owned(true).expect("presence")),
        (
            "ReconciliationPolicy",
            owned("restore".to_owned()).expect("reconciliation"),
        ),
        (
            "DeviceReconciliation",
            owned(vec![("keyboard".to_owned(), "leave".to_owned())]).expect("devices"),
        ),
        ("HasCctEmulation", owned(true).expect("presence")),
        (
            "CctEmulation",
            owned("disabled".to_owned()).expect("CCT mode"),
        ),
        ("HasPreferShm", owned(true).expect("presence")),
        ("PreferShm", owned(true).expect("preference")),
        ("HasPreferClientShm", owned(false).expect("presence")),
    ]);
    let requests = vec![
        dictionary([
            (
                "Kind",
                owned("set-daemon-preferences".to_owned()).expect("kind"),
            ),
            ("Preferences", owned(preferences).expect("preferences")),
        ]),
        dictionary([
            (
                "Kind",
                owned("set-plugin-enabled".to_owned()).expect("kind"),
            ),
            ("Plugin", owned("example".to_owned()).expect("plugin")),
            ("HasEnabled", owned(true).expect("presence")),
            ("Enabled", owned(false).expect("enabled")),
        ]),
        dictionary([
            (
                "Kind",
                owned("set-plugin-reconciliation".to_owned()).expect("kind"),
            ),
            ("Plugin", owned("example".to_owned()).expect("plugin")),
            ("HasReconciliation", owned(true).expect("presence")),
            (
                "Reconciliation",
                owned("adopt".to_owned()).expect("reconciliation"),
            ),
        ]),
        dictionary([
            (
                "Kind",
                owned("clear-plugin-setting".to_owned()).expect("kind"),
            ),
            ("Plugin", owned("example".to_owned()).expect("plugin")),
            ("Key", owned("address".to_owned()).expect("key")),
        ]),
    ];

    let patch = patch(11, requests).expect("parse every mutation");
    assert_eq!(patch.expected_revision, 11);
    assert_eq!(patch.mutations.len(), 4);
    let ManagementMutation::SetDaemonPreferences(preferences) = &patch.mutations[0] else {
        panic!("expected daemon preferences");
    };
    assert_eq!(
        preferences.default_unsupported_policy,
        Some(UnsupportedPolicy::Reject)
    );
    assert_eq!(
        preferences.reconciliation_policy,
        Some(ReconciliationPolicy::Restore)
    );
    assert_eq!(preferences.device_reconciliation.len(), 1);
    assert_eq!(preferences.cct_emulation, Some(CctEmulation::Disabled));
    assert_eq!(preferences.prefer_shm, Some(true));
    assert_eq!(preferences.prefer_client_shm, None);
}

#[test]
fn absent_plugin_activation_uses_explicit_presence_flag() {
    let request = dictionary([
        (
            "Kind",
            owned("set-plugin-enabled".to_owned()).expect("kind"),
        ),
        ("Plugin", owned("example".to_owned()).expect("plugin")),
        ("HasEnabled", owned(false).expect("presence")),
    ]);

    assert_eq!(
        patch(2, vec![request]).expect("parse patch").mutations,
        vec![ManagementMutation::SetPluginEnabled {
            plugin: "example".to_owned(),
            enabled: None,
        }]
    );
}

#[test]
fn malformed_mutations_report_specific_invalid_arguments() {
    let unknown = dictionary([("Kind", owned("surprise".to_owned()).expect("unknown kind"))]);
    assert!(invalid_message(patch(1, vec![unknown]).unwrap_err()).contains("unknown"));

    let missing = dictionary([(
        "Kind",
        owned("clear-plugin-setting".to_owned()).expect("kind"),
    )]);
    assert!(invalid_message(patch(1, vec![missing]).unwrap_err()).contains("Plugin"));

    let wrong_type = dictionary([("Kind", owned(42_u32).expect("wrong type"))]);
    assert!(invalid_message(patch(1, vec![wrong_type]).unwrap_err()).contains("wrong type"));

    for (field, value, expected) in [
        ("DefaultUnsupportedPolicy", "sometimes", "unsupported"),
        ("ReconciliationPolicy", "merge", "reconciliation"),
        ("CctEmulation", "maybe", "CCT"),
    ] {
        let mut preferences = dictionary([
            (
                "HasDefaultUnsupportedPolicy",
                owned(false).expect("presence"),
            ),
            ("HasReconciliationPolicy", owned(false).expect("presence")),
            (
                "DeviceReconciliation",
                owned(Vec::<(String, String)>::new()).expect("devices"),
            ),
            ("HasCctEmulation", owned(false).expect("presence")),
            ("HasPreferShm", owned(false).expect("presence")),
            ("HasPreferClientShm", owned(false).expect("presence")),
        ]);
        preferences.insert(format!("Has{field}"), owned(true).expect("presence"));
        preferences.insert(field.to_owned(), owned(value.to_owned()).expect("value"));
        assert!(
            invalid_message(parse_daemon_preferences(preferences).unwrap_err()).contains(expected)
        );
    }

    let mut preferences = dictionary([
        (
            "HasDefaultUnsupportedPolicy",
            owned(false).expect("presence"),
        ),
        ("HasReconciliationPolicy", owned(false).expect("presence")),
        (
            "DeviceReconciliation",
            owned(Vec::<(String, String)>::new()).expect("devices"),
        ),
        ("HasCctEmulation", owned(false).expect("presence")),
        ("HasPreferShm", owned(false).expect("presence")),
        ("HasPreferClientShm", owned(false).expect("presence")),
    ]);
    preferences.insert("Surprise".to_owned(), owned(true).expect("unknown field"));
    assert!(
        invalid_message(parse_daemon_preferences(preferences).unwrap_err()).contains("Surprise")
    );
}

#[test]
fn setting_values_cover_scalars_nested_collections_and_rejections() {
    assert_eq!(
        setting_value(owned(true).expect("Boolean")).expect("parse Boolean"),
        SettingValue::Boolean(true)
    );
    assert_eq!(
        setting_value(owned(-4_i64).expect("integer")).expect("parse integer"),
        SettingValue::Integer(-4)
    );
    assert_eq!(
        setting_value(owned(1.5_f64).expect("number")).expect("parse number"),
        SettingValue::Number(1.5)
    );
    assert_eq!(
        setting_value(owned("hello".to_owned()).expect("string")).expect("parse string"),
        SettingValue::String("hello".to_owned())
    );
    assert_eq!(
        setting_value(owned(vec![true, false]).expect("array")).expect("parse array"),
        SettingValue::Array(vec![
            SettingValue::Boolean(true),
            SettingValue::Boolean(false)
        ])
    );
    assert!(
        invalid_message(setting_value(owned(f64::NAN).expect("NaN")).unwrap_err())
            .contains("finite")
    );
    assert!(
        invalid_message(setting_value(owned(7_u32).expect("unsupported")).unwrap_err())
            .contains("unsupported")
    );
}

#[test]
fn conversion_helpers_cover_all_serialized_variants() {
    for runtime_state in [
        PluginRuntimeState::Inactive,
        PluginRuntimeState::Loading,
        PluginRuntimeState::Loaded,
        PluginRuntimeState::Failed {
            diagnostic: "boom".to_owned(),
        },
    ] {
        assert!(runtime(runtime_state).is_ok());
    }
    for value in [
        ReportedSettingValue::Unset,
        ReportedSettingValue::Redacted,
        ReportedSettingValue::Visible(SettingValue::Array(vec![
            SettingValue::Boolean(true),
            SettingValue::Integer(2),
            SettingValue::Number(3.0),
            SettingValue::String("four".to_owned()),
            SettingValue::Table(BTreeMap::from([(
                "five".to_owned(),
                SettingValue::Boolean(false),
            )])),
        ])),
    ] {
        assert!(reported(value).is_ok());
    }
    for kind in [
        PluginSettingKind::Boolean,
        PluginSettingKind::Integer,
        PluginSettingKind::Number,
        PluginSettingKind::String,
        PluginSettingKind::Enumeration,
        PluginSettingKind::Array,
    ] {
        assert!(!setting_kind(kind).is_empty());
    }
    assert_eq!(
        apply_mode(PluginSettingApplyMode::RestartRequired),
        "restart-required"
    );
    assert!(optional_bool_dict(None).is_ok());
    assert!(optional_bool_dict(Some(false)).is_ok());
    assert!(optional_string_dict(None).is_ok());
    assert!(optional_string_dict(Some("value")).is_ok());
    assert!(optional_f64_dict(None).is_ok());
    assert!(optional_f64_dict(Some(2.5)).is_ok());
}

#[test]
fn unknown_mutation_fields_are_rejected() {
    let request = dictionary([
        (
            "Kind",
            owned("clear-plugin-setting".to_owned()).expect("kind"),
        ),
        ("Plugin", owned("example".to_owned()).expect("plugin")),
        ("Key", owned("address".to_owned()).expect("key")),
        ("Surprise", owned(true).expect("unknown field")),
    ]);

    let error = patch(1, vec![request]).expect_err("unknown field must fail");
    assert!(invalid_message(error).contains("Surprise"));
}

#[test]
fn change_records_cover_every_variant_without_setting_values() {
    let (revision, records) = changes(ManagementChangeSet {
        revision: 9,
        changes: vec![
            ManagementChange::DaemonPreferencesChanged {
                keys: vec!["prefer-shm".to_owned()],
            },
            ManagementChange::PluginActivationChanged {
                plugin: "first".to_owned(),
            },
            ManagementChange::PluginReconciliationChanged {
                plugin: "second".to_owned(),
            },
            ManagementChange::PluginSettingChanged {
                plugin: "example".to_owned(),
                key: "credentials.token".to_owned(),
                sensitive: true,
            },
        ],
    });

    assert_eq!(revision, 9);
    assert_eq!(records.len(), 4);
    assert_eq!(
        records[3],
        (
            "plugin-setting".to_owned(),
            "example".to_owned(),
            vec!["credentials.token".to_owned()],
            true,
        )
    );
}

#[test]
fn snapshot_converts_complete_preferences_and_plugin_metadata() {
    let desired_settings = BTreeMap::from([(
        "credentials.token".to_owned(),
        ReportedSettingValue::Redacted,
    )]);
    let schema = PluginSettingSchema {
        key: "mode".to_owned(),
        label: "Mode".to_owned(),
        description: "Operating mode".to_owned(),
        kind: PluginSettingKind::Enumeration,
        default: ReportedSettingValue::Visible(SettingValue::String("auto".to_owned())),
        required: true,
        sensitive: false,
        apply_mode: PluginSettingApplyMode::RestartRequired,
        minimum: Some(1.0),
        maximum: Some(4.0),
        constraints: ReportedSettingValue::Visible(SettingValue::Array(vec![
            SettingValue::String("auto".to_owned()),
        ])),
    };
    let snapshot = snapshot(ManagementSnapshot {
        revision: 3,
        desired_daemon: empty_preferences(),
        effective_daemon: DaemonPreferences {
            default_unsupported_policy: Some(UnsupportedPolicy::Skip),
            reconciliation_policy: Some(ReconciliationPolicy::Leave),
            device_reconciliation: vec![DeviceReconciliationPreference {
                device: DeviceId::new("keyboard"),
                policy: ReconciliationPolicy::Adopt,
            }],
            cct_emulation: Some(CctEmulation::Auto),
            prefer_shm: Some(true),
            prefer_client_shm: Some(false),
        },
        locked_daemon_settings: vec!["prefer-shm".to_owned()],
        plugins: vec![ManagedPlugin {
            name: "example".to_owned(),
            version: "1".to_owned(),
            required: false,
            desired_enabled: Some(true),
            desired_reconciliation: Some(ReconciliationPolicy::Adopt),
            effective_reconciliation: Some(ReconciliationPolicy::Restore),
            effective_enabled: true,
            runtime: PluginRuntimeState::Failed {
                diagnostic: "retrying".to_owned(),
            },
            activation_locked: false,
            schema: vec![schema],
            desired_settings,
            effective_settings: BTreeMap::from([(
                "mode".to_owned(),
                ReportedSettingValue::Visible(SettingValue::String("auto".to_owned())),
            )]),
            locked_settings: vec!["mode".to_owned()],
        }],
    })
    .expect("convert snapshot");

    let rendered = format!("{snapshot:?}");
    assert!(rendered.contains("redacted"));
    assert!(rendered.contains("retrying"));
    assert!(!rendered.contains("very-secret"));
}
