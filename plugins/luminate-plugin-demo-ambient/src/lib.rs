// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Reference plugin for an ambient light panel with authoritative readback.
//!
//! The panel exposes exact live state and recommends adopting it at startup.

use luminate_core::colour;
use luminate_core::control::ReconciliationPolicy;
use luminate_core::effect::Effect;

use std::ffi::CStr;

use luminate_core::capability::{
    BrightnessCapability, CapabilityScope, CapabilitySet, CctEmulation, ColourCapability,
    PersistenceCapability, PersistenceRequirement, ReadableFacet, ReadbackFidelity,
    StateReadbackCapability,
};
use luminate_core::device::{DeviceCategory, device_category};
use luminate_core::rgb::Rgb;
use luminate_core::state::{AppearanceState, EmissionState, FacetValue, StateFacetKind};
use luminate_core::surface::SurfaceKind;
use luminate_plugin_api::{
    DeviceDescriptor, PluginBus, PluginError, PluginFacetObservation, PluginProbeHint,
    PluginReadError, PluginReadRequest, PluginRequestContext, PluginStateSnapshot, PluginTarget,
    PluginUpdate, PluginUpdateOperation, PluginVendorId, ProbeOutcome, ShadowState,
    SurfaceDescriptor, luminate_export_plugin,
    sdk::{LuminatePlugin, ReadablePlugin},
};

const NAME: &CStr = c"luminate-plugin-demo-ambient";
const VERSION: &CStr = c"0.1.0";
const DEVICE_ID: &str = "demo-ambient-panel";
const SURFACE_ID: &str = "panel";

static BUSES: &[PluginBus] = &[PluginBus::Platform];
static VENDORS: &[PluginVendorId] = &[];
static HINTS: &[PluginProbeHint] = &[];

struct DemoAmbient;

impl LuminatePlugin for DemoAmbient {
    fn new() -> Result<Self, PluginError> {
        Ok(Self)
    }

    fn probe(&self) -> ProbeOutcome {
        tracing::info!(
            vendor = "Avalon Lightworks",
            product = "Nimue Glowpane",
            "demo ambient panel probe accepted"
        );
        ProbeOutcome::Ready
    }

    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
        Ok(vec![ambient_device()])
    }

    fn apply(
        &self,
        _context: &PluginRequestContext,
        update: &PluginUpdate,
    ) -> Result<(), PluginError> {
        apply_update(update).map_err(PluginError::Unsupported)
    }
}

impl ReadablePlugin for DemoAmbient {
    fn read_state(
        &self,
        _context: &PluginRequestContext,
        request: &PluginReadRequest,
    ) -> Result<PluginStateSnapshot, PluginError> {
        Ok(read_snapshot(request))
    }
}

/// Builds a panel with exact surface readback and best-effort device readback.
///
/// Readback fidelity is declared separately for each target and state facet.
fn ambient_device() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: DEVICE_ID.to_owned(),
        name: "Demo Ambient Panel".to_owned(),
        vendor: Some("Avalon Lightworks".to_owned()),
        model: Some("Nimue Glowpane".to_owned()),
        surfaces: vec![SurfaceDescriptor {
            id: SURFACE_ID.to_owned(),
            name: "Panel".to_owned(),
            kind: SurfaceKind::Zone,
            physical_tags: Vec::new(),
            elements: Vec::new(),
            capabilities: panel_surface_capabilities(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: Vec::new(),
        capabilities: ambient_device_capabilities(),
        category: Some(DeviceCategory::new(device_category::LED_STRIP)),
        physical_tags: Vec::new(),
        host_attached: false,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// Describes exact live readback for the physical panel surface.
///
/// The surface maps directly to hardware, so no aggregation or inference is
/// needed.
fn panel_surface_capabilities() -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: u8::MAX.into(),
            scope: CapabilityScope::Surface,
        },
        persistence: PersistenceCapability::CurrentState {
            requirement: PersistenceRequirement::Optional,
            explicit_commit: false,
            readback: true,
        },
        state_readback: StateReadbackCapability::Readable {
            facets: vec![
                ReadableFacet {
                    facet: StateFacetKind::Appearance,
                    fidelity: ReadbackFidelity::Exact,
                },
                ReadableFacet {
                    facet: StateFacetKind::Brightness,
                    fidelity: ReadbackFidelity::Exact,
                },
                ReadableFacet {
                    facet: StateFacetKind::Emission,
                    fidelity: ReadbackFidelity::Exact,
                },
            ],
            read_disturbs_output: false,
            notifies_external_changes: false,
        },
        hardware_effects: None,
        appearance_slots: None,
        frame_upload: None,
        emission: true,
        off_is_wear_safe: false,
        physical_power: None,
        power_domain: None,
    }
}

/// Describes best-effort readback for the whole device.
///
/// Device scope is an aggregate view. Keeping its fidelity lower remains honest
/// if the demo later grows more than one surface.
fn ambient_device_capabilities() -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: u8::MAX.into(),
            scope: CapabilityScope::Device,
        },
        persistence: PersistenceCapability::None,
        state_readback: StateReadbackCapability::Readable {
            facets: vec![
                ReadableFacet {
                    facet: StateFacetKind::Appearance,
                    fidelity: ReadbackFidelity::BestEffort,
                },
                ReadableFacet {
                    facet: StateFacetKind::Brightness,
                    fidelity: ReadbackFidelity::BestEffort,
                },
                ReadableFacet {
                    facet: StateFacetKind::Emission,
                    fidelity: ReadbackFidelity::BestEffort,
                },
            ],
            read_disturbs_output: false,
            notifies_external_changes: false,
        },
        hardware_effects: None,
        appearance_slots: None,
        frame_upload: None,
        emission: true,
        off_is_wear_safe: false,
        physical_power: None,
        power_domain: None,
    }
}

fn apply_update(update: &PluginUpdate) -> Result<(), String> {
    ensure_owned_target(&update.target)?;
    if matches!(&update.operation, PluginUpdateOperation::SaveCurrent) {
        return Err("demo ambient panel does not implement firmware save-current".to_owned());
    }
    record_shadow_state(&update.target, &update.operation);
    tracing::info!(
        target = %update.target,
        operation = %update.operation.name(),
        "demo ambient panel update applied"
    );
    Ok(())
}

fn ensure_owned_target(target: &PluginTarget) -> Result<(), String> {
    match target {
        PluginTarget::Device { device } if device == DEVICE_ID => Ok(()),
        PluginTarget::Surface { device, surface }
            if device == DEVICE_ID && surface == SURFACE_ID =>
        {
            Ok(())
        }
        PluginTarget::Surface { surface, .. } => Err(format!(
            "unknown demo ambient panel surface target: {surface}"
        )),
        PluginTarget::Element { .. } => {
            Err("demo ambient panel has no addressable elements".to_owned())
        }
        PluginTarget::Group { .. } => Err("demo ambient panel has no groups".to_owned()),
        PluginTarget::Device { device } => Err(format!(
            "target is not owned by the demo ambient plugin: {device}"
        )),
    }
}

fn record_shadow_state(target: &PluginTarget, operation: &PluginUpdateOperation) {
    let mut state = shadow_state().lock().expect("lock poisoned");
    let current = state.entry(target.to_string()).or_default();
    match operation {
        PluginUpdateOperation::SetEffect { effect } => {
            // `Off` changes emission without discarding the configured appearance.
            current.powered = !matches!(effect, Effect::Off);
            if current.powered {
                current.appearance = match effect {
                    Effect::Static { colour } => AppearanceState::Static(colour.clone()),
                    Effect::Off
                    | Effect::Breathe { .. }
                    | Effect::Pulse { .. }
                    | Effect::Strobe { .. }
                    | Effect::Scanner { .. }
                    | Effect::Morph { .. }
                    | Effect::Spectrum { .. }
                    | Effect::Rainbow { .. }
                    | Effect::Hardware { .. } => AppearanceState::Effect(effect.clone()),
                };
            }
        }
        PluginUpdateOperation::SetBrightness { value } => {
            current.brightness = *value;
        }
        PluginUpdateOperation::Clear => *current = PanelState::default(),
        PluginUpdateOperation::SetAppearanceSlots { .. } | PluginUpdateOperation::SaveCurrent => {}
    }
}

#[derive(Clone)]
struct PanelState {
    appearance: AppearanceState,
    brightness: u32,
    powered: bool,
}

impl PanelState {
    fn emission(&self) -> EmissionState {
        if self.powered && self.brightness != 0 {
            EmissionState::Emitting
        } else {
            EmissionState::Dark
        }
    }
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            appearance: AppearanceState::Static(colour::Colour::rgb(Rgb::new(255, 255, 255))),
            brightness: 255,
            powered: true,
        }
    }
}

fn read_snapshot(request: &PluginReadRequest) -> PluginStateSnapshot {
    let state = shadow_state().lock().expect("lock poisoned");
    let mut snapshot = PluginStateSnapshot::default();
    for requested in &request.targets {
        if let Err(diagnostic) = ensure_owned_target(&requested.target) {
            snapshot.errors.push(PluginReadError {
                target: requested.target.clone(),
                diagnostic,
            });
            continue;
        }
        let current = state
            .get(&requested.target.to_string())
            .cloned()
            .unwrap_or_default();
        for facet in &requested.facets {
            let value = match facet {
                StateFacetKind::Appearance => FacetValue::Appearance(current.appearance.clone()),
                StateFacetKind::Brightness => FacetValue::Brightness(current.brightness),
                StateFacetKind::Emission => FacetValue::Emission(current.emission()),
                StateFacetKind::PhysicalPower => {
                    snapshot.errors.push(PluginReadError {
                        target: requested.target.clone(),
                        diagnostic: "panel has no independent physical-power facet".to_owned(),
                    });
                    continue;
                }
                StateFacetKind::EffectiveAppearance => {
                    snapshot.errors.push(PluginReadError {
                        target: requested.target.clone(),
                        diagnostic: "effective appearance is daemon-synthesized and cannot be \
                            requested from a plugin"
                            .to_owned(),
                    });
                    continue;
                }
                StateFacetKind::AppearanceSlots => {
                    snapshot.errors.push(PluginReadError {
                        target: requested.target.clone(),
                        diagnostic: "panel has no appearance slots".to_owned(),
                    });
                    continue;
                }
            };
            snapshot.observations.push(PluginFacetObservation {
                target: requested.target.clone(),
                value,
            });
        }
    }
    snapshot
}

fn shadow_state() -> &'static ShadowState<String, PanelState> {
    static SHADOW: ShadowState<String, PanelState> = ShadowState::new();
    &SHADOW
}

// Readback is implemented by the plugin. Batch updates use the SDK fallback,
// and callbacks for unsupported features are left out.
luminate_export_plugin! {
    plugin: DemoAmbient,
    name: NAME,
    version: VERSION,
    priority: 50,
    recommended_reconciliation: Some(ReconciliationPolicy::Adopt),
    buses: BUSES,
    vendors: VENDORS,
    probe_hints: HINTS,
    start: none,
    rescan: none,
    batch: default,
    read_state: native,
    frame_upload: none,
    shm_frame: none,
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
