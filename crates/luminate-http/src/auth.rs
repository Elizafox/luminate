// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! HTTP bearer extraction for daemon-authenticated requests.

use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::Response;
use axum::{body::Body, http::Request};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use luminate::Credential;
use serde::Deserialize;
use std::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq as _;

use crate::resource::credential_digest;
use crate::websocket::TicketQuery;
use crate::{AppState, AuthenticationMode, PeerAddress, canonical_ip, problem_response};

#[derive(Clone)]
pub struct AuthContext {
    pub credential: Credential,
    pub delegation: Option<DelegationClaims>,
    pub(crate) peer: IpAddr,
    pub(crate) credential_digest: [u8; 32],
}

const MAX_DELEGATION_HEADER_BYTES: usize = 16 * 1024;
const MAX_DELEGATION_GROUPS: usize = 256;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationClaims {
    pub authority: String,
    pub subject: String,
    #[serde(default)]
    pub verified_groups: Vec<String>,
    pub expires_at: u64,
}

pub async fn auth_middleware(
    State(state): State<AppState>,
    ConnectInfo(PeerAddress(peer)): ConnectInfo<PeerAddress>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    if is_unprotected_path(request.uri().path()) {
        if is_websocket_path(request.uri().path()) {
            let Some(ticket) = request
                .uri()
                .query()
                .and_then(ticket_query_value)
                .map(str::to_owned)
            else {
                return unauthorized_response();
            };
            let redacted_uri = match request.uri().path() {
                "/api/v0/ws/events" => Uri::from_static("/api/v0/ws/events"),
                "/api/v0/ws/frames" => Uri::from_static("/api/v0/ws/frames"),
                _ => return unauthorized_response(),
            };
            *request.uri_mut() = redacted_uri;
            request.extensions_mut().insert(TicketQuery { ticket });
        }
        return next.run(request).await;
    }

    let peer = canonical_ip(peer.ip());
    let credential = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .and_then(|value| URL_SAFE_NO_PAD.decode(value).ok())
        .and_then(|value| Credential::new(value).ok());
    let Some(credential) = credential else {
        let _allowed = state.resources.failed_source.allow(peer).await;
        return unauthorized_response();
    };
    let credential_digest = credential_digest(credential.expose());
    if !state.resources.failed_source.available(peer).await
        || !state
            .resources
            .failed_credential
            .available(credential_digest)
            .await
    {
        return unauthorized_response();
    }

    if state.authentication.mode == AuthenticationMode::TrustedProxy {
        let trusted_address = state.authentication.trusted_proxies.contains(&peer);
        let trusted_credential =
            state
                .authentication
                .proxy_credential
                .as_ref()
                .is_some_and(|expected| {
                    expected.expose().len() == credential.expose().len()
                        && bool::from(expected.expose().ct_eq(credential.expose()))
                });
        if !trusted_address || !trusted_credential {
            let _source_allowed = state.resources.failed_source.allow(peer).await;
            let _credential_allowed = state
                .resources
                .failed_credential
                .allow(credential_digest)
                .await;
            return unauthorized_response();
        }
    }

    let delegation = match delegation_claims(request.headers()) {
        Ok(delegation) => delegation,
        Err(detail) => {
            return problem_response(
                StatusCode::BAD_REQUEST,
                "invalid-delegation",
                detail,
                None,
                None,
            );
        }
    };
    if delegation.is_some() && state.authentication.mode != AuthenticationMode::TrustedProxy {
        return problem_response(
            StatusCode::BAD_REQUEST,
            "delegation-not-permitted",
            "Luminate-Delegation is accepted only in trusted-proxy mode",
            None,
            None,
        );
    }

    request.extensions_mut().insert(AuthContext {
        credential,
        delegation,
        peer,
        credential_digest,
    });
    next.run(request).await
}

fn delegation_claims(headers: &HeaderMap) -> Result<Option<DelegationClaims>, &'static str> {
    let mut values = headers.get_all("luminate-delegation").iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err("exactly one Luminate-Delegation header is permitted");
    }
    let encoded = value
        .to_str()
        .map_err(|_| "the Luminate-Delegation header is not valid ASCII")?;
    if encoded.is_empty() || encoded.len() > MAX_DELEGATION_HEADER_BYTES || encoded.contains('=') {
        return Err("the Luminate-Delegation header is empty, oversized, or padded");
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| "the Luminate-Delegation header is not valid unpadded base64url")?;
    let claims = serde_json::from_slice::<DelegationClaims>(&decoded)
        .map_err(|_| "the Luminate-Delegation claims are malformed")?;
    if claims.authority.is_empty()
        || claims.subject.is_empty()
        || claims.verified_groups.len() > MAX_DELEGATION_GROUPS
        || claims.verified_groups.iter().any(String::is_empty)
    {
        return Err("the Luminate-Delegation identity or groups are invalid");
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(u64::MAX, |duration| duration.as_secs());
    if claims.expires_at <= now {
        return Err("the Luminate-Delegation claims have expired");
    }
    Ok(Some(claims))
}

fn ticket_query_value(query: &str) -> Option<&str> {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("ticket="))
        .filter(|ticket| !ticket.is_empty())
}

pub(crate) fn is_websocket_path(path: &str) -> bool {
    path == "/api/v0/ws/events" || path == "/api/v0/ws/frames"
}

fn unauthorized_response() -> Response {
    let mut response = problem_response(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "authentication token missing or invalid",
        None,
        None,
    );
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

fn is_unprotected_path(path: &str) -> bool {
    let public = is_public_path(path) || path == "/api/v0/ws/events" || path == "/api/v0/ws/frames";
    #[cfg(test)]
    let public = public || path == "/__test/forwarded" || path == "/__test/body";
    public
}

pub(crate) fn is_public_path(path: &str) -> bool {
    path == "/health/live" || path == "/health/ready" || path == "/api/v0/openapi.json"
}

#[cfg(test)]
mod tests {
    use super::{delegation_claims, is_unprotected_path, is_websocket_path, ticket_query_value};
    use axum::http::{HeaderMap, HeaderValue};
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn websocket_ticket_query_is_extracted_without_accepting_empty_values() {
        assert_eq!(ticket_query_value("ticket=abc&extra=value"), Some("abc"));
        assert_eq!(ticket_query_value("extra=value&ticket=abc"), Some("abc"));
        assert_eq!(ticket_query_value("ticket="), None);
        assert_eq!(ticket_query_value("not_ticket=abc"), None);
    }

    #[test]
    fn delegation_claims_are_strict_bounded_and_unpadded() {
        let future = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current Unix time")
            .as_secs()
            + 60;
        let payload = format!(
            r#"{{"authority":"proxy","subject":"alice","verified_groups":["operators"],"expires_at":{future}}}"#
        );
        let encoded = URL_SAFE_NO_PAD.encode(payload);
        let mut headers = HeaderMap::new();
        headers.insert(
            "luminate-delegation",
            HeaderValue::from_str(&encoded).expect("header value"),
        );
        let claims = delegation_claims(&headers)
            .expect("valid claims")
            .expect("present claims");
        assert_eq!(claims.subject, "alice");
        assert_eq!(claims.verified_groups, ["operators"]);

        headers.append("luminate-delegation", HeaderValue::from_static("duplicate"));
        assert!(delegation_claims(&headers).is_err());
        headers.clear();
        headers.insert("luminate-delegation", HeaderValue::from_static("e30="));
        assert!(delegation_claims(&headers).is_err());
    }

    #[test]
    fn delegation_claims_reject_malformed_expired_and_ambiguous_identities() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current Unix time")
            .as_secs();
        let cases = [
            String::new(),
            "not-base64url!".to_owned(),
            URL_SAFE_NO_PAD.encode("not JSON"),
            URL_SAFE_NO_PAD.encode(format!(
                r#"{{"authority":"proxy","subject":"alice","expires_at":{now},"extra":true}}"#
            )),
            URL_SAFE_NO_PAD.encode(format!(
                r#"{{"authority":"","subject":"alice","expires_at":{}}}"#,
                now + 60
            )),
            URL_SAFE_NO_PAD.encode(format!(
                r#"{{"authority":"proxy","subject":"","expires_at":{}}}"#,
                now + 60
            )),
            URL_SAFE_NO_PAD.encode(format!(
                r#"{{"authority":"proxy","subject":"alice","verified_groups":[""],"expires_at":{}}}"#,
                now + 60
            )),
            URL_SAFE_NO_PAD.encode(format!(
                r#"{{"authority":"proxy","subject":"alice","expires_at":{now}}}"#
            )),
            "a".repeat(super::MAX_DELEGATION_HEADER_BYTES + 1),
        ];

        for encoded in cases {
            let mut headers = HeaderMap::new();
            headers.insert(
                "luminate-delegation",
                HeaderValue::from_str(&encoded).expect("ASCII test header"),
            );
            assert!(delegation_claims(&headers).is_err(), "accepted {encoded}");
        }

        let oversized_groups = vec!["group"; super::MAX_DELEGATION_GROUPS + 1];
        let encoded = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&serde_json::json!({
                "authority": "proxy",
                "subject": "alice",
                "verified_groups": oversized_groups,
                "expires_at": now + 60,
            }))
            .expect("serialize claims"),
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            "luminate-delegation",
            HeaderValue::from_str(&encoded).expect("header value"),
        );
        assert!(delegation_claims(&headers).is_err());
    }

    #[test]
    fn route_classification_is_exact() {
        for path in [
            "/health/live",
            "/health/ready",
            "/api/v0/openapi.json",
            "/api/v0/ws/events",
            "/api/v0/ws/frames",
        ] {
            assert!(is_unprotected_path(path), "{path}");
        }
        for path in ["/health", "/api/v0/ws", "/api/v0/ws/events/"] {
            assert!(!is_unprotected_path(path), "{path}");
        }
        assert!(is_websocket_path("/api/v0/ws/events"));
        assert!(is_websocket_path("/api/v0/ws/frames"));
        assert!(!is_websocket_path("/api/v0/ws/events/"));
    }
}
