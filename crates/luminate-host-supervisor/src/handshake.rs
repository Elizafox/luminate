// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Version negotiation between `luminated` and a supervised host child.

use serde::{Deserialize, Serialize};

use crate::HOST_SUPERVISOR_PROTOCOL_VERSION;

/// Sent once by the supervisor immediately after spawning a host child.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupervisorHello {
    pub protocol_version: u32,
}

impl SupervisorHello {
    #[must_use]
    #[inline]
    pub const fn new() -> Self {
        Self {
            protocol_version: HOST_SUPERVISOR_PROTOCOL_VERSION,
        }
    }
}

impl Default for SupervisorHello {
    fn default() -> Self {
        Self::new()
    }
}

/// Sent once by the host child in response to [`SupervisorHello`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostHello {
    pub compatibility: Compatibility,

    pub protocol_version: u32,

    pub host_version: String,
}

/// A version mismatch is reported explicitly rather than inferred from a
/// dropped connection, so the supervisor can log a clear diagnostic before
/// refusing to proceed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Compatibility {
    Compatible,
    Incompatible {
        supported_protocol_version: u32,
        reason: Option<String>,
    },
}

impl Compatibility {
    /// Checks `peer_protocol_version` (as reported by the other side of the
    /// handshake) against this build's [`HOST_SUPERVISOR_PROTOCOL_VERSION`].
    #[must_use]
    #[inline]
    pub fn check(peer_protocol_version: u32) -> Self {
        if peer_protocol_version == HOST_SUPERVISOR_PROTOCOL_VERSION {
            Self::Compatible
        } else {
            Self::Incompatible {
                supported_protocol_version: HOST_SUPERVISOR_PROTOCOL_VERSION,
                reason: None,
            }
        }
    }
}

#[cfg(test)]
#[path = "handshake_tests.rs"]
mod tests;
