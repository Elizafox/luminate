// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Luminate topology and capabilities advertised for Alienware hardware.

use luminate_core::appearance_slot::{
    AppearanceCapability, AppearanceSlotDescriptor, AppearanceSlotId, AppearanceSlotUpdatePolicy,
    AppearanceSlotsCapability,
};
use luminate_core::capability::{
    BrightnessCapability, BufferingMode, CapabilityScope, CapabilitySet, CctEmulation,
    ColourCapability, EffectParameter, FrameUpdateMode, FrameUploadCapability,
    HardwareEffectDescriptor, HardwareEffectId, HardwareEffectsCapability, PersistenceCapability,
    PersistenceRequirement, StateReadbackCapability,
};
use luminate_core::device::{DeviceCategory, device_category};
use luminate_core::element::ElementKind;
use luminate_core::group::GroupKind;
use luminate_core::surface::SurfaceKind;
use luminate_core::util::DiscreteRange;
use luminate_plugin_api::{
    ClaimExclusivity, DeviceDescriptor, ElementDescriptor, GroupDescriptor, GroupMemberDescriptor,
    HardwareBus, HardwareClaim, SurfaceDescriptor,
};

use crate::aw_elc_profile::{AwElcIdentity, M16_R2};
use crate::keyboard_layout::KeyboardLayoutId;

pub const KEYBOARD_DEVICE_ID: &str = "alienware-keyboard";
pub const AW_ELC_DEVICE_ID: &str = "alienware-aw-elc";

pub const AW_ELC_SURFACE_TRACKPAD_RING: &str = "trackpad-ring";
pub const AW_ELC_SURFACE_REAR_LOGO: &str = "rear-logo";
pub const AW_ELC_SURFACE_POWER_BUTTON: &str = "power-button";
pub const AW_ELC_SLOT_AC: &str = "ac";
pub const AW_ELC_SLOT_BATTERY: &str = "battery";

pub const AW_KEYBOARD_BREATHE_EFFECT_ID: &str = "aw-keyboard-breathe";
pub const AW_KEYBOARD_PULSE_EFFECT_ID: &str = "aw-keyboard-pulse";
pub const AW_KEYBOARD_SPECTRUM_EFFECT_ID: &str = "aw-keyboard-spectrum";
pub const AW_KEYBOARD_RAINBOW_EFFECT_ID: &str = "aw-keyboard-rainbow";
pub const AW_ELC_BREATHE_EFFECT_ID: &str = "aw-elc-breathe";
pub const AW_ELC_PULSE_EFFECT_ID: &str = "aw-elc-pulse";
pub const AW_ELC_SPECTRUM_EFFECT_ID: &str = "aw-elc-spectrum";
pub const AW_ELC_RAINBOW_EFFECT_ID: &str = "aw-elc-rainbow";

pub fn keyboard_device(layout: Option<KeyboardLayoutId>) -> DeviceDescriptor {
    let model = keyboard_model(layout);
    let physical_tags = keyboard_physical_tags(layout);

    DeviceDescriptor {
        id: KEYBOARD_DEVICE_ID.to_owned(),
        name: "Alienware Internal Keyboard".to_owned(),
        vendor: Some("Alienware / Darfon".to_owned()),
        model: Some(model),
        surfaces: vec![SurfaceDescriptor {
            id: "keyboard".to_owned(),
            name: "Keyboard".to_owned(),
            kind: SurfaceKind::Opaque,
            physical_tags: Vec::new(),
            elements: keyboard_key_elements(layout),
            capabilities: keyboard_capabilities(
                CapabilityScope::Surface,
                layout.is_some_and(|layout| layout.key_map().is_some()),
            ),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: vec![GroupDescriptor {
            id: "all".to_owned(),
            name: "All Keys".to_owned(),
            description: Some("Whole-keyboard AlienFX effect target".to_owned()),
            kind: GroupKind::Topology,
            members: vec![GroupMemberDescriptor::Surface("keyboard".to_owned())],
            capabilities: keyboard_capabilities(CapabilityScope::Device, false),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        capabilities: keyboard_capabilities(CapabilityScope::Device, false),
        claims: vec![HardwareClaim {
            bus: HardwareBus::Hid,
            physical_identity: "0d62:d2b1".to_owned(),
            control_domain: "keyboard-lighting".to_owned(),
            exclusivity: ClaimExclusivity::Exclusive,
        }],
        category: Some(DeviceCategory::new(device_category::KEYBOARD)),
        physical_tags,
        host_attached: true,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

fn keyboard_physical_tags(layout: Option<KeyboardLayoutId>) -> Vec<String> {
    let mut tags = vec!["shape:keyboard".to_owned()];
    if let Some(layout) = layout {
        tags.extend(layout.physical_tags().iter().map(|tag| (*tag).to_owned()));
    }
    tags.push("form:laptop".to_owned());
    tags
}

/// Per-key `ElementDescriptor`s for the `"keyboard"` surface, one per named
/// physical key in the layout's key map (§8 of the protocol spec). Goes
/// entirely through `KeyboardLayoutId::key_map()`, so it stays correct for
/// any future layout without change here. An unidentified layout (or one
/// with no captured name table) gets zero per-key elements, leaving
/// the surface whole-keyboard-only exactly as before.
fn keyboard_key_elements(layout: Option<KeyboardLayoutId>) -> Vec<ElementDescriptor> {
    let Some(key_map) = layout.and_then(KeyboardLayoutId::key_map) else {
        return Vec::new();
    };

    key_map
        .named_positions()
        .map(|(name, _index)| ElementDescriptor {
            id: name.to_owned(),
            name: None,
            kind: ElementKind::Key,
            geometry: None,
            physical_tags: Vec::new(),
            capabilities: CapabilitySet {
                colour: vec![ColourCapability::rgb8()],
                cct_emulation: CctEmulation::Auto,
                ..CapabilitySet::default()
            },
            notes: Vec::new(),
            warnings: Vec::new(),
        })
        .collect()
}

fn keyboard_model(layout: Option<KeyboardLayoutId>) -> String {
    layout.map_or_else(
        || "0D62:D2B1 RGB Keyboard".to_owned(),
        |layout| {
            let layout_name = layout.name();
            let key_count = layout
                .key_map()
                .map(|map| {
                    debug_assert_eq!(
                        map.presence.len(),
                        map.position_indices().count(),
                        "keyboard presence bitmap and position map must stay aligned"
                    );
                    map.position_indices().count()
                })
                .map_or_else(
                    || "unknown key map".to_owned(),
                    |count| format!("{count} keys"),
                );
            format!("0D62:D2B1 RGB Keyboard ({layout_name}, {key_count})")
        },
    )
}

pub fn aw_elc_device(identity: AwElcIdentity) -> DeviceDescriptor {
    if identity.profile == &M16_R2 {
        return m16_r2_aw_elc_device();
    }

    imported_aw_elc_device(identity)
}

fn m16_r2_aw_elc_device() -> DeviceDescriptor {
    DeviceDescriptor {
        id: AW_ELC_DEVICE_ID.to_owned(),
        name: "Alienware AW-ELC Lighting Controller".to_owned(),
        vendor: Some("Dell / Alienware".to_owned()),
        model: Some("187C:0551 AW-ELC".to_owned()),
        surfaces: vec![
            aw_elc_surface(
                AW_ELC_SURFACE_TRACKPAD_RING,
                "Trackpad Ring",
                ElementKind::RingSegment,
                &["shape:ring", "form:trackpad-surround"],
            ),
            aw_elc_surface(
                AW_ELC_SURFACE_REAR_LOGO,
                "Rear Alien Head",
                ElementKind::Logo,
                &[
                    "shape:logo",
                    "form:laptop-lid",
                    "alienware:shape/alien-head",
                ],
            ),
            aw_elc_surface(
                AW_ELC_SURFACE_POWER_BUTTON,
                "Power Button Alien Head",
                ElementKind::Logo,
                &[
                    "shape:logo",
                    "form:power-button",
                    "alienware:shape/alien-head",
                ],
            ),
        ],
        groups: vec![
            GroupDescriptor {
                id: "live-zones".to_owned(),
                name: "Live Zones".to_owned(),
                description: Some(
                    "Zones controlled by the AW-ELC live animation envelope".to_owned(),
                ),
                kind: GroupKind::Topology,
                members: vec![
                    GroupMemberDescriptor::Surface(AW_ELC_SURFACE_TRACKPAD_RING.to_owned()),
                    GroupMemberDescriptor::Surface(AW_ELC_SURFACE_REAR_LOGO.to_owned()),
                ],
                capabilities: aw_elc_live_capabilities(CapabilityScope::Device),
                notes: Vec::new(),
                warnings: Vec::new(),
            },
            GroupDescriptor {
                id: "all".to_owned(),
                name: "All AW-ELC Zones".to_owned(),
                description: Some(
                    "Topology-only grouping for the trackpad ring, rear logo, and power button; \
                     target live-zones for transient effects or power-button for persistent slots"
                        .to_owned(),
                ),
                kind: GroupKind::Topology,
                members: vec![
                    GroupMemberDescriptor::Group("live-zones".to_owned()),
                    GroupMemberDescriptor::Surface(AW_ELC_SURFACE_POWER_BUTTON.to_owned()),
                ],
                capabilities: CapabilitySet::default(),
                notes: Vec::new(),
                warnings: Vec::new(),
            },
        ],
        capabilities: CapabilitySet::default(),
        claims: vec![HardwareClaim {
            bus: HardwareBus::Hid,
            physical_identity: "187c:0551".to_owned(),
            control_domain: "lighting-controller".to_owned(),
            exclusivity: ClaimExclusivity::Exclusive,
        }],
        category: Some(DeviceCategory::new("led-controller")),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

fn imported_aw_elc_device(identity: AwElcIdentity) -> DeviceDescriptor {
    let profile = identity.profile;
    let warning = format!(
        "The {} AW-ELC profile is corroborated by OpenRGB but has not been hardware-validated in Luminate.",
        profile.model
    );
    let note = "Zone identities are derived from OpenRGB revision \
                5e6d627f519a791487e766aeee51eed7bf7129ef."
        .to_owned();

    DeviceDescriptor {
        id: AW_ELC_DEVICE_ID.to_owned(),
        name: format!("{} AW-ELC Lighting Controller", profile.model),
        vendor: Some("Dell / Alienware".to_owned()),
        model: Some(format!(
            "{:04X}:{:04X} AW-ELC ({})",
            identity.vendor_id, identity.product_id, profile.model
        )),
        surfaces: profile
            .zones
            .iter()
            .map(|zone| SurfaceDescriptor {
                id: zone.surface_id.to_owned(),
                name: zone.name.to_owned(),
                kind: SurfaceKind::Opaque,
                physical_tags: vec![
                    format!("form:{}", zone.form),
                    format!("shape:{}", zone.shape),
                ],
                elements: Vec::new(),
                capabilities: imported_aw_elc_live_capabilities(CapabilityScope::Surface),
                notes: Vec::new(),
                warnings: Vec::new(),
            })
            .collect(),
        groups: vec![GroupDescriptor {
            id: "all".to_owned(),
            name: "All AW-ELC Zones".to_owned(),
            description: Some(
                "Every live lighting zone declared by the resolved profile".to_owned(),
            ),
            kind: GroupKind::Topology,
            members: profile
                .zones
                .iter()
                .map(|zone| GroupMemberDescriptor::Surface(zone.surface_id.to_owned()))
                .collect(),
            capabilities: imported_aw_elc_live_capabilities(CapabilityScope::Device),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        capabilities: CapabilitySet::default(),
        claims: vec![HardwareClaim {
            bus: HardwareBus::Hid,
            physical_identity: format!("{:04x}:{:04x}", identity.vendor_id, identity.product_id),
            control_domain: "lighting-controller".to_owned(),
            exclusivity: ClaimExclusivity::Exclusive,
        }],
        category: Some(DeviceCategory::new("led-controller")),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: vec![note],
        warnings: vec![warning],
    }
}

fn aw_elc_surface(
    id: &str,
    name: &str,
    kind: ElementKind,
    physical_tags: &[&str],
) -> SurfaceDescriptor {
    SurfaceDescriptor {
        id: id.to_owned(),
        name: name.to_owned(),
        kind: SurfaceKind::Zone,
        physical_tags: physical_tags.iter().map(|tag| (*tag).to_owned()).collect(),
        elements: vec![ElementDescriptor {
            id: "led".to_owned(),
            name: Some(name.to_owned()),
            kind,
            geometry: None,
            physical_tags: Vec::new(),
            capabilities: if id == AW_ELC_SURFACE_POWER_BUTTON {
                CapabilitySet::default()
            } else {
                aw_elc_live_capabilities(CapabilityScope::Element)
            },
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        capabilities: if id == AW_ELC_SURFACE_POWER_BUTTON {
            aw_elc_power_button_capabilities()
        } else {
            aw_elc_live_capabilities(CapabilityScope::Surface)
        },
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

fn keyboard_capabilities(scope: CapabilityScope, frame_upload: bool) -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 7,
            maximum: 100,
            scope,
        },
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![
                fixed_colour_effect(AW_KEYBOARD_BREATHE_EFFECT_ID, "Breathe"),
                fixed_colour_effect(AW_KEYBOARD_PULSE_EFFECT_ID, "Pulse"),
                fixed_effect(AW_KEYBOARD_RAINBOW_EFFECT_ID, "Rainbow Wave"),
                sweeper_effect(),
                fixed_effect(AW_KEYBOARD_SPECTRUM_EFFECT_ID, "Spectrum"),
            ],
            scope,
            concurrent_with_streaming: false,
        }),
        // Optional, not Required: the keyboard renders every ordinary write
        // immediately without persisting it. `SaveCurrent` (protocol spec §7,
        // `cc:84:03:00`) is a separate, explicit commit of whatever effect is
        // currently active to non-volatile firmware storage, confirmed
        // experimentally to survive a full power-off/reboot cycle.
        persistence: PersistenceCapability::CurrentState {
            requirement: PersistenceRequirement::Optional,
            explicit_commit: true,
            readback: false,
        },
        // Streamed frames are volatile `cc:8c` custom-colour writes. They do
        // not use the `cc:84` command that persists an effect to firmware.
        // One full frame can span several HID reports, so it is observable as
        // an immediate, non-atomic update on the recognized keyboard surface.
        frame_upload: frame_upload.then_some(FrameUploadCapability {
            scope: CapabilityScope::Surface,
            update_mode: FrameUpdateMode::FullFrameOnly,
            max_rate_hz: Some(15),
            atomic: false,
            buffering: BufferingMode::Immediate,
            shm: None,
        }),
        emission: true,
        ..CapabilitySet::default()
    }
}

fn aw_elc_power_button_capabilities() -> CapabilitySet {
    let slot = |id: &str, name: &str| AppearanceSlotDescriptor {
        id: AppearanceSlotId::new(id),
        name: name.to_owned(),
        appearance: AppearanceCapability {
            colour: vec![ColourCapability::rgb8()],
            cct_emulation: CctEmulation::Disabled,
            hardware_effects: None,
        },
        persistence: PersistenceCapability::CurrentState {
            requirement: PersistenceRequirement::Required,
            explicit_commit: false,
            readback: false,
        },
        notes: Vec::new(),
        warnings: vec!["Updates write through to persistent firmware storage.".to_owned()],
    };

    CapabilitySet {
        appearance_slots: Some(AppearanceSlotsCapability {
            slots: vec![
                slot(AW_ELC_SLOT_AC, "AC"),
                slot(AW_ELC_SLOT_BATTERY, "Battery"),
            ],
            update_policy: AppearanceSlotUpdatePolicy::PartialIfKnown,
        }),
        ..CapabilitySet::default()
    }
}

fn aw_elc_live_capabilities(scope: CapabilityScope) -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::None,
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![
                fixed_colour_effect(AW_ELC_BREATHE_EFFECT_ID, "Breathe"),
                fixed_colour_effect(AW_ELC_PULSE_EFFECT_ID, "Pulse"),
                morph_effect(),
                fixed_effect(AW_ELC_RAINBOW_EFFECT_ID, "Rainbow Wave"),
                fixed_effect(AW_ELC_SPECTRUM_EFFECT_ID, "Spectrum"),
            ],
            scope,
            concurrent_with_streaming: false,
        }),
        // Optional, not Required: live zones render every ordinary write
        // immediately without persisting it. `SaveCurrent` replays whatever
        // was last applied live into the non-volatile saved-animation slot
        // (protocol spec §6.2/§11), confirmed by an independent reference
        // implementation to survive a full reboot and auto-restore at boot.
        persistence: PersistenceCapability::CurrentState {
            requirement: PersistenceRequirement::Optional,
            explicit_commit: true,
            readback: false,
        },
        state_readback: StateReadbackCapability::None,
        ..CapabilitySet::default()
    }
}

fn imported_aw_elc_live_capabilities(scope: CapabilityScope) -> CapabilitySet {
    let mut capabilities = aw_elc_live_capabilities(scope);
    capabilities.persistence = PersistenceCapability::None;
    capabilities
}

/// A firmware program with a fixed cadence that takes one colour. It is a
/// custom hardware effect rather than a typed portable effect because callers
/// cannot control the typed effect's required duration.
fn fixed_colour_effect(id: &str, name: &str) -> HardwareEffectDescriptor {
    HardwareEffectDescriptor {
        id: HardwareEffectId::new(id),
        name: name.to_owned(),
        parameters: vec![EffectParameter::Colour {
            minimum_colours: 1,
            maximum_colours: 1,
        }],
    }
}

/// A firmware program with no caller-controlled parameters.
fn fixed_effect(id: &str, name: &str) -> HardwareEffectDescriptor {
    HardwareEffectDescriptor {
        id: HardwareEffectId::new(id),
        name: name.to_owned(),
        parameters: Vec::new(),
    }
}

/// The keyboard's built-in "knight rider" side-to-side sweep. It has no
/// portable typed equivalent (the generic `Scanner` effect doesn't capture
/// this single-colour, fixed-cadence vendor animation), so it is exposed as a
/// data-driven `Hardware` effect keyed on `aw-sweeper`, taking exactly the one
/// colour the firmware sweeps.
fn sweeper_effect() -> HardwareEffectDescriptor {
    HardwareEffectDescriptor {
        id: HardwareEffectId::new("aw-sweeper"),
        name: "Sweeper".to_owned(),
        parameters: vec![EffectParameter::Colour {
            minimum_colours: 1,
            maximum_colours: 1,
        }],
    }
}

fn morph_effect() -> HardwareEffectDescriptor {
    HardwareEffectDescriptor {
        id: HardwareEffectId::new("morph"),
        name: "Morph".to_owned(),
        parameters: vec![
            EffectParameter::Colour {
                minimum_colours: 1,
                maximum_colours: 16,
            },
            EffectParameter::Duration {
                milliseconds: DiscreteRange::new(1, u16::MAX.into(), 1),
            },
        ],
    }
}

#[cfg(test)]
#[path = "topology_tests.rs"]
mod tests;
