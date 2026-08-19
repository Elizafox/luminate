// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unnecessary_wraps,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! In-process state for hardware properties that plugins cannot read back.

use std::collections::BTreeMap;
use std::sync::{LockResult, Mutex, MutexGuard};

/// A process-lifetime, synchronized map of state last written to hardware.
///
/// This is intended for properties without hardware readback, where a plugin
/// must remember its own successful writes.
#[derive(Debug)]
pub struct ShadowState<K, V> {
    entries: Mutex<BTreeMap<K, V>>,
}

impl<K, V> ShadowState<K, V> {
    /// Constructs an empty shadow map.
    #[must_use]
    #[inline]
    pub const fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// Locks the shadow map for reading or mutation.
    ///
    /// # Errors
    ///
    /// The error variant is retained for source compatibility; lock poisoning
    /// panics instead of being represented as an ordinary error.
    ///
    /// # Panics
    ///
    /// Panics if another holder panicked while the lock was held, because the
    /// remembered hardware state may no longer satisfy its invariants.
    #[inline]
    pub fn lock(&self) -> LockResult<MutexGuard<'_, BTreeMap<K, V>>> {
        Ok(self.entries.lock().expect("shadow-state lock poisoned"))
    }
}

impl<K, V> Default for ShadowState<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "shadow_state_tests.rs"]
mod tests;
