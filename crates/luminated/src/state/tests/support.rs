// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Shared `#[cfg(test)]` fixtures used across more than one `state` submodule's
//! test suite. Fixtures scoped to a single concern's tests stay local to that
//! concern's own test module instead of living here.

#![cfg(test)]

use luminate_core::capability::{
    BrightnessCapability, CapabilityScope, CapabilitySet, CctEmulation, ColourCapability,
    EffectParameter, HardwareEffectDescriptor, HardwareEffectId, HardwareEffectsCapability,
    ReadableFacet, ReadbackFidelity, StateReadbackCapability,
};
use luminate_core::element::ElementKind;
use luminate_core::group::GroupKind;
use luminate_core::state::StateFacetKind;
use luminate_core::surface::SurfaceKind;
use luminate_core::target::TargetId;
use luminate_core::util::DiscreteRange;
use luminate_plugin_api::{
    DeviceDescriptor, ElementDescriptor, GroupDescriptor, GroupMemberDescriptor, SurfaceDescriptor,
};

use super::DaemonState;
use super::target_state::TargetState;

/// The only legitimate consumer of this fixture is test code. The daemon
/// itself must never fabricate topology that didn't come from a loaded
/// plugin.
pub(super) fn demo_device_descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: "demo-kbd".to_owned(),
        name: "Demo Keyboard".to_owned(),
        vendor: Some("Luminate".to_owned()),
        model: Some("Virtual RGB".to_owned()),
        surfaces: vec![SurfaceDescriptor {
            id: "main".to_owned(),
            name: "Main".to_owned(),
            kind: SurfaceKind::Opaque,
            physical_tags: Vec::new(),
            elements: vec![
                ElementDescriptor {
                    id: "logo".to_owned(),
                    name: Some("Logo".to_owned()),
                    kind: ElementKind::Logo,
                    geometry: None,
                    physical_tags: Vec::new(),
                    capabilities: CapabilitySet {
                        colour: vec![ColourCapability::rgb8()],
                        cct_emulation: CctEmulation::Auto,
                        ..CapabilitySet::default()
                    },
                    notes: Vec::new(),
                    warnings: Vec::new(),
                },
                ElementDescriptor {
                    id: "topology-only".to_owned(),
                    name: Some("Topology Only".to_owned()),
                    kind: ElementKind::Led,
                    geometry: None,
                    physical_tags: Vec::new(),
                    capabilities: CapabilitySet::default(),
                    notes: Vec::new(),
                    warnings: Vec::new(),
                },
            ],
            capabilities: CapabilitySet {
                colour: vec![ColourCapability::rgb8()],
                cct_emulation: CctEmulation::Auto,
                state_readback: StateReadbackCapability::Readable {
                    facets: vec![ReadableFacet {
                        facet: StateFacetKind::Appearance,
                        fidelity: ReadbackFidelity::BestEffort,
                    }],
                    read_disturbs_output: false,
                    notifies_external_changes: false,
                },
                ..CapabilitySet::default()
            },
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: vec![GroupDescriptor {
            id: "all".to_owned(),
            name: "All".to_owned(),
            description: Some("All addressable lighting targets".to_owned()),
            kind: GroupKind::Topology,
            members: vec![
                GroupMemberDescriptor::Surface("main".to_owned()),
                GroupMemberDescriptor::Element {
                    surface: "main".to_owned(),
                    element: "logo".to_owned(),
                },
            ],
            capabilities: CapabilitySet {
                colour: vec![ColourCapability::rgb8()],
                cct_emulation: CctEmulation::Auto,
                ..CapabilitySet::default()
            },
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        capabilities: CapabilitySet {
            colour: vec![ColourCapability::rgb8()],
            cct_emulation: CctEmulation::Auto,
            state_readback: StateReadbackCapability::Readable {
                facets: vec![ReadableFacet {
                    facet: StateFacetKind::Appearance,
                    fidelity: ReadbackFidelity::BestEffort,
                }],
                read_disturbs_output: false,
                notifies_external_changes: false,
            },
            ..CapabilitySet::default()
        },
        category: None,
        physical_tags: Vec::new(),
        host_attached: false,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

pub(super) fn demo_state() -> DaemonState {
    DaemonState::from_descriptors(&[demo_device_descriptor()]).expect("demo state should build")
}

pub(super) fn validation_state() -> DaemonState {
    let mut descriptor = demo_device_descriptor();
    descriptor.capabilities = CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 7,
            maximum: 100,
            scope: CapabilityScope::Device,
        },
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![HardwareEffectDescriptor {
                id: HardwareEffectId::new("morph"),
                name: "Morph".to_owned(),
                parameters: vec![
                    EffectParameter::Colour {
                        minimum_colours: 1,
                        maximum_colours: 3,
                    },
                    EffectParameter::Duration {
                        milliseconds: DiscreteRange::new(100, 1_000, 100),
                    },
                ],
            }],
            scope: CapabilityScope::Device,
            concurrent_with_streaming: false,
        }),
        ..CapabilitySet::default()
    };
    descriptor.surfaces[0].capabilities = CapabilitySet {
        colour: vec![ColourCapability::monochrome(1)],
        cct_emulation: CctEmulation::Auto,
        ..CapabilitySet::default()
    };
    DaemonState::from_descriptors(&[descriptor]).expect("validation state should build")
}

pub(super) fn has_target(state: &DaemonState, target: &TargetId) -> bool {
    state
        .target_states()
        .iter()
        .any(|entry| &entry.target == target)
}

pub(super) fn effective_state_for_target<'a>(
    state: &'a DaemonState,
    target: &TargetId,
) -> Option<&'a TargetState> {
    state
        .target_states()
        .iter()
        .filter(|entry| state.target_covers(&entry.target, target))
        .map(|entry| &entry.state)
        .next_back()
}
