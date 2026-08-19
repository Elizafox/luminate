// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::collections::HashSet;

use super::*;

#[test]
fn event_ids_are_unique_and_in_allocated_ranges() {
    let mut ids = HashSet::new();

    for event in OperatorEvent::ALL {
        let id = event.id();
        assert!(ids.insert(id), "duplicate operator Event ID {id}");
        assert!(
            (100..=499).contains(&id),
            "operator Event ID {id} is outside an allocated phase 1 range"
        );
    }
}

#[test]
fn event_ids_remain_stable() {
    assert_eq!(OperatorEvent::ServiceStarting.id(), 100);
    assert_eq!(OperatorEvent::ServiceReady.id(), 101);
    assert_eq!(OperatorEvent::ServiceStopRequested.id(), 102);
    assert_eq!(OperatorEvent::ServicePreshutdownRequested.id(), 103);
    assert_eq!(OperatorEvent::ServiceStopped.id(), 104);
    assert_eq!(OperatorEvent::ServiceLoggingDegraded.id(), 105);
    assert_eq!(OperatorEvent::ConfigurationLoadFailed.id(), 200);
    assert_eq!(OperatorEvent::PersistedStateLoadFailed.id(), 201);
    assert_eq!(OperatorEvent::PersistedStateSaveFailed.id(), 202);
    assert_eq!(OperatorEvent::ListenerBindFailed.id(), 300);
    assert_eq!(OperatorEvent::PluginHostCrashed.id(), 400);
}
