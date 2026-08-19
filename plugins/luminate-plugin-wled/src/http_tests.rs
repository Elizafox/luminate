// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Bounded HTTP parsing, framing, and error-handling tests.

use super::*;

#[test]
fn parses_content_length_response() {
    let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
    assert_eq!(parse_response(response).expect("parse response"), b"{}");
}

#[test]
fn parses_chunked_response() {
    let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n";
    assert_eq!(parse_response(response).expect("parse response"), b"{}");
}

#[test]
fn rejects_error_status() {
    let response = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
    assert!(matches!(
        parse_response(response),
        Err(HttpError::Status {
            code: 404,
            retry_after: None,
            ..
        })
    ));
}

#[test]
fn retry_after_accepts_seconds_and_caps_the_delay() {
    let now = SystemTime::UNIX_EPOCH;
    assert_eq!(parse_retry_after("12", now), Some(Duration::from_secs(12)));
    assert_eq!(parse_retry_after("600", now), Some(MAX_RETRY_AFTER));
}

#[test]
fn retry_after_accepts_http_dates_and_past_dates() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(784_111_777);
    assert_eq!(
        parse_retry_after("Sun, 06 Nov 1994 08:49:49 GMT", now),
        Some(Duration::from_secs(12))
    );
    assert_eq!(
        parse_retry_after("Sun, 06 Nov 1994 08:49:25 GMT", now),
        Some(Duration::ZERO)
    );
}

#[test]
fn error_response_preserves_retry_after() {
    let response = b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 5\r\nContent-Length: 0\r\n\r\n";
    assert!(matches!(
        parse_response(response),
        Err(HttpError::Status {
            code: 429,
            retry_after: Some(delay),
            ..
        }) if delay == Duration::from_secs(5)
    ));
}
