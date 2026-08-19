// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::ptr;

use super::*;

fn bytes(view: LuminateStringView) -> Option<&'static [u8]> {
    if view.data.is_null() {
        None
    } else {
        // SAFETY: every test keeps the value backing its borrowed view alive.
        Some(unsafe { std::slice::from_raw_parts(view.data.cast(), view.len) })
    }
}

fn preferences() -> DaemonPreferences {
    DaemonPreferences {
        default_unsupported_policy: Some(UnsupportedPolicy::Reject),
        reconciliation_policy: Some(ReconciliationPolicy::Adopt),
        device_reconciliation: vec![
            DeviceReconciliationPreference {
                device: DeviceId::new("keyboard"),
                policy: ReconciliationPolicy::Restore,
            },
            DeviceReconciliationPreference {
                device: DeviceId::new("desk"),
                policy: ReconciliationPolicy::Leave,
            },
        ],
        cct_emulation: Some(CctEmulation::Disabled),
        prefer_shm: Some(true),
        prefer_client_shm: Some(false),
    }
}

fn schemas() -> Vec<PluginSettingSchema> {
    [
        PluginSettingKind::Boolean,
        PluginSettingKind::Integer,
        PluginSettingKind::Number,
        PluginSettingKind::String,
        PluginSettingKind::Enumeration,
        PluginSettingKind::Array,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, kind)| PluginSettingSchema {
        key: format!("setting.{index}"),
        label: format!("Setting {index}"),
        description: format!("Description {index}"),
        kind,
        default: if index == 0 {
            ReportedSettingValue::Unset
        } else {
            ReportedSettingValue::Visible(SettingValue::Integer(
                i64::try_from(index).expect("fixture index fits i64"),
            ))
        },
        required: index == 1,
        sensitive: index == 2,
        apply_mode: PluginSettingApplyMode::RestartRequired,
        minimum: (index == 2).then_some(1.5),
        maximum: (index == 2).then_some(9.5),
        constraints: if index == 4 {
            ReportedSettingValue::Redacted
        } else {
            ReportedSettingValue::Visible(SettingValue::Array(Vec::new()))
        },
    })
    .collect()
}

fn plugin(runtime: PluginRuntimeState) -> ManagedPlugin {
    ManagedPlugin {
        name: "example".to_owned(),
        version: "1.2.3".to_owned(),
        required: true,
        desired_enabled: Some(false),
        desired_reconciliation: Some(ReconciliationPolicy::Leave),
        effective_reconciliation: Some(ReconciliationPolicy::Restore),
        effective_enabled: true,
        runtime,
        activation_locked: true,
        schema: schemas(),
        desired_settings: BTreeMap::from([
            ("a".to_owned(), ReportedSettingValue::Unset),
            (
                "b".to_owned(),
                ReportedSettingValue::Visible(SettingValue::Boolean(true)),
            ),
        ]),
        effective_settings: BTreeMap::from([("secret".to_owned(), ReportedSettingValue::Redacted)]),
        locked_settings: vec!["secret".to_owned()],
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keeping the builder lifecycle together makes ownership and cleanup at the C boundary auditable."
)]
fn patch_builder_covers_valid_mutations_and_validation_failures() {
    let mut builder = ptr::null_mut();
    // SAFETY: each output pointer is writable and all C strings are static.
    unsafe {
        assert_eq!(
            luminate_management_patch_builder_new(3, &raw mut builder),
            LuminateStatus::Ok
        );
        assert!(!builder.is_null());
        assert_eq!(
            luminate_management_patch_builder_set_expected_revision(builder, 9),
            LuminateStatus::Ok
        );

        let devices = [
            LuminateDeviceReconciliationPreferenceInput {
                device_id: c"keyboard".as_ptr(),
                policy: LUMINATE_RECONCILIATION_POLICY_ADOPT,
            },
            LuminateDeviceReconciliationPreferenceInput {
                device_id: c"desk".as_ptr(),
                policy: LUMINATE_RECONCILIATION_POLICY_LEAVE,
            },
        ];
        let input = LuminateDaemonPreferencesInput {
            has_default_unsupported_policy: true,
            default_unsupported_policy: LUMINATE_UNSUPPORTED_POLICY_REJECT,
            has_reconciliation_policy: true,
            reconciliation_policy: LUMINATE_RECONCILIATION_POLICY_RESTORE,
            device_reconciliation: devices.as_ptr(),
            device_reconciliation_count: devices.len(),
            has_cct_emulation: true,
            cct_emulation: LUMINATE_CCT_EMULATION_DISABLED,
            has_prefer_shm: true,
            prefer_shm: true,
            has_prefer_client_shm: true,
            prefer_client_shm: false,
        };
        assert_eq!(
            luminate_management_patch_builder_set_daemon_preferences(builder, &raw const input),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_management_patch_builder_set_plugin_enabled(
                builder,
                c"example".as_ptr(),
                true,
                false,
            ),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_management_patch_builder_set_plugin_enabled(
                builder,
                c"example".as_ptr(),
                false,
                true,
            ),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_management_patch_builder_set_plugin_reconciliation(
                builder,
                c"example".as_ptr(),
                true,
                LUMINATE_RECONCILIATION_POLICY_ADOPT,
            ),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_management_patch_builder_set_plugin_reconciliation(
                builder,
                c"example".as_ptr(),
                false,
                u32::MAX,
            ),
            LuminateStatus::Ok
        );

        let value = LuminateSettingValue(SettingValue::String("token".to_owned()));
        assert_eq!(
            luminate_management_patch_builder_set_plugin_setting(
                builder,
                c"example".as_ptr(),
                c"credentials.token".as_ptr(),
                &raw const value,
            ),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_management_patch_builder_clear_plugin_setting(
                builder,
                c"example".as_ptr(),
                c"credentials.token".as_ptr(),
            ),
            LuminateStatus::Ok
        );
        assert_eq!((*builder).0.expected_revision, 9);
        assert_eq!((*builder).0.mutations.len(), 7);

        assert_eq!(
            luminate_management_patch_builder_set_expected_revision(ptr::null_mut(), 1),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_management_patch_builder_set_daemon_preferences(builder, ptr::null()),
            LuminateStatus::NullPointer
        );
        let invalid_input = LuminateDaemonPreferencesInput {
            default_unsupported_policy: u32::MAX,
            ..input
        };
        assert_eq!(
            luminate_management_patch_builder_set_daemon_preferences(
                builder,
                &raw const invalid_input
            ),
            LuminateStatus::InvalidArgument
        );
        let null_devices = LuminateDaemonPreferencesInput {
            device_reconciliation: ptr::null(),
            device_reconciliation_count: 1,
            ..input
        };
        assert_eq!(
            luminate_management_patch_builder_set_daemon_preferences(
                builder,
                &raw const null_devices
            ),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_management_patch_builder_set_plugin_reconciliation(
                builder,
                c"example".as_ptr(),
                true,
                u32::MAX,
            ),
            LuminateStatus::InvalidArgument
        );
        assert_eq!(
            luminate_management_patch_builder_set_plugin_setting(
                builder,
                c"example".as_ptr(),
                c"key".as_ptr(),
                ptr::null(),
            ),
            LuminateStatus::NullPointer
        );
        luminate_management_patch_builder_free(builder);
        luminate_management_patch_builder_free(ptr::null_mut());
    }
}

#[test]
fn owned_setting_values_build_nested_arrays_and_tables() {
    let mut boolean = ptr::null_mut();
    let mut integer = ptr::null_mut();
    let mut number = ptr::null_mut();
    let mut string = ptr::null_mut();
    let mut array = ptr::null_mut();
    let mut table = ptr::null_mut();
    // SAFETY: all output slots are writable and returned values remain live.
    unsafe {
        assert_eq!(
            luminate_setting_value_new_boolean(true, &raw mut boolean),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_setting_value_new_integer(-4, &raw mut integer),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_setting_value_new_number(2.5, &raw mut number),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_setting_value_new_string(c"hello".as_ptr(), &raw mut string),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_setting_value_new_array(&raw mut array),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_setting_value_new_table(&raw mut table),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_setting_value_array_push(array, boolean),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_setting_value_array_push(array, integer),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_setting_value_table_insert(table, c"items".as_ptr(), array),
            LuminateStatus::Ok
        );

        assert!(matches!((*boolean).0, SettingValue::Boolean(true)));
        assert!(matches!((*integer).0, SettingValue::Integer(-4)));
        assert!(matches!((*number).0, SettingValue::Number(2.5)));
        assert!(matches!(&(*string).0, SettingValue::String(value) if value == "hello"));
        assert!(matches!(&(*array).0, SettingValue::Array(values) if values.len() == 2));
        assert!(matches!(&(*table).0, SettingValue::Table(values) if values.len() == 1));

        assert_eq!(
            luminate_setting_value_new_number(f64::INFINITY, &raw mut number),
            LuminateStatus::InvalidArgument
        );
        assert_eq!(
            luminate_setting_value_array_push(ptr::null_mut(), boolean),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_setting_value_array_push(boolean, integer),
            LuminateStatus::InvalidArgument
        );
        assert_eq!(
            luminate_setting_value_table_insert(boolean, c"x".as_ptr(), integer),
            LuminateStatus::InvalidArgument
        );
        assert_eq!(
            luminate_setting_value_table_insert(table, c"x".as_ptr(), ptr::null()),
            LuminateStatus::NullPointer
        );

        for value in [boolean, integer, number, string, array, table] {
            luminate_setting_value_free(value);
        }
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "The snapshot and its borrowed nested views stay in one scope to demonstrate their lifetime contract."
)]
fn snapshot_preferences_and_plugin_accessors_cover_nested_views() {
    let snapshot = LuminateManagementSnapshot(ManagementSnapshot {
        revision: 42,
        desired_daemon: preferences(),
        effective_daemon: DaemonPreferences {
            default_unsupported_policy: Some(UnsupportedPolicy::Skip),
            reconciliation_policy: Some(ReconciliationPolicy::Leave),
            device_reconciliation: Vec::new(),
            cct_emulation: Some(CctEmulation::Auto),
            prefer_shm: None,
            prefer_client_shm: None,
        },
        locked_daemon_settings: vec!["prefer-shm".to_owned()],
        plugins: vec![plugin(PluginRuntimeState::Failed {
            diagnostic: "boom".to_owned(),
        })],
    });
    // SAFETY: every pointer is borrowed from `snapshot`, which remains live.
    unsafe {
        assert_eq!(
            luminate_management_snapshot_revision(&raw const snapshot),
            42
        );
        assert_eq!(
            luminate_management_snapshot_locked_daemon_setting_count(&raw const snapshot),
            1
        );
        assert_eq!(
            bytes(luminate_management_snapshot_locked_daemon_setting_at(
                &raw const snapshot,
                0
            )),
            Some(b"prefer-shm".as_slice())
        );
        assert!(
            bytes(luminate_management_snapshot_locked_daemon_setting_at(
                &raw const snapshot,
                1
            ))
            .is_none()
        );
        assert_eq!(
            luminate_management_snapshot_plugin_count(&raw const snapshot),
            1
        );
        assert!(luminate_management_snapshot_plugin_at(&raw const snapshot, 1).is_null());

        let desired = luminate_management_snapshot_desired_daemon(&raw const snapshot);
        assert!(luminate_daemon_preferences_has_default_unsupported_policy(
            desired
        ));
        assert_eq!(
            luminate_daemon_preferences_default_unsupported_policy(desired),
            LUMINATE_UNSUPPORTED_POLICY_REJECT
        );
        assert!(luminate_daemon_preferences_has_reconciliation_policy(
            desired
        ));
        assert_eq!(
            luminate_daemon_preferences_reconciliation_policy(desired),
            LUMINATE_RECONCILIATION_POLICY_ADOPT
        );
        assert!(luminate_daemon_preferences_has_cct_emulation(desired));
        assert_eq!(
            luminate_daemon_preferences_cct_emulation(desired),
            LUMINATE_CCT_EMULATION_DISABLED
        );
        assert!(luminate_daemon_preferences_has_prefer_shm(desired));
        assert!(luminate_daemon_preferences_prefer_shm(desired));
        assert!(luminate_daemon_preferences_has_prefer_client_shm(desired));
        assert!(!luminate_daemon_preferences_prefer_client_shm(desired));
        assert_eq!(
            luminate_daemon_preferences_device_reconciliation_count(desired),
            2
        );
        let device = luminate_daemon_preferences_device_reconciliation_at(desired, 1);
        assert_eq!(
            bytes(luminate_device_reconciliation_preference_device_id(device)),
            Some(b"desk".as_slice())
        );
        assert_eq!(
            luminate_device_reconciliation_preference_policy(device),
            LUMINATE_RECONCILIATION_POLICY_LEAVE
        );

        let plugin = luminate_management_snapshot_plugin_at(&raw const snapshot, 0);
        assert_eq!(
            bytes(luminate_managed_plugin_name(plugin)),
            Some(b"example".as_slice())
        );
        assert_eq!(
            bytes(luminate_managed_plugin_version(plugin)),
            Some(b"1.2.3".as_slice())
        );
        assert!(luminate_managed_plugin_required(plugin));
        assert!(luminate_managed_plugin_has_desired_enabled(plugin));
        assert!(!luminate_managed_plugin_desired_enabled(plugin));
        assert!(luminate_managed_plugin_effective_enabled(plugin));
        assert!(luminate_managed_plugin_activation_locked(plugin));
        assert!(luminate_managed_plugin_has_desired_reconciliation(plugin));
        assert_eq!(
            luminate_managed_plugin_desired_reconciliation(plugin),
            LUMINATE_RECONCILIATION_POLICY_LEAVE
        );
        assert!(luminate_managed_plugin_has_effective_reconciliation(plugin));
        assert_eq!(
            luminate_managed_plugin_effective_reconciliation(plugin),
            LUMINATE_RECONCILIATION_POLICY_RESTORE
        );
        assert_eq!(
            luminate_managed_plugin_runtime_kind(plugin),
            LUMINATE_PLUGIN_RUNTIME_FAILED
        );
        assert_eq!(
            bytes(luminate_managed_plugin_runtime_diagnostic(plugin)),
            Some(b"boom".as_slice())
        );
        assert_eq!(luminate_managed_plugin_schema_count(plugin), 6);
        assert_eq!(luminate_managed_plugin_desired_setting_count(plugin), 2);
        assert_eq!(
            bytes(luminate_managed_plugin_desired_setting_key_at(plugin, 0)),
            Some(b"a".as_slice())
        );
        assert!(!luminate_managed_plugin_desired_setting_value_at(plugin, 1).is_null());
        assert_eq!(luminate_managed_plugin_effective_setting_count(plugin), 1);
        assert_eq!(
            bytes(luminate_managed_plugin_effective_setting_key_at(plugin, 0)),
            Some(b"secret".as_slice())
        );
        assert!(!luminate_managed_plugin_effective_setting_value_at(plugin, 0).is_null());
        assert_eq!(luminate_managed_plugin_locked_setting_count(plugin), 1);
        assert_eq!(
            bytes(luminate_managed_plugin_locked_setting_at(plugin, 0)),
            Some(b"secret".as_slice())
        );
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "One table-driven accessor walk keeps every setting discriminant and nested view comparable."
)]
fn schema_reported_and_recursive_setting_accessors_cover_every_variant() {
    let plugin = plugin(PluginRuntimeState::Loaded);
    let plugin_ptr: *const LuminateManagedPlugin = ptr::from_ref(&plugin).cast();
    // SAFETY: all borrowed pointers refer to live values owned by `plugin`.
    unsafe {
        for (index, expected_kind) in (0_u32..).enumerate().take(6) {
            let schema = luminate_managed_plugin_schema_at(plugin_ptr, index);
            assert!(!schema.is_null());
            assert_eq!(luminate_plugin_setting_schema_kind(schema), expected_kind);
            assert!(bytes(luminate_plugin_setting_schema_key(schema)).is_some());
            assert!(bytes(luminate_plugin_setting_schema_label(schema)).is_some());
            assert!(bytes(luminate_plugin_setting_schema_description(schema)).is_some());
            assert!(!luminate_plugin_setting_schema_default(schema).is_null());
            assert!(!luminate_plugin_setting_schema_constraints(schema).is_null());
            assert!(luminate_plugin_setting_schema_restart_required(schema));
        }
        let numeric = luminate_managed_plugin_schema_at(plugin_ptr, 2);
        assert!(luminate_plugin_setting_schema_sensitive(numeric));
        assert!(luminate_plugin_setting_schema_has_minimum(numeric));
        assert!((luminate_plugin_setting_schema_minimum(numeric) - 1.5).abs() <= f64::EPSILON);
        assert!(luminate_plugin_setting_schema_has_maximum(numeric));
        assert!((luminate_plugin_setting_schema_maximum(numeric) - 9.5).abs() <= f64::EPSILON);
        let required = luminate_managed_plugin_schema_at(plugin_ptr, 1);
        assert!(luminate_plugin_setting_schema_required(required));

        let values = [
            ReportedSettingValue::Unset,
            ReportedSettingValue::Visible(SettingValue::Boolean(true)),
            ReportedSettingValue::Redacted,
        ];
        for (value, expected) in values.iter().zip(0_u32..) {
            let value: *const LuminateReportedSettingValue = ptr::from_ref(value).cast();
            assert_eq!(luminate_reported_setting_value_kind(value), expected);
        }
        let visible: *const LuminateReportedSettingValue = ptr::from_ref(&values[1]).cast();
        assert!(!luminate_reported_setting_value_visible(visible).is_null());
        let unset: *const LuminateReportedSettingValue = ptr::from_ref(&values[0]).cast();
        assert!(luminate_reported_setting_value_visible(unset).is_null());

        let setting = SettingValue::Table(BTreeMap::from([
            (
                "array".to_owned(),
                SettingValue::Array(vec![SettingValue::Integer(7)]),
            ),
            ("boolean".to_owned(), SettingValue::Boolean(true)),
            ("number".to_owned(), SettingValue::Number(2.5)),
            (
                "string".to_owned(),
                SettingValue::String("hello".to_owned()),
            ),
        ]));
        let setting_ptr: *const LuminateSettingValueView = ptr::from_ref(&setting).cast();
        assert_eq!(
            luminate_setting_value_view_kind(setting_ptr),
            LUMINATE_MANAGEMENT_SETTING_TABLE
        );
        assert_eq!(luminate_setting_value_view_count(setting_ptr), 4);
        assert!(bytes(luminate_setting_value_view_table_key_at(setting_ptr, 0)).is_some());
        assert!(!luminate_setting_value_view_table_value_at(setting_ptr, 0).is_null());

        let array = SettingValue::Array(vec![SettingValue::Integer(7)]);
        let array_ptr: *const LuminateSettingValueView = ptr::from_ref(&array).cast();
        assert_eq!(
            luminate_setting_value_view_kind(array_ptr),
            LUMINATE_MANAGEMENT_SETTING_ARRAY
        );
        assert_eq!(luminate_setting_value_view_count(array_ptr), 1);
        assert!(!luminate_setting_value_view_array_at(array_ptr, 0).is_null());
        assert!(luminate_setting_value_view_array_at(array_ptr, 1).is_null());

        for (value, expected_kind) in [
            (
                SettingValue::Boolean(true),
                LUMINATE_MANAGEMENT_SETTING_BOOLEAN,
            ),
            (
                SettingValue::Integer(7),
                LUMINATE_MANAGEMENT_SETTING_INTEGER,
            ),
            (
                SettingValue::Number(2.5),
                LUMINATE_MANAGEMENT_SETTING_NUMBER,
            ),
            (
                SettingValue::String("hello".to_owned()),
                LUMINATE_MANAGEMENT_SETTING_STRING,
            ),
        ] {
            let value_ptr: *const LuminateSettingValueView = ptr::from_ref(&value).cast();
            assert_eq!(luminate_setting_value_view_kind(value_ptr), expected_kind);
            match value {
                SettingValue::Boolean(_) => assert!(luminate_setting_value_view_boolean(value_ptr)),
                SettingValue::Integer(_) => {
                    assert_eq!(luminate_setting_value_view_integer(value_ptr), 7);
                }
                SettingValue::Number(_) => {
                    assert!(
                        (luminate_setting_value_view_number(value_ptr) - 2.5).abs() <= f64::EPSILON
                    );
                }
                SettingValue::String(_) => assert_eq!(
                    bytes(luminate_setting_value_view_string(value_ptr)),
                    Some(b"hello".as_slice())
                ),
                _ => unreachable!("scalar cases only"),
            }
        }
    }
}

#[test]
fn visible_setting_values_clone_recursively_and_independently() {
    for source in [
        SettingValue::Boolean(true),
        SettingValue::Integer(-42),
        SettingValue::Number(1.25),
        SettingValue::String("scalar".to_owned()),
        SettingValue::Array(Vec::new()),
        SettingValue::Table(BTreeMap::new()),
    ] {
        let expected = source.clone();
        let mut cloned = ptr::null_mut();
        assert_eq!(
            // SAFETY: the borrowed scalar/container and output pointer are live.
            unsafe {
                luminate_setting_value_view_clone(ptr::from_ref(&source).cast(), &raw mut cloned)
            },
            LuminateStatus::Ok
        );
        drop(source);
        // SAFETY: `cloned` is the uniquely owned result above.
        unsafe {
            assert_eq!((*cloned).0, expected);
            luminate_setting_value_free(cloned);
        }
    }

    let source = ReportedSettingValue::Visible(SettingValue::Table(BTreeMap::from([
        (
            "nested".to_owned(),
            SettingValue::Array(vec![
                SettingValue::Boolean(true),
                SettingValue::Table(BTreeMap::from([(
                    "leaf".to_owned(),
                    SettingValue::String("value".to_owned()),
                )])),
            ]),
        ),
        ("integer".to_owned(), SettingValue::Integer(42)),
    ])));
    let expected = match &source {
        ReportedSettingValue::Visible(value) => value.clone(),
        _ => unreachable!("test constructs a visible value"),
    };
    let source_ptr: *const LuminateReportedSettingValue = ptr::from_ref(&source).cast();
    let mut cloned = ptr::null_mut();

    // SAFETY: the reported value, borrowed child, and output remain live.
    unsafe {
        let visible = luminate_reported_setting_value_visible(source_ptr);
        assert_eq!(
            luminate_setting_value_view_clone(visible, &raw mut cloned),
            LuminateStatus::Ok
        );
    }
    drop(source);
    // SAFETY: `cloned` is the uniquely owned output returned above.
    unsafe {
        assert_eq!((*cloned).0, expected);
        luminate_setting_value_free(cloned);
    }

    let sentinel = ptr::dangling_mut::<LuminateSettingValue>();
    let mut unchanged = sentinel;
    // SAFETY: null inputs exercise validation and must not write the output.
    unsafe {
        assert_eq!(
            luminate_setting_value_view_clone(ptr::null(), &raw mut unchanged),
            LuminateStatus::NullPointer
        );
        assert_eq!(unchanged, sentinel);
        for reported in [ReportedSettingValue::Unset, ReportedSettingValue::Redacted] {
            let reported: *const LuminateReportedSettingValue = ptr::from_ref(&reported).cast();
            assert!(luminate_reported_setting_value_visible(reported).is_null());
        }
        let scalar = SettingValue::Boolean(false);
        assert_eq!(
            luminate_setting_value_view_clone(ptr::from_ref(&scalar).cast(), ptr::null_mut()),
            LuminateStatus::NullPointer
        );
    }
}

#[test]
fn change_set_accessors_cover_every_change_shape_and_null_sentinels() {
    let changes = LuminateManagementChangeSet(ManagementChangeSet {
        revision: 8,
        changes: vec![
            ManagementChange::DaemonPreferencesChanged {
                keys: vec!["prefer-shm".to_owned()],
            },
            ManagementChange::PluginActivationChanged {
                plugin: "one".to_owned(),
            },
            ManagementChange::PluginReconciliationChanged {
                plugin: "two".to_owned(),
            },
            ManagementChange::PluginSettingChanged {
                plugin: "three".to_owned(),
                key: "secret".to_owned(),
                sensitive: true,
            },
        ],
    });
    // SAFETY: every returned change borrows from the live `changes` value.
    unsafe {
        assert_eq!(
            luminate_management_change_set_revision(&raw const changes),
            8
        );
        assert_eq!(luminate_management_change_set_count(&raw const changes), 4);
        assert!(luminate_management_change_set_at(&raw const changes, 4).is_null());
        for (index, expected_kind) in (0_u32..4).enumerate() {
            let change = luminate_management_change_set_at(&raw const changes, index);
            assert_eq!(luminate_management_change_kind(change), expected_kind);
        }
        let daemon = luminate_management_change_set_at(&raw const changes, 0);
        assert_eq!(luminate_management_change_key_count(daemon), 1);
        assert_eq!(
            bytes(luminate_management_change_key_at(daemon, 0)),
            Some(b"prefer-shm".as_slice())
        );
        assert!(bytes(luminate_management_change_plugin(daemon)).is_none());
        let activation = luminate_management_change_set_at(&raw const changes, 1);
        assert_eq!(
            bytes(luminate_management_change_plugin(activation)),
            Some(b"one".as_slice())
        );
        let setting = luminate_management_change_set_at(&raw const changes, 3);
        assert_eq!(luminate_management_change_key_count(setting), 1);
        assert_eq!(
            bytes(luminate_management_change_key_at(setting, 0)),
            Some(b"secret".as_slice())
        );
        assert!(luminate_management_change_setting_sensitive(setting));

        assert_eq!(luminate_management_snapshot_revision(ptr::null()), 0);
        assert_eq!(luminate_management_change_set_count(ptr::null()), 0);
        assert_eq!(luminate_management_change_kind(ptr::null()), u32::MAX);
        assert!(bytes(luminate_management_change_key_at(ptr::null(), 0)).is_none());
        assert!(bytes(luminate_managed_plugin_name(ptr::null())).is_none());
        assert_eq!(luminate_managed_plugin_runtime_kind(ptr::null()), u32::MAX);
        assert_eq!(luminate_setting_value_view_kind(ptr::null()), u32::MAX);
    }
}
