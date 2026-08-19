// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Tests for the parent module.

use super::*;

#[test]
fn retains_entries_between_lock_acquisitions() {
    let shadow = ShadowState::new();
    shadow.lock().expect("lock poisoned").insert("zone", 42);

    assert_eq!(shadow.lock().expect("lock poisoned").get("zone"), Some(&42));
}
