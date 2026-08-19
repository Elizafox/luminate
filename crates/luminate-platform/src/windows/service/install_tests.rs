// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn recovery_policy_restarts_three_times_then_stops() {
    let policy = failure_actions();
    assert_eq!(
        policy.reset_period,
        ServiceFailureResetPeriod::After(Duration::from_secs(24 * 60 * 60))
    );
    assert_eq!(policy.reboot_msg, Some(OsString::new()));
    assert_eq!(policy.command, Some(OsString::new()));
    assert_eq!(
        policy.actions,
        Some(vec![
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(5),
            },
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(30),
            },
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(5 * 60),
            },
            ServiceAction {
                action_type: ServiceActionType::None,
                delay: Duration::ZERO,
            },
        ])
    );
}

#[test]
fn unquote_binary_path_strips_the_quoting_query_config_leaves_in_place() {
    assert_eq!(
        unquote_binary_path(Path::new(r#""C:\Program Files\Luminate\luminated.exe""#)),
        Path::new(r"C:\Program Files\Luminate\luminated.exe")
    );
    assert_eq!(
        unquote_binary_path(Path::new(r"C:\Luminate\luminated.exe")),
        Path::new(r"C:\Luminate\luminated.exe"),
        "a path with no space is never quoted and must round-trip unchanged"
    );
}

#[test]
fn required_privileges_are_encoded_as_a_multi_sz() {
    let encoded = encode_multi_sz(&["SeChangeNotifyPrivilege"]);
    let expected: Vec<u16> = "SeChangeNotifyPrivilege\0\0".encode_utf16().collect();

    assert_eq!(encoded, expected);
}

#[test]
fn client_group_purge_requires_ownership_and_an_empty_group() {
    assert_eq!(
        client_group_purge_error(false, true),
        Some("installer ownership metadata is absent or does not match")
    );
    assert_eq!(
        client_group_purge_error(true, false),
        Some("the group still has members")
    );
    assert_eq!(client_group_purge_error(true, true), None);
}
