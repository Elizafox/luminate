// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Human-readable plugin-management output.

use std::fmt::Write as _;

use luminate::{
    ManagedPlugin, PluginRuntimeState, PluginSettingApplyMode, PluginSettingKind,
    ReportedSettingValue, SettingValue,
};
use luminate_core::control::ReconciliationPolicy;
use serde_json::{Value, json};

use crate::output::terminal_safe;

pub(crate) fn format_plugin_list(revision: u64, plugins: &[ManagedPlugin]) -> String {
    let mut output = format!("revision: {revision}\n");
    if plugins.is_empty() {
        output.push_str("(no installed plugins)\n");
        return output;
    }

    for plugin in plugins {
        let _ = writeln!(
            output,
            "{} {}",
            terminal_safe(&plugin.name),
            terminal_safe(&plugin.version)
        );
        let _ = writeln!(
            output,
            "  reconciliation: desired={}, effective={}",
            format_reconciliation(plugin.desired_reconciliation),
            format_reconciliation(plugin.effective_reconciliation)
        );
        let _ = writeln!(
            output,
            "  activation: desired={}, effective={}{}",
            format_desired_activation(plugin.desired_enabled),
            enabled(plugin.effective_enabled),
            if plugin.activation_locked {
                ", locked"
            } else {
                ""
            }
        );
        let _ = writeln!(output, "  runtime: {}", format_runtime(&plugin.runtime));
        if plugin.required {
            output.push_str("  required: yes\n");
        }
    }

    output
}

pub(crate) fn format_plugin(plugin: &ManagedPlugin) -> String {
    let mut output = format!(
        "{} {}\n",
        terminal_safe(&plugin.name),
        terminal_safe(&plugin.version)
    );
    let _ = writeln!(output, "required: {}", yes_no(plugin.required));
    let _ = writeln!(
        output,
        "activation: desired={}, effective={}, locked={}",
        format_desired_activation(plugin.desired_enabled),
        enabled(plugin.effective_enabled),
        yes_no(plugin.activation_locked)
    );
    let _ = writeln!(
        output,
        "reconciliation: desired={}, effective={}",
        format_reconciliation(plugin.desired_reconciliation),
        format_reconciliation(plugin.effective_reconciliation)
    );
    let _ = writeln!(output, "runtime: {}", format_runtime(&plugin.runtime));
    output.push_str("settings:\n");

    if plugin.schema.is_empty() {
        output.push_str("  (none)\n");
        return output;
    }

    for setting in &plugin.schema {
        let locked = plugin.locked_settings.iter().any(|key| key == &setting.key);
        let _ = writeln!(
            output,
            "  {} ({}, {})",
            terminal_safe(&setting.key),
            format_kind(setting.kind),
            format_apply_mode(setting.apply_mode)
        );
        let _ = writeln!(output, "    label: {}", terminal_safe(&setting.label));
        let _ = writeln!(
            output,
            "    description: {}",
            terminal_safe(&setting.description)
        );
        let _ = writeln!(
            output,
            "    required: {}; sensitive: {}; locked: {}",
            yes_no(setting.required),
            yes_no(setting.sensitive),
            yes_no(locked)
        );
        let _ = writeln!(
            output,
            "    desired: {}",
            format_reported(
                plugin
                    .desired_settings
                    .get(&setting.key)
                    .unwrap_or(&ReportedSettingValue::Unset)
            )
        );
        let _ = writeln!(
            output,
            "    effective: {}",
            format_reported(
                plugin
                    .effective_settings
                    .get(&setting.key)
                    .unwrap_or(&ReportedSettingValue::Unset)
            )
        );
        let _ = writeln!(output, "    default: {}", format_reported(&setting.default));
        if let Some(minimum) = setting.minimum {
            let _ = writeln!(output, "    minimum: {minimum}");
        }
        if let Some(maximum) = setting.maximum {
            let _ = writeln!(output, "    maximum: {maximum}");
        }
        if setting.constraints != ReportedSettingValue::Unset {
            let _ = writeln!(
                output,
                "    constraints: {}",
                format_reported(&setting.constraints)
            );
        }
    }

    output
}

pub(crate) fn plugin_list_json(revision: u64, plugins: &[ManagedPlugin]) -> Value {
    json!({
        "revision": revision,
        "plugins": plugins,
    })
}

pub(crate) fn plugin_json(revision: u64, plugin: &ManagedPlugin) -> Value {
    json!({
        "revision": revision,
        "plugin": plugin,
    })
}

fn format_desired_activation(value: Option<bool>) -> &'static str {
    value.map_or("global", enabled)
}

fn enabled(value: bool) -> &'static str {
    if value { "enabled" } else { "disabled" }
}

fn format_reconciliation(value: Option<ReconciliationPolicy>) -> &'static str {
    match value {
        None => "default",
        Some(ReconciliationPolicy::Leave) => "leave",
        Some(ReconciliationPolicy::Adopt) => "adopt",
        Some(ReconciliationPolicy::Restore) => "restore",
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn format_runtime(runtime: &PluginRuntimeState) -> String {
    match runtime {
        PluginRuntimeState::Inactive => "inactive".to_owned(),
        PluginRuntimeState::Loading => "loading".to_owned(),
        PluginRuntimeState::Loaded => "loaded".to_owned(),
        PluginRuntimeState::Failed { diagnostic } => {
            format!("failed: {}", terminal_safe(diagnostic))
        }
    }
}

fn format_kind(kind: PluginSettingKind) -> &'static str {
    match kind {
        PluginSettingKind::Boolean => "boolean",
        PluginSettingKind::Integer => "integer",
        PluginSettingKind::Number => "number",
        PluginSettingKind::String => "string",
        PluginSettingKind::Enumeration => "enumeration",
        PluginSettingKind::Array => "array",
    }
}

fn format_apply_mode(mode: PluginSettingApplyMode) -> &'static str {
    match mode {
        PluginSettingApplyMode::RestartRequired => "restart-required",
    }
}

fn format_reported(value: &ReportedSettingValue) -> String {
    match value {
        ReportedSettingValue::Unset => "(unset)".to_owned(),
        ReportedSettingValue::Redacted => "[REDACTED]".to_owned(),
        ReportedSettingValue::Visible(value) => format_value(value),
    }
}

fn format_value(value: &SettingValue) -> String {
    match value {
        SettingValue::Boolean(value) => value.to_string(),
        SettingValue::Integer(value) => value.to_string(),
        SettingValue::Number(value) => value.to_string(),
        SettingValue::String(value) => format!("{:?}", terminal_safe(value)),
        SettingValue::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(format_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        SettingValue::Table(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!("{} = {}", terminal_safe(key), format_value(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

#[cfg(test)]
#[path = "management_output_tests.rs"]
mod tests;
