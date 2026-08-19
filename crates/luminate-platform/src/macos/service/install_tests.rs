// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn validate_install_path_accepts_the_canonical_prefix() {
    let path = default_path::program_root().join("luminated");

    validate_install_path(&path, false).expect("canonical path must be accepted");
}

#[test]
fn validate_install_path_rejects_other_locations_without_the_override() {
    let error = validate_install_path(Path::new("/Users/dev/luminated"), false)
        .expect_err("non-canonical path must be rejected");
    assert!(matches!(error, ServiceError::ExecutablePath(_)));
}

#[test]
fn validate_install_path_accepts_any_location_with_the_override() {
    validate_install_path(Path::new("/Users/dev/luminated"), true)
        .expect("development override must be accepted");
}
