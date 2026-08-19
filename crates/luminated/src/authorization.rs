// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Client authorization model and policy interface.
//!
//! ## Authorization model
//!
//! The model consists of a captured peer principal, an exhaustive semantic
//! request classification, and a policy trait.
//!
//! `luminated::daemon::dispatch_request` resolves every request's selector
//! to concrete [`Resource`]s before authorizing them. Collection selectors
//! are generally authorized as a single resource set, so denying any
//! member denies the entire request. Collection writes are the exception;
//! see `daemon/authz.rs::authorize_partial`.
//!
//! ## Policy implementations
//!
//! [`SocketAccessPolicy`] always allows, relying on the IPC endpoint's access
//! controls.
//!
//! The `[authorization]` configuration section selects the built-in socket
//! admission policy. Authenticated subject permissions are evaluated by the
//! daemon's validated access-policy engine.
//!
//! ## Async policy interface
//!
//! [`AuthorizationPolicy::authorize`] returns a boxed future rather than a
//! `Decision` directly so a single `dyn AuthorizationPolicy` can represent
//! synchronous implementations without requiring an async-trait dependency.
//!
//! Topology-derived resources are generation-pinned across asynchronous policy
//! evaluation and their eventual observation or execution. A replacement
//! topology produces a retryable authorization conflict instead of reusing a
//! decision made from stale metadata.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use luminate_core::collection::OwnerIdentity;
use luminate_core::policy::PrincipalId;
use luminate_core::policy::{
    AuthorizationRequest as SessionAuthorizationRequest, ManagedAccessPolicy,
    Principal as SessionPrincipal, SessionScope,
};
pub use luminate_core::policy::{Operation, Resource};
use luminate_platform::transport::PeerCredential;
use luminate_protocol::Request;

/// The connected peer, captured once at accept time and retained for the
/// connection's lifetime. `client_name`/`client_version` in the protocol
/// handshake are caller-supplied observability fields and must never be
/// treated as authority.
///
/// Both variants compile on every target; only construction (from a live OS
/// credential) and platform-specific comparisons are `cfg`-gated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    /// Captured from `SO_PEERCRED` at accept time.
    Unix {
        /// Kernel-supplied user ID of the connected process.
        uid: u32,

        /// Kernel-supplied primary group ID of the connected process.
        gid: u32,

        /// Kernel-supplied process ID, retained for auditing only. It must
        /// not be used as a stable policy identity: PIDs are reused.
        pid: Option<u32>,
    },

    /// Captured from a Windows named-pipe client via impersonation.
    Windows {
        /// The client's security identifier, in its canonical string form
        /// (`ConvertSidToStringSidW`).
        sid: String,

        /// The client's process ID (`GetNamedPipeClientProcessId`), retained
        /// for auditing only, same caveat as [`Principal::Unix::pid`].
        pid: Option<u32>,
    },
}

/// A `Hash + Eq` identity for grouping connections by principal (for
/// example, [`crate::daemon::listener`]'s per-principal connection rate
/// limiting).
///
/// Unlike [`Principal`]'s own `Eq`, this deliberately ignores `pid`: two
/// connections from the same UID or SID should be treated as the same
/// principal even if they originate from different (or reused) process
/// IDs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RateLimitKey {
    Uid(u32),
    Sid(String),
}

impl Principal {
    /// The connected process's ID, if the platform reports one. Retained for
    /// auditing only. This must never be a stable policy ID, as PIDs are reused.
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        match self {
            Self::Unix { pid, .. } | Self::Windows { pid, .. } => *pid,
        }
    }

    /// A `Hash + Eq` key identifying this principal for rate limiting.
    #[must_use]
    pub fn rate_limit_key(&self) -> RateLimitKey {
        match self {
            Self::Unix { uid, .. } => RateLimitKey::Uid(*uid),
            Self::Windows { sid, .. } => RateLimitKey::Sid(sid.clone()),
        }
    }

    /// Whether the daemon should offer the client → daemon shared-memory frame
    /// streaming fast path to this principal.
    ///
    /// This is the single authorization gate for the feature. Because
    /// iceoryx2 0.9.3 provides no cross-principal access control, the daemon
    /// offers shared-memory streaming only to a client running as the same OS
    /// user as itself. All other principals continue using the ordinary
    /// [`Request::BeginFrameStream`] transport unchanged, so shared-memory
    /// support is strictly an optimization, never a required capability.
    ///
    /// Widening this predicate is sufficient to enable future cross-principal
    /// support once an equivalent access-control mechanism exists on each
    /// supported platform (for example, POSIX ACLs on Unix).
    #[must_use]
    pub fn is_same_user_as_daemon(&self) -> bool {
        #[cfg(unix)]
        {
            use luminate_platform::identity::daemon_own_uid;
            match self {
                // The opposite-platform principal cannot represent this
                // daemon's OS user.
                Self::Unix { uid, .. } => *uid == daemon_own_uid(),
                Self::Windows { .. } => false,
            }
        }

        #[cfg(windows)]
        {
            use luminate_platform::windows::identity::daemon_own_sid;

            match self {
                // The opposite-platform principal cannot represent this
                // daemon's OS user.
                Self::Windows { sid, .. } => daemon_own_sid() == *sid,
                Self::Unix { .. } => false,
            }
        }
    }

    /// Whether this peer is an intrinsic daemon recovery identity.
    ///
    /// This is derived only from the kernel credential captured at admission;
    /// caller-supplied names, groups, and process metadata cannot create it.
    #[must_use]
    pub fn is_intrinsic_recovery(&self) -> bool {
        #[cfg(unix)]
        {
            use luminate_platform::identity::daemon_own_uid;

            matches!(self, Self::Unix { uid, .. } if *uid == 0 || *uid == daemon_own_uid())
        }

        #[cfg(windows)]
        {
            use luminate_platform::windows::identity::daemon_own_sid;

            matches!(self, Self::Windows { sid, .. } if sid == daemon_own_sid())
        }
    }
}

impl From<PeerCredential> for Principal {
    fn from(credential: PeerCredential) -> Self {
        match credential {
            PeerCredential::Unix { uid, gid, pid } => Self::Unix { uid, gid, pid },
            PeerCredential::Windows { sid, pid } => Self::Windows { sid, pid },
        }
    }
}

impl From<&Principal> for OwnerIdentity {
    /// Unix and Windows principals remain distinct identities. There is no
    /// cross-platform identity equivalence, so conversion preserves the
    /// platform-specific identity and therefore fails closed.
    fn from(principal: &Principal) -> Self {
        match principal {
            Principal::Unix { uid, .. } => {
                PrincipalId::new("unix", uid.to_string()).map_or(Self::Uid(*uid), Self::Principal)
            }
            Principal::Windows { sid, .. } => PrincipalId::new("windows", sid.clone())
                .map_or_else(|_| Self::Sid(sid.clone()), Self::Principal),
        }
    }
}

/// Semantic classification of a consumer protocol request, independent of
/// its wire representation.
///
/// Every [`Request`] variant maps to exactly one operation. Because
/// [`operation_for`] uses an exhaustive match, adding a request variant
/// requires an explicit classification and cannot silently inherit
/// permission from a catch-all default.
/// Returns the semantic [`Operation`] class for an authorization-bearing
/// request. Authorization-free liveness requests return `None`.
#[must_use]
pub const fn operation_for(request: &Request) -> Option<Operation> {
    match request {
        Request::Ping | Request::IssueEventTicket => None,
        Request::ServerInfo
        | Request::ListDevices
        | Request::GetDevice { .. }
        | Request::ListCollections
        | Request::GetCollection { .. }
        | Request::ListScenes
        | Request::GetScene { .. }
        | Request::GetState { .. }
        | Request::GetCollectionState { .. } => Some(Operation::Observe),
        Request::RefreshState { .. } => Some(Operation::Refresh),
        // `Rescan` is daemon-wide administration, not a device operation: it
        // names no target, and the reconciliation it triggers runs under each
        // device's already-configured policy rather than anything the caller
        // chooses. It shares `PurgeWithdrawnDevice`'s class for this reason.
        Request::ListWithdrawnDevices
        | Request::PurgeWithdrawnDevice { .. }
        | Request::Rescan
        | Request::UnloadPlugin { .. }
        | Request::ReloadPlugin { .. } => Some(Operation::DaemonAdministration),
        Request::GetManagement
        | Request::PatchManagement { .. }
        | Request::ListPluginSetupWorkflows { .. }
        | Request::StartPluginSetup { .. }
        | Request::RespondPluginSetup { .. }
        | Request::GetPluginSetup { .. }
        | Request::CancelPluginSetup { .. } => Some(Operation::ManagePlugins),
        Request::GetAccessPolicy | Request::ReplaceAccessPolicy { .. } => {
            Some(Operation::ManagePolicy)
        }
        Request::CreateToken { .. }
        | Request::ListTokens
        | Request::RotateToken { .. }
        | Request::RevokeToken { .. } => Some(Operation::ManageAuthentication),
        Request::CreateAttestation { .. }
        | Request::ListAttestations
        | Request::RevokeAttestation { .. } => Some(Operation::AdministerFrontend),
        Request::SetEffect(_)
        | Request::SetAppearanceSlots(_)
        | Request::SetBrightness(_)
        | Request::RestoreAppearance { .. }
        | Request::ClearTarget { .. }
        | Request::BeginFrameStream { .. }
        | Request::UploadFrame { .. }
        | Request::EndFrameStream { .. }
        | Request::BeginShmFrameStream { .. }
        | Request::EndShmFrameStream { .. }
        | Request::ApplyScene { .. }
        | Request::StartTransition(_)
        | Request::GetTransition { .. }
        | Request::AbortTransition { .. }
        | Request::RenewTransition { .. } => Some(Operation::Control),
        Request::SaveCurrent { .. } => Some(Operation::HardwareAdministration),
        Request::CreateCollection { .. } => Some(Operation::CreateCollection),
        Request::DestroyCollection { .. } => Some(Operation::DestroyCollection),
        Request::AddCollectionMember { .. } | Request::RemoveCollectionMember { .. } => {
            Some(Operation::ModifyCollection)
        }
        Request::CreateScene { .. } | Request::CaptureScene { .. } => Some(Operation::CreateScene),
        Request::ReplaceScene { .. } | Request::RecaptureScene { .. } => {
            Some(Operation::ModifyScene)
        }
        Request::DeleteScene { .. } => Some(Operation::DestroyScene),
    }
}

/// An authorization outcome. Policies allow or deny the entire request; they
/// do not rewrite it, drop targets, or return partial success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The request may proceed.
    Allow,

    /// The request is refused. `reason` is a safe diagnostic; it must not
    /// leak secrets, directory details, or hidden device identities.
    #[allow(
        dead_code,
        reason = "No shipped policy denies yet; SocketAccessPolicy is the only implementation \
                  and always allows. This will change in the future."
    )]
    Deny {
        /// Optional safe diagnostic surfaced to the client.
        reason: Option<String>,
    },
}

/// Makes the allow/deny decision for a request from a connected principal.
pub trait AuthorizationPolicy: Send + Sync {
    /// Returns the decision for `principal` performing `operation` against
    /// `resources`. An empty `resources` slice means the request has not yet
    /// been resolved to concrete targets.
    ///
    /// Returns a boxed future rather than `async fn` so `dyn
    /// AuthorizationPolicy` stays object-safe (see the module doc); a
    /// synchronous policy returns an already-ready future.
    fn authorize<'a>(
        &'a self,
        principal: &'a Principal,
        operation: Operation,
        resources: &'a [Resource],
    ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>>;

    /// Returns the authenticated subject represented by this connection
    /// policy, when it has one.
    fn session_principal(&self) -> Option<SessionPrincipal> {
        None
    }

    /// Returns the non-secret credential ID for the authenticated subject,
    /// when one exists.
    fn session_credential_id(&self) -> Option<String> {
        None
    }

    /// Returns the immutable front-end actor ceiling for a delegated session.
    fn frontend_principal(&self) -> Option<SessionPrincipal> {
        None
    }

    /// Returns the non-secret credential ID for the immutable front-end actor.
    fn frontend_credential_id(&self) -> Option<String> {
        None
    }

    /// A short, stable name for startup logging and diagnostics.
    fn name(&self) -> &'static str;
}

/// Grants the intrinsic peer recovery identity the daemon-wide policy
/// ceiling. Protocol validation, resource capability checks, and admission
/// semaphores remain outside this policy and therefore still apply.
pub struct IntrinsicRecoveryPolicy {
    inner: Arc<dyn AuthorizationPolicy>,
}

/// Applies a caller-selected allow-only scope before the configured policy.
pub struct ScopedPolicy {
    inner: Arc<dyn AuthorizationPolicy>,
    scope: SessionScope,
}

/// Evaluates a non-peer authenticated subject against the daemon policy.
pub struct AuthenticatedSubjectPolicy {
    inner: Arc<dyn ManagedAccessPolicy>,
    subject: SessionPrincipal,
    frontend_actor: Option<SessionPrincipal>,
    credential_id: Option<String>,
    frontend_credential_id: Option<String>,
}

impl AuthenticatedSubjectPolicy {
    #[must_use]
    pub fn new(inner: Arc<dyn ManagedAccessPolicy>, subject: SessionPrincipal) -> Self {
        Self {
            inner,
            subject,
            frontend_actor: None,
            credential_id: None,
            frontend_credential_id: None,
        }
    }

    /// Adds the immutable authenticated front-end ceiling for a delegated
    /// session. Both identities must independently permit every operation.
    #[must_use]
    pub fn with_frontend_actor(
        mut self,
        frontend_actor: SessionPrincipal,
        credential_id: Option<String>,
    ) -> Self {
        self.frontend_actor = Some(frontend_actor);
        self.frontend_credential_id = credential_id;
        self
    }

    /// Associates the subject policy with its revocable credential.
    #[must_use]
    pub fn with_credential_id(mut self, credential_id: Option<String>) -> Self {
        self.credential_id = credential_id;
        self
    }
}

impl AuthorizationPolicy for AuthenticatedSubjectPolicy {
    fn authorize<'a>(
        &'a self,
        _principal: &'a Principal,
        operation: Operation,
        resources: &'a [Resource],
    ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        let request = SessionAuthorizationRequest {
            principal: self.subject.clone(),
            operation,
            resources: resources.to_vec(),
        };
        let frontend_request =
            self.frontend_actor
                .clone()
                .map(|principal| SessionAuthorizationRequest {
                    principal,
                    operation: request.operation,
                    resources: request.resources.clone(),
                });
        Box::pin(async move {
            let subject = self.inner.authorize(&request).await;
            let frontend = if let Some(request) = frontend_request {
                Some(self.inner.authorize(&request).await)
            } else {
                None
            };
            let subject = match subject {
                Ok(decision) if decision.is_allowed() => decision,
                Ok(decision) => {
                    return Decision::Deny {
                        reason: decision.reason,
                    };
                }
                Err(error) => {
                    return Decision::Deny {
                        reason: Some(format!("authorization policy unavailable: {error}")),
                    };
                }
            };
            let Some(frontend) = frontend else {
                return Decision::Allow;
            };
            match frontend {
                Ok(decision) if decision.is_allowed() => Decision::Allow,
                Ok(decision) => Decision::Deny {
                    reason: decision.reason.or(subject.reason),
                },
                Err(error) => Decision::Deny {
                    reason: Some(format!("authorization policy unavailable: {error}")),
                },
            }
        })
    }

    fn name(&self) -> &'static str {
        "authenticated-subject"
    }

    fn session_principal(&self) -> Option<SessionPrincipal> {
        Some(self.subject.clone())
    }

    fn session_credential_id(&self) -> Option<String> {
        self.credential_id.clone()
    }

    fn frontend_principal(&self) -> Option<SessionPrincipal> {
        self.frontend_actor.clone()
    }

    fn frontend_credential_id(&self) -> Option<String> {
        self.frontend_credential_id.clone()
    }
}

impl ScopedPolicy {
    /// Wraps the configured policy with one immutable connection scope.
    #[must_use]
    pub fn new(inner: Arc<dyn AuthorizationPolicy>, scope: SessionScope) -> Self {
        Self { inner, scope }
    }
}

impl AuthorizationPolicy for ScopedPolicy {
    fn authorize<'a>(
        &'a self,
        principal: &'a Principal,
        operation: Operation,
        resources: &'a [Resource],
    ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        if !self.scope.allows(operation, resources) {
            return Box::pin(async {
                Decision::Deny {
                    reason: Some("session scope denied access".to_owned()),
                }
            });
        }
        self.inner.authorize(principal, operation, resources)
    }

    fn name(&self) -> &'static str {
        "session-scope"
    }

    fn session_principal(&self) -> Option<SessionPrincipal> {
        self.inner.session_principal()
    }

    fn session_credential_id(&self) -> Option<String> {
        self.inner.session_credential_id()
    }

    fn frontend_principal(&self) -> Option<SessionPrincipal> {
        self.inner.frontend_principal()
    }

    fn frontend_credential_id(&self) -> Option<String> {
        self.inner.frontend_credential_id()
    }
}

impl IntrinsicRecoveryPolicy {
    /// Wraps a policy with the kernel-derived break-glass ceiling.
    #[must_use]
    pub fn new(inner: Arc<dyn AuthorizationPolicy>) -> Self {
        Self { inner }
    }
}

impl AuthorizationPolicy for IntrinsicRecoveryPolicy {
    fn authorize<'a>(
        &'a self,
        principal: &'a Principal,
        operation: Operation,
        resources: &'a [Resource],
    ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        if principal.is_intrinsic_recovery() {
            return Box::pin(async { Decision::Allow });
        }

        self.inner.authorize(principal, operation, resources)
    }

    fn name(&self) -> &'static str {
        "intrinsic-recovery"
    }
}

/// The built-in default policy: every request from a process the operating
/// system allowed to connect to the socket is authorized. This preserves
/// today's behaviour; it is not anonymous network access, since the
/// runtime-directory and socket permissions remain an outer access-control
/// boundary.
#[derive(Debug, Clone, Copy, Default)]
pub struct SocketAccessPolicy;

impl AuthorizationPolicy for SocketAccessPolicy {
    fn authorize<'a>(
        &'a self,
        _principal: &'a Principal,
        _operation: Operation,
        _resources: &'a [Resource],
    ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        Box::pin(async { Decision::Allow })
    }

    fn name(&self) -> &'static str {
        "socket-access"
    }
}

#[cfg(test)]
#[path = "authorization_tests.rs"]
mod tests;
