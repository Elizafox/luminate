// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Shared bookkeeping for dynamically discovered plugin devices.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::notification;

/// Paces a discovery thread's cycles while letting a rescan cut the wait
/// short.
///
/// A discovery thread ordinarily sleeps between cycles, which means a plugin
/// serving topology from [`DynamicDeviceRegistry`] keeps reporting its
/// pre-suspend view for up to one interval after a resume. Using this pacer
/// lets [`crate::sdk::RescanPlugin::rescan`] wake the thread immediately.
///
/// A wake that arrives while a cycle is already running is remembered, so the
/// next wait returns at once rather than dropping the request: the cycle in
/// flight may have already passed the point where it would have seen the new
/// hardware.
#[derive(Debug)]
pub struct DiscoveryPacer {
    state: Mutex<bool>,
    woken: Condvar,
}

impl DiscoveryPacer {
    /// Creates a pacer with no wake pending.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Mutex::new(false),
            woken: Condvar::new(),
        }
    }

    /// Waits up to `interval` for a wake, returning as soon as one arrives.
    ///
    /// Consumes any pending wake, so one request produces one early cycle.
    ///
    /// # Panics
    ///
    /// Panics if another thread panicked while holding the pacer lock.
    pub fn wait(&self, interval: Duration) {
        let mut pending = self.state.lock().expect("lock poisoned");
        if !*pending {
            // `wait_timeout_while` re-checks the predicate on spurious
            // wake-ups, so this cannot return early without a real wake.
            let (guard, _) = self
                .woken
                .wait_timeout_while(pending, interval, |pending| !*pending)
                .expect("lock poisoned");
            pending = guard;
        }
        *pending = false;
    }

    /// Asks the waiting discovery thread to run a cycle now.
    ///
    /// # Panics
    ///
    /// Panics if another thread panicked while holding the pacer lock.
    pub fn wake(&self) {
        *self.state.lock().expect("lock poisoned") = true;
        self.woken.notify_all();
    }
}

impl Default for DiscoveryPacer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
struct RegistryEntry<T, F> {
    value: T,
    fingerprint: F,
    discovery_source: Option<IpAddr>,
    last_seen: Instant,
}

/// Result of merging one dynamic discovery refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryRefresh {
    /// Whether the published topology fingerprint changed.
    pub topology_changed: bool,
    /// Number of new entries rejected after the configured limit was reached.
    pub rejected: usize,
    /// Number of entries removed because their last-seen time expired.
    pub expired: usize,
}

/// Defines whether an entry is retained exactly at its expiry duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryExpiry {
    /// Expire an entry when its age reaches the configured duration.
    AtOrAfter,
    /// Retain an entry at the boundary and expire it only when older.
    After,
}

/// Indicates that a dynamic registry lock was poisoned by a panic.
///
/// The registry deliberately does not expose the protected data after this
/// error. A panic may have interrupted an update while its invariants were
/// temporarily incomplete, so continuing would make device identity and
/// topology state uncertain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("dynamic device registry lock poisoned")]
pub struct RegistryPoisonError;

type RegistryGuard<'a, T, F> = MutexGuard<'a, HashMap<String, RegistryEntry<T, F>>>;

/// Bounded, deterministically ordered registry for dynamically discovered devices.
///
/// Missed devices remain present until `expiry` elapses. Refreshes may always
/// update an existing identity, even while the registry is at capacity.
pub struct DynamicDeviceRegistry<T, F> {
    maximum_entries: usize,
    expiry: Duration,
    expiry_boundary: RegistryExpiry,
    entries: Mutex<HashMap<String, RegistryEntry<T, F>>>,
}

impl<T, F> DynamicDeviceRegistry<T, F>
where
    T: Clone,
    F: Clone + Eq,
{
    /// Constructs an empty bounded registry.
    ///
    /// A zero entry limit creates a deliberately disabled registry. A zero
    /// expiry accepts refreshed values only for the duration of that refresh.
    #[must_use]
    pub fn new(maximum_entries: usize, expiry: Duration, expiry_boundary: RegistryExpiry) -> Self {
        Self {
            maximum_entries,
            expiry,
            expiry_boundary,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Merges complete refreshed entries, expires stale identities, and emits
    /// at most one topology-change notification.
    ///
    /// # Errors
    ///
    /// The error variant is retained for source compatibility; lock poisoning
    /// panics instead of being represented as an ordinary error.
    ///
    /// # Panics
    ///
    /// Panics if another thread panicked while holding the registry lock.
    pub fn refresh<I>(
        &self,
        now: Instant,
        refreshed: I,
    ) -> Result<RegistryRefresh, RegistryPoisonError>
    where
        I: IntoIterator<Item = (String, T, F)>,
    {
        self.refresh_with_source_limit(
            now,
            refreshed
                .into_iter()
                .map(|(id, value, fingerprint)| (id, value, fingerprint, None)),
            usize::MAX,
        )
    }

    /// Merges refreshed entries while limiting identities from each discovery source.
    ///
    /// Entries with no source are trusted and subject only to the registry's
    /// global limit. Existing identities may always be refreshed, including
    /// when either limit is already reached. Expired identities release their
    /// capacity before refreshed entries are considered. New identities are
    /// admitted in iterator order when a limit leaves fewer slots than the
    /// refresh contains.
    ///
    /// # Errors
    ///
    /// The error variant is retained for source compatibility; lock poisoning
    /// panics instead of being represented as an ordinary error.
    ///
    /// # Panics
    ///
    /// Panics if another thread panicked while holding the registry lock.
    pub fn refresh_with_source_limit<I>(
        &self,
        now: Instant,
        refreshed: I,
        maximum_entries_per_source: usize,
    ) -> Result<RegistryRefresh, RegistryPoisonError>
    where
        I: IntoIterator<Item = (String, T, F, Option<IpAddr>)>,
    {
        let mut entries = self.lock_entries();
        let before = fingerprints(&entries);
        let previous_len = entries.len();
        entries.retain(|_, entry| {
            let age = now.saturating_duration_since(entry.last_seen);
            match self.expiry_boundary {
                RegistryExpiry::AtOrAfter => age < self.expiry,
                RegistryExpiry::After => age <= self.expiry,
            }
        });
        let expired = previous_len.saturating_sub(entries.len());
        let mut entries_per_source = source_counts(&entries);
        let mut rejected = 0;

        for (id, value, fingerprint, discovery_source) in refreshed {
            let existing_source = entries.get(&id).and_then(|entry| entry.discovery_source);
            let source_has_capacity = discovery_source.is_none_or(|source| {
                entries_per_source.get(&source).copied().unwrap_or(0) < maximum_entries_per_source
            });
            if entries.contains_key(&id)
                || (entries.len() < self.maximum_entries && source_has_capacity)
            {
                if existing_source != discovery_source {
                    decrement_source_count(&mut entries_per_source, existing_source);
                    increment_source_count(&mut entries_per_source, discovery_source);
                } else if !entries.contains_key(&id) {
                    increment_source_count(&mut entries_per_source, discovery_source);
                }
                entries.insert(
                    id,
                    RegistryEntry {
                        value,
                        fingerprint,
                        discovery_source,
                        last_seen: now,
                    },
                );
            } else {
                rejected += 1;
            }
        }

        let topology_changed = before != fingerprints(&entries);
        drop(entries);

        if topology_changed {
            notification::topology_changed();
        }

        Ok(RegistryRefresh {
            topology_changed,
            rejected,
            expired,
        })
    }

    /// Returns a cloned entry by its stable plugin-local identity.
    ///
    /// # Errors
    ///
    /// The error variant is retained for source compatibility; lock poisoning
    /// panics instead of being represented as an ordinary error.
    ///
    /// # Panics
    ///
    /// Panics if another thread panicked while holding the registry lock.
    pub fn get(&self, id: &str) -> Result<Option<T>, RegistryPoisonError> {
        Ok(self.lock_entries().get(id).map(|entry| entry.value.clone()))
    }

    /// Returns all values in deterministic identity order.
    ///
    /// # Errors
    ///
    /// The error variant is retained for source compatibility; lock poisoning
    /// panics instead of being represented as an ordinary error.
    ///
    /// # Panics
    ///
    /// Panics if another thread panicked while holding the registry lock.
    pub fn snapshot(&self) -> Result<Vec<T>, RegistryPoisonError> {
        let entries = self.lock_entries();
        let mut values = entries
            .iter()
            .map(|(id, entry)| (id, entry.value.clone()))
            .collect::<Vec<_>>();
        values.sort_by(|left, right| left.0.cmp(right.0));
        Ok(values.into_iter().map(|(_, value)| value).collect())
    }

    /// Returns whether an identity is already present.
    ///
    /// # Errors
    ///
    /// The error variant is retained for source compatibility; lock poisoning
    /// panics instead of being represented as an ordinary error.
    ///
    /// # Panics
    ///
    /// Panics if another thread panicked while holding the registry lock.
    pub fn contains(&self, id: &str) -> Result<bool, RegistryPoisonError> {
        Ok(self.lock_entries().contains_key(id))
    }

    /// Returns the current number of retained entries.
    ///
    /// # Errors
    ///
    /// The error variant is retained for source compatibility; lock poisoning
    /// panics instead of being represented as an ordinary error.
    ///
    /// # Panics
    ///
    /// Panics if another thread panicked while holding the registry lock.
    pub fn len(&self) -> Result<usize, RegistryPoisonError> {
        Ok(self.lock_entries().len())
    }

    /// Returns whether the registry contains no entries.
    ///
    /// # Errors
    ///
    /// The error variant is retained for source compatibility; lock poisoning
    /// panics instead of being represented as an ordinary error.
    ///
    /// # Panics
    ///
    /// Panics if another thread panicked while holding the registry lock.
    pub fn is_empty(&self) -> Result<bool, RegistryPoisonError> {
        Ok(self.len()? == 0)
    }

    fn lock_entries(&self) -> RegistryGuard<'_, T, F> {
        self.entries.lock().expect("dynamic registry lock poisoned")
    }
}

fn source_counts<T, F>(entries: &HashMap<String, RegistryEntry<T, F>>) -> HashMap<IpAddr, usize> {
    let mut counts = HashMap::new();
    for source in entries.values().filter_map(|entry| entry.discovery_source) {
        increment_source_count(&mut counts, Some(source));
    }
    counts
}

fn increment_source_count(counts: &mut HashMap<IpAddr, usize>, source: Option<IpAddr>) {
    if let Some(source) = source {
        *counts.entry(source).or_default() += 1;
    }
}

fn decrement_source_count(counts: &mut HashMap<IpAddr, usize>, source: Option<IpAddr>) {
    if let Some(source) = source
        && let Some(count) = counts.get_mut(&source)
    {
        *count = count.saturating_sub(1);
    }
}

fn fingerprints<T, F: Clone>(entries: &HashMap<String, RegistryEntry<T, F>>) -> Vec<(String, F)> {
    let mut fingerprints = entries
        .iter()
        .map(|(id, entry)| (id.clone(), entry.fingerprint.clone()))
        .collect::<Vec<_>>();
    fingerprints.sort_by(|left, right| left.0.cmp(&right.0));
    fingerprints
}

#[cfg(test)]
#[path = "dynamic_registry_tests.rs"]
mod tests;
