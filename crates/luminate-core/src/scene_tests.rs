// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

fn state() -> SceneTargetState {
    SceneTargetState {
        appearance: Some(Effect::Spectrum { period_ms: 1000 }),
        brightness: Some(50),
        emission: Some(EmissionState::Emitting),
        appearance_slots: None,
    }
}

#[test]
fn validates_sparse_state_combinations() {
    assert_eq!(
        SceneTargetState {
            appearance: None,
            brightness: None,
            emission: None,
            appearance_slots: None,
        }
        .validate(),
        Err(SceneValidationError::EmptyTargetState)
    );
    assert_eq!(
        SceneTargetState {
            appearance: Some(Effect::Off),
            brightness: None,
            emission: None,
            appearance_slots: None,
        }
        .validate(),
        Err(SceneValidationError::OffAppearance)
    );
    assert_eq!(
        SceneTargetState {
            appearance: None,
            brightness: None,
            emission: Some(EmissionState::Emitting),
            appearance_slots: None,
        }
        .validate(),
        Err(SceneValidationError::EmittingWithoutAppearance)
    );
    assert_eq!(
        SceneTargetState {
            appearance: Some(Effect::Spectrum { period_ms: 1000 }),
            brightness: Some(0),
            emission: Some(EmissionState::Emitting),
            appearance_slots: None,
        }
        .validate(),
        Err(SceneValidationError::EmittingAtZeroBrightness)
    );
    SceneTargetState {
        appearance: Some(Effect::Spectrum { period_ms: 1000 }),
        brightness: Some(0),
        emission: Some(EmissionState::Dark),
        appearance_slots: None,
    }
    .validate()
    .expect("configured-but-dark state is valid");
}

#[test]
fn rejects_duplicate_targets_across_binding_kinds() {
    let target = TargetId::device("lamp");
    let scene = Scene {
        id: SceneId::generate(),
        revision: 1,
        name: "Evening".to_owned(),
        description: None,
        owner: OwnerIdentity::Uid(1000),
        bindings: vec![
            SceneBinding::Frozen {
                target: target.clone(),
                state: state(),
            },
            SceneBinding::DynamicCollectionMember {
                collection: CollectionId::new("room"),
                target: target.clone(),
                state: state(),
            },
        ],
    };

    assert_eq!(
        scene.validate(),
        Err(SceneValidationError::DuplicateTarget(target))
    );
}
