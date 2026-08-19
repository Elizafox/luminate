// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Per-SKU capability and topology-profile tests.

use super::*;

#[test]
fn h6022_profile_advertises_its_known_capabilities() {
    let profile = profile_for("H6022");
    assert_eq!(profile.model_name, "H6022");
    assert!(profile.supports_cct);
    assert!(profile.supports_scenes);
    assert_eq!(profile.matrix, Some((11, 12)));
}

#[test]
fn h6022_scene_table_distinguishes_both_rainbow_scenes() {
    let profile = profile_for("H6022");
    assert!(
        profile
            .scenes
            .iter()
            .any(|&(code, name)| code == 0x16 && name == "Rainbow Drawing")
    );
    assert!(
        profile
            .scenes
            .iter()
            .any(|&(code, name)| code == 0x2a && name == "Rainbow Striped")
    );
}

#[test]
fn unknown_sku_falls_back_to_the_conservative_profile() {
    let profile = profile_for("H1234-DOES-NOT-EXIST");
    assert!(!profile.supports_cct);
    assert!(!profile.supports_scenes);
    assert!(profile.scenes.is_empty());
    assert_eq!(profile.matrix, None);
}
