// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use luminate_core::device::DeviceId;
use luminate_core::target::TargetId;

use super::*;

fn status(index: usize) -> TransitionStatus {
    TransitionStatus {
        id: TransitionId::new(format!("transition-{index}")),
        targets: vec![TargetId::Device(DeviceId::new("lamp"))],
        elapsed_ms: 0,
        duration_ms: 100,
        outcome: None,
    }
}

#[test]
fn registry_retains_only_latest_256_terminal_statuses() {
    let registry = TransitionRegistry::default();
    for index in 0..257 {
        let status = status(index);
        let id = status.id.clone();
        registry.insert(status, None);
        registry.finish(&id, TransitionOutcome::Completed);
    }
    assert!(registry.get(&TransitionId::new("transition-0")).is_none());
    assert!(registry.get(&TransitionId::new("transition-1")).is_some());
    assert!(registry.get(&TransitionId::new("transition-256")).is_some());
}

#[test]
fn entry_tracks_elapsed_lease_and_first_cancellation_reason() {
    let entry = TransitionEntry::new(status(1), Some(100));
    assert_eq!(entry.snapshot().elapsed_ms, 0);
    assert!(entry.lease_deadline().is_some());
    entry.update_elapsed(42);
    assert_eq!(entry.snapshot().elapsed_ms, 42);

    for reason in [
        TransitionCancellation::Aborted,
        TransitionCancellation::Replaced,
        TransitionCancellation::ConflictingMutation,
        TransitionCancellation::AuthorizationExpired,
    ] {
        let entry = TransitionEntry::new(status(2), None);
        entry.request_abort(reason);
        assert_eq!(entry.cancellation(), Some(reason));
        entry.request_abort(TransitionCancellation::Replaced);
        assert_eq!(entry.cancellation(), Some(reason));
    }

    let before = entry.lease_deadline();
    entry.renew(250);
    assert!(entry.lease_deadline() > before);
}

#[tokio::test]
async fn finished_entries_leave_active_registry_and_release_waiters() {
    let registry = TransitionRegistry::default();
    let current = status(10);
    let id = current.id.clone();
    let entry = registry.insert(current, None);
    assert_eq!(registry.active().len(), 1);

    let waiter = Arc::clone(&entry);
    let waiting = tokio::spawn(async move {
        waiter.wait_finished().await;
        waiter.snapshot().outcome
    });
    registry.finish(
        &id,
        TransitionOutcome::Cancelled(TransitionCancellation::Aborted),
    );
    assert_eq!(
        waiting.await.expect("waiter task"),
        Some(TransitionOutcome::Cancelled(
            TransitionCancellation::Aborted
        ))
    );
    assert!(registry.active().is_empty());
    assert_eq!(
        registry
            .get(&id)
            .expect("terminal entry retained")
            .snapshot()
            .outcome,
        Some(TransitionOutcome::Cancelled(
            TransitionCancellation::Aborted
        ))
    );
}
