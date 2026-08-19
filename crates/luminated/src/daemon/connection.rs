// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Per-connection handshake, request loop, and event subscription handling.

use tokio::io::AsyncReadExt as _;
use tokio::time::sleep;

use std::future::pending;
use std::time::SystemTime;

use luminate_core::policy::{Principal as SessionPrincipal, SessionScope};

use super::authz::RequestAuthorizer;
#[cfg(test)]
use super::dispatch::PING_BUCKET_CAPACITY;
use super::dispatch::{ConnectionDependencies, DispatchContext, PingLimiter, dispatch_request};
use super::{
    Arc, AuthenticationRequest, AuthenticationResponse, AuthenticationService, AuthorizationPolicy,
    ClientHello, Compatibility, DaemonHello, DaemonState, Duration, EVENT_PROTOCOL_VERSION, Event,
    EventCompatibility, EventPublisher, FramingError, HANDSHAKE_TIMEOUT, HashSet,
    IDLE_CONNECTION_TIMEOUT, Mutex, Operation, PROTOCOL_ABI_VERSION, PluginManager, Principal,
    PublishedEvent, Request, RequestMessage, Response, ResponseMessage, ResponseStatus,
    SubscribeAck, SubscribeHello, TargetId, broadcast, device, env, framing, io, mem, panic, slice,
    sync, timeout,
};
use crate::audit::AuditedPolicy;
use crate::authentication::{ConsumedTicket, EphemeralCapacityError};
use crate::authorization::{AuthenticatedSubjectPolicy, ScopedPolicy};
use luminate_protocol::{ErrorCode, OperationError};

use luminate_platform::transport::Connection;

struct EventTicketContext {
    authentication_expiry: Option<SystemTime>,
    session_scope: Option<SessionScope>,
    subject: Option<SessionPrincipal>,
    frontend_actor: Option<SessionPrincipal>,
    credential_id: Option<String>,
    frontend_credential_id: Option<String>,
}

fn authenticated_policy(
    administration: &super::dispatch::AccessAdministration,
    subject: SessionPrincipal,
    frontend_actor: Option<SessionPrincipal>,
    credential_id: Option<String>,
    frontend_credential_id: Option<String>,
) -> Arc<dyn AuthorizationPolicy> {
    let mut policy = AuthenticatedSubjectPolicy::new(Arc::clone(&administration.policy), subject)
        .with_credential_id(credential_id);
    if let Some(frontend_actor) = frontend_actor {
        policy = policy.with_frontend_actor(frontend_actor, frontend_credential_id);
    }
    Arc::new(AuditedPolicy::new(
        Arc::new(policy),
        Arc::clone(&administration.audit),
    ))
}

#[allow(
    clippy::too_many_arguments,
    reason = "The event accept path passes its established connection dependencies plus the phase-4 administration service."
)]
pub(super) async fn handle_event_subscription(
    mut stream: Connection,
    events: EventPublisher,
    state: Arc<Mutex<DaemonState>>,
    plugin_manager: Arc<PluginManager>,
    mut policy: Arc<dyn AuthorizationPolicy>,
    principal: Principal,
    authentication: AuthenticationService,
    access_administration: Option<Arc<super::dispatch::AccessAdministration>>,
) -> anyhow::Result<()> {
    let mut pending_event = None;
    let hello: SubscribeHello = timeout(HANDSHAKE_TIMEOUT, framing::receive(&mut stream))
        .await
        .map_err(|_| anyhow::anyhow!("event subscription handshake timed out"))??;

    let ticket = authentication
        .consume(&hello.ticket, &principal)
        .map_err(|()| {
            anyhow::anyhow!(
                "event subscription ticket is invalid, expired, used, or actor-mismatched"
            )
        })?;
    let ConsumedTicket {
        scope,
        subject,
        frontend_actor,
        credential_id,
        frontend_credential_id,
        mut revocation,
        authentication_expiry,
    } = ticket;
    if let Some(subject) = subject {
        let Some(administration) = access_administration else {
            anyhow::bail!("authenticated-subject event policy is unavailable");
        };
        policy = authenticated_policy(
            &administration,
            subject,
            frontend_actor,
            credential_id,
            frontend_credential_id,
        );
    }
    if let Some(scope) = scope {
        policy = Arc::new(ScopedPolicy::new(policy, scope));
    }

    tracing::info!(
        client_name = %hello.client_name,
        client_version = %hello.client_version,
        event_protocol_version = hello.event_protocol_version,
        rate_limit_key = ?principal.rate_limit_key(),
        "event subscriber connected"
    );

    // Register before acknowledging the handshake. A client that fetches its
    // baseline after receiving this ack cannot miss a topology change between
    // subscription and baseline; at worst it receives a redundant dirty bit.
    let mut receiver = events.subscribe();
    let compatible = hello.event_protocol_version == EVENT_PROTOCOL_VERSION;
    let ack = SubscribeAck {
        compatibility: if compatible {
            EventCompatibility::Compatible
        } else {
            EventCompatibility::Incompatible {
                supported_event_protocol_version: EVENT_PROTOCOL_VERSION,
                reason: Some("event protocol version mismatch".to_owned()),
            }
        },
        event_protocol_version: EVENT_PROTOCOL_VERSION,
        daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    timeout(framing::IO_TIMEOUT, framing::send(&mut stream, &ack)).await??;
    if !compatible {
        return Ok(());
    }

    // A boxed `Connection` has no `readable()`/`try_read()` (those are
    // inherent, non-trait methods on the concrete socket/pipe types), so
    // idle-disconnect detection uses a plain generic read instead: its
    // future only resolves on real progress (data or EOF), so there is no
    // spurious-wakeup/`WouldBlock` case left to handle, unlike the
    // readiness-poll-then-non-blocking-read dance this replaces.
    let mut probe = [0_u8; 1];
    loop {
        let event = if let Some(event) = pending_event.take() {
            event
        } else {
            tokio::select! {
                event_result = receiver.recv() => match event_result {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "event subscriber lagged; requesting full refresh");
                        let topology_generation = state.lock().await.topology_generation();
                        PublishedEvent {
                            event: Event::ResyncRequired,
                            topology_generation,
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                },
                read_result = stream.read(&mut probe) => {
                    match read_result {
                        Ok(0) => return Ok(()),
                        Ok(_) => anyhow::bail!("event subscriber sent unexpected data"),
                        Err(error) => return Err(error.into()),
                    }
                },
                () = wait_for_revocation(&mut revocation) => return Ok(()),
                () = wait_for_expiry(authentication_expiry) => return Ok(()),
            }
        };
        if let Some(event) =
            filter_event(event, &state, &plugin_manager, policy.as_ref(), &principal).await
        {
            timeout(framing::IO_TIMEOUT, framing::send(&mut stream, &event)).await??;
        }
    }
}

async fn filter_event(
    published: impl Into<PublishedEvent>,
    state: &Arc<Mutex<DaemonState>>,
    plugin_manager: &PluginManager,
    policy: &dyn AuthorizationPolicy,
    principal: &Principal,
) -> Option<Event> {
    let published = published.into();
    if !state
        .lock()
        .await
        .topology_generation_is(published.topology_generation)
    {
        return Some(Event::ResyncRequired);
    }
    let topology_generation = published.topology_generation;
    let filtered = match published.event {
        Event::ResyncRequired => Some(Event::ResyncRequired),
        Event::TopologyChanged { devices } => {
            filter_device_event(
                devices,
                state,
                plugin_manager,
                policy,
                principal,
                |devices| Event::TopologyChanged { devices },
            )
            .await
        }
        Event::StateChanged { devices } => {
            filter_device_event(
                devices,
                state,
                plugin_manager,
                policy,
                principal,
                |devices| Event::StateChanged { devices },
            )
            .await
        }
        event @ Event::ConfigurationChanged { .. } => {
            let auth = RequestAuthorizer::new(
                state,
                plugin_manager,
                policy,
                principal,
                Operation::ManagePlugins,
            );
            auth.authorize(&[]).await.is_ok().then_some(event)
        }
        event @ Event::ScenesChanged => {
            let auth = RequestAuthorizer::new(
                state,
                plugin_manager,
                policy,
                principal,
                Operation::Observe,
            );
            auth.authorize(&[]).await.is_ok().then_some(event)
        }
        event @ Event::TransitionsChanged { .. } => {
            let auth = RequestAuthorizer::new(
                state,
                plugin_manager,
                policy,
                principal,
                Operation::Control,
            );
            auth.authorize(&[]).await.is_ok().then_some(event)
        }
        Event::ShmStreamEnded { target, generation } => {
            let auth = RequestAuthorizer::new(
                state,
                plugin_manager,
                policy,
                principal,
                Operation::Control,
            );
            auth.authorize(slice::from_ref(target.device_id()))
                .await
                .is_ok()
                .then_some(Event::ShmStreamEnded { target, generation })
        }
    };
    if state
        .lock()
        .await
        .topology_generation_is(topology_generation)
    {
        filtered
    } else {
        Some(Event::TopologyChanged {
            devices: Vec::new(),
        })
    }
}

async fn filter_device_event(
    devices: Vec<device::DeviceId>,
    state: &Arc<Mutex<DaemonState>>,
    plugin_manager: &PluginManager,
    policy: &dyn AuthorizationPolicy,
    principal: &Principal,
    make_event: impl FnOnce(Vec<device::DeviceId>) -> Event,
) -> Option<Event> {
    let auth = RequestAuthorizer::new(state, plugin_manager, policy, principal, Operation::Observe);
    if devices.is_empty() {
        return auth
            .authorize(&[])
            .await
            .is_ok()
            .then(|| make_event(devices));
    }

    let (visible, generation) = auth.filter_devices(&devices).await.ok()?;
    auth.topology_is_current(generation)
        .await
        .then_some(())
        .and_then(|()| (!visible.is_empty()).then(|| make_event(visible)))
}

#[allow(
    clippy::too_many_lines,
    reason = "the negotiated handshake and connection-owned cleanup form one ordered lifecycle"
)]
pub(super) async fn handle_connection(
    mut stream: Connection,
    mut dependencies: ConnectionDependencies,
) -> anyhow::Result<()> {
    let hello: ClientHello = timeout(HANDSHAKE_TIMEOUT, framing::receive(&mut stream))
        .await
        .map_err(|_| anyhow::anyhow!("client handshake timed out"))??;

    tracing::info!(
        client_name = %hello.client_name,
        client_version = %hello.client_version,
        protocol_abi_version = hello.protocol_abi_version,
        "client connected"
    );

    let daemon_hello = if hello.protocol_abi_version == PROTOCOL_ABI_VERSION {
        DaemonHello {
            compatibility: Compatibility::Compatible,
            protocol_abi_version: PROTOCOL_ABI_VERSION,
            daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    } else {
        DaemonHello {
            compatibility: Compatibility::Incompatible {
                supported_protocol_abi_version: PROTOCOL_ABI_VERSION,
                reason: Some("protocol ABI version mismatch".to_owned()),
            },
            protocol_abi_version: PROTOCOL_ABI_VERSION,
            daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    };

    let compatible = matches!(daemon_hello.compatibility, Compatibility::Compatible);
    timeout(
        framing::IO_TIMEOUT,
        framing::send(&mut stream, &daemon_hello),
    )
    .await??;

    if !compatible {
        return Ok(());
    }

    let authentication: AuthenticationRequest =
        timeout(HANDSHAKE_TIMEOUT, framing::receive(&mut stream))
            .await
            .map_err(|_| anyhow::anyhow!("client authentication timed out"))??;
    let session_scope = authentication.scope.clone();
    let (authentication_response, mut revocation, frontend_actor, frontend_credential_id) =
        dependencies
            .authentication
            .authenticate_with_frontend(&authentication, &dependencies.principal)
            .await?;
    let authenticated = matches!(
        authentication_response,
        AuthenticationResponse::Authenticated { .. }
    );
    let (authentication_expiry, ticket_subject, ticket_credential_id) =
        match &authentication_response {
            AuthenticationResponse::Authenticated { session } => {
                let subject = (!matches!(
                    session.source,
                    luminate_protocol::AuthenticationSource::Peer
                ))
                .then(|| {
                    SessionPrincipal::new(
                        session.subject.authority(),
                        session.subject.subject(),
                        session.verified_groups.iter().cloned(),
                    )
                })
                .transpose()?;
                let credential_id = subject.as_ref().and_then(|_| session.credential_id.clone());
                (session.expires_at, subject, credential_id)
            }
            AuthenticationResponse::Rejected { .. } => (None, None, None),
        };
    if let AuthenticationResponse::Authenticated { session } = &authentication_response
        && !matches!(
            session.source,
            luminate_protocol::AuthenticationSource::Peer
        )
    {
        let Some(administration) = dependencies.access_administration.as_ref() else {
            anyhow::bail!("authenticated-subject policy is unavailable");
        };
        let subject = SessionPrincipal::new(
            session.subject.authority(),
            session.subject.subject(),
            session.verified_groups.iter().cloned(),
        )?;
        dependencies.policy = authenticated_policy(
            administration,
            subject,
            frontend_actor.clone(),
            session.credential_id.clone(),
            frontend_credential_id.clone(),
        );
    }
    timeout(
        framing::IO_TIMEOUT,
        framing::send(&mut stream, &authentication_response),
    )
    .await??;
    if !authenticated {
        return Ok(());
    }
    if let Some(scope) = session_scope.clone() {
        dependencies.policy = Arc::new(ScopedPolicy::new(Arc::clone(&dependencies.policy), scope));
    }

    // Frame streams started by this connection. If the connection drops
    // (client crash, network loss) before sending `EndFrameStream`, these are
    // released during teardown. Otherwise their targets would remain locked
    // against both streaming and ordinary mutations until the daemon restarts.
    let owned_streams: sync::Mutex<Vec<TargetId>> = sync::Mutex::new(Vec::new());
    let mut ping_limiter = PingLimiter::new();
    let result = connection_request_loop(
        &mut stream,
        &dependencies,
        &owned_streams,
        &mut ping_limiter,
        &mut revocation,
        &EventTicketContext {
            authentication_expiry,
            session_scope: session_scope.clone(),
            subject: ticket_subject,
            frontend_actor,
            credential_id: ticket_credential_id,
            frontend_credential_id,
        },
        IDLE_CONNECTION_TIMEOUT,
    )
    .await;

    let abandoned = mem::take(&mut *owned_streams.lock().expect("lock poisoned"));
    if !abandoned.is_empty() {
        dependencies
            .state
            .lock()
            .await
            .end_all_frame_streams(&abandoned);
        let devices: HashSet<_> = abandoned
            .iter()
            .map(|target| target.device_id().clone())
            .collect();
        let shm_plugin_manager = Arc::clone(&dependencies.plugin_manager);
        let shm_abandoned = abandoned.clone();
        let shm_devices = devices.iter().cloned().collect();
        let ended_client_streams = Arc::new(sync::Mutex::new(Vec::new()));
        let ended_client_streams_in_job = Arc::clone(&ended_client_streams);

        // A target is streamed by exactly one publisher: either the daemon
        // (`BeginFrameStream`) or a client (`BeginShmFrameStream`).
        // `owned_streams` records only the target, not which side owns it, so
        // both registries are swept. The inactive registry is a no-op.
        let _ = dependencies
            .mutations
            .execute_hardware_only(
                shm_devices,
                Box::new(move || {
                    shm_plugin_manager.end_all_shm_streams(&shm_abandoned);
                    let ended = shm_plugin_manager.end_all_shm_client_streams(&shm_abandoned);
                    *ended_client_streams_in_job.lock().expect("lock poisoned") = ended;
                    Ok(())
                }),
            )
            .await;
        let _ = dependencies.mutations.events.send(Event::StateChanged {
            devices: devices.into_iter().collect(),
        });

        // A dropped connection is the revocation case described in the design
        // document. Once the client (or at least this connection) is gone, no
        // further `EndShmFrameStream` request can arrive, so the daemon reclaims
        // any client-owned frame streams itself.
        for (target, generation) in
            mem::take(&mut *ended_client_streams.lock().expect("lock poisoned"))
        {
            let _ = dependencies
                .mutations
                .events
                .send(Event::ShmStreamEnded { target, generation });
        }
    }
    result
}

async fn connection_request_loop(
    stream: &mut Connection,
    dependencies: &ConnectionDependencies,
    owned_streams: &sync::Mutex<Vec<TargetId>>,
    ping_limiter: &mut PingLimiter,
    revocation: &mut Option<broadcast::Receiver<()>>,
    ticket_context: &EventTicketContext,
    idle_timeout: Duration,
) -> anyhow::Result<()> {
    loop {
        // The idle timeout measures inactivity between requests, not connection
        // age. Every successfully received request resets it.
        //
        // `IO_TIMEOUT` is deliberately shorter, so normal frame transfers are
        // bounded by I/O progress rather than by the idle timer.
        let received = tokio::select! {
            result = timeout(
                idle_timeout,
                framing::receive_with_frame_timeout::<RequestMessage>(stream, framing::IO_TIMEOUT),
            ) => result,
            () = wait_for_revocation(revocation) => {
                tracing::debug!("closing revoked authenticated session");
                return Ok(());
            }
            () = wait_for_expiry(ticket_context.authentication_expiry) => {
                tracing::debug!("closing expired authenticated session");
                return Ok(());
            }
        };
        let request = match received {
            Ok(Ok(request)) => request,
            Ok(Err(FramingError::Io(io_error)))
                if io_error.kind() == io::ErrorKind::UnexpectedEof =>
            {
                return Ok(());
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(_elapsed) => {
                tracing::debug!(timeout = ?idle_timeout, "closing idle connection");
                return Ok(());
            }
        };

        let response = if matches!(&request.request, Request::Ping) {
            ping_limiter.response()
        } else if matches!(&request.request, Request::IssueEventTicket) {
            match dependencies.authentication.mint_with_frontend(
                &dependencies.principal,
                ticket_context.session_scope.clone(),
                ticket_context.subject.clone(),
                ticket_context.frontend_actor.clone(),
                ticket_context.credential_id.clone(),
                ticket_context.frontend_credential_id.clone(),
                ticket_context.authentication_expiry,
                revocation.as_ref().map(broadcast::Receiver::resubscribe),
            ) {
                Ok(ticket) => Response {
                    status: ResponseStatus::EventTicket(ticket),
                },
                Err(error) => Response {
                    status: ResponseStatus::Error(OperationError {
                        code: if error.downcast_ref::<EphemeralCapacityError>().is_some() {
                            ErrorCode::RateLimited
                        } else {
                            ErrorCode::Unavailable
                        },
                        message: error.to_string(),
                        retry_after_ms: None,
                        applied_targets: Vec::new(),
                    }),
                },
            }
        } else {
            // Re-read before every request so a committed management patch
            // takes effect on the next request rather than only on the next
            // connection.
            let config = dependencies.management.effective();
            let ctx = DispatchContext::new(dependencies, owned_streams, &config);
            dispatch_request(request.request, &ctx).await
        };
        let response = ResponseMessage {
            id: request.id,
            response,
        };
        timeout(framing::IO_TIMEOUT, framing::send(stream, &response)).await??;
    }
}

async fn wait_for_revocation(receiver: &mut Option<broadcast::Receiver<()>>) {
    match receiver {
        Some(receiver) => {
            let _ = receiver.recv().await;
        }
        None => pending().await,
    }
}

async fn wait_for_expiry(expiry: Option<SystemTime>) {
    match expiry {
        Some(expiry) => {
            let remaining = expiry.duration_since(SystemTime::now()).unwrap_or_default();
            sleep(remaining).await;
        }
        None => pending().await,
    }
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
