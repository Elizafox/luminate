// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Errors produced by client transport, protocol, and daemon operations.

use ciborium::de;
use ciborium::ser;
use std::error;
use std::io::{self, ErrorKind as IoErrorKind};
use std::result;

use luminate_core::target::TargetId;
use luminate_protocol::framing::FramingError;
use luminate_protocol::{ErrorCode, OperationError};
use tokio::time::error::Elapsed;

/// Stable category for a [`Error`], independent of its diagnostic metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The daemon cannot be reached.
    DaemonUnavailable,

    /// The daemon rejected the selected authentication method or credential.
    AuthenticationFailed,

    /// The primary protocol is incompatible.
    IncompatibleDaemon,

    /// The event protocol is incompatible.
    IncompatibleEventSocket,

    /// The operation is not authorized.
    PermissionDenied,

    /// The requested object does not exist.
    NotFound,

    /// The target does not support the operation.
    Unsupported,

    /// Required hardware state is unknown.
    UnknownState,

    /// An argument is invalid.
    InvalidArgument,

    /// An internal component failed.
    Internal,

    /// Transport input or output failed.
    Io,

    /// The addressed hardware is temporarily unavailable.
    Unavailable,

    /// A provider temporarily rate-limited work.
    RateLimited,

    /// A consumer protocol violation occurred.
    Protocol,

    /// A bounded operation timed out.
    Timeout,

    /// A connection became unsafe to reuse.
    ConnectionPoisoned,

    /// Some targets changed before a fan-out failed.
    PartialMutation,

    /// A frame stream or hardware effect already owns the target in a way
    /// incompatible with the requested operation.
    Conflict,

    /// Complete transition preflight found incompatible or unknown endpoints.
    TransitionImpossible,
}

#[allow(
    clippy::error_impl_error,
    reason = "This is the public libluminate error type; renaming it would churn the API."
)]
#[derive(Debug, thiserror::Error)]
/// Failures reported by the daemon, transport, framing layer, or client safety
/// checks.
pub enum Error {
    /// The daemon socket does not exist or refused the connection.
    #[error("daemon is not available")]
    DaemonUnavailable,

    /// The daemon rejected the selected authentication method or credential.
    #[error("authentication failed: {0}")]
    AuthenticationFailed(String),

    /// The daemon and client do not share a compatible request protocol.
    #[error("daemon is incompatible: protocol ABI {supported_protocol_abi_version} required")]
    IncompatibleDaemon {
        /// Version string advertised by the daemon.
        daemon_version: String,

        /// Protocol ABI version accepted by the daemon.
        supported_protocol_abi_version: u32,

        /// Optional diagnostic supplied during the handshake.
        reason: Option<String>,
    },

    /// The daemon and client do not share a compatible event protocol.
    #[error(
        "daemon event socket is incompatible: event protocol {supported_event_protocol_version} required"
    )]
    IncompatibleEventSocket {
        /// Version string advertised by the daemon.
        daemon_version: String,

        /// Event protocol version accepted by the daemon.
        supported_event_protocol_version: u32,

        /// Optional diagnostic supplied during the event handshake.
        reason: Option<String>,
    },

    /// The daemon rejected the operation for lack of permission.
    #[error("permission denied")]
    PermissionDenied {
        /// Safe diagnostic supplied by the daemon's authorization policy, if
        /// any. Never contains secrets, directory details, or hidden device
        /// identities.
        reason: Option<String>,
    },

    /// A requested device or target was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// The selected target cannot perform the requested operation.
    #[error("unsupported operation: {0}")]
    Unsupported(String),

    /// Exact hardware state was required but unavailable.
    #[error("unknown state: {0}")]
    UnknownState(String),

    /// An argument violated target or operation constraints.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// The daemon or client encountered an internal failure.
    #[error("internal error: {0}")]
    Internal(String),

    /// A Unix-socket or other input/output operation failed.
    #[error("I/O error: {0}")]
    Io(String),

    /// The addressed hardware is temporarily absent or unreachable.
    #[error("device unavailable: {0}")]
    Unavailable(String),

    /// The hardware or provider temporarily refused work because of a rate limit.
    #[error("rate limited: {message}")]
    RateLimited {
        /// Provider diagnostic.
        message: String,

        /// Minimum delay before retrying, in milliseconds.
        retry_after_ms: Option<u64>,
    },

    /// Some fan-out targets were changed before a later target failed.
    #[error("partial mutation: {message}")]
    PartialMutation {
        /// Diagnostic describing the failed portion.
        message: String,

        /// Targets successfully changed and committed by the daemon.
        applied_targets: Vec<TargetId>,
    },

    /// A frame or payload violated the consumer protocol.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// A frame stream or hardware effect already owns the target in a way
    /// incompatible with the requested operation.
    #[error("conflict: {0}")]
    Conflict(String),

    /// Complete preflight proved that no safe transition can be constructed.
    #[error("transition impossible: {0}")]
    TransitionImpossible(String),

    /// A bounded handshake or request operation timed out.
    #[error("Operation timed out: {0}")]
    Timeout(#[from] Elapsed),

    /// A streaming receive was cancelled or failed after partial I/O.
    ///
    /// This applies to [`crate::EventSubscription`], whose event stream is not
    /// correlated. Ordinary client requests are cancellation-safe.
    #[error(
        "stream is desynchronized after a previous receive failed or was cancelled mid-flight; \
         reconnect"
    )]
    ConnectionPoisoned,
}

impl Error {
    /// Returns the stable category of this error.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::DaemonUnavailable => ErrorKind::DaemonUnavailable,
            Self::AuthenticationFailed(_) => ErrorKind::AuthenticationFailed,
            Self::IncompatibleDaemon { .. } => ErrorKind::IncompatibleDaemon,
            Self::IncompatibleEventSocket { .. } => ErrorKind::IncompatibleEventSocket,
            Self::PermissionDenied { .. } => ErrorKind::PermissionDenied,
            Self::NotFound(_) => ErrorKind::NotFound,
            Self::Unsupported(_) => ErrorKind::Unsupported,
            Self::UnknownState(_) => ErrorKind::UnknownState,
            Self::InvalidArgument(_) => ErrorKind::InvalidArgument,
            Self::Internal(_) => ErrorKind::Internal,
            Self::Io(_) => ErrorKind::Io,
            Self::Unavailable(_) => ErrorKind::Unavailable,
            Self::RateLimited { .. } => ErrorKind::RateLimited,
            Self::Protocol(_) => ErrorKind::Protocol,
            Self::Timeout(_) => ErrorKind::Timeout,
            Self::ConnectionPoisoned => ErrorKind::ConnectionPoisoned,
            Self::PartialMutation { .. } => ErrorKind::PartialMutation,
            Self::Conflict(_) => ErrorKind::Conflict,
            Self::TransitionImpossible(_) => ErrorKind::TransitionImpossible,
        }
    }

    pub(crate) fn from_operation_error(error: OperationError) -> Self {
        let OperationError {
            code,
            message,
            retry_after_ms,
            applied_targets,
        } = error;
        match code {
            ErrorCode::DaemonUnavailable => Self::DaemonUnavailable,
            ErrorCode::AuthenticationFailed => Self::AuthenticationFailed(message),
            ErrorCode::PermissionDenied => Self::PermissionDenied {
                reason: (!message.is_empty()).then_some(message),
            },
            ErrorCode::NotFound => Self::NotFound(message),
            ErrorCode::Unsupported => Self::Unsupported(message),
            ErrorCode::UnknownState => Self::UnknownState(message),
            ErrorCode::InvalidArgument => Self::InvalidArgument(message),
            ErrorCode::Internal => Self::Internal(message),
            ErrorCode::Io => Self::Io(message),
            ErrorCode::Unavailable => Self::Unavailable(message),
            ErrorCode::RateLimited => Self::RateLimited {
                message,
                retry_after_ms,
            },
            ErrorCode::PartialMutation => Self::PartialMutation {
                message,
                applied_targets,
            },
            ErrorCode::Conflict => Self::Conflict(message),
            ErrorCode::TransitionImpossible => Self::TransitionImpossible(message),
        }
    }
}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        if matches!(
            value.kind(),
            IoErrorKind::NotFound | IoErrorKind::ConnectionRefused
        ) {
            Self::DaemonUnavailable
        } else {
            Self::Io(value.to_string())
        }
    }
}

impl<T: error::Error> From<de::Error<T>> for Error {
    fn from(value: de::Error<T>) -> Self {
        Self::Protocol(value.to_string())
    }
}

impl<T: error::Error> From<ser::Error<T>> for Error {
    fn from(value: ser::Error<T>) -> Self {
        Self::Protocol(value.to_string())
    }
}

impl From<FramingError> for Error {
    fn from(value: FramingError) -> Self {
        match value {
            FramingError::Io(io_error) => io_error.into(),
            FramingError::CborEncode(cbor_error) => cbor_error.into(),
            FramingError::CborDecode(cbor_error) => cbor_error.into(),
            other @ (FramingError::PayloadTooLarge(_) | FramingError::FrameTooLarge { .. }) => {
                Self::Protocol(other.to_string())
            }
            FramingError::Timeout(elapsed) => Self::Timeout(elapsed),
        }
    }
}

/// A libluminate operation result.
pub type Result<T> = result::Result<T, Error>;

impl Error {
    #[must_use]
    /// Returns an owned, human-readable diagnostic for this error.
    pub fn message(&self) -> String {
        self.to_string()
    }

    /// Returns provider retry guidance in milliseconds, when available.
    #[must_use]
    #[inline]
    pub const fn retry_after_ms(&self) -> Option<u64> {
        if let Self::RateLimited { retry_after_ms, .. } = self {
            *retry_after_ms
        } else {
            None
        }
    }

    /// Returns the daemon-supplied denial diagnostic, when this is a
    /// [`Self::PermissionDenied`] error.
    #[must_use]
    #[inline]
    pub fn permission_denied_reason(&self) -> Option<&str> {
        if let Self::PermissionDenied { reason } = self {
            reason.as_deref()
        } else {
            None
        }
    }

    /// Returns targets changed before a partial mutation failed.
    #[must_use]
    #[inline]
    pub fn applied_targets(&self) -> &[TargetId] {
        if let Self::PartialMutation {
            applied_targets, ..
        } = self
        {
            applied_targets
        } else {
            &[]
        }
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
