// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn to_wide_appends_a_single_nul_terminator() {
    assert_eq!(to_wide("ab"), vec![u16::from(b'a'), u16::from(b'b'), 0]);
}

#[test]
fn to_wide_checked_rejects_embedded_nuls() {
    let error = to_wide_checked("Luminate\0Clients").expect_err("embedded NUL must be rejected");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn to_wide_checked_accepts_an_ordinary_name() {
    assert_eq!(
        to_wide_checked("Luminate Clients").expect("a NUL-free name is accepted"),
        to_wide("Luminate Clients")
    );
}
