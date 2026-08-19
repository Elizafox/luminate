// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for canonical target identifiers and D-Bus object paths.

use super::*;

#[test]
fn paths_are_stable_valid_and_collision_free() {
    assert_eq!(
        target_path(&TargetId::device("kbd-1")),
        "/org/luminate/Luminate1/devices/x6b62642d31"
    );
    assert_ne!(
        target_path(&TargetId::device("a-b")),
        target_path(&TargetId::device("a_2db"))
    );
    assert!(target_path(&TargetId::element("d", "s", "e")).starts_with(ROOT));
}

#[test]
fn canonical_identifiers_parse_every_target_shape_and_reject_malformed_values() {
    for identifier in [
        "device:d",
        "device:d/surface:s",
        "device:d/group:g",
        "device:d/surface:s/element:e",
    ] {
        let target = parse_canonical_id(identifier).expect("parse canonical identifier");
        assert_eq!(canonical_id(&target), identifier);
    }

    for identifier in [
        "",
        "device:",
        "surface:s",
        "device:d/group:",
        "device:d/surface:s/element:",
        "device:d/surface:s/element:e/extra:x",
    ] {
        assert!(
            parse_canonical_id(identifier).is_err(),
            "{identifier:?} should be rejected"
        );
    }
}
