// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

declare_staged_c_test!(
    c_consumer_builds_from_staged_install,
    include_str!("../c/staged.c")
);

#[test]
fn generated_header_compiles_and_links_as_strict_c23() {
    run_header_consumer(
        "header_c23",
        include_str!("../c/header.c"),
        Language::C,
        best_available_c_standard(),
    );
}

#[test]
fn generated_header_compiles_and_links_as_cpp17() {
    run_header_consumer(
        "header_cpp17",
        include_str!("../c/header.cpp"),
        Language::Cxx,
        "c++17",
    );
}
