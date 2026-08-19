// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::panic::catch_unwind;

use tokio::runtime::Builder;

use super::*;

/// Registering Tokio signal handlers requires a running reactor on the
/// current thread. `luminated`'s entry point once built its `RunContext`
/// eagerly as a `block_on` argument, which evaluated `install` outside the
/// runtime and crashed the daemon on startup. This pins the requirement so
/// a future caller that installs signals outside a runtime is caught here
/// rather than at service start.
#[test]
fn install_outside_a_runtime_panics() {
    let outcome = catch_unwind(ShutdownSignals::install);
    assert!(
        outcome.is_err(),
        "installing shutdown signals without a runtime must panic"
    );
}

/// The counterpart: driven from inside a runtime, as the entry point now
/// does, installation succeeds. `enable_all` is what wires up the signal
/// driver the handlers depend on.
#[test]
fn install_within_a_runtime_succeeds() {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("building a current-thread runtime should not fail");

    runtime.block_on(async {
        ShutdownSignals::install().expect("installing within a runtime should succeed");
    });
}
