// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! The stable asynchronous Rust client for the Luminate daemon.
//!
//! Start with [`Client::connect`], inspect [`Client::server_info`] and
//! [`Client::list_devices`], then construct targets with [`TargetId`] helpers.
//! Requests on one client may run concurrently, and cancelling one request does
//! not disrupt the connection or other in-flight requests.
//!
//! # First client
//!
//! ```no_run
//! use luminate::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let client = Client::connect().await?;
//!     let devices = client.list_devices().await?;
//!
//!     if let Some(device) = devices.first() {
//!         client
//!             .set_rgb(TargetId::Device(device.id.clone()), 40, 120, 255)
//!             .await?;
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! The high-level colour helpers use the same [`Effect::Static`] operation and
//! capability validation as [`Client::set_effect`]. See the
//! [Rust client guide](https://github.com/Elizafox/luminate/blob/main/docs/development/rust-client.md)
//! for direct targets, selectors, and lower-level effects.
//!
//! See the `inspect` example in this package for a complete topology walk.
#![deny(missing_docs)]

mod all_off;
mod c_abi;
mod client;
mod error;
mod ffi;
mod ffi_typed;
mod model;

// Re-export the complete consumer model modules as well as the common leaf
// types below. Companion adapters can therefore interpret new additive fields
// without reaching through to `luminate-core` or protocol-internal crates.
pub use luminate_core::appearance_slot::{
    AppearanceSlotId, AppearanceSlotUpdatePolicy, AppearanceSlotValue,
};
pub use luminate_core::policy;
pub use luminate_core::{
    appearance_slot, capability, collection, colour, control, device, effect, element, frame,
    group, rgb, scene, state, surface, target, transition,
};

pub use all_off::{AllOffPlan, all_off_plan};
pub use c_abi::LUMINATE_C_ABI_VERSION;
pub use client::{
    AuthenticationAdministration, Client, ClientBuilder, CollectionOutcome, CreatedAttestation,
    CreatedToken, EventSubscription, FrameAck, PolicyAdministration, SceneOutcome, SessionMetadata,
    ShmStreamReady, Transitions,
};
pub use error::{Error, ErrorKind, Result};
pub use luminate_core::capability::{
    BrightnessCapability, CapabilityScope, CapabilitySet, CctEmulation, ColourCapability,
    ColourChannel, ColourChannelCapability, ColourEncoding, EffectDirection, HardwareEffectId,
    PersistenceCapability, StateReadbackCapability,
};
pub use luminate_core::collection::{
    Collection, CollectionCategory, CollectionId, CollectionMember, OwnerIdentity,
};
pub use luminate_core::colour::{Colour, ColourChannelValue, ColourError, Rgb8Error};
pub use luminate_core::control::{ReconciliationPolicy, Selector, UnsupportedPolicy};
pub use luminate_core::device::DeviceCategory;
pub use luminate_core::device::{Device, DeviceId};
pub use luminate_core::effect::{Effect, EffectArguments};
pub use luminate_core::element::{Element, ElementGeometry, ElementId, ElementKind};
pub use luminate_core::frame::{FrameEnvelope, FramePayload};
pub use luminate_core::group::{Group, GroupId, GroupKind, GroupMember};
pub use luminate_core::policy::{ScopeGrant, SessionScope};
pub use luminate_core::rgb::Rgb;
pub use luminate_core::scene::{
    Scene, SceneBinding, SceneCaptureMode, SceneId, SceneTargetState, SceneValidationError,
};
pub use luminate_core::shm_frame::ShmPixelFormat;
pub use luminate_core::state::{
    AdoptionStatus, AggregateAppearanceObservation, AggregateEffectiveAppearanceObservation,
    AppearanceSlotsState, AppearanceState, CollectionStateStatus, DeviceStateStatus,
    EffectiveAppearanceState, EmissionState, FacetObservation, FacetValue, ObservationConfidence,
    ObservationSource, PhysicalPowerState, Reachability, ReconciliationStatus, StateFacetKind,
};
pub use luminate_core::surface::{Surface, SurfaceId, SurfaceKind};
pub use luminate_core::target::TargetId;
pub use luminate_core::transition::{
    HueDirection, TransitionCancellation, TransitionColourInterpolation, TransitionFunction,
    TransitionId, TransitionInterpolationError, TransitionOptions, TransitionOutcome,
    TransitionStatus, TransitionTargetState, TransitionValidationError,
};
pub use luminate_protocol::{AttestationMetadata, TokenMetadata};
pub use luminate_protocol::{Authentication, AuthenticationSource, Credential};
pub use luminate_protocol::{
    DaemonPreferences, DeviceReconciliationPreference, ManagedPlugin, ManagementChange,
    ManagementChangeSet, ManagementMutation, ManagementPatch, ManagementSnapshot,
    PluginRuntimeState, PluginSettingApplyMode, PluginSettingKind, PluginSettingSchema,
    ReportedSettingValue, SettingValue, WriteOnly,
};
pub use luminate_protocol::{
    PluginSetupChoice, PluginSetupInteractionResponse, PluginSetupSession, PluginSetupSessionId,
    PluginSetupSessionState, PluginSetupWorkflow, PluginSetupWorkflowKind,
};
pub use model::{Event, ServerInfo};

/// Common imports for ordinary Rust clients.
///
/// The crate's model modules remain the complete, authoritative API surface.
pub mod prelude {
    pub use crate::{
        BrightnessCapability, CapabilityScope, CapabilitySet, CctEmulation, Client, Collection,
        CollectionCategory, CollectionId, CollectionMember, CollectionOutcome, Colour,
        ColourCapability, ColourChannel, ColourChannelCapability, ColourChannelValue,
        ColourEncoding, ColourError, Device, DeviceCategory, DeviceId, Effect, EffectArguments,
        EffectDirection, Element, ElementId, Error, ErrorKind, Group, GroupId, HardwareEffectId,
        OwnerIdentity, ReconciliationPolicy, Result, Rgb, Scene, SceneBinding, SceneCaptureMode,
        SceneId, SceneOutcome, SceneTargetState, Selector, Surface, SurfaceId, TargetId,
        TransitionId, TransitionOptions, TransitionOutcome, TransitionStatus,
        TransitionTargetState, UnsupportedPolicy,
    };
}

#[must_use]
/// Returns this client library's package version.
#[inline]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
