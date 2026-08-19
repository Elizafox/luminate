// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::slice;

use luminate_core::collection::OwnerIdentity;
use luminate_core::colour::Colour;
use luminate_core::effect::Effect;
use luminate_core::rgb::Rgb;
use luminate_core::scene::{SceneBinding, SceneCaptureMode, SceneTargetState};
use luminate_core::state::EmissionState;
use luminate_core::state::{
    AdoptedFacet, AppearanceState, EffectiveAppearanceState, FacetValue, PhysicalPowerState,
};
use luminate_core::target::TargetId;

use crate::error::DaemonError;
use crate::state::tests_support::validation_state;
use crate::state::validation::tests::{appearance_slot_state, slot};

#[test]
fn capture_composes_desired_facets_without_observations() {
    let mut state = validation_state();
    let target = TargetId::device("demo-kbd");
    let appearance = Effect::Static {
        colour: Colour::rgb(Rgb::new(20, 30, 40)),
    };
    state
        .set_effect(target.clone(), appearance.clone())
        .expect("set appearance");
    state
        .set_brightness(target.clone(), 25)
        .expect("set brightness");

    let bindings = state
        .capture_bindings(&SceneCaptureMode::Frozen, slice::from_ref(&target))
        .expect("capture intended state");
    let SceneBinding::Frozen {
        target: captured_target,
        state: captured,
    } = &bindings[0]
    else {
        panic!("expected frozen binding");
    };
    assert_eq!(captured_target, &target);
    assert_eq!(captured.appearance, Some(appearance));
    assert_eq!(captured.brightness, Some(25));
    assert_eq!(captured.emission, Some(EmissionState::Emitting));
}

#[test]
fn capture_preserves_known_slot_values_and_complete_set_safety() {
    use luminate_core::appearance_slot::AppearanceSlotUpdatePolicy;

    let (mut state, target) = appearance_slot_state(AppearanceSlotUpdatePolicy::CompleteSet);
    let values = vec![slot("ac", Effect::Off), slot("battery", Effect::Off)];
    state
        .set_appearance_slots(target.clone(), values.clone())
        .expect("store slot state");

    let bindings = state
        .capture_bindings(&SceneCaptureMode::Frozen, &[target])
        .expect("capture complete slots");
    assert_eq!(bindings[0].state().appearance_slots.as_ref(), Some(&values));
}

#[test]
fn replacement_and_deletion_enforce_revisions() {
    let mut state = validation_state();
    let target = TargetId::device("demo-kbd");
    let bindings = vec![SceneBinding::Frozen {
        target,
        state: SceneTargetState {
            appearance: Some(Effect::Static {
                colour: Colour::rgb(Rgb::new(1, 2, 3)),
            }),
            brightness: None,
            emission: Some(EmissionState::Emitting),
            appearance_slots: None,
        },
    }];
    let scene = state
        .create_scene(
            "One".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            bindings.clone(),
        )
        .expect("create scene");
    assert!(matches!(
        state.replace_scene(&scene.id, 2, "Two".to_owned(), None, bindings.clone()),
        Err(DaemonError::SceneRevisionConflict { .. })
    ));
    let replaced = state
        .replace_scene(&scene.id, 1, "Two".to_owned(), None, bindings)
        .expect("replace scene");
    assert_eq!(replaced.revision, 2);
    assert!(matches!(
        state.delete_scene(&scene.id, 1),
        Err(DaemonError::SceneRevisionConflict { .. })
    ));
    state
        .delete_scene(&scene.id, 2)
        .expect("delete current revision");
}

#[test]
fn dynamic_capture_resolves_members_and_rejects_out_of_scope_targets() {
    use luminate_core::collection::CollectionMember;

    let mut state = validation_state();
    let target = TargetId::device("demo-kbd");
    state
        .set_effect(
            target.clone(),
            Effect::Static {
                colour: Colour::rgb(Rgb::new(4, 5, 6)),
            },
        )
        .expect("set appearance");
    let collection = state
        .create_collection(
            "Lights".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![CollectionMember::Target(target.clone())],
        )
        .expect("create collection");
    let mode = SceneCaptureMode::DynamicCollectionMembers {
        collection: collection.clone(),
    };
    let bindings = state
        .capture_bindings(&mode, &[])
        .expect("capture all collection members");
    assert!(matches!(
        bindings.as_slice(),
        [SceneBinding::DynamicCollectionMember { collection: id, target: captured, .. }]
            if id == &collection && captured == &target
    ));
    let outside = TargetId::device("missing");
    assert!(
        state
            .capture_bindings(&mode, slice::from_ref(&outside))
            .is_err()
    );
    assert!(
        state
            .capture_bindings(&SceneCaptureMode::Frozen, slice::from_ref(&outside))
            .is_err()
    );
}

#[test]
fn capture_includes_adopted_facets_but_ignores_non_scene_facets() {
    let mut state = validation_state();
    let target = TargetId::device("demo-kbd");
    let expected = Effect::Static {
        colour: Colour::rgb(Rgb::new(8, 9, 10)),
    };
    state.restore_adopted_baseline(vec![
        AdoptedFacet {
            target: target.clone(),
            value: FacetValue::Appearance(AppearanceState::Static(Colour::rgb(Rgb::new(1, 2, 3)))),
            confirmed_at_ms: 1,
        },
        AdoptedFacet {
            target: target.clone(),
            value: FacetValue::Appearance(AppearanceState::Effect(expected.clone())),
            confirmed_at_ms: 2,
        },
        AdoptedFacet {
            target: target.clone(),
            value: FacetValue::Brightness(44),
            confirmed_at_ms: 3,
        },
        AdoptedFacet {
            target: target.clone(),
            value: FacetValue::Emission(EmissionState::Emitting),
            confirmed_at_ms: 4,
        },
        AdoptedFacet {
            target: target.clone(),
            value: FacetValue::PhysicalPower(PhysicalPowerState::On),
            confirmed_at_ms: 5,
        },
        AdoptedFacet {
            target,
            value: FacetValue::EffectiveAppearance(EffectiveAppearanceState::Static(Colour::rgb(
                Rgb::new(11, 12, 13),
            ))),
            confirmed_at_ms: 6,
        },
    ]);

    let captured = state
        .capture_target_state(&TargetId::device("demo-kbd"))
        .expect("adopted facets should form a scene endpoint");
    assert_eq!(captured.appearance, Some(expected));
    assert_eq!(captured.brightness, Some(44));
    assert_eq!(captured.emission, Some(EmissionState::Emitting));
}
