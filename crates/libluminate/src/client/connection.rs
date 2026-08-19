// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

use super::{
    Arc, AtomicU64, Authentication, AuthenticationAdministration, AuthenticationRequest,
    AuthenticationResponse, Client, ClientBuilder, ClientHello, CollectionId, Compatibility,
    Connection, DaemonHello, Device, DeviceId, Error, EventSubscription, HashMap, IO_TIMEOUT,
    PROTOCOL_ABI_VERSION, Path, PathBuf, PolicyAdministration, Request, ResponseStatus, Result,
    ServerInfo, SessionMetadata, SessionScope, SyncMutex, Transitions, event_socket_path, mpsc,
    read_responses, receive, send, split, state, timeout, unexpected_response, write_requests,
};

impl Client {
    /// Returns grouped daemon access-policy administration operations.
    #[must_use]
    pub const fn policy_administration(&self) -> PolicyAdministration<'_> {
        PolicyAdministration { client: self }
    }

    /// Returns grouped daemon authentication administration operations.
    #[must_use]
    pub const fn authentication_administration(&self) -> AuthenticationAdministration<'_> {
        AuthenticationAdministration { client: self }
    }

    /// Starts configuring a single authenticated daemon connection.
    #[must_use]
    pub fn builder() -> ClientBuilder {
        ClientBuilder::new()
    }

    /// Returns this client's daemon-managed transition operations.
    #[must_use]
    pub const fn transitions(&self) -> Transitions<'_> {
        Transitions { client: self }
    }
    /// Connects to the daemon through its default IPC endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DaemonUnavailable`] if the socket doesn't exist or
    /// refuses the connection, [`Error::IncompatibleDaemon`] if the daemon's
    /// protocol ABI version isn't supported, or another [`Error`] variant for
    /// an I/O or protocol failure during the handshake.
    pub async fn connect() -> Result<Self> {
        ClientBuilder::new().connect().await
    }

    /// Connects to the daemon through the endpoint derived from `path`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DaemonUnavailable`] if the socket doesn't exist or
    /// refuses the connection, [`Error::IncompatibleDaemon`] if the daemon's
    /// protocol ABI version isn't supported, or another [`Error`] variant for
    /// an I/O or protocol failure during the handshake.
    pub async fn connect_path(path: impl AsRef<Path>) -> Result<Self> {
        ClientBuilder::new().path(path.as_ref()).connect().await
    }

    pub(super) async fn connect_stream(
        mut stream: Connection,
        socket_path: PathBuf,
        authentication: Authentication,
        scope: Option<SessionScope>,
    ) -> Result<Self> {
        timeout(
            IO_TIMEOUT,
            send(
                &mut stream,
                &ClientHello::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")),
            ),
        )
        .await??;

        // Bound peers that accept a connection without completing the hello.
        let hello: DaemonHello = timeout(IO_TIMEOUT, receive(&mut stream)).await??;
        let daemon_version = hello.daemon_version.clone();

        match hello.compatibility {
            Compatibility::Compatible => {
                // A compatible verdict and advertised version must agree.
                if hello.protocol_abi_version != PROTOCOL_ABI_VERSION {
                    return Err(Error::IncompatibleDaemon {
                        daemon_version,
                        supported_protocol_abi_version: hello.protocol_abi_version,
                        reason: Some(format!(
                            "daemon reported compatibility but advertised protocol ABI version \
                             {}, which does not match this client's {PROTOCOL_ABI_VERSION}",
                            hello.protocol_abi_version
                        )),
                    });
                }

                timeout(
                    IO_TIMEOUT,
                    send(
                        &mut stream,
                        &AuthenticationRequest {
                            authentication,
                            scope,
                        },
                    ),
                )
                .await??;
                let authentication: AuthenticationResponse =
                    timeout(IO_TIMEOUT, receive(&mut stream)).await??;
                let session = match authentication {
                    AuthenticationResponse::Authenticated { session } => {
                        SessionMetadata::from(&session)
                    }
                    AuthenticationResponse::Rejected { reason } => {
                        return Err(Error::AuthenticationFailed(reason));
                    }
                };

                let (reader, writer) = split(stream);
                let (requests, request_rx) = mpsc::unbounded_channel();
                let pending = Arc::new(SyncMutex::new(HashMap::new()));

                tokio::spawn(write_requests(writer, request_rx, Arc::clone(&pending)));
                tokio::spawn(read_responses(reader, Arc::clone(&pending)));

                Ok(Self {
                    requests,
                    pending,
                    next_request_id: AtomicU64::new(0),
                    socket_path,
                    daemon_version,
                    session,
                })
            }
            Compatibility::Incompatible {
                supported_protocol_abi_version,
                reason,
            } => Err(Error::IncompatibleDaemon {
                daemon_version,
                supported_protocol_abi_version,
                reason,
            }),
        }
    }

    #[must_use]
    /// Returns the daemon version recorded during the compatibility handshake.
    ///
    /// This string is owned by the client and remains valid until the client is
    /// dropped.
    #[inline]
    pub fn daemon_version(&self) -> &str {
        &self.daemon_version
    }

    /// Returns sanitized metadata for this authenticated connection.
    #[must_use]
    pub const fn session(&self) -> &SessionMetadata {
        &self.session
    }

    #[must_use]
    /// Returns the primary daemon socket path this client connected to.
    #[inline]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Registers for daemon events on the socket conventionally derived from
    /// this client's primary socket path.
    ///
    /// # Errors
    ///
    /// Returns event connection, framing, timeout, compatibility, or ticket
    /// errors.
    pub async fn subscribe(&self) -> Result<EventSubscription> {
        self.subscribe_path(event_socket_path(&self.socket_path))
            .await
    }

    /// Registers for daemon events on an explicit event socket path.
    ///
    /// # Errors
    ///
    /// Returns event connection, framing, timeout, compatibility, or ticket
    /// errors.
    ///
    pub async fn subscribe_path(&self, path: impl AsRef<Path>) -> Result<EventSubscription> {
        let ticket = match self.request(Request::IssueEventTicket).await?.status {
            ResponseStatus::EventTicket(ticket) => ticket,
            other @ (ResponseStatus::ServerInfo(_)
            | ResponseStatus::Devices(_)
            | ResponseStatus::WithdrawnDevices(_)
            | ResponseStatus::Device(_)
            | ResponseStatus::State(_)
            | ResponseStatus::CollectionState(_)
            | ResponseStatus::Ack
            | ResponseStatus::FrameStreamStarted { .. }
            | ResponseStatus::FrameAck { .. }
            | ResponseStatus::CollectionCreated { .. }
            | ResponseStatus::Collections(_)
            | ResponseStatus::CollectionInfo(_)
            | ResponseStatus::Scene(_)
            | ResponseStatus::Scenes(_)
            | ResponseStatus::SceneInfo(_)
            | ResponseStatus::SceneApplied { .. }
            | ResponseStatus::Transition(_)
            | ResponseStatus::ManagementSnapshot(_)
            | ResponseStatus::ManagementPatched(_)
            | ResponseStatus::PluginSetupWorkflows(_)
            | ResponseStatus::PluginSetupSession(_)
            | ResponseStatus::AccessPolicy(_)
            | ResponseStatus::TokenCreated { .. }
            | ResponseStatus::Tokens(_)
            | ResponseStatus::AttestationCreated { .. }
            | ResponseStatus::Attestations(_)
            | ResponseStatus::CollectionApplied { .. }
            | ResponseStatus::ShmFrameStreamReady { .. }
            | ResponseStatus::Error(_)) => {
                return Err(unexpected_response("event ticket", &other));
            }
        };
        EventSubscription::connect_path_with_ticket(path, ticket).await
    }

    #[must_use]
    /// Returns the conventional event-socket path derived from this client's
    /// primary socket path.
    pub fn event_socket_path(&self) -> PathBuf {
        event_socket_path(&self.socket_path)
    }

    /// Registers an event subscriber before fetching the authoritative device
    /// baseline, closing the subscribe-time race described by the protocol.
    ///
    /// # Errors
    ///
    /// Returns a subscription error or any error from [`Self::list_devices`].
    pub async fn subscribe_with_baseline(&self) -> Result<(EventSubscription, Vec<Device>)> {
        let subscription = self.subscribe().await?;
        let devices = self.list_devices().await?;
        Ok((subscription, devices))
    }

    /// Explicit-event-path variant of [`Self::subscribe_with_baseline`].
    ///
    /// # Errors
    ///
    /// Returns a subscription error or any error from [`Self::list_devices`].
    pub async fn subscribe_with_baseline_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<(EventSubscription, Vec<Device>)> {
        let subscription = self.subscribe_path(path).await?;
        let devices = self.list_devices().await?;
        Ok((subscription, devices))
    }

    /// Fetch the daemon's server info (name, version, protocol ABI version).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed, or another [`Error`]
    /// variant if the request itself fails or the daemon responds with an
    /// error.
    pub async fn server_info(&self) -> Result<ServerInfo> {
        match self.request(Request::ServerInfo).await?.status {
            ResponseStatus::ServerInfo(info) => Ok(info.into()),
            other @ (ResponseStatus::Devices(_)
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
            | ResponseStatus::ManagementSnapshot(_)
            | ResponseStatus::AccessPolicy(_)
            | ResponseStatus::TokenCreated { .. }
            | ResponseStatus::Tokens(_)
            | ResponseStatus::AttestationCreated { .. }
            | ResponseStatus::Attestations(_)
            | ResponseStatus::ManagementPatched(_)
            | ResponseStatus::Scene(_)
            | ResponseStatus::Scenes(_)
            | ResponseStatus::SceneInfo(_)
            | ResponseStatus::SceneApplied { .. }
            | ResponseStatus::Transition(_)
            | ResponseStatus::PluginSetupSession(_)
            | ResponseStatus::Error { .. }) => Err(unexpected_response("server info", &other)),
        }
    }

    /// Checks that the existing connection and daemon request loop are
    /// responsive.
    ///
    /// The daemon returns no metadata and performs no authorization. Calls are
    /// subject to a small per-connection rate limit.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed,
    /// [`Error::RateLimited`] if this connection has pinged too frequently, or
    /// another [`Error`] if the request fails.
    pub async fn ping(&self) -> Result<()> {
        self.expect_ack(Request::Ping).await
    }

    /// List every device the daemon currently knows about.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed, or another [`Error`]
    /// variant if the request itself fails or the daemon responds with an
    /// error.
    pub async fn list_devices(&self) -> Result<Vec<Device>> {
        match self.request(Request::ListDevices).await?.status {
            ResponseStatus::Devices(devices) => Ok(devices),
            other @ (ResponseStatus::ServerInfo(_)
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
            | ResponseStatus::Error { .. }) => Err(unexpected_response("device list", &other)),
        }
    }

    /// Lists device identifiers with retained state that are absent from the
    /// current topology.
    ///
    /// The returned identifiers are sorted lexically and can be passed to
    /// [`Self::purge_withdrawn_device`].
    ///
    /// # Errors
    ///
    /// Returns an authorization, transport, daemon, or protocol error if the
    /// request cannot be completed.
    pub async fn list_withdrawn_devices(&self) -> Result<Vec<DeviceId>> {
        match self.request(Request::ListWithdrawnDevices).await?.status {
            ResponseStatus::WithdrawnDevices(devices) => Ok(devices),
            other @ (ResponseStatus::ServerInfo(_)
            | ResponseStatus::Devices(_)
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
                Err(unexpected_response("withdrawn device list", &other))
            }
        }
    }

    /// Look up a single device by ID; `Ok(None)` if no such device exists.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed, or another [`Error`]
    /// variant if the request itself fails or the daemon responds with an
    /// error.
    pub async fn get_device(&self, id: DeviceId) -> Result<Option<Device>> {
        match self.request(Request::GetDevice { id }).await?.status {
            ResponseStatus::Device(device) => Ok(device.map(|device| *device)),
            other @ (ResponseStatus::ServerInfo(_)
            | ResponseStatus::Devices(_)
            | ResponseStatus::WithdrawnDevices(_)
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
            | ResponseStatus::Error { .. }) => Err(unexpected_response("device lookup", &other)),
        }
    }

    /// Returns observed hardware state and reconciliation diagnostics separately
    /// from desired replay state. `None` means the device is not present.
    ///
    /// # Errors
    ///
    /// Returns an error for connection, protocol, timeout, or daemon failures.
    pub async fn get_state(&self, device: DeviceId) -> Result<Option<state::DeviceStateStatus>> {
        match self.request(Request::GetState { device }).await?.status {
            ResponseStatus::State(state) => Ok(state.map(|state| *state)),
            other @ (ResponseStatus::ServerInfo(_)
            | ResponseStatus::Devices(_)
            | ResponseStatus::WithdrawnDevices(_)
            | ResponseStatus::Device(_)
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
            | ResponseStatus::Error { .. }) => Err(unexpected_response("device state", &other)),
        }
    }

    /// Returns configured and effective appearance synthesized across a
    /// collection's transitive membership. `None` means the collection is
    /// unknown.
    ///
    /// # Errors
    ///
    /// Returns an error for connection, protocol, timeout, or daemon failures.
    pub async fn get_collection_state(
        &self,
        collection: CollectionId,
    ) -> Result<Option<state::CollectionStateStatus>> {
        match self
            .request(Request::GetCollectionState { collection })
            .await?
            .status
        {
            ResponseStatus::CollectionState(state) => Ok(state.map(|state| *state)),
            other @ (ResponseStatus::ServerInfo(_)
            | ResponseStatus::Devices(_)
            | ResponseStatus::WithdrawnDevices(_)
            | ResponseStatus::Device(_)
            | ResponseStatus::State(_)
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
            | ResponseStatus::Error { .. }) => Err(unexpected_response("collection state", &other)),
        }
    }

    /// Requests a fresh hardware snapshot. Existing observations are retained
    /// as stale if the read fails.
    ///
    /// # Errors
    ///
    /// Returns an error when readback is unsupported or the request fails.
    pub async fn refresh_state(&self, device: DeviceId) -> Result<()> {
        self.expect_ack(Request::RefreshState { device }).await
    }

    /// Permanently removes retained daemon state for an absent device.
    ///
    /// This does not clear or mutate hardware. The daemon rejects a device that
    /// is still present, preventing accidental removal of live desired state.
    /// The presence check and purge are atomic with respect to topology
    /// changes, so an identifier that reappears after enumeration is not
    /// purged.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] when no retained state exists,
    /// [`Error::InvalidArgument`] when the device is active, or another client
    /// or persistence error if the request cannot be completed.
    pub async fn purge_withdrawn_device(&self, device: DeviceId) -> Result<()> {
        self.expect_ack(Request::PurgeWithdrawnDevice { device })
            .await
    }

    /// Asks the daemon to re-enumerate every plugin's hardware and reconcile
    /// the resulting topology changes, as it does on resume from suspend.
    ///
    /// Useful after hardware has re-enumerated without the daemon noticing,
    /// on a platform with no native suspend/resume hook, or when a device
    /// returns under a different kernel device node.
    ///
    /// Returns once the rescan is scheduled, not once it completes:
    /// re-enumerating network hardware can take a while. Subscribe to events
    /// and fetch authoritative topology and state after receiving a change.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PermissionDenied`] when the caller isn't authorized
    /// for daemon administration, or another [`Error`] variant for an
    /// I/O/protocol failure.
    pub async fn rescan(&self) -> Result<()> {
        self.expect_ack(Request::Rescan).await
    }
}
