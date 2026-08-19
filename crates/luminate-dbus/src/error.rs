// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! D-Bus method errors exposed by the compatibility service.

use crate::path::canonical_id;

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.luminate.Error", impl_display = true)]
pub enum MethodError {
    DaemonUnavailable(String),
    AuthenticationFailed(String),
    IncompatibleDaemon(String),
    PermissionDenied(String),
    NotFound(String),
    Unsupported(String),
    UnknownState(String),
    InvalidArgument(String),
    Internal(String),
    Io(String),
    Unavailable(String),
    RateLimited(String, bool, u64),
    PartialMutation(String, Vec<String>),
    Protocol(String),
    Timeout(String),
    ConnectionPoisoned(String),
    Conflict(String),
}

impl From<luminate::Error> for MethodError {
    fn from(error: luminate::Error) -> Self {
        let message = error.to_string();
        let retry_after_ms = error.retry_after_ms();
        let applied_targets = error
            .applied_targets()
            .iter()
            .map(canonical_id)
            .collect::<Vec<_>>();
        match error {
            luminate::Error::DaemonUnavailable => Self::DaemonUnavailable(message),
            luminate::Error::AuthenticationFailed(_) => Self::AuthenticationFailed(message),
            luminate::Error::IncompatibleDaemon { .. }
            | luminate::Error::IncompatibleEventSocket { .. } => Self::IncompatibleDaemon(message),
            luminate::Error::PermissionDenied { .. } => Self::PermissionDenied(message),
            luminate::Error::NotFound(_) => Self::NotFound(message),
            luminate::Error::Unsupported(_) => Self::Unsupported(message),
            luminate::Error::UnknownState(_) => Self::UnknownState(message),
            luminate::Error::InvalidArgument(_) | luminate::Error::TransitionImpossible(_) => {
                Self::InvalidArgument(message)
            }
            luminate::Error::Internal(_) => Self::Internal(message),
            luminate::Error::Io(_) => Self::Io(message),
            luminate::Error::Unavailable(_) => Self::Unavailable(message),
            luminate::Error::RateLimited { .. } => Self::RateLimited(
                message,
                retry_after_ms.is_some(),
                retry_after_ms.unwrap_or_default(),
            ),
            luminate::Error::PartialMutation { .. } => {
                Self::PartialMutation(message, applied_targets)
            }
            luminate::Error::Protocol(_) => Self::Protocol(message),
            luminate::Error::Timeout(_) => Self::Timeout(message),
            luminate::Error::ConnectionPoisoned => Self::ConnectionPoisoned(message),
            luminate::Error::Conflict(_) => Self::Conflict(message),
        }
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
