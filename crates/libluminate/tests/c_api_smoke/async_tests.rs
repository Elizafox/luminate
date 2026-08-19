// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

declare_c_test!(
    c_consumer_exercises_async_cancellation_and_early_client_release,
    include_str!("../c/async_operations.c"),
    with_stalling_events
);

declare_c_test!(
    c_consumer_integrates_asynchronous_api,
    include_str!("../c/async_api_integration.c")
);

declare_c_test!(
    c_consumer_exercises_all_async_symbols,
    include_str!("../c/async_symbol_coverage.c"),
    without_daemon
);

declare_c_test!(
    c_consumer_exercises_async_success_paths,
    include_str!("../c/async_success_path_coverage.c"),
    with_stalling_events
);

#[test]
fn cpp_consumer_retains_async_diagnostics_and_context_lifetime() {
    let missing_path = unique_path("cpp-async-missing");
    run_cpp_consumer(
        "cpp_consumer_retains_async_diagnostics_and_context_lifetime",
        include_str!("../c/async_diagnostics.cpp"),
        &[missing_path.as_os_str()],
    );
}
