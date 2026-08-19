// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Identity lookup and authored key-layout tests.

use super::*;

#[test]
fn us_ansi_presence_mask_excludes_documented_reserved_positions() {
    let map = KeyboardLayoutId::M16R2UsAnsi
        .key_map()
        .expect("known layout should have a key map");

    assert_eq!(
        map.presence.len(),
        usize::from(KEYBOARD_MATRIX_POSITIONS) - M16_R2_US_ANSI_RESERVED_INDICES.len()
    );
    for index in M16_R2_US_ANSI_RESERVED_INDICES {
        assert!(!map.position_indices().any(|present| present == *index));
    }
}

#[test]
fn us_ansi_key_map_has_no_duplicate_indices() {
    let map = KeyboardLayoutId::M16R2UsAnsi
        .key_map()
        .expect("known layout should have a key map");
    let indices = map.position_indices().collect::<Vec<_>>();
    let unique = indices.iter().copied().collect::<BTreeSet<_>>();

    assert_eq!(indices.len(), unique.len());
}

#[test]
fn unknown_layout_has_no_key_map() {
    let layout = KeyboardLayoutId::Unknown {
        hardware_variant: [0x01, 0x02],
        layout: 0xff,
        chassis_colour: 0x03,
    };

    assert!(layout.key_map().is_none());
    assert!(layout.physical_tags().is_empty());
}

#[test]
fn captured_us_ansi_identity_resolves_to_m16_r2_us_ansi() {
    let identity = KeyboardIdentity {
        hardware_variant: [0x17, 0x11],
        layout: 0x21,
        chassis_colour: 0x00,
    };

    assert_eq!(identity.layout_id(), KeyboardLayoutId::M16R2UsAnsi);
    assert_eq!(identity.layout_id().physical_tags(), ["layout:us-ansi"]);
}

#[test]
fn different_identity_resolves_to_unknown() {
    let identity = KeyboardIdentity {
        hardware_variant: [0x01, 0x02],
        layout: 0xff,
        chassis_colour: 0x03,
    };

    assert!(matches!(
        identity.layout_id(),
        KeyboardLayoutId::Unknown { .. }
    ));
}

#[test]
fn parse_identity_report_reads_documented_offsets() {
    let report = [0xcc, 0x93, 0x12, 0x34, 0x56, 0x78];

    let identity = parse_identity_report(&report).expect("identity should parse");

    assert_eq!(identity.hardware_variant, [0x12, 0x34]);
    assert_eq!(identity.layout, 0x56);
    assert_eq!(identity.chassis_colour, 0x78);
}

#[test]
fn short_identity_report_is_unknown() {
    assert!(parse_identity_report(&[0xcc, 0x93, 0x12]).is_none());
}

#[test]
fn us_ansi_key_names_has_no_duplicate_names() {
    let names = M16_R2_US_ANSI_KEY_NAMES
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>();
    let unique = names.iter().copied().collect::<BTreeSet<_>>();

    assert_eq!(names.len(), unique.len());
}

#[test]
fn us_ansi_key_names_has_no_duplicate_indices() {
    let indices = M16_R2_US_ANSI_KEY_NAMES
        .iter()
        .map(|(_, index)| *index)
        .collect::<Vec<_>>();
    let unique = indices.iter().copied().collect::<BTreeSet<_>>();

    assert_eq!(indices.len(), unique.len());
}

#[test]
fn us_ansi_named_indices_are_all_present_positions() {
    let map = KeyboardLayoutId::M16R2UsAnsi
        .key_map()
        .expect("known layout should have a key map");
    let present = map.position_indices().collect::<BTreeSet<_>>();

    for (name, index) in M16_R2_US_ANSI_KEY_NAMES {
        assert!(
            present.contains(index),
            "named key {name} at index {index:#04x} should not be a reserved position"
        );
    }
}

#[test]
fn us_ansi_index_for_name_resolves_documented_key() {
    let map = KeyboardLayoutId::M16R2UsAnsi
        .key_map()
        .expect("known layout should have a key map");

    assert_eq!(map.index_for_name("escape"), Some(0x01));
    assert_eq!(map.index_for_name("space"), Some(0x6c));
    assert_eq!(map.index_for_name("not-a-real-key"), None);
}

#[test]
fn us_ansi_named_positions_has_documented_key_count() {
    let map = KeyboardLayoutId::M16R2UsAnsi
        .key_map()
        .expect("known layout should have a key map");

    assert_eq!(map.named_positions().count(), 85);
}
