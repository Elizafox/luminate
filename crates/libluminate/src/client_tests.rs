// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Unix-only: this fake-daemon test fixture binds a raw `tokio::net::UnixListener`
//! rather than the portable `luminate_platform::transport::Listener` the real
//! daemon uses. The production `Client`
//! this exercises is already cross-platform, but generalizing this fixture to
//! the `Connection` abstraction across every test in this file has not been
//! done yet.

#![cfg(unix)]

use std::any::Any;
use std::future;
use std::path::PathBuf;
use std::result::Result as StdResult;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{fs, process};
use tokio::time;

use tokio::net::{UnixListener, UnixStream};

use serde::Serialize;

use luminate_core::capability::CapabilitySet;
use luminate_core::colour::Colour;
use luminate_core::device::{Device, DeviceId};
use luminate_core::element::{Element, ElementId, ElementKind};
use luminate_core::policy::{
    Operation, PolicyRevision, Preset, PrincipalId, ResourceConstraints, ScopeGrant, SessionScope,
    materialize_presets,
};
use luminate_core::rgb::Rgb;
use luminate_core::surface::{Surface, SurfaceId, SurfaceKind};
use luminate_protocol::PROTOCOL_ABI_VERSION;
use luminate_protocol::framing::{FramingError, send as protocol_send};
use luminate_protocol::{
    AuthenticationSource, Compatibility, Credential, DaemonHello, ErrorCode, EventCompatibility,
    OperationError, PluginSetupInteractionResponse, PluginSetupSession, PluginSetupSessionId,
    PluginSetupSessionState, Response, SessionMetadata, SubscribeAck, SubscribeHello,
};

use super::*;

fn unique_socket_path() -> PathBuf {
    static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(0);

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
    Path::new("/tmp").join(format!("luminate-test-{}-{nanos}-{id}.sock", process::id()))
}

fn tagged_device(id: &str) -> Device {
    Device {
        id: DeviceId::new(id),
        name: "Beam".to_owned(),
        vendor: None,
        model: None,
        provider_instance: Some("example".to_owned()),
        surfaces: vec![Surface {
            id: SurfaceId::new("light-bars"),
            name: "Light bars".to_owned(),
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
    }
}

async fn send<T>(stream: &mut UnixStream, value: &T) -> StdResult<(), FramingError>
where
    T: Any + Serialize,
{
    if let Some(hello) = (value as &dyn Any).downcast_ref::<DaemonHello>() {
        protocol_send(stream, hello).await?;
        if matches!(hello.compatibility, Compatibility::Compatible)
            && hello.protocol_abi_version == PROTOCOL_ABI_VERSION
        {
            let authentication: AuthenticationRequest = receive(stream).await?;
            assert!(matches!(
                authentication.authentication,
                Authentication::Peer
            ));
            protocol_send(
                stream,
                &AuthenticationResponse::Authenticated {
                    session: SessionMetadata {
                        subject: PrincipalId::new("unix", "1000").expect("valid test principal"),
                        verified_groups: Vec::new(),
                        source: AuthenticationSource::Peer,
                        credential_id: None,
                        expires_at: None,
                    },
                },
            )
            .await?;
        }
        return Ok(());
    }

    if let Some(response) = (value as &dyn Any).downcast_ref::<Response>() {
        return protocol_send(
            stream,
            &ResponseMessage {
                id: 0,
                response: response.clone(),
            },
        )
        .await;
    }

    protocol_send(stream, value).await
}

async fn receive_request(stream: &mut UnixStream) -> (u64, Request) {
    let message: RequestMessage = receive(stream).await.expect("receive request");
    (message.id, message.request)
}

async fn send_response(stream: &mut UnixStream, id: u64, status: ResponseStatus) {
    send(
        stream,
        &ResponseMessage {
            id,
            response: Response { status },
        },
    )
    .await
    .expect("send response");
}

#[tokio::test]
async fn plugin_setup_workflows_preserve_typed_metadata() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                daemon_version: "test".to_owned(),
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                compatibility: Compatibility::Compatible,
            },
        )
        .await
        .expect("send hello");

        let (id, request) = receive_request(&mut stream).await;
        assert!(matches!(
            request,
            Request::ListPluginSetupWorkflows { plugin } if plugin == "example"
        ));
        send_response(
            &mut stream,
            id,
            ResponseStatus::PluginSetupWorkflows(vec![PluginSetupWorkflow::new(
                "example",
                "pair",
                "Pair hardware",
                "Connect nearby hardware.",
                PluginSetupWorkflowKind::Provision,
            )]),
        )
        .await;
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    let workflows = client
        .plugin_setup_workflows("example")
        .await
        .expect("list workflows");
    assert_eq!(workflows.len(), 1);
    assert_eq!(workflows[0].id, "pair");
    assert_eq!(workflows[0].kind, PluginSetupWorkflowKind::Provision);

    server.await.expect("join fake daemon");
    fs::remove_file(path).expect("remove test socket");
}

#[tokio::test]
async fn plugin_setup_session_methods_preserve_generation_and_typed_state() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");
    let session_id =
        PluginSetupSessionId::parse("0123456789abcdef0123456789abcdef").expect("valid session ID");
    let server_id = session_id.clone();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                daemon_version: "test".to_owned(),
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                compatibility: Compatibility::Compatible,
            },
        )
        .await
        .expect("send hello");

        let (id, request) = receive_request(&mut stream).await;
        assert!(
            matches!(request, Request::StartPluginSetup { plugin, workflow } if plugin == "example" && workflow == "pair")
        );
        send_response(
            &mut stream,
            id,
            ResponseStatus::PluginSetupSession(Box::new(PluginSetupSession {
                id: server_id.clone(),
                plugin: "example".to_owned(),
                workflow: "pair".to_owned(),
                generation: 1,
                state: PluginSetupSessionState::PhysicalAction {
                    instruction: "Press the button.".to_owned(),
                },
            })),
        )
        .await;

        let (id, request) = receive_request(&mut stream).await;
        assert!(matches!(
            request,
            Request::RespondPluginSetup {
                session,
                generation: 1,
                response: PluginSetupInteractionResponse::Confirmed,
            } if session == server_id
        ));
        send_response(
            &mut stream,
            id,
            ResponseStatus::PluginSetupSession(Box::new(PluginSetupSession {
                id: server_id,
                plugin: "example".to_owned(),
                workflow: "pair".to_owned(),
                generation: 2,
                state: PluginSetupSessionState::Completed {
                    summary: "Connected.".to_owned(),
                    revision: 4,
                },
            })),
        )
        .await;
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    let session = client
        .start_plugin_setup("example", "pair")
        .await
        .expect("start setup");
    assert_eq!(session.id, session_id);
    assert!(matches!(
        session.state,
        PluginSetupSessionState::PhysicalAction { .. }
    ));
    let session = client
        .respond_plugin_setup(
            session.id,
            session.generation,
            PluginSetupInteractionResponse::Confirmed,
        )
        .await
        .expect("confirm setup action");
    assert!(matches!(
        session.state,
        PluginSetupSessionState::Completed { revision: 4, .. }
    ));

    server.await.expect("join fake daemon");
    fs::remove_file(path).expect("remove test socket");
}

#[tokio::test]
async fn connect_and_fetch_server_info() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");

        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        let request = receive_request(&mut stream).await.1;
        let Request::ServerInfo = request else {
            panic!("unexpected request: {request:?}");
        };
        send(
            &mut stream,
            &Response {
                status: ResponseStatus::ServerInfo(luminate_protocol::ServerInfo {
                    daemon_name: "luminated".to_owned(),
                    daemon_version: "0.1.0".to_owned(),
                    protocol_abi_version: PROTOCOL_ABI_VERSION,
                }),
            },
        )
        .await
        .expect("send server info");
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    let info = client.server_info().await.expect("fetch server info");
    assert_eq!(info.daemon_name, "luminated");
    assert_eq!(client.daemon_version(), "0.1.0");

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn builder_sends_authentication_scope_and_retains_session_metadata() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");
    let scope = SessionScope::new(vec![ScopeGrant {
        operations: [Operation::Observe].into_iter().collect(),
        resources: ResourceConstraints::default(),
    }])
    .expect("valid scope");
    let expected_scope = scope.clone();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        protocol_send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");
        let request: AuthenticationRequest =
            receive(&mut stream).await.expect("receive authentication");
        assert!(matches!(
            request.authentication,
            Authentication::Bearer { .. }
        ));
        assert_eq!(request.scope, Some(expected_scope));
        protocol_send(
            &mut stream,
            &AuthenticationResponse::Authenticated {
                session: SessionMetadata {
                    subject: PrincipalId::new("local", "alice").expect("valid principal"),
                    verified_groups: vec!["operators".to_owned()],
                    source: AuthenticationSource::Bearer,
                    credential_id: Some("token-1".to_owned()),
                    expires_at: None,
                },
            },
        )
        .await
        .expect("send authentication response");
    });

    let client = ClientBuilder::new()
        .path(&path)
        .authentication(Authentication::Bearer {
            credential: Credential::new("secret").expect("valid credential"),
        })
        .scope(scope)
        .connect()
        .await
        .expect("connect client");
    assert_eq!(client.session().subject.authority(), "local");
    assert_eq!(client.session().subject.subject(), "alice");
    assert_eq!(client.session().credential_id.as_deref(), Some("token-1"));

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn grouped_administration_handles_use_daemon_requests() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");
    let source = materialize_presets(PolicyRevision(7), [Preset::Administrator]);
    let expected_source = source.clone();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        let (id, request) = receive_request(&mut stream).await;
        assert!(matches!(request, Request::GetAccessPolicy));
        send_response(
            &mut stream,
            id,
            ResponseStatus::AccessPolicy(Box::new(expected_source)),
        )
        .await;

        let (id, request) = receive_request(&mut stream).await;
        assert!(matches!(&request, Request::CreateToken { id, .. } if id == "front-end"));
        send_response(
            &mut stream,
            id,
            ResponseStatus::TokenCreated {
                metadata: TokenMetadata {
                    id: "front-end".to_owned(),
                    subject: PrincipalId::new("local", "alice").expect("valid principal"),
                    expires_at: None,
                    revoked: false,
                },
                secret: Credential::new("display-once").expect("valid credential"),
            },
        )
        .await;
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    let document = client
        .policy_administration()
        .get()
        .await
        .expect("get policy");
    assert_eq!(document.revision(), PolicyRevision(7));
    let created = client
        .authentication_administration()
        .create_token(
            "front-end",
            PrincipalId::new("local", "alice").expect("valid principal"),
            None,
        )
        .await
        .expect("create token");
    assert_eq!(created.metadata.id, "front-end");
    assert_eq!(created.secret.expose(), b"display-once");

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn ping_uses_the_connected_control_stream_and_expects_ack() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        let (id, request) = receive_request(&mut stream).await;
        assert!(matches!(request, Request::Ping));
        send_response(&mut stream, id, ResponseStatus::Ack).await;
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    client.ping().await.expect("ping daemon");

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn incompatible_daemon_is_reported() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Incompatible {
                    supported_protocol_abi_version: 99,
                    reason: Some("test incompatibility".to_owned()),
                },
                protocol_abi_version: 99,
                daemon_version: "9.9.9".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");
    });

    let err = Client::connect_path(&path)
        .await
        .expect_err("client should reject incompatible daemon");

    let Error::IncompatibleDaemon {
        daemon_version,
        supported_protocol_abi_version,
        ..
    } = err
    else {
        panic!("unexpected error: {err:?}");
    };
    assert_eq!(daemon_version, "9.9.9");
    assert_eq!(supported_protocol_abi_version, 99);

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn daemon_error_maps_to_typed_error() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        let _: RequestMessage = receive(&mut stream).await.expect("receive request");
        send(
            &mut stream,
            &Response {
                status: ResponseStatus::Error(OperationError {
                    code: ErrorCode::Unsupported,
                    message: "not supported by device".to_owned(),
                    retry_after_ms: None,
                    applied_targets: Vec::new(),
                }),
            },
        )
        .await
        .expect("send error response");
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    let err = client
        .clear_target(TargetId::Device(DeviceId::new("device0")))
        .await
        .expect_err("request should fail");

    let Error::Unsupported(message) = err else {
        panic!("unexpected error: {err:?}");
    };
    assert_eq!(message, "not supported by device");

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn save_current_sends_wire_request() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        let request = receive_request(&mut stream).await.1;
        let Request::SaveCurrent { target } = request else {
            panic!("unexpected request: {request:?}");
        };
        assert_eq!(
            target,
            Selector::Target(TargetId::Device(DeviceId::new("device0")))
        );

        send(
            &mut stream,
            &Response {
                status: ResponseStatus::Ack,
            },
        )
        .await
        .expect("send ack");
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    client
        .save_current(TargetId::Device(DeviceId::new("device0")))
        .await
        .expect("save-current should ack");

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn rescan_sends_wire_request() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        let request = receive_request(&mut stream).await.1;
        assert!(
            matches!(request, Request::Rescan),
            "unexpected request: {request:?}"
        );
        send(
            &mut stream,
            &Response {
                status: ResponseStatus::Ack,
            },
        )
        .await
        .expect("send ack");
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    client.rescan().await.expect("rescan should ack");

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn purge_withdrawn_device_sends_wire_request() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        let request = receive_request(&mut stream).await.1;
        let Request::PurgeWithdrawnDevice { device } = request else {
            panic!("unexpected request: {request:?}");
        };
        assert_eq!(device, DeviceId::new("retired-device"));
        send(
            &mut stream,
            &Response {
                status: ResponseStatus::Ack,
            },
        )
        .await
        .expect("send ack");
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    client
        .purge_withdrawn_device(DeviceId::new("retired-device"))
        .await
        .expect("purge should ack");

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn list_withdrawn_devices_returns_daemon_ids() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        assert!(matches!(
            receive_request(&mut stream).await.1,
            Request::ListWithdrawnDevices
        ));
        send(
            &mut stream,
            &Response {
                status: ResponseStatus::WithdrawnDevices(vec![
                    DeviceId::new("retired-a"),
                    DeviceId::new("retired-b"),
                ]),
            },
        )
        .await
        .expect("send withdrawn devices");
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    assert_eq!(
        client
            .list_withdrawn_devices()
            .await
            .expect("list withdrawn devices"),
        vec![DeviceId::new("retired-a"), DeviceId::new("retired-b")]
    );

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn set_effect_sends_static_colour_request() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        let request = receive_request(&mut stream).await.1;
        let Request::SetEffect(SetEffectRequest {
            selector,
            effect,
            on_unsupported,
        }) = request
        else {
            panic!("unexpected request: {request:?}");
        };
        assert_eq!(
            selector,
            Selector::Target(TargetId::element("keyboard", "keys", "escape"))
        );
        assert_eq!(
            effect,
            Effect::Static {
                colour: Colour::rgb(Rgb::new(1, 2, 3))
            }
        );
        assert_eq!(on_unsupported, None);

        send(
            &mut stream,
            &Response {
                status: ResponseStatus::Ack,
            },
        )
        .await
        .expect("send ack");
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    client
        .set_effect(
            TargetId::element("keyboard", "keys", "escape"),
            Effect::Static {
                colour: Colour::rgb(Rgb::new(1, 2, 3)),
            },
        )
        .await
        .expect("set-rgb should ack");

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn set_effect_sends_non_rgb_static_colour_request() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        let request = receive_request(&mut stream).await.1;
        let Request::SetEffect(SetEffectRequest {
            selector,
            effect,
            on_unsupported,
        }) = request
        else {
            panic!("unexpected request: {request:?}");
        };
        assert_eq!(
            selector,
            Selector::Target(TargetId::group("keyboard", "wasd"))
        );
        assert_eq!(on_unsupported, None);
        let Effect::Static { colour } = effect else {
            panic!("unexpected effect: {effect:?}");
        };
        assert_eq!(colour, Colour::cct(4_000));

        send(
            &mut stream,
            &Response {
                status: ResponseStatus::Ack,
            },
        )
        .await
        .expect("send ack");
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    client
        .set_effect(
            TargetId::group("keyboard", "wasd"),
            Effect::Static {
                colour: Colour::cct(4_000),
            },
        )
        .await
        .expect("set-static-rgb should ack");

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "all helper variants share one ordered fake-daemon exchange"
)]
async fn static_colour_helpers_delegate_to_set_effect() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");
    let target = TargetId::device("lamp");
    let collection = CollectionId::new("room");
    let applied = TargetId::device("lamp");
    let denied = TargetId::device("accent");

    let expected = [
        (
            Selector::Target(target.clone()),
            Colour::monochrome(17),
            None,
            false,
        ),
        (
            Selector::Target(target.clone()),
            Colour::rgb(Rgb::new(1, 2, 3)),
            None,
            false,
        ),
        (
            Selector::Target(target.clone()),
            Colour::cct(4_000),
            None,
            false,
        ),
        (
            Selector::Collection(collection.clone()),
            Colour::hsl(20, 30, 40),
            Some(UnsupportedPolicy::Skip),
            true,
        ),
        (
            Selector::Collection(collection.clone()),
            Colour::rgb(Rgb::new(4, 5, 6)),
            Some(UnsupportedPolicy::Reject),
            true,
        ),
        (
            Selector::Collection(collection.clone()),
            Colour::cct(5_000),
            None,
            true,
        ),
    ];

    let expected_applied = applied.clone();
    let expected_denied = denied.clone();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        for (expected_selector, expected_colour, expected_policy, collection) in expected {
            let (id, request) = receive_request(&mut stream).await;
            let Request::SetEffect(SetEffectRequest {
                selector,
                effect,
                on_unsupported,
            }) = request
            else {
                panic!("unexpected request: {request:?}");
            };
            assert_eq!(selector, expected_selector);
            assert_eq!(
                effect,
                Effect::Static {
                    colour: expected_colour
                }
            );
            assert_eq!(on_unsupported, expected_policy);

            let status = if collection {
                ResponseStatus::CollectionApplied {
                    applied: vec![expected_applied.clone()],
                    denied: vec![expected_denied.clone()],
                }
            } else {
                ResponseStatus::Ack
            };
            send_response(&mut stream, id, status).await;
        }
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    client
        .set_colour(target.clone(), Colour::monochrome(17))
        .await
        .expect("set colour");
    client
        .set_rgb(target.clone(), 1, 2, 3)
        .await
        .expect("set RGB");
    client
        .set_cct(target, 4_000)
        .await
        .expect("set colour temperature");

    for outcome in [
        client
            .set_colour_selector(
                Selector::Collection(collection.clone()),
                Colour::hsl(20, 30, 40),
                Some(UnsupportedPolicy::Skip),
            )
            .await
            .expect("set selector colour"),
        client
            .set_rgb_selector(
                Selector::Collection(collection.clone()),
                4,
                5,
                6,
                Some(UnsupportedPolicy::Reject),
            )
            .await
            .expect("set selector RGB"),
        client
            .set_cct_selector(Selector::Collection(collection), 5_000, None)
            .await
            .expect("set selector colour temperature"),
    ] {
        assert_eq!(outcome.applied, vec![applied.clone()]);
        assert_eq!(outcome.denied, vec![denied.clone()]);
    }

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn unknown_state_error_maps_to_typed_error() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        let _: RequestMessage = receive(&mut stream).await.expect("receive request");
        send(
            &mut stream,
            &Response {
                status: ResponseStatus::Error(OperationError {
                    code: ErrorCode::UnknownState,
                    message: "cannot save current state for device0".to_owned(),
                    retry_after_ms: None,
                    applied_targets: Vec::new(),
                }),
            },
        )
        .await
        .expect("send error response");
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    let error = client
        .save_current(TargetId::Device(DeviceId::new("device0")))
        .await
        .expect_err("save-current should fail");

    let Error::UnknownState(message) = error else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(message, "cannot save current state for device0");

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn cancelled_request_does_not_disrupt_the_connection() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        // Leave the first request unanswered so its caller cancels, then
        // prove a later response is routed independently.
        let _: RequestMessage = receive(&mut stream).await.expect("receive request");
        let second: RequestMessage = receive(&mut stream).await.expect("receive request");
        send_response(
            &mut stream,
            second.id,
            ResponseStatus::ServerInfo(luminate_protocol::ServerInfo {
                daemon_name: "luminated".to_owned(),
                daemon_version: "0.1.0".to_owned(),
                protocol_abi_version: PROTOCOL_ABI_VERSION,
            }),
        )
        .await;
    });

    let client = Client::connect_path(&path).await.expect("connect client");

    let cancelled = timeout(Duration::from_millis(50), client.server_info()).await;
    assert!(
        cancelled.is_err(),
        "request should have been cancelled by the timeout"
    );

    let info = client
        .server_info()
        .await
        .expect("a later request should still complete");
    assert_eq!(info.daemon_name, "luminated");

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn concurrent_requests_are_routed_by_id() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        let first: RequestMessage = receive(&mut stream).await.expect("receive request");
        let second: RequestMessage = receive(&mut stream).await.expect("receive request");
        let (server_info_id, devices_id) = match (&first.request, &second.request) {
            (Request::ServerInfo, Request::ListDevices) => (first.id, second.id),
            (Request::ListDevices, Request::ServerInfo) => (second.id, first.id),
            requests => panic!("unexpected requests: {requests:?}"),
        };

        send_response(
            &mut stream,
            devices_id,
            ResponseStatus::Devices(vec![tagged_device("device0")]),
        )
        .await;
        send_response(
            &mut stream,
            server_info_id,
            ResponseStatus::ServerInfo(luminate_protocol::ServerInfo {
                daemon_name: "luminated".to_owned(),
                daemon_version: "0.1.0".to_owned(),
                protocol_abi_version: PROTOCOL_ABI_VERSION,
            }),
        )
        .await;
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    let (info, devices) = tokio::join!(client.server_info(), client.list_devices());
    assert_eq!(info.expect("server info").daemon_name, "luminated");
    let devices = devices.expect("device list");
    assert_eq!(devices[0].physical_tags, ["shape:modular-light-bar"]);
    assert_eq!(devices[0].surfaces[0].physical_tags, ["layout:horizontal"]);
    assert_eq!(
        devices[0].surfaces[0].elements[0].physical_tags,
        ["shape:round", "position:left"]
    );

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn compatible_verdict_with_mismatched_protocol_version_is_rejected() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let advertised = PROTOCOL_ABI_VERSION.wrapping_add(1);
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        // Contradictory handshake: claims compatibility but advertises a
        // protocol ABI version this client does not speak.
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: advertised,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");
    });

    let err = Client::connect_path(&path)
        .await
        .expect_err("client should reject a contradictory handshake");

    let Error::IncompatibleDaemon {
        supported_protocol_abi_version,
        ..
    } = err
    else {
        panic!("unexpected error: {err:?}");
    };
    assert_eq!(supported_protocol_abi_version, advertised);

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test(start_paused = true)]
async fn handshake_times_out_when_daemon_never_sends_hello() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        // Accept the connection and the hello, then never reply. With the
        // test clock paused, tokio auto-advances to the client's I/O
        // deadline instead of waiting the real 10s.
        future::pending::<()>().await;
    });

    let err = Client::connect_path(&path)
        .await
        .expect_err("handshake read should time out");
    assert!(
        matches!(err, Error::Timeout(_)),
        "unexpected error: {err:?}"
    );

    server.abort();
    let _ = fs::remove_file(path);
}

#[tokio::test(start_paused = true)]
async fn timed_out_request_does_not_disrupt_the_connection() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        let first: RequestMessage = receive(&mut stream).await.expect("receive request");
        time::sleep(IO_TIMEOUT + Duration::from_secs(1)).await;
        send_response(&mut stream, first.id, ResponseStatus::Ack).await;

        let second: RequestMessage = receive(&mut stream).await.expect("receive request");
        send_response(
            &mut stream,
            second.id,
            ResponseStatus::ServerInfo(luminate_protocol::ServerInfo {
                daemon_name: "luminated".to_owned(),
                daemon_version: "0.1.0".to_owned(),
                protocol_abi_version: PROTOCOL_ABI_VERSION,
            }),
        )
        .await;
    });

    let client = Client::connect_path(&path).await.expect("connect client");

    let err = client
        .server_info()
        .await
        .expect_err("response read should time out");
    assert!(
        matches!(err, Error::Timeout(_)),
        "unexpected error: {err:?}"
    );

    let info = client
        .server_info()
        .await
        .expect("a later request should still complete");
    assert_eq!(info.daemon_name, "luminated");

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn get_device_preserves_physical_tags_and_missing_values_return_none() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        let (request_id, request) = receive_request(&mut stream).await;
        let Request::GetDevice { id } = request else {
            panic!("unexpected request: {request:?}");
        };
        assert_eq!(id, DeviceId::new("device0"));
        send_response(
            &mut stream,
            request_id,
            ResponseStatus::Device(Some(Box::new(tagged_device("device0")))),
        )
        .await;

        let (request_id, request) = receive_request(&mut stream).await;
        let Request::GetState { device } = request else {
            panic!("unexpected request: {request:?}");
        };
        assert_eq!(device, DeviceId::new("device0"));
        send_response(&mut stream, request_id, ResponseStatus::State(None)).await;

        let (request_id, request) = receive_request(&mut stream).await;
        let Request::GetCollectionState { collection } = request else {
            panic!("unexpected request: {request:?}");
        };
        assert_eq!(collection, CollectionId::new("collection0"));
        send_response(
            &mut stream,
            request_id,
            ResponseStatus::CollectionState(None),
        )
        .await;
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    let device = client
        .get_device(DeviceId::new("device0"))
        .await
        .expect("get_device should succeed");
    let device = device.expect("device should be present");
    assert_eq!(device.physical_tags, ["shape:modular-light-bar"]);
    assert_eq!(device.surfaces[0].physical_tags, ["layout:horizontal"]);
    assert_eq!(
        device.surfaces[0].elements[0].physical_tags,
        ["shape:round", "position:left"]
    );

    let state = client
        .get_state(DeviceId::new("device0"))
        .await
        .expect("get_state should succeed");
    assert!(state.is_none());

    let collection_state = client
        .get_collection_state(CollectionId::new("collection0"))
        .await
        .expect("get_collection_state should succeed");
    assert!(collection_state.is_none());

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn refresh_state_request_sends_wire_request() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        let (request_id, request) = receive_request(&mut stream).await;
        assert!(matches!(request, Request::RefreshState { .. }));
        send_response(&mut stream, request_id, ResponseStatus::Ack).await;
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    client
        .refresh_state(DeviceId::new("device0"))
        .await
        .expect("refresh-state should ack");

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn management_methods_send_typed_wire_requests() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        let (request_id, request) = receive_request(&mut stream).await;
        assert!(matches!(request, Request::GetManagement));
        send_response(
            &mut stream,
            request_id,
            ResponseStatus::ManagementSnapshot(Box::new(ManagementSnapshot {
                revision: 3,
                desired_daemon: empty_daemon_preferences(),
                effective_daemon: empty_daemon_preferences(),
                locked_daemon_settings: Vec::new(),
                plugins: Vec::new(),
            })),
        )
        .await;

        let (request_id, request) = receive_request(&mut stream).await;
        let Request::PatchManagement { patch } = request else {
            panic!("unexpected request: {request:?}");
        };
        assert_eq!(patch.expected_revision, 3);
        assert!(patch.mutations.is_empty());
        send_response(
            &mut stream,
            request_id,
            ResponseStatus::ManagementPatched(ManagementChangeSet {
                revision: 4,
                changes: Vec::new(),
            }),
        )
        .await;
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    let snapshot = client
        .get_management()
        .await
        .expect("get management snapshot");
    assert_eq!(snapshot.revision, 3);

    let changes = client
        .patch_management(ManagementPatch {
            expected_revision: snapshot.revision,
            mutations: Vec::new(),
        })
        .await
        .expect("patch managed configuration");
    assert_eq!(changes.revision, 4);

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

fn empty_daemon_preferences() -> crate::DaemonPreferences {
    crate::DaemonPreferences {
        default_unsupported_policy: None,
        reconciliation_policy: None,
        device_reconciliation: Vec::new(),
        cct_emulation: None,
        prefer_shm: None,
        prefer_client_shm: None,
    }
}

#[tokio::test]
async fn unexpected_response_shapes_are_reported_as_protocol_errors() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        // Deliberately answer a `ServerInfo` request with a mismatched
        // status to exercise the client's "unexpected response" guard.
        let _: RequestMessage = receive(&mut stream).await.expect("receive request");
        send_response(&mut stream, 0, ResponseStatus::Ack).await;

        let _: RequestMessage = receive(&mut stream).await.expect("receive request");
        send_response(&mut stream, 1, ResponseStatus::Ack).await;
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    let error = client
        .server_info()
        .await
        .expect_err("mismatched response should be reported");
    assert!(matches!(error, Error::Protocol(_)));

    let error = client
        .get_device(DeviceId::new("device0"))
        .await
        .expect_err("mismatched response should be reported");
    assert!(matches!(error, Error::Protocol(_)));

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn event_handshake_rejects_a_contradictory_compatible_verdict() {
    let primary_path = unique_socket_path();
    let event_path = event_socket_path(&primary_path);
    let event_listener = UnixListener::bind(&event_path).expect("bind event socket");

    let advertised = EVENT_PROTOCOL_VERSION.wrapping_add(1);
    let event_server = tokio::spawn(async move {
        let (mut stream, _) = event_listener.accept().await.expect("accept subscriber");
        let _: SubscribeHello = receive(&mut stream).await.expect("receive subscribe hello");
        send(
            &mut stream,
            &SubscribeAck {
                compatibility: EventCompatibility::Compatible,
                event_protocol_version: advertised,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send subscribe ack");
    });

    let error = EventSubscription::connect_path_with_ticket(
        &event_path,
        EventTicket::new([3_u8; 32]).expect("valid ticket"),
    )
    .await
    .expect_err("mismatched event protocol version should be rejected");
    let Error::IncompatibleEventSocket {
        supported_event_protocol_version,
        ..
    } = error
    else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(supported_event_protocol_version, advertised);

    event_server.await.expect("event server task");
    let _ = fs::remove_file(event_path);
}

#[tokio::test]
async fn cancelled_event_wait_poisons_the_subscription() {
    let primary_path = unique_socket_path();
    let event_path = event_socket_path(&primary_path);
    let event_listener = UnixListener::bind(&event_path).expect("bind event socket");

    let event_server = tokio::spawn(async move {
        let (mut stream, _) = event_listener.accept().await.expect("accept subscriber");
        let _: SubscribeHello = receive(&mut stream).await.expect("receive subscribe hello");
        send(
            &mut stream,
            &SubscribeAck {
                compatibility: EventCompatibility::Compatible,
                event_protocol_version: EVENT_PROTOCOL_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send subscribe ack");
        // Never send an event; the client's wait is cancelled instead.
        future::pending::<()>().await;
    });

    let mut subscription = EventSubscription::connect_path_with_ticket(
        &event_path,
        EventTicket::new([4_u8; 32]).expect("valid ticket"),
    )
    .await
    .expect("connect subscription");

    let cancelled = timeout(Duration::from_millis(50), subscription.next_event()).await;
    assert!(cancelled.is_err(), "wait should have been cancelled");

    let error = subscription
        .next_event()
        .await
        .expect_err("a poisoned subscription must be replaced, not reused");
    assert!(matches!(error, Error::ConnectionPoisoned));

    event_server.abort();
    let _ = fs::remove_file(event_path);
}

#[tokio::test]
async fn subscribe_with_baseline_registers_before_listing_devices() {
    let primary_path = unique_socket_path();
    let event_path = event_socket_path(&primary_path);
    let primary_listener = UnixListener::bind(&primary_path).expect("bind primary socket");
    let event_listener = UnixListener::bind(&event_path).expect("bind event socket");

    let primary_server = tokio::spawn(async move {
        let (mut stream, _) = primary_listener.accept().await.expect("accept client");
        let _: ClientHello = receive(&mut stream).await.expect("receive client hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");
        let (ticket_id, request) = receive_request(&mut stream).await;
        assert!(matches!(request, Request::IssueEventTicket));
        send_response(
            &mut stream,
            ticket_id,
            ResponseStatus::EventTicket(EventTicket::new([5_u8; 32]).expect("valid event ticket")),
        )
        .await;
        let (baseline_id, request) = receive_request(&mut stream).await;
        assert!(matches!(request, Request::ListDevices));
        send_response(
            &mut stream,
            baseline_id,
            ResponseStatus::Devices(Vec::new()),
        )
        .await;
    });
    let event_server = tokio::spawn(async move {
        let (mut stream, _) = event_listener.accept().await.expect("accept subscriber");
        let _: SubscribeHello = receive(&mut stream).await.expect("receive subscribe hello");
        send(
            &mut stream,
            &SubscribeAck {
                compatibility: EventCompatibility::Compatible,
                event_protocol_version: EVENT_PROTOCOL_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send subscribe ack");
        send(
            &mut stream,
            &Event::TopologyChanged {
                devices: vec![DeviceId::new("new-bulb")],
            },
        )
        .await
        .expect("send topology event");
    });

    let client = Client::connect_path(&primary_path)
        .await
        .expect("connect primary client");
    let (mut subscription, baseline) = client
        .subscribe_with_baseline()
        .await
        .expect("subscribe and fetch baseline");
    assert!(baseline.is_empty());
    assert_eq!(
        subscription.next_event().await.expect("receive event"),
        Event::TopologyChanged {
            devices: vec![DeviceId::new("new-bulb")],
        }
    );

    primary_server.await.expect("primary server task");
    event_server.await.expect("event server task");
    let _ = fs::remove_file(primary_path);
    let _ = fs::remove_file(event_path);
}

#[tokio::test]
async fn restore_and_emission_helpers_send_expected_requests() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");
    let expected = TargetId::Device(DeviceId::new("lamp"));
    let expected_for_server = expected.clone();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        for expected_request in [
            Request::RestoreAppearance {
                target: Selector::Target(expected_for_server.clone()),
            },
            Request::SetEffect(SetEffectRequest {
                selector: Selector::Target(expected_for_server.clone()),
                effect: Effect::Off,
                on_unsupported: None,
            }),
            Request::RestoreAppearance {
                target: Selector::Target(expected_for_server),
            },
        ] {
            let (id, actual) = receive_request(&mut stream).await;
            assert_eq!(
                format!("{actual:?}"),
                format!("{expected_request:?}"),
                "unexpected helper request"
            );
            send_response(&mut stream, id, ResponseStatus::Ack).await;
        }
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    client
        .restore_appearance(expected.clone())
        .await
        .expect("restore appearance");
    client
        .set_emission(expected.clone(), EmissionState::Dark)
        .await
        .expect("set dark");
    client
        .set_emission(expected, EmissionState::Emitting)
        .await
        .expect("set emitting");

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn collection_restore_and_emission_helpers_return_collection_outcomes() {
    let path = unique_socket_path();
    let listener = UnixListener::bind(&path).expect("bind test socket");
    let collection = CollectionId::new("living-room");
    let expected_collection = collection.clone();
    let applied = TargetId::device("lamp");
    let denied = TargetId::device("ceiling");
    let expected_applied = applied.clone();
    let expected_denied = denied.clone();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        let _: ClientHello = receive(&mut stream).await.expect("receive hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("send daemon hello");

        for expected_request in [
            Request::RestoreAppearance {
                target: Selector::Collection(expected_collection.clone()),
            },
            Request::SetEffect(SetEffectRequest {
                selector: Selector::Collection(expected_collection),
                effect: Effect::Off,
                on_unsupported: None,
            }),
        ] {
            let (id, actual) = receive_request(&mut stream).await;
            assert_eq!(
                format!("{actual:?}"),
                format!("{expected_request:?}"),
                "unexpected collection helper request"
            );
            send_response(
                &mut stream,
                id,
                ResponseStatus::CollectionApplied {
                    applied: vec![expected_applied.clone()],
                    denied: vec![expected_denied.clone()],
                },
            )
            .await;
        }
    });

    let client = Client::connect_path(&path).await.expect("connect client");
    for outcome in [
        client
            .restore_appearance_selector(Selector::Collection(collection.clone()))
            .await
            .expect("restore collection appearance"),
        client
            .set_emission_selector(Selector::Collection(collection), EmissionState::Dark)
            .await
            .expect("set collection dark"),
    ] {
        assert_eq!(outcome.applied, vec![applied.clone()]);
        assert_eq!(outcome.denied, vec![denied.clone()]);
    }

    server.await.expect("server task");
    let _ = fs::remove_file(path);
}
