// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Tests for the parent module.

use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::thread;

use super::*;

fn registry() -> DynamicDeviceRegistry<String, String> {
    DynamicDeviceRegistry::new(2, Duration::from_secs(10), RegistryExpiry::AtOrAfter)
}

#[test]
fn discovery_pacer_waits_out_its_interval_when_nobody_wakes_it() {
    let pacer = DiscoveryPacer::new();
    let started = Instant::now();
    pacer.wait(Duration::from_millis(50));
    assert!(
        started.elapsed() >= Duration::from_millis(50),
        "an unwoken wait must not return early"
    );
}

#[test]
fn discovery_pacer_returns_immediately_for_a_wake_raised_before_the_wait() {
    // The rescan path: a wake can land while a discovery cycle is still
    // running, so it must be remembered rather than lost, or a resume's
    // rescan silently does nothing until the next ordinary interval.
    let pacer = DiscoveryPacer::new();
    pacer.wake();

    let started = Instant::now();
    pacer.wait(Duration::from_secs(30));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a pending wake must short-circuit the next wait"
    );

    // One wake produces exactly one early cycle, not a permanently hot
    // discovery loop.
    let started = Instant::now();
    pacer.wait(Duration::from_millis(50));
    assert!(
        started.elapsed() >= Duration::from_millis(50),
        "a consumed wake must not shorten the following wait"
    );
}

#[test]
fn discovery_pacer_wakes_a_thread_already_waiting() {
    let pacer = Arc::new(DiscoveryPacer::new());
    let discovery_thread = Arc::clone(&pacer);
    let handle = thread::spawn(move || {
        let started = Instant::now();
        discovery_thread.wait(Duration::from_secs(30));
        started.elapsed()
    });

    // Give the waiter time to actually block before waking it, so this
    // exercises the notify path rather than the pending-flag path above.
    thread::sleep(Duration::from_millis(50));
    pacer.wake();

    let waited = handle.join().expect("waiter thread should not panic");
    assert!(
        waited < Duration::from_secs(5),
        "wake should interrupt an in-progress wait, waited {waited:?}"
    );
}

#[test]
fn refresh_is_bounded_but_existing_identity_can_update_at_capacity() {
    let registry = registry();
    let now = Instant::now();
    let first = registry
        .refresh(
            now,
            [
                ("b".to_owned(), "B".to_owned(), "one".to_owned()),
                ("a".to_owned(), "A".to_owned(), "one".to_owned()),
                ("c".to_owned(), "C".to_owned(), "one".to_owned()),
            ],
        )
        .expect("registry should be available");

    assert_eq!(first.rejected, 1);
    assert_eq!(
        registry.snapshot().expect("registry should be available"),
        ["A", "B"]
    );

    let second = registry
        .refresh(
            now + Duration::from_secs(1),
            [("a".to_owned(), "A2".to_owned(), "one".to_owned())],
        )
        .expect("registry should be available");
    assert!(!second.topology_changed);
    assert_eq!(
        registry
            .get("a")
            .expect("registry should be available")
            .as_deref(),
        Some("A2")
    );
}

#[test]
fn topology_changes_depend_on_fingerprints_not_transport_data() {
    let registry = registry();
    let now = Instant::now();
    assert!(
        registry
            .refresh(
                now,
                [("a".to_owned(), "address-1".to_owned(), "same".to_owned())]
            )
            .expect("registry should be available")
            .topology_changed
    );
    assert!(
        !registry
            .refresh(
                now,
                [("a".to_owned(), "address-2".to_owned(), "same".to_owned())]
            )
            .expect("registry should be available")
            .topology_changed
    );
}

#[test]
fn stale_entries_expire_at_the_configured_boundary() {
    let registry = registry();
    let now = Instant::now();
    registry
        .refresh(now, [("a".to_owned(), "A".to_owned(), "one".to_owned())])
        .expect("registry should be available");

    let refresh = registry
        .refresh(now + Duration::from_secs(10), [])
        .expect("registry should be available");

    assert_eq!(refresh.expired, 1);
    assert!(refresh.topology_changed);
    assert!(registry.is_empty().expect("registry should be available"));
}

#[test]
fn after_policy_retains_an_entry_at_the_boundary() {
    let registry = DynamicDeviceRegistry::new(1, Duration::from_secs(10), RegistryExpiry::After);
    let now = Instant::now();
    registry
        .refresh(now, [("a".to_owned(), "A".to_owned(), "one".to_owned())])
        .expect("registry should be available");

    assert_eq!(
        registry
            .refresh(now + Duration::from_secs(10), [])
            .expect("registry should be available")
            .expired,
        0
    );
    assert_eq!(
        registry
            .refresh(now + Duration::from_secs(10) + Duration::from_nanos(1), [])
            .expect("registry should be available")
            .expired,
        1
    );
}

#[test]
fn expired_entry_releases_global_capacity_before_admission() {
    for (boundary, elapsed) in [
        (RegistryExpiry::AtOrAfter, Duration::from_secs(10)),
        (
            RegistryExpiry::After,
            Duration::from_secs(10) + Duration::from_nanos(1),
        ),
    ] {
        let registry = DynamicDeviceRegistry::new(1, Duration::from_secs(10), boundary);
        let now = Instant::now();
        registry
            .refresh(
                now,
                [("old".to_owned(), "Old".to_owned(), "old".to_owned())],
            )
            .expect("registry should be available");

        let refresh = registry
            .refresh(
                now + elapsed,
                [("new".to_owned(), "New".to_owned(), "new".to_owned())],
            )
            .expect("registry should be available");

        assert_eq!(refresh.expired, 1);
        assert_eq!(refresh.rejected, 0);
        assert!(refresh.topology_changed);
        assert_eq!(
            registry.snapshot().expect("registry should be available"),
            ["New"]
        );
    }
}

#[test]
fn expired_entry_releases_source_capacity_before_admission() {
    for (boundary, elapsed) in [
        (RegistryExpiry::AtOrAfter, Duration::from_secs(10)),
        (
            RegistryExpiry::After,
            Duration::from_secs(10) + Duration::from_nanos(1),
        ),
    ] {
        let registry = DynamicDeviceRegistry::new(2, Duration::from_secs(10), boundary);
        let now = Instant::now();
        let source = IpAddr::from([192, 0, 2, 1]);
        registry
            .refresh_with_source_limit(
                now,
                [(
                    "old".to_owned(),
                    "Old".to_owned(),
                    "old".to_owned(),
                    Some(source),
                )],
                1,
            )
            .expect("registry should be available");

        let refresh = registry
            .refresh_with_source_limit(
                now + elapsed,
                [(
                    "new".to_owned(),
                    "New".to_owned(),
                    "new".to_owned(),
                    Some(source),
                )],
                1,
            )
            .expect("registry should be available");

        assert_eq!(refresh.expired, 1);
        assert_eq!(refresh.rejected, 0);
        assert_eq!(
            registry.snapshot().expect("registry should be available"),
            ["New"]
        );
    }
}

#[test]
fn over_capacity_new_identities_are_admitted_in_iterator_order() {
    for (first, second) in [("a", "b"), ("b", "a")] {
        let registry =
            DynamicDeviceRegistry::new(1, Duration::from_secs(10), RegistryExpiry::After);
        let now = Instant::now();
        let refresh = registry
            .refresh(
                now,
                [
                    (first.to_owned(), first.to_owned(), first.to_owned()),
                    (second.to_owned(), second.to_owned(), second.to_owned()),
                ],
            )
            .expect("registry should be available");

        assert_eq!(refresh.rejected, 1);
        assert_eq!(
            registry.snapshot().expect("registry should be available"),
            [first]
        );
    }
}

#[test]
fn source_limit_rejects_new_identities_but_allows_refreshes_and_trusted_entries() {
    let registry =
        DynamicDeviceRegistry::new(4, Duration::from_secs(10), RegistryExpiry::AtOrAfter);
    let now = Instant::now();
    let source = IpAddr::from([192, 0, 2, 1]);
    let other_source = IpAddr::from([192, 0, 2, 2]);

    let first = registry
        .refresh_with_source_limit(
            now,
            [
                (
                    "a".to_owned(),
                    "A".to_owned(),
                    "one".to_owned(),
                    Some(source),
                ),
                (
                    "b".to_owned(),
                    "B".to_owned(),
                    "one".to_owned(),
                    Some(source),
                ),
                (
                    "c".to_owned(),
                    "C".to_owned(),
                    "one".to_owned(),
                    Some(other_source),
                ),
                ("trusted".to_owned(), "T".to_owned(), "one".to_owned(), None),
            ],
            1,
        )
        .expect("registry should be available");

    assert_eq!(first.rejected, 1);
    assert_eq!(
        registry.snapshot().expect("registry should be available"),
        ["A", "C", "T"]
    );
    let second = registry
        .refresh_with_source_limit(
            now + Duration::from_secs(1),
            [(
                "a".to_owned(),
                "A2".to_owned(),
                "one".to_owned(),
                Some(source),
            )],
            1,
        )
        .expect("registry should be available");
    assert_eq!(second.rejected, 0);
    assert_eq!(
        registry
            .get("a")
            .expect("registry should be available")
            .as_deref(),
        Some("A2")
    );
}

#[test]
fn poisoned_registry_refuses_every_access_path() {
    let registry = registry();
    let poisoned = panic::catch_unwind(AssertUnwindSafe(|| {
        let _entries = registry.entries.lock().expect("fresh registry lock");
        panic!("deliberately poison registry lock");
    }));
    assert!(poisoned.is_err());

    assert!(
        panic::catch_unwind(AssertUnwindSafe(|| {
            registry.refresh(Instant::now(), [])
        }))
        .is_err()
    );
    assert!(panic::catch_unwind(AssertUnwindSafe(|| registry.get("a"))).is_err());
    assert!(panic::catch_unwind(AssertUnwindSafe(|| registry.snapshot())).is_err());
    assert!(panic::catch_unwind(AssertUnwindSafe(|| registry.contains("a"))).is_err());
    assert!(panic::catch_unwind(AssertUnwindSafe(|| registry.len())).is_err());
    assert!(panic::catch_unwind(AssertUnwindSafe(|| registry.is_empty())).is_err());
}
