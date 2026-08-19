// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Protocol request dispatch.
//!
//! [`dispatch_request`] is the entry point for authorization-bearing requests.
//! It authorizes, then hands the request to the module owning that category.
//! The connection loop handles the authorization-free, rate-limited
//! [`Request::Ping`] directly:
//!
//! - `inspection`: read-only queries about devices, state, and collections
//! - `admin`: rescans, plugin unload/reload, and withdrawn-device purges
//! - `management`: reading and patching managed configuration
//! - `appearance`: effect, brightness, colour, restore, clear, and save
//! - `collections`: creating, destroying, and re-shaping collections
//! - `streaming`: frame streams, including the shared-memory fast paths
//!
//! Every handler receives a [`DispatchContext`], which carries the
//! connection's fixed dependencies alongside the management configuration
//! read for this particular request.

mod access_administration;
mod admin;
mod appearance;
mod collections;
mod inspection;
mod management;
mod scenes;
mod setup;
mod streaming;
mod transitions;

use super::AuthenticationService;
use super::authz::RequestAuthorizer;
use super::executor::MutationExecutor;
use super::{
    Arc, AuthorizationPolicy, DaemonError, DaemonState, ManagementReadState, Mutex, Principal,
    Request, RescanRequester, Response, ResponseStatus, TargetId, UnsupportedPolicy, device,
    operation_for, sync,
};
use crate::audit::Sink;
use crate::device_config::DaemonConfig;
use crate::plugins::PluginManager;
use luminate_core::policy::ManagedAccessPolicy;
use luminate_core::transition::TransitionCancellation;
use luminate_protocol::{ErrorCode, OperationError};
use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::authorization::RateLimitKey;

pub(super) struct AccessAdministration {
    pub(super) policy: Arc<dyn ManagedAccessPolicy>,
    pub(super) authentication: AuthenticationService,
    pub(super) audit: Arc<dyn Sink>,
    pub(super) frontend_actors: HashSet<RateLimitKey>,
}

pub(super) const PING_BUCKET_CAPACITY: u32 = 4;
const PING_REFILL_INTERVAL: Duration = Duration::from_secs(1);

/// Small per-connection token bucket for the authorization-free liveness
/// request. Keeping it connection-local avoids global coordination on the
/// daemon's cheapest request path.
pub(super) struct PingLimiter {
    tokens: u32,
    last_refill: Instant,
}

impl PingLimiter {
    pub(super) fn new() -> Self {
        Self {
            tokens: PING_BUCKET_CAPACITY,
            last_refill: Instant::now(),
        }
    }

    pub(super) fn response(&mut self) -> Response {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_refill);
        let refill_intervals = elapsed.as_nanos() / PING_REFILL_INTERVAL.as_nanos();
        if refill_intervals >= u128::from(PING_BUCKET_CAPACITY) {
            self.tokens = PING_BUCKET_CAPACITY;
            self.last_refill = now;
        } else if refill_intervals != 0 {
            let refill = u32::try_from(refill_intervals).unwrap_or(PING_BUCKET_CAPACITY);
            self.tokens = self.tokens.saturating_add(refill).min(PING_BUCKET_CAPACITY);
            self.last_refill += PING_REFILL_INTERVAL.saturating_mul(refill);
        }

        if self.tokens == 0 {
            let retry_after = PING_REFILL_INTERVAL
                .saturating_sub(now.saturating_duration_since(self.last_refill));
            let retry_after_ms = u64::try_from(retry_after.as_millis())
                .unwrap_or(u64::MAX)
                .max(1);
            return Response {
                status: ResponseStatus::Error(OperationError {
                    code: ErrorCode::RateLimited,
                    message: "ping rate limit exceeded".to_owned(),
                    retry_after_ms: Some(retry_after_ms),
                    applied_targets: Vec::new(),
                }),
            };
        }

        self.tokens -= 1;
        Response {
            status: ResponseStatus::Ack,
        }
    }
}

/// Everything one accepted connection needs to serve requests, owned so it can
/// be moved into the connection's task.
///
/// The listener builds this once per accepted connection; without it the
/// dependencies would travel as a long positional parameter list through both
/// `handle_connection` and its request loop.
pub(super) struct ConnectionDependencies {
    pub(super) state: Arc<Mutex<DaemonState>>,
    pub(super) plugin_manager: Arc<PluginManager>,
    pub(super) management: Arc<ManagementReadState>,
    pub(super) mutations: MutationExecutor,
    pub(super) rescans: RescanRequester,
    pub(super) principal: Principal,
    pub(super) policy: Arc<dyn AuthorizationPolicy>,
    pub(super) authentication: AuthenticationService,
    pub(super) access_administration: Option<Arc<AccessAdministration>>,
}

/// One request's view of its connection.
///
/// The borrowed dependencies are fixed for the connection's lifetime, but the
/// three configuration values are re-read from [`ManagementReadState`] before
/// every request, so a committed management patch takes effect on the next
/// request rather than only on the next connection.
pub(super) struct DispatchContext<'a> {
    pub(super) state: &'a Arc<Mutex<DaemonState>>,
    pub(super) plugin_manager: &'a Arc<PluginManager>,
    pub(super) management: &'a Arc<ManagementReadState>,
    pub(super) mutations: &'a MutationExecutor,
    pub(super) owned_streams: &'a sync::Mutex<Vec<TargetId>>,
    pub(super) rescans: &'a RescanRequester,
    pub(super) principal: &'a Principal,
    pub(super) policy: &'a dyn AuthorizationPolicy,
    pub(super) access_administration: Option<&'a AccessAdministration>,

    pub(super) prefer_shm: bool,
    pub(super) prefer_client_shm: bool,
    pub(super) default_unsupported_policy: UnsupportedPolicy,
}

impl<'a> DispatchContext<'a> {
    /// Borrows `dependencies` for one request, pinning the management
    /// configuration values that request will see.
    pub(super) fn new(
        dependencies: &'a ConnectionDependencies,
        owned_streams: &'a sync::Mutex<Vec<TargetId>>,
        config: &DaemonConfig,
    ) -> Self {
        Self {
            state: &dependencies.state,
            plugin_manager: &dependencies.plugin_manager,
            management: &dependencies.management,
            mutations: &dependencies.mutations,
            owned_streams,
            rescans: &dependencies.rescans,
            principal: &dependencies.principal,
            policy: dependencies.policy.as_ref(),
            access_administration: dependencies.access_administration.as_deref(),
            prefer_shm: config.prefer_shm,
            prefer_client_shm: config.prefer_client_shm,
            default_unsupported_policy: config.default_unsupported_policy,
        }
    }

    pub(super) async fn cancel_transition_targets(&self, targets: &[TargetId]) {
        self.cancel_transition_targets_with_reason(
            targets,
            TransitionCancellation::ConflictingMutation,
        )
        .await;
    }

    pub(super) async fn cancel_transition_targets_with_reason(
        &self,
        targets: &[TargetId],
        reason: TransitionCancellation,
    ) {
        let state = self.state.lock().await;
        let overlapping = self
            .mutations
            .transitions
            .active()
            .into_iter()
            .filter(|entry| {
                entry.snapshot().targets.iter().any(|controlled| {
                    targets.iter().any(|target| {
                        state.target_covers(controlled, target)
                            || state.target_covers(target, controlled)
                    })
                })
            })
            .collect::<Vec<_>>();
        drop(state);
        for entry in &overlapping {
            entry.request_abort(reason);
        }
        for entry in overlapping {
            entry.wait_finished().await;
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the exhaustive wire request router keeps every operation classification visible"
)]
pub(super) async fn dispatch_request(request: Request, ctx: &DispatchContext<'_>) -> Response {
    let Some(operation) = operation_for(&request) else {
        return Response {
            status: ResponseStatus::Ack,
        };
    };
    let auth = RequestAuthorizer::new(
        ctx.state,
        ctx.plugin_manager.as_ref(),
        ctx.policy,
        ctx.principal,
        operation,
    );

    match request {
        // Handled above before an authorizer is constructed.
        Request::Ping | Request::IssueEventTicket => Response {
            status: ResponseStatus::Ack,
        },
        Request::ServerInfo => inspection::server_info(&auth).await,
        Request::ListDevices => inspection::list_devices(ctx, &auth).await,
        Request::ListWithdrawnDevices => inspection::list_withdrawn_devices(ctx, &auth).await,
        Request::GetDevice { id } => inspection::get_device(ctx, &auth, id).await,
        Request::GetState { device } => inspection::get_state(ctx, &auth, device).await,
        Request::GetCollectionState { collection } => {
            inspection::get_collection_state(ctx, &auth, collection).await
        }
        Request::ListCollections => inspection::list_collections(ctx, &auth).await,
        Request::GetCollection { id } => inspection::get_collection(ctx, &auth, id).await,
        Request::ListScenes => scenes::list_scenes(ctx, &auth).await,
        Request::GetScene { id } => scenes::get_scene(ctx, &auth, id).await,

        Request::RefreshState { device } => admin::refresh_state(ctx, &auth, device).await,
        Request::PurgeWithdrawnDevice { device } => {
            admin::purge_withdrawn_device(ctx, &auth, device).await
        }
        Request::Rescan => admin::rescan(ctx, &auth).await,
        Request::UnloadPlugin { name } => admin::unload_plugin(ctx, &auth, name).await,
        Request::ReloadPlugin { name } => admin::reload_plugin(ctx, &auth, name).await,

        Request::GetManagement => management::get_management(ctx, &auth).await,
        Request::PatchManagement { patch } => management::patch_management(ctx, &auth, patch).await,
        Request::ListPluginSetupWorkflows { plugin } => {
            setup::list_workflows(ctx, &auth, &plugin).await
        }
        Request::StartPluginSetup { plugin, workflow } => {
            setup::start(ctx, &auth, plugin, workflow).await
        }
        Request::RespondPluginSetup {
            session,
            generation,
            response,
        } => setup::respond(ctx, &auth, session, generation, response).await,
        Request::GetPluginSetup { session } => setup::get(ctx, &auth, &session).await,
        Request::CancelPluginSetup { session } => setup::cancel(ctx, &auth, &session).await,
        Request::GetAccessPolicy => access_administration::get_policy(ctx, &auth).await,
        Request::ReplaceAccessPolicy {
            expected,
            replacement,
        } => access_administration::replace_policy(ctx, &auth, expected, replacement).await,
        Request::CreateToken {
            id,
            subject,
            expires_at,
        } => access_administration::create_token(ctx, &auth, id, subject, expires_at).await,
        Request::ListTokens => access_administration::list_tokens(ctx, &auth).await,
        Request::RotateToken { id, expires_at } => {
            access_administration::rotate_token(ctx, &auth, id, expires_at).await
        }
        Request::RevokeToken { id } => access_administration::revoke_token(ctx, &auth, id).await,
        Request::CreateAttestation {
            name,
            subject,
            verified_groups,
            expires_at,
        } => {
            access_administration::create_attestation(
                ctx,
                &auth,
                name,
                subject,
                verified_groups,
                expires_at,
            )
            .await
        }
        Request::ListAttestations => access_administration::list_attestations(ctx, &auth).await,
        Request::RevokeAttestation { name } => {
            access_administration::revoke_attestation(ctx, &auth, name).await
        }

        Request::SetEffect(request) => appearance::set_effect(ctx, &auth, request).await,
        Request::SetAppearanceSlots(request) => {
            appearance::set_appearance_slots(ctx, &auth, request).await
        }
        Request::SetBrightness(request) => appearance::set_brightness(ctx, &auth, request).await,
        Request::RestoreAppearance { target } => {
            appearance::restore_appearance_request(ctx, &auth, target).await
        }
        Request::ClearTarget { target } => appearance::clear_target(ctx, &auth, target).await,
        Request::SaveCurrent { target } => appearance::save_current(ctx, &auth, target).await,

        Request::CreateCollection {
            name,
            description,
            kind,
            members,
        } => collections::create_collection(ctx, &auth, name, description, kind, members).await,
        Request::DestroyCollection { id } => collections::destroy_collection(ctx, &auth, id).await,
        Request::AddCollectionMember { id, member } => {
            collections::add_collection_member(ctx, &auth, id, member).await
        }
        Request::RemoveCollectionMember { id, member } => {
            collections::remove_collection_member(ctx, &auth, id, member).await
        }
        Request::CreateScene {
            name,
            description,
            bindings,
        } => scenes::create_scene(ctx, &auth, name, description, bindings).await,
        Request::CaptureScene {
            name,
            description,
            mode,
            targets,
        } => scenes::capture_scene(ctx, &auth, name, description, mode, targets).await,
        Request::ReplaceScene {
            id,
            expected_revision,
            name,
            description,
            bindings,
        } => {
            scenes::replace_scene(
                ctx,
                &auth,
                id,
                expected_revision,
                name,
                description,
                bindings,
            )
            .await
        }
        Request::RecaptureScene {
            id,
            expected_revision,
            mode,
            targets,
        } => scenes::recapture_scene(ctx, &auth, id, expected_revision, mode, targets).await,
        Request::DeleteScene {
            id,
            expected_revision,
        } => scenes::delete_scene(ctx, &auth, id, expected_revision).await,
        Request::ApplyScene {
            id,
            authorized_targets,
        } => scenes::apply_scene(ctx, &auth, id, authorized_targets).await,
        Request::StartTransition(request) => transitions::start(ctx, &auth, request).await,
        Request::GetTransition { id } => transitions::get(ctx, &auth, id).await,
        Request::AbortTransition { id } => transitions::abort(ctx, &auth, id).await,
        Request::RenewTransition { id, lease_ms } => {
            transitions::renew(ctx, &auth, id, lease_ms).await
        }

        Request::BeginFrameStream { target } => {
            streaming::begin_frame_stream(ctx, &auth, target).await
        }
        Request::UploadFrame { target, envelope } => {
            streaming::upload_frame(ctx, &auth, target, envelope).await
        }
        Request::EndFrameStream { target, generation } => {
            streaming::end_frame_stream(ctx, target, generation).await
        }
        Request::BeginShmFrameStream { target } => {
            streaming::begin_shm_frame_stream(ctx, &auth, target).await
        }
        Request::EndShmFrameStream { target, generation } => {
            streaming::end_shm_frame_stream(ctx, target, generation).await
        }
    }
}

/// Executes an authorized mutation outside the async reactor and maps its
/// typed result to the public protocol response.
///
/// Connection limits bound overall concurrency, while per-plugin host locks
/// serialize each backend independently so unrelated plugins can proceed in
/// parallel. Daemon state is never held across plugin-host IPC.
pub(super) async fn mutate<F>(
    mutations: &MutationExecutor,
    devices: Vec<device::DeviceId>,
    topology_generation: u64,
    func: F,
) -> Response
where
    F: FnOnce(&Arc<Mutex<DaemonState>>) -> Result<(), DaemonError> + Send + 'static,
{
    match mutations
        .execute_authorized(devices, topology_generation, Box::new(func))
        .await
    {
        Ok(()) => Response {
            status: ResponseStatus::Ack,
        },
        Err(error) => error.into(),
    }
}

/// As [`mutate`], but for a collection mutation whose leaf targets have
/// already been partitioned by `daemon::authz::authorize_partial` into an
/// authorized `applied` set and a `denied` remainder.
///
/// Success always returns [`ResponseStatus::CollectionApplied`], even when
/// `denied` is empty. Collection operations therefore have one stable
/// response shape, and partial authorization is treated as normal
/// best-effort application rather than an error.
///
/// A hardware failure partway through `applied` is different: `func` reports
/// it as [`DaemonError::PartialMutation`], which is handled exactly as it is
/// by [`mutate`]. That represents an erroneous partial execution, not this
/// function's successful partial-authorization path.
pub(super) async fn mutate_collection<F>(
    mutations: &MutationExecutor,
    devices: Vec<device::DeviceId>,
    topology_generation: u64,
    applied: Vec<TargetId>,
    denied: Vec<TargetId>,
    func: F,
) -> Response
where
    F: FnOnce(&Arc<Mutex<DaemonState>>) -> Result<(), DaemonError> + Send + 'static,
{
    match mutations
        .execute_authorized(devices, topology_generation, Box::new(func))
        .await
    {
        Ok(()) => Response {
            status: ResponseStatus::CollectionApplied { applied, denied },
        },
        Err(error) => error.into(),
    }
}

#[cfg(test)]
mod tests;
