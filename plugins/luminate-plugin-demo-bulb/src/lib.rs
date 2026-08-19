// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Reference plugin for a self-persisting smart bulb.
//!
//! Every write is durable, but the stored state cannot be read back. Its
//! capability declaration shows how those two properties are represented.

use luminate_core::control::ReconciliationPolicy;

use std::ffi::CStr;

use luminate_core::capability::{
    BrightnessCapability, BufferingMode, CapabilityScope, CapabilitySet, CctEmulation,
    ColourCapability, FrameUpdateMode, FrameUploadCapability, PersistenceCapability,
    PersistenceRequirement, StateReadbackCapability,
};
use luminate_core::device::{DeviceCategory, device_category};
use luminate_core::effect::Effect;
use luminate_core::frame::{FrameEnvelope, FramePayload};
use luminate_plugin_api::{
    DeviceDescriptor, PluginBus, PluginError, PluginProbeHint, PluginRequestContext, PluginTarget,
    PluginUpdate, PluginUpdateOperation, PluginVendorId, ProbeOutcome, ShadowState,
    luminate_export_plugin,
    sdk::{FrameStreamingPlugin, LuminatePlugin},
};

const NAME: &CStr = c"luminate-plugin-demo-bulb";
const VERSION: &CStr = c"0.1.0";
const DEVICE_ID: &str = "demo-smart-bulb";

static BUSES: &[PluginBus] = &[PluginBus::Platform];
static VENDORS: &[PluginVendorId] = &[];
static HINTS: &[PluginProbeHint] = &[];

struct DemoBulb;

impl LuminatePlugin for DemoBulb {
    fn new() -> Result<Self, PluginError> {
        Ok(Self)
    }

    fn probe(&self) -> ProbeOutcome {
        tracing::info!(
            vendor = "Djinn Foundry",
            product = "Efreet Ember",
            "demo smart bulb probe accepted"
        );
        ProbeOutcome::Ready
    }

    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
        Ok(vec![bulb_device()])
    }

    fn apply(
        &self,
        _context: &PluginRequestContext,
        update: &PluginUpdate,
    ) -> Result<(), PluginError> {
        apply_update(update).map_err(PluginError::Unsupported)
    }
}

impl FrameStreamingPlugin for DemoBulb {
    fn upload_frame(
        &self,
        _context: &PluginRequestContext,
        target: &PluginTarget,
        envelope: &FrameEnvelope,
    ) -> Result<(), PluginError> {
        apply_frame(target, envelope).map_err(PluginError::Unsupported)
    }
}

fn bulb_device() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: DEVICE_ID.to_owned(),
        name: "Demo Smart Bulb".to_owned(),
        vendor: Some("Djinn Foundry".to_owned()),
        model: Some("Efreet Ember".to_owned()),
        surfaces: Vec::new(),
        groups: Vec::new(),
        capabilities: bulb_capabilities(),
        category: Some(DeviceCategory::new(device_category::LED_STRIP)),
        physical_tags: vec!["shape:a19".to_owned()],
        host_attached: false,
        notes: vec!["Persists every change to flash immediately; save is a no-op.".to_owned()],
        warnings: Vec::new(),
    }
}

/// Describes a bulb where every write is saved automatically.
///
/// Saving is implicit, but readback is unavailable, so those two properties are
/// declared separately.
fn bulb_capabilities() -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: u8::MAX.into(),
            scope: CapabilityScope::Device,
        },
        persistence: PersistenceCapability::CurrentState {
            requirement: PersistenceRequirement::Required,
            explicit_commit: false,
            readback: false,
        },
        state_readback: StateReadbackCapability::None,
        hardware_effects: None,
        appearance_slots: None,
        frame_upload: Some(FrameUploadCapability {
            scope: CapabilityScope::Device,
            update_mode: FrameUpdateMode::FullFrameOnly,
            max_rate_hz: None,
            atomic: true,
            buffering: BufferingMode::Immediate,
            shm: None,
        }),
        emission: true,
        off_is_wear_safe: false,
        physical_power: None,
        power_domain: None,
    }
}

fn apply_update(update: &PluginUpdate) -> Result<(), String> {
    ensure_device_target(&update.target)?;

    match &update.operation {
        // Writes are already durable, so an explicit save is a no-op.
        PluginUpdateOperation::SaveCurrent => {
            tracing::info!(
                target = %update.target,
                "demo smart bulb save-current accepted: write-through persistence is already durable"
            );
        }
        PluginUpdateOperation::SetAppearanceSlots { .. } => {
            return Err("demo smart bulb does not advertise appearance slots".to_owned());
        }
        operation @ (PluginUpdateOperation::SetEffect { .. }
        | PluginUpdateOperation::SetBrightness { .. }
        | PluginUpdateOperation::Clear) => {
            record_shadow_state(&update.target, operation);
            tracing::info!(
                target = %update.target,
                operation = %operation.name(),
                "demo smart bulb update applied"
            );
        }
    }
    Ok(())
}

fn apply_frame(target: &PluginTarget, envelope: &FrameEnvelope) -> Result<(), String> {
    ensure_device_target(target)?;
    let FramePayload::Full(pixels) = &envelope.payload else {
        return Err("demo smart bulb frame streaming only supports full frames".to_owned());
    };
    match pixels.as_slice() {
        [colour] => {
            record_shadow_state(
                target,
                &PluginUpdateOperation::SetEffect {
                    effect: Effect::Static {
                        colour: colour.clone(),
                    },
                },
            );
            Ok(())
        }
        other => Err(format!(
            "demo smart bulb frame expects exactly 1 pixel, got {}",
            other.len()
        )),
    }
}

/// Accepts only the bulb's single device-level target.
fn ensure_device_target(target: &PluginTarget) -> Result<(), String> {
    match target {
        PluginTarget::Device { device } if device == DEVICE_ID => Ok(()),
        PluginTarget::Device { device } => Err(format!(
            "target is not owned by the demo bulb plugin: {device}"
        )),
        PluginTarget::Surface { .. }
        | PluginTarget::Element { .. }
        | PluginTarget::Group { .. } => {
            Err("demo smart bulb only accepts device-scoped targets".to_owned())
        }
    }
}

fn record_shadow_state(target: &PluginTarget, operation: &PluginUpdateOperation) {
    shadow_state()
        .lock()
        .expect("lock poisoned")
        .insert(target.to_string(), operation.to_string());
}

fn shadow_state() -> &'static ShadowState<String, String> {
    static SHADOW: ShadowState<String, String> = ShadowState::new();
    &SHADOW
}

// Frame upload is implemented by the plugin. Batch updates use the SDK's
// one-at-a-time fallback, and the unsupported callbacks are omitted.
luminate_export_plugin! {
    plugin: DemoBulb,
    name: NAME,
    version: VERSION,
    priority: 50,
    recommended_reconciliation: Some(ReconciliationPolicy::Leave),
    buses: BUSES,
    vendors: VENDORS,
    probe_hints: HINTS,
    start: none,
    rescan: none,
    batch: default,
    read_state: none,
    frame_upload: native,
    shm_frame: none,
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
