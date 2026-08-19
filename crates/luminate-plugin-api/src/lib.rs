// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! C-compatible types and CBOR payloads shared by `luminated` and its native
//! plugins.
//!
//! This ABI is intentionally unstable and kept in lockstep with the daemon,
//! similar to a Linux kernel module ABI rather than a stable third-party
//! plugin contract. See [`PLUGIN_ABI_VERSION`]. This module documents each
//! field.

#![allow(
    unsafe_code,
    reason = "The plugin ABI exposes C-compatible entry points and raw pointer types."
)]

/// Exact ABI version for daemon-loaded plugin shared objects.
///
/// Increment this for incompatible `PluginDescriptor` layout or callback
/// changes and for incompatible CBOR payload changes crossing the plugin ABI.
///
/// ABI 13 adds ordered `physical_tags` to `ElementDescriptor` topology CBOR.
pub const PLUGIN_ABI_VERSION: u32 = 13;

mod abi;
mod apply;
mod codec;
pub mod configuration;
mod deadline;
mod descriptor;
mod dynamic_registry;
pub mod logging;
pub mod notification;
pub mod sdk;
mod settings;
mod setup;
mod shadow_state;
mod shm;
mod topology;

pub use abi::{PluginBus, PluginLogLevel, ProbeHintKind, ProbeOutcome, RescanReason, abi_bool};
pub use apply::{
    PLUGIN_APPLY_DIAGNOSTIC_CAPACITY, PLUGIN_APPLY_DIAGNOSTIC_TRUNCATED, PluginApplyResult,
    PluginApplyStatus, PluginError, dispatch_frame_upload_cbor, dispatch_update_batch_cbor,
    dispatch_update_cbor, write_apply_result,
};
pub use codec::{decode_cbor, encode_cbor, sanitize_cstring};
pub use deadline::{
    PluginDeadline, PluginRequestContext, current_request_deadline, with_request_context,
};
pub use descriptor::{
    ABI_VERSION_SYMBOL_NAME, PLUGIN_DESCRIPTOR_SYMBOL_NAME, PLUGIN_STATE_CBOR_CAPACITY,
    PluginAbiVersion, PluginApplyBatchFn, PluginApplyUpdateFn, PluginDescriptor,
    PluginFrameUploadFn, PluginInitFn, PluginLogFn, PluginNotifyFn, PluginProbeFn, PluginProbeHint,
    PluginReadStateFn, PluginRescanFn, PluginSetupFn, PluginShmFrameApplyFn,
    PluginShmStreamBeginFn, PluginShmStreamEndFn, PluginStartFn, PluginTopologyCborFn,
    PluginVendorId, reconciliation_policy_from_abi, reconciliation_policy_to_abi,
};
pub use settings::{PluginSettingApplyMode, PluginSettingDescriptor, PluginSettingKind};
pub use setup::{
    PLUGIN_SETUP_CBOR_CAPACITY, PluginSetupChoice, PluginSetupInteraction, PluginSetupRequest,
    PluginSetupResponse, PluginSetupSettingValue, PluginSetupStep, PluginSetupWorkflowDescriptor,
    PluginSetupWorkflowKind,
};
pub use shadow_state::ShadowState;
pub use shm::{dispatch_shm_frame, dispatch_shm_stream_begin};
pub use topology::{
    ClaimExclusivity, DeviceDescriptor, ElementDescriptor, GroupDescriptor, GroupMemberDescriptor,
    HardwareBus, HardwareClaim, PluginFacetObservation, PluginFrameUpload, PluginReadError,
    PluginReadRequest, PluginReadTarget, PluginStateSnapshot, PluginTarget, PluginUpdate,
    PluginUpdateBatch, PluginUpdateOperation, SurfaceDescriptor, topology_cbor,
    write_state_snapshot,
};
