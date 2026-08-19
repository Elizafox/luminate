// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "Safety is documented before each C-boundary assertion; Clippy cannot associate comments outside assertion macro expansions."
)]

use super::*;
use luminate_core::device::{DeviceCategory, DeviceId};
use std::ffi::CString;

use crate::ffi::FfiClient;
use crate::ffi_typed::scenes::*;
use crate::ffi_typed::transitions::*;
use luminate_core::transition::TransitionCancellation;

fn bytes(view: LuminateStringView) -> Option<&'static [u8]> {
    if view.data.is_null() {
        None
    } else {
        // SAFETY: the fixture owning each view remains alive during the assertion.
        Some(unsafe { std::slice::from_raw_parts(view.data.cast(), view.len) })
    }
}

fn device() -> Device {
    Device {
        id: DeviceId::new("keyboard"),
        name: "Desk Keyboard".to_owned(),
        vendor: Some("Luminate".to_owned()),
        model: None,
        provider_instance: Some("demo".to_owned()),
        surfaces: Vec::new(),
        groups: Vec::new(),
        capabilities: CapabilitySet::default(),
        category: Some(DeviceCategory::new("keyboard")),
        physical_tags: vec!["shape:keyboard".to_owned()],
        host_attached: false,
        notes: vec!["USB attached".to_owned()],
        warnings: vec!["Very bright".to_owned()],
    }
}

fn view_bytes(view: LuminateStringView) -> Option<&'static [u8]> {
    if view.data.is_null() {
        None
    } else {
        // SAFETY: every view inspected by these tests borrows from a live fixture.
        Some(unsafe { std::slice::from_raw_parts(view.data.cast(), view.len) })
    }
}

#[test]
fn topology_snapshot_accessors_cover_contents_bounds_and_null() {
    let snapshot = LuminateTopologySnapshot(vec![device()]);
    let snapshot = ptr::from_ref(&snapshot);

    // SAFETY: `snapshot` and its nested device remain live for all calls.
    unsafe {
        assert_eq!(luminate_topology_snapshot_device_count(snapshot), 1);
        let borrowed = luminate_topology_snapshot_device_at(snapshot, 0);
        assert!(!borrowed.is_null());
        assert_eq!(
            view_bytes(luminate_device_id(borrowed)),
            Some(&b"keyboard"[..])
        );
        assert_eq!(
            view_bytes(luminate_device_name(borrowed)),
            Some(&b"Desk Keyboard"[..])
        );
        assert_eq!(
            luminate_topology_snapshot_device_at(snapshot, 1),
            ptr::null()
        );
        assert_eq!(luminate_topology_snapshot_device_count(ptr::null()), 0);
        assert_eq!(
            luminate_topology_snapshot_device_at(ptr::null(), 0),
            ptr::null()
        );
    }
}

#[test]
fn device_snapshot_and_metadata_accessors_preserve_optional_views() {
    let snapshot = LuminateDeviceSnapshot(device());
    let snapshot = ptr::from_ref(&snapshot);

    // SAFETY: `snapshot` and its nested device remain live for all calls.
    unsafe {
        let borrowed = luminate_device_snapshot_device(snapshot);
        assert!(!borrowed.is_null());
        assert_eq!(
            view_bytes(luminate_device_vendor(borrowed)),
            Some(&b"Luminate"[..])
        );
        assert_eq!(view_bytes(luminate_device_model(borrowed)), None);
        assert_eq!(
            view_bytes(luminate_device_provider_instance(borrowed)),
            Some(b"demo".as_slice())
        );
        assert_eq!(
            view_bytes(luminate_device_category(borrowed)),
            Some(&b"keyboard"[..])
        );
        assert!(!luminate_device_host_attached(borrowed));
        assert_eq!(view_bytes(luminate_device_vendor(ptr::null())), None);
        assert!(!luminate_device_host_attached(ptr::null()));
        assert_eq!(luminate_device_snapshot_device(ptr::null()), ptr::null());
    }
}

#[test]
fn nested_string_vectors_cover_values_bounds_and_null() {
    let fixture = device();
    let borrowed = ptr::from_ref(&fixture).cast::<LuminateDevice>();
    let mut empty_fixture = device();
    empty_fixture.physical_tags.clear();
    let empty = ptr::from_ref(&empty_fixture).cast::<LuminateDevice>();

    // SAFETY: `borrowed` points to the live native device expected by the opaque API.
    unsafe {
        assert_eq!(models::luminate_device_physical_tag_count(borrowed), 1);
        assert_eq!(
            view_bytes(models::luminate_device_physical_tag_at(borrowed, 0)),
            Some(&b"shape:keyboard"[..])
        );
        assert_eq!(
            view_bytes(models::luminate_device_physical_tag_at(borrowed, 1)),
            None
        );
        assert_eq!(models::luminate_device_physical_tag_count(empty), 0);
        assert_eq!(
            view_bytes(models::luminate_device_physical_tag_at(empty, 0)),
            None
        );
        assert_eq!(models::luminate_device_note_count(borrowed), 1);
        assert_eq!(
            view_bytes(models::luminate_device_note_at(borrowed, 0)),
            Some(&b"USB attached"[..])
        );
        assert_eq!(
            view_bytes(models::luminate_device_note_at(borrowed, 1)),
            None
        );
        assert_eq!(models::luminate_device_warning_count(borrowed), 1);
        assert_eq!(
            view_bytes(models::luminate_device_warning_at(borrowed, 0)),
            Some(&b"Very bright"[..])
        );
        assert_eq!(models::luminate_device_note_count(ptr::null()), 0);
        assert_eq!(models::luminate_device_physical_tag_count(ptr::null()), 0);
        assert_eq!(
            view_bytes(models::luminate_device_physical_tag_at(ptr::null(), 0)),
            None
        );
        assert_eq!(
            view_bytes(models::luminate_device_note_at(ptr::null(), 0)),
            None
        );
    }
}

#[test]
fn state_snapshot_accessor_covers_live_and_null_roots() {
    let snapshot = LuminateStateSnapshot(DeviceStateStatus {
        device: DeviceId::new("keyboard"),
        observations: Vec::new(),
        reachability: Reachability::Reachable,
        reconciliation: ReconciliationStatus::Complete,
        adoption: Vec::new(),
        latest_error: None,
        latest_attempt_ms: None,
    });

    // SAFETY: the snapshot remains live and null is an explicitly supported sentinel.
    unsafe {
        assert!(!luminate_state_snapshot_state(ptr::from_ref(&snapshot)).is_null());
        assert_eq!(luminate_state_snapshot_state(ptr::null()), ptr::null());
    }
}

#[test]
fn owned_root_free_functions_accept_owned_values_and_null() {
    let topology = Box::into_raw(Box::new(LuminateTopologySnapshot(vec![device()])));
    let device = Box::into_raw(Box::new(LuminateDeviceSnapshot(device())));
    let event = Box::into_raw(Box::new(LuminateEvent(Event::TopologyChanged {
        devices: vec![DeviceId::new("keyboard")],
    })));

    // SAFETY: each non-null pointer is uniquely owned and came from `Box::into_raw`.
    unsafe {
        luminate_topology_snapshot_free(topology);
        luminate_device_snapshot_free(device);
        luminate_event_free(event);
        luminate_topology_snapshot_free(ptr::null_mut());
        luminate_device_snapshot_free(ptr::null_mut());
        luminate_state_snapshot_free(ptr::null_mut());
        luminate_event_free(ptr::null_mut());
        luminate_effect_free(ptr::null_mut());
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "This accessor test intentionally audits the complete frozen and dynamic scene shape."
)]
fn scene_accessors_cover_frozen_dynamic_optional_and_boundary_values() {
    let effect = Effect::Static {
        colour: Colour::rgb(Rgb::new(1, 2, 3)),
    };
    let state = SceneTargetState {
        appearance: Some(effect.clone()),
        brightness: Some(42),
        emission: Some(EmissionState::Emitting),
        appearance_slots: None,
    };
    let frozen = SceneBinding::Frozen {
        target: TargetId::device("desk"),
        state: state.clone(),
    };
    let dynamic = SceneBinding::DynamicCollectionMember {
        collection: CollectionId::new("lights"),
        target: TargetId::surface("desk", "front"),
        state,
    };
    let scene = Scene {
        id: SceneId::new("scene-1"),
        revision: 7,
        name: "Evening".to_owned(),
        description: Some("Warm light".to_owned()),
        owner: OwnerIdentity::Uid(1000),
        bindings: vec![frozen, dynamic],
    };
    let snapshot = LuminateSceneSnapshot(scene);
    let snapshot_ptr = ptr::from_ref(&snapshot);
    let scene_ptr = unsafe { luminate_scene_snapshot_scene(snapshot_ptr) };

    unsafe {
        assert_eq!(luminate_scene_list_count(ptr::null()), 0);
        assert_eq!(luminate_scene_list_at(ptr::null(), 0), ptr::null());
        assert_eq!(luminate_scene_snapshot_scene(ptr::null()), ptr::null());
        assert_eq!(bytes(luminate_scene_id(scene_ptr)), Some(&b"scene-1"[..]));
        assert_eq!(bytes(luminate_scene_name(scene_ptr)), Some(&b"Evening"[..]));
        assert_eq!(luminate_scene_revision(scene_ptr), 7);
        assert_eq!(
            bytes(luminate_scene_description(scene_ptr)),
            Some(&b"Warm light"[..])
        );
        assert_eq!(luminate_scene_binding_count(scene_ptr), 2);
        assert_eq!(luminate_scene_binding_at(scene_ptr, 2), ptr::null());

        let first = luminate_scene_binding_at(scene_ptr, 0);
        let second = luminate_scene_binding_at(scene_ptr, 1);
        assert!(!first.is_null());
        assert!(!second.is_null());
        assert_eq!(luminate_scene_binding_collection_id(first).len, 0);
        assert_eq!(
            luminate_scene_binding_collection_id(first).data,
            ptr::null()
        );
        assert_eq!(
            bytes(luminate_scene_binding_collection_id(second)),
            Some(&b"lights"[..])
        );
        assert!(!luminate_scene_binding_target(first).is_null());
        assert!(!luminate_scene_binding_target(second).is_null());
        assert!(!luminate_scene_binding_appearance(first).is_null());
        assert!(luminate_scene_binding_has_brightness(first));
        assert_eq!(luminate_scene_binding_brightness(first), 42);
        assert!(luminate_scene_binding_has_emission(first));
        assert_eq!(luminate_scene_binding_emission(first), 1);
        let owner = luminate_scene_owner(scene_ptr);
        assert_eq!(
            luminate_owner_identity_kind(owner),
            LuminateOwnerKind::LuminateOwnerKindUid
        );
        let mut uid = 0;
        assert!(luminate_owner_identity_uid(owner, &raw mut uid));
        assert_eq!(uid, 1000);
        assert_eq!(luminate_owner_identity_sid(owner).data, ptr::null());

        assert_eq!(luminate_scene_revision(ptr::null()), 0);
        assert!(luminate_scene_owner(ptr::null()).is_null());
    }

    let sid_scene = LuminateSceneSnapshot(Scene {
        id: SceneId::new("scene-2"),
        revision: 1,
        name: "SID".to_owned(),
        description: None,
        owner: OwnerIdentity::Sid("S-1-5-21".to_owned()),
        bindings: vec![SceneBinding::Frozen {
            target: TargetId::group("desk", "all"),
            state: SceneTargetState {
                appearance: None,
                brightness: Some(1),
                emission: None,
                appearance_slots: None,
            },
        }],
    });
    unsafe {
        let view = luminate_scene_snapshot_scene(ptr::from_ref(&sid_scene));
        let owner = luminate_scene_owner(view);
        assert_eq!(
            luminate_owner_identity_kind(owner),
            LuminateOwnerKind::LuminateOwnerKindSid
        );
        assert_eq!(
            bytes(luminate_owner_identity_sid(owner)),
            Some(&b"S-1-5-21"[..])
        );
        let binding = luminate_scene_binding_at(view, 0);
        assert!(luminate_scene_binding_has_brightness(binding));
        assert_eq!(luminate_scene_binding_brightness(binding), 1);
        assert!(!luminate_scene_binding_has_emission(binding));
        assert_eq!(luminate_scene_binding_emission(binding), u32::MAX);
        assert_eq!(luminate_scene_binding_appearance(binding), ptr::null());
    }
}

#[test]
fn transition_accessors_cover_each_terminal_outcome_and_bounds() {
    let target = TargetId::element("desk", "front", "key");
    let statuses = [
        TransitionStatus {
            id: TransitionId::new("active"),
            targets: vec![target.clone()],
            elapsed_ms: 5,
            duration_ms: 100,
            outcome: None,
        },
        TransitionStatus {
            id: TransitionId::new("completed"),
            targets: vec![],
            elapsed_ms: 100,
            duration_ms: 100,
            outcome: Some(TransitionOutcome::Completed),
        },
        TransitionStatus {
            id: TransitionId::new("cancelled"),
            targets: vec![],
            elapsed_ms: 12,
            duration_ms: 100,
            outcome: Some(TransitionOutcome::Cancelled(
                TransitionCancellation::AuthorizationExpired,
            )),
        },
        TransitionStatus {
            id: TransitionId::new("failed"),
            targets: vec![],
            elapsed_ms: 20,
            duration_ms: 100,
            outcome: Some(TransitionOutcome::Failed {
                diagnostic: "write failed".to_owned(),
                applied_targets: vec![target],
            }),
        },
    ];

    unsafe {
        assert_eq!(luminate_transition_status_kind(ptr::null()), u32::MAX);
        assert_eq!(luminate_transition_target_count(ptr::null()), 0);
        assert_eq!(luminate_transition_target_at(ptr::null(), 0), ptr::null());
        assert_eq!(
            luminate_transition_failure_diagnostic(ptr::null()).data,
            ptr::null()
        );
    }

    let snapshots: Vec<_> = statuses
        .into_iter()
        .map(LuminateTransitionSnapshot)
        .collect();
    unsafe {
        let active = ptr::from_ref(&snapshots[0]);
        assert_eq!(bytes(luminate_transition_id(active)), Some(&b"active"[..]));
        assert_eq!(luminate_transition_elapsed_ms(active), 5);
        assert_eq!(luminate_transition_duration_ms(active), 100);
        assert_eq!(luminate_transition_status_kind(active), 0);
        assert_eq!(luminate_transition_cancellation_reason(active), u32::MAX);
        assert_eq!(luminate_transition_failed_target_count(active), 0);
        assert_eq!(luminate_transition_target_count(active), 1);
        assert!(!luminate_transition_target_at(active, 0).is_null());
        assert_eq!(luminate_transition_target_at(active, 1), ptr::null());

        let completed = ptr::from_ref(&snapshots[1]);
        assert_eq!(luminate_transition_status_kind(completed), 1);
        assert_eq!(luminate_transition_cancellation_reason(completed), u32::MAX);
        assert_eq!(
            luminate_transition_failure_diagnostic(completed).data,
            ptr::null()
        );

        let cancelled = ptr::from_ref(&snapshots[2]);
        assert_eq!(luminate_transition_status_kind(cancelled), 2);
        assert_eq!(luminate_transition_cancellation_reason(cancelled), 3);

        let failed = ptr::from_ref(&snapshots[3]);
        assert_eq!(luminate_transition_status_kind(failed), 3);
        assert_eq!(
            bytes(luminate_transition_failure_diagnostic(failed)),
            Some(&b"write failed"[..])
        );
        assert_eq!(luminate_transition_failed_target_count(failed), 1);
        assert!(!luminate_transition_failed_target_at(failed, 0).is_null());
        assert_eq!(luminate_transition_failed_target_at(failed, 1), ptr::null());
        luminate_transition_snapshot_free(Box::into_raw(Box::new(LuminateTransitionSnapshot(
            snapshots[0].0.clone(),
        ))));
        luminate_transition_snapshot_free(ptr::null_mut());
    }
}

#[test]
fn abi_version_matches_the_public_constant() {
    assert_eq!(luminate_c_abi_version(), crate::LUMINATE_C_ABI_VERSION);
}

#[test]
#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "The test covers the validation contract of independent typed C operations."
)]
fn daemon_backed_operations_validate_inputs_before_dispatch() {
    let id = CString::new("keyboard").expect("valid device id");
    let mut client = FfiClient::stopped();
    let client = ptr::from_mut(&mut client).cast::<LuminateClient>();
    let mut topology = ptr::null_mut();
    let mut withdrawn = ptr::null_mut();
    let mut device = ptr::null_mut();
    let mut state = ptr::null_mut();
    let mut subscription = ptr::null_mut();

    // SAFETY: each non-null output points to live writable storage. Null inputs exercise
    // documented validation paths before any daemon dispatch.
    unsafe {
        assert_eq!(
            luminate_client_list_devices(ptr::null_mut(), &raw mut topology),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_list_devices(client, ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_list_withdrawn_devices(ptr::null_mut(), &raw mut withdrawn),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_list_withdrawn_devices(client, ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_get_device(client, ptr::null(), &raw mut device),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_get_device(client, id.as_ptr(), ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_get_state(client, ptr::null(), &raw mut state),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_get_state(client, id.as_ptr(), ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_subscribe_with_baseline(client, &raw mut subscription, ptr::null_mut(),),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_subscribe_with_baseline_path(
                client,
                ptr::null(),
                &raw mut subscription,
                &raw mut topology,
            ),
            LuminateStatus::NullPointer
        );
    }
}
