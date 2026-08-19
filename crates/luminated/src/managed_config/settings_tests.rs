// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;
use luminate_plugin_api::PluginSettingKind;

fn schema(key: &str, kind: PluginSettingKind, default: Option<toml::Value>) -> PluginSettingSchema {
    PluginSettingSchema {
        key: key.to_owned(),
        kind,
        default,
        required: false,
        sensitive: false,
        minimum: None,
        maximum: None,
        constraints: None,
    }
}

#[test]
fn merges_defaults_layers_and_locked_values_into_nested_tables() {
    let mut integer = schema(
        "brightness.limit",
        PluginSettingKind::Integer,
        Some(toml::Value::Integer(10)),
    );
    integer.required = true;
    let schemas = vec![integer, schema("mode", PluginSettingKind::String, None)];
    let global = toml::toml! {
        brightness = { limit = 20 }
        mode = "global"
    };
    let managed = toml::toml! {
        brightness = { limit = 30 }
        mode = "managed"
    };
    let merged = merge_plugin_settings(
        &global,
        &managed,
        &["brightness.limit".to_owned()],
        &schemas,
    )
    .expect("settings should merge");
    assert_eq!(merged["brightness"]["limit"].as_integer(), Some(20));
    assert_eq!(merged["mode"].as_str(), Some("managed"));
}

#[test]
fn validates_numeric_enum_array_and_key_constraints() {
    let mut bounded = schema("value", PluginSettingKind::Integer, None);
    bounded.minimum = Some(1.0);
    bounded.maximum = Some(3.0);
    assert!(validate_setting_value(&bounded, &toml::Value::Integer(2)).is_ok());
    assert!(validate_setting_value(&bounded, &toml::Value::Integer(4)).is_err());

    let mut enumeration = schema("mode", PluginSettingKind::Enumeration, None);
    enumeration.constraints = Some(toml::toml! { choices = ["a", "b"] }.into());
    assert!(validate_setting_value(&enumeration, &toml::Value::String("b".into())).is_ok());
    assert!(validate_setting_value(&enumeration, &toml::Value::String("c".into())).is_err());

    let mut array = schema("names", PluginSettingKind::Array, None);
    array.constraints = Some(toml::toml! { element_kind = "string" }.into());
    assert!(
        validate_setting_value(
            &array,
            &toml::Value::Array(vec![toml::Value::String("one".into())])
        )
        .is_ok()
    );
    assert!(
        validate_setting_value(&array, &toml::Value::Array(vec![toml::Value::Integer(1)])).is_err()
    );
    assert!(!valid_setting_key(""));
    assert!(!valid_setting_key("a..b"));
    assert!(valid_setting_key("a.b_c-2"));
}

#[test]
fn dotted_setting_updates_remove_empty_parent_tables_and_convert_values() {
    let mut table = toml::Table::new();
    set_dotted_setting(&mut table, "nested.value", toml::Value::Boolean(true))
        .expect("set dotted value");
    assert_eq!(table["nested"]["value"].as_bool(), Some(true));
    clear_dotted_setting(&mut table, "nested.value");
    assert!(table.is_empty());

    assert!(setting_value_to_toml(SettingValue::Number(f64::INFINITY)).is_err());
    let value = setting_value_to_toml(SettingValue::Table(
        [("enabled".into(), SettingValue::Boolean(true))]
            .into_iter()
            .collect(),
    ))
    .expect("nested value conversion");
    assert_eq!(value["enabled"].as_bool(), Some(true));
}
