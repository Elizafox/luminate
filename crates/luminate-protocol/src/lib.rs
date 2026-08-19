// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Internal wire types and framing shared by the daemon and its clients.

/// Exact compatibility version for the client ↔ daemon request protocol.
///
/// Increment this whenever either peer could misinterpret the other's framed
/// request, response, handshake, authentication, or payload representation.
pub const PROTOCOL_ABI_VERSION: u32 = 30;

/// Exact compatibility version for the additive event protocol.
///
/// This stays independent from [`PROTOCOL_ABI_VERSION`] so clients which do
/// not subscribe are unaffected by event-only changes.
pub const EVENT_PROTOCOL_VERSION: u32 = 8;

/// Version of the bounded executable authentication-provider protocol.
pub const AUTHENTICATION_PROVIDER_PROTOCOL_VERSION: u32 = 1;

pub mod authentication;
pub mod authentication_provider;
pub mod event;
pub mod framing;
pub mod handshake;
pub mod management;
pub mod message;
pub mod server_info;
pub mod setup;

pub use authentication::{
    AttestationMetadata, Authentication, AuthenticationRequest, AuthenticationResponse,
    AuthenticationSource, Continuation, ContinuationError, Credential, EventTicket,
    SessionMetadata, TokenMetadata,
};
pub use authentication_provider::{
    ProviderHello, ProviderHelloResponse, ProviderIdentity, ProviderRequest, ProviderResponse,
    validate_identity,
};
pub use event::{Event, EventCompatibility, SubscribeAck, SubscribeHello};
pub use handshake::{ClientHello, Compatibility, DaemonHello};
pub use luminate_core::scene::{
    Scene, SceneBinding, SceneCaptureMode, SceneId, SceneTargetState, SceneValidationError,
};
pub use management::{
    DaemonPreferences, DeviceReconciliationPreference, ManagedPlugin, ManagementChange,
    ManagementChangeSet, ManagementMutation, ManagementPatch, ManagementSnapshot,
    PluginRuntimeState, PluginSettingApplyMode, PluginSettingKind, PluginSettingSchema,
    ReportedSettingValue, SettingValue, WriteOnly,
};
pub use message::{
    ErrorCode, OperationError, Request, RequestMessage, Response, ResponseMessage, ResponseStatus,
    Selector, SetAppearanceSlotsRequest, SetBrightnessRequest, SetEffectRequest,
    StartTransitionRequest, TransitionDestination, TransitionSource, UnsupportedPolicy,
};
pub use server_info::ServerInfo;
pub use setup::{
    PluginSetupChoice, PluginSetupInteractionResponse, PluginSetupSession, PluginSetupSessionId,
    PluginSetupSessionState, PluginSetupWorkflow, PluginSetupWorkflowKind,
};

#[cfg(test)]
#[path = "setup_tests.rs"]
mod setup_tests;
