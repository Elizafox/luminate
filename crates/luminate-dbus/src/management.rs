// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Native D-Bus representation of managed daemon and plugin configuration.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Display;

use luminate::capability::CctEmulation;
use luminate::control::ReconciliationPolicy;
use luminate::{
    DaemonPreferences, DeviceId, DeviceReconciliationPreference, ManagedPlugin, ManagementChange,
    ManagementChangeSet, ManagementMutation, ManagementPatch, ManagementSnapshot,
    PluginRuntimeState, PluginSettingApplyMode, PluginSettingKind, PluginSettingSchema,
    ReportedSettingValue, SettingValue, UnsupportedPolicy, WriteOnly,
};
use zbus::zvariant::{Error as ValueError, OwnedValue, Type, Value};

use crate::error::MethodError;

pub(crate) type Dictionary = HashMap<String, OwnedValue>;
pub(crate) type ChangeRecord = (String, String, Vec<String>, bool);

pub(crate) fn snapshot(value: ManagementSnapshot) -> Result<Dictionary, MethodError> {
    Ok(dictionary([
        ("Revision", owned(value.revision)?),
        (
            "DesiredDaemon",
            owned(daemon_preferences(value.desired_daemon)?)?,
        ),
        (
            "EffectiveDaemon",
            owned(daemon_preferences(value.effective_daemon)?)?,
        ),
        ("LockedDaemonSettings", owned(value.locked_daemon_settings)?),
        (
            "Plugins",
            owned(
                value
                    .plugins
                    .into_iter()
                    .map(plugin)
                    .collect::<Result<Vec<_>, _>>()?,
            )?,
        ),
    ]))
}

pub(crate) fn patch(
    expected_revision: u64,
    mutations: Vec<Dictionary>,
) -> Result<ManagementPatch, MethodError> {
    Ok(ManagementPatch {
        expected_revision,
        mutations: mutations
            .into_iter()
            .map(mutation)
            .collect::<Result<_, _>>()?,
    })
}

pub(crate) fn changes(value: ManagementChangeSet) -> (u64, Vec<ChangeRecord>) {
    let records = value
        .changes
        .into_iter()
        .map(|change| match change {
            ManagementChange::DaemonPreferencesChanged { keys } => {
                ("daemon-preferences".to_owned(), String::new(), keys, false)
            }
            ManagementChange::PluginActivationChanged { plugin } => {
                ("plugin-activation".to_owned(), plugin, Vec::new(), false)
            }
            ManagementChange::PluginReconciliationChanged { plugin } => (
                "plugin-reconciliation".to_owned(),
                plugin,
                Vec::new(),
                false,
            ),
            ManagementChange::PluginSettingChanged {
                plugin,
                key,
                sensitive,
            } => ("plugin-setting".to_owned(), plugin, vec![key], sensitive),
        })
        .collect();
    (value.revision, records)
}

fn mutation(mut value: Dictionary) -> Result<ManagementMutation, MethodError> {
    let kind = take::<String>(&mut value, "Kind")?;
    let mutation = match kind.as_str() {
        "set-daemon-preferences" => ManagementMutation::SetDaemonPreferences(
            parse_daemon_preferences(take(&mut value, "Preferences")?)?,
        ),
        "set-plugin-enabled" => {
            let plugin = take(&mut value, "Plugin")?;
            let enabled = if take::<bool>(&mut value, "HasEnabled")? {
                Some(take(&mut value, "Enabled")?)
            } else {
                value.remove("Enabled");
                None
            };
            ManagementMutation::SetPluginEnabled { plugin, enabled }
        }
        "set-plugin-reconciliation" => {
            let plugin = take(&mut value, "Plugin")?;
            let reconciliation = optional_string(&mut value, "Reconciliation")?
                .map(|value| parse_reconciliation(&value))
                .transpose()?;
            ManagementMutation::SetPluginReconciliation {
                plugin,
                reconciliation,
            }
        }
        "set-plugin-setting" => ManagementMutation::SetPluginSetting {
            plugin: take(&mut value, "Plugin")?,
            key: take(&mut value, "Key")?,
            value: WriteOnly::new(setting_value(
                value
                    .remove("Value")
                    .ok_or_else(|| missing_field("Value"))?,
            )?),
        },
        "clear-plugin-setting" => ManagementMutation::ClearPluginSetting {
            plugin: take(&mut value, "Plugin")?,
            key: take(&mut value, "Key")?,
        },
        _ => {
            return Err(invalid(format!(
                "unknown management mutation kind {kind:?}"
            )));
        }
    };
    if let Some(field) = value.keys().next() {
        return Err(invalid(format!(
            "unknown field {field:?} for mutation {kind:?}"
        )));
    }
    Ok(mutation)
}

fn parse_daemon_preferences(value: Dictionary) -> Result<DaemonPreferences, MethodError> {
    let mut value = value;
    let default_unsupported_policy = optional_string(&mut value, "DefaultUnsupportedPolicy")?
        .map(|value| match value.as_str() {
            "skip" => Ok(UnsupportedPolicy::Skip),
            "reject" => Ok(UnsupportedPolicy::Reject),
            _ => Err(invalid(format!("unknown unsupported policy {value:?}"))),
        })
        .transpose()?;
    let reconciliation_policy = optional_string(&mut value, "ReconciliationPolicy")?
        .map(|value| parse_reconciliation(&value))
        .transpose()?;
    let cct_emulation = optional_string(&mut value, "CctEmulation")?
        .map(|value| match value.as_str() {
            "auto" => Ok(CctEmulation::Auto),
            "disabled" => Ok(CctEmulation::Disabled),
            _ => Err(invalid(format!("unknown CCT emulation mode {value:?}"))),
        })
        .transpose()?;
    let prefer_shm = optional_bool(&mut value, "PreferShm")?;
    let prefer_client_shm = optional_bool(&mut value, "PreferClientShm")?;
    let device_reconciliation = take::<Vec<(String, String)>>(&mut value, "DeviceReconciliation")?
        .into_iter()
        .map(|(device, policy)| {
            Ok(DeviceReconciliationPreference {
                device: DeviceId::new(device),
                policy: parse_reconciliation(&policy)?,
            })
        })
        .collect::<Result<_, MethodError>>()?;
    if let Some(field) = value.keys().next() {
        return Err(invalid(format!(
            "unknown daemon preference field {field:?}"
        )));
    }
    Ok(DaemonPreferences {
        default_unsupported_policy,
        reconciliation_policy,
        device_reconciliation,
        cct_emulation,
        prefer_shm,
        prefer_client_shm,
    })
}

fn setting_value(value: OwnedValue) -> Result<SettingValue, MethodError> {
    match Value::from(value) {
        Value::Bool(value) => Ok(SettingValue::Boolean(value)),
        Value::I64(value) => Ok(SettingValue::Integer(value)),
        Value::F64(value) if value.is_finite() => Ok(SettingValue::Number(value)),
        Value::F64(_) => Err(invalid("setting numbers must be finite")),
        Value::Str(value) => Ok(SettingValue::String(value.to_string())),
        Value::Array(value) => value
            .iter()
            .map(|value| {
                OwnedValue::try_from(value)
                    .map_err(|error| dbus_value_error(&error))
                    .and_then(setting_value)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(SettingValue::Array),
        Value::Dict(value) => HashMap::<String, OwnedValue>::try_from(Value::Dict(value))
            .map_err(|error| dbus_value_error(&error))?
            .into_iter()
            .map(|(key, value)| Ok((key, setting_value(value)?)))
            .collect::<Result<BTreeMap<_, _>, MethodError>>()
            .map(SettingValue::Table),
        other @ (Value::U8(_)
        | Value::I16(_)
        | Value::U16(_)
        | Value::I32(_)
        | Value::U32(_)
        | Value::U64(_)
        | Value::Signature(_)
        | Value::ObjectPath(_)
        | Value::Value(_)
        | Value::Structure(_)) => Err(invalid(format!(
            "unsupported D-Bus setting value type {}",
            other.value_signature()
        ))),
        #[cfg(unix)]
        other @ Value::Fd(_) => Err(invalid(format!(
            "unsupported D-Bus setting value type {}",
            other.value_signature()
        ))),
    }
}

fn daemon_preferences(value: DaemonPreferences) -> Result<Dictionary, MethodError> {
    let mut result = Dictionary::new();
    insert_optional_string(
        &mut result,
        "DefaultUnsupportedPolicy",
        value.default_unsupported_policy.map(|value| match value {
            UnsupportedPolicy::Skip => "skip",
            UnsupportedPolicy::Reject => "reject",
        }),
    )?;
    insert_optional_string(
        &mut result,
        "ReconciliationPolicy",
        value.reconciliation_policy.map(reconciliation),
    )?;
    result.insert(
        "DeviceReconciliation".to_owned(),
        owned(
            value
                .device_reconciliation
                .into_iter()
                .map(|value| {
                    (
                        value.device.as_str().to_owned(),
                        reconciliation(value.policy).to_owned(),
                    )
                })
                .collect::<Vec<_>>(),
        )?,
    );
    insert_optional_string(
        &mut result,
        "CctEmulation",
        value.cct_emulation.map(|value| match value {
            CctEmulation::Auto => "auto",
            CctEmulation::Disabled => "disabled",
        }),
    )?;
    insert_optional_bool(&mut result, "PreferShm", value.prefer_shm)?;
    insert_optional_bool(&mut result, "PreferClientShm", value.prefer_client_shm)?;
    Ok(result)
}

fn plugin(value: ManagedPlugin) -> Result<Dictionary, MethodError> {
    Ok(dictionary([
        ("Name", owned(value.name)?),
        ("Version", owned(value.version)?),
        ("Required", owned(value.required)?),
        (
            "DesiredEnabled",
            owned(optional_bool_dict(value.desired_enabled)?)?,
        ),
        ("EffectiveEnabled", owned(value.effective_enabled)?),
        (
            "DesiredReconciliation",
            owned(optional_string_dict(
                value.desired_reconciliation.map(reconciliation),
            )?)?,
        ),
        (
            "EffectiveReconciliation",
            owned(optional_string_dict(
                value.effective_reconciliation.map(reconciliation),
            )?)?,
        ),
        ("Runtime", owned(runtime(value.runtime)?)?),
        ("ActivationLocked", owned(value.activation_locked)?),
        (
            "Schema",
            owned(
                value
                    .schema
                    .into_iter()
                    .map(schema)
                    .collect::<Result<Vec<_>, _>>()?,
            )?,
        ),
        (
            "DesiredSettings",
            owned(reported_map(value.desired_settings)?)?,
        ),
        (
            "EffectiveSettings",
            owned(reported_map(value.effective_settings)?)?,
        ),
        ("LockedSettings", owned(value.locked_settings)?),
    ]))
}

fn schema(value: PluginSettingSchema) -> Result<Dictionary, MethodError> {
    Ok(dictionary([
        ("Key", owned(value.key)?),
        ("Label", owned(value.label)?),
        ("Description", owned(value.description)?),
        ("Kind", owned(setting_kind(value.kind).to_owned())?),
        ("Default", owned(reported(value.default)?)?),
        ("Required", owned(value.required)?),
        ("Sensitive", owned(value.sensitive)?),
        ("ApplyMode", owned(apply_mode(value.apply_mode).to_owned())?),
        ("Minimum", owned(optional_f64_dict(value.minimum)?)?),
        ("Maximum", owned(optional_f64_dict(value.maximum)?)?),
        ("Constraints", owned(reported(value.constraints)?)?),
    ]))
}

fn reported_map(
    values: BTreeMap<String, ReportedSettingValue>,
) -> Result<HashMap<String, Dictionary>, MethodError> {
    values
        .into_iter()
        .map(|(key, value)| Ok((key, reported(value)?)))
        .collect()
}

fn reported(value: ReportedSettingValue) -> Result<Dictionary, MethodError> {
    match value {
        ReportedSettingValue::Unset => Ok(dictionary([("State", owned("unset".to_owned())?)])),
        ReportedSettingValue::Redacted => {
            Ok(dictionary([("State", owned("redacted".to_owned())?)]))
        }
        ReportedSettingValue::Visible(value) => Ok(dictionary([
            ("State", owned("visible".to_owned())?),
            ("Value", setting_owned(value)?),
        ])),
    }
}

fn setting_owned(value: SettingValue) -> Result<OwnedValue, MethodError> {
    match value {
        SettingValue::Boolean(value) => owned(value),
        SettingValue::Integer(value) => owned(value),
        SettingValue::Number(value) => owned(value),
        SettingValue::String(value) => owned(value),
        SettingValue::Array(values) => owned(
            values
                .into_iter()
                .map(setting_owned)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        SettingValue::Table(values) => owned(
            values
                .into_iter()
                .map(|(key, value)| Ok((key, setting_owned(value)?)))
                .collect::<Result<HashMap<_, _>, MethodError>>()?,
        ),
    }
}

fn runtime(value: PluginRuntimeState) -> Result<Dictionary, MethodError> {
    match value {
        PluginRuntimeState::Inactive => Ok(dictionary([("State", owned("inactive".to_owned())?)])),
        PluginRuntimeState::Loading => Ok(dictionary([("State", owned("loading".to_owned())?)])),
        PluginRuntimeState::Loaded => Ok(dictionary([("State", owned("loaded".to_owned())?)])),
        PluginRuntimeState::Failed { diagnostic } => Ok(dictionary([
            ("State", owned("failed".to_owned())?),
            ("Diagnostic", owned(diagnostic)?),
        ])),
    }
}

fn optional_bool_dict(value: Option<bool>) -> Result<Dictionary, MethodError> {
    let mut result = Dictionary::new();
    result.insert("Present".to_owned(), owned(value.is_some())?);
    if let Some(value) = value {
        result.insert("Value".to_owned(), owned(value)?);
    }
    Ok(result)
}

fn optional_string_dict(value: Option<&str>) -> Result<Dictionary, MethodError> {
    let mut result = Dictionary::new();
    result.insert("Present".to_owned(), owned(value.is_some())?);
    if let Some(value) = value {
        result.insert("Value".to_owned(), owned(value.to_owned())?);
    }
    Ok(result)
}

fn optional_f64_dict(value: Option<f64>) -> Result<Dictionary, MethodError> {
    let mut result = Dictionary::new();
    result.insert("Present".to_owned(), owned(value.is_some())?);
    if let Some(value) = value {
        result.insert("Value".to_owned(), owned(value)?);
    }
    Ok(result)
}

fn optional_string(value: &mut Dictionary, field: &str) -> Result<Option<String>, MethodError> {
    if take::<bool>(value, &format!("Has{field}"))? {
        take(value, field).map(Some)
    } else {
        value.remove(field);
        Ok(None)
    }
}

fn optional_bool(value: &mut Dictionary, field: &str) -> Result<Option<bool>, MethodError> {
    if take::<bool>(value, &format!("Has{field}"))? {
        take(value, field).map(Some)
    } else {
        value.remove(field);
        Ok(None)
    }
}

fn insert_optional_string(
    result: &mut Dictionary,
    field: &str,
    value: Option<&str>,
) -> Result<(), MethodError> {
    result.insert(format!("Has{field}"), owned(value.is_some())?);
    if let Some(value) = value {
        result.insert(field.to_owned(), owned(value.to_owned())?);
    }
    Ok(())
}

fn insert_optional_bool(
    result: &mut Dictionary,
    field: &str,
    value: Option<bool>,
) -> Result<(), MethodError> {
    result.insert(format!("Has{field}"), owned(value.is_some())?);
    if let Some(value) = value {
        result.insert(field.to_owned(), owned(value)?);
    }
    Ok(())
}

fn take<T>(value: &mut Dictionary, field: &str) -> Result<T, MethodError>
where
    T: TryFrom<OwnedValue>,
    T::Error: Display,
{
    value
        .remove(field)
        .ok_or_else(|| missing_field(field))?
        .try_into()
        .map_err(|error: T::Error| invalid(format!("{field} has the wrong type: {error}")))
}

pub(crate) fn dictionary<const N: usize>(fields: [(&str, OwnedValue); N]) -> Dictionary {
    fields
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

pub(crate) fn owned<T>(value: T) -> Result<OwnedValue, MethodError>
where
    T: Type + Into<Value<'static>>,
{
    OwnedValue::try_from(Value::new(value)).map_err(|error| dbus_value_error(&error))
}

fn parse_reconciliation(value: &str) -> Result<ReconciliationPolicy, MethodError> {
    match value {
        "restore" => Ok(ReconciliationPolicy::Restore),
        "adopt" => Ok(ReconciliationPolicy::Adopt),
        "leave" => Ok(ReconciliationPolicy::Leave),
        _ => Err(invalid(format!("unknown reconciliation policy {value:?}"))),
    }
}

fn reconciliation(value: ReconciliationPolicy) -> &'static str {
    match value {
        ReconciliationPolicy::Restore => "restore",
        ReconciliationPolicy::Adopt => "adopt",
        ReconciliationPolicy::Leave => "leave",
    }
}

fn setting_kind(value: PluginSettingKind) -> &'static str {
    match value {
        PluginSettingKind::Boolean => "boolean",
        PluginSettingKind::Integer => "integer",
        PluginSettingKind::Number => "number",
        PluginSettingKind::String => "string",
        PluginSettingKind::Enumeration => "enumeration",
        PluginSettingKind::Array => "array",
    }
}

fn apply_mode(value: PluginSettingApplyMode) -> &'static str {
    match value {
        PluginSettingApplyMode::RestartRequired => "restart-required",
    }
}

fn missing_field(field: &str) -> MethodError {
    invalid(format!("management request is missing {field}"))
}

fn dbus_value_error(error: &ValueError) -> MethodError {
    invalid(format!("invalid D-Bus management value: {error}"))
}

fn invalid(message: impl Into<String>) -> MethodError {
    MethodError::InvalidArgument(message.into())
}

#[cfg(test)]
#[path = "management_tests.rs"]
mod tests;
