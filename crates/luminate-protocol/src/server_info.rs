// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Daemon identity and protocol-version metadata reported to clients.

use serde::{Deserialize, Serialize};

/// Daemon identity and protocol metadata returned to a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Human-readable daemon name.
    pub daemon_name: String,

    /// Human-readable daemon version.
    pub daemon_version: String,

    /// Request protocol version spoken by the daemon.
    pub protocol_abi_version: u32,
}
