// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use luminate_core::capability::{
    BrightnessCapability, CapabilityScope, ColourCapability, PhysicalPowerCapability,
    PowerDomainRef, ReadableFacet, ReadbackFidelity, StateReadbackCapability,
};
use luminate_core::collection::{CollectionMember, OwnerIdentity};
use luminate_core::colour::Colour;
use luminate_core::device::DeviceId;
use luminate_core::effect::Effect;
use luminate_core::rgb::Rgb;
use luminate_core::state::{
    AdoptedFacet, AdoptionStatus, AppearanceState, EffectiveAppearanceState, EmissionState,
    FacetValue, ObservationConfidence, ObservationSource, PhysicalPowerState, Reachability,
    ReconciliationStatus, StateFacetKind,
};
use luminate_core::target::TargetId;
use luminate_plugin_api::{
    PluginFacetObservation, PluginReadError, PluginStateSnapshot, PluginTarget,
    PluginUpdateOperation,
};

use crate::error::DaemonError;
use crate::state::DaemonState;

use super::super::tests_support::*;
use super::TargetState;

fn reconciliation_state(fidelity: ReadbackFidelity) -> DaemonState {
    let mut descriptor = demo_device_descriptor();
    descriptor.capabilities.brightness = BrightnessCapability::Independent {
        bits: 8,
        maximum: 255,
        scope: CapabilityScope::Device,
    };
    descriptor.capabilities.emission = true;
    descriptor.capabilities.physical_power = Some(PhysicalPowerCapability {
        scope: CapabilityScope::Device,
    });
    descriptor.capabilities.power_domain = Some(PowerDomainRef::Device);
    descriptor.capabilities.state_readback = StateReadbackCapability::Readable {
        facets: [
            StateFacetKind::Appearance,
            StateFacetKind::Brightness,
            StateFacetKind::Emission,
            StateFacetKind::PhysicalPower,
        ]
        .into_iter()
        .map(|facet| ReadableFacet { facet, fidelity })
        .collect(),
        read_disturbs_output: false,
        notifies_external_changes: false,
    };
    DaemonState::from_descriptors(&[descriptor]).expect("reconciliation state should build")
}

fn plugin_device() -> PluginTarget {
    PluginTarget::Device {
        device: "demo-kbd".to_owned(),
    }
}

fn heterogeneous_demo_state() -> DaemonState {
    let mut descriptor = demo_device_descriptor();
    descriptor.surfaces[0].elements[1].capabilities.colour = vec![ColourCapability::rgb8()];
    let mut state = DaemonState::from_descriptors(&[descriptor]).expect("state");
    state
        .set_static_colour_for_test(
            TargetId::element("demo-kbd", "main", "logo"),
            Colour::rgb(Rgb::new(1, 2, 3)),
        )
        .expect("left colour");
    state
        .set_static_colour_for_test(
            TargetId::element("demo-kbd", "main", "topology-only"),
            Colour::rgb(Rgb::new(4, 5, 6)),
        )
        .expect("right colour");
    state
}

fn observation(value: FacetValue) -> PluginFacetObservation {
    PluginFacetObservation {
        target: plugin_device(),
        value,
    }
}

#[test]
fn exact_snapshot_is_confirmed_and_adoption_stays_out_of_desired_state() {
    let mut state = reconciliation_state(ReadbackFidelity::Exact);
    let device = DeviceId::new("demo-kbd");
    let generation = state.generation(&device);
    let adopted = state
        .accept_snapshot(
            &device,
            generation,
            PluginStateSnapshot {
                observations: vec![observation(FacetValue::Brightness(42))],
                errors: Vec::new(),
            },
            true,
        )
        .expect("snapshot should validate");

    assert_eq!(adopted.len(), 1);
    assert!(state.target_states().is_empty());
    let status = state.device_state_status(&device).expect("status");
    assert_eq!(
        status.observations[0].confidence,
        ObservationConfidence::Confirmed
    );
    assert!(status.adoption.iter().any(|(_, facet, adoption)| {
        *facet == StateFacetKind::Brightness && *adoption == AdoptionStatus::Pending
    }));
}

#[test]
fn best_effort_snapshot_is_visible_but_ineligible_for_adoption() {
    let mut state = reconciliation_state(ReadbackFidelity::BestEffort);
    let device = DeviceId::new("demo-kbd");
    let adopted = state
        .accept_snapshot(
            &device,
            state.generation(&device),
            PluginStateSnapshot {
                observations: vec![observation(FacetValue::Brightness(7))],
                errors: Vec::new(),
            },
            true,
        )
        .expect("best-effort snapshot should validate");
    assert!(adopted.is_empty());
    let status = state.device_state_status(&device).expect("status");
    assert_eq!(
        status.observations[0].confidence,
        ObservationConfidence::BestEffort
    );
    assert_eq!(status.observations[0].source, ObservationSource::Readback);
    assert!(
        status
            .adoption
            .iter()
            .any(|(_, _, adoption)| { *adoption == AdoptionStatus::IneligibleFidelity })
    );
}

#[test]
fn malformed_foreign_or_group_snapshot_is_rejected_atomically() {
    let mut state = reconciliation_state(ReadbackFidelity::Exact);
    let device = DeviceId::new("demo-kbd");
    for target in [
        PluginTarget::Device {
            device: "foreign".to_owned(),
        },
        PluginTarget::Group {
            device: "demo-kbd".to_owned(),
            group: "all".to_owned(),
        },
    ] {
        let error = state
            .accept_snapshot(
                &device,
                state.generation(&device),
                PluginStateSnapshot {
                    observations: vec![PluginFacetObservation {
                        target,
                        value: FacetValue::Brightness(1),
                    }],
                    errors: Vec::new(),
                },
                true,
            )
            .expect_err("malformed target must fail");
        assert!(matches!(error, DaemonError::Internal(_)));
        assert!(
            state
                .device_state_status(&device)
                .expect("status")
                .observations
                .is_empty()
        );
        assert!(state.adopted_baseline().is_empty());
        assert!(state.target_states().is_empty());
    }
}

#[test]
fn successful_group_write_projects_observations_to_canonical_targets() {
    let mut state = demo_state();
    state
        .set_static_colour_for_test(
            TargetId::group("demo-kbd", "all"),
            Colour::rgb(Rgb::new(1, 2, 3)),
        )
        .expect("group colour");
    let observations = state
        .device_state_status(&DeviceId::new("demo-kbd"))
        .expect("status")
        .observations;
    assert!(observations.iter().any(|observation| {
        matches!(observation.target, TargetId::Group { .. })
            && observation.value
                == FacetValue::Appearance(AppearanceState::Static(Colour::rgb(Rgb::new(1, 2, 3))))
    }));
    assert!(
        observations
            .iter()
            .any(|observation| matches!(observation.target, TargetId::Surface { .. }))
    );
    assert!(
        observations
            .iter()
            .any(|observation| matches!(observation.target, TargetId::Element { .. }))
    );
}

#[test]
fn heterogeneous_elements_percolate_mixed_appearance_to_every_aggregate() {
    let mut descriptor = demo_device_descriptor();
    descriptor.surfaces[0].elements[1].capabilities.colour = vec![ColourCapability::rgb8()];
    let mut state = DaemonState::from_descriptors(&[descriptor]).expect("state");
    state
        .set_static_colour_for_test(
            TargetId::element("demo-kbd", "main", "logo"),
            Colour::rgb(Rgb::new(1, 2, 3)),
        )
        .expect("first colour");
    state
        .set_static_colour_for_test(
            TargetId::element("demo-kbd", "main", "topology-only"),
            Colour::rgb(Rgb::new(4, 5, 6)),
        )
        .expect("second colour");

    let status = state
        .device_state_status(&DeviceId::new("demo-kbd"))
        .expect("status");
    for target in [
        TargetId::surface("demo-kbd", "main"),
        TargetId::group("demo-kbd", "all"),
        TargetId::device("demo-kbd"),
    ] {
        assert_eq!(
            status
                .observation(&target, StateFacetKind::Appearance)
                .map(|observation| &observation.value),
            Some(&FacetValue::Appearance(AppearanceState::Mixed))
        );
        assert_eq!(
            status
                .observation(&target, StateFacetKind::EffectiveAppearance)
                .map(|observation| &observation.value),
            Some(&FacetValue::EffectiveAppearance(
                EffectiveAppearanceState::Mixed
            ))
        );
    }
}

#[test]
fn successful_surface_and_device_writes_propagate_to_elements() {
    let mut state = demo_state();
    let surface_colour = Colour::rgb(Rgb::new(7, 8, 9));
    state
        .set_static_colour_for_test(
            TargetId::surface("demo-kbd", "main"),
            surface_colour.clone(),
        )
        .expect("surface colour");
    let status = state
        .device_state_status(&DeviceId::new("demo-kbd"))
        .expect("surface status");
    for element in ["logo", "topology-only"] {
        assert_eq!(
            status
                .observation(
                    &TargetId::element("demo-kbd", "main", element),
                    StateFacetKind::Appearance
                )
                .map(|observation| &observation.value),
            Some(&FacetValue::Appearance(AppearanceState::Static(
                surface_colour.clone()
            )))
        );
    }

    let device_colour = Colour::rgb(Rgb::new(10, 11, 12));
    state
        .set_static_colour_for_test(TargetId::device("demo-kbd"), device_colour.clone())
        .expect("device colour");
    let status = state
        .device_state_status(&DeviceId::new("demo-kbd"))
        .expect("device status");
    for element in ["logo", "topology-only"] {
        assert_eq!(
            status
                .observation(
                    &TargetId::element("demo-kbd", "main", element),
                    StateFacetKind::Appearance
                )
                .map(|observation| &observation.value),
            Some(&FacetValue::Appearance(AppearanceState::Static(
                device_colour.clone()
            )))
        );
    }

    let group_colour = Colour::rgb(Rgb::new(13, 14, 15));
    state
        .set_static_colour_for_test(TargetId::group("demo-kbd", "all"), group_colour.clone())
        .expect("group colour");
    let status = state
        .device_state_status(&DeviceId::new("demo-kbd"))
        .expect("group status");
    for element in ["logo", "topology-only"] {
        assert_eq!(
            status
                .observation(
                    &TargetId::element("demo-kbd", "main", element),
                    StateFacetKind::Appearance
                )
                .map(|observation| &observation.value),
            Some(&FacetValue::Appearance(AppearanceState::Static(
                group_colour.clone()
            )))
        );
    }
}

#[test]
fn incomplete_constituent_knowledge_does_not_guess_aggregate_homogeneity() {
    let mut descriptor = demo_device_descriptor();
    descriptor.surfaces[0].elements[1].capabilities.colour = vec![ColourCapability::rgb8()];
    let mut state = DaemonState::from_descriptors(&[descriptor]).expect("state");
    state
        .set_static_colour_for_test(
            TargetId::element("demo-kbd", "main", "logo"),
            Colour::rgb(Rgb::new(1, 2, 3)),
        )
        .expect("one known colour");

    let status = state
        .device_state_status(&DeviceId::new("demo-kbd"))
        .expect("status");
    assert!(
        status
            .observation(
                &TargetId::surface("demo-kbd", "main"),
                StateFacetKind::Appearance
            )
            .is_none()
    );
    assert!(
        status
            .observation(&TargetId::device("demo-kbd"), StateFacetKind::Appearance)
            .is_none()
    );
}

#[test]
fn nested_collections_synthesize_mixed_appearance_bottom_up() {
    let mut state = heterogeneous_demo_state();
    let left = state
        .create_collection(
            "Left".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![CollectionMember::Target(TargetId::element(
                "demo-kbd", "main", "logo",
            ))],
        )
        .expect("left collection");
    let right = state
        .create_collection(
            "Right".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![CollectionMember::Target(TargetId::element(
                "demo-kbd",
                "main",
                "topology-only",
            ))],
        )
        .expect("right collection");
    let outer = state
        .create_collection(
            "Outer".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![
                CollectionMember::Collection(left.clone()),
                CollectionMember::Collection(right.clone()),
            ],
        )
        .expect("outer collection");
    let status = state
        .collection_state_status(&outer)
        .expect("collection status");
    assert_eq!(
        status.appearance.map(|observation| observation.value),
        Some(AppearanceState::Mixed)
    );
    assert_eq!(
        status
            .effective_appearance
            .map(|observation| observation.value),
        Some(EffectiveAppearanceState::Mixed)
    );
}

#[test]
fn collections_aggregate_every_concrete_member_kind() {
    let mut state = heterogeneous_demo_state();
    for target in [
        TargetId::device("demo-kbd"),
        TargetId::surface("demo-kbd", "main"),
        TargetId::group("demo-kbd", "all"),
    ] {
        let aggregate = state
            .create_collection(
                format!("{target:?}"),
                None,
                OwnerIdentity::Uid(1000),
                None,
                vec![CollectionMember::Target(target)],
            )
            .expect("aggregate-target collection");
        assert_eq!(
            state
                .collection_state_status(&aggregate)
                .and_then(|status| status.appearance)
                .map(|observation| observation.value),
            Some(AppearanceState::Mixed)
        );
        assert_eq!(
            state
                .collection_state_status(&aggregate)
                .and_then(|status| status.effective_appearance)
                .map(|observation| observation.value),
            Some(EffectiveAppearanceState::Mixed)
        );
    }

    let element = state
        .create_collection(
            "Element".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![CollectionMember::Target(TargetId::element(
                "demo-kbd", "main", "logo",
            ))],
        )
        .expect("element-target collection");
    assert_eq!(
        state
            .collection_state_status(&element)
            .and_then(|status| status.appearance)
            .map(|observation| observation.value),
        Some(AppearanceState::Static(Colour::rgb(Rgb::new(1, 2, 3))))
    );
}

#[test]
fn partial_read_adopts_only_the_confirmed_facet() {
    let mut state = reconciliation_state(ReadbackFidelity::Exact);
    let device = DeviceId::new("demo-kbd");
    let adopted = state
        .accept_snapshot(
            &device,
            0,
            PluginStateSnapshot {
                observations: vec![observation(FacetValue::Brightness(88))],
                errors: Vec::new(),
            },
            true,
        )
        .expect("partial read is valid");
    assert_eq!(adopted.len(), 1);
    assert_eq!(adopted[0].value.kind(), StateFacetKind::Brightness);
    assert!(
        state
            .device_state_status(&device)
            .expect("status")
            .observations
            .iter()
            .all(|facet| facet.value.kind() != StateFacetKind::Appearance)
    );
}

#[test]
fn failed_refresh_retains_prior_value_as_stale() {
    let mut state = reconciliation_state(ReadbackFidelity::Exact);
    let device = DeviceId::new("demo-kbd");
    state
        .accept_snapshot(
            &device,
            0,
            PluginStateSnapshot {
                observations: vec![observation(FacetValue::Brightness(12))],
                errors: Vec::new(),
            },
            false,
        )
        .expect("initial read");
    state.fail_reconciliation(&device, "timeout".to_owned());
    let status = state.device_state_status(&device).expect("status");
    assert_eq!(status.reachability, Reachability::Unavailable);
    assert_eq!(status.observations[0].value, FacetValue::Brightness(12));
    assert!(status.observations[0].stale);
}

#[test]
fn partial_error_keeps_successful_facets_fresh() {
    let mut state = reconciliation_state(ReadbackFidelity::Exact);
    let device = DeviceId::new("demo-kbd");
    state
        .accept_snapshot(
            &device,
            0,
            PluginStateSnapshot {
                observations: vec![observation(FacetValue::Brightness(13))],
                errors: vec![PluginReadError {
                    target: plugin_device(),
                    diagnostic: "appearance unavailable".to_owned(),
                }],
            },
            false,
        )
        .expect("partial snapshot is structurally valid");
    let status = state.device_state_status(&device).expect("status");
    assert_eq!(status.reachability, Reachability::Reachable);
    assert_eq!(status.reconciliation, ReconciliationStatus::Failed);
    assert!(!status.observations[0].stale);
}

#[test]
fn stale_generation_cannot_publish_or_adopt() {
    let mut state = reconciliation_state(ReadbackFidelity::Exact);
    let device = DeviceId::new("demo-kbd");
    let captured = state.generation(&device);
    state.reserve_generation(device.clone());
    assert!(
        state
            .accept_snapshot(
                &device,
                captured,
                PluginStateSnapshot {
                    observations: vec![observation(FacetValue::Brightness(99))],
                    errors: Vec::new(),
                },
                true,
            )
            .is_err()
    );
    assert!(
        state
            .device_state_status(&device)
            .expect("status")
            .observations
            .is_empty()
    );
}

#[test]
fn exact_verification_mismatch_preserves_desired_and_can_be_marked_drifted() {
    let mut state = reconciliation_state(ReadbackFidelity::Exact);
    let device = DeviceId::new("demo-kbd");
    let target = TargetId::device("demo-kbd");
    state
        .set_brightness(target.clone(), 10)
        .expect("desired brightness");
    state
        .accept_snapshot(
            &device,
            0,
            PluginStateSnapshot {
                observations: vec![observation(FacetValue::Brightness(11))],
                errors: Vec::new(),
            },
            false,
        )
        .expect("verification snapshot");
    let operations = vec![(target, PluginUpdateOperation::SetBrightness { value: 10 })];
    assert!(!state.confirmed_observations_match(&operations));
    state.mark_reconciliation_drifted(&device, "mismatch".to_owned());
    assert_eq!(
        state
            .device_state_status(&device)
            .expect("status")
            .reconciliation,
        ReconciliationStatus::Drifted
    );
    assert!(matches!(
        state.target_states()[0].state,
        TargetState::Brightness(10)
    ));
}

#[test]
fn whole_device_off_changes_power_and_emission_without_erasing_other_facets() {
    let mut state = reconciliation_state(ReadbackFidelity::Exact);
    let target = TargetId::device("demo-kbd");
    state
        .set_static_colour_for_test(target.clone(), Colour::rgb(Rgb::new(4, 5, 6)))
        .expect("colour");
    state
        .set_brightness(target.clone(), 70)
        .expect("brightness");
    state.set_effect(target, Effect::Off).expect("off");
    let observations = state
        .device_state_status(&DeviceId::new("demo-kbd"))
        .expect("status")
        .observations;
    assert!(observations.iter().any(|facet| matches!(
        facet.value,
        FacetValue::PhysicalPower(PhysicalPowerState::Off)
    )));
    assert!(
        observations
            .iter()
            .any(|facet| matches!(facet.value, FacetValue::Emission(EmissionState::Dark)))
    );
    assert!(
        observations
            .iter()
            .any(|facet| facet.value.kind() == StateFacetKind::Appearance)
    );
    assert!(
        observations
            .iter()
            .any(|facet| facet.value.kind() == StateFacetKind::Brightness)
    );
}

#[test]
fn element_off_keeps_broader_power_domain_on_without_local_power_facet() {
    let mut descriptor = demo_device_descriptor();
    descriptor.capabilities.physical_power = Some(PhysicalPowerCapability {
        scope: CapabilityScope::Device,
    });
    descriptor.surfaces[0].elements[0].capabilities.emission = true;
    descriptor.surfaces[0].elements[0].capabilities.power_domain = Some(PowerDomainRef::Device);
    let mut state = DaemonState::from_descriptors(&[descriptor]).expect("state");
    let element = TargetId::element("demo-kbd", "main", "logo");
    state.set_effect(element.clone(), Effect::Off).expect("off");
    let observations = state
        .device_state_status(&DeviceId::new("demo-kbd"))
        .expect("status")
        .observations;
    assert!(observations.iter().any(|facet| {
        facet.target == element && facet.value == FacetValue::Emission(EmissionState::Dark)
    }));
    assert!(!observations.iter().any(|facet| {
        facet.target == element && facet.value.kind() == StateFacetKind::PhysicalPower
    }));
    assert!(observations.iter().any(|facet| matches!(
        (&facet.target, &facet.value),
        (
            TargetId::Device(_),
            FacetValue::PhysicalPower(PhysicalPowerState::On)
        )
    )));
}

#[test]
fn zero_brightness_is_dark_while_nonzero_does_not_invent_emission() {
    let mut state = reconciliation_state(ReadbackFidelity::Exact);
    let target = TargetId::device("demo-kbd");
    state
        .set_brightness(target.clone(), 10)
        .expect("brightness");
    assert!(
        !state
            .device_state_status(&DeviceId::new("demo-kbd"))
            .expect("status")
            .observations
            .iter()
            .any(|facet| facet.value.kind() == StateFacetKind::Emission)
    );
    state.set_brightness(target, 0).expect("zero brightness");
    assert!(
        state
            .device_state_status(&DeviceId::new("demo-kbd"))
            .expect("status")
            .observations
            .iter()
            .any(|facet| facet.value == FacetValue::Emission(EmissionState::Dark))
    );
}

#[test]
fn failed_adoption_persistence_keeps_old_baseline_and_marks_degraded() {
    let mut state = reconciliation_state(ReadbackFidelity::Exact);
    let target = TargetId::device("demo-kbd");
    state.commit_adopted_baseline(&[AdoptedFacet {
        target: target.clone(),
        value: FacetValue::Brightness(1),
        confirmed_at_ms: 1,
    }]);
    let candidate = state
        .accept_snapshot(
            &DeviceId::new("demo-kbd"),
            0,
            PluginStateSnapshot {
                observations: vec![observation(FacetValue::Brightness(2))],
                errors: Vec::new(),
            },
            true,
        )
        .expect("confirmed candidate");
    state.stage_adoption(candidate).expect("stage adoption");
    assert!(
        state
            .adopted_baseline_for_persistence()
            .iter()
            .any(|facet| { facet.value == FacetValue::Brightness(2) })
    );
    state.finish_pending_adoption(false);
    assert_eq!(state.adopted_baseline()[0].value, FacetValue::Brightness(1));
    let status = state
        .device_state_status(&DeviceId::new("demo-kbd"))
        .expect("status");
    assert!(
        status
            .adoption
            .iter()
            .any(|(_, _, adoption)| { *adoption == AdoptionStatus::PersistenceFailed })
    );
    assert!(
        status
            .observations
            .iter()
            .all(|observation| !observation.stale)
    );
}

#[test]
fn restored_adopted_baseline_is_stale_last_known_not_current_readback() {
    let mut state = reconciliation_state(ReadbackFidelity::Exact);
    state.restore_adopted_baseline(vec![AdoptedFacet {
        target: TargetId::device("demo-kbd"),
        value: FacetValue::Brightness(33),
        confirmed_at_ms: 123,
    }]);
    let observation = state
        .device_state_status(&DeviceId::new("demo-kbd"))
        .expect("status")
        .observations
        .pop()
        .expect("restored observation");
    assert!(observation.stale);
    assert_eq!(observation.source, ObservationSource::AdoptedBaseline);
    assert_eq!(observation.observed_at_ms, 123);
}
