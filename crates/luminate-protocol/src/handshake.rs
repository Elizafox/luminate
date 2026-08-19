// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Version-negotiating handshakes for request and event connections.

use serde::{Deserialize, Serialize};

use crate::PROTOCOL_ABI_VERSION;

/// Initial message sent by a client opening a control connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientHello {
    /// Request protocol version required by the client.
    pub protocol_abi_version: u32,

    /// Human-readable client name.
    pub client_name: String,

    /// Human-readable client version.
    pub client_version: String,
}

impl ClientHello {
    /// Constructs a client hello for the current request protocol.
    #[must_use]
    pub fn new(client_name: impl Into<String>, client_version: impl Into<String>) -> Self {
        Self {
            protocol_abi_version: PROTOCOL_ABI_VERSION,
            client_name: client_name.into(),
            client_version: client_version.into(),
        }
    }
}

/// Daemon response to a control-connection handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonHello {
    /// Whether the requested protocol is compatible.
    pub compatibility: Compatibility,

    /// Request protocol version reported by the daemon.
    pub protocol_abi_version: u32,

    /// Human-readable daemon version.
    pub daemon_version: String,
}

/// Compatibility result for a request protocol handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Compatibility {
    /// The client and daemon can exchange requests.
    Compatible,

    /// The requested protocol is incompatible.
    Incompatible {
        /// Request protocol version supported by the daemon.
        supported_protocol_abi_version: u32,

        /// Optional human-readable explanation.
        reason: Option<String>,
    },
}
