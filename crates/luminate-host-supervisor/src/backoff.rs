// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Fixed-window backoff for supervised host respawns.

use std::time::{Duration, Instant};

/// Refuses a respawn until a fixed window has elapsed since the last attempt.
#[derive(Debug)]
pub struct RespawnBackoff {
    window: Duration,
    last_attempt: Option<Instant>,
}

impl RespawnBackoff {
    #[must_use]
    #[inline]
    pub const fn new(window: Duration) -> Self {
        Self {
            window,
            last_attempt: None,
        }
    }

    /// Records and permits a respawn attempt if `window` has elapsed since the
    /// previous one (or if no attempt has yet been recorded).
    ///
    /// A refused attempt does not reset the window.
    ///
    /// `now` is passed explicitly to avoid coupling callers and tests to
    /// wall-clock time.
    pub fn allow_attempt_at(&mut self, now: Instant) -> bool {
        if let Some(last) = self.last_attempt
            && now.saturating_duration_since(last) < self.window
        {
            return false;
        }
        self.last_attempt = Some(now);
        true
    }

    /// [`Self::allow_attempt_at`] using the current time.
    #[inline]
    pub fn allow_attempt(&mut self) -> bool {
        self.allow_attempt_at(Instant::now())
    }
}

#[cfg(test)]
#[path = "backoff_tests.rs"]
mod tests;
