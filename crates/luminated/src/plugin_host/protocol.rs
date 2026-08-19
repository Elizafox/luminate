// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Messages exchanged between the daemon and an isolated plugin host.

use anyhow::{Context as _, Result};
use luminate_core::capability::ShmFrameShape;
use luminate_core::control;
use luminate_plugin_api::{
    DeviceDescriptor, PluginBus, PluginFrameUpload, PluginReadRequest, PluginStateSnapshot,
    PluginTarget, PluginUpdate, PluginVendorId, ProbeOutcome,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct HostMetadata {
    pub name: String,
    pub version: String,
    pub priority: i32,
    pub recommended_reconciliation: Option<control::ReconciliationPolicy>,
    pub probe_outcome: ProbeOutcome,

    pub buses: Vec<PluginBus>,
    pub vendors: Vec<PluginVendorId>,
    pub probe_hints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireMetadata {
    pub(super) name: String,
    pub(super) version: String,
    pub(super) priority: i32,
    pub(super) recommended_reconciliation: Option<control::ReconciliationPolicy>,
    pub(super) probe_outcome: u8,

    pub(super) buses: Vec<u32>,
    pub(super) vendors: Vec<(u32, u32)>,
    pub(super) probe_hints: Vec<String>,
}

impl TryFrom<WireMetadata> for HostMetadata {
    type Error = anyhow::Error;

    fn try_from(metadata: WireMetadata) -> Result<Self> {
        let buses = metadata
            .buses
            .into_iter()
            .map(|code| {
                PluginBus::from_abi(code)
                    .with_context(|| format!("plugin host returned unknown bus code {code}"))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            name: metadata.name,
            version: metadata.version,
            priority: metadata.priority,
            recommended_reconciliation: metadata.recommended_reconciliation,
            probe_outcome: ProbeOutcome::from_abi(metadata.probe_outcome).with_context(|| {
                format!(
                    "plugin host returned unknown probe outcome {}",
                    metadata.probe_outcome
                )
            })?,
            buses,
            vendors: metadata
                .vendors
                .into_iter()
                .map(|(vendor, product)| PluginVendorId { vendor, product })
                .collect(),
            probe_hints: metadata.probe_hints,
        })
    }
}

impl From<&HostMetadata> for WireMetadata {
    fn from(metadata: &HostMetadata) -> Self {
        Self {
            name: metadata.name.clone(),
            version: metadata.version.clone(),
            priority: metadata.priority,
            recommended_reconciliation: metadata.recommended_reconciliation,
            probe_outcome: metadata.probe_outcome.to_abi(),
            buses: metadata.buses.iter().map(|bus| bus.to_abi()).collect(),
            vendors: metadata
                .vendors
                .iter()
                .map(|vendor| (vendor.vendor, vendor.product))
                .collect(),
            probe_hints: metadata.probe_hints.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct HostReady {
    pub(super) metadata: WireMetadata,
    pub(super) descriptors: Vec<DeviceDescriptor>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct HostBootstrap {
    pub(super) configuration_cbor: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct HostRequest {
    pub(super) id: u64,
    pub(super) timeout_millis: u64,
    pub(super) command: HostCommand,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) enum HostCommand {
    Topology,
    /// Asks the plugin to discard any cached view of its hardware. Always
    /// followed by [`Self::Topology`]; see `PluginDescriptor::rescan`.
    ///
    /// `reason` is a `luminate_plugin_api::RescanReason` ABI byte, the same way
    /// [`WireMetadata::probe_outcome`] carries a `ProbeOutcome`: this hop
    /// forwards what the plugin ABI defines rather than re-encoding it.
    Rescan {
        reason: u8,
    },
    Apply(PluginUpdate),
    ApplyBatch(Vec<PluginUpdate>),
    ReadState(PluginReadRequest),
    UploadFrame(PluginFrameUpload),
    BeginShmStream(BeginShmStreamRequest),
    EndShmStream {
        target: PluginTarget,
        generation: u32,
    },
    ResetShmStream {
        target: PluginTarget,
        generation: u32,
    },
    Shutdown,
}

/// Negotiation request for the opt-in shared-memory frame fast path. Only
/// this one-shot negotiation crosses the ordinary pipe; the frames
/// themselves never do.
#[derive(Debug, Serialize, Deserialize)]
pub struct BeginShmStreamRequest {
    pub target: PluginTarget,
    pub generation: u32,
    /// A [`luminate_core::shm_frame::ShmPixelFormat`] discriminant, already
    /// validated by the daemon against this target's advertised
    /// `ShmFrameCapability` before this request is sent.
    pub pixel_format: u32,
    pub shape: ShmFrameShape,
}

/// Outcome of a [`HostCommand::BeginShmStream`], [`HostCommand::EndShmStream`],
/// or [`HostCommand::ResetShmStream`] request.
#[derive(Debug, Serialize, Deserialize)]
pub enum ShmStreamOutcome {
    /// A [`HostCommand::BeginShmStream`] succeeded; the segment sized to
    /// hold one header plus one full frame is `segment_bytes` long.
    Ready {
        segment_bytes: u32,
    },
    /// A [`HostCommand::EndShmStream`] or [`HostCommand::ResetShmStream`]
    /// was processed. Idempotent: ending or resetting a stream the
    /// plugin-host has no record of is not an error.
    Acknowledged,
    Unsupported(String),
    Io(String),
    Internal(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ApplyOutcome {
    Applied,
    Unsupported(String),
    InvalidArgument(String),
    Io(String),
    Unavailable(String),
    RateLimited {
        diagnostic: String,
        retry_after_ms: Option<u64>,
    },
    Internal(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) enum HostResponse {
    Topology(Vec<DeviceDescriptor>),
    /// A [`HostCommand::Rescan`] was delivered. Carries no outcome: a rescan
    /// only invalidates a cache, and whatever the plugin then discovers
    /// surfaces through the following topology pull.
    Rescan,
    Apply(ApplyOutcome),
    Batch(Vec<ApplyOutcome>),
    State(PluginStateSnapshot),
    Frame(ApplyOutcome),
    ShmStream(ShmStreamOutcome),
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) enum HostMessage {
    Ready(Result<HostReady, String>),
    Response {
        id: u64,
        result: Result<HostResponse, String>,
    },
    TopologyChanged,
    Log {
        plugin: String,
        level: u8,
        message: String,
    },
}
