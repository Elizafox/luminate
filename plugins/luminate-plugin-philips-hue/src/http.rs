// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Bounded authenticated HTTPS/HTTP 1.1 transport for Hue CLIP v2.

use std::io::{self, Read as _, Write as _};
use std::str;
use std::sync::Arc;
use std::time::Duration;

use luminate_plugin_api::current_request_deadline;
use rustls::ClientConfig;
use serde::Deserialize;
use thiserror::Error;

use crate::configuration::{ApplicationKey, BridgeId, Endpoint};
use crate::tls::{self, TlsError};

const TIMEOUT: Duration = Duration::from_secs(3);
const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const PAIRING_DEVICE_TYPE: &str = "luminate#local";

pub(crate) struct HttpClient {
    tls: Arc<ClientConfig>,
    bridge_id: BridgeId,
    application_key: ApplicationKey,
}

impl HttpClient {
    pub(crate) fn new(
        bridge_id: BridgeId,
        application_key: ApplicationKey,
    ) -> Result<Self, HttpError> {
        Ok(Self {
            tls: tls::client_config(&bridge_id)?,
            bridge_id,
            application_key,
        })
    }

    pub(crate) fn get(&self, endpoint: &Endpoint, path: &str) -> Result<Vec<u8>, HttpError> {
        self.request(endpoint, "GET", path, None)
    }

    pub(crate) fn put(
        &self,
        endpoint: &Endpoint,
        path: &str,
        body: &[u8],
    ) -> Result<Vec<u8>, HttpError> {
        self.request(endpoint, "PUT", path, Some(body))
    }

    fn request(
        &self,
        endpoint: &Endpoint,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<Vec<u8>, HttpError> {
        validate_path(path)?;
        let timeout = current_request_deadline()
            .map_or(Some(TIMEOUT), |deadline| deadline.cap(TIMEOUT))
            .filter(|timeout| !timeout.is_zero())
            .ok_or(HttpError::TimedOut)?;
        let mut stream = tls::connect(Arc::clone(&self.tls), endpoint, &self.bridge_id, timeout)?;
        let request = render_request(
            method,
            path,
            self.bridge_id.as_str(),
            self.application_key.expose(),
            body,
        )?;
        stream.write_all(&request)?;
        stream.flush()?;

        let response = read_response(stream)?;
        parse_response(&response)
    }
}

pub(crate) fn create_application_key(
    bridge_id: &BridgeId,
    endpoint: &Endpoint,
) -> Result<ApplicationKey, HttpError> {
    let timeout = current_request_deadline()
        .map_or(Some(TIMEOUT), |deadline| deadline.cap(TIMEOUT))
        .filter(|timeout| !timeout.is_zero())
        .ok_or(HttpError::TimedOut)?;
    let config = tls::client_config(bridge_id)?;
    let mut stream = tls::connect(config, endpoint, bridge_id, timeout)?;
    let body = format!(r#"{{"devicetype":"{PAIRING_DEVICE_TYPE}","generateclientkey":true}}"#);
    let request = render_pairing_request(bridge_id.as_str(), body.as_bytes())?;
    stream.write_all(&request)?;
    stream.flush()?;
    let response = read_response(stream)?;
    let body = parse_response(&response)?;
    parse_pairing_response(&body)
}

fn read_response(reader: impl io::Read) -> Result<Vec<u8>, HttpError> {
    let mut response = Vec::new();
    let result = reader
        .take(u64::try_from(MAX_RESPONSE_BYTES + 1).unwrap_or(u64::MAX))
        .read_to_end(&mut response);
    if let Err(error) = result
        && (error.kind() != io::ErrorKind::UnexpectedEof || response.is_empty())
    {
        return Err(error.into());
    }
    if response.len() > MAX_RESPONSE_BYTES {
        return Err(HttpError::Protocol(
            "Hue HTTP response is too large".to_owned(),
        ));
    }
    Ok(response)
}

fn render_pairing_request(bridge_id: &str, body: &[u8]) -> Result<Vec<u8>, HttpError> {
    let mut request = Vec::new();
    write!(
        request,
        "POST /api HTTP/1.1\r\nHost: {bridge_id}\r\nAccept: application/json\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )?;
    request.extend_from_slice(body);
    Ok(request)
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PairingEntry {
    Success { success: PairingSuccess },
    Error { error: PairingError },
}

#[derive(Deserialize)]
struct PairingSuccess {
    username: String,
}

#[derive(Deserialize)]
struct PairingError {
    #[serde(rename = "type")]
    kind: u32,
}

fn parse_pairing_response(body: &[u8]) -> Result<ApplicationKey, HttpError> {
    let entries = serde_json::from_slice::<Vec<PairingEntry>>(body)
        .map_err(|error| HttpError::Protocol(format!("invalid Hue pairing response: {error}")))?;
    let [entry] = <Vec<PairingEntry> as TryInto<[PairingEntry; 1]>>::try_into(entries)
        .map_err(|_| HttpError::Protocol("ambiguous Hue pairing response".to_owned()))?;
    match entry {
        PairingEntry::Success { success } => ApplicationKey::parse(success.username)
            .map_err(|error| HttpError::Protocol(error.to_string())),
        PairingEntry::Error { error } if error.kind == 101 => Err(HttpError::Protocol(
            "Hue Bridge link button was not pressed".to_owned(),
        )),
        PairingEntry::Error { error } => Err(HttpError::Protocol(format!(
            "Hue Bridge rejected pairing with error {}",
            error.kind
        ))),
    }
}

fn validate_path(path: &str) -> Result<(), HttpError> {
    if !path.starts_with("/clip/v2/")
        || path
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
    {
        Err(HttpError::Protocol(
            "invalid Hue API request path".to_owned(),
        ))
    } else {
        Ok(())
    }
}

fn render_request(
    method: &str,
    path: &str,
    bridge_id: &str,
    application_key: &str,
    body: Option<&[u8]>,
) -> Result<Vec<u8>, HttpError> {
    validate_path(path)?;
    if !matches!(method, "GET" | "PUT") {
        return Err(HttpError::Protocol(
            "unsupported Hue HTTP method".to_owned(),
        ));
    }
    let body = body.unwrap_or_default();
    let mut request = Vec::new();
    write!(
        request,
        "{method} {path} HTTP/1.1\r\nHost: {bridge_id}\r\nAccept: application/json\r\nhue-application-key: {application_key}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    )?;
    if !body.is_empty() {
        request.extend_from_slice(b"Content-Type: application/json\r\n");
    }
    request.extend_from_slice(b"\r\n");
    request.extend_from_slice(body);
    Ok(request)
}

fn parse_response(response: &[u8]) -> Result<Vec<u8>, HttpError> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| HttpError::Protocol("incomplete Hue HTTP response".to_owned()))?;
    if header_end > MAX_HEADER_BYTES {
        return Err(HttpError::Protocol(
            "Hue HTTP headers are too large".to_owned(),
        ));
    }
    let header_bytes = response
        .get(..header_end)
        .ok_or_else(|| HttpError::Protocol("invalid Hue HTTP header boundary".to_owned()))?;
    let headers = str::from_utf8(header_bytes)
        .map_err(|_| HttpError::Protocol("Hue returned non-UTF-8 HTTP headers".to_owned()))?;
    let mut lines = headers.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| HttpError::Protocol("Hue omitted the HTTP status".to_owned()))?;
    let mut status = status_line.splitn(3, ' ');
    if status.next() != Some("HTTP/1.1") {
        return Err(HttpError::Protocol(
            "unsupported Hue HTTP version".to_owned(),
        ));
    }
    let code = status
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| HttpError::Protocol("invalid Hue HTTP status".to_owned()))?;
    let reason = status.next().unwrap_or_default().to_owned();

    let mut content_length = None;
    let mut content_type_json = false;
    let mut chunked = false;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| HttpError::Protocol("malformed Hue HTTP header".to_owned()))?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            let parsed = value
                .parse::<usize>()
                .ok()
                .filter(|length| *length <= MAX_RESPONSE_BYTES)
                .ok_or_else(|| HttpError::Protocol("invalid Hue content length".to_owned()))?;
            if content_length.replace(parsed).is_some() {
                return Err(HttpError::Protocol(
                    "duplicate Hue content length".to_owned(),
                ));
            }
        } else if name.eq_ignore_ascii_case("content-type") {
            content_type_json = value
                .split(';')
                .next()
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"));
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            chunked = value.eq_ignore_ascii_case("chunked");
            if !chunked {
                return Err(HttpError::Protocol(
                    "unsupported Hue transfer encoding".to_owned(),
                ));
            }
        }
    }
    if (300..400).contains(&code) {
        return Err(HttpError::Redirect(code));
    }
    if !(200..300).contains(&code) {
        return Err(HttpError::Status { code, reason });
    }
    if !content_type_json {
        return Err(HttpError::Protocol(
            "Hue response is not application/json".to_owned(),
        ));
    }
    if chunked && content_length.is_some() {
        return Err(HttpError::Protocol(
            "Hue response has ambiguous framing".to_owned(),
        ));
    }
    let body = response
        .get(header_end + 4..)
        .ok_or_else(|| HttpError::Protocol("invalid Hue HTTP body boundary".to_owned()))?;
    if chunked {
        decode_chunked(body)
    } else {
        let expected = content_length.ok_or_else(|| {
            HttpError::Protocol("Hue response omitted content framing".to_owned())
        })?;
        if body.len() != expected {
            return Err(HttpError::Protocol(
                "Hue response content length does not match its body".to_owned(),
            ));
        }
        Ok(body.to_vec())
    }
}

fn decode_chunked(mut input: &[u8]) -> Result<Vec<u8>, HttpError> {
    let mut output = Vec::new();
    loop {
        let line_end = input
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| HttpError::Protocol("invalid Hue chunk framing".to_owned()))?;
        let size_line = input
            .get(..line_end)
            .ok_or_else(|| HttpError::Protocol("invalid Hue chunk boundary".to_owned()))?;
        let size = str::from_utf8(size_line)
            .ok()
            .and_then(|line| line.split(';').next())
            .and_then(|value| usize::from_str_radix(value.trim(), 16).ok())
            .ok_or_else(|| HttpError::Protocol("invalid Hue chunk size".to_owned()))?;
        input = input
            .get(line_end + 2..)
            .ok_or_else(|| HttpError::Protocol("invalid Hue chunk boundary".to_owned()))?;
        if size == 0 {
            if input == b"\r\n" {
                return Ok(output);
            }
            return Err(HttpError::Protocol(
                "unsupported Hue chunk trailers".to_owned(),
            ));
        }
        let chunk = input
            .get(..size)
            .ok_or_else(|| HttpError::Protocol("truncated Hue chunk".to_owned()))?;
        output.extend_from_slice(chunk);
        if output.len() > MAX_RESPONSE_BYTES || input.get(size..size + 2) != Some(b"\r\n") {
            return Err(HttpError::Protocol("invalid Hue chunk body".to_owned()));
        }
        input = input
            .get(size + 2..)
            .ok_or_else(|| HttpError::Protocol("invalid Hue chunk boundary".to_owned()))?;
    }
}

#[derive(Debug, Error)]
pub(crate) enum HttpError {
    #[error("Hue request deadline expired")]
    TimedOut,
    #[error(transparent)]
    Tls(#[from] TlsError),
    #[error("Hue transport failed: {0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Protocol(String),
    #[error("Hue rejected redirects with HTTP {0}")]
    Redirect(u16),
    #[error("Hue returned HTTP {code} {reason}")]
    Status { code: u16, reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ResponseWithoutCloseNotify {
        response: io::Cursor<Vec<u8>>,
    }

    impl io::Read for ResponseWithoutCloseNotify {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let length = self.response.read(buffer)?;
            if length == 0 {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "peer closed without close_notify",
                ))
            } else {
                Ok(length)
            }
        }
    }

    #[test]
    fn renders_minimal_authenticated_json_request() {
        let request = render_request(
            "PUT",
            "/clip/v2/resource/light/id",
            "001788fffe123456",
            "test-secret",
            Some(br#"{"on":{"on":true}}"#),
        )
        .expect("render request");
        let request = str::from_utf8(&request).expect("UTF-8 request");
        assert!(request.contains("hue-application-key: test-secret\r\n"));
        assert!(request.contains("Host: 001788fffe123456\r\n"));
        assert_eq!(request.matches("hue-application-key").count(), 1);
        assert!(request.contains("Content-Type: application/json\r\n"));
    }

    #[test]
    fn parses_content_length_and_chunked_json() {
        assert_eq!(
            parse_response(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}"
            )
            .expect("content-length response"),
            b"{}"
        );
        assert_eq!(
            parse_response(b"HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n")
                .expect("chunked response"),
            b"{}"
        );
    }

    #[test]
    fn retains_a_framed_response_when_tls_close_notify_is_missing() {
        let response =
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}";
        let response = read_response(ResponseWithoutCloseNotify {
            response: io::Cursor::new(response.to_vec()),
        })
        .expect("HTTP framing can validate the response after an abrupt TLS close");

        assert_eq!(parse_response(&response).expect("framed response"), b"{}");
    }

    #[test]
    fn rejects_an_abrupt_tls_close_before_any_response() {
        let error = read_response(ResponseWithoutCloseNotify {
            response: io::Cursor::new(Vec::new()),
        })
        .expect_err("an empty abrupt close is not a response");

        assert!(matches!(error, HttpError::Io(_)));
    }

    #[test]
    fn rejects_redirects_bad_content_types_and_ambiguous_framing() {
        assert!(matches!(
            parse_response(b"HTTP/1.1 302 Found\r\nLocation: https://example.test/\r\nContent-Length: 0\r\n\r\n"),
            Err(HttpError::Redirect(302))
        ));
        assert!(
            parse_response(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\n{}"
            )
            .is_err()
        );
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\n\r\n{}").is_err());
    }

    #[test]
    fn rejects_header_injection_and_paths_outside_clip_v2() {
        assert!(render_request("GET", "/api/config", "bridge", "key", None).is_err());
        assert!(
            render_request(
                "GET",
                "/clip/v2/resource\r\nInjected: yes",
                "bridge",
                "key",
                None
            )
            .is_err()
        );
    }

    #[test]
    fn parses_pairing_success_without_exposing_auxiliary_secrets() {
        let key = parse_pairing_response(
            br#"[{"success":{"username":"generated-application-key","clientkey":"ignored-entertainment-key"}}]"#,
        )
        .expect("valid pairing response");
        assert_eq!(key.expose(), "generated-application-key");
    }

    #[test]
    fn reports_an_unpressed_link_button() {
        let error = parse_pairing_response(
            br#"[{"error":{"type":101,"address":"","description":"link button not pressed"}}]"#,
        )
        .expect_err("pairing should require the link button");
        assert!(error.to_string().contains("link button was not pressed"));
    }
}
