// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Metadata describing what a plugin's settings may contain.
//!
//! These types are built from inspected plugin metadata rather than read
//! from disk, and they are what makes a management patch checkable: a patch
//! is validated against the schema its plugin actually declared.

/// Inputs which determine whether one installed plugin is desired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginActivationInputs {
    /// The plugin is listed explicitly in global configuration.
    pub explicit: bool,

    /// The plugin is an explicit required dependency of the daemon.
    pub required: bool,

    /// Static metadata qualifies the plugin for the global activation mode.
    pub automatically_selected: bool,
}

/// Validated setting metadata needed to resolve one plugin's configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginSettingSchema {
    /// Canonical dotted setting key.
    pub key: String,

    /// Expected TOML value kind.
    pub kind: luminate_plugin_api::PluginSettingKind,

    /// Schema default, when one is declared.
    pub default: Option<toml::Value>,

    /// Whether a value must remain after all layers are resolved.
    pub required: bool,

    /// Whether values must be redacted outside write-only requests.
    pub sensitive: bool,

    /// Inclusive numeric lower bound.
    pub minimum: Option<f64>,

    /// Inclusive numeric upper bound.
    pub maximum: Option<f64>,

    /// Kind-specific constraints supplied by the plugin.
    pub constraints: Option<toml::Value>,
}

/// Installed-plugin metadata required to validate management patches.
#[derive(Debug, Clone)]
pub struct ManageablePlugin {
    pub name: String,
    pub global_settings: toml::Table,
    pub locked_settings: Vec<String>,
    pub schema: Vec<PluginSettingSchema>,
}
