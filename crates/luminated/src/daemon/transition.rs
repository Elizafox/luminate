// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! In-memory transition status retention and cancellation coordination.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use luminate_core::transition::{
    TransitionCancellation, TransitionId, TransitionOutcome, TransitionStatus,
};
use tokio::sync::Notify;
use tokio::time::Instant;

const TERMINAL_STATUS_LIMIT: usize = 256;

pub(super) struct TransitionEntry {
    status: Mutex<TransitionStatus>,
    cancellation: AtomicU8,
    changed: Notify,
    finished: Notify,
    lease_deadline: Mutex<Option<Instant>>,
}

impl TransitionEntry {
    pub(super) fn new(status: TransitionStatus, renewable_lease_ms: Option<u64>) -> Self {
        Self {
            status: Mutex::new(status),
            cancellation: AtomicU8::new(0),
            changed: Notify::new(),
            finished: Notify::new(),
            lease_deadline: Mutex::new(
                renewable_lease_ms.map(|lease| Instant::now() + Duration::from_millis(lease)),
            ),
        }
    }

    pub(super) fn snapshot(&self) -> TransitionStatus {
        self.status.lock().expect("lock poisoned").clone()
    }

    pub(super) fn update_elapsed(&self, elapsed_ms: u64) {
        self.status.lock().expect("lock poisoned").elapsed_ms = elapsed_ms;
    }

    pub(super) fn request_abort(&self, reason: TransitionCancellation) {
        let value = match reason {
            TransitionCancellation::Aborted => 1,
            TransitionCancellation::Replaced => 2,
            TransitionCancellation::ConflictingMutation => 3,
            TransitionCancellation::AuthorizationExpired => 4,
        };
        let _ = self
            .cancellation
            .compare_exchange(0, value, Ordering::AcqRel, Ordering::Acquire);
        self.changed.notify_waiters();
    }

    pub(super) fn cancellation(&self) -> Option<TransitionCancellation> {
        match self.cancellation.load(Ordering::Acquire) {
            1 => Some(TransitionCancellation::Aborted),
            2 => Some(TransitionCancellation::Replaced),
            3 => Some(TransitionCancellation::ConflictingMutation),
            4 => Some(TransitionCancellation::AuthorizationExpired),
            _ => None,
        }
    }

    pub(super) async fn wait_for_change(&self) {
        self.changed.notified().await;
    }

    pub(super) async fn wait_finished(&self) {
        loop {
            if self.snapshot().is_terminal() {
                return;
            }
            self.finished.notified().await;
        }
    }

    pub(super) fn renew(&self, lease_ms: u64) {
        *self.lease_deadline.lock().expect("lock poisoned") =
            Some(Instant::now() + Duration::from_millis(lease_ms));
        self.changed.notify_waiters();
    }

    pub(super) fn lease_deadline(&self) -> Option<Instant> {
        *self.lease_deadline.lock().expect("lock poisoned")
    }

    fn finish(&self, outcome: TransitionOutcome) {
        self.status.lock().expect("lock poisoned").outcome = Some(outcome);
        self.finished.notify_waiters();
    }
}

#[derive(Default)]
pub(super) struct TransitionRegistry {
    entries: Mutex<HashMap<TransitionId, Arc<TransitionEntry>>>,
    terminal: Mutex<VecDeque<TransitionId>>,
}

impl TransitionRegistry {
    pub(super) fn insert(
        &self,
        status: TransitionStatus,
        renewable_lease_ms: Option<u64>,
    ) -> Arc<TransitionEntry> {
        let id = status.id.clone();
        let entry = Arc::new(TransitionEntry::new(status, renewable_lease_ms));
        self.entries
            .lock()
            .expect("lock poisoned")
            .insert(id, Arc::clone(&entry));
        entry
    }

    pub(super) fn get(&self, id: &TransitionId) -> Option<Arc<TransitionEntry>> {
        self.entries.lock().expect("lock poisoned").get(id).cloned()
    }

    pub(super) fn active(&self) -> Vec<Arc<TransitionEntry>> {
        self.entries
            .lock()
            .expect("lock poisoned")
            .values()
            .filter(|entry| !entry.snapshot().is_terminal())
            .cloned()
            .collect()
    }

    pub(super) fn finish(&self, id: &TransitionId, outcome: TransitionOutcome) {
        if let Some(entry) = self.get(id) {
            entry.finish(outcome);
        }
        let evicted = {
            let mut terminal = self.terminal.lock().expect("lock poisoned");
            terminal.push_back(id.clone());
            (terminal.len() > TERMINAL_STATUS_LIMIT)
                .then(|| terminal.pop_front())
                .flatten()
        };
        if let Some(evicted) = evicted {
            self.entries.lock().expect("lock poisoned").remove(&evicted);
        }
    }
}

#[cfg(test)]
#[path = "transition_tests.rs"]
mod tests;
