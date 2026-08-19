// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn parse_print_state_recognizes_a_running_job() {
    let stdout = "\tstate = running\n\tprogram = /usr/local/bin/luminated\n";
    assert_eq!(
        parse_print_state(stdout),
        ServiceState::Loaded { running: true }
    );
}

#[test]
fn parse_print_state_recognizes_a_loaded_but_stopped_job() {
    let stdout = "\tstate = not running\n";
    assert_eq!(
        parse_print_state(stdout),
        ServiceState::Loaded { running: false }
    );
}

#[test]
fn parse_print_state_defaults_to_not_running_when_state_is_absent() {
    assert_eq!(
        parse_print_state("some unrelated output\n"),
        ServiceState::Loaded { running: false }
    );
}

#[test]
fn not_loaded_recognizes_the_documented_error_messages() {
    assert!(not_loaded(
        "Could not find service \"com.wilcoxti.luminate.luminated\" in domain for system"
    ));
    assert!(!not_loaded("permission denied"));
}

#[test]
fn already_bootstrapped_recognizes_the_documented_message() {
    assert!(already_bootstrapped(
        "Bootstrap failed: 5: Input/output error\nalready bootstrapped"
    ));
}
