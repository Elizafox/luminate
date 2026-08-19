// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn wide_strings_reject_embedded_nuls() {
    let error = to_wide_checked("Luminate\0Clients").expect_err("embedded NUL must be rejected");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn net_api_error_preserves_status_and_parameter() {
    let error = net_api_error(87, Some(2));
    assert_eq!(
        error.to_string(),
        "Windows local-group operation failed with status 87 (invalid parameter 2)"
    );
}
