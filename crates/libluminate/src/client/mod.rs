// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Asynchronous daemon client and topology-event subscription.

use luminate_core::shm_frame::ShmPixelFormat;
use luminate_core::state;

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as SyncMutex};
use std::time::{Duration, SystemTime};

use tokio::io::{ReadHalf, WriteHalf, split};
#[cfg(all(test, unix))]
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};
use tokio::time::sleep as sleep_for;
use tokio::time::timeout;

use luminate_platform::default_path::{default_socket_path, event_socket_path};
use luminate_platform::transport::{Address, Connection, connect};

use luminate_core::policy::{PolicyDocument, PolicyRevision, PrincipalId, SessionScope};
#[cfg(test)]
use luminate_protocol::EventTicket;
use luminate_protocol::{
    Authentication, AuthenticationRequest, AuthenticationResponse, ClientHello, Compatibility,
    DaemonHello, Event as ProtocolEvent, EventCompatibility, Request, RequestMessage, Response,
    ResponseMessage, ResponseStatus, SetAppearanceSlotsRequest, SetBrightnessRequest,
    SetEffectRequest, StartTransitionRequest, SubscribeAck, SubscribeHello, TransitionDestination,
    TransitionSource,
};
use luminate_protocol::{
    AuthenticationSource, SessionMetadata as WireSessionMetadata, TokenMetadata,
};
use luminate_protocol::{EVENT_PROTOCOL_VERSION, PROTOCOL_ABI_VERSION};
#[cfg(test)]
use luminate_protocol::{PluginSetupWorkflow, PluginSetupWorkflowKind};

use crate::error::{Error, Result};
use crate::{
    AppearanceSlotValue, Collection, CollectionCategory, CollectionId, CollectionMember, Colour,
    Device, DeviceId, Effect, EmissionState, Event, FrameEnvelope, ManagementChangeSet,
    ManagementPatch, ManagementSnapshot, Rgb, Scene, SceneBinding, SceneCaptureMode, SceneId,
    Selector, ServerInfo, TargetId, UnsupportedPolicy,
};
use crate::{TransitionId, TransitionOptions, TransitionStatus, TransitionTargetState};

use luminate_protocol::framing::{IO_TIMEOUT, receive, send};

type PendingRequests = Arc<SyncMutex<HashMap<u64, oneshot::Sender<Response>>>>;

#[cfg(all(test, unix))]
#[allow(
    clippy::expect_used,
    reason = "shared fake-daemon test setup should fail immediately when its fixed handshake is invalid"
)]
pub(crate) async fn authenticate_test_client(stream: &mut UnixStream) {
    use luminate_core::policy::PrincipalId;
    use luminate_protocol::{AuthenticationSource, SessionMetadata};

    let request: AuthenticationRequest =
        receive(stream).await.expect("receive test authentication");
    assert!(
        matches!(request.authentication, Authentication::Peer),
        "ordinary test clients must default to peer authentication"
    );
    send(
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
    .await
    .expect("send test authentication response");
}

/// A connected client.
///
/// Requests may be made concurrently. Each request carries a connection-local
/// identifier, and background I/O tasks route responses to the matching
/// caller. Cancelling one request future does not affect the connection or any
/// other in-flight request.
#[derive(Debug)]
pub struct Client {
    /// Feeds the connection's single request-writing task.
    requests: mpsc::UnboundedSender<RequestMessage>,

    /// Routes responses to the request futures waiting for them.
    pending: PendingRequests,

    /// Supplies connection-local identifiers for response routing.
    next_request_id: AtomicU64,

    /// Retained so event subscriptions connect to the matching endpoint.
    socket_path: PathBuf,

    daemon_version: String,
    session: SessionMetadata,
}

/// Sanitized identity and lease information for one connected client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMetadata {
    /// Authenticated subject.
    pub subject: PrincipalId,

    /// Verified groups supplied by the authentication method.
    pub verified_groups: Vec<String>,

    /// Authentication method accepted by the daemon.
    pub source: AuthenticationSource,

    /// Non-secret credential identifier, when one exists.
    pub credential_id: Option<String>,

    /// Authentication expiry, when finite.
    pub expires_at: Option<SystemTime>,
}

impl From<&WireSessionMetadata> for SessionMetadata {
    fn from(value: &WireSessionMetadata) -> Self {
        Self {
            subject: value.subject.clone(),
            verified_groups: value.verified_groups.clone(),
            source: value.source.clone(),
            credential_id: value.credential_id.clone(),
            expires_at: value.expires_at,
        }
    }
}

/// Configures authentication and voluntary scope for one [`Client`].
#[derive(Debug, Clone)]
pub struct ClientBuilder {
    /// `None` selects the platform's default daemon endpoint.
    path: Option<PathBuf>,

    authentication: Authentication,

    /// `None` requests no voluntary restriction beyond daemon policy.
    scope: Option<SessionScope>,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self {
            path: None,
            authentication: Authentication::Peer,
            scope: None,
        }
    }
}

impl ClientBuilder {
    /// Creates a builder using peer authentication and the default endpoint.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Uses the daemon endpoint derived from `path`.
    #[must_use]
    pub fn path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Selects the authentication method for this connection.
    #[must_use]
    pub fn authentication(mut self, authentication: Authentication) -> Self {
        self.authentication = authentication;
        self
    }

    /// Restricts this connection to the supplied allow-only scope.
    #[must_use]
    pub fn scope(mut self, scope: SessionScope) -> Self {
        self.scope = Some(scope);
        self
    }

    /// Connects using this configuration.
    ///
    /// # Errors
    ///
    /// Returns an authentication, compatibility, transport, or protocol error.
    pub async fn connect(self) -> Result<Client> {
        let path = match self.path {
            Some(path) => path,
            None => default_socket_path()?,
        };
        let stream = connect(&Address::from_configured_path(&path)).await?;
        Client::connect_stream(stream, path, self.authentication, self.scope).await
    }
}

/// Removes an abandoned request from the response-routing table when dropped.
struct PendingRequest {
    id: u64,
    pending: PendingRequests,
}

impl Drop for PendingRequest {
    fn drop(&mut self) {
        self.pending.lock().expect("lock poisoned").remove(&self.id);
    }
}

/// The daemon's response to one [`Client::upload_frame`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameAck {
    /// The sequence number the daemon accepted (echoes the uploaded frame's).
    pub sequence: u64,

    /// `true` if the frame was accepted but rate-limited, not forwarded to
    /// the plugin.
    pub dropped: bool,
}

/// The daemon's response to a successfully negotiated
/// [`Client::begin_shm_frame_stream`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShmStreamReady {
    /// Echoed in every published sample's header and in
    /// [`Client::end_shm_frame_stream`].
    pub generation: u32,

    /// Deterministic, target-derived iceoryx2 publish-subscribe service name
    /// to open for publishing sample data.
    pub service_name: String,

    /// Deterministic, target-derived iceoryx2 event service name to open for
    /// notifying the daemon a sample is ready.
    pub event_service_name: String,

    /// Pixel format every published sample must be encoded in.
    pub pixel_format: ShmPixelFormat,

    /// Must be carried in every sample's header
    /// (`ShmClientFrameHeader::stream_nonce`).
    pub stream_nonce: u64,

    /// Byte size of the shared-memory slice each sample occupies (header
    /// plus packed pixel payload).
    pub segment_bytes: u32,
}

/// The daemon's per-leaf result for a mutation addressed to a [`Selector`].
///
/// Direct-target mutations leave both lists empty. Collection mutations
/// partition resolved leaves into `applied` and `denied`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CollectionOutcome {
    /// Leaf targets the caller was authorized for, and that changed.
    pub applied: Vec<TargetId>,
    /// Leaf targets the caller's authorization didn't cover.
    pub denied: Vec<TargetId>,
}

/// The daemon's authorization-partitioned result for applying a scene.
pub type SceneOutcome = CollectionOutcome;

mod administration;
mod collections;
mod connection;
mod control;
mod events;
mod frames;
mod management;
mod scenes;
mod setup;
mod transitions;

pub use administration::{
    AuthenticationAdministration, CreatedAttestation, CreatedToken, PolicyAdministration,
};
pub use events::EventSubscription;
pub use transitions::Transitions;

impl Client {
    async fn expect_ack(&self, request: Request) -> Result<()> {
        match self.request(request).await?.status {
            ResponseStatus::Ack => Ok(()),
            other @ (ResponseStatus::ServerInfo(_)
            | ResponseStatus::Devices(_)
            | ResponseStatus::WithdrawnDevices(_)
            | ResponseStatus::Device(_)
            | ResponseStatus::State(_)
            | ResponseStatus::CollectionState(_)
            | ResponseStatus::EventTicket(_)
            | ResponseStatus::FrameStreamStarted { .. }
            | ResponseStatus::FrameAck { .. }
            | ResponseStatus::CollectionCreated { .. }
            | ResponseStatus::Collections(_)
            | ResponseStatus::CollectionInfo(_)
            | ResponseStatus::CollectionApplied { .. }
            | ResponseStatus::ShmFrameStreamReady { .. }
            | ResponseStatus::PluginSetupWorkflows(_)
            | ResponseStatus::ManagementSnapshot(_)
            | ResponseStatus::ManagementPatched(_)
            | ResponseStatus::AccessPolicy(_)
            | ResponseStatus::TokenCreated { .. }
            | ResponseStatus::Tokens(_)
            | ResponseStatus::AttestationCreated { .. }
            | ResponseStatus::Attestations(_)
            | ResponseStatus::Scene(_)
            | ResponseStatus::Scenes(_)
            | ResponseStatus::SceneInfo(_)
            | ResponseStatus::SceneApplied { .. }
            | ResponseStatus::Transition(_)
            | ResponseStatus::PluginSetupSession(_)
            | ResponseStatus::Error { .. }) => Err(unexpected_response("ack", &other)),
        }
    }

    /// As [`Self::expect_ack`], but for a [`Selector`]-addressed mutation
    /// that may resolve to a collection: accepts both the plain
    /// [`ResponseStatus::Ack`] a [`Selector::Target`] mutation returns, and
    /// the [`ResponseStatus::CollectionApplied`] a
    /// [`Selector::Collection`] mutation returns, normalizing either into a
    /// [`CollectionOutcome`].
    async fn expect_collection_outcome(&self, request: Request) -> Result<CollectionOutcome> {
        match self.request(request).await?.status {
            ResponseStatus::Ack => Ok(CollectionOutcome::default()),
            ResponseStatus::CollectionApplied { applied, denied } => {
                Ok(CollectionOutcome { applied, denied })
            }
            other @ (ResponseStatus::ServerInfo(_)
            | ResponseStatus::Devices(_)
            | ResponseStatus::WithdrawnDevices(_)
            | ResponseStatus::Device(_)
            | ResponseStatus::State(_)
            | ResponseStatus::CollectionState(_)
            | ResponseStatus::EventTicket(_)
            | ResponseStatus::FrameStreamStarted { .. }
            | ResponseStatus::FrameAck { .. }
            | ResponseStatus::CollectionCreated { .. }
            | ResponseStatus::Collections(_)
            | ResponseStatus::CollectionInfo(_)
            | ResponseStatus::ShmFrameStreamReady { .. }
            | ResponseStatus::PluginSetupWorkflows(_)
            | ResponseStatus::ManagementSnapshot(_)
            | ResponseStatus::ManagementPatched(_)
            | ResponseStatus::AccessPolicy(_)
            | ResponseStatus::TokenCreated { .. }
            | ResponseStatus::Tokens(_)
            | ResponseStatus::AttestationCreated { .. }
            | ResponseStatus::Attestations(_)
            | ResponseStatus::Scene(_)
            | ResponseStatus::Scenes(_)
            | ResponseStatus::SceneInfo(_)
            | ResponseStatus::SceneApplied { .. }
            | ResponseStatus::Transition(_)
            | ResponseStatus::PluginSetupSession(_)
            | ResponseStatus::Error { .. }) => {
                Err(unexpected_response("collection outcome", &other))
            }
        }
    }

    async fn request(&self, request: Request) -> Result<Response> {
        let (id, response_rx) = loop {
            let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
            let (response_tx, response_rx) = oneshot::channel();
            let mut pending = self.pending.lock().expect("lock poisoned");
            if let Entry::Vacant(entry) = pending.entry(id) {
                entry.insert(response_tx);
                break (id, response_rx);
            }
        };
        let pending = PendingRequest {
            id,
            pending: Arc::clone(&self.pending),
        };

        self.requests
            .send(RequestMessage { id, request })
            .map_err(|_| Error::Unavailable("daemon connection closed".to_owned()))?;
        let response = timeout(IO_TIMEOUT, response_rx)
            .await?
            .map_err(|_| Error::Unavailable("daemon connection closed".to_owned()))?;
        drop(pending);

        match response.status {
            ResponseStatus::Error(error) => Err(Error::from_operation_error(error)),
            ResponseStatus::ServerInfo(_)
            | ResponseStatus::Devices(_)
            | ResponseStatus::WithdrawnDevices(_)
            | ResponseStatus::Device(_)
            | ResponseStatus::State(_)
            | ResponseStatus::CollectionState(_)
            | ResponseStatus::EventTicket(_)
            | ResponseStatus::Ack
            | ResponseStatus::FrameStreamStarted { .. }
            | ResponseStatus::FrameAck { .. }
            | ResponseStatus::CollectionCreated { .. }
            | ResponseStatus::Collections(_)
            | ResponseStatus::CollectionInfo(_)
            | ResponseStatus::CollectionApplied { .. }
            | ResponseStatus::ShmFrameStreamReady { .. }
            | ResponseStatus::PluginSetupWorkflows(_)
            | ResponseStatus::PluginSetupSession(_)
            | ResponseStatus::ManagementSnapshot(_)
            | ResponseStatus::ManagementPatched(_)
            | ResponseStatus::AccessPolicy(_)
            | ResponseStatus::TokenCreated { .. }
            | ResponseStatus::Tokens(_)
            | ResponseStatus::AttestationCreated { .. }
            | ResponseStatus::Attestations(_)
            | ResponseStatus::Scene(_)
            | ResponseStatus::Scenes(_)
            | ResponseStatus::SceneInfo(_)
            | ResponseStatus::SceneApplied { .. }
            | ResponseStatus::Transition(_) => Ok(response),
        }
    }
}

async fn write_requests(
    mut writer: WriteHalf<Connection>,
    mut requests: mpsc::UnboundedReceiver<RequestMessage>,
    pending: PendingRequests,
) {
    while let Some(request) = requests.recv().await {
        if !matches!(
            timeout(IO_TIMEOUT, send(&mut writer, &request)).await,
            Ok(Ok(()))
        ) {
            break;
        }
    }

    pending.lock().expect("lock poisoned").clear();
}

async fn read_responses(mut reader: ReadHalf<Connection>, pending: PendingRequests) {
    while let Ok(message) = receive::<ResponseMessage>(&mut reader).await {
        let response = pending.lock().expect("lock poisoned").remove(&message.id);
        if let Some(response) = response {
            let _ = response.send(message.response);
        }
    }

    pending.lock().expect("lock poisoned").clear();
}

fn unexpected_response(expected: &str, response: &ResponseStatus) -> Error {
    Error::Protocol(format!("expected {expected} response, got {response:?}"))
}

#[cfg(test)]
#[path = "../client_tests.rs"]
mod tests;
