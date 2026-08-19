// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use std::time::Duration;

use super::{PluginRequestContext, current_request_deadline, with_request_context};

#[test]
fn request_context_scopes_and_expires_monotonic_deadline() {
    assert!(current_request_deadline().is_none());
    with_request_context(PluginRequestContext::new(Duration::ZERO), || {
        let deadline = current_request_deadline().expect("callback deadline");
        assert!(deadline.is_expired());
        assert!(
            deadline
                .cap(Duration::from_secs(1))
                .is_none_or(|value| value.is_zero())
        );
    });
    assert!(current_request_deadline().is_none());
}
