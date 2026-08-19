// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::process;

use super::{
    account_sid, current_process_sid, daemon_own_sid, interactive_sid, local_system_sid,
    process_has_enabled_group_sid, process_sid,
};

#[test]
fn current_process_sid_returns_a_well_formed_sid() {
    let sid = current_process_sid().expect("read the test process's own SID");
    assert!(sid.starts_with("S-1-"), "unexpected SID shape: {sid}");
}

#[test]
fn daemon_own_sid_is_stable_and_matches_a_fresh_lookup() {
    let cached = daemon_own_sid();
    let fresh = current_process_sid().expect("read the test process's own SID");

    // Exercise the owned-string comparison used by the daemon's Windows
    // principal check.
    let owned = fresh.clone();
    assert_eq!(daemon_own_sid(), owned);
    assert_eq!(cached, fresh);
}

#[test]
fn process_sid_reads_the_current_process_identity() {
    let through_process_handle =
        process_sid(process::id()).expect("read the test process SID by process ID");
    let directly = current_process_sid().expect("read the test process's own SID");

    assert_eq!(through_process_handle, directly);
}

#[test]
fn account_name_resolution_returns_a_canonical_sid() {
    let sid =
        account_sid(r"BUILTIN\Administrators").expect("resolve the built-in Administrators group");

    assert_eq!(sid, "S-1-5-32-544");
}

#[test]
fn interactive_logon_sid_is_canonical() {
    assert_eq!(
        interactive_sid().expect("construct interactive SID"),
        "S-1-5-4"
    );
}

#[test]
fn local_system_sid_is_canonical() {
    assert_eq!(
        local_system_sid().expect("construct LocalSystem SID"),
        "S-1-5-18"
    );
}

#[test]
fn process_group_lookup_distinguishes_enabled_groups_from_non_members() {
    let users_sid = account_sid(r"BUILTIN\Users").expect("resolve built-in Users group");
    assert!(
        process_has_enabled_group_sid(process::id(), &users_sid)
            .expect("read this process's enabled groups")
    );
    assert!(
        !process_has_enabled_group_sid(process::id(), "S-1-5-21-0-0-0-424242")
            .expect("search this process's enabled groups")
    );
}
