// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn parses_soname_out_of_readelf_dynamic_section_output() {
    let output = "\n\
Dynamic section at offset 0x2df08 contains 30 entries:\n\
  Tag        Type                         Name/Value\n\
 0x000000000000000e (SONAME)             Library soname: [libluminate.so.4]\n\
 0x000000000000000c (INIT)               0x2000\n";
    assert_eq!(parse_elf_soname(output), Some("libluminate.so.4"));
}

#[test]
fn returns_none_when_readelf_output_has_no_soname_entry() {
    let output = " 0x000000000000000c (INIT)               0x2000\n";
    assert_eq!(parse_elf_soname(output), None);
}
