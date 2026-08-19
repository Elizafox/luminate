// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Ensures packaging metadata agrees with the canonical C ABI version.

#![allow(
    clippy::tests_outside_test_module,
    reason = "Integration tests are crate roots."
)]

use luminate::LUMINATE_C_ABI_VERSION;

const INSTALL_SCRIPT: &str = include_str!("../../../packaging/install-linux.sh");
const PACKAGE_MANIFEST: &str = include_str!("../../luminated/Cargo.toml");

#[test]
fn packaging_uses_canonical_c_abi_version() {
    let assignment = format!("LUMINATE_C_ABI_VERSION={LUMINATE_C_ABI_VERSION}");
    assert!(
        INSTALL_SCRIPT.lines().any(|line| line == assignment),
        "packaging/install-linux.sh must assign {assignment}"
    );

    let versions: Vec<u32> = PACKAGE_MANIFEST
        .match_indices("libluminate.so.")
        .filter_map(|(start, _)| {
            PACKAGE_MANIFEST
                .get(start..)?
                .strip_prefix("libluminate.so.")?
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .collect();

    assert!(
        !versions.is_empty(),
        "package manifest must name the C library SONAME"
    );
    assert!(
        versions
            .iter()
            .all(|version| *version == LUMINATE_C_ABI_VERSION),
        "package manifest C ABI versions {versions:?} must all equal {LUMINATE_C_ABI_VERSION}"
    );

    for (library_dir, capability_suffix) in [("/usr/lib64", "()(64bit)"), ("/usr/lib", "")] {
        let asset = format!("{library_dir}/libluminate.so.{LUMINATE_C_ABI_VERSION}");
        assert!(
            PACKAGE_MANIFEST.contains(&asset),
            "package manifest must include the {asset} asset"
        );

        let provides =
            format!("\"libluminate.so.{LUMINATE_C_ABI_VERSION}{capability_suffix}\" = \"\"");
        assert!(
            PACKAGE_MANIFEST.contains(&provides),
            "package manifest must include {provides}"
        );
    }
}
