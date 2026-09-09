// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::{future, net::SocketAddr};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use axum::{Extension, Json};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use luminate::policy::PrincipalId;
use luminate::{Credential, TokenMetadata};
use luminate_protocol::{ResponseStatus, ServerInfo};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;

use super::*;

fn unavailable_state() -> AppState {
    AppState {
        request_id: Arc::new(AtomicU64::new(1)),
        socket_path: Some(PathBuf::from("/tmp/luminate-http-definitely-missing.sock")),
        transitions: rest::TransitionStore::default(),
        tickets: Arc::new(websocket::TicketStore::default()),
        authentication: Arc::new(AuthenticationBoundary {
            mode: AuthenticationMode::Direct,
            trusted_proxies: Vec::new(),
            proxy_credential: None,
        }),
        origins: Arc::new(OriginPolicy::default()),
        resources: Arc::new(test_resource_limits()),
        maximum_request_bytes: DEFAULT_MAXIMUM_REQUEST_BYTES,
    }
}

fn test_args() -> Args {
    Args::try_parse_from(["luminate-http"]).expect("default test arguments")
}

#[test]
fn exposure_validation_distinguishes_loopback_tls_and_insecure_modes() {
    let loopbacks = ["127.0.0.1:8080", "[::1]:8080", "[::ffff:127.0.0.1]:8080"];
    for address in loopbacks {
        assert!(
            validate_args(&test_args(), address.parse().expect("loopback address")).is_ok(),
            "rejected {address}"
        );
    }

    let exposed = [
        "0.0.0.0:8080",
        "192.0.2.1:8080",
        "[::]:8080",
        "[2001:db8::1]:8080",
    ];
    for address in exposed {
        let address = address.parse().expect("exposed address");
        assert!(validate_args(&test_args(), address).is_err());

        let mut tls = test_args();
        tls.tls_certificate = Some(PathBuf::from("certificate.pem"));
        tls.tls_private_key = Some(PathBuf::from("private-key.pem"));
        assert!(validate_args(&tls, address).is_ok());

        let mut insecure = test_args();
        insecure.authentication_mode = AuthenticationMode::InsecureDevelopment;
        assert!(validate_args(&insecure, address).is_ok());
    }
}

#[test]
fn trusted_proxy_mode_requires_its_complete_boundary() {
    let address = "127.0.0.1:8080".parse().expect("loopback address");
    let mut args = test_args();
    args.authentication_mode = AuthenticationMode::TrustedProxy;
    assert!(validate_args(&args, address).is_err());

    args.trusted_proxy
        .push("127.0.0.1".parse().expect("proxy address"));
    args.trusted_proxy_credential_file = Some(PathBuf::from("proxy.secret"));
    assert!(validate_args(&args, address).is_ok());
}

async fn raw_request(path: &str, headers: &[(&str, String)]) -> String {
    raw_request_with_state(unavailable_state(), path, headers).await
}

async fn raw_request_with_state(state: AppState, path: &str, headers: &[(&str, String)]) -> String {
    raw_request_with_method(state, "GET", path, headers).await
}

async fn raw_request_with_method(
    state: AppState,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
) -> String {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind HTTP test server");
    let address = listener.local_addr().expect("test server address");
    let http_limits = state.resources.http;
    let server = tokio::spawn(async move {
        server::serve(
            transport::BoundedListener::plaintext(listener, DEFAULT_MAXIMUM_CONNECTIONS),
            build_app(&state),
            http_limits,
            future::pending(),
        )
        .await
        .expect("serve HTTP test app");
    });
    let mut stream = TcpStream::connect(address)
        .await
        .expect("connect HTTP test server");
    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write HTTP request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .await
        .expect("read HTTP response");
    server.abort();
    response
}

async fn bounded_test_server(state: AppState) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind bounded HTTP test server");
    let address = listener.local_addr().expect("bounded test server address");
    let limits = state.resources.http;
    let task = tokio::spawn(async move {
        server::serve(
            transport::BoundedListener::plaintext(listener, DEFAULT_MAXIMUM_CONNECTIONS),
            build_app(&state),
            limits,
            future::pending(),
        )
        .await
        .expect("serve bounded HTTP test app");
    });
    (address, task)
}

#[tokio::test]
async fn browser_boundary_denies_origins_by_default_and_answers_allowed_preflights() {
    let response = raw_request(
        "/health/live",
        &[("Origin", "https://hostile.example".to_owned())],
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 403"));
    assert!(
        !response
            .to_ascii_lowercase()
            .contains("access-control-allow-origin")
    );

    let mut state = unavailable_state();
    state.origins = Arc::new(
        OriginPolicy::from_configured(
            &["https://app.example".to_owned()],
            AuthenticationMode::Direct,
        )
        .expect("origin policy"),
    );
    let response = raw_request_with_method(
        state,
        "OPTIONS",
        "/api/v0/me",
        &[
            ("Origin", "https://app.example".to_owned()),
            ("Access-Control-Request-Method", "GET".to_owned()),
            ("Access-Control-Request-Headers", "Authorization".to_owned()),
        ],
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 204"));
    assert!(response.contains("access-control-allow-origin: https://app.example"));
    assert!(!response.to_ascii_lowercase().contains("allow-credentials"));
}

#[tokio::test]
async fn browser_boundary_strips_forwarding_headers() {
    let response = raw_request(
        "/__test/forwarded",
        &[
            ("Forwarded", "for=192.0.2.1".to_owned()),
            ("X-Forwarded-For", "192.0.2.1".to_owned()),
            ("X-Forwarded-Surprise", "spoofed".to_owned()),
            ("X-Real-IP", "192.0.2.1".to_owned()),
        ],
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 204"));
}

#[tokio::test]
async fn rejected_websocket_origin_does_not_consume_ticket() {
    let fixture = test_support::scripted(vec![Vec::new()]);
    let ticket = fixture
        .state
        .tickets
        .issue_frame_test_ticket(&fixture.state, fixture.auth.clone())
        .await;
    let response = raw_request_with_state(
        fixture.state.clone(),
        &format!("/api/v0/ws/frames?ticket={ticket}"),
        &[("Origin", "https://hostile.example".to_owned())],
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 403"));
    assert!(fixture.state.tickets.contains_test_ticket(ticket).await);
    fixture.finish().await;
}

#[tokio::test]
async fn assembled_router_enforces_authentication_and_request_ids() {
    let response = raw_request("/health/live", &[]).await;
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.to_ascii_lowercase().contains("x-request-id: 1"));

    let response = raw_request("/api/v0/me", &[]).await;
    assert!(response.starts_with("HTTP/1.1 401"));
    assert!(
        response.contains("WWW-Authenticate: Bearer")
            || response.contains("www-authenticate: Bearer")
    );

    let response = raw_request("/api/v0/me", &[("Authorization", "Basic nope".to_owned())]).await;
    assert!(response.starts_with("HTTP/1.1 401"));

    let bearer = URL_SAFE_NO_PAD.encode("valid-token-shape");
    let response = raw_request(
        "/api/v0/me",
        &[("Authorization", format!("Bearer {bearer}"))],
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 503"));

    let response = raw_request(
        "/api/v0/me",
        &[
            ("Authorization", format!("Bearer {bearer}")),
            ("Luminate-Delegation", URL_SAFE_NO_PAD.encode("{}")),
        ],
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 400"));

    let response = raw_request("/api/v0/ws/events", &[]).await;
    assert!(response.starts_with("HTTP/1.1 401"));
    let response = raw_request("/api/v0/ws/events?ticket=not-a-ticket", &[]).await;
    assert!(response.starts_with("HTTP/1.1 400") || response.starts_with("HTTP/1.1 401"));
}

#[tokio::test]
async fn assembled_router_bounds_requests_per_peer() {
    let mut args = test_args();
    args.pre_auth_requests_per_minute = 1;
    args.pre_auth_request_burst = 0;
    let mut state = unavailable_state();
    state.resources = Arc::new(resource_limits(&args));
    let first = raw_request_with_state(state.clone(), "/health/live", &[]).await;
    assert!(first.starts_with("HTTP/1.1 200"));
    let response = raw_request_with_state(state, "/health/live", &[]).await;
    assert!(response.starts_with("HTTP/1.1 429"));
}

#[tokio::test]
async fn public_and_authentication_floods_use_separate_uniform_limits() {
    let mut args = test_args();
    args.public_requests_per_minute = 1;
    args.public_request_burst = 0;
    let mut public_state = unavailable_state();
    public_state.resources = Arc::new(resource_limits(&args));
    assert!(
        raw_request_with_state(public_state.clone(), "/health/live", &[])
            .await
            .starts_with("HTTP/1.1 200")
    );
    assert!(
        raw_request_with_state(public_state, "/health/live", &[])
            .await
            .starts_with("HTTP/1.1 429")
    );

    let mut args = test_args();
    args.failed_auth_per_minute = 1;
    args.failed_auth_burst = 0;
    let mut auth_state = unavailable_state();
    auth_state.resources = Arc::new(resource_limits(&args));
    for _ in 0..2 {
        let response = raw_request_with_state(auth_state.clone(), "/api/v0/me", &[]).await;
        assert!(response.starts_with("HTTP/1.1 401"));
        assert!(response.contains("authentication token missing or invalid"));
    }
}

#[tokio::test]
async fn websocket_upgrade_floods_are_bounded_before_ticket_lookup() {
    let mut args = test_args();
    args.websocket_upgrades_per_minute = 1;
    args.websocket_upgrade_burst = 0;
    let mut state = unavailable_state();
    state.resources = Arc::new(resource_limits(&args));
    let first =
        raw_request_with_state(state.clone(), "/api/v0/ws/events?ticket=not-a-ticket", &[]).await;
    assert!(first.starts_with("HTTP/1.1 400") || first.starts_with("HTTP/1.1 401"));
    let second = raw_request_with_state(state, "/api/v0/ws/events?ticket=not-a-ticket", &[]).await;
    assert!(second.starts_with("HTTP/1.1 429"));
}

#[tokio::test]
async fn bounded_server_rejects_slow_oversized_and_excessive_headers() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let mut args = test_args();
    args.request_header_timeout_seconds = 1;
    args.maximum_headers = 4;
    let mut state = unavailable_state();
    state.resources = Arc::new(resource_limits(&args));
    let (address, server) = bounded_test_server(state).await;

    let mut slow = TcpStream::connect(address)
        .await
        .expect("connect slow client");
    slow.write_all(b"GET /health/live HTTP/1.1\r\nHost:")
        .await
        .expect("write partial headers");
    let mut byte = [0_u8; 1];
    let result = time::timeout(Duration::from_secs(2), slow.read(&mut byte)).await;
    assert!(result.is_ok(), "slow header connection remained open");

    let mut excessive = TcpStream::connect(address)
        .await
        .expect("connect excessive-header client");
    excessive
        .write_all(b"GET /health/live HTTP/1.1\r\nHost: localhost\r\nX-A: 1\r\nX-B: 2\r\nX-C: 3\r\nX-D: 4\r\n\r\n")
        .await
        .expect("write excessive headers");
    let mut response = String::new();
    excessive
        .read_to_string(&mut response)
        .await
        .expect("read excessive-header response");
    assert!(response.starts_with("HTTP/1.1 431"));

    let mut oversized = TcpStream::connect(address)
        .await
        .expect("connect oversized-header client");
    let request = format!(
        "GET /health/live HTTP/1.1\r\nHost: localhost\r\nX-Oversized: {}\r\n\r\n",
        "x".repeat(resource::DEFAULT_MAXIMUM_HEADER_BYTES)
    );
    oversized
        .write_all(request.as_bytes())
        .await
        .expect("write oversized header");
    let mut response = String::new();
    let read_result = oversized.read_to_string(&mut response).await;
    assert!(
        read_result.is_err() || response.is_empty() || response.starts_with("HTTP/1.1 431"),
        "oversized header was accepted: {response}"
    );
    server.abort();
}

#[tokio::test]
async fn bounded_server_times_out_slow_bodies_and_rejects_oversized_bodies() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let mut args = test_args();
    args.request_timeout_seconds = 1;
    args.maximum_request_bytes = 4;
    let mut state = unavailable_state();
    state.maximum_request_bytes = args.maximum_request_bytes;
    state.resources = Arc::new(resource_limits(&args));
    let (address, server) = bounded_test_server(state).await;

    let mut slow = TcpStream::connect(address)
        .await
        .expect("connect slow body client");
    slow.write_all(b"POST /__test/body HTTP/1.1\r\nHost: localhost\r\nContent-Length: 4\r\nConnection: close\r\n\r\nx")
        .await
        .expect("write partial body");
    let mut response = String::new();
    time::timeout(Duration::from_secs(2), slow.read_to_string(&mut response))
        .await
        .expect("slow body request timed out at the transport")
        .expect("read slow-body response");
    assert!(response.starts_with("HTTP/1.1 408"));

    let mut oversized = TcpStream::connect(address)
        .await
        .expect("connect oversized body client");
    oversized.write_all(b"POST /__test/body HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\nConnection: close\r\n\r\n12345")
        .await
        .expect("write oversized body");
    response.clear();
    oversized
        .read_to_string(&mut response)
        .await
        .expect("read oversized-body response");
    assert!(response.starts_with("HTTP/1.1 413"));
    server.abort();
}

#[tokio::test]
async fn bounded_server_closes_idle_keep_alive_connections() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let mut args = test_args();
    args.keep_alive_idle_seconds = 1;
    let mut state = unavailable_state();
    state.resources = Arc::new(resource_limits(&args));
    let (address, server) = bounded_test_server(state).await;
    let mut stream = TcpStream::connect(address)
        .await
        .expect("connect idle client");
    stream
        .write_all(b"GET /health/live HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .expect("write keep-alive request");
    let mut response = vec![0_u8; 512];
    let count = stream
        .read(&mut response)
        .await
        .expect("read health response");
    assert!(String::from_utf8_lossy(&response[..count]).starts_with("HTTP/1.1 200"));
    let closed = time::timeout(Duration::from_secs(2), stream.read(&mut response)).await;
    assert!(closed.is_ok(), "idle keep-alive connection remained open");
    server.abort();
}

#[test]
fn resource_limit_configuration_rejects_zero_and_unsafe_header_bounds() {
    let address = "127.0.0.1:8080".parse().expect("loopback address");
    let mut args = test_args();
    args.maximum_header_bytes = 8191;
    assert!(validate_args(&args, address).is_err());
    args.maximum_header_bytes = resource::DEFAULT_MAXIMUM_HEADER_BYTES;
    args.maximum_websocket_sessions = 0;
    assert!(validate_args(&args, address).is_err());
}

async fn send_masked_text(stream: &mut TcpStream, text: &str) {
    use tokio::io::AsyncWriteExt as _;

    let payload = text.as_bytes();
    assert!(payload.len() < 126);
    let mask = [1_u8, 2, 3, 4];
    let mut frame = vec![
        0x81,
        0x80 | u8::try_from(payload.len()).expect("short payload"),
    ];
    frame.extend(mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    stream
        .write_all(&frame)
        .await
        .expect("write WebSocket frame");
}

#[tokio::test]
async fn frame_websocket_upgrades_starts_and_rejects_an_invalid_frame() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let fixture = test_support::scripted(vec![vec![
        ResponseStatus::FrameStreamStarted { generation: 7 },
        ResponseStatus::Ack,
    ]]);
    let ticket = fixture
        .state
        .tickets
        .issue_frame_test_ticket(&fixture.state, fixture.auth.clone())
        .await;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WebSocket test server");
    let address = listener.local_addr().expect("test server address");
    let state = fixture.state.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            build_app(&state).into_make_service_with_connect_info::<PeerAddress>(),
        )
        .await
        .expect("serve WebSocket test app");
    });
    let mut stream = TcpStream::connect(address)
        .await
        .expect("connect WebSocket");
    let request = format!(
        "GET /api/v0/ws/frames?ticket={ticket} HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write upgrade");
    let mut headers = Vec::new();
    loop {
        let mut byte = [0_u8; 1];
        stream
            .read_exact(&mut byte)
            .await
            .expect("read upgrade response");
        headers.push(byte[0]);
        if headers.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    assert!(String::from_utf8_lossy(&headers).starts_with("HTTP/1.1 101"));

    send_masked_text(
        &mut stream,
        r#"{"type":"start","protocol":0,"target":{"Device":"lamp"}}"#,
    )
    .await;
    let mut started = [0_u8; 128];
    let count = stream.read(&mut started).await.expect("read started frame");
    assert!(String::from_utf8_lossy(&started[..count]).contains("started"));
    send_masked_text(&mut stream, "not a frame").await;
    let count = stream.read(&mut started).await.expect("read error frame");
    assert!(String::from_utf8_lossy(&started[..count]).contains("invalid-frame"));
    drop(stream);
    server.abort();
    fixture.finish().await;
}

async fn websocket_first_reply(message: &str) -> String {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let responses = if message.contains(r#""protocol":0"#) {
        vec![ResponseStatus::Error(luminate_protocol::OperationError {
            code: luminate_protocol::ErrorCode::DaemonUnavailable,
            message: "test daemon unavailable".to_owned(),
            retry_after_ms: None,
            applied_targets: Vec::new(),
        })]
    } else {
        Vec::new()
    };
    let fixture = test_support::scripted(vec![responses]);
    let state = fixture.state.clone();
    let ticket = state
        .tickets
        .issue_frame_test_ticket(&state, test_support::auth())
        .await;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WebSocket test server");
    let address = listener.local_addr().expect("test server address");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            build_app(&state).into_make_service_with_connect_info::<PeerAddress>(),
        )
        .await
        .expect("serve WebSocket test app");
    });
    let mut stream = TcpStream::connect(address)
        .await
        .expect("connect WebSocket");
    let request = format!(
        "GET /api/v0/ws/frames?ticket={ticket} HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write upgrade");
    let mut headers = Vec::new();
    loop {
        let mut byte = [0_u8; 1];
        stream
            .read_exact(&mut byte)
            .await
            .expect("read upgrade response");
        headers.push(byte[0]);
        if headers.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let payload = message.as_bytes();
    let mask = [5_u8, 6, 7, 8];
    let mut frame = vec![
        0x81,
        0x80 | u8::try_from(payload.len()).expect("short payload"),
    ];
    frame.extend(mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    stream.write_all(&frame).await.expect("write first frame");
    let mut frame_header = [0_u8; 2];
    stream
        .read_exact(&mut frame_header)
        .await
        .expect("read reply header");
    let payload_len = usize::from(frame_header[1] & 0x7f);
    assert!(payload_len < 126);
    let mut response = vec![0_u8; payload_len];
    stream
        .read_exact(&mut response)
        .await
        .expect("read reply payload");
    server.abort();
    fixture.finish().await;
    String::from_utf8_lossy(&response).into_owned()
}

#[tokio::test]
async fn frame_websocket_rejects_bad_starts_before_daemon_access() {
    assert!(
        websocket_first_reply("not json")
            .await
            .contains("start-required")
    );
    assert!(
        websocket_first_reply(r#"{"type":"start","protocol":1,"target":{"Device":"lamp"}}"#)
            .await
            .contains("unsupported-protocol")
    );
    assert!(
        websocket_first_reply(r#"{"type":"start","protocol":0,"target":{"Device":"lamp"}}"#)
            .await
            .contains("stream-start-failed")
    );
}

#[tokio::test]
async fn frame_websocket_bounds_start_time_and_message_size() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    for oversized in [false, true] {
        let fixture = test_support::scripted(vec![Vec::new()]);
        let ticket = fixture
            .state
            .tickets
            .issue_frame_test_ticket(&fixture.state, fixture.auth.clone())
            .await;
        let mut args = test_args();
        args.websocket_start_timeout_seconds = 1;
        args.maximum_websocket_message_bytes = 64;
        let mut state = fixture.state.clone();
        state.resources = Arc::new(resource_limits(&args));
        let (address, server) = bounded_test_server(state).await;
        let mut stream = TcpStream::connect(address)
            .await
            .expect("connect bounded WebSocket client");
        let request = format!(
            "GET /api/v0/ws/frames?ticket={ticket} HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .await
            .expect("write bounded upgrade");
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            let mut byte = [0_u8; 1];
            stream
                .read_exact(&mut byte)
                .await
                .expect("read bounded upgrade response");
            headers.push(byte[0]);
        }
        assert!(String::from_utf8_lossy(&headers).starts_with("HTTP/1.1 101"));
        if oversized {
            send_masked_text(&mut stream, &"x".repeat(100)).await;
        }
        let mut response = [0_u8; 256];
        let read = time::timeout(Duration::from_secs(2), stream.read(&mut response))
            .await
            .expect("bounded WebSocket did not close or reply");
        assert!(read.is_ok());
        server.abort();
        fixture.finish().await;
    }
}

#[tokio::test]
async fn ticket_creation_authenticates_before_issuing_a_single_use_secret() {
    let fixture = test_support::scripted(vec![Vec::new()]);
    assert_eq!(
        websocket::create_ticket(
            State(fixture.state.clone()),
            Extension(fixture.auth.clone()),
            Json(websocket::event_ticket_request_for_test()),
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    fixture.finish().await;
}

#[tokio::test]
async fn health_openapi_and_app_construction_cover_public_boundaries() {
    assert_eq!(health_live().await.into_response().status(), StatusCode::OK);
    assert_eq!(
        health_ready(State(unavailable_state())).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let openapi = openapi_json().await;
    assert_eq!(openapi.status(), StatusCode::OK);
    assert_eq!(
        openapi.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/json"))
    );
    let app = build_app(&unavailable_state());
    drop(app);

    assert_eq!(
        add_delegation_openapi_header(serde_json::Value::Null),
        serde_json::Value::Null
    );
    let malformed = serde_json::json!({"paths": {"/api/v0/me": "not an object"}});
    assert_eq!(add_delegation_openapi_header(malformed.clone()), malformed);
    let malformed = serde_json::json!({"paths": {"/api/v0/me": {"get": "not an object"}}});
    assert_eq!(add_delegation_openapi_header(malformed.clone()), malformed);
    let malformed = serde_json::json!({"paths": {"/api/v0/me": {"get": {"parameters": {}}}}});
    assert_eq!(add_delegation_openapi_header(malformed.clone()), malformed);

    let mut document = serde_json::to_value(ApiDoc::openapi()).expect("serialize OpenAPI");
    document = add_websocket_openapi_messages(document);
    assert_eq!(
        document["paths"]["/api/v0/ws/events"]["get"]["x-luminate-websocket-messages"]["server"]["$ref"],
        "#/components/schemas/EventServerMessage"
    );
    assert_eq!(
        document["paths"]["/api/v0/ws/frames"]["get"]["x-luminate-websocket-messages"]["client"]["$ref"],
        "#/components/schemas/FrameClientMessage"
    );
}

#[tokio::test]
async fn health_and_session_metadata_succeed_against_a_daemon() {
    let fixture = test_support::one_response(ResponseStatus::ServerInfo(ServerInfo {
        daemon_name: "luminated".to_owned(),
        daemon_version: "1.2.3".to_owned(),
        protocol_abi_version: luminate_protocol::PROTOCOL_ABI_VERSION,
    }));
    assert_eq!(
        health_ready(State(fixture.state.clone())).await.status(),
        StatusCode::OK
    );
    fixture.finish().await;

    let fixture = test_support::one_response(ResponseStatus::Ack);
    assert_eq!(
        ping(
            State(fixture.state.clone()),
            Extension(fixture.auth.clone())
        )
        .await
        .status(),
        StatusCode::OK
    );
    fixture.finish().await;

    let fixture = test_support::one_response(ResponseStatus::ServerInfo(ServerInfo {
        daemon_name: "luminated".to_owned(),
        daemon_version: "1.2.3".to_owned(),
        protocol_abi_version: luminate_protocol::PROTOCOL_ABI_VERSION,
    }));
    assert_eq!(
        get_server(
            State(fixture.state.clone()),
            Extension(fixture.auth.clone())
        )
        .await
        .status(),
        StatusCode::OK
    );
    fixture.finish().await;

    let fixture = test_support::scripted(vec![Vec::new()]);
    assert_eq!(
        get_me(
            State(fixture.state.clone()),
            Extension(fixture.auth.clone())
        )
        .await
        .status(),
        StatusCode::OK
    );
    fixture.finish().await;
}

#[test]
fn server_projection_reports_each_component_version() {
    let response = ServerBrief {
        luminate_http_version: "1.2.3".to_owned(),
        libluminate_version: "2.3.4".to_owned(),
        daemon_name: "luminated".to_owned(),
        daemon_version: "3.4.5".to_owned(),
        protocol_abi_version: 6,
    };
    assert_eq!(
        serde_json::to_value(response).expect("serialize server response"),
        serde_json::json!({
            "luminate_http_version": "1.2.3",
            "libluminate_version": "2.3.4",
            "daemon_name": "luminated",
            "daemon_version": "3.4.5",
            "protocol_abi_version": 6,
        })
    );

    let document = serde_json::to_value(ApiDoc::openapi()).expect("serialize OpenAPI");
    assert!(document["paths"]["/api/v0/ping"]["get"].is_object());
}

#[tokio::test]
async fn token_handlers_cover_listing_creation_rotation_and_revocation() {
    let principal = PrincipalId::new("local", "alice").expect("principal");
    let metadata = || TokenMetadata {
        id: "desk".to_owned(),
        subject: principal.clone(),
        expires_at: None,
        revoked: false,
    };

    let fixture = test_support::one_response(ResponseStatus::Tokens(vec![metadata()]));
    assert_eq!(
        list_tokens(
            State(fixture.state.clone()),
            Extension(fixture.auth.clone()),
            Query(TokenListQuery {
                include_expired: None
            }),
        )
        .await
        .status(),
        StatusCode::OK
    );
    fixture.finish().await;

    let fixture = test_support::one_response(ResponseStatus::TokenCreated {
        metadata: metadata(),
        secret: Credential::new("created-secret").expect("credential"),
    });
    let response = create_token(
        State(fixture.state.clone()),
        Extension(fixture.auth.clone()),
        Json(DaemonTokenRequest {
            id: "desk".to_owned(),
            authority: "local".to_owned(),
            subject: "alice".to_owned(),
            expires_at: None,
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert!(response.headers().contains_key(header::LOCATION));
    fixture.finish().await;

    let fixture = test_support::one_response(ResponseStatus::TokenCreated {
        metadata: metadata(),
        secret: Credential::new("rotated-secret").expect("credential"),
    });
    assert_eq!(
        rotate_token(
            State(fixture.state.clone()),
            Extension(fixture.auth.clone()),
            Path("desk".to_owned()),
            Json(RotateDaemonTokenRequest { expires_at: None }),
        )
        .await
        .status(),
        StatusCode::OK
    );
    fixture.finish().await;

    let fixture = test_support::one_response(ResponseStatus::Ack);
    assert_eq!(
        revoke_token(
            State(fixture.state.clone()),
            Extension(fixture.auth.clone()),
            Path("desk".to_owned()),
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    fixture.finish().await;
}

#[test]
fn token_projection_and_problem_helpers_cover_expiry_edges() {
    let record = token_response(TokenMetadata {
        id: "old".to_owned(),
        subject: PrincipalId::new("local", "alice").expect("principal"),
        expires_at: Some(UNIX_EPOCH),
        revoked: true,
    });
    assert_eq!(record.expires_at, Some(0));
    assert!(record.revoked);
    assert_eq!(unix_expiry(None).expect("absent expiry"), None);
    assert_eq!(
        unix_expiry(Some(1)).expect("valid expiry"),
        Some(UNIX_EPOCH + Duration::from_secs(1))
    );
    assert_eq!(
        problem_response(
            StatusCode::BAD_REQUEST,
            "bad-request",
            "bad",
            Some("7".to_owned()),
            Some(5)
        )
        .status(),
        StatusCode::BAD_REQUEST
    );
}
