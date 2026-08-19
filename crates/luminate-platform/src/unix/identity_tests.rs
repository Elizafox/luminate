// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::{daemon_own_uid, uid_for_user};

#[test]
#[allow(
    unsafe_code,
    reason = "libc::getuid() takes no arguments, touches no memory, and cannot fail; matches \
                  daemon_own_uid's own justification."
)]
fn matches_a_fresh_getuid_call() {
    // SAFETY: see daemon_own_uid's own SAFETY comment.
    let fresh = unsafe { libc::getuid() };
    assert_eq!(daemon_own_uid(), fresh);
}

#[test]
fn account_lookup_distinguishes_missing_and_invalid_names() {
    assert_eq!(
        uid_for_user("luminate-account-that-must-not-exist").expect("complete account lookup"),
        None
    );
    assert!(uid_for_user("invalid\0account").is_err());
}
