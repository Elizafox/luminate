// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Browser origin policy, CORS preflights, and untrusted forwarding-header removal.

use std::collections::HashSet;
use std::net::IpAddr;

use anyhow::Context as _;
use axum::body::Body;
use axum::extract::State;
use axum::http::uri::Authority;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};

use crate::{AppState, AuthenticationMode, is_loopback, problem_response};

#[derive(Debug, Default)]
pub(super) struct OriginPolicy {
    allowed: HashSet<String>,
}

impl OriginPolicy {
    pub(super) fn from_configured(
        origins: &[String],
        mode: AuthenticationMode,
    ) -> anyhow::Result<Self> {
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

pub(super) async fn browser_and_header_boundary(
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

#[cfg(test)]
pub(super) async fn forwarded_headers_visible(headers: HeaderMap) -> StatusCode {
    if headers.keys().any(|name| {
        name.as_str().starts_with("x-forwarded-") || FORWARDING_HEADERS.contains(&name.as_str())
    }) {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::NO_CONTENT
    }
}

#[cfg(test)]
#[path = "browser_tests.rs"]
mod tests;
