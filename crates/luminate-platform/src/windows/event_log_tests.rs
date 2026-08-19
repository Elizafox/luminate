// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn message_file_is_an_expandable_absolute_path() {
    assert!(MESSAGE_FILE.starts_with("%SystemRoot%\\"));
    assert!(MESSAGE_FILE.ends_with("EventLogMessages.dll"));
}

#[test]
fn event_log_types_cover_information_warning_and_error() {
    assert_eq!(TYPES_SUPPORTED, 0x7);
}

#[test]
#[ignore = "modifies HKLM and requires administrator privileges"]
fn event_source_registration_is_idempotent() {
    const TEST_SOURCE_KEY: &str =
        r"SYSTEM\CurrentControlSet\Services\EventLog\Application\luminated-test";

    let outcome = ensure_event_source_at(TEST_SOURCE_KEY).expect("event source should register");
    assert_eq!(outcome, EnsureEventSourceOutcome::Created);
    assert!(event_source_is_registered_at(TEST_SOURCE_KEY).expect("event source should match"));
    assert_eq!(
        ensure_event_source_at(TEST_SOURCE_KEY).expect("registration should be idempotent"),
        EnsureEventSourceOutcome::AlreadyRegistered
    );

    delete_source_key(TEST_SOURCE_KEY).expect("test-created source should be removed");
    assert!(
        !event_source_is_registered_at(TEST_SOURCE_KEY).expect("event source should be absent")
    );
}
