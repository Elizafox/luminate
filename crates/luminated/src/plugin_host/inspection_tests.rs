// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

fn inspected_setting() -> InspectedSetting {
    InspectedSetting {
        key: "connection.address".to_owned(),
        label: "Address".to_owned(),
        description: "Discovery address".to_owned(),
        kind: PluginSettingKind::String as u32,
        default_toml: Some("\"broadcast\"".to_owned()),
        required: true,
        sensitive: false,
        apply_mode: PluginSettingApplyMode::RestartRequired as u32,
        minimum: None,
        maximum: None,
        constraints_toml: Some(r#"{ choices = ["broadcast", "unicast"] }"#.to_owned()),
    }
}

#[test]
fn transported_setting_becomes_typed_metadata() {
    let setting = InspectedPluginSetting::try_from(inspected_setting()).expect("convert setting");

    assert_eq!(setting.key, "connection.address");
    assert_eq!(setting.label, "Address");
    assert_eq!(setting.description, "Discovery address");
    assert_eq!(setting.kind, PluginSettingKind::String);
    assert_eq!(
        setting.default,
        Some(toml::Value::String("broadcast".to_owned()))
    );
    assert!(setting.required);
    assert!(!setting.sensitive);
    assert_eq!(setting.apply_mode, PluginSettingApplyMode::RestartRequired);
    assert_eq!(setting.minimum, None);
    assert_eq!(setting.maximum, None);
    assert_eq!(
        setting
            .constraints
            .as_ref()
            .and_then(|value| value.get("choices"))
            .and_then(toml::Value::as_array)
            .map(Vec::len),
        Some(2)
    );
}

#[test]
fn transported_setting_rejects_unknown_codes_and_invalid_toml() {
    let mut setting = inspected_setting();
    setting.kind = u32::MAX;
    assert!(InspectedPluginSetting::try_from(setting).is_err());

    let mut setting = inspected_setting();
    setting.apply_mode = u32::MAX;
    assert!(InspectedPluginSetting::try_from(setting).is_err());

    let mut setting = inspected_setting();
    setting.default_toml = Some("invalid =".to_owned());
    assert!(InspectedPluginSetting::try_from(setting).is_err());

    let mut setting = inspected_setting();
    setting.constraints_toml = Some("invalid =".to_owned());
    assert!(InspectedPluginSetting::try_from(setting).is_err());
}
