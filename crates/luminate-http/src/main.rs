// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Optional HTTP companion for Luminate.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read as _, Write as _};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use axum::body::Body;
#[cfg(test)]
use axum::body::Bytes;
use axum::extract::connect_info::Connected;
use axum::extract::{ConnectInfo, Path as ExtractPath, Query, State};
use axum::http::uri::Authority;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse as _, Response};
use axum::routing::{delete, get, post};
use axum::serve::IncomingStream;
use axum::{Extension, Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use clap::{Parser, ValueEnum};
use luminate::policy::PrincipalId;
use luminate::{Client, Credential};
use luminate_platform::secure_storage::open_private_file_for_read;
use luminate_platform::terminal::{TerminalSafeFields, escape};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::signal::ctrl_c;
use tokio::time::{self, MissedTickBehavior};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::sensitive_headers::SetSensitiveRequestHeadersLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use utoipa::{OpenApi, ToSchema};

use crate::auth::{AuthContext, auth_middleware};
use crate::websocket::{
    __path_create_ticket, __path_events_upgrade, __path_frames_upgrade, TicketRequest,
    TicketResponse,
};

mod auth;
mod resource;
mod rest;
mod server;
mod transport;
mod websocket;

const DEFAULT_MAXIMUM_CONNECTIONS: usize = 128;
const DEFAULT_MAXIMUM_REQUEST_BYTES: usize = 1024 * 1024;
const DEFAULT_REQUESTS_PER_MINUTE: u32 = 60;
const DEFAULT_REQUEST_BURST: u32 = 20;
use resource::{HttpLimits, Rate, ResourceLimits};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum AuthenticationMode {
    #[default]
    Direct,
    TrustedProxy,
    InsecureDevelopment,
}

#[derive(Clone, Copy, Debug)]
struct PeerAddress(SocketAddr);

impl Connected<IncomingStream<'_, transport::BoundedListener>> for PeerAddress {
    fn connect_info(stream: IncomingStream<'_, transport::BoundedListener>) -> Self {
        Self(*stream.remote_addr())
    }
}

impl Connected<IncomingStream<'_, TcpListener>> for PeerAddress {
    fn connect_info(stream: IncomingStream<'_, TcpListener>) -> Self {
        Self(*stream.remote_addr())
    }
}

#[cfg(test)]
mod test_support;

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

#[derive(Debug, Parser)]
#[command(about = "Optional HTTP companion for Luminate")]
struct Args {
    #[arg(long)]
    listen: Option<SocketAddr>,

    #[arg(long)]
    socket_path: Option<PathBuf>,

    #[arg(long, value_enum, default_value_t)]
    authentication_mode: AuthenticationMode,

    #[arg(long, requires = "tls_private_key")]
    tls_certificate: Option<PathBuf>,

    #[arg(long, requires = "tls_certificate")]
    tls_private_key: Option<PathBuf>,

    #[arg(
        long,
        value_delimiter = ',',
        requires = "trusted_proxy_credential_file"
    )]
    trusted_proxy: Vec<IpAddr>,

    #[arg(long, requires = "trusted_proxy")]
    trusted_proxy_credential_file: Option<PathBuf>,

    #[arg(long, default_value_t = DEFAULT_MAXIMUM_CONNECTIONS)]
    maximum_connections: usize,

    #[arg(long, default_value_t = DEFAULT_MAXIMUM_REQUEST_BYTES)]
    maximum_request_bytes: usize,

    #[arg(long, default_value_t = DEFAULT_REQUESTS_PER_MINUTE)]
    requests_per_minute: u32,

    #[arg(long, default_value_t = DEFAULT_REQUEST_BURST)]
    request_burst: u32,

    #[arg(long, default_value_t = resource::DEFAULT_MAXIMUM_HEADER_BYTES)]
    maximum_header_bytes: usize,

    #[arg(long, default_value_t = resource::DEFAULT_MAXIMUM_HEADERS)]
    maximum_headers: usize,

    #[arg(long, default_value_t = resource::DEFAULT_REQUEST_HEADER_TIMEOUT_SECONDS)]
    request_header_timeout_seconds: u64,

    #[arg(long, default_value_t = resource::DEFAULT_REQUEST_TIMEOUT_SECONDS)]
    request_timeout_seconds: u64,

    #[arg(long, default_value_t = resource::DEFAULT_KEEP_ALIVE_IDLE_SECONDS)]
    keep_alive_idle_seconds: u64,

    #[arg(long, default_value_t = resource::DEFAULT_TLS_HANDSHAKE_TIMEOUT_SECONDS)]
    tls_handshake_timeout_seconds: u64,

    #[arg(long, default_value_t = resource::DEFAULT_PUBLIC_REQUESTS_PER_MINUTE)]
    public_requests_per_minute: u32,

    #[arg(long, default_value_t = resource::DEFAULT_PUBLIC_REQUEST_BURST)]
    public_request_burst: u32,

    #[arg(long, default_value_t = resource::DEFAULT_PRE_AUTH_REQUESTS_PER_MINUTE)]
    pre_auth_requests_per_minute: u32,

    #[arg(long, default_value_t = resource::DEFAULT_PRE_AUTH_REQUEST_BURST)]
    pre_auth_request_burst: u32,

    #[arg(long, default_value_t = resource::DEFAULT_FAILED_AUTH_PER_MINUTE)]
    failed_auth_per_minute: u32,

    #[arg(long, default_value_t = resource::DEFAULT_FAILED_AUTH_BURST)]
    failed_auth_burst: u32,

    #[arg(long, default_value_t = resource::DEFAULT_FAILED_CREDENTIALS_PER_MINUTE)]
    failed_credentials_per_minute: u32,

    #[arg(long, default_value_t = resource::DEFAULT_FAILED_CREDENTIAL_BURST)]
    failed_credential_burst: u32,

    #[arg(long, default_value_t = resource::DEFAULT_WEBSOCKET_UPGRADES_PER_MINUTE)]
    websocket_upgrades_per_minute: u32,

    #[arg(long, default_value_t = resource::DEFAULT_WEBSOCKET_UPGRADE_BURST)]
    websocket_upgrade_burst: u32,

    #[arg(long, default_value_t = resource::DEFAULT_MAXIMUM_WEBSOCKET_SESSIONS)]
    maximum_websocket_sessions: usize,

    #[arg(long, default_value_t = resource::DEFAULT_MAXIMUM_WEBSOCKET_MESSAGE_BYTES)]
    maximum_websocket_message_bytes: usize,

    #[arg(long, default_value_t = resource::DEFAULT_MAXIMUM_WEBSOCKET_QUEUE)]
    maximum_websocket_queue: usize,

    #[arg(long, default_value_t = resource::DEFAULT_WEBSOCKET_START_TIMEOUT_SECONDS)]
    websocket_start_timeout_seconds: u64,

    #[arg(long, default_value_t = resource::DEFAULT_WEBSOCKET_IDLE_SECONDS)]
    websocket_idle_seconds: u64,

    #[arg(long, default_value_t = resource::DEFAULT_RATE_LIMIT_ENTRIES)]
    maximum_rate_limit_entries: usize,

    /// Browser origins permitted to use the API. May be repeated.
    #[arg(long = "allowed-origin")]
    allowed_origins: Vec<String>,
}

#[derive(Clone)]
struct AppState {
    request_id: Arc<AtomicU64>,
    socket_path: Option<PathBuf>,
    transitions: rest::TransitionStore,
    tickets: Arc<websocket::TicketStore>,
    authentication: Arc<AuthenticationBoundary>,
    origins: Arc<OriginPolicy>,
    resources: Arc<ResourceLimits>,
    maximum_request_bytes: usize,
}

#[derive(Debug, Default)]
struct OriginPolicy {
    allowed: HashSet<String>,
}

impl OriginPolicy {
    fn from_configured(origins: &[String], mode: AuthenticationMode) -> anyhow::Result<Self> {
        let allowed = origins
            .iter()
            .map(|origin| normalize_origin(origin, mode))
            .collect::<anyhow::Result<HashSet<_>>>()?;
        Ok(Self { allowed })
    }

    fn decision(&self, value: &HeaderValue) -> OriginDecision {
        let Ok(value) = value.to_str() else {
            return OriginDecision::Reject;
        };
        let Ok(origin) = normalize_origin(value, AuthenticationMode::InsecureDevelopment) else {
            return OriginDecision::Reject;
        };
        if self.allowed.contains(&origin) {
            OriginDecision::Allow(origin)
        } else {
            OriginDecision::Reject
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum OriginDecision {
    NoOrigin,
    Allow(String),
    Reject,
}

#[derive(Debug)]
struct AuthenticationBoundary {
    mode: AuthenticationMode,
    trusted_proxies: Vec<IpAddr>,
    proxy_credential: Option<Credential>,
}

#[derive(Clone, Serialize, ToSchema)]
struct ProblemResponse {
    #[schema(example = "https://luminate.example.com/problems/not-found")]
    r#type: String,
    title: String,
    status: u16,
    detail: String,
    #[schema(example = "req-6f2f")]
    request_id: Option<String>,
    retry_after_ms: Option<u64>,
    #[schema(example = "[]")]
    applied_targets: Vec<serde_json::Value>,
}

#[derive(Serialize, ToSchema)]
struct HealthResponse {
    status: String,
}

#[derive(Serialize, ToSchema)]
struct ServerBrief {
    luminate_http_version: String,
    libluminate_version: String,
    daemon_name: String,
    daemon_version: String,
    protocol_abi_version: u32,
}

#[derive(Serialize, ToSchema)]
struct MeResponse {
    token_id: String,
    authority: String,
    subject: String,
    groups: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frontend_actor: Option<FrontendActorResponse>,
}

#[derive(Serialize, ToSchema)]
struct FrontendActorResponse {
    token_id: String,
    authority: String,
    subject: String,
    groups: Vec<String>,
}

#[derive(Serialize, ToSchema)]
struct TokenRecordResponse {
    id: String,
    authority: String,
    subject: String,
    expires_at: Option<u64>,
    revoked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    secret: Option<String>,
}

#[derive(Deserialize, ToSchema)]
struct DaemonTokenRequest {
    id: String,
    authority: String,
    subject: String,
    expires_at: Option<u64>,
}

#[derive(Deserialize, ToSchema)]
struct RotateDaemonTokenRequest {
    expires_at: Option<u64>,
}

#[derive(ToSchema)]
#[allow(
    dead_code,
    reason = "this DTO exists solely for compile-time OpenAPI schema generation"
)]
struct OpenApiDocumentResponse {
    openapi: String,
    #[schema(value_type = Object)]
    info: serde_json::Value,
    #[schema(value_type = Object)]
    paths: serde_json::Value,
    #[schema(value_type = Object)]
    components: serde_json::Value,
}

#[derive(Deserialize, ToSchema)]
struct TokenListQuery {
    include_expired: Option<bool>,
}

#[utoipa::path(
    get,
    path = "/health/live",
    responses(
        (status = 200, description = "Companion service is running", body = HealthResponse)
    )
)]
async fn health_live() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "live".to_owned(),
    })
}

#[cfg(test)]
async fn forwarded_headers_visible(headers: HeaderMap) -> StatusCode {
    if headers.keys().any(|name| {
        name.as_str().starts_with("x-forwarded-") || FORWARDING_HEADERS.contains(&name.as_str())
    }) {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::NO_CONTENT
    }
}

#[cfg(test)]
async fn test_body(_body: Bytes) -> StatusCode {
    StatusCode::NO_CONTENT
}

#[utoipa::path(
    get,
    path = "/health/ready",
    responses(
        (status = 200, description = "Service is live and daemon is reachable", body = HealthResponse),
        (status = 503, description = "Daemon unavailable", body = ProblemResponse)
    )
)]
async fn health_ready(State(state): State<AppState>) -> Response {
    let daemon = connect_client(state.socket_path.clone()).await;
    match daemon {
        Ok(client) => match client.server_info().await {
            Ok(_) => (
                StatusCode::OK,
                Json(HealthResponse {
                    status: "ready".to_owned(),
                }),
            )
                .into_response(),
            Err(error) => problem_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "daemon-timeout",
                &error.to_string(),
                None,
                None,
            ),
        },
        Err(error) => problem_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "daemon-unavailable",
            &error.to_string(),
            None,
            Some(200),
        ),
    }
}

#[utoipa::path(
    get,
    path = "/api/v0/openapi.json",
    responses(
        (status = 200, description = "OpenAPI 3.1 document", body = OpenApiDocumentResponse, content_type = "application/json"),
        (status = 500, description = "Document generation failed", body = ProblemResponse)
    )
)]
async fn openapi_json() -> Response {
    let mut document = ApiDoc::openapi();
    document.merge(rest::RestApi::openapi());
    match serde_json::to_value(document)
        .map(add_delegation_openapi_header)
        .map(add_websocket_openapi_messages)
    {
        Ok(payload) => Json(payload).into_response(),
        Err(error) => problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "openapi-failed",
            &error.to_string(),
            None,
            None,
        ),
    }
}

fn add_websocket_openapi_messages(mut document: serde_json::Value) -> serde_json::Value {
    let Some(paths) = document
        .get_mut("paths")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return document;
    };
    let messages = [
        (
            "/api/v0/ws/events",
            serde_json::json!({
                "server": { "$ref": "#/components/schemas/EventServerMessage" }
            }),
        ),
        (
            "/api/v0/ws/frames",
            serde_json::json!({
                "client": { "$ref": "#/components/schemas/FrameClientMessage" },
                "server": { "$ref": "#/components/schemas/FrameServerMessage" },
                "binary": "LFRM version 0; see docs/development/http-api.md"
            }),
        ),
    ];
    for (path, description) in messages {
        if let Some(operation) = paths
            .get_mut(path)
            .and_then(|item| item.get_mut("get"))
            .and_then(serde_json::Value::as_object_mut)
        {
            operation.insert("x-luminate-websocket-messages".to_owned(), description);
        }
    }
    document
}

fn add_delegation_openapi_header(mut document: serde_json::Value) -> serde_json::Value {
    let Some(paths) = document
        .get_mut("paths")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return document;
    };
    for (path, item) in paths {
        if path.starts_with("/api/v0/ws/")
            || path.starts_with("/health/")
            || path == "/api/v0/openapi.json"
        {
            continue;
        }
        let Some(operations) = item.as_object_mut() else {
            continue;
        };
        for operation in operations.values_mut() {
            let Some(operation) = operation.as_object_mut() else {
                continue;
            };
            let parameters = operation
                .entry("parameters")
                .or_insert_with(|| serde_json::Value::Array(Vec::new()));
            let Some(parameters) = parameters.as_array_mut() else {
                continue;
            };
            parameters.push(serde_json::json!({
                "name": "Luminate-Delegation",
                "in": "header",
                "required": false,
                "description": "Unpadded base64url-encoded delegated principal claims",
                "schema": { "type": "string" }
            }));
        }
    }
    document
}

#[utoipa::path(
    get,
    path = "/api/v0/me",
    responses(
        (status = 200, description = "Session metadata", body = MeResponse),
        (status = 401, description = "Authentication failed", body = ProblemResponse)
    )
)]
async fn get_me(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Response {
    let session = match rest::session(&state, auth).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let metadata = session.session();
    let frontend_actor = session
        .frontend_session
        .as_ref()
        .map(|metadata| FrontendActorResponse {
            token_id: metadata.credential_id.clone().unwrap_or_default(),
            authority: metadata.subject.authority().to_owned(),
            subject: metadata.subject.subject().to_owned(),
            groups: metadata.verified_groups.clone(),
        });
    Json(MeResponse {
        token_id: metadata.credential_id.clone().unwrap_or_default(),
        authority: metadata.subject.authority().to_owned(),
        subject: metadata.subject.subject().to_owned(),
        groups: metadata.verified_groups.clone(),
        frontend_actor,
    })
    .into_response()
}

#[utoipa::path(
    get,
    path = "/api/v0/server",
    responses(
        (status = 200, description = "Daemon metadata", body = ServerBrief),
        (status = 503, description = "Daemon unavailable", body = ProblemResponse)
    )
)]
async fn get_server(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Response {
    let server_info = match rest::session(&state, auth).await {
        Ok(session) => session.server_info().await.map(|server_info| ServerBrief {
            luminate_http_version: env!("CARGO_PKG_VERSION").to_owned(),
            libluminate_version: luminate::version().to_owned(),
            daemon_name: server_info.daemon_name,
            daemon_version: server_info.daemon_version,
            protocol_abi_version: server_info.protocol_abi_version,
        }),
        Err(response) => return response,
    };
    match server_info {
        Ok(info) => (StatusCode::OK, Json(info)).into_response(),
        Err(error) => rest::error_response(&error),
    }
}

#[utoipa::path(
    get,
    path = "/api/v0/ping",
    responses(
        (status = 200, description = "The authenticated daemon session is responsive", body = HealthResponse),
        (status = 401, description = "Authentication failed", body = ProblemResponse),
        (status = 503, description = "Daemon unavailable", body = ProblemResponse)
    )
)]
async fn ping(State(state): State<AppState>, Extension(auth): Extension<AuthContext>) -> Response {
    let session = match rest::session(&state, auth).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    match session.ping().await {
        Ok(()) => Json(HealthResponse {
            status: "ok".to_owned(),
        })
        .into_response(),
        Err(error) => rest::error_response(&error),
    }
}

#[utoipa::path(
    get,
    path = "/api/v0/tokens",
    params(("include_expired" = Option<bool>, Query, description = "Include expired token records")),
    responses((status = 200, body = Vec<TokenRecordResponse>))
)]
async fn list_tokens(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(query): Query<TokenListQuery>,
) -> Response {
    let include_expired = query.include_expired.unwrap_or(false);
    let session = match rest::session(&state, auth).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    match session.authentication_administration().list_tokens().await {
        Ok(records) => {
            let now = SystemTime::now();
            let records = records
                .into_iter()
                .filter(|record| {
                    include_expired || record.expires_at.is_none_or(|expiry| expiry > now)
                })
                .map(token_response)
                .collect::<Vec<_>>();
            (StatusCode::OK, Json(records)).into_response()
        }
        Err(error) => rest::error_response(&error),
    }
}

#[utoipa::path(
    post,
    path = "/api/v0/tokens",
    request_body = DaemonTokenRequest,
    responses((status = 201, body = TokenRecordResponse, headers(("Location" = String, description = "Created token resource"))), (status = 409, body = ProblemResponse), (status = 422, body = ProblemResponse))
)]
async fn create_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(payload): Json<DaemonTokenRequest>,
) -> Response {
    let session = match rest::session(&state, auth).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let subject = match PrincipalId::new(payload.authority, payload.subject) {
        Ok(subject) => subject,
        Err(error) => {
            return problem_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid-token",
                &error.to_string(),
                None,
                None,
            );
        }
    };
    let expires_at = match unix_expiry(payload.expires_at) {
        Ok(expires_at) => expires_at,
        Err(response) => return response,
    };
    let created = match session
        .authentication_administration()
        .create_token(payload.id, subject, expires_at)
        .await
    {
        Ok(created) => created,
        Err(error) => return rest::error_response(&error),
    };
    let mut response = token_response(created.metadata);
    response.secret = Some(URL_SAFE_NO_PAD.encode(created.secret.expose()));
    let location = format!("/api/v0/tokens/{}", response.id);
    let mut result = (StatusCode::CREATED, Json(response)).into_response();
    if let Ok(value) = HeaderValue::from_str(&location) {
        result.headers_mut().insert(header::LOCATION, value);
    }
    result
}

#[utoipa::path(
    delete,
    path = "/api/v0/tokens/{id}",
    params(("id" = String, Path, description = "Token identifier")),
    responses((status = 204), (status = 404, body = ProblemResponse))
)]
async fn revoke_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ExtractPath(id): ExtractPath<String>,
) -> Response {
    let session = match rest::session(&state, auth).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    match session
        .authentication_administration()
        .revoke_token(id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => rest::error_response(&error),
    }
}

#[utoipa::path(
    put,
    path = "/api/v0/tokens/{id}",
    params(("id" = String, Path, description = "Token identifier")),
    request_body = RotateDaemonTokenRequest,
    responses((status = 200, body = TokenRecordResponse), (status = 404, body = ProblemResponse))
)]
async fn rotate_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ExtractPath(id): ExtractPath<String>,
    Json(payload): Json<RotateDaemonTokenRequest>,
) -> Response {
    let session = match rest::session(&state, auth).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let expires_at = match unix_expiry(payload.expires_at) {
        Ok(expires_at) => expires_at,
        Err(response) => return response,
    };
    let created = match session
        .authentication_administration()
        .rotate_token(id, expires_at)
        .await
    {
        Ok(created) => created,
        Err(error) => return rest::error_response(&error),
    };
    let mut response = token_response(created.metadata);
    response.secret = Some(URL_SAFE_NO_PAD.encode(created.secret.expose()));
    (StatusCode::OK, Json(response)).into_response()
}

fn token_response(record: luminate::TokenMetadata) -> TokenRecordResponse {
    TokenRecordResponse {
        id: record.id,
        authority: record.subject.authority().to_owned(),
        subject: record.subject.subject().to_owned(),
        expires_at: record.expires_at.and_then(|expiry| {
            expiry
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|value| value.as_secs())
        }),
        revoked: record.revoked,
        secret: None,
    }
}

#[allow(
    clippy::result_large_err,
    reason = "the terminal Axum response is returned directly by token handlers"
)]
fn unix_expiry(value: Option<u64>) -> Result<Option<SystemTime>, Response> {
    value
        .map(|seconds| {
            UNIX_EPOCH
                .checked_add(Duration::from_secs(seconds))
                .ok_or_else(|| {
                    problem_response(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "invalid-token",
                        "token expiry is out of range",
                        None,
                        None,
                    )
                })
        })
        .transpose()
}

fn problem_response(
    status: StatusCode,
    code: &str,
    detail: &str,
    request_id: Option<String>,
    retry_after_ms: Option<u64>,
) -> Response {
    let payload = ProblemResponse {
        r#type: format!("https://luminate.example.com/problems/{code}"),
        title: code.replace('-', " "),
        status: status.as_u16(),
        detail: detail.to_owned(),
        request_id,
        retry_after_ms,
        applied_targets: Vec::new(),
    };
    (status, Json(payload)).into_response()
}

async fn connect_client(socket_path: Option<PathBuf>) -> luminate::Result<Client> {
    match socket_path {
        Some(path) => Client::connect_path(path).await,
        None => Client::connect().await,
    }
}

async fn request_id_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let request_id = state.request_id.fetch_add(1, Ordering::Relaxed).to_string();
    let mut request = request;
    request.extensions_mut().insert(request_id.clone());

    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&request_id).unwrap_or_else(|_| HeaderValue::from_static("invalid")),
    );
    response
}

#[derive(OpenApi)]
#[openapi(
    paths(
        health_live,
        health_ready,
        openapi_json,
        get_me,
        get_server,
        ping,
        list_tokens,
        create_token,
        revoke_token,
        rotate_token,
        create_ticket,
        events_upgrade,
        frames_upgrade
    ),
    components(schemas(
        ProblemResponse,
        HealthResponse,
        ServerBrief,
        MeResponse,
        TokenRecordResponse,
        DaemonTokenRequest,
        RotateDaemonTokenRequest,
        OpenApiDocumentResponse,
        TicketRequest,
        TicketResponse,
        websocket::EventServerMessage,
        websocket::FrameClientMessage,
        websocket::FrameServerMessage
    ))
)]
struct ApiDoc;

#[tokio::main]
#[allow(
    clippy::too_many_lines,
    reason = "startup keeps the service's security-sensitive stores and middleware assembly visible in one place"
)]
async fn main() -> ExitCode {
    match run_main().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let exit_code = error
                .downcast_ref::<clap::Error>()
                .map_or(1, clap::Error::exit_code);
            let _ = writeln!(
                io::stderr().lock(),
                "{}",
                escape(&format!("error: {error:#}"))
            );
            u8::try_from(exit_code).map_or(ExitCode::FAILURE, ExitCode::from)
        }
    }
}

async fn run_main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .fmt_fields(TerminalSafeFields)
        .init();

    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error) if error.use_stderr() => return Err(error.into()),
        Err(error) => {
            error.print()?;
            return Ok(());
        }
    };

    let listen_address = args
        .listen
        .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 8080)));
    validate_args(&args, listen_address)?;
    let origins = OriginPolicy::from_configured(&args.allowed_origins, args.authentication_mode)?;
    let proxy_credential = args
        .trusted_proxy_credential_file
        .as_deref()
        .map(read_credential)
        .transpose()?;
    let resources = Arc::new(resource_limits(&args));
    let app_state = AppState {
        request_id: Arc::new(AtomicU64::new(1)),
        socket_path: args.socket_path,
        transitions: rest::TransitionStore::default(),
        tickets: Arc::new(websocket::TicketStore::default()),
        authentication: Arc::new(AuthenticationBoundary {
            mode: args.authentication_mode,
            trusted_proxies: args
                .trusted_proxy
                .iter()
                .copied()
                .map(canonical_ip)
                .collect(),
            proxy_credential,
        }),
        origins: Arc::new(origins),
        resources,
        maximum_request_bytes: args.maximum_request_bytes,
    };

    let app = build_app(&app_state);
    let ticket_maintenance = Arc::clone(&app_state.tickets);
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            ticket_maintenance.prune_expired().await;
        }
    });

    let listener = TcpListener::bind(listen_address)
        .await
        .with_context(|| format!("failed to bind requested address {listen_address}"))?;
    let listen_addr = listener
        .local_addr()
        .context("failed to read listen address")?;
    let (listener, tls_reloader) = match (args.tls_certificate, args.tls_private_key) {
        (Some(certificate), Some(private_key)) => {
            let (listener, reloader) = transport::BoundedListener::tls(
                listener,
                args.maximum_connections,
                Duration::from_secs(args.tls_handshake_timeout_seconds),
                transport::TlsFiles::new(certificate, private_key),
            )?;
            (listener, Some(reloader))
        }
        (None, None) => (
            transport::BoundedListener::plaintext(listener, args.maximum_connections),
            None,
        ),
        _ => anyhow::bail!("TLS certificate and private key must be configured together"),
    };
    info!(%listen_addr, tls = tls_reloader.is_some(), "luminate-http listening");

    if args.authentication_mode == AuthenticationMode::InsecureDevelopment
        && !is_loopback(listen_addr.ip())
    {
        tracing::warn!(%listen_addr, "insecure development mode exposes plaintext credentials");
    }
    install_tls_reload(tls_reloader);

    if let Err(error) = server::serve(
        listener,
        app,
        app_state.resources.http,
        shutdown_and_reload(app_state),
    )
    .await
    {
        error!(%error, "server exited with error");
    }

    Ok(())
}

fn build_app(app_state: &AppState) -> Router {
    let open_routes = Router::new()
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))
        .route("/api/v0/openapi.json", get(openapi_json));
    #[cfg(test)]
    let open_routes = open_routes
        .route("/__test/forwarded", get(forwarded_headers_visible))
        .route("/__test/body", post(test_body));

    let auth_routes = Router::new()
        .route("/api/v0/me", get(get_me))
        .route("/api/v0/server", get(get_server))
        .route("/api/v0/ping", get(ping))
        .route("/api/v0/tokens", get(list_tokens).post(create_token))
        .route(
            "/api/v0/tokens/{id}",
            delete(revoke_token).put(rotate_token),
        )
        .route("/api/v0/websocket-tickets", post(websocket::create_ticket));

    let websocket_routes = Router::new()
        .route("/api/v0/ws/events", get(websocket::events_upgrade))
        .route("/api/v0/ws/frames", get(websocket::frames_upgrade));

    let auth_routes = auth_routes.merge(rest::routes());

    Router::new()
        .merge(open_routes)
        .merge(auth_routes)
        .merge(websocket_routes)
        .with_state(app_state.clone())
        .layer(RequestBodyLimitLayer::new(app_state.maximum_request_bytes))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            app_state.resources.http.request_timeout,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(SetSensitiveRequestHeadersLayer::new([
            header::AUTHORIZATION,
            HeaderName::from_static("luminate-delegation"),
        ]))
        .layer(middleware::from_fn_with_state(
            app_state.clone(),
            auth_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            app_state.clone(),
            pre_auth_rate_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            app_state.clone(),
            browser_and_header_boundary,
        ))
        .layer(middleware::from_fn_with_state(
            app_state.clone(),
            request_id_middleware,
        ))
}

const FORWARDING_HEADERS: &[&str] = &[
    "forwarded",
    "x-real-ip",
    "client-ip",
    "x-client-ip",
    "x-cluster-client-ip",
    "true-client-ip",
    "cf-connecting-ip",
    "fastly-client-ip",
];

async fn browser_and_header_boundary(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let forwarding_headers = request
        .headers()
        .keys()
        .filter(|name| {
            name.as_str().starts_with("x-forwarded-") || FORWARDING_HEADERS.contains(&name.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    for name in forwarding_headers {
        request.headers_mut().remove(name);
    }

    let decision = match request
        .headers()
        .get_all(header::ORIGIN)
        .iter()
        .collect::<Vec<_>>()[..]
    {
        [] => OriginDecision::NoOrigin,
        [origin] => state.origins.decision(origin),
        _ => OriginDecision::Reject,
    };
    if decision == OriginDecision::Reject {
        return problem_response(
            StatusCode::FORBIDDEN,
            "origin-not-allowed",
            "the request origin is not allowed",
            None,
            None,
        );
    }

    if request.method() == Method::OPTIONS
        && request
            .headers()
            .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD)
    {
        let OriginDecision::Allow(origin) = decision else {
            return problem_response(
                StatusCode::FORBIDDEN,
                "origin-not-allowed",
                "CORS preflight requests require an allowed origin",
                None,
                None,
            );
        };
        if !valid_preflight(request.headers()) {
            return problem_response(
                StatusCode::FORBIDDEN,
                "cors-preflight-not-allowed",
                "the requested cross-origin method or headers are not allowed",
                None,
                None,
            );
        }
        let mut response = StatusCode::NO_CONTENT.into_response();
        add_cors_headers(response.headers_mut(), &origin, true);
        return response;
    }

    let mut response = next.run(request).await;
    if let OriginDecision::Allow(origin) = decision {
        add_cors_headers(response.headers_mut(), &origin, false);
    }
    response
}

fn valid_preflight(headers: &HeaderMap) -> bool {
    let method_allowed = headers
        .get(header::ACCESS_CONTROL_REQUEST_METHOD)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|method| matches!(method, "GET" | "POST" | "PUT" | "DELETE"));
    let headers_allowed = headers
        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        .is_none_or(|value| {
            value.to_str().is_ok_and(|value| {
                value.split(',').all(|name| {
                    matches!(
                        name.trim().to_ascii_lowercase().as_str(),
                        "authorization" | "content-type"
                    )
                })
            })
        });
    method_allowed && headers_allowed
}

fn add_cors_headers(headers: &mut HeaderMap, origin: &str, preflight: bool) {
    if let Ok(origin) = HeaderValue::from_str(origin) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    }
    headers.append(header::VARY, HeaderValue::from_static("Origin"));
    if preflight {
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, PUT, DELETE"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("Authorization, Content-Type"),
        );
        headers.append(
            header::VARY,
            HeaderValue::from_static("Access-Control-Request-Method"),
        );
        headers.append(
            header::VARY,
            HeaderValue::from_static("Access-Control-Request-Headers"),
        );
    }
}

fn normalize_origin(value: &str, mode: AuthenticationMode) -> anyhow::Result<String> {
    anyhow::ensure!(value != "null", "opaque origin `null` is not permitted");
    let (scheme, authority) = value
        .split_once("://")
        .context("origin must contain a scheme and authority")?;
    let scheme = scheme.to_ascii_lowercase();
    anyhow::ensure!(
        matches!(scheme.as_str(), "http" | "https"),
        "origin scheme must be http or https"
    );
    anyhow::ensure!(
        !authority.is_empty()
            && !authority.contains(['/', '?', '#', '@', '*'])
            && !authority.chars().any(char::is_whitespace),
        "origin must contain only a host and optional port"
    );
    let authority = authority
        .parse::<Authority>()
        .context("origin authority is invalid")?;
    let host = authority.host().to_ascii_lowercase();
    if scheme == "http" {
        let loopback = host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(is_loopback);
        anyhow::ensure!(
            mode == AuthenticationMode::InsecureDevelopment && loopback,
            "http origins require insecure-development mode and a loopback host"
        );
    }
    let port = authority
        .port_u16()
        .filter(|port| !((scheme == "https" && *port == 443) || (scheme == "http" && *port == 80)));
    Ok(port.map_or_else(
        || format!("{scheme}://{host}"),
        |port| format!("{scheme}://{host}:{port}"),
    ))
}

async fn pre_auth_rate_middleware(
    State(state): State<AppState>,
    ConnectInfo(PeerAddress(peer)): ConnectInfo<PeerAddress>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let header_bytes = request
        .headers()
        .iter()
        .fold(0_usize, |total, (name, value)| {
            total
                .saturating_add(name.as_str().len())
                .saturating_add(value.as_bytes().len())
        });
    if request.headers().len() > state.resources.http.maximum_headers
        || header_bytes > state.resources.http.maximum_header_bytes
    {
        return problem_response(
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            "request-headers-too-large",
            "request headers exceed the configured limit",
            None,
            None,
        );
    }
    let peer_ip = canonical_ip(peer.ip());
    if !state.resources.pre_auth.allow(peer_ip).await {
        return problem_response(
            StatusCode::TOO_MANY_REQUESTS,
            "network-rate-limited",
            "HTTP network request rate limit exceeded",
            None,
            Some(1_000),
        );
    }
    if auth::is_public_path(request.uri().path()) && !state.resources.public.allow(peer_ip).await {
        return problem_response(
            StatusCode::TOO_MANY_REQUESTS,
            "public-endpoint-rate-limited",
            "public endpoint request rate limit exceeded",
            None,
            Some(1_000),
        );
    }
    if auth::is_websocket_path(request.uri().path())
        && !state.resources.websocket_upgrade.allow(peer_ip).await
    {
        return problem_response(
            StatusCode::TOO_MANY_REQUESTS,
            "websocket-upgrade-rate-limited",
            "WebSocket upgrade rate limit exceeded",
            None,
            Some(1_000),
        );
    }
    next.run(request).await
}

fn resource_limits(args: &Args) -> ResourceLimits {
    ResourceLimits::new(
        HttpLimits {
            maximum_header_bytes: args.maximum_header_bytes,
            maximum_headers: args.maximum_headers,
            request_header_timeout: Duration::from_secs(args.request_header_timeout_seconds),
            request_timeout: Duration::from_secs(args.request_timeout_seconds),
            keep_alive_idle: Duration::from_secs(args.keep_alive_idle_seconds),
        },
        Rate {
            per_minute: args.pre_auth_requests_per_minute,
            burst: args.pre_auth_request_burst,
        },
        Rate {
            per_minute: args.public_requests_per_minute,
            burst: args.public_request_burst,
        },
        Rate {
            per_minute: args.failed_auth_per_minute,
            burst: args.failed_auth_burst,
        },
        Rate {
            per_minute: args.failed_credentials_per_minute,
            burst: args.failed_credential_burst,
        },
        Rate {
            per_minute: args.requests_per_minute,
            burst: args.request_burst,
        },
        Rate {
            per_minute: args.websocket_upgrades_per_minute,
            burst: args.websocket_upgrade_burst,
        },
        args.maximum_rate_limit_entries,
        args.maximum_websocket_sessions,
        args.maximum_websocket_message_bytes,
        args.maximum_websocket_queue,
        Duration::from_secs(args.websocket_start_timeout_seconds),
        Duration::from_secs(args.websocket_idle_seconds),
    )
}

#[cfg(test)]
fn test_resource_limits() -> ResourceLimits {
    resource_limits(&Args::try_parse_from(["luminate-http"]).expect("default test arguments"))
}

#[allow(
    clippy::too_many_lines,
    reason = "startup validation keeps every related resource bound in one auditable list"
)]
fn validate_args(args: &Args, listen_address: SocketAddr) -> anyhow::Result<()> {
    anyhow::ensure!(
        args.maximum_connections > 0,
        "maximum connections must be nonzero"
    );
    anyhow::ensure!(
        args.maximum_request_bytes > 0,
        "maximum request bytes must be nonzero"
    );
    anyhow::ensure!(
        args.requests_per_minute > 0,
        "requests per minute must be nonzero"
    );
    anyhow::ensure!(args.request_burst > 0, "request burst must be nonzero");
    anyhow::ensure!(
        args.maximum_header_bytes >= 8 * 1024,
        "maximum header bytes must be at least 8192"
    );
    for (name, value) in [
        (
            "maximum headers",
            u64::try_from(args.maximum_headers).unwrap_or(u64::MAX),
        ),
        (
            "request header timeout seconds",
            args.request_header_timeout_seconds,
        ),
        ("request timeout seconds", args.request_timeout_seconds),
        ("keep-alive idle seconds", args.keep_alive_idle_seconds),
        (
            "TLS handshake timeout seconds",
            args.tls_handshake_timeout_seconds,
        ),
        (
            "public requests per minute",
            u64::from(args.public_requests_per_minute),
        ),
        ("public request burst", u64::from(args.public_request_burst)),
        (
            "pre-auth requests per minute",
            u64::from(args.pre_auth_requests_per_minute),
        ),
        (
            "pre-auth request burst",
            u64::from(args.pre_auth_request_burst),
        ),
        (
            "failed auth per minute",
            u64::from(args.failed_auth_per_minute),
        ),
        ("failed auth burst", u64::from(args.failed_auth_burst)),
        (
            "failed credentials per minute",
            u64::from(args.failed_credentials_per_minute),
        ),
        (
            "failed credential burst",
            u64::from(args.failed_credential_burst),
        ),
        (
            "WebSocket upgrades per minute",
            u64::from(args.websocket_upgrades_per_minute),
        ),
        (
            "WebSocket upgrade burst",
            u64::from(args.websocket_upgrade_burst),
        ),
        (
            "maximum WebSocket sessions",
            u64::try_from(args.maximum_websocket_sessions).unwrap_or(u64::MAX),
        ),
        (
            "maximum WebSocket message bytes",
            u64::try_from(args.maximum_websocket_message_bytes).unwrap_or(u64::MAX),
        ),
        (
            "maximum WebSocket queue",
            u64::try_from(args.maximum_websocket_queue).unwrap_or(u64::MAX),
        ),
        (
            "WebSocket start timeout seconds",
            args.websocket_start_timeout_seconds,
        ),
        ("WebSocket idle seconds", args.websocket_idle_seconds),
        (
            "maximum rate-limit entries",
            u64::try_from(args.maximum_rate_limit_entries).unwrap_or(u64::MAX),
        ),
    ] {
        anyhow::ensure!(value > 0, "{name} must be nonzero");
    }
    let tls = args.tls_certificate.is_some() && args.tls_private_key.is_some();
    if !is_loopback(listen_address.ip())
        && !tls
        && args.authentication_mode != AuthenticationMode::InsecureDevelopment
    {
        anyhow::bail!(
            "non-loopback listeners require TLS; use --authentication-mode insecure-development only for explicitly insecure development"
        );
    }
    if args.authentication_mode == AuthenticationMode::TrustedProxy {
        anyhow::ensure!(
            !args.trusted_proxy.is_empty() && args.trusted_proxy_credential_file.is_some(),
            "trusted-proxy mode requires --trusted-proxy and --trusted-proxy-credential-file"
        );
    } else {
        anyhow::ensure!(
            args.trusted_proxy.is_empty() && args.trusted_proxy_credential_file.is_none(),
            "trusted proxy options require --authentication-mode trusted-proxy"
        );
    }
    Ok(())
}

fn is_loopback(address: IpAddr) -> bool {
    canonical_ip(address).is_loopback()
}

fn canonical_ip(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V4(address) => IpAddr::V4(address),
        IpAddr::V6(address) => address
            .to_ipv4_mapped()
            .map_or(IpAddr::V6(address), IpAddr::V4),
    }
}

fn read_credential(path: &Path) -> anyhow::Result<Credential> {
    let metadata = fs::metadata(path).with_context(|| {
        format!(
            "failed to inspect trusted proxy credential {}",
            path.display()
        )
    })?;
    anyhow::ensure!(
        metadata.is_file(),
        "trusted proxy credential is not a regular file"
    );
    validate_private_credential_access(&metadata)?;
    let mut file = open_private_file_for_read(path)
        .with_context(|| format!("failed to open trusted proxy credential {}", path.display()))?;
    let mut bytes = Vec::new();
    #[allow(
        clippy::verbose_file_reads,
        reason = "read through the securely opened handle to avoid a path replacement race"
    )]
    file.read_to_end(&mut bytes)
        .with_context(|| format!("failed to read trusted proxy credential {}", path.display()))?;
    let encoded = str::from_utf8(&bytes)
        .context("trusted proxy credential file is not UTF-8")?
        .trim();
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .context("trusted proxy credential is not unpadded base64url")?;
    Credential::new(decoded).context("trusted proxy credential is invalid")
}

#[cfg(unix)]
fn validate_private_credential_access(metadata: &fs::Metadata) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt as _;

    anyhow::ensure!(
        metadata.mode().trailing_zeros() >= 6,
        "trusted proxy credential permissions must deny group and other access"
    );
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_credential_access(_metadata: &fs::Metadata) -> anyhow::Result<()> {
    Ok(())
}

fn install_tls_reload(reloader: Option<transport::TlsReloader>) {
    let Some(reloader) = reloader else {
        return;
    };
    let periodic = reloader.clone();
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(error) = periodic.reload() {
                error!(%error, "periodic TLS reload rejected; retaining current certificate");
            }
        }
    });
    install_signal_tls_reload(reloader);
}

#[cfg(unix)]
fn install_signal_tls_reload(reloader: transport::TlsReloader) {
    use tokio::signal::unix::{SignalKind, signal};

    tokio::spawn(async move {
        let mut signal = match signal(SignalKind::hangup()) {
            Ok(signal) => signal,
            Err(error) => {
                error!(%error, "failed to install TLS reload signal");
                return;
            }
        };
        while signal.recv().await.is_some() {
            match reloader.reload() {
                Ok(()) => info!("reloaded TLS certificate and private key"),
                Err(error) => error!(%error, "TLS reload rejected; retaining current certificate"),
            }
        }
    });
}

#[cfg(not(unix))]
fn install_signal_tls_reload(_reloader: transport::TlsReloader) {}

async fn shutdown_and_reload(_state: AppState) {
    if let Err(error) = ctrl_c().await {
        error!(%error, "failed to wait for shutdown signal");
    }
}
