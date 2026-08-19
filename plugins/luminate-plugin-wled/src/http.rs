// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Minimal bounded HTTP/1.1 client for the WLED JSON API.

use std::io::{self, Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::str;
use std::time::{Duration, SystemTime};

use luminate_plugin_api::current_request_deadline;
use thiserror::Error;

const TIMEOUT: Duration = Duration::from_secs(2);
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Endpoint {
    pub(crate) address: SocketAddr,
    pub(crate) host: String,
}

impl Endpoint {
    pub(crate) fn new(address: SocketAddr) -> Self {
        let mut host = match address {
            SocketAddr::V4(address) => address.ip().to_string(),
            SocketAddr::V6(address) => format!("[{}]", address.ip()),
        };
        if address.port() != 80 {
            host.push(':');
            host.push_str(&address.port().to_string());
        }
        Self { address, host }
    }
}

#[derive(Debug, Error)]
pub(crate) enum HttpError {
    #[error(transparent)]
    Io(io::Error),
    #[error("{0}")]
    Protocol(String),
    #[error("HTTP {code} {reason}")]
    Status {
        code: u16,
        reason: String,
        retry_after: Option<Duration>,
    },
}

impl From<io::Error> for HttpError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub(crate) fn get(endpoint: &Endpoint, path: &str) -> Result<Vec<u8>, HttpError> {
    request(endpoint, "GET", path, None)
}

pub(crate) fn post(endpoint: &Endpoint, path: &str, body: &[u8]) -> Result<Vec<u8>, HttpError> {
    request(endpoint, "POST", path, Some(body))
}

fn request(
    endpoint: &Endpoint,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
) -> Result<Vec<u8>, HttpError> {
    let timeout = current_request_deadline()
        .map_or(Some(TIMEOUT), |deadline| deadline.cap(TIMEOUT))
        .filter(|timeout| !timeout.is_zero())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::TimedOut, "plugin request deadline expired")
        })?;
    let mut stream = TcpStream::connect_timeout(&endpoint.address, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let body = body.unwrap_or_default();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\nContent-Length: {}\r\n",
        endpoint.host,
        body.len()
    )?;
    if !body.is_empty() {
        stream.write_all(b"Content-Type: application/json\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.write_all(body)?;
    stream.flush()?;

    let mut response = Vec::new();
    stream
        .take(u64::try_from(MAX_RESPONSE_BYTES + 1).unwrap_or(u64::MAX))
        .read_to_end(&mut response)?;
    if response.len() > MAX_RESPONSE_BYTES {
        return Err(HttpError::Protocol(
            "WLED HTTP response is too large".to_owned(),
        ));
    }
    parse_response(&response)
}

fn parse_response(response: &[u8]) -> Result<Vec<u8>, HttpError> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            HttpError::Protocol("WLED returned an incomplete HTTP response".to_owned())
        })?;
    let headers = response
        .get(..header_end)
        .ok_or_else(|| HttpError::Protocol("invalid HTTP header boundary".to_owned()))?;
    let headers = str::from_utf8(headers)
        .map_err(|_| HttpError::Protocol("WLED returned non-UTF-8 HTTP headers".to_owned()))?;
    let mut lines = headers.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| HttpError::Protocol("WLED omitted the HTTP status line".to_owned()))?;
    let mut status = status_line.splitn(3, ' ');
    let version = status.next().unwrap_or_default();
    if !version.starts_with("HTTP/1.") {
        return Err(HttpError::Protocol(
            "WLED returned an unsupported HTTP version".to_owned(),
        ));
    }
    let code = status
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| HttpError::Protocol("WLED returned an invalid HTTP status".to_owned()))?;
    let reason = status.next().unwrap_or_default().to_owned();
    let retry_after = lines.clone().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("retry-after")
            .then(|| parse_retry_after(value.trim(), SystemTime::now()))
            .flatten()
    });
    if !(200..300).contains(&code) {
        return Err(HttpError::Status {
            code,
            reason,
            retry_after,
        });
    }
    let body = response
        .get(header_end + 4..)
        .ok_or_else(|| HttpError::Protocol("invalid HTTP body boundary".to_owned()))?;
    let chunked = lines.any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("transfer-encoding")
                && value
                    .split(',')
                    .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
        })
    });
    if chunked {
        decode_chunked(body)
    } else {
        Ok(body.to_vec())
    }
}

fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let delay = value
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
        .or_else(|| {
            httpdate::parse_http_date(value)
                .ok()
                .map(|date| date.duration_since(now).unwrap_or(Duration::ZERO))
        })?;
    Some(delay.min(MAX_RETRY_AFTER))
}

fn decode_chunked(mut input: &[u8]) -> Result<Vec<u8>, HttpError> {
    let mut output = Vec::new();
    loop {
        let line_end = input
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| HttpError::Protocol("invalid chunked WLED response".to_owned()))?;
        let line = input
            .get(..line_end)
            .ok_or_else(|| HttpError::Protocol("invalid WLED HTTP chunk size".to_owned()))?;
        let size = str::from_utf8(line)
            .ok()
            .and_then(|line| line.split(';').next())
            .and_then(|value| usize::from_str_radix(value.trim(), 16).ok())
            .ok_or_else(|| HttpError::Protocol("invalid WLED HTTP chunk size".to_owned()))?;
        input = input
            .get(line_end + 2..)
            .ok_or_else(|| HttpError::Protocol("truncated WLED HTTP chunk".to_owned()))?;
        if size == 0 {
            return Ok(output);
        }
        let chunk = input
            .get(..size)
            .ok_or_else(|| HttpError::Protocol("truncated WLED HTTP chunk".to_owned()))?;
        output.extend_from_slice(chunk);
        if output.len() > MAX_RESPONSE_BYTES {
            return Err(HttpError::Protocol(
                "WLED HTTP response is too large".to_owned(),
            ));
        }
        if input.get(size..size + 2) != Some(b"\r\n") {
            return Err(HttpError::Protocol(
                "invalid WLED HTTP chunk terminator".to_owned(),
            ));
        }
        input = input
            .get(size + 2..)
            .ok_or_else(|| HttpError::Protocol("truncated WLED HTTP response".to_owned()))?;
    }
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
