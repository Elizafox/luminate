// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn ownership_schema_is_stable() {
    assert_eq!(INSTALLER_KEY, r"Software\Luminate");
    assert_eq!(OWNED_CLIENT_GROUP_VALUE, "InstallerOwnedClientGroup");
    assert_eq!(OWNED_EVENT_LOG_SOURCE_VALUE, "InstallerOwnedEventLogSource");
}

#[test]
#[ignore = "modifies HKLM and requires administrator privileges"]
fn event_log_source_ownership_round_trips() {
    const SOURCE: &str = "luminated";

    record_owned_event_log_source(SOURCE).expect("ownership should be recorded");
    assert!(owns_event_log_source(SOURCE).expect("ownership should match"));
    remove_owned_event_log_source(SOURCE).expect("ownership should be removed");
    assert!(!owns_event_log_source(SOURCE).expect("ownership should be absent"));
}
