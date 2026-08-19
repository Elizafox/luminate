// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Static, inspectable plugin setting metadata.

use std::ffi::{CStr, c_char};
use std::ptr;

/// Stable setting-type codes stored in [`PluginSettingDescriptor::kind`].
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginSettingKind {
    Boolean = 1,
    Integer = 2,
    Number = 3,
    String = 4,
    Enumeration = 5,
    Array = 6,
}

/// Stable application-mode codes stored in
/// [`PluginSettingDescriptor::apply_mode`].
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginSettingApplyMode {
    RestartRequired = 1,
}

/// One setting in a plugin's static schema.
///
/// All strings are NUL-terminated static UTF-8. `default_toml` is either null
/// or a TOML scalar/array expression. Bounds are present according to
/// `has_minimum` and `has_maximum`. `constraints` is an optional
/// NUL-terminated TOML value containing kind-specific constraints, including
/// enumeration choices or an array element type.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PluginSettingDescriptor {
    pub key: *const c_char,
    pub label: *const c_char,
    pub description: *const c_char,
    pub kind: u32,
    pub default_toml: *const c_char,
    pub required: bool,
    pub sensitive: bool,
    pub apply_mode: u32,
    pub minimum: f64,
    pub maximum: f64,
    pub has_minimum: bool,
    pub has_maximum: bool,
    pub constraints: *const c_char,
}

// SAFETY: setting descriptors are immutable metadata whose pointers reference
// static storage owned by the plugin image.
unsafe impl Sync for PluginSettingDescriptor {}

impl PluginSettingDescriptor {
    /// Builds a restart-required string setting without numeric bounds.
    #[must_use]
    pub const fn string(
        key: &'static CStr,
        label: &'static CStr,
        description: &'static CStr,
        default_toml: &'static CStr,
        required: bool,
        sensitive: bool,
    ) -> Self {
        Self {
            key: key.as_ptr(),
            label: label.as_ptr(),
            description: description.as_ptr(),
            kind: PluginSettingKind::String as u32,
            default_toml: default_toml.as_ptr(),
            required,
            sensitive,
            apply_mode: PluginSettingApplyMode::RestartRequired as u32,
            minimum: 0.0,
            maximum: 0.0,
            has_minimum: false,
            has_maximum: false,
            constraints: ptr::null(),
        }
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
