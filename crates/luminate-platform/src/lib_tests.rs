// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::{
    DLL_EXTENSION, DLL_PREFIX, DLL_SUFFIX, EXE_SUFFIX, dynamic_library_candidates, executable_name,
};

#[test]
fn extension_matches_the_current_platform_dll_extension() {
    assert_eq!(super::dynamic_library_extension(), DLL_EXTENSION);
}

#[test]
fn candidates_are_built_from_the_current_platform_prefix_and_suffix() {
    let candidates = dynamic_library_candidates("foo");
    assert_eq!(
        candidates,
        [
            format!("{DLL_PREFIX}foo{DLL_SUFFIX}"),
            format!("foo{DLL_SUFFIX}"),
        ]
    );
}

#[test]
fn candidates_preserve_the_requested_name() {
    for candidate in dynamic_library_candidates("luminate_policy_config_acl") {
        assert!(candidate.contains("luminate_policy_config_acl"));
    }
}

#[test]
fn executable_name_applies_the_current_platform_suffix() {
    assert_eq!(
        executable_name("luminated"),
        format!("luminated{EXE_SUFFIX}")
    );
    assert!(executable_name("luminated").starts_with("luminated"));
}
