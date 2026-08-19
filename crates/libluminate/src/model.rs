// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Client-facing daemon metadata and event models.

use luminate_core::device::DeviceId;
use luminate_core::target::TargetId;
use luminate_core::transition::TransitionId;
use luminate_protocol::ManagementChangeSet;
use serde::{Deserialize, Serialize};

/// Stable consumer-facing daemon metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Server implementation name, normally `luminated`.
    pub daemon_name: String,

    /// Server package version.
    pub daemon_version: String,

    /// Stable consumer protocol ABI spoken on the primary socket.
    pub protocol_abi_version: u32,
}

impl From<luminate_protocol::ServerInfo> for ServerInfo {
    fn from(value: luminate_protocol::ServerInfo) -> Self {
        Self {
            daemon_name: value.daemon_name,
            daemon_version: value.daemon_version,
            protocol_abi_version: value.protocol_abi_version,
        }
    }
}

/// A stable consumer-facing dirty-bit event. Fetch the authoritative data
/// after receiving it; an empty device list requests a full refresh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    /// Events were lost; fetch every authoritative baseline again before
    /// trusting subsequent incremental invalidations.
    ResyncRequired,

    /// One or more devices may have changed topology.
    ///
    /// An empty list invalidates the complete topology. Non-empty lists identify
    /// dirty devices, but the event is still only a hint: fetch authoritative
    /// device data after receiving it.
    TopologyChanged {
        /// Dirty device identifiers, or an empty list for a full refresh.
        devices: Vec<DeviceId>,
    },

    /// One or more devices may have changed client-visible state.
    ///
    /// An empty list invalidates all device state. Non-empty lists identify
    /// dirty devices, but the event is still only a hint: fetch authoritative
    /// state after receiving it.
    StateChanged {
        /// Dirty device identifiers, or an empty list for a full refresh.
        devices: Vec<DeviceId>,
    },

    /// Managed configuration was durably committed.
    ///
    /// The change record contains keys and metadata, never setting values.
    /// Fetch the authoritative management snapshot after receiving it.
    ConfigurationChanged {
        /// Redacted changes committed in the transaction.
        changes: ManagementChangeSet,
    },

    /// A client-published shared-memory frame stream
    /// (`begin_shm_frame_stream`) ended on the daemon side, rather than
    /// through this client's own explicit end call. The client should stop
    /// publishing into the segment.
    ShmStreamEnded {
        /// The stream's target.
        target: TargetId,

        /// The generation the ended stream was negotiated under.
        generation: u32,
    },

    /// The persistent scene registry changed.
    ScenesChanged,

    /// One or more daemon-managed transition statuses changed.
    TransitionsChanged {
        /// Dirty transition identifiers, or empty for a full refresh.
        transitions: Vec<TransitionId>,
    },
}

impl From<luminate_protocol::Event> for Event {
    fn from(value: luminate_protocol::Event) -> Self {
        match value {
            luminate_protocol::Event::ResyncRequired => Self::ResyncRequired,
            luminate_protocol::Event::TopologyChanged { devices } => {
                Self::TopologyChanged { devices }
            }
            luminate_protocol::Event::StateChanged { devices } => Self::StateChanged { devices },
            luminate_protocol::Event::ConfigurationChanged { changes } => {
                Self::ConfigurationChanged { changes }
            }
            luminate_protocol::Event::ScenesChanged => Self::ScenesChanged,
            luminate_protocol::Event::TransitionsChanged { transitions } => {
                Self::TransitionsChanged { transitions }
            }
            luminate_protocol::Event::ShmStreamEnded { target, generation } => {
                Self::ShmStreamEnded { target, generation }
            }
        }
    }
}
