// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::pin::Pin;

use luminate_core::policy::{
    InMemoryPolicyStore, ManagedAccessPolicy, PolicyDocument, PolicyDocumentSource, PolicyRevision,
    PolicyStore, PrincipalId, RuntimeAccessPolicy,
};
use luminate_protocol::{EventTicket, ManagementChangeSet};
use tokio::io::{AsyncWriteExt as _, DuplexStream, duplex};

use super::super::tests_support::{
    discarding_rescans, empty_plugin_manager, remove_request_test_dir, request_test_context,
    test_management_read_state, test_policy, test_principal,
};
use super::super::*;
use super::*;

async fn authenticate_peer(client: &mut DuplexStream) {
    framing::send(
        client,
        &AuthenticationRequest {
            authentication: luminate_protocol::Authentication::Peer,
            scope: None,
        },
    )
    .await
    .expect("send peer authentication");
    let response: AuthenticationResponse = framing::receive(client)
        .await
        .expect("receive authentication response");
    assert!(matches!(
        response,
        AuthenticationResponse::Authenticated { .. }
    ));
}

fn event_authentication() -> (AuthenticationService, EventTicket) {
    let authentication = AuthenticationService::default();
    let ticket = authentication.mint_event_ticket(&test_principal());
    (authentication, ticket)
}

struct EventPolicy {
    visible: device::DeviceId,
    manage_plugins: bool,
}

impl AuthorizationPolicy for EventPolicy {
    fn authorize<'a>(
        &'a self,
        _principal: &'a Principal,
        operation: Operation,
        resources: &'a [Resource],
    ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        let allowed = match operation {
            Operation::Observe | Operation::Control => {
                !resources.is_empty()
                    && resources
                        .iter()
                        .all(|resource| resource.device_id == self.visible)
            }
            Operation::ManagePlugins => self.manage_plugins && resources.is_empty(),
            Operation::Refresh
            | Operation::HardwareAdministration
            | Operation::DaemonAdministration
            | Operation::ManagePolicy
            | Operation::ManageAuthentication
            | Operation::AdministerFrontend
            | Operation::CreateCollection
            | Operation::DestroyCollection
            | Operation::ModifyCollection
            | Operation::AdministerCollections
            | Operation::CreateScene
            | Operation::ModifyScene
            | Operation::DestroyScene
            | Operation::AdministerScenes => false,
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
        "event-test-policy"
    }
}

#[tokio::test]
async fn event_filter_uses_each_events_semantic_operation_and_resources() {
    let (state, manager, _mutations, runtime_dir) = request_test_context("event-authorization");
    let visible = device::DeviceId::new("request-device");
    let hidden = device::DeviceId::new("hidden-device");
    let policy = EventPolicy {
        visible: visible.clone(),
        manage_plugins: false,
    };
    let principal = test_principal();

    let topology = filter_event(
        Event::TopologyChanged {
            devices: vec![hidden.clone(), visible.clone()],
        },
        &state,
        &manager,
        &policy,
        &principal,
    )
    .await;
    assert_eq!(
        topology,
        Some(Event::TopologyChanged {
            devices: vec![visible.clone()]
        })
    );

    let full_refresh = filter_event(
        Event::StateChanged {
            devices: Vec::new(),
        },
        &state,
        &manager,
        &policy,
        &principal,
    )
    .await;
    assert_eq!(full_refresh, None);

    let configuration = filter_event(
        Event::ConfigurationChanged {
            changes: ManagementChangeSet {
                revision: 1,
                changes: Vec::new(),
            },
        },
        &state,
        &manager,
        &policy,
        &principal,
    )
    .await;
    assert_eq!(configuration, None);

    let visible_stream = filter_event(
        Event::ShmStreamEnded {
            target: TargetId::Device(visible),
            generation: 7,
        },
        &state,
        &manager,
        &policy,
        &principal,
    )
    .await;
    assert!(matches!(
        visible_stream,
        Some(Event::ShmStreamEnded { generation: 7, .. })
    ));

    let hidden_stream = filter_event(
        Event::ShmStreamEnded {
            target: TargetId::Device(hidden),
            generation: 8,
        },
        &state,
        &manager,
        &policy,
        &principal,
    )
    .await;
    assert_eq!(hidden_stream, None);

    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn configuration_events_require_resource_free_plugin_management() {
    let (state, manager, _mutations, runtime_dir) =
        request_test_context("event-configuration-authorization");
    let policy = EventPolicy {
        visible: device::DeviceId::new("request-device"),
        manage_plugins: true,
    };

    let event = Event::ConfigurationChanged {
        changes: ManagementChangeSet {
            revision: 1,
            changes: Vec::new(),
        },
    };
    let filtered = filter_event(event.clone(), &state, &manager, &policy, &test_principal()).await;
    assert_eq!(filtered, Some(event));

    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn stale_event_publication_requests_a_full_topology_refresh() {
    let (state, manager, _mutations, runtime_dir) = request_test_context("stale-event-topology");
    let topology_generation = state.lock().await.topology_generation();
    let published = PublishedEvent {
        event: Event::StateChanged {
            devices: vec![device::DeviceId::new("request-device")],
        },
        topology_generation,
    };
    state
        .lock()
        .await
        .replace_devices_preserving_withdrawn_state(Vec::new());

    let filtered = filter_event(
        published,
        &state,
        &manager,
        test_policy().as_ref(),
        &test_principal(),
    )
    .await;

    assert_eq!(filtered, Some(Event::ResyncRequired));
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn idle_connection_is_closed_after_the_idle_timeout() {
    let (state, manager, mutations, runtime_dir) = request_test_context("idle-timeout");
    let (server, client) = duplex(64 * 1024);
    let mut server: Connection = Box::new(server);
    let owned_streams = sync::Mutex::new(Vec::new());
    let mut ping_limiter = PingLimiter::new();
    let mut revocation = None;

    let dependencies = ConnectionDependencies {
        state: Arc::clone(&state),
        plugin_manager: Arc::clone(&manager),
        management: test_management_read_state(),
        mutations: mutations.clone(),
        rescans: discarding_rescans(),
        principal: test_principal(),
        policy: test_policy(),
        authentication: AuthenticationService::default(),
        access_administration: None,
    };
    let result = connection_request_loop(
        &mut server,
        &dependencies,
        &owned_streams,
        &mut ping_limiter,
        &mut revocation,
        &EventTicketContext {
            authentication_expiry: None,
            session_scope: None,
            subject: None,
            frontend_actor: None,
            credential_id: None,
            frontend_credential_id: None,
        },
        Duration::from_millis(20),
    )
    .await;

    assert!(
        result.is_ok(),
        "an idle-timed-out connection should close cleanly, got {result:?}"
    );
    drop(client);
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn primary_connection_handshake_and_read_requests_round_trip() {
    let (state, manager, mutations, runtime_dir) = request_test_context("primary-round-trip");
    let (server, mut client) = duplex(64 * 1024);
    let handler = tokio::spawn(handle_connection(
        Box::new(server),
        ConnectionDependencies {
            state,
            plugin_manager: manager,
            management: test_management_read_state(),
            mutations,
            rescans: discarding_rescans(),
            principal: test_principal(),
            policy: test_policy(),
            authentication: AuthenticationService::default(),
            access_administration: None,
        },
    ));

    framing::send(&mut client, &ClientHello::new("test-client", "1"))
        .await
        .expect("send client hello");
    let hello: DaemonHello = framing::receive(&mut client)
        .await
        .expect("receive daemon hello");
    assert!(matches!(hello.compatibility, Compatibility::Compatible));
    authenticate_peer(&mut client).await;

    let requests = [
        Request::ServerInfo,
        Request::ListDevices,
        Request::GetDevice {
            id: device::DeviceId::new("request-device"),
        },
        Request::GetDevice {
            id: device::DeviceId::new("missing"),
        },
        Request::GetState {
            device: device::DeviceId::new("request-device"),
        },
    ];
    for (id, request) in requests.into_iter().enumerate() {
        framing::send(
            &mut client,
            &RequestMessage {
                id: id as u64,
                request,
            },
        )
        .await
        .expect("send request");
        let response: ResponseMessage = framing::receive(&mut client)
            .await
            .expect("receive response");
        assert_eq!(response.id, id as u64);
        assert!(!matches!(
            response.response.status,
            ResponseStatus::Error { .. }
        ));
    }

    drop(client);
    handler
        .await
        .expect("join primary handler")
        .expect("clean EOF should end connection");
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "This process-boundary scenario covers the complete access-administration lifecycle."
)]
async fn authenticated_connection_administers_policy_and_tokens() {
    let (state, manager, mutations, runtime_dir) = request_test_context("access-administration");
    let authentication = AuthenticationService::default();
    let source = PolicyDocumentSource {
        revision: PolicyRevision(0),
        roles: BTreeMap::new(),
        bindings: Vec::new(),
    };
    let document = PolicyDocument::new(source).expect("valid policy");
    let store: Arc<dyn PolicyStore> = Arc::new(InMemoryPolicyStore::new(document.clone()));
    let policy: Arc<dyn ManagedAccessPolicy> = Arc::new(
        RuntimeAccessPolicy::load(store, document)
            .await
            .expect("load policy"),
    );
    let administration = Arc::new(dispatch::AccessAdministration {
        policy,
        authentication: authentication.clone(),
        audit: Arc::new(NullSink),
        frontend_actors: HashSet::from([test_principal().rate_limit_key()]),
    });
    let (server, mut client) = duplex(64 * 1024);
    let handler = tokio::spawn(handle_connection(
        Box::new(server),
        ConnectionDependencies {
            state,
            plugin_manager: manager,
            management: test_management_read_state(),
            mutations,
            rescans: discarding_rescans(),
            principal: test_principal(),
            policy: test_policy(),
            authentication,
            access_administration: Some(administration),
        },
    ));

    framing::send(&mut client, &ClientHello::new("test-client", "1"))
        .await
        .expect("send client hello");
    let _: DaemonHello = framing::receive(&mut client)
        .await
        .expect("receive daemon hello");
    authenticate_peer(&mut client).await;

    let replacement = PolicyDocumentSource {
        revision: PolicyRevision(1),
        roles: BTreeMap::new(),
        bindings: Vec::new(),
    };
    for (id, (request, expect_error)) in [
        (Request::GetAccessPolicy, false),
        (
            Request::ReplaceAccessPolicy {
                expected: PolicyRevision(0),
                replacement: replacement.clone(),
            },
            false,
        ),
        (
            Request::ReplaceAccessPolicy {
                expected: PolicyRevision(0),
                replacement,
            },
            true,
        ),
        (
            Request::CreateToken {
                id: "front-end".to_owned(),
                subject: PrincipalId::new("local", "alice").expect("valid principal"),
                expires_at: None,
            },
            false,
        ),
        (
            Request::RotateToken {
                id: "front-end".to_owned(),
                expires_at: None,
            },
            false,
        ),
        (
            Request::CreateAttestation {
                name: "front-end-caller".to_owned(),
                subject: PrincipalId::new("local", "bob").expect("valid principal"),
                verified_groups: Vec::new(),
                expires_at: None,
            },
            false,
        ),
        (Request::ListTokens, false),
        (Request::ListAttestations, false),
        (
            Request::RevokeAttestation {
                name: "front-end-caller".to_owned(),
            },
            false,
        ),
        (
            Request::RevokeAttestation {
                name: "front-end-caller".to_owned(),
            },
            true,
        ),
        (
            Request::RevokeToken {
                id: "front-end".to_owned(),
            },
            false,
        ),
        (
            Request::RevokeToken {
                id: "front-end".to_owned(),
            },
            true,
        ),
        (
            Request::RotateToken {
                id: "missing".to_owned(),
                expires_at: None,
            },
            true,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        framing::send(
            &mut client,
            &RequestMessage {
                id: id as u64,
                request,
            },
        )
        .await
        .expect("send administration request");
        let response: ResponseMessage = framing::receive(&mut client)
            .await
            .expect("receive administration response");
        assert_eq!(response.id, id as u64);
        assert_eq!(
            matches!(response.response.status, ResponseStatus::Error(_)),
            expect_error,
            "unexpected administration response: {:?}",
            response.response.status
        );
    }

    drop(client);
    handler.await.expect("join handler").expect("clean close");
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn ping_bypasses_authorization_and_is_rate_limited_per_connection() {
    let (state, manager, mutations, runtime_dir) = request_test_context("ping");
    let (server, mut client) = duplex(64 * 1024);
    let handler = tokio::spawn(handle_connection(
        Box::new(server),
        ConnectionDependencies {
            state,
            plugin_manager: manager,
            management: test_management_read_state(),
            mutations,
            rescans: discarding_rescans(),
            principal: test_principal(),
            policy: Arc::new(EventPolicy {
                visible: device::DeviceId::new("nothing"),
                manage_plugins: false,
            }),
            authentication: AuthenticationService::default(),
            access_administration: None,
        },
    ));

    framing::send(&mut client, &ClientHello::new("ping-test", "1"))
        .await
        .expect("send client hello");
    let _: DaemonHello = framing::receive(&mut client)
        .await
        .expect("receive daemon hello");
    authenticate_peer(&mut client).await;

    for id in 0..u64::from(PING_BUCKET_CAPACITY) {
        framing::send(
            &mut client,
            &RequestMessage {
                id,
                request: Request::Ping,
            },
        )
        .await
        .expect("send permitted ping");
        let response: ResponseMessage = framing::receive(&mut client)
            .await
            .expect("receive permitted ping");
        assert_eq!(response.id, id);
        assert!(matches!(response.response.status, ResponseStatus::Ack));
    }

    let id = u64::from(PING_BUCKET_CAPACITY);
    framing::send(
        &mut client,
        &RequestMessage {
            id,
            request: Request::Ping,
        },
    )
    .await
    .expect("send rate-limited ping");
    let response: ResponseMessage = framing::receive(&mut client)
        .await
        .expect("receive rate-limited ping");
    assert!(matches!(
        response.response.status,
        ResponseStatus::Error(OperationError {
            code: ErrorCode::RateLimited,
            retry_after_ms: Some(retry_after_ms),
            ..
        }) if retry_after_ms > 0
    ));

    framing::send(
        &mut client,
        &RequestMessage {
            id: id + 1,
            request: Request::ServerInfo,
        },
    )
    .await
    .expect("send authorized server info");
    let response: ResponseMessage = framing::receive(&mut client)
        .await
        .expect("receive denied server info");
    assert!(matches!(
        response.response.status,
        ResponseStatus::Error(OperationError {
            code: ErrorCode::PermissionDenied,
            ..
        })
    ));

    drop(client);
    handler
        .await
        .expect("join ping handler")
        .expect("clean EOF should end connection");
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn primary_connection_rejects_incompatible_and_malformed_clients() {
    let (state, manager, mutations, runtime_dir) = request_test_context("primary-errors");
    let (server, mut client) = duplex(64 * 1024);
    let handler = tokio::spawn(handle_connection(
        Box::new(server),
        ConnectionDependencies {
            state: Arc::clone(&state),
            plugin_manager: Arc::clone(&manager),
            management: test_management_read_state(),
            mutations: mutations.clone(),
            rescans: discarding_rescans(),
            principal: test_principal(),
            policy: test_policy(),
            authentication: AuthenticationService::default(),
            access_administration: None,
        },
    ));
    let hello = ClientHello {
        protocol_abi_version: PROTOCOL_ABI_VERSION + 1,
        client_name: "future-client".to_owned(),
        client_version: "1".to_owned(),
    };
    framing::send(&mut client, &hello)
        .await
        .expect("send incompatible hello");
    let reply: DaemonHello = framing::receive(&mut client)
        .await
        .expect("receive rejection");
    assert!(matches!(
        reply.compatibility,
        Compatibility::Incompatible { .. }
    ));
    handler
        .await
        .expect("join incompatible handler")
        .expect("incompatible client closes cleanly");

    let (server, mut client) = duplex(64 * 1024);
    let handler = tokio::spawn(handle_connection(
        Box::new(server),
        ConnectionDependencies {
            state,
            plugin_manager: manager,
            management: test_management_read_state(),
            mutations,
            rescans: discarding_rescans(),
            principal: test_principal(),
            policy: test_policy(),
            authentication: AuthenticationService::default(),
            access_administration: None,
        },
    ));
    framing::send(&mut client, &ClientHello::new("malformed-client", "1"))
        .await
        .expect("send compatible hello");
    let _: DaemonHello = framing::receive(&mut client)
        .await
        .expect("receive compatible hello");
    authenticate_peer(&mut client).await;
    client.write_u32(1).await.expect("write malformed length");
    client
        .write_u8(u8::MAX)
        .await
        .expect("write malformed CBOR");
    client.flush().await.expect("flush malformed request");
    let error = handler
        .await
        .expect("join malformed handler")
        .expect_err("malformed request must end connection");
    assert!(error.to_string().contains("CBOR"));
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn event_subscription_negotiates_and_forwards_dirty_events() {
    let state = Arc::new(Mutex::new(DaemonState::default()));
    let (events, _) = EventPublisher::test_channel(&state, 4);
    let (server, mut client) = duplex(64 * 1024);
    let (authentication, ticket) = event_authentication();
    let handler = tokio::spawn(handle_event_subscription(
        Box::new(server),
        events.clone(),
        state,
        empty_plugin_manager(),
        test_policy(),
        test_principal(),
        authentication,
        None,
    ));
    framing::send(
        &mut client,
        &SubscribeHello::new("event-client", "1", ticket),
    )
    .await
    .expect("send subscribe hello");
    let ack: SubscribeAck = framing::receive(&mut client)
        .await
        .expect("receive subscribe ack");
    assert!(matches!(ack.compatibility, EventCompatibility::Compatible));

    let device = device::DeviceId::new("changed");
    events
        .send(Event::TopologyChanged {
            devices: vec![device.clone()],
        })
        .expect("publish topology event");
    let event: Event = framing::receive(&mut client)
        .await
        .expect("receive topology event");
    assert!(matches!(
        event,
        Event::TopologyChanged { devices } if devices == vec![device.clone()]
    ));
    events
        .send(Event::StateChanged {
            devices: vec![device.clone()],
        })
        .expect("publish state event");
    let event: Event = framing::receive(&mut client)
        .await
        .expect("receive state event");
    assert!(matches!(
        event,
        Event::StateChanged { devices } if devices == vec![device]
    ));

    drop(client);
    timeout(Duration::from_secs(1), handler)
        .await
        .expect("disconnected event handler should stop")
        .expect("join event handler")
        .expect("peer EOF should close the subscription cleanly");
}

#[tokio::test]
async fn event_subscription_projects_devices_through_the_connected_principal() {
    let (state, manager, _mutations, runtime_dir) =
        request_test_context("event-subscription-authorization");
    let visible = device::DeviceId::new("request-device");
    let hidden = device::DeviceId::new("hidden-device");
    let policy: Arc<dyn AuthorizationPolicy> = Arc::new(EventPolicy {
        visible: visible.clone(),
        manage_plugins: false,
    });
    let (events, _) = EventPublisher::test_channel(&state, 4);
    let (server, mut client) = duplex(64 * 1024);
    let (authentication, ticket) = event_authentication();
    let handler = tokio::spawn(handle_event_subscription(
        Box::new(server),
        events.clone(),
        state,
        manager,
        policy,
        test_principal(),
        authentication,
        None,
    ));
    framing::send(
        &mut client,
        &SubscribeHello::new("restricted-event-client", "1", ticket),
    )
    .await
    .expect("send subscribe hello");
    let ack: SubscribeAck = framing::receive(&mut client)
        .await
        .expect("receive subscribe ack");
    assert!(matches!(ack.compatibility, EventCompatibility::Compatible));

    events
        .send(Event::TopologyChanged {
            devices: vec![hidden, visible.clone()],
        })
        .expect("publish mixed-visibility topology event");
    let event: Event = framing::receive(&mut client)
        .await
        .expect("receive projected topology event");
    assert_eq!(
        event,
        Event::TopologyChanged {
            devices: vec![visible]
        }
    );

    drop(client);
    timeout(Duration::from_secs(1), handler)
        .await
        .expect("disconnected event handler should stop")
        .expect("join event handler")
        .expect("peer EOF should close the subscription cleanly");
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn event_subscription_rejects_protocol_mismatch() {
    let state = Arc::new(Mutex::new(DaemonState::default()));
    let (events, _) = EventPublisher::test_channel(&state, 1);
    let (server, mut client) = duplex(64 * 1024);
    let (authentication, ticket) = event_authentication();
    let handler = tokio::spawn(handle_event_subscription(
        Box::new(server),
        events,
        state,
        empty_plugin_manager(),
        test_policy(),
        test_principal(),
        authentication,
        None,
    ));
    let hello = SubscribeHello {
        event_protocol_version: EVENT_PROTOCOL_VERSION + 1,
        client_name: "future-event-client".to_owned(),
        client_version: "1".to_owned(),
        ticket,
    };
    framing::send(&mut client, &hello)
        .await
        .expect("send mismatched event hello");
    let ack: SubscribeAck = framing::receive(&mut client)
        .await
        .expect("receive mismatch ack");
    assert!(matches!(
        ack.compatibility,
        EventCompatibility::Incompatible { .. }
    ));
    handler
        .await
        .expect("join mismatch handler")
        .expect("mismatch closes cleanly");
}

#[tokio::test]
async fn event_subscription_rejects_unexpected_client_data() {
    let state = Arc::new(Mutex::new(DaemonState::default()));
    let (events, _) = EventPublisher::test_channel(&state, 1);
    let (server, mut client) = duplex(64 * 1024);
    let (authentication, ticket) = event_authentication();
    let handler = tokio::spawn(handle_event_subscription(
        Box::new(server),
        events,
        state,
        empty_plugin_manager(),
        test_policy(),
        test_principal(),
        authentication,
        None,
    ));
    framing::send(
        &mut client,
        &SubscribeHello::new("chatty-client", "1", ticket),
    )
    .await
    .expect("send subscribe hello");
    let _: SubscribeAck = framing::receive(&mut client)
        .await
        .expect("receive subscribe ack");
    client.write_u8(1).await.expect("send unexpected byte");
    let error = timeout(Duration::from_secs(1), handler)
        .await
        .expect("chatty subscriber should stop")
        .expect("join event handler")
        .expect_err("subscriber input is a protocol violation");
    assert!(error.to_string().contains("unexpected data"));
}

#[tokio::test]
async fn lagged_event_subscription_requests_a_full_refresh() {
    let state = Arc::new(Mutex::new(DaemonState::default()));
    let (events, _) = EventPublisher::test_channel(&state, 1);
    let (server, mut client) = duplex(64 * 1024);
    let (authentication, ticket) = event_authentication();
    let handler = tokio::spawn(handle_event_subscription(
        Box::new(server),
        events.clone(),
        state,
        empty_plugin_manager(),
        test_policy(),
        test_principal(),
        authentication,
        None,
    ));
    framing::send(
        &mut client,
        &SubscribeHello::new("slow-client", "1", ticket),
    )
    .await
    .expect("send subscribe hello");
    let _: SubscribeAck = framing::receive(&mut client)
        .await
        .expect("receive subscribe ack");

    // No await points between these sends: the capacity-one receiver must
    // observe a lag before the handler can run again.
    events
        .send(Event::TopologyChanged {
            devices: vec![device::DeviceId::new("old")],
        })
        .expect("send old topology event");
    events
        .send(Event::TopologyChanged {
            devices: vec![device::DeviceId::new("new")],
        })
        .expect("send replacement topology event");
    let event: Event = framing::receive(&mut client)
        .await
        .expect("receive refresh marker");
    assert_eq!(event, Event::ResyncRequired);

    handler.abort();
    let _ = handler.await;
}
