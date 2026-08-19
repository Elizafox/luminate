// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for appearance restoration and persistence dispatch.

use super::*;
use luminate_core::colour::Colour;
use luminate_core::rgb::Rgb;

use super::super::super::tests_support::{remove_request_test_dir, request_test_context};

fn static_effect() -> Effect {
    Effect::Static {
        colour: Colour::rgb(Rgb::new(10, 20, 30)),
    }
}

#[test]
fn appearance_projection_preserves_restorable_effects_only() {
    assert!(matches!(
        appearance_target_state(AppearanceState::Static(Colour::rgb(Rgb::new(1, 2, 3)))),
        Some(TargetState::Effect(Effect::Static { colour }))
            if colour == Colour::rgb(Rgb::new(1, 2, 3))
    ));
    assert!(matches!(
        appearance_target_state(AppearanceState::Effect(Effect::Spectrum { period_ms: 100 })),
        Some(TargetState::Effect(Effect::Spectrum { period_ms: 100 }))
    ));
    assert!(appearance_target_state(AppearanceState::Effect(Effect::Off)).is_none());
    assert!(appearance_target_state(AppearanceState::Mixed).is_none());

    assert!(target_states_equal(
        &TargetState::Effect(static_effect()),
        &TargetState::Effect(static_effect()),
    ));
    assert!(!target_states_equal(
        &TargetState::Effect(static_effect()),
        &TargetState::Brightness(50),
    ));
}

#[test]
fn restoration_preflights_unknown_state_before_touching_hardware() {
    let (state, manager, _mutations, runtime_dir) = request_test_context("restore-unknown");
    let target = TargetId::device("request-device");

    let error = restore_appearances(&state, &manager, slice::from_ref(&target))
        .expect_err("unknown retained appearance must fail preflight");
    assert!(matches!(error, DaemonError::UnknownState { target: failed } if failed == target));

    remove_request_test_dir(&runtime_dir);
}

#[test]
fn restoration_reports_hardware_failure_without_claiming_an_application() {
    let (state, manager, _mutations, runtime_dir) = request_test_context("restore-unavailable");
    let target = TargetId::device("request-device");
    state
        .blocking_lock()
        .set_effect(target.clone(), static_effect())
        .expect("record retained appearance");

    let error = restore_appearances(&state, &manager, slice::from_ref(&target))
        .expect_err("an empty plugin manager cannot apply the appearance");
    assert!(!matches!(error, DaemonError::PartialMutation { .. }));
    assert_eq!(
        state.blocking_lock().retained_appearance(&target),
        Some(AppearanceState::Static(Colour::rgb(Rgb::new(10, 20, 30))))
    );

    remove_request_test_dir(&runtime_dir);
}

#[test]
fn required_write_through_persistence_avoids_a_redundant_hardware_save() {
    let (state, manager, _mutations, runtime_dir) = request_test_context("save-write-through");
    let target = TargetId::device("request-device");
    state
        .blocking_lock()
        .set_effect(target.clone(), static_effect())
        .expect("record known target state");

    save_current_target(&state, &manager, &target)
        .expect("required write-through persistence is already durable");
    save_current_many(&state, &manager, slice::from_ref(&target))
        .expect("collection save shares the same durability rule");

    remove_request_test_dir(&runtime_dir);
}

#[test]
fn resolved_mutation_preflights_support_and_reports_first_hardware_failure() {
    let (state, manager, _mutations, runtime_dir) = request_test_context("resolved-mutation");
    let target = TargetId::device("request-device");

    let unsupported = apply_collection_mutation(
        &state,
        &manager,
        None,
        slice::from_ref(&target),
        &TargetState::Brightness(101),
        UnsupportedPolicy::Reject,
    )
    .expect_err("brightness above the advertised maximum must fail preflight");
    assert!(!matches!(unsupported, DaemonError::PartialMutation { .. }));

    let unavailable = apply_collection_mutation(
        &state,
        &manager,
        None,
        slice::from_ref(&target),
        &TargetState::Brightness(50),
        UnsupportedPolicy::Reject,
    )
    .expect_err("an empty plugin manager cannot apply supported brightness");
    assert!(!matches!(unavailable, DaemonError::PartialMutation { .. }));

    remove_request_test_dir(&runtime_dir);
}
