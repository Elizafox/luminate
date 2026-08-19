// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Resolving and validating one plugin's settings table.
//!
//! Settings are addressed by dotted key but stored as nested TOML tables, so
//! most of this module is the translation between those two views. Values are
//! checked against their declared kind and bounds before they can reach a
//! plugin, and locked keys are resolved from global configuration regardless
//! of what the managed layer asked for.

/// Resolves schema defaults, global settings, and unlocked managed overrides.
///
/// # Errors
///
/// Returns an error for duplicate schema keys, undeclared settings, values of
/// the wrong kind, missing required values, or conflicting dotted keys.
use std::collections::{BTreeMap, HashMap};

use anyhow::{Context as _, Result};
use luminate_protocol::SettingValue;

use super::preferences::daemon_setting_locked;
use super::schema::PluginSettingSchema;

pub fn merge_plugin_settings(
    global: &toml::Table,
    managed: &toml::Table,
    locked_settings: &[String],
    schema: &[PluginSettingSchema],
) -> Result<toml::Table> {
    let mut declarations = HashMap::new();
    let mut declared_keys: Vec<&str> = Vec::with_capacity(schema.len());
    let mut values = BTreeMap::new();

    for setting in schema {
        anyhow::ensure!(
            valid_setting_key(&setting.key),
            "invalid plugin setting schema key {:?}",
            setting.key
        );
        anyhow::ensure!(
            declared_keys.iter().all(|declared| {
                !dotted_key_contains(declared, &setting.key)
                    && !dotted_key_contains(&setting.key, declared)
            }),
            "conflicting plugin setting schema key {}",
            setting.key
        );
        anyhow::ensure!(
            declarations.insert(setting.key.as_str(), setting).is_none(),
            "duplicate plugin setting schema key {}",
            setting.key
        );
        declared_keys.push(&setting.key);
        if let Some(default) = &setting.default {
            validate_setting_value(setting, default)?;
            values.insert(setting.key.clone(), default.clone());
        }
    }

    let mut global_values = BTreeMap::new();
    flatten_settings(None, global, &mut global_values)?;
    for (key, value) in global_values {
        let setting = declarations
            .get(key.as_str())
            .with_context(|| format!("global configuration contains undeclared setting {key}"))?;
        validate_setting_value(setting, &value)?;
        values.insert(key, value);
    }

    let mut managed_values = BTreeMap::new();
    flatten_settings(None, managed, &mut managed_values)?;
    for (key, value) in managed_values {
        let setting = declarations
            .get(key.as_str())
            .with_context(|| format!("managed configuration contains undeclared setting {key}"))?;
        validate_setting_value(setting, &value)?;
        if !daemon_setting_locked(locked_settings, &key) {
            values.insert(key, value);
        }
    }

    for setting in schema {
        anyhow::ensure!(
            !setting.required || values.contains_key(&setting.key),
            "required plugin setting {} has no value",
            setting.key
        );
    }

    let mut merged = toml::Table::new();
    for (key, value) in values {
        insert_dotted_setting(&mut merged, &key, value)?;
    }
    Ok(merged)
}

pub(super) fn dotted_key_contains(parent: &str, child: &str) -> bool {
    child
        .strip_prefix(parent)
        .is_some_and(|suffix| suffix.starts_with('.'))
}

pub(super) fn valid_setting_key(key: &str) -> bool {
    !key.is_empty()
        && key.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        })
}

fn flatten_settings(
    prefix: Option<&str>,
    table: &toml::Table,
    flattened: &mut BTreeMap<String, toml::Value>,
) -> Result<()> {
    for (key, value) in table {
        let dotted = prefix.map_or_else(|| key.clone(), |prefix| format!("{prefix}.{key}"));
        if let toml::Value::Table(nested) = value {
            flatten_settings(Some(&dotted), nested, flattened)?;
        } else {
            anyhow::ensure!(
                flattened.insert(dotted.clone(), value.clone()).is_none(),
                "duplicate plugin setting {dotted}"
            );
        }
    }
    Ok(())
}

fn insert_dotted_setting(table: &mut toml::Table, key: &str, value: toml::Value) -> Result<()> {
    let mut parts = key.split('.').peekable();
    let mut current = table;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            anyhow::ensure!(
                current.insert(part.to_owned(), value).is_none(),
                "conflicting plugin setting key {key}"
            );
            return Ok(());
        }
        let entry = current
            .entry(part.to_owned())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        current = entry
            .as_table_mut()
            .with_context(|| format!("conflicting plugin setting key {key}"))?;
    }
    anyhow::bail!("plugin setting key must not be empty")
}

fn validate_setting_kind(setting: &PluginSettingSchema, value: &toml::Value) -> Result<()> {
    use luminate_plugin_api::PluginSettingKind::{
        Array, Boolean, Enumeration, Integer, Number, String,
    };

    let valid = match setting.kind {
        Boolean => value.is_bool(),
        Integer => value.is_integer(),
        Number => value.is_integer() || value.is_float(),
        String | Enumeration => value.is_str(),
        Array => value.is_array(),
    };
    anyhow::ensure!(
        valid,
        "plugin setting {} has the wrong value kind",
        setting.key
    );
    Ok(())
}

pub(super) fn validate_setting_value(
    setting: &PluginSettingSchema,
    value: &toml::Value,
) -> Result<()> {
    use luminate_plugin_api::PluginSettingKind::{
        Array, Boolean, Enumeration, Integer, Number, String,
    };

    validate_setting_kind(setting, value)?;

    #[allow(
        clippy::cast_precision_loss,
        reason = "the plugin ABI deliberately expresses numeric bounds as f64 for both integer and floating-point settings"
    )]
    let numeric_value = value
        .as_float()
        .or_else(|| value.as_integer().map(|value| value as f64));
    if let Some(number) = numeric_value {
        if let Some(minimum) = setting.minimum {
            anyhow::ensure!(
                number >= minimum,
                "plugin setting {} is below its minimum {minimum}",
                setting.key
            );
        }
        if let Some(maximum) = setting.maximum {
            anyhow::ensure!(
                number <= maximum,
                "plugin setting {} exceeds its maximum {maximum}",
                setting.key
            );
        }
    }

    match setting.kind {
        Enumeration => {
            if let Some(constraints) = &setting.constraints {
                let choices = constraints
                    .get("choices")
                    .and_then(toml::Value::as_array)
                    .context("enumeration setting constraints must contain a choices array")?;
                anyhow::ensure!(
                    choices.iter().any(|choice| choice == value),
                    "plugin setting {} is not one of its declared choices",
                    setting.key
                );
            }
        }
        Array => {
            if let Some(constraints) = &setting.constraints
                && let Some(element_kind) = constraints
                    .get("element_kind")
                    .and_then(toml::Value::as_str)
            {
                let values = value
                    .as_array()
                    .context("array setting value is not an array")?;
                anyhow::ensure!(
                    values
                        .iter()
                        .all(|value| setting_value_has_named_kind(value, element_kind)),
                    "plugin setting {} contains an element of the wrong kind",
                    setting.key
                );
            }
        }
        Boolean | Integer | Number | String => {}
    }

    Ok(())
}

fn setting_value_has_named_kind(value: &toml::Value, kind: &str) -> bool {
    match kind {
        "boolean" => value.is_bool(),
        "integer" => value.is_integer(),
        "number" => value.is_integer() || value.is_float(),
        "string" => value.is_str(),
        _ => false,
    }
}

pub(super) fn setting_value_to_toml(value: SettingValue) -> Result<toml::Value> {
    Ok(match value {
        SettingValue::Boolean(value) => toml::Value::Boolean(value),
        SettingValue::Integer(value) => toml::Value::Integer(value),
        SettingValue::Number(value) => {
            anyhow::ensure!(value.is_finite(), "plugin setting numbers must be finite");
            toml::Value::Float(value)
        }
        SettingValue::String(value) => toml::Value::String(value),
        SettingValue::Array(values) => toml::Value::Array(
            values
                .into_iter()
                .map(setting_value_to_toml)
                .collect::<Result<Vec<_>>>()?,
        ),
        SettingValue::Table(values) => toml::Value::Table(
            values
                .into_iter()
                .map(|(key, value)| setting_value_to_toml(value).map(|value| (key, value)))
                .collect::<Result<toml::Table>>()?,
        ),
    })
}

pub(super) fn set_dotted_setting(
    table: &mut toml::Table,
    key: &str,
    value: toml::Value,
) -> Result<()> {
    anyhow::ensure!(valid_setting_key(key), "invalid plugin setting key {key:?}");
    clear_dotted_setting(table, key);
    insert_dotted_setting(table, key, value)
}

pub(super) fn clear_dotted_setting(table: &mut toml::Table, key: &str) {
    let parts = key.split('.').collect::<Vec<_>>();
    clear_dotted_setting_parts(table, &parts);
}

fn clear_dotted_setting_parts(table: &mut toml::Table, parts: &[&str]) -> bool {
    let Some((head, tail)) = parts.split_first() else {
        return table.is_empty();
    };
    if tail.is_empty() {
        table.remove(*head);
    } else if let Some(child) = table.get_mut(*head).and_then(toml::Value::as_table_mut)
        && clear_dotted_setting_parts(child, tail)
    {
        table.remove(*head);
    }
    table.is_empty()
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
