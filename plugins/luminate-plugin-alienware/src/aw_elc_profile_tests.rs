// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! AW-ELC profile registry and identity-resolution tests.

use super::*;

#[test]
fn imported_profiles_resolve_only_with_their_exact_zone_count() {
    for profile in PROFILES {
        let platform_id = profile
            .platform_id
            .expect("imported profile has a platform ID");
        let identity = resolve(
            AW_ELC_VENDOR_ID,
            profile.product_id,
            platform_id,
            profile.expected_raw_zone_count,
        )
        .expect("registered profile should resolve");

        assert_eq!(identity.profile, profile);
    }
}

#[test]
fn mismatched_zone_count_fails_closed() {
    let error = resolve(AW_ELC_VENDOR_ID, AW_ELC_LEGACY_PRODUCT_ID, 0x0c01, 5)
        .expect_err("an unexpected count must not resolve");

    assert_eq!(
        error,
        ProfileResolutionError::ZoneCountMismatch {
            platform_id: 0x0c01,
            expected: 4,
            reported: 5,
        }
    );
}

#[test]
fn platform_id_is_scoped_to_the_exact_product() {
    let error = resolve(AW_ELC_VENDOR_ID, AW_ELC_M16_R2_PRODUCT_ID, 0x0c01, 4)
        .expect_err("a profile on another product must not resolve");

    assert!(matches!(
        error,
        ProfileResolutionError::UnknownPlatform { .. }
    ));
}

#[test]
fn profile_zone_ids_are_unique_and_bounded() {
    for profile in PROFILES.iter().chain([&M16_R2]) {
        let mut ids = profile
            .zones
            .iter()
            .map(|zone| zone.firmware_id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();

        assert_eq!(ids.len(), profile.zones.len(), "{}", profile.model);
        assert!(profile.zones.len() <= 28, "{}", profile.model);
    }
}

#[test]
fn imported_profiles_have_only_live_zones() {
    assert!(PROFILES.iter().all(|profile| {
        profile
            .zones
            .iter()
            .all(|zone| zone.operation_class == ZoneOperationClass::Live)
    }));
}

#[test]
fn m16_r2_resolves_with_its_observed_raw_count() {
    let identity = resolve(AW_ELC_VENDOR_ID, AW_ELC_M16_R2_PRODUCT_ID, 0x1102, 5)
        .expect("the hardware-validated m16 R2 identity should resolve");

    assert_eq!(identity.profile, &M16_R2);
    assert_eq!(identity.profile.zones.len(), 3);
}
