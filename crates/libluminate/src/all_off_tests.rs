// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use super::*;
use luminate_core::capability::{
    CapabilityScope, ColourCapability, HardwareEffectDescriptor, HardwareEffectId,
    HardwareEffectsCapability,
};
use luminate_core::device::DeviceId;
use luminate_core::element::{Element, ElementId, ElementKind};
use luminate_core::surface::{Surface, SurfaceId, SurfaceKind};

fn capabilities(drivable: bool, required_profiles: bool) -> CapabilitySet {
    CapabilitySet {
        colour: drivable.then(ColourCapability::rgb8).into_iter().collect(),
        persistence: if required_profiles {
            PersistenceCapability::Profiles {
                requirement: PersistenceRequirement::Required,
                slots: 2,
                explicit_commit: false,
                readback: false,
            }
        } else {
            PersistenceCapability::None
        },
        ..CapabilitySet::default()
    }
}

fn device(capabilities: CapabilitySet, surface: Surface) -> Device {
    Device {
        id: DeviceId::new("device"),
        name: "Device".to_owned(),
        vendor: None,
        model: None,
        provider_instance: None,
        surfaces: vec![surface],
        groups: Vec::new(),
        capabilities,
        category: None,
        physical_tags: Vec::new(),
        host_attached: false,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

fn surface(capabilities: CapabilitySet, elements: Vec<Element>) -> Surface {
    Surface {
        id: SurfaceId::new("surface"),
        name: "Surface".to_owned(),
        kind: SurfaceKind::Zone,
        physical_tags: Vec::new(),
        elements,
        capabilities,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

fn element(id: &str, capabilities: CapabilitySet) -> Element {
    Element {
        id: ElementId::new(id),
        name: None,
        kind: ElementKind::Led,
        geometry: None,
        physical_tags: Vec::new(),
        capabilities,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

#[test]
fn device_capability_is_the_maximal_target() {
    let device = device(
        capabilities(true, false),
        surface(capabilities(true, false), Vec::new()),
    );
    assert_eq!(
        all_off_plan(&device).targets(),
        [TargetId::device("device")]
    );
}

#[test]
fn required_profile_target_is_skipped() {
    let device = device(
        CapabilitySet::default(),
        surface(capabilities(true, true), Vec::new()),
    );
    assert_eq!(
        all_off_plan(&device).skipped_persistent(),
        [TargetId::surface("device", "surface")]
    );
}

#[test]
fn required_persistence_target_is_included_when_off_is_wear_safe() {
    let mut capabilities = capabilities(true, true);
    capabilities.off_is_wear_safe = true;
    let device = device(CapabilitySet::default(), surface(capabilities, Vec::new()));

    assert_eq!(
        all_off_plan(&device).targets(),
        [TargetId::surface("device", "surface")]
    );
    assert!(all_off_plan(&device).skipped_persistent().is_empty());
}

#[test]
fn falls_back_to_only_drivable_elements() {
    let device = device(
        CapabilitySet::default(),
        surface(
            CapabilitySet::default(),
            vec![
                element("dark", CapabilitySet::default()),
                element("lit", capabilities(true, false)),
            ],
        ),
    );
    assert_eq!(
        all_off_plan(&device).targets(),
        [TargetId::element("device", "surface", "lit")]
    );
}

#[test]
fn advertised_off_effect_is_drivable() {
    let capabilities = CapabilitySet {
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![HardwareEffectDescriptor {
                id: HardwareEffectId::new("off"),
                name: "Off".to_owned(),
                parameters: Vec::new(),
            }],
            scope: CapabilityScope::Device,
            concurrent_with_streaming: false,
        }),
        ..CapabilitySet::default()
    };
    let device = device(capabilities, surface(CapabilitySet::default(), Vec::new()));
    assert_eq!(
        all_off_plan(&device).targets(),
        [TargetId::device("device")]
    );
}
