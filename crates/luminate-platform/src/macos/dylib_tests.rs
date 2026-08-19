// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn parses_install_name_out_of_otool_output() {
    let output = "libluminate.dylib:\n@rpath/libluminate.4.dylib\n";
    assert_eq!(
        parse_macho_install_name(output),
        Some("@rpath/libluminate.4.dylib")
    );
}

#[test]
fn trims_whitespace_around_the_install_name_line() {
    let output = "libluminate.dylib:\n   libluminate.dylib  \n";
    assert_eq!(parse_macho_install_name(output), Some("libluminate.dylib"));
}
