// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Typed C bindings for managed daemon and plugin configuration.

use super::*;

use std::collections::BTreeMap;
use std::ffi::c_char;

use luminate_core::capability::CctEmulation;
use luminate_core::control::{ReconciliationPolicy, UnsupportedPolicy};
use luminate_core::device::DeviceId;

use crate::ffi::read_required_str;
use crate::{
    DaemonPreferences, DeviceReconciliationPreference, ManagedPlugin, ManagementChange,
    ManagementChangeSet, ManagementMutation, ManagementPatch, ManagementSnapshot,
    PluginRuntimeState, PluginSettingApplyMode, PluginSettingKind, PluginSettingSchema,
    ReportedSettingValue, SettingValue, WriteOnly,
};

pub type LuminateManagementSettingKind = u32;
pub const LUMINATE_MANAGEMENT_SETTING_BOOLEAN: LuminateManagementSettingKind = 0;
pub const LUMINATE_MANAGEMENT_SETTING_INTEGER: LuminateManagementSettingKind = 1;
pub const LUMINATE_MANAGEMENT_SETTING_NUMBER: LuminateManagementSettingKind = 2;
pub const LUMINATE_MANAGEMENT_SETTING_STRING: LuminateManagementSettingKind = 3;
pub const LUMINATE_MANAGEMENT_SETTING_ARRAY: LuminateManagementSettingKind = 4;
pub const LUMINATE_MANAGEMENT_SETTING_TABLE: LuminateManagementSettingKind = 5;

pub type LuminateReportedSettingKind = u32;
pub const LUMINATE_REPORTED_SETTING_UNSET: LuminateReportedSettingKind = 0;
pub const LUMINATE_REPORTED_SETTING_VISIBLE: LuminateReportedSettingKind = 1;
pub const LUMINATE_REPORTED_SETTING_REDACTED: LuminateReportedSettingKind = 2;

pub type LuminatePluginSettingKind = u32;
pub const LUMINATE_PLUGIN_SETTING_BOOLEAN: LuminatePluginSettingKind = 0;
pub const LUMINATE_PLUGIN_SETTING_INTEGER: LuminatePluginSettingKind = 1;
pub const LUMINATE_PLUGIN_SETTING_NUMBER: LuminatePluginSettingKind = 2;
pub const LUMINATE_PLUGIN_SETTING_STRING: LuminatePluginSettingKind = 3;
pub const LUMINATE_PLUGIN_SETTING_ENUMERATION: LuminatePluginSettingKind = 4;
pub const LUMINATE_PLUGIN_SETTING_ARRAY: LuminatePluginSettingKind = 5;

pub type LuminatePluginRuntimeStateKind = u32;
pub const LUMINATE_PLUGIN_RUNTIME_INACTIVE: LuminatePluginRuntimeStateKind = 0;
pub const LUMINATE_PLUGIN_RUNTIME_LOADING: LuminatePluginRuntimeStateKind = 1;
pub const LUMINATE_PLUGIN_RUNTIME_LOADED: LuminatePluginRuntimeStateKind = 2;
pub const LUMINATE_PLUGIN_RUNTIME_FAILED: LuminatePluginRuntimeStateKind = 3;

pub type LuminateManagementChangeKind = u32;
pub const LUMINATE_MANAGEMENT_CHANGE_DAEMON_PREFERENCES: LuminateManagementChangeKind = 0;
pub const LUMINATE_MANAGEMENT_CHANGE_PLUGIN_ACTIVATION: LuminateManagementChangeKind = 1;
pub const LUMINATE_MANAGEMENT_CHANGE_PLUGIN_RECONCILIATION: LuminateManagementChangeKind = 2;
pub const LUMINATE_MANAGEMENT_CHANGE_PLUGIN_SETTING: LuminateManagementChangeKind = 3;

pub type LuminateReconciliationPolicy = u32;
pub const LUMINATE_RECONCILIATION_POLICY_RESTORE: LuminateReconciliationPolicy = 0;
pub const LUMINATE_RECONCILIATION_POLICY_ADOPT: LuminateReconciliationPolicy = 1;
pub const LUMINATE_RECONCILIATION_POLICY_LEAVE: LuminateReconciliationPolicy = 2;

pub type LuminateUnsupportedPolicy = u32;
pub const LUMINATE_UNSUPPORTED_POLICY_SKIP: LuminateUnsupportedPolicy = 0;
pub const LUMINATE_UNSUPPORTED_POLICY_REJECT: LuminateUnsupportedPolicy = 1;

pub type LuminateCctEmulation = u32;
pub const LUMINATE_CCT_EMULATION_AUTO: LuminateCctEmulation = 0;
pub const LUMINATE_CCT_EMULATION_DISABLED: LuminateCctEmulation = 1;

/// Owned authoritative management snapshot.
pub struct LuminateManagementSnapshot(pub(crate) ManagementSnapshot);
/// Owned redacted result of an applied management patch.
pub struct LuminateManagementChangeSet(pub(crate) ManagementChangeSet);
/// Borrowed redacted management changes carried by an event.
pub struct LuminateManagementChangeSetView;
/// Mutable, reusable atomic management patch builder.
pub struct LuminateManagementPatchBuilder(pub(crate) ManagementPatch);
/// Owned recursive plugin setting value.
pub struct LuminateSettingValue(SettingValue);

/// Borrowed daemon-preferences view.
pub struct LuminateDaemonPreferences;
/// Borrowed per-device reconciliation preference.
pub struct LuminateDeviceReconciliationPreference;
/// Borrowed managed-plugin view.
pub struct LuminateManagedPlugin;
/// Borrowed plugin-setting schema.
pub struct LuminatePluginSettingSchema;
/// Borrowed reported setting value.
pub struct LuminateReportedSettingValue;
/// Borrowed recursive setting value.
pub struct LuminateSettingValueView;
/// Borrowed management change.
pub struct LuminateManagementChange;

/// Optional daemon preferences used by a patch mutation.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LuminateDaemonPreferencesInput {
    pub has_default_unsupported_policy: bool,
    pub default_unsupported_policy: LuminateUnsupportedPolicy,
    pub has_reconciliation_policy: bool,
    pub reconciliation_policy: LuminateReconciliationPolicy,
    pub device_reconciliation: *const LuminateDeviceReconciliationPreferenceInput,
    pub device_reconciliation_count: usize,
    pub has_cct_emulation: bool,
    pub cct_emulation: LuminateCctEmulation,
    pub has_prefer_shm: bool,
    pub prefer_shm: bool,
    pub has_prefer_client_shm: bool,
    pub prefer_client_shm: bool,
}

/// One per-device reconciliation preference used by a patch mutation.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LuminateDeviceReconciliationPreferenceInput {
    pub device_id: *const c_char,
    pub policy: LuminateReconciliationPolicy,
}

fn reconciliation(value: u32) -> Result<ReconciliationPolicy, LuminateStatus> {
    match value {
        LUMINATE_RECONCILIATION_POLICY_RESTORE => Ok(ReconciliationPolicy::Restore),
        LUMINATE_RECONCILIATION_POLICY_ADOPT => Ok(ReconciliationPolicy::Adopt),
        LUMINATE_RECONCILIATION_POLICY_LEAVE => Ok(ReconciliationPolicy::Leave),
        _ => {
            crate::ffi::set_last_error("invalid LuminateReconciliationPolicy");
            Err(LuminateStatus::InvalidArgument)
        }
    }
}

fn unsupported(value: u32) -> Result<UnsupportedPolicy, LuminateStatus> {
    match value {
        LUMINATE_UNSUPPORTED_POLICY_SKIP => Ok(UnsupportedPolicy::Skip),
        LUMINATE_UNSUPPORTED_POLICY_REJECT => Ok(UnsupportedPolicy::Reject),
        _ => {
            crate::ffi::set_last_error("invalid LuminateUnsupportedPolicy");
            Err(LuminateStatus::InvalidArgument)
        }
    }
}

fn cct_emulation(value: u32) -> Result<CctEmulation, LuminateStatus> {
    match value {
        LUMINATE_CCT_EMULATION_AUTO => Ok(CctEmulation::Auto),
        LUMINATE_CCT_EMULATION_DISABLED => Ok(CctEmulation::Disabled),
        _ => {
            crate::ffi::set_last_error("invalid LuminateCctEmulation");
            Err(LuminateStatus::InvalidArgument)
        }
    }
}

unsafe fn preferences_input(
    input: &LuminateDaemonPreferencesInput,
) -> Result<DaemonPreferences, LuminateStatus> {
    let default_unsupported_policy = input
        .has_default_unsupported_policy
        .then(|| unsupported(input.default_unsupported_policy))
        .transpose()?;
    let reconciliation_policy = input
        .has_reconciliation_policy
        .then(|| reconciliation(input.reconciliation_policy))
        .transpose()?;
    let cct_emulation = input
        .has_cct_emulation
        .then(|| cct_emulation(input.cct_emulation))
        .transpose()?;

    let device_reconciliation = if input.device_reconciliation_count == 0 {
        Vec::new()
    } else {
        if input.device_reconciliation.is_null() {
            crate::ffi::set_last_error("device reconciliation input pointer is null");
            return Err(LuminateStatus::NullPointer);
        }
        // SAFETY: the input contract requires this many initialized elements.
        let entries = unsafe {
            std::slice::from_raw_parts(
                input.device_reconciliation,
                input.device_reconciliation_count,
            )
        };
        let mut result = Vec::with_capacity(entries.len());
        for entry in entries {
            // SAFETY: the input contract requires a NUL-terminated string.
            let id = unsafe { read_required_str(entry.device_id, "device id") }?;
            result.push(DeviceReconciliationPreference {
                device: DeviceId::new(id),
                policy: reconciliation(entry.policy)?,
            });
        }
        result
    };

    Ok(DaemonPreferences {
        default_unsupported_policy,
        reconciliation_policy,
        device_reconciliation,
        cct_emulation,
        prefer_shm: input.has_prefer_shm.then_some(input.prefer_shm),
        prefer_client_shm: input
            .has_prefer_client_shm
            .then_some(input.prefer_client_shm),
    })
}

null_safe_free!(
    "Releases a management snapshot returned by a management get call. Null is a no-op.",
    luminate_management_snapshot_free,
    LuminateManagementSnapshot
);
null_safe_free!(
    "Releases a management change set returned by a management patch call. Null is a no-op.",
    luminate_management_change_set_free,
    LuminateManagementChangeSet
);
null_safe_free!(
    "Releases a management patch builder. Null is a no-op.",
    luminate_management_patch_builder_free,
    LuminateManagementPatchBuilder
);
null_safe_free!(
    "Releases an owned setting value. Null is a no-op.",
    luminate_setting_value_free,
    LuminateSettingValue
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_patch_builder_new(
    expected_revision: u64,
    out_builder: *mut *mut LuminateManagementPatchBuilder,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's pointer contract.
        if let Err(status) = unsafe {
            write_box(
                out_builder,
                LuminateManagementPatchBuilder(ManagementPatch {
                    expected_revision,
                    mutations: Vec::new(),
                }),
                "management patch builder",
            )
        } {
            return status;
        }
        clear_last_error();
        LuminateStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_patch_builder_set_expected_revision(
    builder: *mut LuminateManagementPatchBuilder,
    expected_revision: u64,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            crate::ffi::set_last_error("management patch builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        builder.0.expected_revision = expected_revision;
        clear_last_error();
        LuminateStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_patch_builder_set_daemon_preferences(
    builder: *mut LuminateManagementPatchBuilder,
    preferences: *const LuminateDaemonPreferencesInput,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            crate::ffi::set_last_error("management patch builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: required by the public pointer contract.
        let Some(preferences) = (unsafe { preferences.as_ref() }) else {
            crate::ffi::set_last_error("daemon preferences input pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: nested pointers are governed by the input contract.
        let preferences = match unsafe { preferences_input(preferences) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        builder
            .0
            .mutations
            .push(ManagementMutation::SetDaemonPreferences(preferences));
        clear_last_error();
        LuminateStatus::Ok
    })
}

fn read_optional_bool(has_value: bool, value: bool) -> Option<bool> {
    has_value.then_some(value)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_patch_builder_set_plugin_enabled(
    builder: *mut LuminateManagementPatchBuilder,
    plugin: *const c_char,
    has_enabled: bool,
    enabled: bool,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            crate::ffi::set_last_error("management patch builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: required by the public pointer contract.
        let plugin = match unsafe { read_required_str(plugin, "plugin name") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        builder
            .0
            .mutations
            .push(ManagementMutation::SetPluginEnabled {
                plugin,
                enabled: read_optional_bool(has_enabled, enabled),
            });
        clear_last_error();
        LuminateStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_patch_builder_set_plugin_reconciliation(
    builder: *mut LuminateManagementPatchBuilder,
    plugin: *const c_char,
    has_reconciliation: bool,
    reconciliation_value: LuminateReconciliationPolicy,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            crate::ffi::set_last_error("management patch builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: required by the public pointer contract.
        let plugin = match unsafe { read_required_str(plugin, "plugin name") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let value = if has_reconciliation {
            match reconciliation(reconciliation_value) {
                Ok(value) => Some(value),
                Err(status) => return status,
            }
        } else {
            None
        };
        builder
            .0
            .mutations
            .push(ManagementMutation::SetPluginReconciliation {
                plugin,
                reconciliation: value,
            });
        clear_last_error();
        LuminateStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_patch_builder_set_plugin_setting(
    builder: *mut LuminateManagementPatchBuilder,
    plugin: *const c_char,
    key: *const c_char,
    value: *const LuminateSettingValue,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            crate::ffi::set_last_error("management patch builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: required by the public pointer contract.
        let Some(value) = (unsafe { value.as_ref() }) else {
            crate::ffi::set_last_error("setting value pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: required by the public pointer contract.
        let plugin = match unsafe { read_required_str(plugin, "plugin name") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        // SAFETY: required by the public pointer contract.
        let key = match unsafe { read_required_str(key, "setting key") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        builder
            .0
            .mutations
            .push(ManagementMutation::SetPluginSetting {
                plugin,
                key,
                value: WriteOnly::new(value.0.clone()),
            });
        clear_last_error();
        LuminateStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_patch_builder_clear_plugin_setting(
    builder: *mut LuminateManagementPatchBuilder,
    plugin: *const c_char,
    key: *const c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            crate::ffi::set_last_error("management patch builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: required by the public pointer contract.
        let plugin = match unsafe { read_required_str(plugin, "plugin name") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        // SAFETY: required by the public pointer contract.
        let key = match unsafe { read_required_str(key, "setting key") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        builder
            .0
            .mutations
            .push(ManagementMutation::ClearPluginSetting { plugin, key });
        clear_last_error();
        LuminateStatus::Ok
    })
}

macro_rules! setting_constructor {
    ($name:ident, $ty:ty, $variant:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(
            value: $ty,
            out_value: *mut *mut LuminateSettingValue,
        ) -> LuminateStatus {
            ffi_guard(|| {
                // SAFETY: required by the public pointer contract.
                if let Err(status) = unsafe {
                    write_box(
                        out_value,
                        LuminateSettingValue(SettingValue::$variant(value)),
                        "setting value",
                    )
                } {
                    return status;
                }
                clear_last_error();
                LuminateStatus::Ok
            })
        }
    };
}

setting_constructor!(luminate_setting_value_new_boolean, bool, Boolean);
setting_constructor!(luminate_setting_value_new_integer, i64, Integer);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_setting_value_new_number(
    value: f64,
    out_value: *mut *mut LuminateSettingValue,
) -> LuminateStatus {
    ffi_guard(|| {
        if !value.is_finite() {
            crate::ffi::set_last_error("setting number must be finite");
            return LuminateStatus::InvalidArgument;
        }
        // SAFETY: required by the public pointer contract.
        if let Err(status) = unsafe {
            write_box(
                out_value,
                LuminateSettingValue(SettingValue::Number(value)),
                "setting value",
            )
        } {
            return status;
        }
        clear_last_error();
        LuminateStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_setting_value_new_string(
    value: *const c_char,
    out_value: *mut *mut LuminateSettingValue,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let value = match unsafe { read_required_str(value, "setting string") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        // SAFETY: required by the public pointer contract.
        if let Err(status) = unsafe {
            write_box(
                out_value,
                LuminateSettingValue(SettingValue::String(value)),
                "setting value",
            )
        } {
            return status;
        }
        clear_last_error();
        LuminateStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_setting_value_new_array(
    out_value: *mut *mut LuminateSettingValue,
) -> LuminateStatus {
    // SAFETY: forwarded unchanged to a function with the same out-pointer contract.
    unsafe { setting_empty(out_value, SettingValue::Array(Vec::new())) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_setting_value_new_table(
    out_value: *mut *mut LuminateSettingValue,
) -> LuminateStatus {
    // SAFETY: forwarded unchanged to a function with the same out-pointer contract.
    unsafe { setting_empty(out_value, SettingValue::Table(BTreeMap::new())) }
}

unsafe fn setting_empty(
    out_value: *mut *mut LuminateSettingValue,
    value: SettingValue,
) -> LuminateStatus {
    ffi_guard(|| {
        if let Err(status) = unsafe {
            // SAFETY: required by the caller's pointer contract.
            write_box(out_value, LuminateSettingValue(value), "setting value")
        } {
            return status;
        }
        clear_last_error();
        LuminateStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_setting_value_array_push(
    array: *mut LuminateSettingValue,
    value: *const LuminateSettingValue,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let Some(array) = (unsafe { array.as_mut() }) else {
            crate::ffi::set_last_error("setting array pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: required by the public pointer contract.
        let Some(value) = (unsafe { value.as_ref() }) else {
            crate::ffi::set_last_error("setting value pointer is null");
            return LuminateStatus::NullPointer;
        };
        let SettingValue::Array(values) = &mut array.0 else {
            crate::ffi::set_last_error("setting value is not an array");
            return LuminateStatus::InvalidArgument;
        };
        values.push(value.0.clone());
        clear_last_error();
        LuminateStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_setting_value_table_insert(
    table: *mut LuminateSettingValue,
    key: *const c_char,
    value: *const LuminateSettingValue,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let Some(table) = (unsafe { table.as_mut() }) else {
            crate::ffi::set_last_error("setting table pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: required by the public pointer contract.
        let Some(value) = (unsafe { value.as_ref() }) else {
            crate::ffi::set_last_error("setting value pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: required by the public pointer contract.
        let key = match unsafe { read_required_str(key, "setting table key") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let SettingValue::Table(values) = &mut table.0 else {
            crate::ffi::set_last_error("setting value is not a table");
            return LuminateStatus::InvalidArgument;
        };
        values.insert(key, value.0.clone());
        clear_last_error();
        LuminateStatus::Ok
    })
}

fn preferences_ptr(value: &DaemonPreferences) -> *const LuminateDaemonPreferences {
    cast_ref(value)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_snapshot_revision(
    snapshot: *const LuminateManagementSnapshot,
) -> u64 {
    // SAFETY: required by the public pointer contract.
    unsafe { snapshot.as_ref() }.map_or(0, |value| value.0.revision)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_snapshot_desired_daemon(
    snapshot: *const LuminateManagementSnapshot,
) -> *const LuminateDaemonPreferences {
    // SAFETY: required by the public pointer contract.
    unsafe { snapshot.as_ref() }.map_or(ptr::null(), |value| {
        preferences_ptr(&value.0.desired_daemon)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_snapshot_effective_daemon(
    snapshot: *const LuminateManagementSnapshot,
) -> *const LuminateDaemonPreferences {
    // SAFETY: required by the public pointer contract.
    unsafe { snapshot.as_ref() }.map_or(ptr::null(), |value| {
        preferences_ptr(&value.0.effective_daemon)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_snapshot_locked_daemon_setting_count(
    snapshot: *const LuminateManagementSnapshot,
) -> usize {
    // SAFETY: required by the public pointer contract.
    unsafe { snapshot.as_ref() }.map_or(0, |value| value.0.locked_daemon_settings.len())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_snapshot_locked_daemon_setting_at(
    snapshot: *const LuminateManagementSnapshot,
    index: usize,
) -> LuminateStringView {
    // SAFETY: required by the public pointer contract.
    unsafe { snapshot.as_ref() }
        .and_then(|value| value.0.locked_daemon_settings.get(index))
        .map_or_else(|| optional_sv(None), |value| sv(value))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_snapshot_plugin_count(
    snapshot: *const LuminateManagementSnapshot,
) -> usize {
    // SAFETY: required by the public pointer contract.
    unsafe { snapshot.as_ref() }.map_or(0, |value| value.0.plugins.len())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_snapshot_plugin_at(
    snapshot: *const LuminateManagementSnapshot,
    index: usize,
) -> *const LuminateManagedPlugin {
    // SAFETY: required by the public pointer contract.
    unsafe { snapshot.as_ref() }
        .and_then(|value| value.0.plugins.get(index))
        .map_or(ptr::null(), cast_ref)
}

macro_rules! optional_enum_accessors {
    ($has:ident, $get:ident, $c_ty:ty, $native:ty, $field:ident, $map:expr) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $has(value: *const LuminateDaemonPreferences) -> bool {
            native_ref!(value, DaemonPreferences).is_some_and(|value| value.$field.is_some())
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $get(value: *const LuminateDaemonPreferences) -> $c_ty {
            native_ref!(value, DaemonPreferences)
                .and_then(|value| value.$field)
                .map_or(u32::MAX, $map)
        }
    };
}

optional_enum_accessors!(
    luminate_daemon_preferences_has_default_unsupported_policy,
    luminate_daemon_preferences_default_unsupported_policy,
    LuminateUnsupportedPolicy,
    UnsupportedPolicy,
    default_unsupported_policy,
    |value| match value {
        UnsupportedPolicy::Skip => LUMINATE_UNSUPPORTED_POLICY_SKIP,
        UnsupportedPolicy::Reject => LUMINATE_UNSUPPORTED_POLICY_REJECT,
    }
);
optional_enum_accessors!(
    luminate_daemon_preferences_has_reconciliation_policy,
    luminate_daemon_preferences_reconciliation_policy,
    LuminateReconciliationPolicy,
    ReconciliationPolicy,
    reconciliation_policy,
    |value| match value {
        ReconciliationPolicy::Restore => LUMINATE_RECONCILIATION_POLICY_RESTORE,
        ReconciliationPolicy::Adopt => LUMINATE_RECONCILIATION_POLICY_ADOPT,
        ReconciliationPolicy::Leave => LUMINATE_RECONCILIATION_POLICY_LEAVE,
    }
);
optional_enum_accessors!(
    luminate_daemon_preferences_has_cct_emulation,
    luminate_daemon_preferences_cct_emulation,
    LuminateCctEmulation,
    CctEmulation,
    cct_emulation,
    |value| match value {
        CctEmulation::Auto => LUMINATE_CCT_EMULATION_AUTO,
        CctEmulation::Disabled => LUMINATE_CCT_EMULATION_DISABLED,
    }
);

macro_rules! optional_bool_accessors {
    ($has:ident, $get:ident, $field:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $has(value: *const LuminateDaemonPreferences) -> bool {
            native_ref!(value, DaemonPreferences).is_some_and(|value| value.$field.is_some())
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $get(value: *const LuminateDaemonPreferences) -> bool {
            native_ref!(value, DaemonPreferences)
                .and_then(|value| value.$field)
                .unwrap_or(false)
        }
    };
}

optional_bool_accessors!(
    luminate_daemon_preferences_has_prefer_shm,
    luminate_daemon_preferences_prefer_shm,
    prefer_shm
);
optional_bool_accessors!(
    luminate_daemon_preferences_has_prefer_client_shm,
    luminate_daemon_preferences_prefer_client_shm,
    prefer_client_shm
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_daemon_preferences_device_reconciliation_count(
    value: *const LuminateDaemonPreferences,
) -> usize {
    native_ref!(value, DaemonPreferences).map_or(0, |value| value.device_reconciliation.len())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_daemon_preferences_device_reconciliation_at(
    value: *const LuminateDaemonPreferences,
    index: usize,
) -> *const LuminateDeviceReconciliationPreference {
    native_ref!(value, DaemonPreferences)
        .and_then(|value| value.device_reconciliation.get(index))
        .map_or(ptr::null(), cast_ref)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_device_reconciliation_preference_device_id(
    value: *const LuminateDeviceReconciliationPreference,
) -> LuminateStringView {
    native_ref!(value, DeviceReconciliationPreference)
        .map_or_else(|| optional_sv(None), |value| sv(value.device.as_str()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_device_reconciliation_preference_policy(
    value: *const LuminateDeviceReconciliationPreference,
) -> LuminateReconciliationPolicy {
    native_ref!(value, DeviceReconciliationPreference).map_or(u32::MAX, |value| {
        match value.policy {
            ReconciliationPolicy::Restore => LUMINATE_RECONCILIATION_POLICY_RESTORE,
            ReconciliationPolicy::Adopt => LUMINATE_RECONCILIATION_POLICY_ADOPT,
            ReconciliationPolicy::Leave => LUMINATE_RECONCILIATION_POLICY_LEAVE,
        }
    })
}

macro_rules! plugin_string_accessor {
    ($name:ident, $field:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(value: *const LuminateManagedPlugin) -> LuminateStringView {
            native_ref!(value, ManagedPlugin)
                .map_or_else(|| optional_sv(None), |value| sv(&value.$field))
        }
    };
}
plugin_string_accessor!(luminate_managed_plugin_name, name);
plugin_string_accessor!(luminate_managed_plugin_version, version);

macro_rules! plugin_bool_accessor {
    ($name:ident, $field:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(value: *const LuminateManagedPlugin) -> bool {
            native_ref!(value, ManagedPlugin).is_some_and(|value| value.$field)
        }
    };
}
plugin_bool_accessor!(luminate_managed_plugin_required, required);
plugin_bool_accessor!(luminate_managed_plugin_effective_enabled, effective_enabled);
plugin_bool_accessor!(luminate_managed_plugin_activation_locked, activation_locked);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_managed_plugin_has_desired_enabled(
    value: *const LuminateManagedPlugin,
) -> bool {
    native_ref!(value, ManagedPlugin).is_some_and(|value| value.desired_enabled.is_some())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_managed_plugin_desired_enabled(
    value: *const LuminateManagedPlugin,
) -> bool {
    native_ref!(value, ManagedPlugin)
        .and_then(|value| value.desired_enabled)
        .unwrap_or(false)
}

fn reconciliation_or_invalid(value: Option<ReconciliationPolicy>) -> u32 {
    value.map_or(u32::MAX, |value| match value {
        ReconciliationPolicy::Restore => LUMINATE_RECONCILIATION_POLICY_RESTORE,
        ReconciliationPolicy::Adopt => LUMINATE_RECONCILIATION_POLICY_ADOPT,
        ReconciliationPolicy::Leave => LUMINATE_RECONCILIATION_POLICY_LEAVE,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_managed_plugin_has_desired_reconciliation(
    value: *const LuminateManagedPlugin,
) -> bool {
    native_ref!(value, ManagedPlugin).is_some_and(|value| value.desired_reconciliation.is_some())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_managed_plugin_desired_reconciliation(
    value: *const LuminateManagedPlugin,
) -> LuminateReconciliationPolicy {
    native_ref!(value, ManagedPlugin).map_or(u32::MAX, |value| {
        reconciliation_or_invalid(value.desired_reconciliation)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_managed_plugin_has_effective_reconciliation(
    value: *const LuminateManagedPlugin,
) -> bool {
    native_ref!(value, ManagedPlugin).is_some_and(|value| value.effective_reconciliation.is_some())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_managed_plugin_effective_reconciliation(
    value: *const LuminateManagedPlugin,
) -> LuminateReconciliationPolicy {
    native_ref!(value, ManagedPlugin).map_or(u32::MAX, |value| {
        reconciliation_or_invalid(value.effective_reconciliation)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_managed_plugin_runtime_kind(
    value: *const LuminateManagedPlugin,
) -> LuminatePluginRuntimeStateKind {
    native_ref!(value, ManagedPlugin).map_or(u32::MAX, |value| match value.runtime {
        PluginRuntimeState::Inactive => LUMINATE_PLUGIN_RUNTIME_INACTIVE,
        PluginRuntimeState::Loading => LUMINATE_PLUGIN_RUNTIME_LOADING,
        PluginRuntimeState::Loaded => LUMINATE_PLUGIN_RUNTIME_LOADED,
        PluginRuntimeState::Failed { .. } => LUMINATE_PLUGIN_RUNTIME_FAILED,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_managed_plugin_runtime_diagnostic(
    value: *const LuminateManagedPlugin,
) -> LuminateStringView {
    native_ref!(value, ManagedPlugin).map_or_else(
        || optional_sv(None),
        |value| match &value.runtime {
            PluginRuntimeState::Failed { diagnostic } => sv(diagnostic),
            _ => optional_sv(None),
        },
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_managed_plugin_schema_count(
    value: *const LuminateManagedPlugin,
) -> usize {
    native_ref!(value, ManagedPlugin).map_or(0, |value| value.schema.len())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_managed_plugin_schema_at(
    value: *const LuminateManagedPlugin,
    index: usize,
) -> *const LuminatePluginSettingSchema {
    native_ref!(value, ManagedPlugin)
        .and_then(|value| value.schema.get(index))
        .map_or(ptr::null(), cast_ref)
}

fn map_entry_at(
    map: &BTreeMap<String, ReportedSettingValue>,
    index: usize,
) -> Option<(&String, &ReportedSettingValue)> {
    map.iter().nth(index)
}

macro_rules! reported_map_accessors {
    ($count:ident, $key:ident, $value:ident, $field:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $count(plugin: *const LuminateManagedPlugin) -> usize {
            native_ref!(plugin, ManagedPlugin).map_or(0, |plugin| plugin.$field.len())
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $key(
            plugin: *const LuminateManagedPlugin,
            index: usize,
        ) -> LuminateStringView {
            native_ref!(plugin, ManagedPlugin)
                .and_then(|plugin| map_entry_at(&plugin.$field, index))
                .map_or_else(|| optional_sv(None), |(key, _)| sv(key))
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $value(
            plugin: *const LuminateManagedPlugin,
            index: usize,
        ) -> *const LuminateReportedSettingValue {
            native_ref!(plugin, ManagedPlugin)
                .and_then(|plugin| map_entry_at(&plugin.$field, index))
                .map_or(ptr::null(), |(_, value)| cast_ref(value))
        }
    };
}

reported_map_accessors!(
    luminate_managed_plugin_desired_setting_count,
    luminate_managed_plugin_desired_setting_key_at,
    luminate_managed_plugin_desired_setting_value_at,
    desired_settings
);
reported_map_accessors!(
    luminate_managed_plugin_effective_setting_count,
    luminate_managed_plugin_effective_setting_key_at,
    luminate_managed_plugin_effective_setting_value_at,
    effective_settings
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_managed_plugin_locked_setting_count(
    plugin: *const LuminateManagedPlugin,
) -> usize {
    native_ref!(plugin, ManagedPlugin).map_or(0, |plugin| plugin.locked_settings.len())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_managed_plugin_locked_setting_at(
    plugin: *const LuminateManagedPlugin,
    index: usize,
) -> LuminateStringView {
    native_ref!(plugin, ManagedPlugin)
        .and_then(|plugin| plugin.locked_settings.get(index))
        .map_or_else(|| optional_sv(None), |value| sv(value))
}

macro_rules! schema_string_accessor {
    ($name:ident, $field:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(
            value: *const LuminatePluginSettingSchema,
        ) -> LuminateStringView {
            native_ref!(value, PluginSettingSchema)
                .map_or_else(|| optional_sv(None), |value| sv(&value.$field))
        }
    };
}
schema_string_accessor!(luminate_plugin_setting_schema_key, key);
schema_string_accessor!(luminate_plugin_setting_schema_label, label);
schema_string_accessor!(luminate_plugin_setting_schema_description, description);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setting_schema_kind(
    value: *const LuminatePluginSettingSchema,
) -> LuminatePluginSettingKind {
    native_ref!(value, PluginSettingSchema).map_or(u32::MAX, |value| match value.kind {
        PluginSettingKind::Boolean => LUMINATE_PLUGIN_SETTING_BOOLEAN,
        PluginSettingKind::Integer => LUMINATE_PLUGIN_SETTING_INTEGER,
        PluginSettingKind::Number => LUMINATE_PLUGIN_SETTING_NUMBER,
        PluginSettingKind::String => LUMINATE_PLUGIN_SETTING_STRING,
        PluginSettingKind::Enumeration => LUMINATE_PLUGIN_SETTING_ENUMERATION,
        PluginSettingKind::Array => LUMINATE_PLUGIN_SETTING_ARRAY,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setting_schema_default(
    value: *const LuminatePluginSettingSchema,
) -> *const LuminateReportedSettingValue {
    native_ref!(value, PluginSettingSchema).map_or(ptr::null(), |value| cast_ref(&value.default))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setting_schema_required(
    value: *const LuminatePluginSettingSchema,
) -> bool {
    native_ref!(value, PluginSettingSchema).is_some_and(|value| value.required)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setting_schema_sensitive(
    value: *const LuminatePluginSettingSchema,
) -> bool {
    native_ref!(value, PluginSettingSchema).is_some_and(|value| value.sensitive)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setting_schema_restart_required(
    value: *const LuminatePluginSettingSchema,
) -> bool {
    native_ref!(value, PluginSettingSchema)
        .is_some_and(|value| matches!(value.apply_mode, PluginSettingApplyMode::RestartRequired))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setting_schema_has_minimum(
    value: *const LuminatePluginSettingSchema,
) -> bool {
    native_ref!(value, PluginSettingSchema).is_some_and(|value| value.minimum.is_some())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setting_schema_minimum(
    value: *const LuminatePluginSettingSchema,
) -> f64 {
    native_ref!(value, PluginSettingSchema)
        .and_then(|value| value.minimum)
        .unwrap_or(0.0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setting_schema_has_maximum(
    value: *const LuminatePluginSettingSchema,
) -> bool {
    native_ref!(value, PluginSettingSchema).is_some_and(|value| value.maximum.is_some())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setting_schema_maximum(
    value: *const LuminatePluginSettingSchema,
) -> f64 {
    native_ref!(value, PluginSettingSchema)
        .and_then(|value| value.maximum)
        .unwrap_or(0.0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setting_schema_constraints(
    value: *const LuminatePluginSettingSchema,
) -> *const LuminateReportedSettingValue {
    native_ref!(value, PluginSettingSchema)
        .map_or(ptr::null(), |value| cast_ref(&value.constraints))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_reported_setting_value_kind(
    value: *const LuminateReportedSettingValue,
) -> LuminateReportedSettingKind {
    native_ref!(value, ReportedSettingValue).map_or(u32::MAX, |value| match value {
        ReportedSettingValue::Unset => LUMINATE_REPORTED_SETTING_UNSET,
        ReportedSettingValue::Visible(_) => LUMINATE_REPORTED_SETTING_VISIBLE,
        ReportedSettingValue::Redacted => LUMINATE_REPORTED_SETTING_REDACTED,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_reported_setting_value_visible(
    value: *const LuminateReportedSettingValue,
) -> *const LuminateSettingValueView {
    native_ref!(value, ReportedSettingValue).map_or(ptr::null(), |value| match value {
        ReportedSettingValue::Visible(value) => cast_ref(value),
        _ => ptr::null(),
    })
}

/// Deep-copies a borrowed visible setting value into an owned value. Nested
/// arrays, tables, strings, and keys are copied recursively. Release the
/// result with `luminate_setting_value_free`.
///
/// Only visible reported values yield a view that can be passed here; callers
/// must reject unset and redacted values using
/// `luminate_reported_setting_value_kind` first.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_setting_value_view_clone(
    view: *const LuminateSettingValueView,
    out_value: *mut *mut LuminateSettingValue,
) -> LuminateStatus {
    ffi_guard(|| {
        let Some(value) = (native_ref!(view, SettingValue)) else {
            crate::ffi::set_last_error("setting value view pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        if let Err(status) = unsafe {
            write_box(
                out_value,
                LuminateSettingValue(value.clone()),
                "setting value",
            )
        } {
            return status;
        }
        clear_last_error();
        LuminateStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_setting_value_view_kind(
    value: *const LuminateSettingValueView,
) -> LuminateManagementSettingKind {
    native_ref!(value, SettingValue).map_or(u32::MAX, |value| match value {
        SettingValue::Boolean(_) => LUMINATE_MANAGEMENT_SETTING_BOOLEAN,
        SettingValue::Integer(_) => LUMINATE_MANAGEMENT_SETTING_INTEGER,
        SettingValue::Number(_) => LUMINATE_MANAGEMENT_SETTING_NUMBER,
        SettingValue::String(_) => LUMINATE_MANAGEMENT_SETTING_STRING,
        SettingValue::Array(_) => LUMINATE_MANAGEMENT_SETTING_ARRAY,
        SettingValue::Table(_) => LUMINATE_MANAGEMENT_SETTING_TABLE,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_setting_value_view_boolean(
    value: *const LuminateSettingValueView,
) -> bool {
    native_ref!(value, SettingValue)
        .is_some_and(|value| matches!(value, SettingValue::Boolean(true)))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_setting_value_view_integer(
    value: *const LuminateSettingValueView,
) -> i64 {
    native_ref!(value, SettingValue).map_or(0, |value| match value {
        SettingValue::Integer(value) => *value,
        _ => 0,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_setting_value_view_number(
    value: *const LuminateSettingValueView,
) -> f64 {
    native_ref!(value, SettingValue).map_or(0.0, |value| match value {
        SettingValue::Number(value) => *value,
        _ => 0.0,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_setting_value_view_string(
    value: *const LuminateSettingValueView,
) -> LuminateStringView {
    native_ref!(value, SettingValue).map_or_else(
        || optional_sv(None),
        |value| match value {
            SettingValue::String(value) => sv(value),
            _ => optional_sv(None),
        },
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_setting_value_view_count(
    value: *const LuminateSettingValueView,
) -> usize {
    native_ref!(value, SettingValue).map_or(0, |value| match value {
        SettingValue::Array(values) => values.len(),
        SettingValue::Table(values) => values.len(),
        _ => 0,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_setting_value_view_array_at(
    value: *const LuminateSettingValueView,
    index: usize,
) -> *const LuminateSettingValueView {
    native_ref!(value, SettingValue).map_or(ptr::null(), |value| match value {
        SettingValue::Array(values) => values.get(index).map_or(ptr::null(), cast_ref),
        _ => ptr::null(),
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_setting_value_view_table_key_at(
    value: *const LuminateSettingValueView,
    index: usize,
) -> LuminateStringView {
    native_ref!(value, SettingValue).map_or_else(
        || optional_sv(None),
        |value| match value {
            SettingValue::Table(values) => values
                .iter()
                .nth(index)
                .map_or_else(|| optional_sv(None), |(key, _)| sv(key)),
            _ => optional_sv(None),
        },
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_setting_value_view_table_value_at(
    value: *const LuminateSettingValueView,
    index: usize,
) -> *const LuminateSettingValueView {
    native_ref!(value, SettingValue).map_or(ptr::null(), |value| match value {
        SettingValue::Table(values) => values
            .iter()
            .nth(index)
            .map_or(ptr::null(), |(_, value)| cast_ref(value)),
        _ => ptr::null(),
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_change_set_revision(
    value: *const LuminateManagementChangeSet,
) -> u64 {
    // SAFETY: required by the public pointer contract.
    unsafe { value.as_ref() }.map_or(0, |value| value.0.revision)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_change_set_count(
    value: *const LuminateManagementChangeSet,
) -> usize {
    // SAFETY: required by the public pointer contract.
    unsafe { value.as_ref() }.map_or(0, |value| value.0.changes.len())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_change_set_at(
    value: *const LuminateManagementChangeSet,
    index: usize,
) -> *const LuminateManagementChange {
    // SAFETY: required by the public pointer contract.
    unsafe { value.as_ref() }
        .and_then(|value| value.0.changes.get(index))
        .map_or(ptr::null(), cast_ref)
}

/// Revision committed by this borrowed event change set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_change_set_view_revision(
    value: *const LuminateManagementChangeSetView,
) -> u64 {
    native_ref!(value, ManagementChangeSet).map_or(0, |value| value.revision)
}

/// Number of redacted changes in this borrowed event change set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_change_set_view_count(
    value: *const LuminateManagementChangeSetView,
) -> usize {
    native_ref!(value, ManagementChangeSet).map_or(0, |value| value.changes.len())
}

/// Borrowed change at `index`, or null if out of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_change_set_view_at(
    value: *const LuminateManagementChangeSetView,
    index: usize,
) -> *const LuminateManagementChange {
    native_ref!(value, ManagementChangeSet)
        .and_then(|value| value.changes.get(index))
        .map_or(ptr::null(), cast_ref)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_change_kind(
    value: *const LuminateManagementChange,
) -> LuminateManagementChangeKind {
    native_ref!(value, ManagementChange).map_or(u32::MAX, |value| match value {
        ManagementChange::DaemonPreferencesChanged { .. } => {
            LUMINATE_MANAGEMENT_CHANGE_DAEMON_PREFERENCES
        }
        ManagementChange::PluginActivationChanged { .. } => {
            LUMINATE_MANAGEMENT_CHANGE_PLUGIN_ACTIVATION
        }
        ManagementChange::PluginReconciliationChanged { .. } => {
            LUMINATE_MANAGEMENT_CHANGE_PLUGIN_RECONCILIATION
        }
        ManagementChange::PluginSettingChanged { .. } => LUMINATE_MANAGEMENT_CHANGE_PLUGIN_SETTING,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_change_key_count(
    value: *const LuminateManagementChange,
) -> usize {
    native_ref!(value, ManagementChange).map_or(0, |value| match value {
        ManagementChange::DaemonPreferencesChanged { keys } => keys.len(),
        ManagementChange::PluginSettingChanged { .. } => 1,
        _ => 0,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_change_key_at(
    value: *const LuminateManagementChange,
    index: usize,
) -> LuminateStringView {
    native_ref!(value, ManagementChange).map_or_else(
        || optional_sv(None),
        |value| match value {
            ManagementChange::DaemonPreferencesChanged { keys } => keys
                .get(index)
                .map_or_else(|| optional_sv(None), |value| sv(value)),
            ManagementChange::PluginSettingChanged { key, .. } if index == 0 => sv(key),
            _ => optional_sv(None),
        },
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_change_plugin(
    value: *const LuminateManagementChange,
) -> LuminateStringView {
    native_ref!(value, ManagementChange).map_or_else(
        || optional_sv(None),
        |value| match value {
            ManagementChange::PluginActivationChanged { plugin }
            | ManagementChange::PluginReconciliationChanged { plugin }
            | ManagementChange::PluginSettingChanged { plugin, .. } => sv(plugin),
            ManagementChange::DaemonPreferencesChanged { .. } => optional_sv(None),
        },
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_management_change_setting_sensitive(
    value: *const LuminateManagementChange,
) -> bool {
    native_ref!(value, ManagementChange).is_some_and(|value| {
        matches!(
            value,
            ManagementChange::PluginSettingChanged {
                sensitive: true,
                ..
            }
        )
    })
}

fn finish_snapshot(
    result: crate::Result<ManagementSnapshot>,
) -> (LuminateStatus, *mut LuminateManagementSnapshot) {
    match result {
        Ok(value) => (
            LuminateStatus::Ok,
            Box::into_raw(Box::new(LuminateManagementSnapshot(value))),
        ),
        Err(error) => (store_error(&error), ptr::null_mut()),
    }
}

fn finish_changes(
    result: crate::Result<ManagementChangeSet>,
) -> (LuminateStatus, *mut LuminateManagementChangeSet) {
    match result {
        Ok(value) => (
            LuminateStatus::Ok,
            Box::into_raw(Box::new(LuminateManagementChangeSet(value))),
        ),
        Err(error) => (store_error(&error), ptr::null_mut()),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_get_management(
    client: *mut LuminateClient,
    out_snapshot: *mut *mut LuminateManagementSnapshot,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        if out_snapshot.is_null() {
            crate::ffi::set_last_error("management snapshot output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let result = match call_client(
            client,
            |client| async move { client.get_management().await },
        ) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let (status, value) = finish_snapshot(result);
        if status == LuminateStatus::Ok {
            // SAFETY: checked for null above.
            unsafe { *out_snapshot = value };
            clear_last_error();
        }
        status
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_patch_management(
    client: *mut LuminateClient,
    patch: *const LuminateManagementPatchBuilder,
    out_changes: *mut *mut LuminateManagementChangeSet,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        // SAFETY: required by the public pointer contract.
        let Some(patch) = (unsafe { patch.as_ref() }) else {
            crate::ffi::set_last_error("management patch builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        if out_changes.is_null() {
            crate::ffi::set_last_error("management change set output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let patch = patch.0.clone();
        let result = match call_client(client, move |client| async move {
            client.patch_management(patch).await
        }) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let (status, value) = finish_changes(result);
        if status == LuminateStatus::Ok {
            // SAFETY: checked for null above.
            unsafe { *out_changes = value };
            clear_last_error();
        }
        status
    })
}

#[cfg(test)]
#[path = "management_tests.rs"]
mod tests;
