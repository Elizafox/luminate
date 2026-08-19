// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

declare_c_test!(
    c_consumer_reports_connection_failures,
    include_str!("../c/unavailable.c"),
    without_daemon
);
declare_c_test!(
    c_consumer_validates_access_administration_contracts,
    include_str!("../c/access_administration.c"),
    without_daemon
);
declare_c_test!(
    c_consumer_reports_protocol_incompatibility,
    include_str!("../c/incompatible.c"),
    with_incompatible_sockets
);
declare_c_test!(
    c_consumer_reports_mid_request_disconnect,
    include_str!("../c/disconnect.c"),
    with_disconnect
);

#[test]
fn c_consumer_reads_structured_error_metadata() {
    let socket_path = unique_path("error-metadata-sock");
    let server = spawn_structured_error_daemon(&socket_path);
    run_c_consumer(
        "error_metadata",
        include_str!("../c/error_metadata.c"),
        &[socket_path.as_os_str()],
    );
    server
        .join()
        .expect("structured error daemon thread should not panic");
    let _ = fs::remove_file(socket_path);
}
