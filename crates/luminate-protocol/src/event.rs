// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Server-to-client invalidation event messages.

use serde::{Deserialize, Serialize};

use crate::EVENT_PROTOCOL_VERSION;
use crate::ManagementChangeSet;
use crate::authentication::EventTicket;
use luminate_core::device::DeviceId;
use luminate_core::target::TargetId;
use luminate_core::transition::TransitionId;

/// Subscription request sent when opening an event connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscribeHello {
    /// Event protocol version requested by the client.
    pub event_protocol_version: u32,

    /// Human-readable client name.
    pub client_name: String,

    /// Human-readable client version.
    pub client_version: String,

    /// One-use ticket minted for this subscription by the primary connection.
    pub ticket: EventTicket,
}

impl SubscribeHello {
    /// Constructs a subscription request for the current event protocol.
    #[must_use]
    pub fn new(
        client_name: impl Into<String>,
        client_version: impl Into<String>,
        ticket: EventTicket,
    ) -> Self {
        Self {
            event_protocol_version: EVENT_PROTOCOL_VERSION,
            client_name: client_name.into(),
            client_version: client_version.into(),
            ticket,
        }
    }
}

/// Daemon response to an event subscription request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscribeAck {
    /// Whether the requested event protocol is compatible.
    pub compatibility: EventCompatibility,

    /// Event protocol version reported by the daemon.
    pub event_protocol_version: u32,

    /// Human-readable daemon version.
    pub daemon_version: String,
}

/// Compatibility result for an event protocol handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventCompatibility {
    /// The client and daemon can exchange events.
    Compatible,

    /// The requested event protocol is incompatible.
    Incompatible {
        /// Event protocol version supported by the daemon.
        supported_event_protocol_version: u32,

        /// Optional human-readable explanation.
        reason: Option<String>,
    },
}

/// An invalidation event sent by the daemon.
///
/// Clients fetch authoritative data from the primary request socket after
/// receiving an event. [`Self::ResyncRequired`] means events were lost and all
/// client-maintained baselines must be fetched again before incremental events
/// can be trusted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    /// The subscriber lost events and must fetch fresh authoritative baselines.
    ///
    /// This event contains no resource information and cannot be suppressed by
    /// authorization or voluntary subscription filters.
    ResyncRequired,

    /// The device topology changed.
    TopologyChanged {
        /// Devices known to be affected, or an empty list for a full refresh.
        devices: Vec<DeviceId>,
    },

    /// Authoritative state changed for one or more devices.
    StateChanged {
        /// Devices known to be affected, or an empty list for a full refresh.
        devices: Vec<DeviceId>,
    },

    /// Managed configuration was durably committed.
    ///
    /// Values are deliberately absent; clients fetch the authoritative
    /// snapshot through `Request::GetManagement`.
    ConfigurationChanged {
        /// Redacted description of the committed changes.
        changes: ManagementChangeSet,
    },

    /// The persistent scene registry changed.
    ScenesChanged,

    /// One or more transition statuses changed.
    ///
    /// An empty list invalidates the complete in-memory transition baseline.
    TransitionsChanged {
        /// Transitions known to be affected, or an empty list for a full refresh.
        transitions: Vec<TransitionId>,
    },

    /// The daemon stopped consuming a client-published shared-memory stream
    /// without receiving `Request::EndShmFrameStream`.
    ///
    /// The client should stop publishing because the daemon cannot revoke the
    /// client's mapping directly.
    ShmStreamEnded {
        /// Target whose stream ended.
        target: TargetId,

        /// Generation of the stream that ended.
        generation: u32,
    },
}
