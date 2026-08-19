// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Authenticated REST projection of the non-streaming multi-user client API.

use std::collections::BTreeMap;
use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{FromRequestParts, Path, State};
use axum::http::{HeaderValue, StatusCode, header, request::Parts};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use luminate::policy::{PolicyDocument, PolicyDocumentSource, PolicyRevision, PrincipalId};
use luminate::{
    AppearanceSlotValue, Authentication, Client, CollectionCategory, CollectionId,
    CollectionMember, CollectionOutcome, Colour, DeviceId, Effect, EmissionState, Error, ErrorKind,
    ManagementPatch, Rgb, SceneBinding, SceneCaptureMode, SceneId, Selector, TargetId,
    TransitionId, TransitionOptions, TransitionTargetState, UnsupportedPolicy,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use utoipa::{OpenApi, ToSchema};

use crate::auth::AuthContext;
use crate::resource::AuthenticatedRateKey;
use crate::{AppState, problem_response};

pub type TransitionStore = Arc<Mutex<BTreeMap<TransitionId, StoredTransition>>>;

#[derive(Clone)]
pub struct StoredTransition {
    principal: PrincipalId,
    id: TransitionId,
}

#[derive(Debug, Deserialize, ToSchema)]
struct ExpectedRevision {
    expected_revision: u64,
}

#[derive(Debug, Deserialize, ToSchema)]
struct CreateCollectionRequest {
    name: String,
    description: Option<String>,
    #[schema(value_type = Option<String>)]
    kind: Option<CollectionCategory>,
    #[schema(value_type = Vec<Object>)]
    members: Vec<CollectionMember>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct CollectionMemberRequest {
    #[schema(value_type = Object)]
    member: CollectionMember,
}

#[derive(Debug, Serialize)]
struct CollectionIdResponse {
    id: CollectionId,
}

#[derive(Debug, Deserialize, ToSchema)]
struct CreateSceneRequest {
    name: String,
    description: Option<String>,
    #[schema(value_type = Vec<Object>)]
    bindings: Vec<SceneBinding>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct CaptureSceneRequest {
    name: String,
    description: Option<String>,
    #[schema(value_type = Object)]
    mode: SceneCaptureMode,
    #[schema(value_type = Vec<Object>)]
    targets: Vec<TargetId>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct ReplaceSceneRequest {
    expected_revision: u64,
    name: String,
    description: Option<String>,
    #[schema(value_type = Vec<Object>)]
    bindings: Vec<SceneBinding>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct RecaptureSceneRequest {
    expected_revision: u64,
    #[schema(value_type = Object)]
    mode: SceneCaptureMode,
    #[schema(value_type = Vec<Object>)]
    targets: Vec<TargetId>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "operation", rename_all = "kebab-case")]
enum ControlRequest {
    AppearanceSlots {
        #[schema(value_type = Object)]
        target: TargetId,
        #[schema(value_type = Vec<Object>)]
        values: Vec<AppearanceSlotValue>,
    },
    Effect {
        #[schema(value_type = Object)]
        selector: Selector,
        #[schema(value_type = Object)]
        effect: Effect,
        #[schema(value_type = Option<String>)]
        on_unsupported: Option<UnsupportedPolicy>,
    },
    Colour {
        #[schema(value_type = Object)]
        selector: Selector,
        #[schema(value_type = Object)]
        colour: Colour,
        #[schema(value_type = Option<String>)]
        on_unsupported: Option<UnsupportedPolicy>,
    },
    Rgb {
        #[schema(value_type = Object)]
        selector: Selector,
        #[schema(value_type = Object)]
        rgb: Rgb,
        #[schema(value_type = Option<String>)]
        on_unsupported: Option<UnsupportedPolicy>,
    },
    Cct {
        #[schema(value_type = Object)]
        selector: Selector,
        kelvin: u32,
        #[schema(value_type = Option<String>)]
        on_unsupported: Option<UnsupportedPolicy>,
    },
    Brightness {
        #[schema(value_type = Object)]
        selector: Selector,
        value: u32,
        #[schema(value_type = Option<String>)]
        on_unsupported: Option<UnsupportedPolicy>,
    },
    Clear {
        #[schema(value_type = Object)]
        selector: Selector,
    },
    SaveCurrent {
        #[schema(value_type = Object)]
        selector: Selector,
    },
    Off {
        #[schema(value_type = Object)]
        target: TargetId,
    },
    RestoreAppearance {
        #[schema(value_type = Object)]
        selector: Selector,
    },
    Emission {
        #[schema(value_type = Object)]
        selector: Selector,
        #[schema(value_type = String)]
        state: EmissionState,
    },
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "source", rename_all = "kebab-case")]
enum StartTransitionRequest {
    SceneToScene {
        #[schema(value_type = String)]
        scene: SceneId,
        #[schema(value_type = String)]
        destination: SceneId,
        #[schema(value_type = Object)]
        options: TransitionOptions,
    },
    CurrentToScene {
        #[schema(value_type = String)]
        destination: SceneId,
        #[schema(value_type = Object)]
        options: TransitionOptions,
    },
    SceneToStates {
        #[schema(value_type = String)]
        scene: SceneId,
        #[schema(value_type = Vec<Object>)]
        states: Vec<TransitionTargetState>,
        #[schema(value_type = Object)]
        options: TransitionOptions,
    },
    CurrentToStates {
        #[schema(value_type = Vec<Object>)]
        states: Vec<TransitionTargetState>,
        #[schema(value_type = Object)]
        options: TransitionOptions,
    },
}

#[derive(Debug, Serialize)]
struct TransitionIdResponse {
    id: TransitionId,
}

#[derive(Debug, Serialize)]
struct OutcomeResponse {
    applied: Vec<TargetId>,
    denied: Vec<TargetId>,
}

fn outcome_response(result: luminate::Result<CollectionOutcome>) -> Response {
    match result {
        Ok(outcome) => Json(OutcomeResponse {
            applied: outcome.applied,
            denied: outcome.denied,
        })
        .into_response(),
        Err(error) => error_response(&error),
    }
}

#[derive(Debug, Deserialize)]
struct ReplacePolicyRequest {
    expected_revision: PolicyRevision,
    document: PolicyDocumentSource,
}

const ATTESTATION_LIFETIME: Duration = Duration::from_secs(30);

pub(crate) struct AuthenticatedClient {
    client: Client,
    pub(crate) frontend_session: Option<luminate::SessionMetadata>,
}

impl Deref for AuthenticatedClient {
    type Target = Client;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl FromRequestParts<AppState> for AuthenticatedClient {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Extension(auth) = Extension::<AuthContext>::from_request_parts(parts, state)
            .await
            .map_err(IntoResponse::into_response)?;
        session(state, auth).await
    }
}

pub(crate) async fn session(
    state: &AppState,
    auth: AuthContext,
) -> Result<AuthenticatedClient, Response> {
    let mut builder = Client::builder().authentication(Authentication::Bearer {
        credential: auth.credential,
    });
    if let Some(path) = &state.socket_path {
        builder = builder.path(path);
    }
    let bearer = match builder.connect().await {
        Ok(client) => client,
        Err(error) => {
            if error.kind() == ErrorKind::AuthenticationFailed {
                let _source_allowed = state.resources.failed_source.allow(auth.peer).await;
                let _credential_allowed = state
                    .resources
                    .failed_credential
                    .allow(auth.credential_digest)
                    .await;
            }
            return Err(error_response(&error));
        }
    };
    let credential_id =
        bearer.session().credential_id.clone().unwrap_or_else(|| {
            format!("digest:{}", URL_SAFE_NO_PAD.encode(auth.credential_digest))
        });
    let rate_key = match &auth.delegation {
        None => AuthenticatedRateKey::Direct {
            peer: auth.peer,
            credential_id,
        },
        Some(delegation) => AuthenticatedRateKey::Delegated {
            peer: auth.peer,
            frontend_credential_id: credential_id,
            authority: delegation.authority.clone(),
            subject: delegation.subject.clone(),
        },
    };
    if !state.resources.authenticated.allow(rate_key).await {
        return Err(problem_response(
            StatusCode::TOO_MANY_REQUESTS,
            "authenticated-rate-limited",
            "authenticated request rate limit exceeded",
            None,
            Some(1_000),
        ));
    }
    let Some(claims) = auth.delegation else {
        return Ok(AuthenticatedClient {
            client: bearer,
            frontend_session: None,
        });
    };

    let subject = PrincipalId::new(claims.authority, claims.subject)
        .map_err(|error| invalid_delegation_response(&error.to_string()))?;
    let now = SystemTime::now();
    let claim_expiry = UNIX_EPOCH
        .checked_add(Duration::from_secs(claims.expires_at))
        .ok_or_else(|| invalid_delegation_response("delegation expiry is out of range"))?;
    let hard_expiry = now.checked_add(ATTESTATION_LIFETIME).unwrap_or(now);
    let expiry = bearer
        .session()
        .expires_at
        .into_iter()
        .chain([claim_expiry, hard_expiry])
        .min();
    let name = format!("http-{}", state.request_id.fetch_add(1, Ordering::Relaxed));
    let created = bearer
        .authentication_administration()
        .create_principal_attestation(name.clone(), subject, claims.verified_groups, expiry)
        .await
        .map_err(|error| error_response(&error))?;
    let frontend_session = bearer.session().clone();
    let mut delegated_builder = Client::builder().authentication(Authentication::Attestation {
        name,
        credential: created.secret,
    });
    if let Some(path) = &state.socket_path {
        delegated_builder = delegated_builder.path(path);
    }
    let client = delegated_builder
        .connect()
        .await
        .map_err(|error| error_response(&error))?;
    Ok(AuthenticatedClient {
        client,
        frontend_session: Some(frontend_session),
    })
}

fn invalid_delegation_response(detail: &str) -> Response {
    problem_response(
        StatusCode::BAD_REQUEST,
        "invalid-delegation",
        detail,
        None,
        None,
    )
}

pub(crate) fn error_response(error: &Error) -> Response {
    let (status, code) = match error.kind() {
        ErrorKind::AuthenticationFailed => (StatusCode::UNAUTHORIZED, "authentication-failed"),
        ErrorKind::PermissionDenied => (StatusCode::FORBIDDEN, "permission-denied"),
        ErrorKind::NotFound => (StatusCode::NOT_FOUND, "not-found"),
        ErrorKind::InvalidArgument | ErrorKind::Unsupported | ErrorKind::UnknownState => {
            (StatusCode::UNPROCESSABLE_ENTITY, "invalid-operation")
        }
        ErrorKind::Conflict | ErrorKind::TransitionImpossible => (StatusCode::CONFLICT, "conflict"),
        ErrorKind::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate-limited"),
        ErrorKind::DaemonUnavailable | ErrorKind::Unavailable | ErrorKind::Timeout => {
            (StatusCode::SERVICE_UNAVAILABLE, "daemon-unavailable")
        }
        ErrorKind::IncompatibleDaemon
        | ErrorKind::IncompatibleEventSocket
        | ErrorKind::Internal
        | ErrorKind::Io
        | ErrorKind::Protocol
        | ErrorKind::ConnectionPoisoned
        | ErrorKind::PartialMutation => (StatusCode::INTERNAL_SERVER_ERROR, "operation-failed"),
    };
    let retry_after_ms = error.retry_after_ms();
    let applied_targets = error
        .applied_targets()
        .iter()
        .filter_map(|target| serde_json::to_value(target).ok())
        .collect();
    let mut response = (
        status,
        Json(crate::ProblemResponse {
            r#type: format!("https://luminate.example.com/problems/{code}"),
            title: code.replace('-', " "),
            status: status.as_u16(),
            detail: error.to_string(),
            request_id: None,
            retry_after_ms,
            applied_targets,
        }),
    )
        .into_response();
    if error.kind() == ErrorKind::AuthenticationFailed {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    }
    response
}

fn json_response<T: Serialize>(result: luminate::Result<T>) -> Response {
    match result {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => error_response(&error),
    }
}

fn empty_response(result: luminate::Result<()>) -> Response {
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => error_response(&error),
    }
}

mod administration;
mod collections;
mod control;
mod devices;
mod scenes;
mod schemas;
mod transitions;

use schemas::{
    CollectionSchema, CollectionStateSchema, DeviceSchema, DeviceStateSchema, IdSchema,
    ManagementChangeSetSchema, ManagementPatchSchema, ManagementSnapshotSchema, OutcomeSchema,
    PolicyDocumentSchema, ReplacePolicySchema, SceneSchema, TransitionStatusSchema,
};

use administration::{
    __path_management, __path_patch_management, __path_policy, __path_replace_policy, management,
    patch_management, policy, replace_policy,
};
use collections::{
    __path_add_member, __path_collection_state, __path_create_collection,
    __path_destroy_collection, __path_get_collection, __path_list_collections,
    __path_remove_member, add_member, collection_state, create_collection, destroy_collection,
    get_collection, list_collections, remove_member,
};
use control::{__path_control, control};
use devices::{
    __path_device_state, __path_get_device, __path_list_devices, __path_purge, __path_refresh,
    __path_rescan, __path_withdrawn, device_state, get_device, list_devices, purge, refresh,
    rescan, withdrawn,
};
use scenes::{
    __path_apply_scene, __path_capture_scene, __path_create_scene, __path_delete_scene,
    __path_get_scene, __path_list_scenes, __path_recapture_scene, __path_replace_scene,
    apply_scene, capture_scene, create_scene, delete_scene, get_scene, list_scenes,
    recapture_scene, replace_scene,
};
use transitions::{
    __path_abort_transition, __path_start_transition, __path_transition_status,
    __path_wait_transition, abort_transition, start_transition, transition_status, wait_transition,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v0/devices", get(list_devices))
        .route("/api/v0/devices/withdrawn", get(withdrawn))
        .route("/api/v0/devices/{id}", get(get_device))
        .route("/api/v0/devices/{id}/state", get(device_state))
        .route("/api/v0/devices/{id}/refresh", post(refresh))
        .route("/api/v0/devices/{id}/purge", delete(purge))
        .route("/api/v0/rescan", post(rescan))
        .route(
            "/api/v0/collections",
            get(list_collections).post(create_collection),
        )
        .route(
            "/api/v0/collections/{id}",
            get(get_collection).delete(destroy_collection),
        )
        .route("/api/v0/collections/{id}/state", get(collection_state))
        .route(
            "/api/v0/collections/{id}/members",
            post(add_member).delete(remove_member),
        )
        .route("/api/v0/scenes", get(list_scenes).post(create_scene))
        .route("/api/v0/scenes/capture", post(capture_scene))
        .route(
            "/api/v0/scenes/{id}",
            get(get_scene).put(replace_scene).delete(delete_scene),
        )
        .route("/api/v0/scenes/{id}/recapture", post(recapture_scene))
        .route("/api/v0/scenes/{id}/apply", post(apply_scene))
        .route(
            "/api/v0/management",
            get(management).patch(patch_management),
        )
        .route("/api/v0/policy", get(policy).put(replace_policy))
        .route("/api/v0/control", post(control))
        .route("/api/v0/transitions", post(start_transition))
        .route(
            "/api/v0/transitions/{id}",
            get(transition_status).delete(abort_transition),
        )
        .route("/api/v0/transitions/{id}/wait", post(wait_transition))
}

#[derive(OpenApi)]
#[openapi(
    paths(
        list_devices,
        get_device,
        withdrawn,
        device_state,
        refresh,
        purge,
        rescan,
        list_collections,
        create_collection,
        get_collection,
        destroy_collection,
        collection_state,
        add_member,
        remove_member,
        list_scenes,
        create_scene,
        capture_scene,
        get_scene,
        replace_scene,
        recapture_scene,
        delete_scene,
        apply_scene,
        management,
        patch_management,
        policy,
        replace_policy,
        control,
        start_transition,
        transition_status,
        abort_transition,
        wait_transition
    ),
    components(schemas(
        DeviceSchema,
        CollectionSchema,
        DeviceStateSchema,
        CollectionStateSchema,
        SceneSchema,
        ManagementSnapshotSchema,
        ManagementChangeSetSchema,
        PolicyDocumentSchema,
        TransitionStatusSchema,
        IdSchema,
        OutcomeSchema,
        ReplacePolicySchema,
        ManagementPatchSchema,
        ExpectedRevision,
        CreateCollectionRequest,
        CollectionMemberRequest,
        CreateSceneRequest,
        CaptureSceneRequest,
        ReplaceSceneRequest,
        RecaptureSceneRequest,
        ControlRequest,
        StartTransitionRequest
    ))
)]
pub struct RestApi;

#[cfg(test)]
mod tests;
