// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Stable Luminate topology derived from authenticated Hue resources.

use luminate_core::capability::{
    BrightnessCapability, CapabilityScope, CapabilitySet, CctEmulation, ColourCapability,
    ReadableFacet, ReadbackFidelity, StateReadbackCapability,
};
use luminate_core::device::DeviceCategory;
use luminate_core::state::StateFacetKind;
use luminate_core::surface::SurfaceKind;
use luminate_plugin_api::{
    ClaimExclusivity, DeviceDescriptor, HardwareBus, HardwareClaim, SurfaceDescriptor,
};

use crate::api::{Device, Gamut, Light, ResourceId, Snapshot};
use crate::configuration::{BridgeId, Endpoint};

const SURFACE_ID: &str = "light";

#[derive(Debug, Clone)]
pub(crate) struct HueDevice {
    pub(crate) id: String,
    pub(crate) bridge_id: BridgeId,
    pub(crate) endpoint: Endpoint,
    pub(crate) device: Device,
    pub(crate) light: Light,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TopologyFingerprint {
    name: String,
    manufacturer: String,
    product_name: String,
    model_id: String,
    owner_device: String,
    dimming: bool,
    colour: bool,
    colour_temperature: Option<(u16, u16)>,
    gamut: Option<[(u64, u64); 3]>,
}

pub(crate) fn registry_entries(
    snapshot: &Snapshot,
    endpoint: &Endpoint,
    bridge_id: &BridgeId,
) -> Vec<(String, HueDevice, TopologyFingerprint)> {
    snapshot
        .owned_lights()
        .into_iter()
        .map(|owned| {
            let id = stable_id(bridge_id, &owned.light.id);
            let device = HueDevice {
                id: id.clone(),
                bridge_id: bridge_id.clone(),
                endpoint: endpoint.clone(),
                device: owned.device.clone(),
                light: owned.light.clone(),
            };
            let fingerprint = fingerprint(&device);
            (id, device, fingerprint)
        })
        .collect()
}

pub(crate) fn descriptor(device: &HueDevice) -> DeviceDescriptor {
    let device_capabilities = capabilities(&device.light, CapabilityScope::Device);
    let surface_capabilities = capabilities(&device.light, CapabilityScope::Surface);
    let resource_suffix = device
        .light
        .id
        .as_str()
        .rsplit('-')
        .next()
        .unwrap_or(device.light.id.as_str());
    let name = format!("{} ({resource_suffix})", device.device.metadata.name);
    let model = format!(
        "{} ({})",
        device.device.product_data.product_name, device.device.product_data.model_id
    );
    let mut surface_notes = Vec::new();
    if let Some(temperature) = &device.light.color_temperature {
        surface_notes.push(format!(
            "Hue colour-temperature range: {}..={} mirek.",
            temperature.mirek_schema.mirek_minimum, temperature.mirek_schema.mirek_maximum
        ));
    }
    if device
        .light
        .color
        .as_ref()
        .and_then(|color| color.gamut.as_ref())
        .is_some()
    {
        surface_notes.push("Hue reports a device-specific CIE xy colour gamut.".to_owned());
    }

    DeviceDescriptor {
        id: device.id.clone(),
        name,
        vendor: Some(device.device.product_data.manufacturer_name.clone()),
        model: Some(model),
        surfaces: vec![SurfaceDescriptor {
            id: SURFACE_ID.to_owned(),
            name: "Light".to_owned(),
            kind: SurfaceKind::Zone,
            physical_tags: Vec::new(),
            elements: Vec::new(),
            capabilities: surface_capabilities,
            notes: surface_notes,
            warnings: Vec::new(),
        }],
        groups: Vec::new(),
        capabilities: device_capabilities,
        claims: vec![HardwareClaim {
            bus: HardwareBus::Network,
            physical_identity: format!(
                "philips-hue:{}/{}",
                device.bridge_id.as_str(),
                device.light.id.as_str()
            ),
            control_domain: "light-output".to_owned(),
            exclusivity: ClaimExclusivity::Exclusive,
        }],
        category: Some(DeviceCategory::new("light-bulb")),
        physical_tags: Vec::new(),
        host_attached: false,
        notes: vec![format!(
            "Hue CLIP v2 light {} on authenticated bridge {}.",
            device.light.id.as_str(),
            device.bridge_id.as_str()
        )],
        warnings: vec![
            "Physical validation currently covers BSB002 discovery and pairing plus LCA007 control and readback; broader Philips Hue support remains experimental."
                .to_owned(),
        ],
    }
}

fn stable_id(bridge_id: &BridgeId, light_id: &ResourceId) -> String {
    format!("philips-hue:{}:{}", bridge_id.as_str(), light_id.as_str())
}

fn capabilities(light: &Light, scope: CapabilityScope) -> CapabilitySet {
    let mut colour = Vec::new();
    if light.has_colour() {
        colour.push(ColourCapability::rgb8());
    }
    if light.has_colour_temperature() {
        colour.push(ColourCapability::cct(16));
    }

    CapabilitySet {
        colour,
        cct_emulation: CctEmulation::Disabled,
        brightness: if light.has_dimming() {
            BrightnessCapability::Independent {
                bits: 7,
                maximum: 100,
                scope,
            }
        } else {
            BrightnessCapability::None
        },
        emission: true,
        off_is_wear_safe: true,
        state_readback: StateReadbackCapability::Readable {
            facets: readable_facets(light),
            read_disturbs_output: false,
            notifies_external_changes: false,
        },
        ..CapabilitySet::default()
    }
}

fn readable_facets(light: &Light) -> Vec<ReadableFacet> {
    let mut facets = Vec::new();
    if light.has_colour() || light.has_colour_temperature() {
        facets.push(ReadableFacet {
            facet: StateFacetKind::Appearance,
            fidelity: ReadbackFidelity::BestEffort,
        });
    }
    if light.has_dimming() {
        facets.push(ReadableFacet {
            facet: StateFacetKind::Brightness,
            fidelity: ReadbackFidelity::BestEffort,
        });
    }
    facets.push(ReadableFacet {
        facet: StateFacetKind::Emission,
        fidelity: ReadbackFidelity::Exact,
    });
    facets
}

fn fingerprint(device: &HueDevice) -> TopologyFingerprint {
    TopologyFingerprint {
        name: device.device.metadata.name.clone(),
        manufacturer: device.device.product_data.manufacturer_name.clone(),
        product_name: device.device.product_data.product_name.clone(),
        model_id: device.device.product_data.model_id.clone(),
        owner_device: device.light.owner.rid.as_str().to_owned(),
        dimming: device.light.has_dimming(),
        colour: device.light.has_colour(),
        colour_temperature: device.light.color_temperature.as_ref().map(|temperature| {
            (
                temperature.mirek_schema.mirek_minimum,
                temperature.mirek_schema.mirek_maximum,
            )
        }),
        gamut: device
            .light
            .color
            .as_ref()
            .and_then(|color| color.gamut.as_ref())
            .map(gamut_fingerprint),
    }
}

fn gamut_fingerprint(gamut: &Gamut) -> [(u64, u64); 3] {
    [
        (gamut.red.x.to_bits(), gamut.red.y.to_bits()),
        (gamut.green.x.to_bits(), gamut.green.y.to_bits()),
        (gamut.blue.x.to_bits(), gamut.blue.y.to_bits()),
    ]
}
