// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::future;
use std::io;
use std::time::Duration;

use serde::Serialize;
use serde::ser::Error as _;
use tokio::time;

use super::*;

#[derive(Debug)]
struct RefusesToSerialize;

impl Serialize for RefusesToSerialize {
    fn serialize<S>(&self, _serializer: S) -> result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Err(S::Error::custom("deliberate encoding failure"))
    }
}

fn deliberate_encode_error() -> ser::Error<io::Error> {
    ciborium::into_writer(&RefusesToSerialize, Vec::new())
        .expect_err("serializer should reject the value")
}

#[test]
fn daemon_error_codes_map_to_public_errors() {
    let cases = [
        (ErrorCode::DaemonUnavailable, "daemon is not available"),
        (ErrorCode::PermissionDenied, "permission denied"),
        (ErrorCode::NotFound, "not found: detail"),
        (ErrorCode::Unsupported, "unsupported operation: detail"),
        (ErrorCode::UnknownState, "unknown state: detail"),
        (ErrorCode::InvalidArgument, "invalid argument: detail"),
        (ErrorCode::Internal, "internal error: detail"),
        (ErrorCode::Io, "I/O error: detail"),
        (ErrorCode::Unavailable, "device unavailable: detail"),
        (ErrorCode::RateLimited, "rate limited: detail"),
        (ErrorCode::Conflict, "conflict: detail"),
    ];

    for (code, expected) in cases {
        assert_eq!(
            Error::from_operation_error(OperationError {
                code,
                message: "detail".to_owned(),
                retry_after_ms: None,
                applied_targets: Vec::new(),
            })
            .message(),
            expected
        );
    }
}

#[test]
fn structured_metadata_is_available_without_parsing_messages() {
    let applied = TargetId::device("lamp0");
    let rate_limit = Error::from_operation_error(OperationError {
        code: ErrorCode::RateLimited,
        message: "provider is cooling down".to_owned(),
        retry_after_ms: Some(750),
        applied_targets: Vec::new(),
    });
    assert_eq!(rate_limit.retry_after_ms(), Some(750));
    assert_eq!(rate_limit.kind(), ErrorKind::RateLimited);
    assert!(rate_limit.applied_targets().is_empty());

    let partial = Error::from_operation_error(OperationError {
        code: ErrorCode::PartialMutation,
        message: "second device failed".to_owned(),
        retry_after_ms: None,
        applied_targets: vec![applied.clone()],
    });
    assert_eq!(partial.retry_after_ms(), None);
    assert_eq!(partial.kind(), ErrorKind::PartialMutation);
    assert_eq!(partial.applied_targets(), [applied]);
}

#[test]
fn unavailable_io_kinds_are_distinct_from_other_io_errors() {
    for kind in [IoErrorKind::NotFound, IoErrorKind::ConnectionRefused] {
        assert!(matches!(
            Error::from(io::Error::from(kind)),
            Error::DaemonUnavailable
        ));
    }

    let error = Error::from(io::Error::new(IoErrorKind::BrokenPipe, "peer vanished"));
    assert!(matches!(error, Error::Io(message) if message == "peer vanished"));
}

#[test]
fn cbor_errors_become_protocol_errors() {
    let decode_error = ciborium::from_reader::<String, _>([0xff].as_slice())
        .expect_err("break is not a standalone CBOR string");
    assert!(matches!(Error::from(decode_error), Error::Protocol(_)));

    let encode_error = deliberate_encode_error();
    assert!(
        matches!(Error::from(encode_error), Error::Protocol(message) if message.contains("deliberate encoding failure"))
    );
}

#[tokio::test]
async fn kind_covers_every_error_variant() {
    let elapsed = time::timeout(Duration::ZERO, future::pending::<()>())
        .await
        .expect_err("pending future should time out");

    let cases: Vec<(Error, ErrorKind)> = vec![
        (Error::Timeout(elapsed), ErrorKind::Timeout),
        (Error::DaemonUnavailable, ErrorKind::DaemonUnavailable),
        (
            Error::AuthenticationFailed("rejected".to_owned()),
            ErrorKind::AuthenticationFailed,
        ),
        (
            Error::IncompatibleDaemon {
                daemon_version: "1.0".to_owned(),
                supported_protocol_abi_version: 7,
                reason: None,
            },
            ErrorKind::IncompatibleDaemon,
        ),
        (
            Error::IncompatibleEventSocket {
                daemon_version: "1.0".to_owned(),
                supported_event_protocol_version: 3,
                reason: None,
            },
            ErrorKind::IncompatibleEventSocket,
        ),
        (
            Error::PermissionDenied { reason: None },
            ErrorKind::PermissionDenied,
        ),
        (Error::NotFound("lamp0".to_owned()), ErrorKind::NotFound),
        (
            Error::Unsupported("no effect support".to_owned()),
            ErrorKind::Unsupported,
        ),
        (
            Error::UnknownState("brightness".to_owned()),
            ErrorKind::UnknownState,
        ),
        (
            Error::InvalidArgument("bad colour".to_owned()),
            ErrorKind::InvalidArgument,
        ),
        (Error::Internal("oops".to_owned()), ErrorKind::Internal),
        (Error::Io("closed".to_owned()), ErrorKind::Io),
        (
            Error::Unavailable("offline".to_owned()),
            ErrorKind::Unavailable,
        ),
        (
            Error::RateLimited {
                message: "cooling down".to_owned(),
                retry_after_ms: None,
            },
            ErrorKind::RateLimited,
        ),
        (Error::Protocol("bad frame".to_owned()), ErrorKind::Protocol),
        (Error::ConnectionPoisoned, ErrorKind::ConnectionPoisoned),
        (
            Error::PartialMutation {
                message: "second device failed".to_owned(),
                applied_targets: Vec::new(),
            },
            ErrorKind::PartialMutation,
        ),
        (
            Error::Conflict("stream active".to_owned()),
            ErrorKind::Conflict,
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.kind(), expected, "{error}");
    }
}

#[test]
fn permission_denied_reason_and_applied_targets_default_to_absent() {
    assert_eq!(
        Error::PermissionDenied {
            reason: Some("policy denied it".to_owned())
        }
        .permission_denied_reason(),
        Some("policy denied it")
    );
    assert_eq!(Error::DaemonUnavailable.permission_denied_reason(), None);
    assert!(Error::DaemonUnavailable.applied_targets().is_empty());
}

#[tokio::test]
async fn framing_errors_preserve_their_public_category() {
    let io_error = FramingError::Io(io::Error::new(IoErrorKind::BrokenPipe, "closed"));
    assert!(matches!(Error::from(io_error), Error::Io(message) if message == "closed"));

    let too_large = FramingError::FrameTooLarge { len: 9, max: 8 };
    assert!(
        matches!(Error::from(too_large), Error::Protocol(message) if message.contains("9 bytes"))
    );

    let integer_error = u32::try_from(usize::MAX).expect_err("usize exceeds u32");
    let payload_error = FramingError::PayloadTooLarge(integer_error);
    assert!(
        matches!(Error::from(payload_error), Error::Protocol(message) if message.contains("too large"))
    );

    let decode_error = ciborium::from_reader::<String, _>([0xff].as_slice())
        .expect_err("invalid CBOR should fail");
    assert!(matches!(
        Error::from(FramingError::CborDecode(decode_error)),
        Error::Protocol(_)
    ));

    assert!(matches!(
        Error::from(FramingError::CborEncode(deliberate_encode_error())),
        Error::Protocol(message) if message.contains("deliberate encoding failure")
    ));

    let elapsed = time::timeout(Duration::ZERO, future::pending::<()>())
        .await
        .expect_err("pending future should time out");
    assert!(matches!(
        Error::from(FramingError::Timeout(elapsed)),
        Error::Timeout(_)
    ));
}
