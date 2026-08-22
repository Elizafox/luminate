// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the authenticated REST projection.

use super::*;
use axum::body::to_bytes;
use axum::http::header;
use luminate::{
    AppearanceSlotId, CapabilitySet, Device, Element, ElementId, ElementKind, Surface, SurfaceId,
    SurfaceKind,
};
use luminate_protocol::ResponseStatus;
use serde_json::json;
use std::fmt::Write as _;

use crate::test_support;

#[test]
fn openapi_contains_every_registered_rest_resource() {
    let document = serde_json::to_value(RestApi::openapi()).expect("serialize OpenAPI");
    let paths = document["paths"].as_object().expect("OpenAPI paths");
    let expected = [
        ("/api/v0/devices", "get"),
        ("/api/v0/devices/withdrawn", "get"),
        ("/api/v0/devices/{id}", "get"),
        ("/api/v0/devices/{id}/state", "get"),
        ("/api/v0/devices/{id}/refresh", "post"),
        ("/api/v0/devices/{id}/purge", "delete"),
        ("/api/v0/rescan", "post"),
        ("/api/v0/collections", "get"),
        ("/api/v0/collections", "post"),
        ("/api/v0/collections/{id}", "get"),
        ("/api/v0/collections/{id}", "delete"),
        ("/api/v0/collections/{id}/state", "get"),
        ("/api/v0/collections/{id}/members", "post"),
        ("/api/v0/collections/{id}/members", "delete"),
        ("/api/v0/scenes", "get"),
        ("/api/v0/scenes", "post"),
        ("/api/v0/scenes/capture", "post"),
        ("/api/v0/scenes/{id}", "get"),
        ("/api/v0/scenes/{id}", "put"),
        ("/api/v0/scenes/{id}", "delete"),
        ("/api/v0/scenes/{id}/recapture", "post"),
        ("/api/v0/scenes/{id}/apply", "post"),
        ("/api/v0/management", "get"),
        ("/api/v0/management", "patch"),
        ("/api/v0/policy", "get"),
        ("/api/v0/policy", "put"),
        ("/api/v0/control", "post"),
        ("/api/v0/transitions", "post"),
        ("/api/v0/transitions/{id}", "get"),
        ("/api/v0/transitions/{id}", "delete"),
        ("/api/v0/transitions/{id}/wait", "post"),
    ];

    for (path, method) in expected {
        assert!(
            paths.get(path).and_then(|item| item.get(method)).is_some(),
            "missing {method} {path}",
        );
    }
}

#[test]
fn openapi_operations_have_non_placeholder_contracts() {
    let document = serde_json::to_value(RestApi::openapi()).expect("serialize OpenAPI");
    let paths = document["paths"].as_object().expect("OpenAPI paths");

    for (path, item) in paths {
        for (method, operation) in item.as_object().expect("OpenAPI path item") {
            let parameters = operation["parameters"].as_array();
            for name in path
                .split('{')
                .skip(1)
                .filter_map(|part| part.split_once('}').map(|(name, _)| name))
            {
                assert!(
                    parameters.is_some_and(|parameters| parameters.iter().any(|parameter| {
                        parameter["name"] == name && parameter["in"] == "path"
                    })),
                    "{method} {path} does not describe path parameter {name}",
                );
            }

            if let Some(request) = operation.get("requestBody") {
                assert_schema_reference(
                    &request["content"]["application/json"]["schema"],
                    &format!("{method} {path} request"),
                );
            }

            let responses = operation["responses"].as_object().expect("responses");
            for (status, response) in responses {
                let body = response
                    .get("content")
                    .and_then(|content| content.get("application/json"))
                    .map(|media| &media["schema"]);
                if status.starts_with('2') && status != "204" {
                    let body =
                        body.unwrap_or_else(|| panic!("{method} {path} {status} has no body"));
                    assert_schema_reference(body, &format!("{method} {path} {status}"));
                } else if !status.starts_with('2') {
                    let body = body.unwrap_or_else(|| {
                        panic!("{method} {path} problem response {status} has no body")
                    });
                    assert_schema_reference(body, &format!("{method} {path} {status}"));
                }
            }
        }
    }
}

#[test]
fn openapi_contract_snapshot_is_current() {
    let document = serde_json::to_value(RestApi::openapi()).expect("serialize OpenAPI");
    let mut snapshot = String::new();
    for (path, item) in document["paths"].as_object().expect("OpenAPI paths") {
        for (method, operation) in item.as_object().expect("OpenAPI path item") {
            let request = operation["requestBody"]["content"]["application/json"]["schema"]["$ref"]
                .as_str()
                .unwrap_or("-");
            let success = operation["responses"]
                .as_object()
                .and_then(|responses| responses.iter().find(|(status, _)| status.starts_with('2')))
                .and_then(|(_, response)| {
                    response["content"]["application/json"]["schema"]
                        .get("$ref")
                        .and_then(serde_json::Value::as_str)
                        .or_else(|| {
                            response["content"]["application/json"]["schema"]["items"]["$ref"]
                                .as_str()
                        })
                })
                .unwrap_or("-");
            writeln!(
                snapshot,
                "{method} {path} request={request} success={success}"
            )
            .expect("write snapshot");
        }
    }
    let expected = include_str!("openapi-rest-v0.snapshot").replace("\r\n", "\n");
    assert_eq!(snapshot, expected);
}

fn assert_schema_reference(schema: &serde_json::Value, context: &str) {
    if schema.get("$ref").is_some() {
        return;
    }
    if schema["type"] == "array" {
        assert_schema_reference(&schema["items"], context);
        return;
    }
    assert!(
        matches!(
            schema["type"].as_str(),
            Some("string" | "integer" | "number" | "boolean")
        ),
        "{context} uses an inline or placeholder schema: {schema}"
    );
}

#[test]
fn daemon_errors_map_to_stable_http_statuses_and_challenges() {
    let cases = [
        (
            Error::AuthenticationFailed("bad token".to_owned()),
            StatusCode::UNAUTHORIZED,
        ),
        (
            Error::PermissionDenied {
                reason: Some("nope".to_owned()),
            },
            StatusCode::FORBIDDEN,
        ),
        (Error::NotFound("lamp".to_owned()), StatusCode::NOT_FOUND),
        (
            Error::InvalidArgument("bad".to_owned()),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            Error::Unsupported("bad".to_owned()),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            Error::UnknownState("bad".to_owned()),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (Error::Conflict("busy".to_owned()), StatusCode::CONFLICT),
        (
            Error::RateLimited {
                message: "slow down".to_owned(),
                retry_after_ms: Some(250),
            },
            StatusCode::TOO_MANY_REQUESTS,
        ),
        (Error::DaemonUnavailable, StatusCode::SERVICE_UNAVAILABLE),
        (
            Error::Unavailable("offline".to_owned()),
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (
            Error::Internal("broken".to_owned()),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        (
            Error::Protocol("broken".to_owned()),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        (Error::ConnectionPoisoned, StatusCode::INTERNAL_SERVER_ERROR),
        (
            Error::PartialMutation {
                message: "partial".to_owned(),
                applied_targets: vec![TargetId::device("lamp")],
            },
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    ];

    for (error, expected) in cases {
        let authentication_failure = error.kind() == ErrorKind::AuthenticationFailed;
        let response = error_response(&error);
        assert_eq!(response.status(), expected);
        assert_eq!(
            response.headers().get(header::WWW_AUTHENTICATE).is_some(),
            authentication_failure
        );
    }
}

#[test]
fn response_helpers_preserve_success_and_failure_statuses() {
    assert_eq!(json_response::<u8>(Ok(7)).status(), StatusCode::OK);
    assert_eq!(empty_response(Ok(())).status(), StatusCode::NO_CONTENT);
    assert_eq!(
        outcome_response(Ok(CollectionOutcome {
            applied: Vec::new(),
            denied: Vec::new()
        }))
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        json_response::<u8>(Err(Error::NotFound("missing".to_owned()))).status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one table-like test covers the complete device and collection HTTP resource family"
)]
async fn device_and_collection_handlers_project_daemon_results() {
    macro_rules! run {
        ($status:expr, $call:expr, $expected:expr) => {{
            let fixture = test_support::one_response($status);
            let authenticated = session(&fixture.state, fixture.auth.clone())
                .await
                .expect("authenticate mock HTTP client");
            let response = $call(authenticated).await;
            assert_eq!(response.status(), $expected);
            fixture.finish().await;
        }};
    }

    run!(
        ResponseStatus::Devices(Vec::new()),
        list_devices,
        StatusCode::OK
    );
    run!(
        ResponseStatus::Device(None),
        |auth| get_device(auth, Path("missing".to_owned())),
        StatusCode::NOT_FOUND
    );
    run!(
        ResponseStatus::WithdrawnDevices(vec![DeviceId::new("retired")]),
        withdrawn,
        StatusCode::OK
    );
    run!(
        ResponseStatus::State(None),
        |auth| device_state(auth, Path("missing".to_owned())),
        StatusCode::NOT_FOUND
    );
    run!(
        ResponseStatus::Ack,
        |auth| refresh(auth, Path("lamp".to_owned())),
        StatusCode::NO_CONTENT
    );
    run!(
        ResponseStatus::Ack,
        |auth| purge(auth, Path("lamp".to_owned())),
        StatusCode::NO_CONTENT
    );
    run!(ResponseStatus::Ack, rescan, StatusCode::NO_CONTENT);
    run!(
        ResponseStatus::Collections(Vec::new()),
        list_collections,
        StatusCode::OK
    );
    run!(
        ResponseStatus::CollectionCreated {
            id: CollectionId::new("room")
        },
        |auth| create_collection(
            auth,
            Json(CreateCollectionRequest {
                name: "Room".to_owned(),
                description: None,
                kind: None,
                members: Vec::new(),
            }),
        ),
        StatusCode::CREATED
    );
    run!(
        ResponseStatus::CollectionInfo(None),
        |auth| get_collection(auth, Path("missing".to_owned())),
        StatusCode::NOT_FOUND
    );
    run!(
        ResponseStatus::CollectionState(None),
        |auth| collection_state(auth, Path("missing".to_owned())),
        StatusCode::NOT_FOUND
    );
    run!(
        ResponseStatus::Ack,
        |auth| destroy_collection(auth, Path("room".to_owned())),
        StatusCode::NO_CONTENT
    );
    run!(
        ResponseStatus::Ack,
        |auth| add_member(
            auth,
            Path("room".to_owned()),
            Json(CollectionMemberRequest {
                member: CollectionMember::Target(TargetId::device("lamp"))
            }),
        ),
        StatusCode::NO_CONTENT
    );
    run!(
        ResponseStatus::Ack,
        |auth| remove_member(
            auth,
            Path("room".to_owned()),
            Json(CollectionMemberRequest {
                member: CollectionMember::Target(TargetId::device("lamp"))
            }),
        ),
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn device_handler_preserves_scoped_physical_tags() {
    let device = Device {
        id: DeviceId::new("beam"),
        name: "Beam".to_owned(),
        vendor: None,
        model: None,
        provider_instance: Some("example".to_owned()),
        surfaces: vec![Surface {
            id: SurfaceId::new("bars"),
            name: "Bars".to_owned(),
            kind: SurfaceKind::Linear { length: 2.0 },
            physical_tags: vec!["layout:horizontal".to_owned()],
            elements: vec![Element {
                id: ElementId::new("left"),
                name: Some("Left".to_owned()),
                kind: ElementKind::Led,
                geometry: None,
                physical_tags: vec!["shape:round".to_owned(), "position:left".to_owned()],
                capabilities: CapabilitySet::default(),
                notes: Vec::new(),
                warnings: Vec::new(),
            }],
            capabilities: CapabilitySet::default(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: Vec::new(),
        capabilities: CapabilitySet::default(),
        category: None,
        physical_tags: vec!["shape:modular-light-bar".to_owned()],
        host_attached: false,
        notes: Vec::new(),
        warnings: Vec::new(),
    };
    let fixture = test_support::one_response(ResponseStatus::Device(Some(Box::new(device))));
    let authenticated = session(&fixture.state, fixture.auth.clone())
        .await
        .expect("authenticate mock HTTP client");
    let response = get_device(authenticated, Path("beam".to_owned())).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("parse response body");

    assert_eq!(body["physical_tags"], json!(["shape:modular-light-bar"]));
    assert_eq!(
        body["surfaces"][0]["physical_tags"],
        json!(["layout:horizontal"])
    );
    assert_eq!(
        body["surfaces"][0]["elements"][0]["physical_tags"],
        json!(["shape:round", "position:left"])
    );
    fixture.finish().await;
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "the exhaustive request-shape table verifies every control projection against the daemon protocol"
)]
async fn every_control_request_shape_reaches_the_daemon() {
    async fn invoke(request: ControlRequest, collection: bool) {
        let status = if collection {
            ResponseStatus::CollectionApplied {
                applied: vec![TargetId::device("lamp")],
                denied: Vec::new(),
            }
        } else {
            ResponseStatus::Ack
        };
        let fixture = test_support::one_response(status);
        let authenticated = session(&fixture.state, fixture.auth.clone())
            .await
            .expect("authenticate mock HTTP client");
        let response = control(authenticated, Json(request)).await;
        assert!(matches!(
            response.status(),
            StatusCode::OK | StatusCode::NO_CONTENT
        ));
        fixture.finish().await;
    }

    let target = || Selector::Target(TargetId::device("lamp"));
    let collection = || Selector::Collection(CollectionId::new("room"));
    let colour = || Colour::rgb(Rgb::new(1, 2, 3));
    invoke(
        ControlRequest::AppearanceSlots {
            target: TargetId::surface("lamp", "power"),
            values: vec![AppearanceSlotValue {
                slot: AppearanceSlotId::new("ac"),
                effect: Effect::Off,
            }],
        },
        false,
    )
    .await;
    invoke(
        ControlRequest::Effect {
            selector: target(),
            effect: Effect::Off,
            on_unsupported: None,
        },
        false,
    )
    .await;
    invoke(
        ControlRequest::Effect {
            selector: collection(),
            effect: Effect::Off,
            on_unsupported: None,
        },
        true,
    )
    .await;
    invoke(
        ControlRequest::Colour {
            selector: target(),
            colour: colour(),
            on_unsupported: None,
        },
        false,
    )
    .await;
    invoke(
        ControlRequest::Colour {
            selector: collection(),
            colour: colour(),
            on_unsupported: None,
        },
        true,
    )
    .await;
    invoke(
        ControlRequest::Rgb {
            selector: target(),
            rgb: Rgb::new(1, 2, 3),
            on_unsupported: None,
        },
        false,
    )
    .await;
    invoke(
        ControlRequest::Rgb {
            selector: collection(),
            rgb: Rgb::new(1, 2, 3),
            on_unsupported: None,
        },
        true,
    )
    .await;
    invoke(
        ControlRequest::Cct {
            selector: target(),
            kelvin: 4_000,
            on_unsupported: None,
        },
        false,
    )
    .await;
    invoke(
        ControlRequest::Cct {
            selector: collection(),
            kelvin: 4_000,
            on_unsupported: None,
        },
        true,
    )
    .await;
    invoke(
        ControlRequest::Brightness {
            selector: target(),
            value: 50,
            on_unsupported: None,
        },
        false,
    )
    .await;
    invoke(
        ControlRequest::Brightness {
            selector: collection(),
            value: 50,
            on_unsupported: None,
        },
        true,
    )
    .await;
    invoke(ControlRequest::Clear { selector: target() }, false).await;
    invoke(
        ControlRequest::Clear {
            selector: collection(),
        },
        true,
    )
    .await;
    invoke(ControlRequest::SaveCurrent { selector: target() }, false).await;
    invoke(
        ControlRequest::SaveCurrent {
            selector: collection(),
        },
        true,
    )
    .await;
    invoke(
        ControlRequest::Off {
            target: TargetId::device("lamp"),
        },
        false,
    )
    .await;
    invoke(
        ControlRequest::RestoreAppearance { selector: target() },
        false,
    )
    .await;
    invoke(
        ControlRequest::RestoreAppearance {
            selector: collection(),
        },
        true,
    )
    .await;
    invoke(
        ControlRequest::Emission {
            selector: target(),
            state: EmissionState::Emitting,
        },
        false,
    )
    .await;
    invoke(
        ControlRequest::Emission {
            selector: collection(),
            state: EmissionState::Emitting,
        },
        true,
    )
    .await;
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one integration table covers the related scene, policy, management, and transition resources"
)]
async fn scene_policy_management_and_transition_boundaries_are_projected() {
    use luminate::OwnerIdentity;
    use luminate::policy::{Preset, materialize_presets};
    use luminate_protocol::{DaemonPreferences, ManagementChangeSet, ManagementSnapshot};

    fn scene() -> luminate::Scene {
        luminate::Scene {
            id: SceneId::new("scene"),
            revision: 1,
            name: "Scene".to_owned(),
            description: None,
            owner: OwnerIdentity::Principal(
                PrincipalId::new("local", "http-test").expect("principal"),
            ),
            bindings: Vec::new(),
        }
    }

    macro_rules! run {
        ($status:expr, $call:expr, $expected:expr) => {{
            let fixture = test_support::one_response($status);
            let authenticated = session(&fixture.state, fixture.auth.clone())
                .await
                .expect("authenticate mock HTTP client");
            let response = $call(authenticated).await;
            assert_eq!(response.status(), $expected);
            fixture.finish().await;
        }};
    }

    run!(
        ResponseStatus::Scenes(Vec::new()),
        list_scenes,
        StatusCode::OK
    );
    run!(
        ResponseStatus::Scene(Box::new(scene())),
        |auth| create_scene(
            auth,
            Json(CreateSceneRequest {
                name: "Scene".to_owned(),
                description: None,
                bindings: Vec::new()
            })
        ),
        StatusCode::CREATED
    );
    run!(
        ResponseStatus::Scene(Box::new(scene())),
        |auth| capture_scene(
            auth,
            Json(CaptureSceneRequest {
                name: "Scene".to_owned(),
                description: None,
                mode: SceneCaptureMode::Frozen,
                targets: Vec::new()
            })
        ),
        StatusCode::CREATED
    );
    run!(
        ResponseStatus::SceneInfo(None),
        |auth| get_scene(auth, Path("missing".to_owned())),
        StatusCode::NOT_FOUND
    );
    run!(
        ResponseStatus::Scene(Box::new(scene())),
        |auth| replace_scene(
            auth,
            Path("scene".to_owned()),
            Json(ReplaceSceneRequest {
                expected_revision: 1,
                name: "Scene".to_owned(),
                description: None,
                bindings: Vec::new()
            })
        ),
        StatusCode::OK
    );
    run!(
        ResponseStatus::Scene(Box::new(scene())),
        |auth| recapture_scene(
            auth,
            Path("scene".to_owned()),
            Json(RecaptureSceneRequest {
                expected_revision: 1,
                mode: SceneCaptureMode::Frozen,
                targets: Vec::new()
            })
        ),
        StatusCode::OK
    );
    run!(
        ResponseStatus::Ack,
        |auth| delete_scene(
            auth,
            Path("scene".to_owned()),
            Json(ExpectedRevision {
                expected_revision: 1
            })
        ),
        StatusCode::NO_CONTENT
    );
    run!(
        ResponseStatus::SceneApplied {
            applied: Vec::new(),
            denied: Vec::new()
        },
        |auth| apply_scene(auth, Path("scene".to_owned())),
        StatusCode::OK
    );

    let preferences = DaemonPreferences {
        default_unsupported_policy: None,
        reconciliation_policy: None,
        device_reconciliation: Vec::new(),
        cct_emulation: None,
        prefer_shm: None,
        prefer_client_shm: None,
    };
    run!(
        ResponseStatus::ManagementSnapshot(Box::new(ManagementSnapshot {
            revision: 0,
            desired_daemon: preferences.clone(),
            effective_daemon: preferences,
            locked_daemon_settings: Vec::new(),
            plugins: Vec::new()
        })),
        management,
        StatusCode::OK
    );
    run!(
        ResponseStatus::ManagementPatched(ManagementChangeSet {
            revision: 1,
            changes: Vec::new()
        }),
        |auth| patch_management(
            auth,
            Json(ManagementPatch {
                expected_revision: 0,
                mutations: Vec::new()
            })
        ),
        StatusCode::OK
    );

    let source = materialize_presets(PolicyRevision(1), [Preset::Administrator]);
    run!(
        ResponseStatus::AccessPolicy(Box::new(source.clone())),
        policy,
        StatusCode::OK
    );
    run!(
        ResponseStatus::AccessPolicy(Box::new(source.clone())),
        |auth| replace_policy(
            auth,
            Json(ReplacePolicyRequest {
                expected_revision: PolicyRevision(1),
                document: source.clone()
            })
        ),
        StatusCode::OK
    );

    let fixture = test_support::scripted(vec![Vec::new()]);
    let authenticated = session(&fixture.state, fixture.auth.clone())
        .await
        .expect("authenticate mock HTTP client");
    assert_eq!(
        transition_status(
            State(fixture.state.clone()),
            authenticated,
            Path("missing".to_owned())
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    fixture.finish().await;
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one lifecycle test covers every transition source and owned operation"
)]
async fn transition_handlers_cover_each_source_and_owned_lifecycle() {
    use luminate::{TransitionOutcome, TransitionStatus};
    use std::time::Duration;

    fn options() -> TransitionOptions {
        TransitionOptions::new(Duration::from_secs(1), None).expect("options")
    }

    fn status(id: &str) -> ResponseStatus {
        ResponseStatus::Transition(Box::new(TransitionStatus {
            id: TransitionId::new(id),
            targets: Vec::new(),
            elapsed_ms: 0,
            duration_ms: 1_000,
            outcome: Some(TransitionOutcome::Completed),
        }))
    }

    async fn fixture_with_transition(
        id: &str,
        response: ResponseStatus,
    ) -> test_support::MockDaemon {
        let fixture = test_support::one_response(response);
        fixture.state.transitions.lock().await.insert(
            TransitionId::new(id),
            StoredTransition {
                principal: PrincipalId::new("local", "http-test").expect("principal"),
                id: TransitionId::new(id),
            },
        );
        fixture
    }

    let requests = [
        StartTransitionRequest::SceneToScene {
            scene: SceneId::new("source"),
            destination: SceneId::new("destination"),
            options: options(),
        },
        StartTransitionRequest::CurrentToScene {
            destination: SceneId::new("destination"),
            options: options(),
        },
        StartTransitionRequest::SceneToStates {
            scene: SceneId::new("source"),
            states: Vec::new(),
            options: options(),
        },
        StartTransitionRequest::CurrentToStates {
            states: Vec::new(),
            options: options(),
        },
    ];
    for (index, request) in requests.into_iter().enumerate() {
        let id = format!("transition-{index}");
        let fixture = test_support::one_response(status(&id));
        let authenticated = session(&fixture.state, fixture.auth.clone())
            .await
            .expect("authenticate mock HTTP client");
        assert_eq!(
            start_transition(State(fixture.state.clone()), authenticated, Json(request),)
                .await
                .status(),
            StatusCode::CREATED
        );
        fixture.finish().await;
    }

    let fixture = fixture_with_transition("status", status("status")).await;
    let authenticated = session(&fixture.state, fixture.auth.clone())
        .await
        .expect("authenticate mock HTTP client");
    assert_eq!(
        transition_status(
            State(fixture.state.clone()),
            authenticated,
            Path("status".to_owned())
        )
        .await
        .status(),
        StatusCode::OK
    );
    fixture.finish().await;

    let fixture = fixture_with_transition("abort", status("abort")).await;
    let authenticated = session(&fixture.state, fixture.auth.clone())
        .await
        .expect("authenticate mock HTTP client");
    assert_eq!(
        abort_transition(
            State(fixture.state.clone()),
            authenticated,
            Path("abort".to_owned())
        )
        .await
        .status(),
        StatusCode::OK
    );
    fixture.finish().await;

    let fixture = fixture_with_transition("wait", status("wait")).await;
    let authenticated = session(&fixture.state, fixture.auth.clone())
        .await
        .expect("authenticate mock HTTP client");
    assert_eq!(
        wait_transition(
            State(fixture.state.clone()),
            authenticated,
            Path("wait".to_owned())
        )
        .await
        .status(),
        StatusCode::OK
    );
    fixture.finish().await;
}
