// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Guards the ABI 24 blocking/asynchronous operation coverage contract.

#![allow(
    clippy::tests_outside_test_module,
    reason = "this integration-test crate contains only this parity test"
)]

use std::collections::BTreeSet;

const HEADER: &str = include_str!("../luminate.h");

const LOCAL_ONLY: &[&str] = &[
    "luminate_client_builder_new",
    "luminate_client_builder_authenticate_peer",
    "luminate_client_builder_authenticate_bearer",
    "luminate_client_builder_set_path",
    "luminate_client_builder_authenticate_attestation",
    "luminate_client_builder_authenticate_external",
    "luminate_client_builder_add_scope_operation",
    "luminate_client_builder_set_scope",
    "luminate_client_daemon_version",
    "luminate_client_socket_path",
    "luminate_client_event_socket_path",
    "luminate_client_get_session_metadata",
    "luminate_client_shm_upload_frame_full",
];

fn blocking_status_operations() -> BTreeSet<&'static str> {
    HEADER
        .lines()
        .filter_map(|line| {
            let (_, declaration) = line.trim().split_once("LuminateStatus ")?;
            let name = declaration.split_once('(')?.0;
            (name.starts_with("luminate_client_") || name == "luminate_event_subscription_next")
                .then_some(name)
        })
        .filter(|name| !name.ends_with("_async"))
        .collect()
}

fn asynchronous_status_operations() -> BTreeSet<&'static str> {
    HEADER
        .lines()
        .filter_map(|line| {
            let (_, declaration) = line.trim().split_once("LuminateStatus ")?;
            let name = declaration.split_once('(')?.0;
            name.strip_suffix("_async")
        })
        .collect()
}

#[test]
fn every_external_io_operation_has_exactly_one_async_sibling() {
    let local_only = LOCAL_ONLY.iter().copied().collect::<BTreeSet<_>>();
    for name in &local_only {
        assert!(
            HEADER.contains(&format!("LuminateStatus {name}(")),
            "local-only declaration missing for {name}"
        );
        assert!(
            !HEADER.contains(&format!("LuminateStatus {name}_async(")),
            "local-only operation unexpectedly gained an async sibling: {name}"
        );
    }

    let blocking = blocking_status_operations();
    let external_io = blocking
        .difference(&local_only)
        .copied()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        asynchronous_status_operations(),
        external_io,
        "every external-I/O operation must have exactly one async sibling, and local-only operations must have none"
    );
}
