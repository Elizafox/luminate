// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Per-key custom-animation packet encoding tests.

use super::*;

#[test]
fn slot_clear_matches_documented_bytes() {
    assert_eq!(&slot_clear_report(0x0e)[..4], &[0xcc, 0x8c, 0x01, 0x0e]);
    assert!(slot_clear_report(0x0e)[4..].iter().all(|&byte| byte == 0));
}

#[test]
fn apply_custom_colours_shifts_assignment_position_below_key_index() {
    let assigned_positions = assignment_positions([0x01]).unwrap();
    let reports = assignment_table_reports(&assigned_positions);

    assert_eq!(reports[0][4], 0x01, "key_index 0x01 assigns position 0x00");
    assert_eq!(
        reports[0][5], 0x00,
        "position 0x01 (the unshifted, buggy target) must stay unassigned"
    );
}

#[test]
fn assignment_positions_rejects_zero_key_index() {
    let error = assignment_positions([0x00]).unwrap_err();

    assert_eq!(
        error,
        "keyboard key index 0 has no corresponding assignment-table position"
    );
}

#[test]
fn reset_matches_documented_bytes() {
    assert_eq!(&reset_report()[..4], &[0xcc, 0x8c, 0x10, 0x00]);
}

#[test]
fn static_slot_config_matches_documented_bytes() {
    let report = static_slot_config_report(Rgb::new(0xff, 0x00, 0x00));

    assert_eq!(
        &report[..18],
        &[
            0xcc, 0x8c, 0x01, 0x01, 0x01, 0x00, 0x00, 0x01, 0x01, 0x01, 0x00, 0xff, 0x00, 0x00,
            0xff, 0x00, 0x00, 0x01
        ]
    );
}

#[test]
fn assignment_table_marks_boundary_indices_across_all_three_reports() {
    let assigned = [0x00, 0x3b, 0x3c, 0x77, 0x78, 0x8b]
        .into_iter()
        .collect::<BTreeSet<u8>>();
    let reports = assignment_table_reports(&assigned);

    assert_eq!(&reports[0][..4], &[0xcc, 0x8c, 0x05, 0x00]);
    assert_eq!(reports[0][4], 0x01, "index 0x00 -> first byte of report 0");
    assert_eq!(
        reports[0][63], 0x01,
        "index 0x3b (59) -> last byte of report 0"
    );

    assert_eq!(&reports[1][..4], &[0xcc, 0x8c, 0x06, 0x00]);
    assert_eq!(
        reports[1][4], 0x01,
        "index 0x3c (60) -> first byte of report 1"
    );
    assert_eq!(
        reports[1][63], 0x01,
        "index 0x77 (119) -> last byte of report 1"
    );

    assert_eq!(&reports[2][..4], &[0xcc, 0x8c, 0x07, 0x00]);
    assert_eq!(
        reports[2][4], 0x01,
        "index 0x78 (120) -> first byte of report 2"
    );
    assert_eq!(
        reports[2][23], 0x01,
        "index 0x8b (139) -> last of the 20-entry report 2"
    );
    assert_eq!(
        reports[2][24], 0x00,
        "byte past the 20 real entries stays zero"
    );
}

#[test]
fn assignment_table_leaves_unassigned_positions_zero() {
    let reports = assignment_table_reports(&BTreeSet::new());

    assert!(reports[0][4..].iter().all(|&byte| byte == 0));
    assert!(reports[1][4..].iter().all(|&byte| byte == 0));
    assert!(reports[2][4..].iter().all(|&byte| byte == 0));
}

#[test]
fn rgb_table_single_record_matches_documented_bytes() {
    let reports = rgb_table_reports(&[(0x01, Rgb::new(1, 2, 3))]);

    assert_eq!(reports.len(), 1);
    assert_eq!(&reports[0][..8], &[0xcc, 0x8c, 0x02, 0x00, 0x01, 1, 2, 3]);
}

#[test]
fn rgb_table_chunks_at_fifteen_records_per_report() {
    let entries = (0..16_u8)
        .map(|index| (index, Rgb::new(index, index, index)))
        .collect::<Vec<_>>();
    let reports = rgb_table_reports(&entries);

    assert_eq!(reports.len(), 2);
    assert_eq!(&reports[1][..8], &[0xcc, 0x8c, 0x02, 0x00, 15, 15, 15, 15]);
}

#[test]
fn rgb_table_empty_produces_no_reports() {
    assert!(rgb_table_reports(&[]).is_empty());
}

#[test]
fn finalize_matches_documented_bytes() {
    assert_eq!(&finalize_report()[..4], &[0xcc, 0x8c, 0x13, 0x00]);
}

#[test]
fn commit_matches_documented_bytes() {
    assert_eq!(&commit_report()[..4], &[0xcc, 0x8b, 0x01, 0xff]);
}
