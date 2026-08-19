// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Unit tests for protocol request dispatch.

use std::future::Future;
use std::pin::Pin;

use super::super::tests_support::{
    assert_error_response, discarding_rescans, empty_plugin_manager, remove_request_test_dir,
    request_test_context, request_test_descriptor, test_management_read_state, test_policy,
    test_principal, test_windows_principal,
};
use super::super::*;
use super::appearance::restore_appearances;
use super::collections::authorize_collection_owner;
use super::*;
use crate::device_config::ManagementConfig;
use luminate_core::policy::PrincipalId;

#[test]
fn collection_appearance_restoration_preflights_every_leaf() {
    use luminate_core::colour::Colour;
    use luminate_core::rgb::Rgb;

    let first = TargetId::device("first");
    let second = TargetId::device("second");
    let mut first_descriptor = request_test_descriptor();
    first_descriptor.id = "first".to_owned();
    first_descriptor.name = "First".to_owned();
    let mut second_descriptor = request_test_descriptor();
    second_descriptor.id = "second".to_owned();
    second_descriptor.name = "Second".to_owned();
    let mut daemon_state =
        DaemonState::from_descriptors(&[first_descriptor, second_descriptor]).expect("build state");
    daemon_state
        .set_static_colour_for_test(first.clone(), Colour::rgb(Rgb::new(1, 2, 3)))
        .expect("prime first target");
    let state = Arc::new(Mutex::new(daemon_state));

    let error = restore_appearances(
        &state,
        empty_plugin_manager().as_ref(),
        &[first, second.clone()],
    )
    .expect_err("unknown later state must fail before hardware mutation");

    assert!(matches!(
        error,
        DaemonError::UnknownState { target } if target == second
    ));
}

#[tokio::test]
async fn dispatch_covers_mutation_routes_and_public_errors() {
    use luminate_core::colour::Colour;
    use luminate_core::effect::Effect;
    use luminate_core::rgb::Rgb;
    use luminate_core::target::TargetId;
    use luminate_protocol::{SetBrightnessRequest, SetEffectRequest};

    let (state, manager, mutations, runtime_dir) = request_test_context("dispatch-mutations");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let device = device::DeviceId::new("request-device");
    let target = TargetId::Device(device.clone());
    let colour = Colour::rgb(Rgb::new(1, 2, 3));
    state
        .lock()
        .await
        .set_static_colour_for_test(target.clone(), colour.clone())
        .expect("prime known target state");

    let failing_requests = vec![
        Request::RefreshState {
            device: device.clone(),
        },
        Request::PurgeWithdrawnDevice {
            device: device.clone(),
        },
        Request::SetEffect(SetEffectRequest {
            selector: Selector::Target(target.clone()),
            effect: Effect::Off,
            on_unsupported: None,
        }),
        Request::SetBrightness(SetBrightnessRequest {
            target: Selector::Target(target.clone()),
            value: 50,
            on_unsupported: None,
        }),
        Request::SetEffect(SetEffectRequest {
            selector: Selector::Target(target.clone()),
            effect: Effect::Static {
                colour: colour.clone(),
            },
            on_unsupported: None,
        }),
        Request::ClearTarget {
            target: Selector::Target(target.clone()),
        },
    ];
    let principal = test_principal();
    let auth_policy = test_policy();
    let rescans = discarding_rescans();
    let management = test_management_read_state();
    let dispatch = async |request: Request, policy: UnsupportedPolicy| {
        dispatch_request(
            request,
            &DispatchContext {
                state: &state,
                plugin_manager: &manager,
                management: &management,
                mutations: &mutations,
                owned_streams: &owned_streams,
                rescans: &rescans,
                principal: &principal,
                policy: auth_policy.as_ref(),
                access_administration: None,
                prefer_shm: true,
                prefer_client_shm: true,
                default_unsupported_policy: policy,
            },
        )
        .await
    };

    for request in failing_requests {
        let response = dispatch(request, UnsupportedPolicy::Skip).await;
        assert_error_response(&response);
    }

    let save = dispatch(
        Request::SaveCurrent {
            target: Selector::Target(target.clone()),
        },
        UnsupportedPolicy::Skip,
    )
    .await;
    assert!(matches!(save.status, ResponseStatus::Ack));
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn setup_workflow_discovery_distinguishes_unknown_plugins() {
    let (state, manager, mutations, runtime_dir) = request_test_context("dispatch-plugin-setup");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let principal = test_principal();
    let auth_policy = test_policy();
    let rescans = discarding_rescans();
    let management = test_management_read_state();
    let response = dispatch_request(
        Request::ListPluginSetupWorkflows {
            plugin: "does-not-exist".to_owned(),
        },
        &DispatchContext {
            state: &state,
            plugin_manager: &manager,
            management: &management,
            mutations: &mutations,
            owned_streams: &owned_streams,
            rescans: &rescans,
            principal: &principal,
            policy: auth_policy.as_ref(),
            access_administration: None,
            prefer_shm: true,
            prefer_client_shm: true,
            default_unsupported_policy: UnsupportedPolicy::Skip,
        },
    )
    .await;

    assert!(matches!(
        response.status,
        ResponseStatus::Error(OperationError {
            code: ErrorCode::NotFound,
            ..
        })
    ));
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn frame_stream_dispatch_tracks_ownership_and_releases_the_target() {
    use luminate_core::capability::{
        BufferingMode, CapabilityScope, FrameUpdateMode, FrameUploadCapability,
    };
    use luminate_core::frame::{FrameEnvelope, FramePayload};

    let (_unused_state, manager, mutations, runtime_dir) =
        request_test_context("dispatch-frame-stream");
    let mut descriptor = request_test_descriptor();
    descriptor.capabilities.frame_upload = Some(FrameUploadCapability {
        scope: CapabilityScope::Device,
        update_mode: FrameUpdateMode::FullFrameOnly,
        max_rate_hz: None,
        atomic: false,
        buffering: BufferingMode::Immediate,
        shm: None,
    });
    let state = Arc::new(Mutex::new(
        DaemonState::from_descriptors(&[descriptor]).expect("build frame-stream state"),
    ));
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let target = TargetId::device("request-device");
    let principal = test_principal();
    let policy = test_policy();
    let management = test_management_read_state();
    let rescans = discarding_rescans();
    let context = DispatchContext {
        state: &state,
        plugin_manager: &manager,
        management: &management,
        mutations: &mutations,
        owned_streams: &owned_streams,
        rescans: &rescans,
        principal: &principal,
        policy: policy.as_ref(),
        access_administration: None,
        prefer_shm: false,
        prefer_client_shm: false,
        default_unsupported_policy: UnsupportedPolicy::Skip,
    };

    let started = dispatch_request(
        Request::BeginFrameStream {
            target: target.clone(),
        },
        &context,
    )
    .await;
    assert!(matches!(
        started.status,
        ResponseStatus::FrameStreamStarted { generation: 1 }
    ));
    assert_eq!(
        owned_streams.lock().expect("stream ownership lock").len(),
        1
    );

    let duplicate = dispatch_request(
        Request::BeginFrameStream {
            target: target.clone(),
        },
        &context,
    )
    .await;
    assert_error_response(&duplicate);

    let ended = dispatch_request(
        Request::EndFrameStream {
            target: target.clone(),
            generation: 1,
        },
        &context,
    )
    .await;
    assert!(matches!(ended.status, ResponseStatus::Ack));
    assert!(
        owned_streams
            .lock()
            .expect("stream ownership lock")
            .is_empty()
    );

    let uploaded = dispatch_request(
        Request::UploadFrame {
            target: target.clone(),
            envelope: FrameEnvelope {
                generation: 1,
                sequence: 0,
                payload: FramePayload::Full(Vec::new()),
                commit: false,
            },
        },
        &context,
    )
    .await;
    assert!(matches!(uploaded.status, ResponseStatus::Error { .. }));

    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "This dispatch test intentionally exercises the complete revisioned scene lifecycle."
)]
async fn dispatch_covers_scene_authoring_capture_and_revision_routes() {
    use luminate_core::colour::Colour;
    use luminate_core::effect::Effect;
    use luminate_core::rgb::Rgb;
    use luminate_core::scene::{SceneBinding, SceneCaptureMode, SceneTargetState};
    use luminate_core::state::EmissionState;
    use luminate_core::target::TargetId;
    use luminate_protocol::ResponseStatus;

    let (state, manager, mutations, runtime_dir) = request_test_context("dispatch-scenes");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let target = TargetId::device("request-device");
    state
        .lock()
        .await
        .set_static_colour_for_test(target.clone(), Colour::rgb(Rgb::new(1, 2, 3)))
        .expect("prime scene target");
    let binding = SceneBinding::Frozen {
        target: target.clone(),
        state: SceneTargetState {
            appearance: Some(Effect::Static {
                colour: Colour::rgb(Rgb::new(1, 2, 3)),
            }),
            brightness: Some(50),
            emission: Some(EmissionState::Emitting),
            appearance_slots: None,
        },
    };
    let principal = test_principal();
    let policy = test_policy();
    let management = test_management_read_state();
    let rescans = discarding_rescans();
    let context = DispatchContext {
        state: &state,
        plugin_manager: &manager,
        management: &management,
        mutations: &mutations,
        owned_streams: &owned_streams,
        rescans: &rescans,
        principal: &principal,
        policy: policy.as_ref(),
        access_administration: None,
        prefer_shm: true,
        prefer_client_shm: true,
        default_unsupported_policy: UnsupportedPolicy::Skip,
    };
    let dispatch = |request: Request| dispatch_request(request, &context);

    let created_response = dispatch(Request::CreateScene {
        name: "Evening".to_owned(),
        description: Some("A test scene".to_owned()),
        bindings: vec![binding.clone()],
    })
    .await;
    let ResponseStatus::Scene(created) = created_response.status else {
        panic!("scene creation should return a snapshot: {created_response:?}");
    };
    let scene_id = created.id.clone();
    assert_eq!(created.revision, 1);

    let ResponseStatus::Scenes(scenes) = dispatch(Request::ListScenes).await.status else {
        panic!("scene listing should return snapshots");
    };
    assert_eq!(scenes.len(), 1);
    assert_eq!(scenes[0].id, scene_id);

    let ResponseStatus::SceneInfo(Some(found)) = dispatch(Request::GetScene {
        id: scene_id.clone(),
    })
    .await
    .status
    else {
        panic!("scene lookup should return a snapshot");
    };
    assert_eq!(found.id, scene_id);

    let ResponseStatus::Scene(captured) = dispatch(Request::CaptureScene {
        name: "Captured".to_owned(),
        description: None,
        mode: SceneCaptureMode::Frozen,
        targets: vec![target.clone()],
    })
    .await
    .status
    else {
        panic!("scene capture should return a snapshot");
    };
    assert_eq!(captured.name, "Captured");

    let ResponseStatus::Scene(replaced) = dispatch(Request::ReplaceScene {
        id: scene_id.clone(),
        expected_revision: 1,
        name: "Updated".to_owned(),
        description: None,
        bindings: vec![binding],
    })
    .await
    .status
    else {
        panic!("scene replacement should return a snapshot");
    };
    assert_eq!(replaced.revision, 2);
    assert_eq!(replaced.name, "Updated");

    let ResponseStatus::Scene(recaptured) = dispatch(Request::RecaptureScene {
        id: scene_id.clone(),
        expected_revision: 2,
        mode: SceneCaptureMode::Frozen,
        targets: vec![target],
    })
    .await
    .status
    else {
        panic!("scene recapture should return a snapshot");
    };
    assert_eq!(recaptured.revision, 3);

    assert!(matches!(
        dispatch(Request::DeleteScene {
            id: scene_id,
            expected_revision: 3,
        })
        .await
        .status,
        ResponseStatus::Ack
    ));
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn get_management_returns_authorized_snapshot_and_denies_before_reading() {
    let (state, manager, mutations, runtime_dir) = request_test_context("dispatch-get-management");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let managed_path = runtime_dir.join("managed.toml");
    let global = DaemonConfig {
        management: ManagementConfig {
            locked_daemon_settings: vec!["prefer_shm".to_owned()],
            ..ManagementConfig::default()
        },
        ..DaemonConfig::default()
    };
    let management = Arc::new(ManagementReadState {
        effective: RwLock::new(global.clone()),
        global,
        managed_path,
        managed: Mutex::new(managed_config::ManagedConfig {
            revision: 7,
            ..managed_config::ManagedConfig::default()
        }),
    });

    let response = dispatch_request(
        Request::GetManagement,
        &DispatchContext {
            state: &state,
            plugin_manager: &manager,
            management: &management,
            mutations: &mutations,
            owned_streams: &owned_streams,
            rescans: &discarding_rescans(),
            principal: &test_principal(),
            policy: test_policy().as_ref(),
            access_administration: None,
            prefer_shm: true,
            prefer_client_shm: true,
            default_unsupported_policy: UnsupportedPolicy::Skip,
        },
    )
    .await;
    let ResponseStatus::ManagementSnapshot(snapshot) = response.status else {
        panic!("expected management snapshot");
    };
    assert_eq!(snapshot.revision, 7);
    assert_eq!(snapshot.locked_daemon_settings, vec!["prefer_shm"]);
    assert!(snapshot.plugins.is_empty());

    let denied = dispatch_request(
        Request::GetManagement,
        &DispatchContext {
            state: &state,
            plugin_manager: &manager,
            management: &management,
            mutations: &mutations,
            owned_streams: &owned_streams,
            rescans: &discarding_rescans(),
            principal: &test_principal(),
            policy: &DenyAllPolicy,
            access_administration: None,
            prefer_shm: true,
            prefer_client_shm: true,
            default_unsupported_policy: UnsupportedPolicy::Skip,
        },
    )
    .await;
    assert_error_response(&denied);

    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn patch_management_persists_before_reply_and_publishes_redacted_changes() {
    let (state, manager, mutations, runtime_dir) =
        request_test_context("dispatch-patch-management");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let managed_path = runtime_dir.join("managed.toml");
    let global = DaemonConfig::default();
    let management = Arc::new(ManagementReadState {
        effective: RwLock::new(global.clone()),
        global,
        managed_path: managed_path.clone(),
        managed: Mutex::new(managed_config::ManagedConfig::default()),
    });
    let mut events = mutations.events.subscribe();

    let response = dispatch_request(
        Request::PatchManagement {
            patch: luminate_protocol::ManagementPatch {
                expected_revision: 0,
                mutations: Vec::new(),
            },
        },
        &DispatchContext {
            state: &state,
            plugin_manager: &manager,
            management: &management,
            mutations: &mutations,
            owned_streams: &owned_streams,
            rescans: &discarding_rescans(),
            principal: &test_principal(),
            policy: test_policy().as_ref(),
            access_administration: None,
            prefer_shm: true,
            prefer_client_shm: true,
            default_unsupported_policy: UnsupportedPolicy::Skip,
        },
    )
    .await;

    let ResponseStatus::ManagementPatched(changes) = response.status else {
        panic!("expected management patch result");
    };
    assert_eq!(changes.revision, 1);
    assert!(changes.changes.is_empty());
    assert_eq!(
        managed_config::load(&managed_path)
            .expect("load committed managed configuration")
            .revision,
        1
    );
    assert!(matches!(
        events.recv().await.expect("configuration change event").event,
        Event::ConfigurationChanged { changes: event_changes }
            if event_changes == changes
    ));

    let conflict_response = dispatch_request(
        Request::PatchManagement {
            patch: luminate_protocol::ManagementPatch {
                expected_revision: 0,
                mutations: Vec::new(),
            },
        },
        &DispatchContext {
            state: &state,
            plugin_manager: &manager,
            management: &management,
            mutations: &mutations,
            owned_streams: &owned_streams,
            rescans: &discarding_rescans(),
            principal: &test_principal(),
            policy: test_policy().as_ref(),
            access_administration: None,
            prefer_shm: true,
            prefer_client_shm: true,
            default_unsupported_policy: UnsupportedPolicy::Skip,
        },
    )
    .await;
    assert!(matches!(
        conflict_response.status,
        ResponseStatus::Error(OperationError {
            code: ErrorCode::Conflict,
            ..
        })
    ));
    assert_eq!(
        managed_config::load(&managed_path)
            .expect("load managed configuration after conflict")
            .revision,
        1
    );

    fs::remove_file(&managed_path).expect("remove managed configuration");
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn concurrent_management_patches_allow_exactly_one_revision_to_commit() {
    let (state, manager, mutations, runtime_dir) =
        request_test_context("dispatch-concurrent-management");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let managed_path = runtime_dir.join("managed.toml");
    let global = DaemonConfig::default();
    let management = Arc::new(ManagementReadState {
        effective: RwLock::new(global.clone()),
        global,
        managed_path: managed_path.clone(),
        managed: Mutex::new(managed_config::ManagedConfig::default()),
    });
    let rescans = discarding_rescans();
    let principal = test_principal();
    let policy = test_policy();
    let context = DispatchContext {
        state: &state,
        plugin_manager: &manager,
        management: &management,
        mutations: &mutations,
        owned_streams: &owned_streams,
        rescans: &rescans,
        principal: &principal,
        policy: policy.as_ref(),
        access_administration: None,
        prefer_shm: true,
        prefer_client_shm: true,
        default_unsupported_policy: UnsupportedPolicy::Skip,
    };
    let patch = || Request::PatchManagement {
        patch: luminate_protocol::ManagementPatch {
            expected_revision: 0,
            mutations: Vec::new(),
        },
    };

    let (first, second) = tokio::join!(
        dispatch_request(patch(), &context),
        dispatch_request(patch(), &context)
    );
    let statuses = [&first.status, &second.status];

    assert_eq!(
        statuses
            .iter()
            .filter(|status| matches!(status, ResponseStatus::ManagementPatched(changes) if changes.revision == 1))
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| matches!(
                status,
                ResponseStatus::Error(OperationError {
                    code: ErrorCode::Conflict,
                    ..
                })
            ))
            .count(),
        1
    );
    assert_eq!(
        managed_config::load(&managed_path)
            .expect("load concurrently committed managed configuration")
            .revision,
        1
    );
    assert_eq!(management.managed.lock().await.revision, 1);

    fs::remove_file(&managed_path).expect("remove managed configuration");
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn patch_management_applies_effective_daemon_preferences_to_runtime() {
    use luminate_core::colour::Colour;

    let (state, manager, mutations, runtime_dir) =
        request_test_context("dispatch-runtime-preferences");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let managed_path = runtime_dir.join("managed.toml");
    let global = DaemonConfig {
        management: ManagementConfig {
            locked_daemon_settings: vec!["prefer_shm".to_owned()],
            ..ManagementConfig::default()
        },
        ..DaemonConfig::default()
    };
    let management = Arc::new(ManagementReadState {
        effective: RwLock::new(global.clone()),
        global,
        managed_path: managed_path.clone(),
        managed: Mutex::new(managed_config::ManagedConfig::default()),
    });
    let device = device::DeviceId::new("request-device");

    let response = dispatch_request(
        Request::PatchManagement {
            patch: luminate_protocol::ManagementPatch {
                expected_revision: 0,
                mutations: vec![luminate_protocol::ManagementMutation::SetDaemonPreferences(
                    luminate_protocol::DaemonPreferences {
                        default_unsupported_policy: Some(UnsupportedPolicy::Reject),
                        reconciliation_policy: Some(ReconciliationPolicy::Adopt),
                        device_reconciliation: vec![
                            luminate_protocol::DeviceReconciliationPreference {
                                device: device.clone(),
                                policy: ReconciliationPolicy::Leave,
                            },
                        ],
                        cct_emulation: Some(CctEmulation::Disabled),
                        prefer_shm: Some(false),
                        prefer_client_shm: Some(false),
                    },
                )],
            },
        },
        &DispatchContext {
            state: &state,
            plugin_manager: &manager,
            management: &management,
            mutations: &mutations,
            owned_streams: &owned_streams,
            rescans: &discarding_rescans(),
            principal: &test_principal(),
            policy: test_policy().as_ref(),
            access_administration: None,
            prefer_shm: true,
            prefer_client_shm: true,
            default_unsupported_policy: UnsupportedPolicy::Skip,
        },
    )
    .await;

    assert!(matches!(
        response.status,
        ResponseStatus::ManagementPatched(_)
    ));
    let effective = management.effective();
    assert_eq!(
        effective.default_unsupported_policy,
        UnsupportedPolicy::Reject
    );
    assert_eq!(
        effective.reconciliation_policy,
        Some(ReconciliationPolicy::Adopt)
    );
    assert_eq!(
        effective.device_reconciliation.get(&device),
        Some(&ReconciliationPolicy::Leave)
    );
    assert_eq!(effective.cct_emulation, Some(CctEmulation::Disabled));
    assert!(
        effective.prefer_shm,
        "the administrator lock must keep the global value effective"
    );
    assert!(!effective.prefer_client_shm);
    assert!(matches!(
        state
            .lock()
            .await
            .resolve_colour(&TargetId::Device(device), &Colour::cct(4_000),),
        Err(DaemonError::InvalidArgument { .. })
    ));

    fs::remove_file(&managed_path).expect("remove managed configuration");
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn rescan_acks_after_scheduling_and_is_refused_by_policy() {
    let (state, manager, mutations, runtime_dir) = request_test_context("dispatch-rescan");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let (sender, mut requested) = mpsc::unbounded_channel();
    let rescans = RescanRequester::new(sender);

    let response = dispatch_request(
        Request::Rescan,
        &DispatchContext {
            state: &state,
            plugin_manager: &manager,
            management: &test_management_read_state(),
            mutations: &mutations,
            owned_streams: &owned_streams,
            rescans: &rescans,
            principal: &test_principal(),
            policy: test_policy().as_ref(),
            access_administration: None,
            prefer_shm: true,
            prefer_client_shm: true,
            default_unsupported_policy: UnsupportedPolicy::Skip,
        },
    )
    .await;

    assert!(
        matches!(response.status, ResponseStatus::Ack),
        "expected an ack, got {:?}",
        response.status
    );
    assert_eq!(
        requested.try_recv().expect("a rescan should be scheduled"),
        TopologyNotification::Rescan(RescanReason::Operator)
    );

    // A rescan re-enumerates every plugin and reconciles whatever moved,
    // so it must be gated like any other daemon-administration request
    // rather than being an unauthenticated poke.
    let denied = dispatch_request(
        Request::Rescan,
        &DispatchContext {
            state: &state,
            plugin_manager: &manager,
            management: &test_management_read_state(),
            mutations: &mutations,
            owned_streams: &owned_streams,
            rescans: &rescans,
            principal: &test_principal(),
            policy: &DenyAllPolicy,
            access_administration: None,
            prefer_shm: true,
            prefer_client_shm: true,
            default_unsupported_policy: UnsupportedPolicy::Skip,
        },
    )
    .await;

    assert_error_response(&denied);
    assert!(
        requested.try_recv().is_err(),
        "a denied rescan must not be scheduled"
    );

    remove_request_test_dir(&runtime_dir);
}

#[test]
fn rescan_is_classified_as_daemon_administration() {
    assert_eq!(
        operation_for(&Request::Rescan),
        Some(Operation::DaemonAdministration)
    );
}

#[tokio::test]
async fn plugin_administration_reports_unknown_plugins_without_mutating_state() {
    let (state, manager, mutations, runtime_dir) = request_test_context("dispatch-admin-plugin");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let rescans = discarding_rescans();
    let principal = test_principal();
    let policy = test_policy();
    let management = test_management_read_state();
    let context = DispatchContext {
        state: &state,
        plugin_manager: &manager,
        management: &management,
        mutations: &mutations,
        owned_streams: &owned_streams,
        rescans: &rescans,
        principal: &principal,
        policy: policy.as_ref(),
        access_administration: None,
        prefer_shm: true,
        prefer_client_shm: true,
        default_unsupported_policy: UnsupportedPolicy::Skip,
    };

    for request in [
        Request::UnloadPlugin {
            name: "missing".to_owned(),
        },
        Request::ReloadPlugin {
            name: "missing".to_owned(),
        },
    ] {
        let response = dispatch_request(request, &context).await;
        assert_error_response(&response);
    }
    assert!(manager.management_plugins().is_empty());
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn transition_dispatch_rejects_empty_destinations_and_unknown_ids() {
    use std::time::Duration;

    use luminate_core::scene::SceneTargetState;
    use luminate_core::transition::{
        TransitionId, TransitionOptions, TransitionOutcome, TransitionStatus, TransitionTargetState,
    };
    use luminate_protocol::{StartTransitionRequest, TransitionDestination, TransitionSource};

    let (state, manager, mutations, runtime_dir) = request_test_context("dispatch-transitions");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let policy = test_policy();
    let context = DispatchContext {
        state: &state,
        plugin_manager: &manager,
        management: &test_management_read_state(),
        mutations: &mutations,
        owned_streams: &owned_streams,
        rescans: &discarding_rescans(),
        principal: &test_principal(),
        policy: policy.as_ref(),
        access_administration: None,
        prefer_shm: false,
        prefer_client_shm: false,
        default_unsupported_policy: UnsupportedPolicy::Skip,
    };
    let options =
        TransitionOptions::new(Duration::from_millis(100), None).expect("valid transition options");
    let empty = dispatch_request(
        Request::StartTransition(StartTransitionRequest {
            source: TransitionSource::Current,
            destination: TransitionDestination::TargetStates(vec![TransitionTargetState {
                target: TargetId::device("request-device"),
                state: SceneTargetState {
                    appearance: None,
                    brightness: None,
                    emission: None,
                    appearance_slots: None,
                },
            }]),
            options,
            authorized_targets: None,
            renewable_lease_ms: None,
        }),
        &context,
    )
    .await;
    assert_error_response(&empty);

    let missing = TransitionId::new("missing");
    for request in [
        Request::GetTransition {
            id: missing.clone(),
        },
        Request::AbortTransition {
            id: missing.clone(),
        },
        Request::RenewTransition {
            id: missing.clone(),
            lease_ms: 1,
        },
    ] {
        let response = dispatch_request(request, &context).await;
        assert_error_response(&response);
    }
    let zero_lease = dispatch_request(
        Request::RenewTransition {
            id: missing,
            lease_ms: 0,
        },
        &context,
    )
    .await;
    assert_error_response(&zero_lease);

    let active = TransitionStatus {
        id: TransitionId::new("renewable"),
        targets: vec![TargetId::device("request-device")],
        elapsed_ms: 0,
        duration_ms: 1_000,
        outcome: None,
    };
    mutations.transitions.insert(active.clone(), Some(100));
    for request in [
        Request::GetTransition {
            id: active.id.clone(),
        },
        Request::RenewTransition {
            id: active.id.clone(),
            lease_ms: 500,
        },
    ] {
        let response = dispatch_request(request, &context).await;
        assert!(matches!(
            &response.status,
            ResponseStatus::Transition(status) if status.id == active.id
        ));
    }
    mutations
        .transitions
        .finish(&active.id, TransitionOutcome::Completed);

    remove_request_test_dir(&runtime_dir);
}

struct DenyAllPolicy;

impl AuthorizationPolicy for DenyAllPolicy {
    fn authorize<'a>(
        &'a self,
        _principal: &'a Principal,
        _operation: Operation,
        _resources: &'a [Resource],
    ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        Box::pin(async {
            Decision::Deny {
                reason: Some("deliberate test denial".to_owned()),
            }
        })
    }

    fn name(&self) -> &'static str {
        "deny-all-test-policy"
    }
}

struct ObserveOneDevicePolicy {
    allowed: device::DeviceId,
    allow_empty: bool,
}

impl AuthorizationPolicy for ObserveOneDevicePolicy {
    fn authorize<'a>(
        &'a self,
        _principal: &'a Principal,
        operation: Operation,
        resources: &'a [Resource],
    ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        let allowed = operation == Operation::Observe
            && if resources.is_empty() {
                self.allow_empty
            } else {
                resources
                    .iter()
                    .all(|resource| resource.device_id == self.allowed)
            };
        Box::pin(async move {
            if allowed {
                Decision::Allow
            } else {
                Decision::Deny { reason: None }
            }
        })
    }

    fn name(&self) -> &'static str {
        "observe-one-device-test-policy"
    }
}

#[tokio::test]
async fn collection_reads_project_hidden_targets_and_nested_collections() {
    use luminate_core::collection::CollectionMember;

    let (state, manager, mutations, runtime_dir) =
        request_test_context("dispatch-project-collections");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let allowed = device::DeviceId::new("request-device");
    let denied = device::DeviceId::new("hidden-device");
    let (hidden, mixed, empty) = {
        let mut state = state.lock().await;
        let hidden = state
            .create_collection(
                "Hidden".to_owned(),
                None,
                OwnerIdentity::Uid(1000),
                None,
                vec![CollectionMember::Target(TargetId::Device(denied))],
            )
            .expect("create hidden collection");
        let mixed = state
            .create_collection(
                "Mixed".to_owned(),
                None,
                OwnerIdentity::Uid(1000),
                None,
                vec![
                    CollectionMember::Collection(hidden.clone()),
                    CollectionMember::Target(TargetId::Device(allowed.clone())),
                ],
            )
            .expect("create mixed collection");
        let empty = state
            .create_collection(
                "Empty".to_owned(),
                None,
                OwnerIdentity::Uid(1000),
                None,
                Vec::new(),
            )
            .expect("create empty collection");
        (hidden, mixed, empty)
    };
    let policy = ObserveOneDevicePolicy {
        allowed: allowed.clone(),
        allow_empty: false,
    };

    let response = dispatch_request(
        Request::ListCollections,
        &DispatchContext {
            state: &state,
            plugin_manager: &manager,
            management: &test_management_read_state(),
            mutations: &mutations,
            owned_streams: &owned_streams,
            rescans: &discarding_rescans(),
            principal: &test_principal(),
            policy: &policy,
            access_administration: None,
            prefer_shm: true,
            prefer_client_shm: true,
            default_unsupported_policy: UnsupportedPolicy::Skip,
        },
    )
    .await;

    let ResponseStatus::Collections(collections) = response.status else {
        panic!("expected projected collections, got {:?}", response.status);
    };
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].id, mixed);
    assert_eq!(
        collections[0].members,
        vec![CollectionMember::Target(TargetId::Device(allowed))]
    );
    assert!(!collections.iter().any(|collection| collection.id == hidden));
    assert!(!collections.iter().any(|collection| collection.id == empty));

    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn empty_collection_read_requires_resource_free_observe() {
    let (state, manager, mutations, runtime_dir) =
        request_test_context("dispatch-project-empty-collection");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let id = state
        .lock()
        .await
        .create_collection(
            "Empty".to_owned(),
            None,
            OwnerIdentity::Principal(PrincipalId::new("unix", "1000").expect("valid principal")),
            None,
            Vec::new(),
        )
        .expect("create empty collection");
    let policy = ObserveOneDevicePolicy {
        allowed: device::DeviceId::new("request-device"),
        allow_empty: true,
    };

    let response = dispatch_request(
        Request::GetCollection { id: id.clone() },
        &DispatchContext {
            state: &state,
            plugin_manager: &manager,
            management: &test_management_read_state(),
            mutations: &mutations,
            owned_streams: &owned_streams,
            rescans: &discarding_rescans(),
            principal: &test_principal(),
            policy: &policy,
            access_administration: None,
            prefer_shm: true,
            prefer_client_shm: true,
            default_unsupported_policy: UnsupportedPolicy::Skip,
        },
    )
    .await;

    let ResponseStatus::CollectionInfo(Some(collection)) = response.status else {
        panic!(
            "expected a resource-free authorized collection, got {:?}",
            response.status
        );
    };
    assert_eq!(collection.id, id);
    assert!(collection.members.is_empty());

    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn dispatch_denies_before_executing_when_policy_refuses() {
    let (state, manager, mutations, runtime_dir) = request_test_context("dispatch-denied");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let device = device::DeviceId::new("request-device");

    let response = dispatch_request(
        Request::GetState {
            device: device.clone(),
        },
        &DispatchContext {
            state: &state,
            plugin_manager: &manager,
            management: &test_management_read_state(),
            mutations: &mutations,
            owned_streams: &owned_streams,
            rescans: &discarding_rescans(),
            principal: &test_principal(),
            policy: &DenyAllPolicy,
            access_administration: None,
            prefer_shm: true,
            prefer_client_shm: true,
            default_unsupported_policy: UnsupportedPolicy::Skip,
        },
    )
    .await;

    let ResponseStatus::Error(error) = response.status else {
        panic!(
            "expected a permission-denied error, got {:?}",
            response.status
        );
    };
    assert_eq!(error.code, ErrorCode::PermissionDenied);
    assert_eq!(error.message, "deliberate test denial");
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn single_resource_reads_deny_before_revealing_an_unknown_device() {
    let (state, manager, mutations, runtime_dir) =
        request_test_context("dispatch-denied-unknown-read");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let missing = device::DeviceId::new("unknown-device");

    for request in [
        Request::GetDevice {
            id: missing.clone(),
        },
        Request::GetState {
            device: missing.clone(),
        },
    ] {
        let response = dispatch_request(
            request,
            &DispatchContext {
                state: &state,
                plugin_manager: &manager,
                management: &test_management_read_state(),
                mutations: &mutations,
                owned_streams: &owned_streams,
                rescans: &discarding_rescans(),
                principal: &test_principal(),
                policy: &DenyAllPolicy,
                access_administration: None,
                prefer_shm: true,
                prefer_client_shm: true,
                default_unsupported_policy: UnsupportedPolicy::Skip,
            },
        )
        .await;

        let ResponseStatus::Error(error) = response.status else {
            panic!(
                "an unknown denied resource must not be revealed, got {:?}",
                response.status
            );
        };
        assert_eq!(error.code, ErrorCode::PermissionDenied);
        assert_eq!(error.message, "deliberate test denial");
    }

    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn collection_mutation_reports_denied_leaves_instead_of_erroring() {
    use luminate_core::collection::CollectionMember;

    let (state, manager, mutations, runtime_dir) =
        request_test_context("dispatch-collection-denied");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let device = device::DeviceId::new("request-device");
    let id = state
        .lock()
        .await
        .create_collection(
            "Denied room".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![CollectionMember::Target(TargetId::Device(device.clone()))],
        )
        .expect("create collection");

    for request in [
        Request::ClearTarget {
            target: Selector::Collection(id.clone()),
        },
        Request::RestoreAppearance {
            target: Selector::Collection(id),
        },
        Request::ClearTarget {
            target: Selector::Targets(vec![TargetId::Device(device.clone())]),
        },
        Request::RestoreAppearance {
            target: Selector::Targets(vec![TargetId::Device(device.clone())]),
        },
    ] {
        let response = dispatch_request(
            request,
            &DispatchContext {
                state: &state,
                plugin_manager: &manager,
                management: &test_management_read_state(),
                mutations: &mutations,
                owned_streams: &owned_streams,
                rescans: &discarding_rescans(),
                principal: &test_principal(),
                policy: &DenyAllPolicy,
                access_administration: None,
                prefer_shm: true,
                prefer_client_shm: true,
                default_unsupported_policy: UnsupportedPolicy::Skip,
            },
        )
        .await;

        let ResponseStatus::CollectionApplied { applied, denied } = response.status else {
            panic!(
                "expected a CollectionApplied response, got {:?}",
                response.status
            );
        };
        assert!(
            applied.is_empty(),
            "DenyAllPolicy must authorize nothing: {applied:?}"
        );
        assert_eq!(denied, vec![TargetId::Device(device.clone())]);
    }
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn collection_modifications_publish_appearance_updates_for_affected_devices() {
    use luminate_core::collection::CollectionMember;

    let (state, manager, mutations, runtime_dir) =
        request_test_context("dispatch-collection-modification-events");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let principal = test_principal();
    let policy = test_policy();
    let rescans = discarding_rescans();
    let management = test_management_read_state();
    let mut events = mutations.events.subscribe();
    let first = device::DeviceId::new("request-device");
    let second = device::DeviceId::new("second-device");
    let second_member = CollectionMember::Target(TargetId::Device(second.clone()));
    let dispatch = async |request| {
        dispatch_request(
            request,
            &DispatchContext {
                state: &state,
                plugin_manager: &manager,
                management: &management,
                mutations: &mutations,
                owned_streams: &owned_streams,
                rescans: &rescans,
                principal: &principal,
                policy: policy.as_ref(),
                access_administration: None,
                prefer_shm: true,
                prefer_client_shm: true,
                default_unsupported_policy: UnsupportedPolicy::Skip,
            },
        )
        .await
    };

    let created = dispatch(Request::CreateCollection {
        name: "Eventful".to_owned(),
        description: None,
        kind: None,
        members: vec![CollectionMember::Target(TargetId::Device(first.clone()))],
    })
    .await;
    let ResponseStatus::CollectionCreated { id } = created.status else {
        panic!("expected collection creation, got {:?}", created.status);
    };
    assert_eq!(
        events.recv().await.expect("receive creation event"),
        Event::StateChanged {
            devices: vec![first.clone()],
        }
    );

    let added = dispatch(Request::AddCollectionMember {
        id: id.clone(),
        member: second_member.clone(),
    })
    .await;
    assert!(matches!(added.status, ResponseStatus::Ack));
    assert_eq!(
        events.recv().await.expect("receive addition event"),
        Event::StateChanged {
            devices: vec![first.clone(), second.clone()],
        }
    );

    let removed = dispatch(Request::RemoveCollectionMember {
        id: id.clone(),
        member: second_member,
    })
    .await;
    assert!(matches!(removed.status, ResponseStatus::Ack));
    assert_eq!(
        events.recv().await.expect("receive removal event"),
        Event::StateChanged {
            devices: vec![first.clone(), second],
        }
    );

    let destroyed = dispatch(Request::DestroyCollection { id }).await;
    assert!(matches!(destroyed.status, ResponseStatus::Ack));
    assert_eq!(
        events.recv().await.expect("receive destruction event"),
        Event::StateChanged {
            devices: vec![first],
        }
    );

    remove_request_test_dir(&runtime_dir);
}

struct AdministerCollectionsOnlyPolicy;

impl AuthorizationPolicy for AdministerCollectionsOnlyPolicy {
    fn authorize<'a>(
        &'a self,
        _principal: &'a Principal,
        operation: Operation,
        _resources: &'a [Resource],
    ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        let decision = if operation == Operation::AdministerCollections {
            Decision::Allow
        } else {
            Decision::Deny {
                reason: Some("only AdministerCollections is granted".to_owned()),
            }
        };
        Box::pin(async move { decision })
    }

    fn name(&self) -> &'static str {
        "administer-collections-only-test-policy"
    }
}

fn collection_owner_test_state() -> (Arc<Mutex<DaemonState>>, CollectionId) {
    let mut daemon_state = DaemonState::default();
    let id = daemon_state
        .create_collection(
            "Owned".to_owned(),
            None,
            OwnerIdentity::Principal(PrincipalId::new("unix", "1000").expect("valid principal")),
            None,
            Vec::new(),
        )
        .expect("create collection");
    (Arc::new(Mutex::new(daemon_state)), id)
}

#[tokio::test]
async fn authorize_collection_owner_allows_the_owning_principal() {
    let (state, id) = collection_owner_test_state();
    let owner = Principal::Unix {
        uid: 1000,
        gid: 1000,
        pid: None,
    };
    // The owner never needs the policy consulted at all.
    authorize_collection_owner(&state, &id, &owner, &DenyAllPolicy)
        .await
        .expect("the owner should be authorized");
}

#[tokio::test]
async fn authorize_collection_owner_denies_a_non_owner_without_the_override() {
    let (state, id) = collection_owner_test_state();
    let stranger = Principal::Unix {
        uid: 2000,
        gid: 2000,
        pid: None,
    };
    let error = authorize_collection_owner(&state, &id, &stranger, &DenyAllPolicy)
        .await
        .expect_err("a non-owner without the override must be denied");
    assert!(matches!(error, DaemonError::CollectionNotOwned(_)));
}

#[tokio::test]
async fn authorize_collection_owner_allows_a_non_owner_granted_the_override() {
    let (state, id) = collection_owner_test_state();
    let stranger = Principal::Unix {
        uid: 2000,
        gid: 2000,
        pid: None,
    };
    authorize_collection_owner(&state, &id, &stranger, &AdministerCollectionsOnlyPolicy)
        .await
        .expect("AdministerCollections should let a non-owner act on the collection");
}

#[tokio::test]
async fn authorize_collection_owner_denies_a_windows_principal_against_a_uid_owned_collection() {
    // A Windows principal never equals a Uid-owned collection's owner.
    // There is no cross-platform identity equivalence to assert, so this
    // is the same fail-closed path as any other non-owner.
    let (state, id) = collection_owner_test_state();
    let error = authorize_collection_owner(&state, &id, &test_windows_principal(), &DenyAllPolicy)
        .await
        .expect_err("a Windows principal without the override must be denied");
    assert!(matches!(error, DaemonError::CollectionNotOwned(_)));
}

#[tokio::test]
async fn authorize_collection_owner_reports_not_found_for_an_unknown_collection() {
    let state = Arc::new(Mutex::new(DaemonState::default()));
    let error = authorize_collection_owner(
        &state,
        &CollectionId::new("missing"),
        &test_principal(),
        &DenyAllPolicy,
    )
    .await
    .expect_err("an unknown collection must not be authorizable");
    assert!(matches!(error, DaemonError::CollectionNotFound(_)));
}

#[cfg(unix)]
fn same_uid_principal() -> Principal {
    use luminate_platform::identity::daemon_own_uid;

    let uid = daemon_own_uid();
    Principal::Unix {
        uid,
        gid: uid,
        pid: None,
    }
}

#[tokio::test]
async fn begin_shm_frame_stream_is_unsupported_for_a_mismatched_uid() {
    let (state, manager, mutations, runtime_dir) = request_test_context("shm-uid-mismatch");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let target = TargetId::device("request-device");

    let response = dispatch_request(
        Request::BeginShmFrameStream {
            target: target.clone(),
        },
        &DispatchContext {
            state: &state,
            plugin_manager: &manager,
            management: &test_management_read_state(),
            mutations: &mutations,
            owned_streams: &owned_streams,
            rescans: &discarding_rescans(),
            principal: &test_principal(),
            policy: test_policy().as_ref(),
            access_administration: None,
            prefer_shm: true,
            prefer_client_shm: true,
            default_unsupported_policy: UnsupportedPolicy::Skip,
        },
    )
    .await;

    assert_error_response(&response);
    remove_request_test_dir(&runtime_dir);
}

#[cfg(unix)]
#[tokio::test]
async fn begin_shm_frame_stream_is_unsupported_when_prefer_client_shm_disabled() {
    let (state, manager, mutations, runtime_dir) = request_test_context("shm-prefer-disabled");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let target = TargetId::device("request-device");

    let response = dispatch_request(
        Request::BeginShmFrameStream {
            target: target.clone(),
        },
        &DispatchContext {
            state: &state,
            plugin_manager: &manager,
            management: &test_management_read_state(),
            mutations: &mutations,
            owned_streams: &owned_streams,
            rescans: &discarding_rescans(),
            principal: &same_uid_principal(),
            policy: test_policy().as_ref(),
            access_administration: None,
            prefer_shm: true,
            prefer_client_shm: false,
            default_unsupported_policy: UnsupportedPolicy::Skip,
        },
    )
    .await;

    assert_error_response(&response);
    remove_request_test_dir(&runtime_dir);
}

#[cfg(unix)]
#[tokio::test]
async fn begin_shm_frame_stream_is_unsupported_without_shm_capability() {
    let (state, manager, mutations, runtime_dir) = request_test_context("shm-no-capability");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let target = TargetId::device("request-device");

    // `request_test_descriptor` advertises no `FrameUploadCapability` at
    // all, so this exercises the "target doesn't advertise shared-memory
    // frame capability" collapse to `Unsupported`.
    let response = dispatch_request(
        Request::BeginShmFrameStream {
            target: target.clone(),
        },
        &DispatchContext {
            state: &state,
            plugin_manager: &manager,
            management: &test_management_read_state(),
            mutations: &mutations,
            owned_streams: &owned_streams,
            rescans: &discarding_rescans(),
            principal: &same_uid_principal(),
            policy: test_policy().as_ref(),
            access_administration: None,
            prefer_shm: true,
            prefer_client_shm: true,
            default_unsupported_policy: UnsupportedPolicy::Skip,
        },
    )
    .await;

    assert_error_response(&response);
    assert!(
        owned_streams.lock().expect("lock poisoned").is_empty(),
        "a rejected negotiation must not leave the target marked as owned"
    );
    remove_request_test_dir(&runtime_dir);
}

#[cfg(unix)]
#[tokio::test]
async fn end_shm_frame_stream_on_an_unknown_stream_is_idempotent() {
    let (state, manager, mutations, runtime_dir) = request_test_context("shm-end-unknown");
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let target = TargetId::device("request-device");

    let response = dispatch_request(
        Request::EndShmFrameStream {
            target,
            generation: 1,
        },
        &DispatchContext {
            state: &state,
            plugin_manager: &manager,
            management: &test_management_read_state(),
            mutations: &mutations,
            owned_streams: &owned_streams,
            rescans: &discarding_rescans(),
            principal: &same_uid_principal(),
            policy: test_policy().as_ref(),
            access_administration: None,
            prefer_shm: true,
            prefer_client_shm: true,
            default_unsupported_policy: UnsupportedPolicy::Skip,
        },
    )
    .await;

    assert!(matches!(response.status, ResponseStatus::Ack));
    remove_request_test_dir(&runtime_dir);
}
