// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn allocate_id_picks_the_lowest_free_value_in_range() {
    let used = [200, 201, 203].into_iter();
    assert_eq!(allocate_id(used, 200..=400), Some(202));
}

#[test]
fn allocate_id_returns_none_when_the_range_is_exhausted() {
    let used = (200..=400).collect::<Vec<_>>().into_iter();
    assert_eq!(allocate_id(used, 200..=400), None);
}

#[test]
fn mutating_dscl_commands_include_the_local_node() {
    assert_eq!(
        dscl_command_args(&["-create", "/Groups/_luminate"]),
        [".", "-create", "/Groups/_luminate"]
    );
}

#[test]
fn group_requires_the_fixed_real_name_ownership_marker() {
    assert_eq!(
        classify_group(Some(200), Some(GROUP_REAL_NAME)),
        GroupPresence::Matches
    );
    assert!(matches!(
        classify_group(Some(200), None),
        GroupPresence::Conflicts(reason) if reason.contains("RealName")
    ));
    assert!(matches!(
        classify_group(Some(200), Some("Unrelated Group")),
        GroupPresence::Conflicts(reason) if reason.contains("RealName")
    ));
}

#[test]
fn parse_single_valued_attribute_extracts_the_value() {
    let output = "dsAttrTypeStandard:RealName\nRealName: Luminate Lighting Daemon\n";
    assert_eq!(
        parse_single_valued_attribute(output, "RealName"),
        Some("Luminate Lighting Daemon".to_owned())
    );
}

#[test]
fn parse_single_valued_attribute_extracts_a_multiline_value() {
    let output = "RealName:\n Luminate Lighting Daemon\nRecordName: _luminated\n";
    assert_eq!(
        parse_single_valued_attribute(output, "RealName"),
        Some("Luminate Lighting Daemon".to_owned())
    );
}

#[test]
fn parse_single_valued_attribute_returns_none_when_absent() {
    let output = "dsAttrTypeStandard:RealName\n";
    assert_eq!(parse_single_valued_attribute(output, "RealName"), None);
}

#[test]
fn parse_id_list_extracts_trailing_numeric_values() {
    let output = "_amavisd 83\n_appleevents 55\nmyuser 501\n";
    let mut ids = parse_id_list(output);
    ids.sort_unstable();
    assert_eq!(ids, vec![55, 83, 501]);
}

#[test]
fn parse_id_list_skips_lines_with_no_numeric_value() {
    let output = "_weird notanumber\n_ok 42\n";
    assert_eq!(parse_id_list(output), vec![42]);
}

#[test]
fn mismatched_attribute_reports_no_mismatch_when_equal() {
    assert_eq!(mismatched_attribute("RealName", Some("x"), Some("x")), None);
}

#[test]
fn mismatched_attribute_reports_the_difference() {
    let mismatch = mismatched_attribute("RealName", Some("x"), Some("y"));
    assert!(mismatch.is_some());
}
