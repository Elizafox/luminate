// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use super::*;

#[test]
fn string_descriptor_preserves_metadata_and_uses_no_bounds() {
    let descriptor = PluginSettingDescriptor::string(
        c"token",
        c"Token",
        c"Authentication token",
        c"\"default\"",
        true,
        true,
    );

    assert_eq!(descriptor.key, c"token".as_ptr());
    assert_eq!(descriptor.label, c"Token".as_ptr());
    assert_eq!(descriptor.description, c"Authentication token".as_ptr());
    assert_eq!(descriptor.kind, PluginSettingKind::String as u32);
    assert_eq!(descriptor.default_toml, c"\"default\"".as_ptr());
    assert!(descriptor.required);
    assert!(descriptor.sensitive);
    assert_eq!(
        descriptor.apply_mode,
        PluginSettingApplyMode::RestartRequired as u32
    );
    assert!(!descriptor.has_minimum);
    assert!(!descriptor.has_maximum);
    assert!(descriptor.constraints.is_null());
}
