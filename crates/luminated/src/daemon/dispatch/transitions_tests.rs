// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;
use std::slice;
use std::time::Duration;

use luminate_core::capability::{ColourChannelCapability, HardwareEffectId};
use luminate_core::effect::EffectArguments;

use super::super::super::tests_support::{remove_request_test_dir, request_test_context};

fn transition_status(id: &str, targets: Vec<TargetId>, duration_ms: u64) -> TransitionStatus {
    TransitionStatus {
        id: TransitionId::new(id),
        targets,
        elapsed_ms: 0,
        duration_ms,
        outcome: None,
    }
}

#[test]
fn unknowable_impossibility_has_the_canonical_diagnostic() {
    assert_eq!(
        impossible("").to_string(),
        "transition is impossible: It's just not."
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "This test covers one cohesive interpolation capability decision table."
)]
fn interpolation_capability_checks_cover_rgb_shape_and_hue_width_boundaries() {
    let rgb = ColourCapability::rgb8();
    assert!(is_rgb8_capability(&rgb));
    for capability in [
        ColourCapability::monochrome(8),
        ColourCapability::Additive(vec![
            ColourChannelCapability::new(ColourChannel::Red, 8),
            ColourChannelCapability::new(ColourChannel::Green, 8),
        ]),
        ColourCapability::Additive(vec![
            ColourChannelCapability::new(ColourChannel::Blue, 8),
            ColourChannelCapability::new(ColourChannel::Green, 8),
            ColourChannelCapability::new(ColourChannel::Red, 8),
        ]),
        ColourCapability::Additive(vec![
            ColourChannelCapability::new(ColourChannel::Red, 16),
            ColourChannelCapability::new(ColourChannel::Green, 8),
            ColourChannelCapability::new(ColourChannel::Blue, 8),
        ]),
    ] {
        assert!(!is_rgb8_capability(&capability));
    }

    let mut descriptor = super::super::super::tests_support::request_test_descriptor();
    let target = TargetId::device(descriptor.id.clone());
    let additive = Effect::Static {
        colour: Colour::rgb(Rgb::new(1, 2, 3)),
    };
    let rgb_state = super::super::DaemonState::from_descriptors(slice::from_ref(&descriptor))
        .expect("build RGB state");
    let endpoint = Endpoint {
        target: target.clone(),
        appearance: Effect::Static {
            colour: Colour::rgb(Rgb::new(1, 2, 3)),
        },
        brightness: Some(50),
        emission: EmissionState::Dark,
    };
    validate_endpoint(&rgb_state, &endpoint).expect("supported dark endpoint should validate");
    assert!(
        validate_endpoint(
            &rgb_state,
            &Endpoint {
                brightness: Some(101),
                ..endpoint
            }
        )
        .is_err()
    );
    assert!(
        validate_colour_interpolation(
            &rgb_state,
            &target,
            &additive,
            TransitionColourInterpolation::Oklab,
        )
        .is_ok()
    );
    assert!(
        validate_colour_interpolation(
            &rgb_state,
            &target,
            &Effect::Spectrum { period_ms: 100 },
            TransitionColourInterpolation::Oklab,
        )
        .is_ok()
    );
    assert!(
        validate_colour_interpolation(
            &rgb_state,
            &target,
            &additive,
            TransitionColourInterpolation::default(),
        )
        .is_ok()
    );

    descriptor.capabilities.colour = vec![ColourCapability::monochrome(8)];
    let monochrome_state =
        super::super::DaemonState::from_descriptors(slice::from_ref(&descriptor))
            .expect("build monochrome state");
    assert!(
        validate_colour_interpolation(
            &monochrome_state,
            &target,
            &additive,
            TransitionColourInterpolation::Oklab,
        )
        .is_err()
    );
    let hsv_without_hue_capability = Endpoint {
        target: target.clone(),
        appearance: Effect::Static {
            colour: Colour::hsv(1, 2, 3),
        },
        brightness: None,
        emission: EmissionState::Emitting,
    };
    assert_eq!(
        hue_maximum(&monochrome_state, &hsv_without_hue_capability)
            .expect("an absent matching model uses the logical range"),
        u32::MAX
    );

    descriptor.capabilities.colour = vec![ColourCapability::Hsv {
        hue_bits: 8,
        saturation_bits: 8,
        value_bits: 8,
    }];
    let hsv_state = super::super::DaemonState::from_descriptors(slice::from_ref(&descriptor))
        .expect("build HSV state");
    let endpoint = Endpoint {
        target: target.clone(),
        appearance: Effect::Static {
            colour: Colour::hsv(1, 2, 3),
        },
        brightness: None,
        emission: EmissionState::Emitting,
    };
    assert_eq!(
        hue_maximum(&hsv_state, &endpoint).expect("valid width"),
        255
    );
    assert_eq!(
        hue_maximum(
            &hsv_state,
            &Endpoint {
                appearance: Effect::Spectrum { period_ms: 100 },
                ..endpoint.clone()
            }
        )
        .expect("non-static effects use the full logical range"),
        u32::MAX
    );

    descriptor.capabilities.colour = vec![ColourCapability::Hsv {
        hue_bits: 32,
        saturation_bits: 8,
        value_bits: 8,
    }];
    let full_width_state =
        super::super::DaemonState::from_descriptors(slice::from_ref(&descriptor))
            .expect("build full-width HSV state");
    assert_eq!(
        hue_maximum(&full_width_state, &endpoint).expect("32-bit hue is valid"),
        u32::MAX
    );

    descriptor.capabilities.colour = vec![ColourCapability::Hsl {
        hue_bits: 12,
        saturation_bits: 8,
        lightness_bits: 8,
    }];
    let hsl_capability_state =
        super::super::DaemonState::from_descriptors(slice::from_ref(&descriptor))
            .expect("build HSL state");
    let hsl_endpoint = Endpoint {
        appearance: Effect::Static {
            colour: Colour::hsl(1, 2, 3),
        },
        ..endpoint.clone()
    };
    assert_eq!(
        hue_maximum(&hsl_capability_state, &hsl_endpoint).expect("HSL width is resolved"),
        4095
    );

    descriptor.capabilities.colour = vec![ColourCapability::Hsv {
        hue_bits: 0,
        saturation_bits: 8,
        value_bits: 8,
    }];
    let invalid_state = super::super::DaemonState::from_descriptors(slice::from_ref(&descriptor))
        .expect("state accepts descriptor before request validation");
    assert!(hue_maximum(&invalid_state, &endpoint).is_err());
}

#[test]
fn sparse_transition_validation_filters_authorized_targets_and_rejects_bad_shapes() {
    let first = TargetId::device("first");
    let second = TargetId::device("second");
    let state = SceneTargetState {
        appearance: Some(Effect::Static {
            colour: Colour::rgb(Rgb::new(1, 2, 3)),
        }),
        brightness: Some(50),
        emission: Some(EmissionState::Emitting),
        appearance_slots: None,
    };
    let states = vec![
        TransitionTargetState {
            target: first.clone(),
            state: state.clone(),
        },
        TransitionTargetState {
            target: second.clone(),
            state,
        },
    ];
    let normalized = normalize_target_states(&states, Some(slice::from_ref(&first)))
        .expect("authorised target should be retained");
    assert_eq!(normalized.len(), 1);
    assert!(normalized.contains_key(&first));

    let duplicate = vec![
        states[0].clone(),
        TransitionTargetState {
            target: first,
            state: states[0].state.clone(),
        },
    ];
    assert!(normalize_target_states(&duplicate, None).is_err());
    assert!(
        validate_sparse_transition_state(&SceneTargetState {
            appearance: None,
            brightness: None,
            emission: None,
            appearance_slots: None,
        })
        .is_err()
    );
    assert!(
        validate_sparse_transition_state(&SceneTargetState {
            appearance: Some(Effect::Off),
            brightness: Some(50),
            emission: Some(EmissionState::Dark),
            appearance_slots: None,
        })
        .is_err()
    );
    assert!(
        validate_sparse_transition_state(&SceneTargetState {
            appearance: Some(Effect::Static {
                colour: Colour::rgb(Rgb::new(1, 2, 3)),
            }),
            brightness: Some(0),
            emission: Some(EmissionState::Emitting),
            appearance_slots: None,
        })
        .is_err()
    );
}

#[test]
fn transition_endpoint_helpers_preserve_sparse_facets_and_failure_metadata() {
    let target = TargetId::device("lamp");
    let fallback = SceneTargetState {
        appearance: Some(Effect::Static {
            colour: Colour::rgb(Rgb::new(1, 2, 3)),
        }),
        brightness: Some(20),
        emission: Some(EmissionState::Dark),
        appearance_slots: None,
    };
    let merged = merge(
        &SceneTargetState {
            appearance: None,
            brightness: Some(80),
            emission: None,
            appearance_slots: None,
        },
        &fallback,
    );
    assert_eq!(merged.brightness, Some(80));
    assert_eq!(merged.appearance, fallback.appearance);
    assert_eq!(merged.emission, fallback.emission);

    let endpoint = complete(target.clone(), merged).expect("fallback completes endpoint");
    assert_eq!(endpoint.target, target);
    assert_eq!(endpoint.brightness, Some(80));
    assert_eq!(endpoint.emission, EmissionState::Dark);
    assert!(
        complete(
            TargetId::device("missing"),
            SceneTargetState {
                appearance: None,
                brightness: Some(1),
                emission: Some(EmissionState::Dark),
                appearance_slots: None,
            }
        )
        .is_err()
    );

    let diagnostic = transition_failure(DaemonError::PartialMutation {
        diagnostic: "one target failed".to_owned(),
        applied_targets: vec![TargetId::device("lamp")],
    });
    assert_eq!(diagnostic.0, "one target failed");
    assert_eq!(diagnostic.1, vec![TargetId::device("lamp")]);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "This test covers the complete effect-darkening decision table."
)]
fn darkening_preserves_effect_shape_and_rejects_non_dimmable_effects() {
    let rgb = Rgb::new(20, 30, 40);
    let additive = Colour::additive(vec![
        ColourChannelValue::new(ColourChannel::Red, 200),
        ColourChannelValue::new(ColourChannel::Green, 100),
        ColourChannelValue::new(ColourChannel::Blue, 50),
    ])
    .expect("valid additive colour");

    assert_eq!(
        darken_effect(&Effect::Static {
            colour: Colour::rgb(rgb),
        })
        .expect("RGB can be darkened"),
        Effect::Static {
            colour: Colour::rgb(Rgb::new(0, 0, 0)),
        }
    );
    assert_eq!(
        darken_effect(&Effect::Static {
            colour: additive.clone(),
        })
        .expect("additive colour can be darkened"),
        Effect::Static {
            colour: Colour::additive(vec![
                ColourChannelValue::new(ColourChannel::Red, 0),
                ColourChannelValue::new(ColourChannel::Green, 0),
                ColourChannelValue::new(ColourChannel::Blue, 0),
            ])
            .expect("valid dark additive colour"),
        }
    );

    let shape_preserving = [
        Effect::Breathe {
            colour: rgb,
            period_ms: 10,
        },
        Effect::Pulse {
            colour: rgb,
            period_ms: 20,
        },
        Effect::Strobe {
            colour: rgb,
            period_ms: 30,
        },
        Effect::Scanner {
            colour: rgb,
            period_ms: 40,
        },
        Effect::Morph {
            colours: vec![Rgb::new(1, 2, 3), Rgb::new(4, 5, 6)],
            period_ms: 50,
        },
    ];
    for effect in shape_preserving {
        assert!(matches!(
            darken_effect(&effect),
            Ok(Effect::Breathe { .. }
                | Effect::Pulse { .. }
                | Effect::Strobe { .. }
                | Effect::Scanner { .. }
                | Effect::Morph { .. })
        ));
    }

    let arguments = EffectArguments {
        brightness: Some(80),
        ..EffectArguments::default()
    };
    assert_eq!(
        darken_effect(&Effect::Hardware {
            id: HardwareEffectId::new("brightness"),
            arguments,
        })
        .expect("hardware brightness can be darkened"),
        Effect::Hardware {
            id: HardwareEffectId::new("brightness"),
            arguments: EffectArguments {
                brightness: Some(0),
                ..EffectArguments::default()
            },
        }
    );

    let arguments = EffectArguments {
        colours: vec![Rgb::new(1, 2, 3)],
        ..EffectArguments::default()
    };
    assert!(matches!(
        darken_effect(&Effect::Hardware {
            id: HardwareEffectId::new("colour"),
            arguments,
        }),
        Ok(Effect::Hardware { .. })
    ));

    for effect in [
        Effect::Off,
        Effect::Spectrum { period_ms: 1 },
        Effect::Rainbow { period_ms: 1 },
        Effect::Hardware {
            id: HardwareEffectId::new("empty"),
            arguments: EffectArguments::default(),
        },
        Effect::Static {
            colour: Colour::cct(2_700),
        },
    ] {
        assert!(darken_effect(&effect).is_err());
    }
}

#[test]
fn interpolation_handles_dark_logical_brightness_and_terminal_steps() {
    let plan = TargetPlan {
        start: Endpoint {
            target: TargetId::device("lamp"),
            appearance: Effect::Static {
                colour: Colour::rgb(Rgb::new(0, 0, 0)),
            },
            brightness: Some(0),
            emission: EmissionState::Dark,
        },
        end: Endpoint {
            target: TargetId::device("lamp"),
            appearance: Effect::Static {
                colour: Colour::rgb(Rgb::new(100, 100, 100)),
            },
            brightness: Some(100),
            emission: EmissionState::Emitting,
        },
        hue_maximum: u32::MAX,
    };
    let options =
        TransitionOptions::new(Duration::from_millis(100), None).expect("valid transition options");
    let midpoint = interpolate_plans(slice::from_ref(&plan), 50, 100, options)
        .expect("midpoint should interpolate");
    assert_eq!(midpoint[0].brightness, Some(50));
    assert_eq!(midpoint[0].emission, EmissionState::Emitting);
    assert_eq!(
        logical_brightness(&plan.start),
        Some(0),
        "dark endpoints interpolate from zero visibility"
    );
    assert!(matches!(
        logical_appearance(&Endpoint {
            brightness: None,
            emission: EmissionState::Dark,
            ..plan.start.clone()
        }),
        Ok(Effect::Static { .. })
    ));

    let terminal = interpolate_plans(&[plan], 100, 100, options)
        .expect("terminal step should use the exact endpoint");
    assert_eq!(terminal[0].brightness, Some(100));
    assert_eq!(terminal[0].emission, EmissionState::Emitting);
}

#[test]
fn transition_failure_and_partial_error_preserve_sorted_unique_targets() {
    let first = TargetId::device("first");
    let second = TargetId::device("second");
    let error = partial_error(
        DaemonError::PluginUnavailable {
            plugin: "test".to_owned(),
            device: "lamp".to_owned(),
            diagnostic: "hardware failed".to_owned(),
        },
        vec![second.clone(), first.clone(), second.clone()],
    );
    assert!(matches!(
        error,
        DaemonError::PartialMutation {
            applied_targets,
            ..
        } if applied_targets == vec![first.clone(), second.clone()]
    ));
    let (diagnostic, applied) = transition_failure(DaemonError::PluginUnavailable {
        plugin: "test".to_owned(),
        device: "lamp".to_owned(),
        diagnostic: "failed".to_owned(),
    });
    assert_eq!(
        diagnostic,
        "plugin test cannot currently reach lamp: failed"
    );
    assert!(applied.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transition_runner_completes_and_publishes_terminal_state() {
    let (_state, manager, mutations, runtime_dir) =
        request_test_context("transition-run-completes");
    let id = TransitionId::new("completed");
    let entry = mutations
        .transitions
        .insert(transition_status(id.as_str(), Vec::new(), 1), None);
    let options =
        TransitionOptions::new(Duration::from_millis(1), None).expect("valid transition options");

    run_transition(mutations.clone(), manager, entry, Vec::new(), options).await;

    assert_eq!(
        mutations
            .transitions
            .get(&id)
            .expect("terminal status retained")
            .snapshot()
            .outcome,
        Some(TransitionOutcome::Completed)
    );
    assert!(mutations.transitions.active().is_empty());
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replacement_cancels_a_running_transition_and_retains_its_reason() {
    let (_state, manager, mutations, runtime_dir) = request_test_context("transition-run-replaced");
    let id = TransitionId::new("replaced");
    let entry = mutations
        .transitions
        .insert(transition_status(id.as_str(), Vec::new(), 100), None);
    entry.request_abort(TransitionCancellation::Replaced);
    let options =
        TransitionOptions::new(Duration::from_millis(100), None).expect("valid transition options");

    run_transition(mutations.clone(), manager, entry, Vec::new(), options).await;

    assert_eq!(
        mutations
            .transitions
            .get(&id)
            .expect("cancelled status retained")
            .snapshot()
            .outcome,
        Some(TransitionOutcome::Cancelled(
            TransitionCancellation::Replaced
        ))
    );
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn renewable_transition_expires_when_its_authorization_is_not_renewed() {
    let (_state, manager, mutations, runtime_dir) = request_test_context("transition-run-expired");
    let id = TransitionId::new("expired");
    let entry = mutations
        .transitions
        .insert(transition_status(id.as_str(), Vec::new(), 100), Some(1));
    let options =
        TransitionOptions::new(Duration::from_millis(100), None).expect("valid transition options");

    run_transition(mutations.clone(), manager, entry, Vec::new(), options).await;

    assert_eq!(
        mutations
            .transitions
            .get(&id)
            .expect("expired status retained")
            .snapshot()
            .outcome,
        Some(TransitionOutcome::Cancelled(
            TransitionCancellation::AuthorizationExpired
        ))
    );
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardware_failure_ends_the_transition_without_claiming_unapplied_targets() {
    let (_state, manager, mutations, runtime_dir) = request_test_context("transition-run-failure");
    let target = TargetId::device("request-device");
    let id = TransitionId::new("failed");
    let entry = mutations.transitions.insert(
        transition_status(id.as_str(), vec![target.clone()], 1),
        None,
    );
    let endpoint = Endpoint {
        target: target.clone(),
        appearance: Effect::Static {
            colour: Colour::rgb(Rgb::new(20, 30, 40)),
        },
        brightness: Some(50),
        emission: EmissionState::Emitting,
    };
    let plans = vec![TargetPlan {
        start: endpoint.clone(),
        end: endpoint,
        hue_maximum: u32::MAX,
    }];
    let options =
        TransitionOptions::new(Duration::from_millis(1), None).expect("valid transition options");

    run_transition(mutations.clone(), manager, entry, plans, options).await;

    assert!(matches!(
        mutations
            .transitions
            .get(&id)
            .expect("failed status retained")
            .snapshot()
            .outcome,
        Some(TransitionOutcome::Failed {
            applied_targets,
            ..
        }) if applied_targets.is_empty()
    ));
    remove_request_test_dir(&runtime_dir);
}
