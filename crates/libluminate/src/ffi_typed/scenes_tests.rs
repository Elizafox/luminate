// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use super::*;
use crate::ffi::{luminate_client_connect_path, luminate_client_free};
use crate::ffi_typed::test_support::FakeDaemon;
use crate::ffi_typed::transitions::*;
use luminate_protocol::framing::{receive, send};
use luminate_protocol::{
    ErrorCode, OperationError, Request, RequestMessage, Response, ResponseMessage, ResponseStatus,
};
use std::ffi::CString;

fn fixture_scene() -> Scene {
    Scene {
        id: SceneId::new("scene"),
        revision: 1,
        name: "Scene".to_owned(),
        description: Some("description".to_owned()),
        owner: OwnerIdentity::Uid(1000),
        bindings: vec![SceneBinding::Frozen {
            target: TargetId::device("desk"),
            state: SceneTargetState {
                appearance: None,
                brightness: Some(50),
                emission: Some(EmissionState::Dark),
                appearance_slots: None,
            },
        }],
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "The test follows one seeded builder through payload preservation, validation, mutation, and lifetime boundaries."
)]
fn seeded_scene_builder_preserves_and_edits_independent_bindings() {
    let mut scene = fixture_scene();
    scene.bindings.push(SceneBinding::DynamicCollectionMember {
        collection: CollectionId::new("room"),
        target: TargetId::surface("lamp", "shade"),
        state: SceneTargetState {
            appearance: Some(Effect::Static {
                colour: Colour::cct(3_500),
            }),
            brightness: Some(75),
            emission: Some(EmissionState::Emitting),
            appearance_slots: Some(vec![
                AppearanceSlotValue {
                    slot: AppearanceSlotId::new("ac"),
                    effect: Effect::Static {
                        colour: Colour::rgb(Rgb::new(1, 2, 3)),
                    },
                },
                AppearanceSlotValue {
                    slot: AppearanceSlotId::new("battery"),
                    effect: Effect::Rainbow { period_ms: 900 },
                },
            ]),
        },
    });
    let expected = scene.clone();
    let mut builder = ptr::null_mut();

    // SAFETY: the scene view and builder output remain live.
    unsafe {
        assert_eq!(
            luminate_scene_builder_from_scene(cast_ref(&scene), ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_scene_builder_from_scene(cast_ref(&scene), &raw mut builder),
            LuminateStatus::Ok
        );
    }
    drop(scene);
    assert_eq!(unsafe { luminate_scene_builder_binding_count(builder) }, 2);
    // SAFETY: the builder remains live and owns the borrowed binding.
    let second = unsafe { luminate_scene_builder_binding_at(builder, 1) };
    assert_eq!(native_ref!(second, SceneBinding), expected.bindings.get(1));

    let renamed = CString::new("Renamed").expect("valid string");
    let device = CString::new("new-device").expect("valid string");
    let replacement = LuminateSceneBindingInput {
        dynamic_collection_id: ptr::null(),
        target: LuminateTarget {
            device_id: device.as_ptr(),
            surface_id: ptr::null(),
            element_id: ptr::null(),
            group_id: ptr::null(),
        },
        state: LuminateSceneTargetStateInput {
            appearance: ptr::null(),
            has_brightness: true,
            brightness: 25,
            has_emission: false,
            emission: 0,
            appearance_slots: ptr::null(),
            appearance_slot_count: 0,
        },
    };
    let malformed_name = [0xff_u8, 0];
    let invalid_emission = LuminateSceneBindingInput {
        dynamic_collection_id: ptr::null(),
        target: LuminateTarget {
            device_id: device.as_ptr(),
            surface_id: ptr::null(),
            element_id: ptr::null(),
            group_id: ptr::null(),
        },
        state: LuminateSceneTargetStateInput {
            appearance: ptr::null(),
            has_brightness: true,
            brightness: 25,
            has_emission: true,
            emission: u32::MAX,
            appearance_slots: ptr::null(),
            appearance_slot_count: 0,
        },
    };
    // SAFETY: all builder inputs remain live for their calls.
    unsafe {
        assert_eq!(
            luminate_scene_builder_set_name(builder, renamed.as_ptr()),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_scene_builder_set_name(builder, malformed_name.as_ptr().cast()),
            LuminateStatus::InvalidUtf8
        );
        assert_eq!((*builder).name, "Renamed");
        assert_eq!(
            luminate_scene_builder_set_description(builder, ptr::null()),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_scene_builder_replace_binding(builder, 0, &raw const replacement),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_scene_builder_replace_binding(builder, 0, &raw const invalid_emission),
            LuminateStatus::InvalidArgument
        );
        assert_eq!(
            (*builder)
                .bindings
                .first()
                .expect("replacement binding remains present")
                .state()
                .brightness,
            Some(25)
        );
        assert_eq!(
            luminate_scene_builder_remove_binding(builder, 1),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_scene_builder_add_binding(builder, &raw const replacement),
            LuminateStatus::Ok
        );
        assert_eq!(luminate_scene_builder_binding_count(builder), 2);
        assert_eq!((*builder).name, "Renamed");
        assert_eq!((*builder).description, None);
        assert_eq!((*builder).id, expected.id);
        assert_eq!((*builder).expected_revision, expected.revision);
        assert_eq!(
            luminate_scene_builder_remove_binding(builder, 2),
            LuminateStatus::InvalidArgument
        );
        luminate_scene_builder_free(builder);
    }
}

async fn serve_scene_requests(mut daemon: FakeDaemon, expected: usize) {
    let mut stream = daemon.accept("scene-test-daemon").await;

    let scene = fixture_scene();
    let target = TargetId::device("desk");
    let transition = TransitionStatus {
        id: TransitionId::new("transition"),
        targets: vec![target.clone()],
        elapsed_ms: 0,
        duration_ms: 100,
        outcome: Some(TransitionOutcome::Completed),
    };
    for _ in 0..expected {
        let message: RequestMessage = receive(&mut stream).await.expect("receive request");
        let status = match message.request {
            Request::CreateScene { .. }
            | Request::CaptureScene { .. }
            | Request::ReplaceScene { .. }
            | Request::RecaptureScene { .. } => ResponseStatus::Scene(Box::new(scene.clone())),
            Request::DeleteScene { .. } => ResponseStatus::Ack,
            Request::ListScenes => ResponseStatus::Scenes(vec![scene.clone()]),
            Request::GetScene { .. } => ResponseStatus::SceneInfo(Some(Box::new(scene.clone()))),
            Request::ApplyScene { .. } => ResponseStatus::SceneApplied {
                applied: vec![target.clone()],
                denied: Vec::new(),
            },
            Request::StartTransition(_)
            | Request::GetTransition { .. }
            | Request::AbortTransition { .. }
            | Request::RenewTransition { .. } => {
                ResponseStatus::Transition(Box::new(transition.clone()))
            }
            other => panic!("unexpected scene request: {other:?}"),
        };
        send(
            &mut stream,
            &ResponseMessage {
                id: message.id,
                response: Response { status },
            },
        )
        .await
        .expect("send scene response");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn builder_replacement_submits_complete_payload_and_preserves_revision_conflicts() {
    let mut daemon = FakeDaemon::bind("scene-builder-conflict");
    let path = daemon.c_path();
    let expected = fixture_scene();
    let expected_request = expected.clone();
    let server = tokio::spawn(async move {
        let mut stream = daemon.accept("scene-builder-conflict-daemon").await;
        let message: RequestMessage = receive(&mut stream).await.expect("receive replacement");
        match message.request {
            Request::ReplaceScene {
                id,
                expected_revision,
                name,
                description,
                bindings,
            } => {
                assert_eq!(id, expected_request.id);
                assert_eq!(expected_revision, expected_request.revision);
                assert_eq!(name, expected_request.name);
                assert_eq!(description, expected_request.description);
                assert_eq!(bindings, expected_request.bindings);
            }
            other => panic!("unexpected request: {other:?}"),
        }
        send(
            &mut stream,
            &ResponseMessage {
                id: message.id,
                response: Response {
                    status: ResponseStatus::Error(OperationError {
                        code: ErrorCode::Conflict,
                        message: "scene revision conflict".to_owned(),
                        retry_after_ms: None,
                        applied_targets: Vec::new(),
                    }),
                },
            },
        )
        .await
        .expect("send revision conflict");
    });

    let mut client = ptr::null_mut();
    let mut builder = ptr::null_mut();
    let mut output = ptr::null_mut();
    // SAFETY: every input and output pointer remains live for its call.
    unsafe {
        assert_eq!(
            luminate_client_connect_path(path.as_ptr(), &raw mut client),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_scene_builder_from_scene(cast_ref(&expected), &raw mut builder),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_replace_scene_from_builder(client, builder, &raw mut output),
            LuminateStatus::Conflict
        );
        assert!(output.is_null());
        assert_eq!(luminate_scene_builder_binding_count(builder), 1);
        luminate_scene_builder_free(builder);
        luminate_client_free(client);
    }
    server.await.expect("server task");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::too_many_lines,
    reason = "This fixture exercises the complete ordinary scene and transition lifecycle."
)]
async fn ordinary_client_scene_and_transition_operations_round_trip() {
    let daemon = FakeDaemon::bind("scene-ffi");
    let path_c = daemon.c_path();
    let server = tokio::spawn(serve_scene_requests(daemon, 16));
    let mut client = ptr::null_mut();
    assert_eq!(
        unsafe { luminate_client_connect_path(path_c.as_ptr(), &raw mut client) },
        LuminateStatus::Ok
    );

    let name = CString::new("Scene").expect("valid name");
    let description = CString::new("description").expect("valid description");
    let device = CString::new("desk").expect("valid device");
    let binding = LuminateSceneBindingInput {
        dynamic_collection_id: ptr::null(),
        target: LuminateTarget {
            device_id: device.as_ptr(),
            surface_id: ptr::null(),
            element_id: ptr::null(),
            group_id: ptr::null(),
        },
        state: LuminateSceneTargetStateInput {
            appearance: ptr::null(),
            has_brightness: true,
            brightness: 50,
            has_emission: true,
            emission: 0,
            appearance_slots: ptr::null(),
            appearance_slot_count: 0,
        },
    };
    let id = CString::new("scene").expect("valid scene id");
    let mut snapshot = ptr::null_mut();
    unsafe {
        assert_eq!(
            luminate_client_create_scene(
                client,
                name.as_ptr(),
                description.as_ptr(),
                &raw const binding,
                1,
                &raw mut snapshot,
            ),
            LuminateStatus::Ok
        );
        let mut builder = ptr::null_mut();
        assert_eq!(
            luminate_scene_builder_from_scene(
                luminate_scene_snapshot_scene(snapshot),
                &raw mut builder,
            ),
            LuminateStatus::Ok
        );
        let mut replacement = ptr::null_mut();
        assert_eq!(
            luminate_client_replace_scene_from_builder(client, builder, &raw mut replacement),
            LuminateStatus::Ok
        );
        luminate_scene_snapshot_free(replacement);
        luminate_scene_builder_free(builder);
        luminate_scene_snapshot_free(snapshot);
        snapshot = ptr::null_mut();
        assert_eq!(
            luminate_client_capture_scene(
                client,
                name.as_ptr(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                0,
                &raw mut snapshot,
            ),
            LuminateStatus::Ok
        );
        luminate_scene_snapshot_free(snapshot);
        snapshot = ptr::null_mut();
        assert_eq!(
            luminate_client_replace_scene(
                client,
                id.as_ptr(),
                1,
                name.as_ptr(),
                ptr::null(),
                &raw const binding,
                1,
                &raw mut snapshot,
            ),
            LuminateStatus::Ok
        );
        luminate_scene_snapshot_free(snapshot);
        snapshot = ptr::null_mut();
        assert_eq!(
            luminate_client_recapture_scene(
                client,
                id.as_ptr(),
                1,
                ptr::null(),
                ptr::null(),
                0,
                &raw mut snapshot,
            ),
            LuminateStatus::Ok
        );
        luminate_scene_snapshot_free(snapshot);
        assert_eq!(
            luminate_client_delete_scene(client, id.as_ptr(), 1),
            LuminateStatus::Ok
        );
        let mut list = ptr::null_mut();
        assert_eq!(
            luminate_client_list_scenes(client, &raw mut list),
            LuminateStatus::Ok
        );
        assert_eq!(luminate_scene_list_count(list), 1);
        luminate_scene_list_free(list);
        assert_eq!(
            luminate_client_get_scene(client, id.as_ptr(), &raw mut snapshot),
            LuminateStatus::Ok
        );
        luminate_scene_snapshot_free(snapshot);
        assert_eq!(
            luminate_client_apply_scene(client, id.as_ptr()),
            LuminateStatus::Ok
        );

        let timing = LuminateTransitionOptions {
            duration_ms: 100,
            step_interval_ms: 0,
            function: 0,
            colour_interpolation: 0,
            hue_direction: 0,
        };
        let mut transition = ptr::null_mut();
        let transition_id = CString::new("transition").expect("valid transition id");
        assert_eq!(
            luminate_client_transition_current_to_scene(
                client,
                id.as_ptr(),
                timing,
                &raw mut transition,
            ),
            LuminateStatus::Ok
        );
        luminate_transition_snapshot_free(transition);
        transition = ptr::null_mut();
        assert_eq!(
            luminate_client_transition_scene_to_scene(
                client,
                id.as_ptr(),
                id.as_ptr(),
                timing,
                &raw mut transition,
            ),
            LuminateStatus::Ok
        );
        luminate_transition_snapshot_free(transition);
        transition = ptr::null_mut();

        let target_state = LuminateTransitionTargetStateInput {
            target: LuminateTarget {
                device_id: device.as_ptr(),
                surface_id: ptr::null(),
                element_id: ptr::null(),
                group_id: ptr::null(),
            },
            state: LuminateSceneTargetStateInput {
                appearance: ptr::null(),
                has_brightness: true,
                brightness: 50,
                has_emission: true,
                emission: 0,
                appearance_slots: ptr::null(),
                appearance_slot_count: 0,
            },
        };
        assert_eq!(
            luminate_client_transition_scene_to_states(
                client,
                id.as_ptr(),
                &raw const target_state,
                1,
                timing,
                &raw mut transition,
            ),
            LuminateStatus::Ok
        );
        luminate_transition_snapshot_free(transition);
        transition = ptr::null_mut();
        assert_eq!(
            luminate_client_transition_current_to_states(
                client,
                &raw const target_state,
                1,
                timing,
                &raw mut transition,
            ),
            LuminateStatus::Ok
        );
        luminate_transition_snapshot_free(transition);
        let mut transition_snapshot = ptr::null_mut();
        assert_eq!(
            luminate_client_transition_get(
                client,
                transition_id.as_ptr(),
                &raw mut transition_snapshot,
            ),
            LuminateStatus::Ok
        );
        luminate_transition_snapshot_free(transition_snapshot);
        assert_eq!(
            luminate_client_transition_wait(
                client,
                transition_id.as_ptr(),
                &raw mut transition_snapshot,
            ),
            LuminateStatus::Ok
        );
        luminate_transition_snapshot_free(transition_snapshot);
        transition_snapshot = ptr::null_mut();
        let _ = luminate_client_transition_abort(
            client,
            transition_id.as_ptr(),
            &raw mut transition_snapshot,
        );
        luminate_transition_snapshot_free(transition_snapshot);
    }
    unsafe { luminate_client_free(client) };
    server.await.expect("scene fixture should finish");
}
