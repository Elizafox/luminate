// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Authenticated WebSocket tickets, event subscriptions, and frame streams.

use std::collections::{HashMap, HashSet};
use std::iter::repeat_with;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use axum::{Extension, Json};
use futures_util::{SinkExt as _, StreamExt as _};
use luminate::policy::PrincipalId;
use luminate::{
    Collection, CollectionId, CollectionMember, Colour, DeviceId, Event, FrameEnvelope,
    FramePayload, Rgb, TargetId,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::sync::{OwnedSemaphorePermit, mpsc, oneshot};
use tokio::time::{MissedTickBehavior, interval, timeout};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::AuthContext;
use crate::resource::ResourceLimits;
#[cfg(test)]
use crate::resource::credential_digest;
use crate::rest::{AuthenticatedClient, session};
use crate::server::UpgradeFlag;
use crate::{AppState, problem_response};

const TICKET_LIFETIME: Duration = Duration::from_secs(30);
const PACKED_HEADER_LEN: usize = 24;
const PACKED_MAGIC: &[u8; 4] = b"LFRM";

#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "purpose", rename_all = "lowercase")]
pub(crate) enum TicketRequest {
    Events {
        #[serde(default)]
        subscription: EventSubscriptionOptions,
    },
    Frames,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct EventSubscriptionOptions {
    #[serde(default = "default_true")]
    include_baseline: bool,
    #[serde(default)]
    kinds: Vec<EventKind>,
    #[serde(default)]
    selectors: Vec<EventSelector>,
}

impl Default for EventSubscriptionOptions {
    fn default() -> Self {
        Self {
            include_baseline: true,
            kinds: Vec::new(),
            selectors: Vec::new(),
        }
    }
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EventKind {
    ResyncRequired,
    TopologyChanged,
    StateChanged,
    ConfigurationChanged,
    ShmStreamEnded,
    ScenesChanged,
    TransitionsChanged,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum EventSelector {
    Device { id: String },
    Collection { id: String },
}

#[cfg(test)]
pub(crate) fn event_ticket_request_for_test() -> TicketRequest {
    TicketRequest::Events {
        subscription: EventSubscriptionOptions {
            include_baseline: true,
            ..EventSubscriptionOptions::default()
        },
    }
}

#[derive(Serialize, ToSchema)]
pub(crate) struct TicketResponse {
    ticket: String,
    expires_at: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct TicketQuery {
    pub(crate) ticket: String,
}

struct TicketGrant<T> {
    purpose: TicketGrantPurpose,
    session: T,
    subject: PrincipalId,
    expires_at: Instant,
}

enum TicketGrantPurpose {
    Events(EventSubscriptionOptions),
    Frames,
}

const MAX_TICKETS: usize = 1_024;
const MAX_TICKETS_PER_SUBJECT: usize = 32;

pub(crate) struct TicketStore<T = AuthenticatedClient> {
    grants: Mutex<HashMap<Uuid, TicketGrant<T>>>,
}

impl<T> Default for TicketStore<T> {
    fn default() -> Self {
        Self {
            grants: Mutex::new(HashMap::new()),
        }
    }
}

impl<T> TicketStore<T> {
    async fn issue(
        &self,
        purpose: TicketGrantPurpose,
        session: T,
        subject: PrincipalId,
        authentication_expiry: Option<SystemTime>,
    ) -> Result<(Uuid, u64), ()> {
        let now = SystemTime::now();
        let ticket_expiry = now.checked_add(TICKET_LIFETIME).unwrap_or(now);
        let effective_expiry =
            authentication_expiry.map_or(ticket_expiry, |expiry| expiry.min(ticket_expiry));
        let lifetime = effective_expiry
            .duration_since(now)
            .unwrap_or(Duration::ZERO);
        let expires_at = Instant::now() + lifetime;
        let expires_at_unix = effective_expiry
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_secs());
        let mut grants = self.grants.lock().await;
        grants.retain(|_, grant| grant.expires_at > Instant::now());
        if grants.len() >= MAX_TICKETS
            || grants
                .values()
                .filter(|grant| grant.subject == subject)
                .count()
                >= MAX_TICKETS_PER_SUBJECT
        {
            return Err(());
        }
        let ticket = repeat_with(Uuid::new_v4)
            .take(8)
            .find(|ticket| !grants.contains_key(ticket))
            .ok_or(())?;
        grants.insert(
            ticket,
            TicketGrant {
                purpose,
                session,
                subject,
                expires_at,
            },
        );
        Ok((ticket, expires_at_unix))
    }

    async fn consume(&self, ticket: &str) -> Option<TicketGrant<T>> {
        let ticket = Uuid::parse_str(ticket).ok()?;
        let grant = self.grants.lock().await.remove(&ticket)?;
        (grant.expires_at > Instant::now()).then_some(grant)
    }

    pub(crate) async fn prune_expired(&self) {
        self.grants
            .lock()
            .await
            .retain(|_, grant| grant.expires_at > Instant::now());
    }
}

#[cfg(test)]
impl TicketStore<AuthenticatedClient> {
    pub(crate) async fn issue_frame_test_ticket(
        &self,
        state: &AppState,
        auth: AuthContext,
    ) -> Uuid {
        let authenticated = session(state, auth)
            .await
            .expect("authenticate test session");
        let subject = authenticated.session().subject.clone();
        let expiry = authenticated.session().expires_at;
        self.issue(TicketGrantPurpose::Frames, authenticated, subject, expiry)
            .await
            .expect("issue test ticket")
            .0
    }

    pub(crate) async fn contains_test_ticket(&self, ticket: Uuid) -> bool {
        self.grants.lock().await.contains_key(&ticket)
    }
}

#[utoipa::path(
    post,
    path = "/api/v0/websocket-tickets",
    request_body = TicketRequest,
    responses(
        (status = 201, description = "Single-use WebSocket ticket", body = TicketResponse),
        (status = 401, description = "Authentication failed", body = crate::ProblemResponse),
        (status = 429, description = "Ticket capacity reached", body = crate::ProblemResponse)
    )
)]
pub(crate) async fn create_ticket(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(request): Json<TicketRequest>,
) -> Response {
    let authenticated = match session(&state, auth).await {
        Ok(authenticated) => authenticated,
        Err(response) => return response,
    };
    let purpose = match request {
        TicketRequest::Events { subscription } => TicketGrantPurpose::Events(subscription),
        TicketRequest::Frames => TicketGrantPurpose::Frames,
    };
    let subject = authenticated.session().subject.clone();
    let authentication_expiry = authenticated.session().expires_at;
    let Ok((ticket, expires_at)) = state
        .tickets
        .issue(purpose, authenticated, subject, authentication_expiry)
        .await
    else {
        return problem_response(
            StatusCode::TOO_MANY_REQUESTS,
            "websocket-ticket-capacity",
            "WebSocket ticket capacity reached",
            None,
            None,
        );
    };
    (
        StatusCode::CREATED,
        [(header::CACHE_CONTROL, "no-store")],
        Json(TicketResponse {
            ticket: ticket.to_string(),
            expires_at,
        }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/api/v0/ws/events",
    params(("ticket" = String, Query, description = "Single-use events ticket")),
    responses(
        (status = 101, description = "Event WebSocket upgraded"),
        (status = 401, description = "Ticket rejected", body = crate::ProblemResponse)
    )
)]
pub(crate) async fn events_upgrade(
    State(state): State<AppState>,
    Extension(query): Extension<TicketQuery>,
    upgrade_flag: Option<Extension<UpgradeFlag>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let Some(session_permit) = state.resources.websocket_session() else {
        return websocket_capacity_denial();
    };
    let Some(TicketGrant {
        purpose: TicketGrantPurpose::Events(options),
        session,
        ..
    }) = state.tickets.consume(&query.ticket).await
    else {
        return ticket_denial();
    };
    let upgrade = upgrade
        .max_message_size(state.resources.maximum_websocket_message_bytes)
        .max_frame_size(state.resources.maximum_websocket_message_bytes);
    let resources = Arc::clone(&state.resources);
    if let Some(Extension(upgrade_flag)) = upgrade_flag {
        upgrade_flag.mark();
    }
    upgrade
        .on_upgrade(move |socket| event_socket(socket, session, options, resources, session_permit))
}

#[utoipa::path(
    get,
    path = "/api/v0/ws/frames",
    params(("ticket" = String, Query, description = "Single-use frames ticket")),
    responses(
        (status = 101, description = "Frame WebSocket upgraded"),
        (status = 401, description = "Ticket rejected", body = crate::ProblemResponse)
    )
)]
pub(crate) async fn frames_upgrade(
    State(state): State<AppState>,
    Extension(query): Extension<TicketQuery>,
    upgrade_flag: Option<Extension<UpgradeFlag>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let Some(session_permit) = state.resources.websocket_session() else {
        return websocket_capacity_denial();
    };
    let Some(TicketGrant {
        purpose: TicketGrantPurpose::Frames,
        session,
        ..
    }) = state.tickets.consume(&query.ticket).await
    else {
        return ticket_denial();
    };
    let upgrade = upgrade
        .max_message_size(state.resources.maximum_websocket_message_bytes)
        .max_frame_size(state.resources.maximum_websocket_message_bytes);
    let resources = Arc::clone(&state.resources);
    if let Some(Extension(upgrade_flag)) = upgrade_flag {
        upgrade_flag.mark();
    }
    upgrade.on_upgrade(move |socket| frame_socket(socket, session, resources, session_permit))
}

fn websocket_capacity_denial() -> Response {
    problem_response(
        StatusCode::TOO_MANY_REQUESTS,
        "websocket-session-capacity",
        "WebSocket session capacity reached",
        None,
        Some(1_000),
    )
}

fn ticket_denial() -> Response {
    problem_response(
        StatusCode::UNAUTHORIZED,
        "invalid-websocket-ticket",
        "the WebSocket ticket is invalid, expired, already used, or scoped to another endpoint",
        None,
        None,
    )
}

#[derive(Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum EventServerMessage {
    Baseline {
        protocol: u8,
        #[schema(value_type = Vec<Object>)]
        devices: Vec<luminate::Device>,
    },
    Event {
        #[schema(value_type = Object)]
        event: Event,
    },
}

struct EventFilter {
    kinds: HashSet<EventKind>,
    devices: Option<HashSet<DeviceId>>,
}

impl EventFilter {
    async fn build(
        client: &luminate::Client,
        options: &EventSubscriptionOptions,
    ) -> Result<Self, String> {
        let needs_collections = options
            .selectors
            .iter()
            .any(|selector| matches!(selector, EventSelector::Collection { .. }));
        let collections = if needs_collections {
            client
                .list_collections()
                .await
                .map_err(|error| error.to_string())?
        } else {
            Vec::new()
        };
        let indexed = collections
            .iter()
            .map(|collection| (&collection.id, collection))
            .collect::<HashMap<_, _>>();
        let mut devices = HashSet::new();
        for selector in &options.selectors {
            match selector {
                EventSelector::Device { id } => {
                    devices.insert(DeviceId::new(id.clone()));
                }
                EventSelector::Collection { id } => {
                    collect_collection_devices(
                        &CollectionId::new(id.clone()),
                        &indexed,
                        &mut HashSet::new(),
                        &mut devices,
                    )?;
                }
            }
        }
        Ok(Self {
            kinds: options.kinds.iter().copied().collect(),
            devices: (!options.selectors.is_empty()).then_some(devices),
        })
    }

    fn retain_devices(&self, devices: &mut Vec<luminate::Device>) {
        if let Some(selected) = &self.devices {
            devices.retain(|device| selected.contains(&device.id));
        }
    }

    fn project(&self, mut event: Event) -> Option<Event> {
        if matches!(event, Event::ResyncRequired) {
            return Some(event);
        }
        if !self.kinds.is_empty() && !self.kinds.contains(&EventKind::from_event(&event)) {
            return None;
        }
        let Some(selected) = &self.devices else {
            return Some(event);
        };
        match &mut event {
            Event::TopologyChanged { devices } | Event::StateChanged { devices } => {
                if devices.is_empty() {
                    return Some(event);
                }
                devices.retain(|device| selected.contains(device));
                (!devices.is_empty()).then_some(event)
            }
            Event::ShmStreamEnded { target, .. } => {
                selected.contains(target.device_id()).then_some(event)
            }
            Event::ResyncRequired
            | Event::ConfigurationChanged { .. }
            | Event::ScenesChanged
            | Event::TransitionsChanged { .. } => Some(event),
        }
    }
}

impl EventKind {
    const fn from_event(event: &Event) -> Self {
        match event {
            Event::ResyncRequired => Self::ResyncRequired,
            Event::TopologyChanged { .. } => Self::TopologyChanged,
            Event::StateChanged { .. } => Self::StateChanged,
            Event::ConfigurationChanged { .. } => Self::ConfigurationChanged,
            Event::ShmStreamEnded { .. } => Self::ShmStreamEnded,
            Event::ScenesChanged => Self::ScenesChanged,
            Event::TransitionsChanged { .. } => Self::TransitionsChanged,
        }
    }
}

fn collect_collection_devices(
    id: &CollectionId,
    collections: &HashMap<&CollectionId, &Collection>,
    visiting: &mut HashSet<CollectionId>,
    devices: &mut HashSet<DeviceId>,
) -> Result<(), String> {
    if !visiting.insert(id.clone()) {
        return Err(format!(
            "collection {} contains a membership cycle",
            id.as_str()
        ));
    }
    let collection = collections
        .get(id)
        .ok_or_else(|| format!("collection {} is not observable", id.as_str()))?;
    for member in &collection.members {
        match member {
            CollectionMember::Target(target) => {
                devices.insert(target.device_id().clone());
            }
            CollectionMember::Collection(nested) => {
                collect_collection_devices(nested, collections, visiting, devices)?;
            }
        }
    }
    visiting.remove(id);
    Ok(())
}

async fn event_socket(
    socket: WebSocket,
    session: AuthenticatedClient,
    options: EventSubscriptionOptions,
    resources: Arc<ResourceLimits>,
    _session_permit: OwnedSemaphorePermit,
) {
    event_socket_inner(socket, session, options, &resources).await;
}

async fn send_event_baseline(
    socket: &mut WebSocket,
    client: &luminate::Client,
    filter: &EventFilter,
) -> Result<(), ()> {
    let mut devices = match client.list_devices().await {
        Ok(value) => value,
        Err(error) => {
            send_json_error(socket, "baseline-failed", &error.to_string()).await;
            return Err(());
        }
    };
    filter.retain_devices(&mut devices);
    send_json(
        socket,
        &EventServerMessage::Baseline {
            protocol: 0,
            devices,
        },
    )
    .await
}

async fn event_socket_inner(
    mut socket: WebSocket,
    session: AuthenticatedClient,
    options: EventSubscriptionOptions,
    resources: &ResourceLimits,
) {
    let mut subscription = match session.subscribe().await {
        Ok(value) => value,
        Err(error) => {
            send_json_error(&mut socket, "subscription-failed", &error.to_string()).await;
            return;
        }
    };
    let filter = match EventFilter::build(&session, &options).await {
        Ok(value) => value,
        Err(error) => {
            send_json_error(&mut socket, "invalid-subscription", &error).await;
            return;
        }
    };
    if options.include_baseline
        && send_event_baseline(&mut socket, &session, &filter)
            .await
            .is_err()
    {
        return;
    }

    let (mut sender, mut receiver) = socket.split();
    let (outbound_tx, mut outbound_rx) =
        mpsc::channel::<Message>(resources.maximum_websocket_queue);
    let (closed_tx, mut closed_rx) = oneshot::channel();
    let (liveness_tx, mut liveness_rx) = mpsc::channel::<()>(1);
    let writer = tokio::spawn(async move {
        while let Some(message) = outbound_rx.recv().await {
            if sender.send(message).await.is_err() {
                return;
            }
        }
    });
    let reader_outbound = outbound_tx.clone();
    let reader = tokio::spawn(async move {
        while let Some(message) = receiver.next().await {
            match message {
                Ok(Message::Ping(payload)) => {
                    if reader_outbound.send(Message::Pong(payload)).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Pong(_)) => {
                    let _result = liveness_tx.try_send(());
                }
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(_) => {}
            }
        }
        let _result = closed_tx.send(());
    });

    let mut liveness = interval(resources.websocket_idle);
    liveness.set_missed_tick_behavior(MissedTickBehavior::Skip);
    liveness.tick().await;
    let mut awaiting_pong = false;
    loop {
        tokio::select! {
            event = subscription.next_event() => {
                match event {
                    Ok(event) => {
                        let Some(event) = filter.project(event) else {
                            continue;
                        };
                        let Ok(message) = json_message(&EventServerMessage::Event { event }) else {
                            break;
                        };
                        if outbound_tx.send(message).await.is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        if let Ok(message) = error_message("subscription-ended", &error.to_string()) {
                            let _result = outbound_tx.send(message).await;
                        }
                        break;
                    }
                }
            }
            _result = &mut closed_rx => break,
            Some(()) = liveness_rx.recv() => awaiting_pong = false,
            _ = liveness.tick() => {
                if awaiting_pong || outbound_tx.try_send(Message::Ping(Vec::new().into())).is_err() {
                    break;
                }
                awaiting_pong = true;
            }
        }
    }
    reader.abort();
    let _reader_result = reader.await;
    drop(outbound_tx);
    let _writer_result = writer.await;
}

#[derive(Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum FrameClientMessage {
    Start {
        protocol: u8,
        #[schema(value_type = Object)]
        target: TargetId,
    },
    Frame {
        sequence: u64,
        #[schema(value_type = Object)]
        payload: FramePayload,
        #[serde(default)]
        commit: bool,
    },
}

#[derive(Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum FrameServerMessage {
    Started { generation: u32 },
    Ack { sequence: u64, dropped: bool },
    Error { code: String, detail: String },
}

async fn frame_socket(
    socket: WebSocket,
    session: AuthenticatedClient,
    resources: Arc<ResourceLimits>,
    _session_permit: OwnedSemaphorePermit,
) {
    frame_socket_inner(socket, session, &resources).await;
}

async fn frame_socket_inner(
    mut socket: WebSocket,
    session: AuthenticatedClient,
    resources: &ResourceLimits,
) {
    let Ok(Some(Ok(Message::Text(message)))) =
        timeout(resources.websocket_start_timeout, socket.recv()).await
    else {
        send_json_error(
            &mut socket,
            "start-required",
            "the first message must start a frame stream",
        )
        .await;
        return;
    };
    let Ok(FrameClientMessage::Start { protocol, target }) =
        serde_json::from_str::<FrameClientMessage>(&message)
    else {
        send_json_error(
            &mut socket,
            "start-required",
            "the first message must be a valid start message",
        )
        .await;
        return;
    };
    if protocol != 0 {
        send_json_error(
            &mut socket,
            "unsupported-protocol",
            "only frame protocol version 0 is supported",
        )
        .await;
        return;
    }
    let generation = match session.begin_frame_stream(target.clone()).await {
        Ok(generation) => generation,
        Err(error) => {
            send_json_error(&mut socket, "stream-start-failed", &error.to_string()).await;
            return;
        }
    };
    if send_json(&mut socket, &FrameServerMessage::Started { generation })
        .await
        .is_err()
    {
        return;
    }

    run_frame_stream(
        &mut socket,
        &session,
        &target,
        generation,
        resources.websocket_idle,
    )
    .await;
    let _result = session.end_frame_stream(target, generation).await;
}

async fn run_frame_stream(
    socket: &mut WebSocket,
    client: &luminate::Client,
    target: &TargetId,
    generation: u32,
    idle: Duration,
) {
    let mut liveness = interval(idle);
    liveness.set_missed_tick_behavior(MissedTickBehavior::Skip);
    liveness.tick().await;
    let mut awaiting_pong = false;
    loop {
        let message = tokio::select! {
            message = socket.recv() => message,
            _ = liveness.tick() => {
                if awaiting_pong || socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
                awaiting_pong = true;
                continue;
            }
        };
        let Some(message) = message else {
            break;
        };
        let envelope = match message {
            Ok(Message::Text(message)) => match decode_text_frame(&message, generation) {
                Ok(envelope) => envelope,
                Err(detail) => {
                    send_json_error(socket, "invalid-frame", &detail).await;
                    return;
                }
            },
            Ok(Message::Binary(message)) => match decode_packed_frame(&message, generation) {
                Ok(envelope) => envelope,
                Err(detail) => {
                    send_json_error(socket, "invalid-packed-frame", &detail).await;
                    return;
                }
            },
            Ok(Message::Ping(payload)) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    return;
                }
                continue;
            }
            Ok(Message::Pong(_)) => {
                awaiting_pong = false;
                continue;
            }
            Ok(Message::Close(_)) | Err(_) => break,
        };
        match client.upload_frame(target.clone(), envelope).await {
            Ok(ack) => {
                if send_json(
                    socket,
                    &FrameServerMessage::Ack {
                        sequence: ack.sequence,
                        dropped: ack.dropped,
                    },
                )
                .await
                .is_err()
                {
                    break;
                }
            }
            Err(error) => {
                send_json_error(socket, "frame-upload-failed", &error.to_string()).await;
                break;
            }
        }
    }
}

fn decode_text_frame(message: &str, generation: u32) -> Result<FrameEnvelope, String> {
    match serde_json::from_str(message).map_err(|error| error.to_string())? {
        FrameClientMessage::Frame {
            sequence,
            payload,
            commit,
        } => Ok(FrameEnvelope {
            generation,
            sequence,
            payload,
            commit,
        }),
        FrameClientMessage::Start { .. } => Err("a stream is already active".to_owned()),
    }
}

fn decode_packed_frame(message: &[u8], generation: u32) -> Result<FrameEnvelope, String> {
    if message.len() < PACKED_HEADER_LEN {
        return Err("packed frame header is truncated".to_owned());
    }
    let header: &[u8; PACKED_HEADER_LEN] = message
        .get(..PACKED_HEADER_LEN)
        .and_then(|header| header.try_into().ok())
        .ok_or_else(|| "packed frame header is truncated".to_owned())?;
    if header.get(0..4) != Some(PACKED_MAGIC) {
        return Err("packed frame magic is invalid".to_owned());
    }
    if header.get(4) != Some(&0) {
        return Err("packed frame header version is unsupported".to_owned());
    }
    if header.get(5) != Some(&1) {
        return Err("packed frame pixel format is unsupported".to_owned());
    }
    let flags = u16::from_be_bytes(
        header
            .get(6..8)
            .and_then(|value| value.try_into().ok())
            .ok_or_else(|| "packed frame flags are truncated".to_owned())?,
    );
    if flags & !1 != 0 {
        return Err("packed frame contains unrecognized flags".to_owned());
    }
    if header.get(20..24) != Some(&[0; 4]) {
        return Err("packed frame reserved field is non-zero".to_owned());
    }
    let sequence = u64::from_be_bytes(
        header
            .get(8..16)
            .ok_or("packed frame sequence is truncated")?
            .try_into()
            .map_err(|_| "packed frame sequence is truncated")?,
    );
    let pixel_count = u32::from_be_bytes(
        header
            .get(16..20)
            .ok_or("packed frame pixel count is truncated")?
            .try_into()
            .map_err(|_| "packed frame pixel count is truncated")?,
    );
    let payload_len = usize::try_from(pixel_count)
        .ok()
        .and_then(|count| count.checked_mul(3))
        .ok_or_else(|| "packed frame pixel count overflows its payload length".to_owned())?;
    if message.len() != PACKED_HEADER_LEN + payload_len {
        return Err("packed frame payload length does not match its pixel count".to_owned());
    }
    let payload = message
        .get(PACKED_HEADER_LEN..)
        .ok_or_else(|| "packed frame payload is truncated".to_owned())?;
    let pixels = payload
        .chunks_exact(3)
        .filter_map(|pixel| {
            let [red, green, blue] = pixel else {
                return None;
            };
            Some(Colour::rgb(Rgb::new(*red, *green, *blue)))
        })
        .collect();
    Ok(FrameEnvelope {
        generation,
        sequence,
        payload: FramePayload::Full(pixels),
        commit: flags & 1 != 0,
    })
}

async fn send_json<T: Serialize>(socket: &mut WebSocket, message: &T) -> Result<(), ()> {
    socket.send(json_message(message)?).await.map_err(|_| ())
}

fn json_message<T: Serialize>(message: &T) -> Result<Message, ()> {
    serde_json::to_string(message)
        .map(|message| Message::Text(message.into()))
        .map_err(|_| ())
}

fn error_message(code: &str, detail: &str) -> Result<Message, ()> {
    json_message(&FrameServerMessage::Error {
        code: code.to_owned(),
        detail: detail.to_owned(),
    })
}

async fn send_json_error(socket: &mut WebSocket, code: &str, detail: &str) {
    if let Ok(message) = error_message(code, detail) {
        let _result = socket.send(message).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use luminate::{
        CapabilitySet, Device, Element, ElementId, ElementKind, Surface, SurfaceId, SurfaceKind,
    };

    fn auth() -> AuthContext {
        let credential = luminate::Credential::new("test-token").expect("credential");
        AuthContext {
            credential_digest: credential_digest(credential.expose()),
            credential,
            delegation: None,
            peer: "127.0.0.1".parse().expect("test peer"),
        }
    }

    fn subject() -> PrincipalId {
        PrincipalId::new("local", "test").expect("principal")
    }

    #[tokio::test]
    async fn ticket_is_single_use_and_purpose_bound() {
        let store = TicketStore::default();
        let (wrong_ticket, _) = store
            .issue(
                TicketGrantPurpose::Events(EventSubscriptionOptions::default()),
                auth(),
                subject(),
                None,
            )
            .await
            .expect("issue ticket");
        assert!(matches!(
            store.consume(&wrong_ticket.to_string()).await,
            Some(TicketGrant {
                purpose: TicketGrantPurpose::Events(_),
                ..
            })
        ));
        assert!(
            store.consume(&wrong_ticket.to_string()).await.is_none(),
            "an endpoint mismatch still consumes the one-use ticket"
        );

        let (ticket, _) = store
            .issue(TicketGrantPurpose::Frames, auth(), subject(), None)
            .await
            .expect("issue ticket");
        assert!(matches!(
            store.consume(&ticket.to_string()).await,
            Some(TicketGrant {
                purpose: TicketGrantPurpose::Frames,
                ..
            })
        ));
        assert!(store.consume(&ticket.to_string()).await.is_none());
        assert!(store.consume("not-a-uuid").await.is_none());

        let (expired, expires_at) = store
            .issue(
                TicketGrantPurpose::Frames,
                auth(),
                subject(),
                Some(UNIX_EPOCH),
            )
            .await
            .expect("issue expired ticket");
        assert_eq!(expires_at, 0);
        assert!(store.consume(&expired.to_string()).await.is_none());
    }

    #[tokio::test]
    async fn ticket_capacity_is_bounded_per_subject() {
        let store = TicketStore::<usize>::default();
        let first = subject();
        for sequence in 0..MAX_TICKETS_PER_SUBJECT {
            store
                .issue(TicketGrantPurpose::Frames, sequence, first.clone(), None)
                .await
                .expect("issue within subject capacity");
        }
        assert!(
            store
                .issue(TicketGrantPurpose::Frames, usize::MAX, first, None)
                .await
                .is_err()
        );
        assert!(
            store
                .issue(
                    TicketGrantPurpose::Frames,
                    usize::MAX,
                    PrincipalId::new("local", "other").expect("principal"),
                    None,
                )
                .await
                .is_ok()
        );
    }

    #[test]
    fn event_ticket_defaults_to_an_unfiltered_baseline_subscription() {
        let request: TicketRequest =
            serde_json::from_str(r#"{"purpose":"events"}"#).expect("event ticket request");
        let TicketRequest::Events { subscription } = request else {
            panic!("expected event ticket request");
        };
        assert!(subscription.include_baseline);
        assert!(subscription.kinds.is_empty());
        assert!(subscription.selectors.is_empty());
    }

    #[test]
    fn event_baseline_serialization_preserves_element_physical_tags() {
        let message = EventServerMessage::Baseline {
            protocol: 0,
            devices: vec![Device {
                id: DeviceId::new("beam"),
                name: "Beam".to_owned(),
                vendor: None,
                model: None,
                provider_instance: Some("example".to_owned()),
                surfaces: vec![Surface {
                    id: SurfaceId::new("bars"),
                    name: "Bars".to_owned(),
                    kind: SurfaceKind::Linear { length: 1.0 },
                    physical_tags: vec!["layout:horizontal".to_owned()],
                    elements: vec![Element {
                        id: ElementId::new("left"),
                        name: None,
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
            }],
        };

        let value = serde_json::to_value(message).expect("serialize event baseline");
        assert_eq!(
            value["devices"][0]["surfaces"][0]["elements"][0]["physical_tags"],
            serde_json::json!(["shape:round", "position:left"])
        );
    }

    #[test]
    fn event_filter_applies_kinds_and_device_selection() {
        let filter = EventFilter {
            kinds: HashSet::from([EventKind::StateChanged]),
            devices: Some(HashSet::from([DeviceId::new("lamp")])),
        };
        assert_eq!(
            filter.project(Event::StateChanged {
                devices: vec![DeviceId::new("lamp"), DeviceId::new("keyboard")],
            }),
            Some(Event::StateChanged {
                devices: vec![DeviceId::new("lamp")],
            })
        );
        assert!(
            filter
                .project(Event::StateChanged {
                    devices: vec![DeviceId::new("keyboard")],
                })
                .is_none()
        );
        assert_eq!(
            filter.project(Event::ResyncRequired),
            Some(Event::ResyncRequired)
        );
        assert_eq!(
            filter.project(Event::StateChanged {
                devices: Vec::new(),
            }),
            Some(Event::StateChanged {
                devices: Vec::new(),
            })
        );
        assert!(
            filter
                .project(Event::TopologyChanged {
                    devices: vec![DeviceId::new("lamp")],
                })
                .is_none()
        );
    }

    #[test]
    fn collection_event_selection_expands_nested_device_members() {
        let child_id = CollectionId::new("child");
        let root_id = CollectionId::new("root");
        let child = Collection {
            id: child_id.clone(),
            name: "Child".to_owned(),
            description: None,
            owner: luminate::OwnerIdentity::Uid(1000),
            kind: None,
            members: vec![CollectionMember::Target(TargetId::surface("lamp", "light"))],
        };
        let root = Collection {
            id: root_id.clone(),
            name: "Root".to_owned(),
            description: None,
            owner: luminate::OwnerIdentity::Uid(1000),
            kind: None,
            members: vec![CollectionMember::Collection(child_id.clone())],
        };
        let indexed = HashMap::from([(&child.id, &child), (&root.id, &root)]);
        let mut devices = HashSet::new();
        collect_collection_devices(&root_id, &indexed, &mut HashSet::new(), &mut devices)
            .expect("resolve nested collection");
        assert_eq!(devices, HashSet::from([DeviceId::new("lamp")]));
        assert!(
            collect_collection_devices(
                &CollectionId::new("missing"),
                &indexed,
                &mut HashSet::new(),
                &mut HashSet::new(),
            )
            .is_err()
        );
    }

    #[test]
    fn packed_rgb8_frame_decodes() {
        let mut message = Vec::from(*PACKED_MAGIC);
        message.extend([0, 1]);
        message.extend(1_u16.to_be_bytes());
        message.extend(7_u64.to_be_bytes());
        message.extend(2_u32.to_be_bytes());
        message.extend(0_u32.to_be_bytes());
        message.extend([1, 2, 3, 4, 5, 6]);
        let frame = decode_packed_frame(&message, 9).expect("valid packed frame");
        assert_eq!(frame.generation, 9);
        assert_eq!(frame.sequence, 7);
        assert!(frame.commit);
        assert_eq!(
            frame.payload,
            FramePayload::Full(vec![
                Colour::rgb(Rgb::new(1, 2, 3)),
                Colour::rgb(Rgb::new(4, 5, 6)),
            ])
        );
    }

    #[test]
    fn packed_frame_rejects_each_reserved_boundary() {
        let mut message = vec![0_u8; PACKED_HEADER_LEN];
        message[0..4].copy_from_slice(PACKED_MAGIC);
        message[5] = 1;
        assert!(decode_packed_frame(&message[..23], 1).is_err());
        message[4] = 1;
        assert!(decode_packed_frame(&message, 1).is_err());
        message[4] = 0;
        message[7] = 2;
        assert!(decode_packed_frame(&message, 1).is_err());
        message[7] = 0;
        message[23] = 1;
        assert!(decode_packed_frame(&message, 1).is_err());
        message[23] = 0;
        message[19] = 1;
        assert!(decode_packed_frame(&message, 1).is_err());
    }

    #[test]
    fn text_frames_and_json_messages_cover_protocol_boundaries() {
        let frame = decode_text_frame(
            r#"{"type":"frame","sequence":4,"payload":{"Full":[]},"commit":true}"#,
            7,
        )
        .expect("valid text frame");
        assert_eq!(frame.generation, 7);
        assert_eq!(frame.sequence, 4);
        assert!(frame.commit);
        assert!(decode_text_frame("not json", 1).is_err());
        assert!(
            decode_text_frame(
                r#"{"type":"start","protocol":0,"target":{"device":"lamp"}}"#,
                1
            )
            .is_err()
        );

        let error = error_message("invalid-frame", "bad payload").expect("error message");
        assert!(matches!(error, Message::Text(message) if message.contains("invalid-frame")));
        assert_eq!(ticket_denial().status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn packed_frames_reject_magic_format_flags_and_payload_mismatch() {
        let mut message = vec![0_u8; PACKED_HEADER_LEN];
        assert!(decode_packed_frame(&message, 1).is_err());
        message[0..4].copy_from_slice(PACKED_MAGIC);
        assert!(decode_packed_frame(&message, 1).is_err());
        message[5] = 1;
        message[6..8].copy_from_slice(&2_u16.to_be_bytes());
        assert!(decode_packed_frame(&message, 1).is_err());
        message[6..8].copy_from_slice(&0_u16.to_be_bytes());
        message[16..20].copy_from_slice(&1_u32.to_be_bytes());
        assert!(decode_packed_frame(&message, 1).is_err());
    }
}
