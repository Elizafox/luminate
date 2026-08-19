// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::super::tests_support::{empty_plugin_manager, request_test_descriptor};
use super::*;

use luminate_platform::secure_storage::ensure_private_directory;
use luminate_platform::test_support::TestDir;

#[test]
fn resolve_operation_colour_emulates_cct_for_replayed_restore_operations() {
    use luminate_core::colour::Colour;
    use luminate_core::target::TargetId;

    let state = DaemonState::from_descriptors(&[request_test_descriptor()])
        .expect("build resolve-colour test state");
    let target = TargetId::device("request-device");

    let cct_operation = luminate_plugin_api::PluginUpdateOperation::SetEffect {
        effect: Effect::Static {
            colour: Colour::cct(2000),
        },
    };
    let resolved = resolve_operation_colour(&state, &target, cct_operation);
    match resolved {
        luminate_plugin_api::PluginUpdateOperation::SetEffect {
            effect: Effect::Static { colour },
        } => {
            assert_eq!(
                colour,
                state
                    .resolve_colour(&target, &Colour::cct(2000))
                    .expect("device advertises additive RGB, so CCT should emulate"),
                "restored CCT colour should be emulated to the device's native encoding"
            );
            assert_ne!(
                colour,
                Colour::cct(2000),
                "emulated colour must no longer be the raw unsupported CCT encoding"
            );
        }
        other @ (luminate_plugin_api::PluginUpdateOperation::SetEffect { .. }
        | luminate_plugin_api::PluginUpdateOperation::SetAppearanceSlots { .. }
        | luminate_plugin_api::PluginUpdateOperation::SetBrightness { .. }
        | luminate_plugin_api::PluginUpdateOperation::Clear
        | luminate_plugin_api::PluginUpdateOperation::SaveCurrent) => {
            panic!("expected a resolved Static effect operation, got {other:?}")
        }
    }

    let effect_operation = luminate_plugin_api::PluginUpdateOperation::SetEffect {
        effect: Effect::Off,
    };
    assert!(matches!(
        resolve_operation_colour(&state, &target, effect_operation),
        luminate_plugin_api::PluginUpdateOperation::SetEffect {
            effect: Effect::Off
        }
    ));
}

#[test]
fn reconciliation_without_a_host_covers_leave_adopt_restore_and_read_failure() {
    use luminate_core::capability::{ReadableFacet, ReadbackFidelity, StateReadbackCapability};
    use luminate_core::state::StateFacetKind;
    use luminate_core::target::TargetId;

    let manager = empty_plugin_manager();
    let device = device::DeviceId::new("request-device");

    let state = Arc::new(Mutex::new(
        DaemonState::from_descriptors(&[request_test_descriptor()]).expect("build state"),
    ));
    assert!(
        reconcile_device(&state, &manager, &device, ReconciliationPolicy::Leave)
            .expect("leave without readback is a no-op")
            .is_empty()
    );
    let error = reconcile_device(&state, &manager, &device, ReconciliationPolicy::Adopt)
        .expect_err("adopt requires exact readback");
    assert!(error.to_string().contains("no exact readback"));

    let mut restore_descriptor = request_test_descriptor();
    restore_descriptor.capabilities.persistence = PersistenceCapability::None;
    let restore_state = Arc::new(Mutex::new(
        DaemonState::from_descriptors(&[restore_descriptor]).expect("build restore state"),
    ));
    restore_state
        .blocking_lock()
        .set_brightness(TargetId::device("request-device"), 25)
        .expect("cache brightness");
    let error = reconcile_device(
        &restore_state,
        &manager,
        &device,
        ReconciliationPolicy::Restore,
    )
    .expect_err("restore without an owning plugin should fail");
    assert!(error.to_string().contains("restore operations failed"));

    let mut readable_descriptor = request_test_descriptor();
    readable_descriptor.capabilities.state_readback = StateReadbackCapability::Readable {
        facets: vec![ReadableFacet {
            facet: StateFacetKind::Brightness,
            fidelity: ReadbackFidelity::Exact,
        }],
        read_disturbs_output: false,
        notifies_external_changes: false,
    };
    let readable_state = Arc::new(Mutex::new(
        DaemonState::from_descriptors(&[readable_descriptor]).expect("build readable state"),
    ));
    let error = reconcile_device(
        &readable_state,
        &manager,
        &device,
        ReconciliationPolicy::Leave,
    )
    .expect_err("readback without an owning plugin should fail");
    assert!(error.to_string().contains("no plugin owns device"));
}

#[tokio::test]
async fn startup_reconciliation_runs_empty_and_failed_device_paths() {
    let manager = empty_plugin_manager();
    let runtime_dir = TestDir::new("startup-reconciliation");
    ensure_private_directory(&runtime_dir).expect("create runtime directory");
    let state_path: Arc<Path> = Arc::from(runtime_dir.join("state.json").into_boxed_path());

    let empty = Arc::new(Mutex::new(DaemonState::default()));
    run_startup_reconciliation(&empty, &manager, &state_path, &DaemonConfig::default())
        .await
        .expect("empty reconciliation task");

    let state = Arc::new(Mutex::new(
        DaemonState::from_descriptors(&[request_test_descriptor()]).expect("build state"),
    ));
    let config = DaemonConfig {
        reconciliation_policy: Some(ReconciliationPolicy::Adopt),
        ..DaemonConfig::default()
    };
    run_startup_reconciliation(&state, &manager, &state_path, &config)
        .await
        .expect("per-device reconciliation failure is non-fatal");
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the replay matrix is clearer as one scenario with one shared state"
)]
fn cached_state_replay_maps_facets_states_filters_and_persistence() {
    use luminate_core::colour::Colour;
    use luminate_core::effect::Effect;
    use luminate_core::rgb::Rgb;
    use luminate_core::state::{
        AdoptedFacet, AppearanceState, EmissionState, FacetValue, PhysicalPowerState,
    };
    use luminate_core::target::TargetId;
    use luminate_plugin_api::PluginUpdateOperation;

    let mut descriptors = Vec::new();
    for id in ["colour", "brightness", "effect", "clear"] {
        let mut descriptor = request_test_descriptor();
        descriptor.id = id.to_owned();
        descriptor.name = id.to_owned();
        descriptor.capabilities.persistence = PersistenceCapability::None;
        descriptors.push(descriptor);
    }
    let mut state = DaemonState::from_descriptors(&descriptors).expect("build replay state");
    let colour = Colour::rgb(Rgb::new(3, 4, 5));
    state
        .set_static_colour_for_test(TargetId::device("colour"), colour.clone())
        .expect("cache colour");
    state
        .set_brightness(TargetId::device("brightness"), 42)
        .expect("cache brightness");
    state
        .set_effect(TargetId::device("effect"), Effect::Off)
        .expect("cache effect");
    state
        .clear_target(&TargetId::device("clear"))
        .expect("cache clear");
    state.commit_adopted_baseline(&[
        AdoptedFacet {
            target: TargetId::device("colour"),
            value: FacetValue::Appearance(AppearanceState::Static(colour.clone())),
            confirmed_at_ms: 1,
        },
        AdoptedFacet {
            target: TargetId::device("effect"),
            value: FacetValue::Appearance(AppearanceState::Effect(Effect::Off)),
            confirmed_at_ms: 2,
        },
        AdoptedFacet {
            target: TargetId::device("brightness"),
            value: FacetValue::Brightness(7),
            confirmed_at_ms: 3,
        },
        AdoptedFacet {
            target: TargetId::device("effect"),
            value: FacetValue::Emission(EmissionState::Dark),
            confirmed_at_ms: 4,
        },
        AdoptedFacet {
            target: TargetId::device("effect"),
            value: FacetValue::Emission(EmissionState::Emitting),
            confirmed_at_ms: 5,
        },
        AdoptedFacet {
            target: TargetId::device("effect"),
            value: FacetValue::PhysicalPower(PhysicalPowerState::Off),
            confirmed_at_ms: 6,
        },
        AdoptedFacet {
            target: TargetId::device("effect"),
            value: FacetValue::PhysicalPower(PhysicalPowerState::On),
            confirmed_at_ms: 7,
        },
        AdoptedFacet {
            target: TargetId::device("withdrawn"),
            value: FacetValue::Brightness(99),
            confirmed_at_ms: 8,
        },
    ]);

    let all = pending_cached_state(&state, None);
    assert!(all.iter().any(|(_, operation)| matches!(
        operation,
        PluginUpdateOperation::SetEffect {
            effect: Effect::Static { .. }
        }
    )));
    assert!(
        all.iter()
            .any(|(_, operation)| matches!(operation, PluginUpdateOperation::SetBrightness { .. }))
    );
    assert!(
        all.iter()
            .any(|(_, operation)| matches!(operation, PluginUpdateOperation::SetEffect { .. }))
    );
    assert!(
        all.iter()
            .any(|(_, operation)| matches!(operation, PluginUpdateOperation::Clear))
    );
    assert!(
        !all.iter()
            .any(|(target, _)| target.device_id().as_str() == "withdrawn")
    );

    let colour_id = device::DeviceId::new("colour");
    let selected = HashSet::from([&colour_id]);
    assert!(
        pending_cached_state(&state, Some(&selected))
            .iter()
            .all(|(target, _)| target.device_id() == &colour_id)
    );

    let required = DaemonState::from_descriptors(&[request_test_descriptor()])
        .expect("build required-persistence state");
    let target = TargetId::device("request-device");
    assert_eq!(
        persistence_requirement_for_target(&required, &target),
        Some(PersistenceRequirement::Required)
    );
    assert_eq!(
        persistence_requirement_for_target(&required, &TargetId::device("missing")),
        None
    );
}

#[test]
fn write_through_cached_state_carries_what_pending_cached_state_skips() {
    use luminate_core::colour::Colour;
    use luminate_core::rgb::Rgb;
    use luminate_core::target::TargetId;
    use luminate_plugin_api::PluginUpdateOperation;

    let mut state = DaemonState::from_descriptors(&[request_test_descriptor()])
        .expect("build required-persistence state");
    let target = TargetId::device("request-device");
    let colour = Colour::rgb(Rgb::new(9, 8, 7));
    state
        .set_static_colour_for_test(target.clone(), colour.clone())
        .expect("cache colour on write-through target");

    let device_id = device::DeviceId::new("request-device");
    let selected = HashSet::from([&device_id]);

    // A daemon restart's replay pass must not re-write hardware that
    // already durably holds this value...
    assert!(
        pending_cached_state(&state, Some(&selected)).is_empty(),
        "required-persistence target must not be replayed to hardware"
    );

    // ...but restore still needs to know the value, or `GetState` reads
    // back as unknown until the target is next changed even though the
    // physical device is showing the correct persisted colour.
    let write_through = write_through_cached_state(&state, &selected);
    assert_eq!(write_through.len(), 1);
    assert_eq!(write_through[0].0, target);
    assert!(matches!(
        &write_through[0].1,
        PluginUpdateOperation::SetEffect { effect: Effect::Static { colour: applied } } if *applied == colour
    ));
}

#[test]
fn target_mapping_helpers_cover_every_variant() {
    use luminate_core::appearance_slot::{AppearanceSlotId, AppearanceSlotValue};
    use luminate_core::colour::Colour;
    use luminate_core::effect::Effect;
    use luminate_core::rgb::Rgb;
    use luminate_core::target::TargetId;
    use luminate_plugin_api::PluginUpdateOperation;

    for target in [
        TargetId::device("device"),
        TargetId::surface("device", "surface"),
        TargetId::element("device", "surface", "element"),
        TargetId::group("device", "group"),
    ] {
        assert_eq!(target.device_id().as_str(), "device");
    }

    let colour = Colour::rgb(Rgb::new(1, 2, 3));
    let states = [
        TargetState::Effect(Effect::Off),
        TargetState::Effect(Effect::Static {
            colour: colour.clone(),
        }),
        TargetState::Brightness(9),
        TargetState::AppearanceSlots(vec![AppearanceSlotValue {
            slot: AppearanceSlotId::new("ac"),
            effect: Effect::Off,
        }]),
        TargetState::Clear,
    ];
    assert!(matches!(
        operation_for_target_state(&states[0]),
        PluginUpdateOperation::SetEffect { .. }
    ));
    assert!(matches!(
        operation_for_target_state(&states[1]),
        PluginUpdateOperation::SetEffect {
            effect: Effect::Static { .. }
        }
    ));
    assert!(matches!(
        operation_for_target_state(&states[2]),
        PluginUpdateOperation::SetBrightness { value: 9 }
    ));
    assert!(matches!(
        operation_for_target_state(&states[3]),
        PluginUpdateOperation::SetAppearanceSlots { .. }
    ));
    assert!(matches!(
        operation_for_target_state(&states[4]),
        PluginUpdateOperation::Clear
    ));
}

#[test]
fn restart_replay_keeps_each_surfaces_slots_in_one_grouped_operation() {
    use luminate_core::appearance_slot::{
        AppearanceCapability, AppearanceSlotDescriptor, AppearanceSlotId,
        AppearanceSlotUpdatePolicy, AppearanceSlotValue, AppearanceSlotsCapability,
    };
    use luminate_core::capability::{
        CapabilitySet, CctEmulation, ColourCapability, PersistenceCapability,
    };
    use luminate_core::surface::SurfaceKind;
    use luminate_core::target::TargetId;
    use luminate_plugin_api::SurfaceDescriptor;

    let mut descriptor = request_test_descriptor();
    descriptor.surfaces.push(SurfaceDescriptor {
        id: "power-button".to_owned(),
        name: "Power Button".to_owned(),
        kind: SurfaceKind::Opaque,
        physical_tags: Vec::new(),
        elements: Vec::new(),
        capabilities: CapabilitySet::default(),
        notes: Vec::new(),
        warnings: Vec::new(),
    });
    descriptor.surfaces[0].capabilities.appearance_slots = Some(AppearanceSlotsCapability {
        slots: ["ac", "battery"]
            .into_iter()
            .map(|id| AppearanceSlotDescriptor {
                id: AppearanceSlotId::new(id),
                name: id.to_owned(),
                appearance: AppearanceCapability {
                    colour: vec![ColourCapability::rgb8()],
                    cct_emulation: CctEmulation::Disabled,
                    hardware_effects: None,
                },
                persistence: PersistenceCapability::None,
                notes: Vec::new(),
                warnings: Vec::new(),
            })
            .collect(),
        update_policy: AppearanceSlotUpdatePolicy::CompleteSet,
    });
    let target = TargetId::surface(&descriptor.id, &descriptor.surfaces[0].id);
    let mut state = DaemonState::from_descriptors(&[descriptor]).expect("build slotted state");
    state
        .set_appearance_slots(
            target.clone(),
            vec![
                AppearanceSlotValue {
                    slot: AppearanceSlotId::new("ac"),
                    effect: Effect::Off,
                },
                AppearanceSlotValue {
                    slot: AppearanceSlotId::new("battery"),
                    effect: Effect::Off,
                },
            ],
        )
        .expect("store grouped slot state");

    let pending = pending_cached_state(&state, None);
    assert!(matches!(
        pending.as_slice(),
        [(replayed_target, luminate_plugin_api::PluginUpdateOperation::SetAppearanceSlots { values })]
            if replayed_target == &target && values.len() == 2
    ));
}

#[test]
fn reconciliation_policy_uses_documented_precedence() {
    use ReconciliationPolicy::{Adopt, Leave, Restore};

    assert_eq!(
        select_reconciliation_policy(Some(Restore), Some(Leave), Some(Adopt), Some(Leave)),
        Restore
    );
    assert_eq!(
        select_reconciliation_policy(None, Some(Restore), Some(Leave), None),
        Restore
    );
    assert_eq!(
        select_reconciliation_policy(None, None, Some(Restore), Some(Adopt)),
        Restore
    );
    assert_eq!(
        select_reconciliation_policy(None, None, None, Some(Restore)),
        Restore
    );
    assert_eq!(select_reconciliation_policy(None, None, None, None), Leave);
}
