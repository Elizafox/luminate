// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use luminate_core::colour::Colour;
use luminate_core::rgb::Rgb;
use luminate_core::state::{AppearanceState, FacetValue, StateFacetKind};
use luminate_core::target::TargetId;

use super::super::tests_support::demo_state;
use super::*;

fn device() -> DeviceId {
    DeviceId::new("demo-kbd")
}

#[test]
fn begin_reconciliation_marks_reconciling_and_clears_prior_error() {
    let mut state = demo_state();
    let device = device();
    state.fail_reconciliation(&device, "earlier failure".to_owned());

    state.begin_reconciliation(&device);

    let status = state.device_status_mut(&device);
    assert_eq!(status.reconciliation, ReconciliationStatus::Reconciling);
    assert!(status.latest_attempt_ms.is_some());
    assert!(status.latest_error.is_none());
}

#[test]
fn fail_reconciliation_marks_matching_observations_stale_and_records_diagnostic() {
    let mut state = demo_state();
    let device = device();
    let target = TargetId::device("demo-kbd");
    state
        .set_static_colour_for_test(target.clone(), Colour::rgb(Rgb::new(1, 2, 3)))
        .expect("set colour to create an observation");

    state.fail_reconciliation(&device, "hardware unreachable".to_owned());

    let status = state.device_status_mut(&device);
    assert_eq!(status.reachability, Reachability::Unavailable);
    assert_eq!(status.reconciliation, ReconciliationStatus::Failed);
    assert_eq!(status.latest_error.as_deref(), Some("hardware unreachable"));
    assert!(status.latest_attempt_ms.is_some());
    assert!(
        state
            .observations
            .iter()
            .filter(|(key, _)| key.0.device_id() == &device)
            .all(|(_, observation)| observation.stale),
        "every observation belonging to the failed device should be marked stale"
    );
}

#[test]
fn fail_adoption_persistence_marks_candidate_facets_and_degrades_status() {
    let mut state = demo_state();
    let device = device();
    let target = TargetId::device("demo-kbd");
    let candidate = vec![AdoptedFacet {
        target: target.clone(),
        value: FacetValue::Appearance(AppearanceState::Static(Colour::rgb(Rgb::new(4, 5, 6)))),
        confirmed_at_ms: 1,
    }];

    state.fail_adoption_persistence(&device, &candidate, "disk full".to_owned());

    let status = state.device_status_mut(&device);
    assert_eq!(status.reconciliation, ReconciliationStatus::Failed);
    assert_eq!(status.latest_error.as_deref(), Some("disk full"));
    assert!(
        status
            .adoption
            .iter()
            .any(|(recorded_target, kind, adoption)| {
                recorded_target == &target
                    && *kind == StateFacetKind::Appearance
                    && *adoption == AdoptionStatus::PersistenceFailed
            })
    );
}

#[test]
fn complete_reconciliation_marks_reachable_and_complete() {
    let mut state = demo_state();
    let device = device();
    state.fail_reconciliation(&device, "was failing".to_owned());

    state.complete_reconciliation(&device);

    let status = state.device_status_mut(&device);
    assert_eq!(status.reachability, Reachability::Reachable);
    assert_eq!(status.reconciliation, ReconciliationStatus::Complete);
    assert!(status.latest_attempt_ms.is_some());
}

#[test]
fn mark_reconciliation_drifted_records_diagnostic_without_touching_reachability() {
    let mut state = demo_state();
    let device = device();
    state.complete_reconciliation(&device);

    state.mark_reconciliation_drifted(&device, "desired state disagreed".to_owned());

    let status = state.device_status_mut(&device);
    assert_eq!(status.reconciliation, ReconciliationStatus::Drifted);
    assert_eq!(
        status.latest_error.as_deref(),
        Some("desired state disagreed")
    );
    // Reachability is untouched by drift detection: the device answered,
    // it just didn't answer with what was expected.
    assert_eq!(status.reachability, Reachability::Reachable);
}

#[test]
fn partial_reconciliation_failure_marks_reachable_but_failed() {
    let mut state = demo_state();
    let device = device();

    state.partial_reconciliation_failure(&device, "one facet failed".to_owned());

    let status = state.device_status_mut(&device);
    assert_eq!(status.reachability, Reachability::Reachable);
    assert_eq!(status.reconciliation, ReconciliationStatus::Failed);
    assert_eq!(status.latest_error.as_deref(), Some("one facet failed"));
    assert!(status.latest_attempt_ms.is_some());
}

#[test]
fn now_ms_returns_a_plausible_unix_timestamp() {
    // Sanity bound rather than an exact value: this just guards against
    // `now_ms` silently regressing to the `map_or` fallback of zero.
    assert!(now_ms() > 1_700_000_000_000);
}
