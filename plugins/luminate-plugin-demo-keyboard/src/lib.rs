// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Reference plugin for a keyboard-shaped topology.
//!
//! It demonstrates an authored key layout and the same controls at device,
//! surface, group, and element scopes.

#[cfg(test)]
use luminate_core::colour::Colour;
use luminate_core::control::ReconciliationPolicy;

use std::ffi::CStr;

use luminate_core::capability::{
    BrightnessCapability, CapabilityScope, CapabilitySet, CctEmulation, ColourCapability,
    HardwareEffectDescriptor, HardwareEffectId, HardwareEffectsCapability, PersistenceCapability,
    StateReadbackCapability,
};
use luminate_core::device::{DeviceCategory, device_category};
use luminate_core::element::{ElementGeometry, ElementKind};
use luminate_core::group::GroupKind;
use luminate_core::surface::SurfaceKind;
use luminate_plugin_api::{
    DeviceDescriptor, ElementDescriptor, GroupDescriptor, GroupMemberDescriptor, PluginBus,
    PluginError, PluginProbeHint, PluginRequestContext, PluginTarget, PluginUpdate,
    PluginUpdateOperation, PluginVendorId, ProbeOutcome, ShadowState, SurfaceDescriptor,
    luminate_export_plugin,
    sdk::{BatchPlugin, LuminatePlugin},
};

const NAME: &CStr = c"luminate-plugin-demo-keyboard";
const VERSION: &CStr = c"0.1.0";

// Keep this distinct from the keyboard in demo-system so both can be loaded.
const DEVICE_ID: &str = "demo-keyboard-only";
const SURFACE_ID: &str = "keys";

static BUSES: &[PluginBus] = &[PluginBus::Platform];
static VENDORS: &[PluginVendorId] = &[];
static HINTS: &[PluginProbeHint] = &[];

// Coordinates are normalized surface geometry. Stable IDs, not table positions,
// identify keys in targets and group membership.
const KEYS: &[KeySpec] = &[
    KeySpec::new("escape", "Esc", 0.00, 0.00),
    KeySpec::new("f1", "F1", 0.10, 0.00),
    KeySpec::new("f2", "F2", 0.16, 0.00),
    KeySpec::new("f3", "F3", 0.22, 0.00),
    KeySpec::new("f4", "F4", 0.28, 0.00),
    KeySpec::new("1", "1", 0.05, 0.20),
    KeySpec::new("2", "2", 0.11, 0.20),
    KeySpec::new("3", "3", 0.17, 0.20),
    KeySpec::new("4", "4", 0.23, 0.20),
    KeySpec::new("q", "Q", 0.08, 0.40),
    KeySpec::new("w", "W", 0.14, 0.40),
    KeySpec::new("e", "E", 0.20, 0.40),
    KeySpec::new("r", "R", 0.26, 0.40),
    KeySpec::new("a", "A", 0.09, 0.60),
    KeySpec::new("s", "S", 0.15, 0.60),
    KeySpec::new("d", "D", 0.21, 0.60),
    KeySpec::new("f", "F", 0.27, 0.60),
    KeySpec::new("z", "Z", 0.12, 0.80),
    KeySpec::new("x", "X", 0.18, 0.80),
    KeySpec::new("c", "C", 0.24, 0.80),
    KeySpec::new("space", "Space", 0.40, 0.80),
    KeySpec::new("enter", "Enter", 0.52, 0.60),
    KeySpec::new("left", "Left", 0.70, 0.80),
    KeySpec::new("down", "Down", 0.76, 0.80),
    KeySpec::new("right", "Right", 0.82, 0.80),
    KeySpec::new("up", "Up", 0.76, 0.60),
];

#[derive(Debug, Clone, Copy)]
struct KeySpec {
    id: &'static str,
    name: &'static str,
    x: f32,
    y: f32,
}

impl KeySpec {
    const fn new(id: &'static str, name: &'static str, x: f32, y: f32) -> Self {
        Self { id, name, x, y }
    }
}

struct DemoKeyboard;

impl LuminatePlugin for DemoKeyboard {
    fn new() -> Result<Self, PluginError> {
        Ok(Self)
    }

    fn probe(&self) -> ProbeOutcome {
        tracing::info!("demo keyboard probe accepted");
        ProbeOutcome::Ready
    }

    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
        Ok(vec![keyboard_device()])
    }

    fn apply(
        &self,
        _context: &PluginRequestContext,
        update: &PluginUpdate,
    ) -> Result<(), PluginError> {
        apply_update(update).map_err(PluginError::Unsupported)
    }
}

impl BatchPlugin for DemoKeyboard {
    fn apply_batch(
        &self,
        _context: &PluginRequestContext,
        updates: &[PluginUpdate],
    ) -> Vec<Result<(), PluginError>> {
        updates
            .iter()
            .map(|update| apply_update(update).map_err(PluginError::Unsupported))
            .collect()
    }
}

fn keyboard_device() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: DEVICE_ID.to_owned(),
        name: "Standalone Demo Keyboard".to_owned(),
        vendor: Some("Asgard Labs".to_owned()),
        model: Some("Týr Virtual 75%".to_owned()),
        surfaces: vec![SurfaceDescriptor {
            id: SURFACE_ID.to_owned(),
            name: "Keys".to_owned(),
            kind: SurfaceKind::Opaque,
            physical_tags: Vec::new(),
            elements: KEYS.iter().map(key_element).collect(),
            capabilities: keyboard_capabilities(CapabilityScope::Surface),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: keyboard_groups(),
        capabilities: keyboard_capabilities(CapabilityScope::Device),
        category: Some(DeviceCategory::new(device_category::KEYBOARD)),
        physical_tags: Vec::new(),
        host_attached: false,
        notes: vec!["Virtual keyboard plugin for integration and UI testing.".to_owned()],
        warnings: Vec::new(),
    }
}

fn key_element(key: &KeySpec) -> ElementDescriptor {
    ElementDescriptor {
        id: key.id.to_owned(),
        name: Some(key.name.to_owned()),
        kind: ElementKind::Key,
        geometry: Some(ElementGeometry::Rect {
            x: key.x,
            y: key.y,
            w: 0.055,
            h: 0.14,
        }),
        physical_tags: Vec::new(),
        capabilities: keyboard_capabilities(CapabilityScope::Element),
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// Builds named shortcuts such as WASD and the arrow keys.
///
/// Groups refer to the stable key IDs above rather than copying key descriptors.
fn keyboard_groups() -> Vec<GroupDescriptor> {
    vec![
        group(
            "all",
            "All Keys",
            "All exposed keyboard keys",
            vec![GroupMemberDescriptor::Surface(SURFACE_ID.to_owned())],
        ),
        group(
            "wasd",
            "WASD",
            "Common movement cluster",
            key_members(&["w", "a", "s", "d"]),
        ),
        group(
            "arrows",
            "Arrows",
            "Arrow-key cluster",
            key_members(&["up", "left", "down", "right"]),
        ),
        group(
            "function-row",
            "Function Row",
            "Escape and function keys",
            key_members(&["escape", "f1", "f2", "f3", "f4"]),
        ),
    ]
}

fn group(
    id: &str,
    name: &str,
    description: &str,
    members: Vec<GroupMemberDescriptor>,
) -> GroupDescriptor {
    GroupDescriptor {
        id: id.to_owned(),
        name: name.to_owned(),
        description: Some(description.to_owned()),
        kind: GroupKind::Topology,
        members,
        capabilities: keyboard_capabilities(CapabilityScope::Device),
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

fn key_members(keys: &[&str]) -> Vec<GroupMemberDescriptor> {
    keys.iter()
        .map(|key| GroupMemberDescriptor::Element {
            surface: SURFACE_ID.to_owned(),
            element: (*key).to_owned(),
        })
        .collect()
}

fn keyboard_capabilities(scope: CapabilityScope) -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: 100,
            scope,
        },
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![
                HardwareEffectDescriptor {
                    id: HardwareEffectId::new("static"),
                    name: "Static".to_owned(),
                    parameters: Vec::new(),
                },
                HardwareEffectDescriptor {
                    id: HardwareEffectId::new("rainbow"),
                    name: "Rainbow".to_owned(),
                    parameters: Vec::new(),
                },
            ],
            scope,
            concurrent_with_streaming: false,
        }),
        appearance_slots: None,
        persistence: PersistenceCapability::None,
        state_readback: StateReadbackCapability::None,
        frame_upload: None,
        emission: true,
        off_is_wear_safe: false,
        physical_power: None,
        power_domain: None,
    }
}

/// Records the last update sent to each target.
///
/// This small shadow makes the demo observable in tests. Real hardware would
/// also expand a group update consistently across its member keys or zones.
fn apply_update(update: &PluginUpdate) -> Result<(), String> {
    ensure_owned_target(&update.target)?;
    ensure_operation_supported(&update.target, &update.operation)?;
    record_shadow_state(&update.target, &update.operation);
    tracing::info!(
        target = %update.target,
        operation = %update.operation.name(),
        "demo keyboard update applied"
    );
    Ok(())
}

fn ensure_owned_target(target: &PluginTarget) -> Result<(), String> {
    let device = match target {
        PluginTarget::Device { device }
        | PluginTarget::Surface { device, .. }
        | PluginTarget::Element { device, .. }
        | PluginTarget::Group { device, .. } => device,
    };
    if device == DEVICE_ID {
        Ok(())
    } else {
        Err("target is not owned by the demo keyboard plugin".to_owned())
    }
}

fn ensure_operation_supported(
    target: &PluginTarget,
    operation: &PluginUpdateOperation,
) -> Result<(), String> {
    if matches!(operation, PluginUpdateOperation::SaveCurrent) {
        return Err("demo keyboard does not implement firmware save-current".to_owned());
    }
    if let PluginTarget::Surface { surface, .. } | PluginTarget::Element { surface, .. } = target
        && surface != SURFACE_ID
    {
        return Err(format!("unknown demo keyboard surface target: {surface}"));
    }
    if let PluginTarget::Element { element, .. } = target
        && !KEYS.iter().any(|key| key.id == element)
    {
        return Err(format!("unknown demo keyboard key: {element}"));
    }
    Ok(())
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

// Native batching gives a hardware plugin the chance to coalesce writes. The
// remaining callbacks are absent because this keyboard does not advertise them.
luminate_export_plugin! {
    plugin: DemoKeyboard,
    name: NAME,
    version: VERSION,
    priority: 50,
    recommended_reconciliation: Some(ReconciliationPolicy::Restore),
    buses: BUSES,
    vendors: VENDORS,
    probe_hints: HINTS,
    start: none,
    rescan: none,
    batch: native,
    read_state: none,
    frame_upload: none,
    shm_frame: none,
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
