// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for scene operation resolution and result commits.

use luminate_core::colour::Colour;
use luminate_core::rgb::Rgb;
use luminate_core::state::AppearanceState;

use super::*;
use crate::daemon::tests_support::{remove_request_test_dir, request_test_context};

#[test]
fn scene_operations_are_resolved_before_parallel_dispatch() {
    let (state, _manager, _mutations, runtime_dir) = request_test_context("scene-resolution");
    let target = TargetId::device("request-device");
    let colour = Colour::rgb(Rgb::new(12, 34, 56));
    let operations = vec![(
        target.clone(),
        TargetState::Effect(Effect::Static {
            colour: colour.clone(),
        }),
    )];

    let resolved = resolve_operations(&state, &operations).expect("resolve scene operations");
    assert!(matches!(
        resolved.as_slice(),
        [(resolved_target, PluginUpdateOperation::SetEffect {
            effect: Effect::Static { colour: resolved_colour }
        })] if resolved_target == &target && resolved_colour == &colour
    ));

    remove_request_test_dir(&runtime_dir);
}

#[test]
fn scene_commits_successful_operations_before_reporting_parallel_failures() {
    let (state, _manager, _mutations, runtime_dir) = request_test_context("scene-partial");
    let target = TargetId::device("request-device");
    let colour = Colour::rgb(Rgb::new(12, 34, 56));
    let operations = vec![
        (
            target.clone(),
            TargetState::Effect(Effect::Static {
                colour: colour.clone(),
            }),
        ),
        (target.clone(), TargetState::Brightness(40)),
    ];

    let error = commit_operation_outcomes(
        &state,
        &operations,
        vec![
            Ok(()),
            Err(DaemonError::Internal("second plugin failed".to_owned())),
        ],
    )
    .expect_err("one failed operation must make the scene partial");

    assert!(matches!(
        error,
        DaemonError::PartialMutation { applied_targets, diagnostic }
            if applied_targets == [target.clone()] && diagnostic.contains("second plugin failed")
    ));
    assert_eq!(
        state.blocking_lock().retained_appearance(&target),
        Some(AppearanceState::Static(colour))
    );

    remove_request_test_dir(&runtime_dir);
}

#[test]
fn scene_returns_the_first_error_when_no_operation_succeeds() {
    let (state, _manager, _mutations, runtime_dir) = request_test_context("scene-failure");
    let target = TargetId::device("request-device");
    let operations = vec![(target, TargetState::Brightness(40))];

    let error = commit_operation_outcomes(
        &state,
        &operations,
        vec![Err(DaemonError::Internal("plugin failed".to_owned()))],
    )
    .expect_err("a wholly failed scene must return its hardware error");

    assert!(matches!(error, DaemonError::Internal(message) if message == "plugin failed"));
    remove_request_test_dir(&runtime_dir);
}
