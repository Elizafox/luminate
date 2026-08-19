// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! The daemon-provided work budget for one plugin callback.

use std::cell::Cell;
use std::time::{Duration, Instant};

/// The daemon-provided work budget for one plugin callback.
///
/// Plugins should construct a [`PluginDeadline`] when the callback begins and
/// use its remaining duration to cap transport waits and retry loops. The host
/// independently terminates callbacks that exceed the same hard budget.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PluginRequestContext {
    budget_millis: u64,
}

impl PluginRequestContext {
    /// Constructs a callback context from a bounded host timeout.
    #[must_use]
    #[inline]
    pub fn new(budget: Duration) -> Self {
        let millis = budget.as_millis().min(u128::from(u64::MAX));
        Self {
            budget_millis: u64::try_from(millis).unwrap_or(u64::MAX),
        }
    }

    /// Returns the original callback budget.
    #[must_use]
    #[inline]
    pub const fn budget(self) -> Duration {
        Duration::from_millis(self.budget_millis)
    }

    /// Starts a monotonic deadline for this callback.
    #[must_use]
    #[inline]
    pub fn deadline(self) -> PluginDeadline {
        PluginDeadline {
            started: Instant::now(),
            budget: self.budget(),
        }
    }
}

/// A monotonic, plugin-side view of the remaining callback budget.
#[derive(Debug, Clone, Copy)]
pub struct PluginDeadline {
    started: Instant,
    budget: Duration,
}

thread_local! {
    static REQUEST_DEADLINE: Cell<Option<PluginDeadline>> = const { Cell::new(None) };
}

/// Returns the deadline for the callback currently executing on this thread.
///
/// Background discovery threads are not daemon requests and therefore return
/// `None`.
#[must_use]
pub fn current_request_deadline() -> Option<PluginDeadline> {
    REQUEST_DEADLINE.get()
}

#[doc(hidden)]
pub fn with_request_context<T>(context: PluginRequestContext, callback: impl FnOnce() -> T) -> T {
    struct Reset(Option<PluginDeadline>);

    impl Drop for Reset {
        fn drop(&mut self) {
            REQUEST_DEADLINE.set(self.0);
        }
    }

    let previous = REQUEST_DEADLINE.replace(Some(context.deadline()));
    let _reset = Reset(previous);
    callback()
}

impl PluginDeadline {
    /// Returns the remaining work budget, or `None` once it is exhausted.
    #[must_use]
    #[inline]
    pub fn remaining(self) -> Option<Duration> {
        self.budget.checked_sub(self.started.elapsed())
    }

    /// Caps a transport wait to the remaining request budget.
    #[must_use]
    #[inline]
    pub fn cap(self, requested: Duration) -> Option<Duration> {
        self.remaining().map(|remaining| remaining.min(requested))
    }

    /// Reports whether the request budget has been exhausted.
    #[must_use]
    #[inline]
    pub fn is_expired(self) -> bool {
        self.remaining().is_none_or(|remaining| remaining.is_zero())
    }
}

#[cfg(test)]
#[path = "deadline_tests.rs"]
mod tests;
