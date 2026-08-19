// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Plugin-management formatting and JSON projection tests.

use std::collections::BTreeMap;
use std::slice;

use luminate::{PluginSettingSchema, ReportedSettingValue};

use super::*;

fn plugin() -> ManagedPlugin {
    ManagedPlugin {
        name: "demo\u{1b}[2J".to_owned(),
        version: "1.2.3".to_owned(),
        required: false,
        desired_enabled: Some(true),
        desired_reconciliation: Some(ReconciliationPolicy::Adopt),
        effective_reconciliation: Some(ReconciliationPolicy::Adopt),
        effective_enabled: true,
        runtime: PluginRuntimeState::Failed {
            diagnostic: "retry\nlater".to_owned(),
        },
        activation_locked: true,
        schema: vec![PluginSettingSchema {
            key: "credentials.token".to_owned(),
            label: "Token".to_owned(),
            description: "API token".to_owned(),
            kind: PluginSettingKind::String,
            default: ReportedSettingValue::Redacted,
            required: true,
            sensitive: true,
            apply_mode: PluginSettingApplyMode::RestartRequired,
            minimum: None,
            maximum: None,
            constraints: ReportedSettingValue::Redacted,
        }],
        desired_settings: BTreeMap::from([(
            "credentials.token".to_owned(),
            ReportedSettingValue::Redacted,
        )]),
        effective_settings: BTreeMap::from([(
            "credentials.token".to_owned(),
            ReportedSettingValue::Redacted,
        )]),
        locked_settings: vec!["credentials.token".to_owned()],
    }
}

#[test]
fn list_reports_revision_and_activation_without_terminal_controls() {
    let output = format_plugin_list(7, &[plugin()]);

    assert!(output.contains("revision: 7"));
    assert!(output.contains("desired=enabled, effective=enabled, locked"));
    assert!(output.contains("failed: retry\\nlater"));
    assert!(
        output
            .lines()
            .all(|line| !line.chars().any(char::is_control))
    );
}

#[test]
fn detail_preserves_redaction_and_schema_metadata() {
    let output = format_plugin(&plugin());

    assert!(output.contains("credentials.token (string, restart-required)"));
    assert!(output.contains("required: yes; sensitive: yes; locked: yes"));
    assert_eq!(output.matches("[REDACTED]").count(), 4);
    assert!(!output.contains("secret"));
}

#[test]
fn json_views_retain_the_management_revision() {
    let plugin = plugin();
    let list = plugin_list_json(7, slice::from_ref(&plugin));
    let detail = plugin_json(7, &plugin);

    assert_eq!(list["revision"], 7);
    assert!(list["plugins"].is_array());
    assert_eq!(detail["revision"], 7);
    assert_eq!(detail["plugin"]["name"], "demo\u{1b}[2J");
    assert_eq!(
        detail["plugin"]["desired_settings"]["credentials.token"],
        "Redacted"
    );
}
