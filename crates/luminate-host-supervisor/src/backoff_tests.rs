// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use super::RespawnBackoff;
use std::time::{Duration, Instant};

#[test]
fn first_attempt_is_always_allowed() {
    let mut backoff = RespawnBackoff::new(Duration::from_secs(5));
    assert!(backoff.allow_attempt_at(Instant::now()));
}

#[test]
fn a_second_attempt_within_the_window_is_refused() {
    let mut backoff = RespawnBackoff::new(Duration::from_secs(5));
    let t0 = Instant::now();
    assert!(backoff.allow_attempt_at(t0));
    assert!(!backoff.allow_attempt_at(t0 + Duration::from_secs(1)));
}

#[test]
fn an_attempt_once_the_window_elapses_is_allowed_again() {
    let mut backoff = RespawnBackoff::new(Duration::from_secs(5));
    let t0 = Instant::now();
    assert!(backoff.allow_attempt_at(t0));
    assert!(backoff.allow_attempt_at(t0 + Duration::from_secs(5)));
}

#[test]
fn refused_attempts_do_not_reset_the_window() {
    let mut backoff = RespawnBackoff::new(Duration::from_secs(5));
    let t0 = Instant::now();
    assert!(backoff.allow_attempt_at(t0));
    assert!(!backoff.allow_attempt_at(t0 + Duration::from_millis(500)));
    assert!(!backoff.allow_attempt_at(t0 + Duration::from_millis(999)));
    assert!(backoff.allow_attempt_at(t0 + Duration::from_secs(5)));
}
