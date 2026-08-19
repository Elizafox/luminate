// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Shared plugin-host test fixtures.

use std::sync::{Mutex, MutexGuard};

/// Locks a process-global test fixture, ignoring poisoning.
///
/// Every caller resets its guarded fixture state on entry, so no invariant
/// from a panicking holder survives for a later test to rely on.
pub(crate) fn lock_fixture<T>(lock: &'static Mutex<T>) -> MutexGuard<'static, T> {
    lock.lock().expect("lock poisoned")
}
