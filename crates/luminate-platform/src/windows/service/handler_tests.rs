// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn startup_checkpoints_advance_only_while_starting() {
    let mut status = initial_status();
    advance_startup(&mut status);
    assert_eq!(status.dwCheckPoint, 2);

    mark_running(&mut status);
    advance_startup(&mut status);
    assert_eq!(status.dwCheckPoint, 0);
}

#[test]
fn readiness_enables_terminal_controls() {
    let mut status = initial_status();
    mark_running(&mut status);

    assert_eq!(status.dwCurrentState, SERVICE_RUNNING);
    assert_eq!(
        status.dwControlsAccepted,
        SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_PRESHUTDOWN | SERVICE_ACCEPT_POWEREVENT
    );
    assert_eq!(status.dwWaitHint, 0);
}

#[test]
fn terminal_controls_are_idempotent() {
    let requested = AtomicBool::new(false);

    assert!(begin_shutdown(&requested));
    assert!(!begin_shutdown(&requested));
}

#[test]
fn stopping_status_carries_a_bounded_cleanup_hint() {
    let mut status = initial_status();
    mark_stop_pending(&mut status);

    assert_eq!(status.dwCurrentState, SERVICE_STOP_PENDING);
    assert_eq!(status.dwControlsAccepted, 0);
    assert_eq!(status.dwCheckPoint, 1);
    assert_eq!(status.dwWaitHint, duration_millis(STOP_WAIT_HINT));
}

#[test]
fn late_readiness_does_not_reverse_stopping_status() {
    let mut status = initial_status();
    mark_stop_pending(&mut status);
    mark_running(&mut status);

    assert_eq!(status.dwCurrentState, SERVICE_STOP_PENDING);
    assert_eq!(status.dwCheckPoint, 1);
}

#[test]
fn service_failures_use_the_service_specific_exit_field() {
    let mut status = initial_status();
    mark_stopped_with_error(&mut status, 7);

    assert_eq!(status.dwCurrentState, SERVICE_STOPPED);
    assert_eq!(status.dwWin32ExitCode, ERROR_SERVICE_SPECIFIC_ERROR);
    assert_eq!(status.dwServiceSpecificExitCode, 7);
}
