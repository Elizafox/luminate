// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Checks that the documented D-Bus parity inventory follows libluminate additions.

use std::collections::BTreeSet;

const MATRIX: &str = include_str!("../../../docs/development/dbus-parity.md");
const SOURCES: &[(&str, &str)] = &[
    (
        "builder",
        include_str!("../../libluminate/src/client/mod.rs"),
    ),
    (
        "connection",
        include_str!("../../libluminate/src/client/connection.rs"),
    ),
    (
        "control",
        include_str!("../../libluminate/src/client/control.rs"),
    ),
    (
        "collections",
        include_str!("../../libluminate/src/client/collections.rs"),
    ),
    (
        "scenes",
        include_str!("../../libluminate/src/client/scenes.rs"),
    ),
    (
        "transitions",
        include_str!("../../libluminate/src/client/transitions.rs"),
    ),
    (
        "frames",
        include_str!("../../libluminate/src/client/frames.rs"),
    ),
    (
        "setup",
        include_str!("../../libluminate/src/client/setup.rs"),
    ),
    (
        "management",
        include_str!("../../libluminate/src/client/management.rs"),
    ),
    (
        "administration",
        include_str!("../../libluminate/src/client/administration.rs"),
    ),
    (
        "events",
        include_str!("../../libluminate/src/client/events.rs"),
    ),
];

fn public_async_operations() -> BTreeSet<String> {
    SOURCES
        .iter()
        .flat_map(|(source, contents)| {
            contents.lines().filter_map(move |line| {
                let line = line.trim();
                let declaration = line
                    .strip_prefix("pub async fn ")
                    .or_else(|| line.strip_prefix("pub const fn "))
                    .or_else(|| line.strip_prefix("pub fn "))?;
                let name = declaration.split_once('(')?.0;
                Some(format!("{source}::{name}"))
            })
        })
        .collect()
}

#[test]
fn every_public_client_operation_is_in_the_parity_matrix() {
    let missing = public_async_operations()
        .into_iter()
        .filter(|operation| !MATRIX.contains(&format!("`{operation}`")))
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "public libluminate operations missing from the D-Bus parity matrix: {missing:?}"
    );
}

#[test]
fn parity_matrix_has_no_unresolved_operations() {
    assert!(
        !MATRIX.contains("| Planned |"),
        "completed parity matrix still contains planned operations"
    );
}
