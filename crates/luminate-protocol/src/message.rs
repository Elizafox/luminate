// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Request, response, and error messages carried by the control protocol.

use std::time::SystemTime;

use luminate_core::state;

use serde::{Deserialize, Serialize};

use luminate_core::appearance_slot::AppearanceSlotValue;
use luminate_core::collection::{Collection, CollectionCategory, CollectionId, CollectionMember};
pub use luminate_core::control::{Selector, UnsupportedPolicy};
use luminate_core::device::{Device, DeviceId};
use luminate_core::effect::Effect;
use luminate_core::frame::FrameEnvelope;
use luminate_core::policy::{PolicyDocumentSource, PolicyRevision, PrincipalId};
use luminate_core::scene::{Scene, SceneBinding, SceneCaptureMode, SceneId};
use luminate_core::shm_frame::ShmPixelFormat;
use luminate_core::target::TargetId;
use luminate_core::transition::{
    TransitionId, TransitionOptions, TransitionStatus, TransitionTargetState,
};

use crate::server_info::ServerInfo;
use crate::{
    Credential, ManagementChangeSet, ManagementPatch, ManagementSnapshot,
    PluginSetupInteractionResponse, PluginSetupSession, PluginSetupSessionId, PluginSetupWorkflow,
    TokenMetadata,
};

/// Operation requested by a client on the primary control connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// Checks that the established control connection and daemon request loop
    /// are responsive without disclosing daemon metadata or authorizing an
    /// action. `Response` carries [`ResponseStatus::Ack`].
    Ping,

    /// Issues a short-lived, single-use ticket for an event connection bound
    /// to this authenticated control session.
    IssueEventTicket,

    /// Returns daemon identity and protocol metadata.
    ServerInfo,

    /// Lists devices in the current topology.
    ListDevices,

    /// Lists device identifiers for which state is retained even though the
    /// device is absent from the current topology.
    ListWithdrawnDevices,

    /// Looks up a device in the current topology.
    GetDevice {
        /// Device to look up.
        id: DeviceId,
    },

    /// Returns the authoritative state retained for a device.
    GetState {
        /// Device whose state is requested.
        device: DeviceId,
    },

    /// Returns the resolved state of a collection.
    GetCollectionState {
        /// Collection whose state is requested.
        collection: CollectionId,
    },

    /// Refreshes a device's observed state from its plugin.
    RefreshState {
        /// Device to refresh.
        device: DeviceId,
    },

    /// Permanently removes retained state for a device that is not present in
    /// the current topology. Active devices are rejected.
    PurgeWithdrawnDevice {
        /// Withdrawn device whose retained state should be removed.
        device: DeviceId,
    },

    /// Asks the daemon to re-enumerate every plugin's hardware and reconcile
    /// any topology or state changes, as it does after resuming from suspend.
    ///
    /// Exists so an operator, an init system, or a sleep hook can drive a
    /// rescan on a platform where Luminate has no native suspend/resume
    /// hook. It also provides a recovery path when hardware has re-enumerated
    /// without triggering the usual notification. `SIGUSR1` provides the same
    /// operation for callers that cannot speak this protocol.
    ///
    /// Acknowledged as soon as the rescan is scheduled, not once it
    /// finishes: re-enumerating network plugins can take a while, and the
    /// daemon's topology coordinator owns that work. Observe the resulting
    /// `Event::TopologyChanged`/`Event::StateChanged` to learn what actually
    /// changed. `Response` carries [`ResponseStatus::Ack`].
    Rescan,

    /// Unloads a loaded plugin by name: terminates its host process.
    ///
    /// The plugin's devices disappear from the topology as withdrawn devices.
    /// Their configured state and observations remain available until an
    /// explicit [`Self::PurgeWithdrawnDevice`] request or a plugin reload.
    ///
    /// `Response` carries [`ResponseStatus::Ack`] once the plugin has
    /// actually been unloaded and the resulting topology change committed.
    UnloadPlugin {
        /// Canonical name of the plugin to unload.
        name: String,
    },

    /// Reloads a loaded plugin by name: terminates its host process and
    /// starts a fresh one from the same configured plugin entry.
    ///
    /// The restart rereads the plugin binary and plugin-specific configuration
    /// from disk. It does not reread daemon configuration or rerun plugin
    /// discovery.
    ///
    /// `Response` carries [`ResponseStatus::Ack`] once the replacement host
    /// is running and its topology has been committed. A follow-up rescan
    /// is scheduled automatically to restore hardware state, the same way
    /// resume-from-suspend does.
    ReloadPlugin {
        /// Canonical name of the plugin to reload.
        name: String,
    },

    /// Reads the authoritative managed configuration and resolved runtime
    /// view. `Response` carries [`ResponseStatus::ManagementSnapshot`].
    GetManagement,

    /// Validates and durably applies one optimistic-concurrency transaction.
    ///
    /// A stale [`ManagementPatch::expected_revision`] is returned as
    /// [`ErrorCode::Conflict`]. `Response` carries
    /// [`ResponseStatus::ManagementPatched`] after the new revision is
    /// persisted.
    PatchManagement {
        /// Transaction to validate and commit.
        patch: ManagementPatch,
    },

    /// Lists the setup workflows advertised by one installed plugin.
    ///
    /// Plugins advertise no workflows by default. `Response` carries
    /// [`ResponseStatus::PluginSetupWorkflows`].
    ListPluginSetupWorkflows {
        /// Canonical installed plugin name.
        plugin: String,
    },

    /// Starts one advertised plugin setup workflow.
    StartPluginSetup {
        /// Canonical installed plugin name.
        plugin: String,
        /// Stable plugin-local workflow ID.
        workflow: String,
    },

    /// Responds to the current interaction in a setup session.
    RespondPluginSetup {
        /// Daemon-generated session identifier.
        session: PluginSetupSessionId,
        /// Interaction generation observed by the client.
        generation: u64,
        /// Typed interaction response.
        response: PluginSetupInteractionResponse,
    },

    /// Reads the current state of a setup session.
    GetPluginSetup {
        /// Daemon-generated session identifier.
        session: PluginSetupSessionId,
    },

    /// Cancels an active setup session.
    CancelPluginSetup {
        /// Daemon-generated session identifier.
        session: PluginSetupSessionId,
    },

    /// Returns the active access-policy document.
    GetAccessPolicy,

    /// Replaces the access-policy document if its revision is still current.
    ReplaceAccessPolicy {
        /// Revision against which to apply the replacement.
        expected: PolicyRevision,

        /// Complete replacement policy document.
        replacement: PolicyDocumentSource,
    },

    /// Creates a daemon-managed bearer token.
    CreateToken {
        /// Stable, non-secret token identifier.
        id: String,

        /// Principal authenticated by the token.
        subject: PrincipalId,

        /// Optional credential expiry.
        expires_at: Option<SystemTime>,
    },

    /// Lists daemon-managed bearer tokens without exposing their secrets.
    ListTokens,

    /// Replaces a bearer token's secret and optional expiry.
    RotateToken {
        /// Stable identifier of the token to rotate.
        id: String,

        /// New optional credential expiry.
        expires_at: Option<SystemTime>,
    },

    /// Revokes a daemon-managed bearer token.
    RevokeToken {
        /// Stable identifier of the token to revoke.
        id: String,
    },

    /// Creates an actor-bound front-end attestation.
    CreateAttestation {
        /// Administrator-selected attestation name.
        name: String,

        /// Principal authenticated by the attestation.
        subject: PrincipalId,

        /// Groups the front end may attest for the principal.
        verified_groups: Vec<String>,

        /// Optional credential expiry.
        expires_at: Option<SystemTime>,
    },

    /// Lists configured front-end attestations without exposing their secrets.
    ListAttestations,

    /// Revokes a configured front-end attestation.
    RevokeAttestation {
        /// Administrator-selected attestation name.
        name: String,
    },

    /// Applies a hardware effect to the selected targets.
    SetEffect(SetEffectRequest),

    /// Stores named appearance programs on one concrete surface.
    SetAppearanceSlots(SetAppearanceSlotsRequest),

    /// Sets brightness on the selected targets.
    SetBrightness(SetBrightnessRequest),

    /// Reapplies each selected target's last-known configured appearance.
    RestoreAppearance {
        /// Targets whose configured appearance should be restored.
        target: Selector,
    },
    /// Clears the selected targets.
    ClearTarget {
        /// Targets to clear.
        target: Selector,
    },
    /// Saves the selected targets' current appearance as their configured state.
    SaveCurrent {
        /// Targets whose current appearance should be saved.
        target: Selector,
    },

    /// Creates a collection owned by the calling principal. `members` is
    /// validated to contain neither self-references nor cycles before the
    /// collection is created; the assigned ID is returned as
    /// [`ResponseStatus::CollectionCreated`]. The daemon derives the owner
    /// from the authenticated caller; ownership is not accepted from the
    /// request.
    CreateCollection {
        /// Human-readable collection name.
        name: String,

        /// Optional human-readable description.
        description: Option<String>,

        /// Optional collection category.
        kind: Option<CollectionCategory>,

        /// Initial explicit members.
        members: Vec<CollectionMember>,
    },

    /// Destroys a collection. Refused if another collection still
    /// references it, or if the caller doesn't own it.
    DestroyCollection {
        /// Collection to destroy.
        id: CollectionId,
    },

    /// Adds one member to a collection's explicit membership.
    AddCollectionMember {
        /// Collection to modify.
        id: CollectionId,

        /// Member to add.
        member: CollectionMember,
    },

    /// Removes one member from a collection's explicit membership.
    RemoveCollectionMember {
        /// Collection to modify.
        id: CollectionId,

        /// Member to remove.
        member: CollectionMember,
    },

    /// Lists every collection currently registered.
    ListCollections,

    /// Looks up one collection by id.
    GetCollection {
        /// Collection to look up.
        id: CollectionId,
    },

    /// Creates a scene from caller-supplied bindings.
    CreateScene {
        /// Human-readable scene name.
        name: String,

        /// Optional human-readable description.
        description: Option<String>,

        /// Initial scene bindings.
        bindings: Vec<SceneBinding>,
    },

    /// Captures current target state into a new scene.
    CaptureScene {
        /// Human-readable scene name.
        name: String,

        /// Optional human-readable description.
        description: Option<String>,

        /// State-capture strategy.
        mode: SceneCaptureMode,

        /// Targets to capture.
        targets: Vec<TargetId>,
    },

    /// Replaces a scene if its revision is still current.
    ReplaceScene {
        /// Scene to replace.
        id: SceneId,

        /// Revision against which to apply the replacement.
        expected_revision: u64,

        /// Replacement scene name.
        name: String,

        /// Replacement description.
        description: Option<String>,

        /// Replacement bindings.
        bindings: Vec<SceneBinding>,
    },

    /// Recaptures a scene if its revision is still current.
    RecaptureScene {
        /// Scene to recapture.
        id: SceneId,

        /// Revision against which to apply the recapture.
        expected_revision: u64,

        /// State-capture strategy.
        mode: SceneCaptureMode,

        /// Targets to capture.
        targets: Vec<TargetId>,
    },

    /// Deletes a scene if its revision is still current.
    DeleteScene {
        /// Scene to delete.
        id: SceneId,

        /// Revision against which to apply the deletion.
        expected_revision: u64,
    },

    /// Lists all persistent scenes.
    ListScenes,

    /// Looks up a persistent scene.
    GetScene {
        /// Scene to look up.
        id: SceneId,
    },

    /// Applies a persistent scene.
    ApplyScene {
        /// Scene to apply.
        id: SceneId,

        /// Optional explicit target subset authorized by a front end. Ordinary
        /// clients use `None`; a multi-principal façade supplies the targets
        /// to prevent the daemon from using the façade's broader socket
        /// authority.
        authorized_targets: Option<Vec<TargetId>>,
    },

    /// Starts a daemon-owned transition after complete endpoint preflight.
    StartTransition(StartTransitionRequest),

    /// Returns an active or retained terminal transition snapshot.
    GetTransition {
        /// Transition to look up.
        id: TransitionId,
    },

    /// Aborts a transition and waits until no later step can write.
    AbortTransition {
        /// Transition to abort.
        id: TransitionId,
    },

    /// Renews a delegated transition's watchdog lease.
    RenewTransition {
        /// Transition whose lease should be renewed.
        id: TransitionId,
        /// New lease duration in milliseconds.
        lease_ms: u64,
    },

    /// Starts a frame stream on `target`. Rejected if `target` doesn't
    /// advertise frame-upload capability, another stream is already active
    /// on it, or an incompatible hardware effect is active. `Response`
    /// carries [`ResponseStatus::FrameStreamStarted`] on success. If device
    /// topology changes during authorization, the daemon returns
    /// [`ErrorCode::Conflict`] and the caller may retry from discovery.
    BeginFrameStream {
        /// Target that will receive frames.
        target: TargetId,
    },

    /// Uploads one frame to a target with an active stream.
    /// `envelope.generation` must match the generation
    /// [`ResponseStatus::FrameStreamStarted`] returned; a stale or
    /// out-of-order `envelope.sequence` is rejected rather than applied.
    /// `Response` carries [`ResponseStatus::FrameAck`] on success. A topology
    /// replacement terminates the old stream and returns [`ErrorCode::Conflict`];
    /// the caller must discover the replacement target and begin a new stream.
    UploadFrame {
        /// Target receiving the frame.
        target: TargetId,

        /// Generation-bound frame payload.
        envelope: FrameEnvelope,
    },

    /// Ends the frame stream on `target` with the given `generation`,
    /// releasing it for ordinary mutations and other streams. The last
    /// successfully-applied frame remains the target's current state; there
    /// is no separate reconciliation step.
    EndFrameStream {
        /// Target whose stream should end.
        target: TargetId,

        /// Generation of the stream to end.
        generation: u32,
    },

    /// Negotiates the client → daemon shared-memory frame-streaming fast path on
    /// `target` as an explicit alternative to [`Self::BeginFrameStream`].
    ///
    /// The fast path is available only when the principal and daemon have the
    /// same user ID, the target advertises shared-memory support, client shared
    /// memory is preferred, and the target is not already streaming. Otherwise
    /// the daemon returns [`ResponseStatus::Error`] with
    /// [`ErrorCode::Unsupported`], and the caller can continue using
    /// [`Self::BeginFrameStream`].
    ///
    /// `Response` carries [`ResponseStatus::ShmFrameStreamReady`] on
    /// success.
    BeginShmFrameStream {
        /// Target that will receive shared-memory frames.
        target: TargetId,
    },

    /// Ends the client-published SHM stream on `target` with the given
    /// `generation`, mirroring [`Self::EndFrameStream`]'s idempotent
    /// posture: ending an already-ended or never-started stream, or one
    /// whose `generation` no longer matches, is a no-op success rather than
    /// an error.
    EndShmFrameStream {
        /// Target whose shared-memory stream should end.
        target: TargetId,

        /// Generation of the stream to end.
        generation: u32,
    },
}

/// One correlated request carried on the primary control connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMessage {
    /// Connection-local identifier echoed by the corresponding response.
    pub id: u64,

    /// Operation requested by the client.
    pub request: Request,
}

/// Parameters for applying a hardware effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetEffectRequest {
    /// Targets that should receive the effect.
    pub selector: Selector,

    /// Effect to apply.
    pub effect: Effect,

    /// Only used for a fan-out selector ([`Selector::Collection`] or
    /// [`Selector::Targets`]). `None` uses the daemon's configured default.
    pub on_unsupported: Option<UnsupportedPolicy>,
}

/// Parameters for storing named appearance programs on one surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetAppearanceSlotsRequest {
    /// Concrete surface whose slots should change.
    pub target: TargetId,

    /// Slot values in caller-defined order.
    pub values: Vec<AppearanceSlotValue>,
}

/// Parameters for changing target brightness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetBrightnessRequest {
    /// Targets whose brightness should change.
    pub target: Selector,

    /// Requested brightness value.
    pub value: u32,

    /// Only used for a fan-out selector ([`Selector::Collection`] or
    /// [`Selector::Targets`]). `None` uses the daemon's configured default.
    pub on_unsupported: Option<UnsupportedPolicy>,
}

/// Source endpoint of a transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionSource {
    /// Require exact fresh current observations.
    Current,

    /// Apply and persist this snapshotted scene before interpolation.
    Scene(SceneId),
}

/// Destination endpoint of a transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionDestination {
    /// Snapshot this scene and resolve its dynamic membership.
    Scene(SceneId),

    /// Caller-supplied sparse target states.
    TargetStates(Vec<TransitionTargetState>),
}

/// Wire input for starting a transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartTransitionRequest {
    /// State from which interpolation begins.
    pub source: TransitionSource,

    /// State towards which interpolation proceeds.
    pub destination: TransitionDestination,

    /// Timing and interpolation behaviour.
    pub options: TransitionOptions,

    /// Optional explicit target subset authorized by a front end.
    pub authorized_targets: Option<Vec<TargetId>>,

    /// Renewable delegated lease. `None` makes the transition daemon-owned.
    pub renewable_lease_ms: Option<u64>,
}

/// Result of one control-protocol request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    /// Request outcome and any returned data.
    pub status: ResponseStatus,
}

/// One correlated response carried on the primary control connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseMessage {
    /// Connection-local identifier copied from the corresponding request.
    pub id: u64,

    /// Result returned by the daemon.
    pub response: Response,
}

/// Successful result or structured failure returned for a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResponseStatus {
    /// Daemon identity and protocol metadata.
    ServerInfo(ServerInfo),

    /// Devices in the current topology.
    Devices(Vec<Device>),

    /// Device identifiers with retained state but no current topology entry.
    WithdrawnDevices(Vec<DeviceId>),

    /// Requested device, or `None` if it was not found.
    Device(Option<Box<Device>>),

    /// Requested device state, or `None` if no state is retained.
    State(Option<Box<state::DeviceStateStatus>>),

    /// Requested collection state, or `None` if the collection was not found.
    CollectionState(Option<Box<state::CollectionStateStatus>>),

    /// The request completed without additional response data.
    Ack,

    /// Short-lived, single-use ticket for an event connection.
    EventTicket(crate::EventTicket),

    /// A frame stream was started; `generation` must be echoed back in every
    /// subsequent `UploadFrame`/`EndFrameStream` request for this stream.
    FrameStreamStarted {
        /// Connection-local generation assigned to the stream.
        generation: u32,
    },

    /// One frame was processed. `dropped` is `true` when the frame was
    /// rate-limited and never reached the plugin, rather than applied.
    FrameAck {
        /// Sequence number copied from the processed frame.
        sequence: u64,

        /// Whether rate limiting prevented the frame from reaching the plugin.
        dropped: bool,
    },

    /// A collection was created; `id` is the server-generated identifier to
    /// use in subsequent requests.
    CollectionCreated {
        /// Server-generated collection identifier.
        id: CollectionId,
    },

    /// All collections visible to the caller.
    Collections(Vec<Collection>),

    /// Requested collection, or `None` if it was not found.
    CollectionInfo(Option<Box<Collection>>),

    /// Scene created or captured by the request.
    Scene(Box<Scene>),

    /// All scenes visible to the caller.
    Scenes(Vec<Scene>),

    /// Requested scene, or `None` if it was not found.
    SceneInfo(Option<Box<Scene>>),

    /// Result of applying a scene through an authorized target subset.
    SceneApplied {
        /// Targets successfully changed.
        applied: Vec<TargetId>,

        /// Targets excluded by authorization.
        denied: Vec<TargetId>,
    },
    /// Transition created or queried successfully.
    Transition(Box<TransitionStatus>),

    /// Authoritative managed configuration and its resolved runtime view.
    ManagementSnapshot(Box<ManagementSnapshot>),

    /// Redacted record of one committed managed-configuration transaction.
    ManagementPatched(ManagementChangeSet),

    /// Setup workflows advertised by the requested installed plugin.
    PluginSetupWorkflows(Vec<PluginSetupWorkflow>),

    /// Current state of a daemon-owned plugin setup session.
    PluginSetupSession(Box<PluginSetupSession>),

    /// Active access-policy document.
    AccessPolicy(Box<PolicyDocumentSource>),

    /// Newly created token metadata and its one-time secret.
    TokenCreated {
        /// Public metadata for the token.
        metadata: TokenMetadata,
        /// Secret credential returned only at creation or rotation.
        secret: Credential,
    },

    /// Public metadata for configured bearer tokens.
    Tokens(Vec<TokenMetadata>),

    /// Newly created attestation metadata and its one-time secret.
    AttestationCreated {
        /// Public metadata for the attestation.
        metadata: crate::AttestationMetadata,

        /// Secret credential returned only at creation.
        secret: Credential,
    },

    /// Public metadata for configured front-end attestations.
    Attestations(Vec<crate::AttestationMetadata>),

    /// Result of mutating a collection under best-effort authorization.
    ///
    /// Authorization-based skipping is a normal result. A hardware failure
    /// after some authorized targets changed instead returns
    /// [`ErrorCode::PartialMutation`] and records those targets in
    /// [`OperationError::applied_targets`].
    CollectionApplied {
        /// Authorized leaf targets that changed successfully.
        applied: Vec<TargetId>,

        /// Leaf targets excluded by authorization.
        denied: Vec<TargetId>,
    },

    /// The client → daemon SHM fast path was negotiated for a stream; the
    /// client creates/attaches to the named services and, from then on,
    /// publishes samples directly rather than sending
    /// [`Request::UploadFrame`]. See [`Request::BeginShmFrameStream`].
    ShmFrameStreamReady {
        /// Echoed in every sample's header and in
        /// [`Request::EndShmFrameStream`]; matches
        /// [`Self::FrameStreamStarted::generation`]'s role on the ordinary
        /// path.
        generation: u32,

        /// Deterministic, target-derived iceoryx2 publish-subscribe service
        /// name the client must open to publish sample data.
        service_name: String,

        /// Deterministic, target-derived iceoryx2 event service name the
        /// client must open to notify the daemon a sample is ready. Sent
        /// explicitly rather than left for the client to derive from
        /// `service_name` by convention, so the naming scheme stays free to
        /// change without becoming part of this wire contract.
        event_service_name: String,

        /// Pixel format every sample on this stream must be encoded in.
        pixel_format: ShmPixelFormat,

        /// Must be carried in every sample's header
        /// (`ShmClientFrameHeader::stream_nonce`); lets the daemon detect a
        /// stale same-user segment left behind by a crashed and respawned
        /// client reusing the same deterministic service name.
        stream_nonce: u64,

        /// Byte size of the shared-memory slice each sample occupies
        /// (header plus packed pixel payload), for the client to size its
        /// loan.
        segment_bytes: u32,
    },
    /// The request failed.
    Error(OperationError),
}

/// A structured operation failure returned by the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationError {
    /// Stable machine-readable failure category.
    pub code: ErrorCode,

    /// Human-readable diagnostic; consumers must not parse it for control flow.
    pub message: String,

    /// Minimum delay before retrying, when the provider supplied one.
    pub retry_after_ms: Option<u64>,

    /// Targets successfully changed before a partial fan-out failure.
    pub applied_targets: Vec<TargetId>,
}

/// Stable machine-readable category for an operation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    /// The daemon could not serve the request.
    DaemonUnavailable,

    /// The supplied credentials were rejected.
    AuthenticationFailed,

    /// The authenticated principal is not authorized for the operation.
    PermissionDenied,

    /// The requested object does not exist.
    NotFound,

    /// The requested operation or capability is not supported.
    Unsupported,

    /// Required authoritative state is unavailable.
    UnknownState,

    /// A request argument is invalid.
    InvalidArgument,

    /// An unexpected internal failure occurred.
    Internal,

    /// An input or output operation failed.
    Io,

    /// A required service or resource is temporarily unavailable.
    Unavailable,

    /// The request exceeded an applicable rate limit.
    RateLimited,

    /// A fan-out operation failed after changing some targets.
    PartialMutation,

    /// Complete preflight proved that no safe transition can be constructed.
    TransitionImpossible,

    /// A frame stream or hardware effect already owns the target in a way
    /// incompatible with the requested operation (e.g. starting a stream
    /// where a non-concurrent effect is active, or starting a second stream
    /// on a target that already has one).
    Conflict,
}

#[cfg(test)]
mod tests {
    use luminate_core::capability::CapabilitySet;
    use luminate_core::device::{Device, DeviceId};
    use luminate_core::element::{Element, ElementId, ElementKind};
    use luminate_core::surface::{Surface, SurfaceId, SurfaceKind};

    use super::{Response, ResponseStatus};

    #[test]
    fn complete_device_response_round_trip_preserves_ordered_scoped_physical_tags() {
        let response = Response {
            status: ResponseStatus::Devices(vec![Device {
                id: DeviceId::new("beam"),
                name: "Beam".to_owned(),
                vendor: Some("Example".to_owned()),
                model: None,
                provider_instance: Some("example".to_owned()),
                surfaces: vec![Surface {
                    id: SurfaceId::new("light-bars"),
                    name: "Light bars".to_owned(),
                    kind: SurfaceKind::Linear { length: 2.0 },
                    physical_tags: vec![
                        "layout:horizontal".to_owned(),
                        "example:surface".to_owned(),
                    ],
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
                physical_tags: vec![
                    "shape:modular-light-bar".to_owned(),
                    "example:faceted".to_owned(),
                ],
                host_attached: false,
                notes: Vec::new(),
                warnings: Vec::new(),
            }]),
        };
        let mut encoded = Vec::new();
        ciborium::into_writer(&response, &mut encoded).expect("encode response");
        let decoded: Response = ciborium::from_reader(encoded.as_slice()).expect("decode response");
        let ResponseStatus::Devices(devices) = decoded.status else {
            panic!("expected devices response");
        };

        assert_eq!(
            devices[0].physical_tags,
            ["shape:modular-light-bar", "example:faceted"]
        );
        assert_eq!(
            devices[0].surfaces[0].physical_tags,
            ["layout:horizontal", "example:surface"]
        );
        assert_eq!(
            devices[0].surfaces[0].elements[0].physical_tags,
            ["shape:round", "position:left"]
        );
    }
}
