// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unnecessary_wraps,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Experimental host-side support for modern Keychron QMK RGB keyboards.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::sync::Mutex;

use luminate_core::capability::{
    CapabilitySet, CctEmulation, ColourCapability, PersistenceCapability, PersistenceRequirement,
};
use luminate_core::control::ReconciliationPolicy;
use luminate_core::device::{DeviceCategory, device_category};
use luminate_core::effect::Effect;
use luminate_core::element::ElementKind;
use luminate_core::rgb::Rgb;
use luminate_core::surface::SurfaceKind;
use luminate_plugin_api::sdk::LuminatePlugin;
use luminate_plugin_api::{
    ClaimExclusivity, DeviceDescriptor, ElementDescriptor, HardwareBus, HardwareClaim, PluginBus,
    PluginError, PluginProbeHint, PluginRequestContext, PluginTarget, PluginUpdate,
    PluginUpdateOperation, PluginVendorId, ProbeOutcome, SurfaceDescriptor, luminate_export_plugin,
};

use protocol::{Discovery, Hsv, discover, save, set_per_key_colours};
use transport::HidTransport;

mod protocol;
mod transport;

const NAME: &CStr = c"luminate-plugin-keychron";
const VERSION: &CStr = c"0.1.0";
const KEYCHRON_VENDOR_ID: u16 = 0x3434;
const QMK_RAW_HID_USAGE_PAGE: u16 = 0xff60;
const QMK_RAW_HID_USAGE: u16 = 0x0061;
const SURFACE_ID: &str = "leds";

static BUSES: &[PluginBus] = &[PluginBus::Hid];
// Product IDs remain empty until exact model profiles have been validated.
static VENDORS: &[PluginVendorId] = &[];
static HINTS: &[PluginProbeHint] = &[];

#[derive(Debug, Clone)]
struct Device {
    product_id: u16,
    product_name: Option<String>,
    path: CString,
    discovery: Discovery,
}

struct Keychron {
    shadows: Mutex<HashMap<u16, Vec<Option<Hsv>>>>,
}

impl LuminatePlugin for Keychron {
    fn new() -> Result<Self, PluginError> {
        Ok(Self {
            shadows: Mutex::new(HashMap::new()),
        })
    }

    fn probe(&self) -> ProbeOutcome {
        match scan() {
            Ok(devices) if devices.is_empty() => ProbeOutcome::Dormant,
            Ok(devices) => {
                tracing::warn!(
                    count = devices.len(),
                    "found unvalidated Keychron RGB firmware; experimental lighting is enabled"
                );
                ProbeOutcome::Ready
            }
            Err(error) => {
                tracing::warn!(error = %error, "failed to enumerate Keychron HID collections");
                ProbeOutcome::Unsupported
            }
        }
    }

    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
        scan()
            .map(|devices| devices.iter().map(device_descriptor).collect())
            .map_err(PluginError::Unavailable)
    }

    fn apply(
        &self,
        _context: &PluginRequestContext,
        update: &PluginUpdate,
    ) -> Result<(), PluginError> {
        let devices = scan().map_err(PluginError::Unavailable)?;
        let device = devices
            .iter()
            .find(|device| device_id(device.product_id) == update.target.device_id())
            .ok_or_else(|| {
                PluginError::Unavailable("Keychron device is no longer available".to_owned())
            })?;
        let api = hidapi::HidApi::new().map_err(|error| PluginError::Io(error.to_string()))?;
        let hid = api
            .open_path(&device.path)
            .map_err(|error| PluginError::Unavailable(error.to_string()))?;
        let transport = HidTransport::new(hid);

        match &update.operation {
            PluginUpdateOperation::SetEffect { effect } => {
                let hsv = effect_hsv(effect)?;
                let (start, count) = target_range(&update.target, device.discovery.led_count)?;
                let colours = vec![hsv; usize::from(count)];
                set_per_key_colours(&transport, device.discovery.led_count, start, &colours)
                    .map_err(protocol_plugin_error)?;
                self.record_colours(
                    device.product_id,
                    device.discovery.led_count,
                    start,
                    &colours,
                )
            }
            PluginUpdateOperation::SaveCurrent => {
                ensure_whole_target(&update.target)?;
                self.ensure_complete_shadow(device.product_id, device.discovery.led_count)?;
                save(&transport).map_err(protocol_plugin_error)
            }
            PluginUpdateOperation::SetBrightness { .. } => Err(PluginError::Unsupported(
                "the experimental Keychron profile does not advertise brightness".to_owned(),
            )),
            PluginUpdateOperation::Clear => Err(PluginError::Unsupported(
                "the experimental Keychron profile has no validated reset operation".to_owned(),
            )),
            PluginUpdateOperation::SetAppearanceSlots { .. } => Err(PluginError::Unsupported(
                "the experimental Keychron profile does not advertise appearance slots".to_owned(),
            )),
        }
    }
}

impl Keychron {
    fn record_colours(
        &self,
        product_id: u16,
        led_count: u8,
        start: u8,
        colours: &[Hsv],
    ) -> Result<(), PluginError> {
        let mut shadows = self
            .shadows
            .lock()
            .expect("Keychron colour shadow lock poisoned");
        let shadow = shadows
            .entry(product_id)
            .or_insert_with(|| vec![None; usize::from(led_count)]);
        if shadow.len() != usize::from(led_count) {
            *shadow = vec![None; usize::from(led_count)];
        }
        for (index, colour) in colours.iter().enumerate() {
            let position = usize::from(start) + index;
            if let Some(slot) = shadow.get_mut(position) {
                *slot = Some(*colour);
            }
        }
        Ok(())
    }

    fn ensure_complete_shadow(&self, product_id: u16, led_count: u8) -> Result<(), PluginError> {
        let shadows = self
            .shadows
            .lock()
            .expect("Keychron colour shadow lock poisoned");
        let complete = shadows.get(&product_id).is_some_and(|shadow| {
            shadow.len() == usize::from(led_count) && shadow.iter().all(Option::is_some)
        });
        if complete {
            Ok(())
        } else {
            Err(PluginError::InvalidArgument(
                "refusing to persist Keychron lighting until Luminate has set every reported LED"
                    .to_owned(),
            ))
        }
    }
}

fn scan() -> Result<Vec<Device>, String> {
    let api = hidapi::HidApi::new()
        .map_err(|error| format!("failed to initialize Keychron HID discovery: {error}"))?;
    let candidates = api
        .device_list()
        .filter(|info| {
            info.vendor_id() == KEYCHRON_VENDOR_ID
                && info.usage_page() == QMK_RAW_HID_USAGE_PAGE
                && info.usage() == QMK_RAW_HID_USAGE
        })
        .map(|info| {
            (
                info.product_id(),
                info.product_string().map(str::to_owned),
                info.path().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    let mut devices = Vec::new();
    for (product_id, product_name, path) in candidates {
        let hid = match api.open_path(&path) {
            Ok(hid) => hid,
            Err(error) => {
                tracing::debug!(
                    product = format_args!("{product_id:04x}"),
                    error = %error,
                    "could not open candidate Keychron Raw HID collection"
                );
                continue;
            }
        };
        match discover(&HidTransport::new(hid)) {
            Ok(discovery) => devices.push(Device {
                product_id,
                product_name,
                path,
                discovery,
            }),
            Err(error) => tracing::debug!(
                product = format_args!("{product_id:04x}"),
                error = %error,
                "candidate Keychron collection does not support the expected RGB protocol"
            ),
        }
    }

    Ok(unique_product_devices(devices))
}

fn unique_product_devices(mut devices: Vec<Device>) -> Vec<Device> {
    let mut counts = HashMap::new();
    for device in &devices {
        *counts.entry(device.product_id).or_insert(0_usize) += 1;
    }
    devices.retain(|device| {
        let unique = counts.get(&device.product_id) == Some(&1);
        if !unique {
            tracing::warn!(
                product = format_args!("{:04x}", device.product_id),
                "omitting indistinguishable Keychron devices with the same product ID"
            );
        }
        unique
    });
    devices
}

fn device_descriptor(device: &Device) -> DeviceDescriptor {
    let warning = "Experimental, unvalidated generic Keychron RGB profile. LED numbers are firmware indices, not identified physical keys.".to_owned();
    DeviceDescriptor {
        id: device_id(device.product_id),
        name: device
            .product_name
            .clone()
            .unwrap_or_else(|| format!("Keychron {:04X}", device.product_id)),
        vendor: Some("Keychron".to_owned()),
        model: device.product_name.clone(),
        surfaces: vec![SurfaceDescriptor {
            id: SURFACE_ID.to_owned(),
            name: "Unvalidated LED indices".to_owned(),
            kind: SurfaceKind::Opaque,
            physical_tags: vec!["keychron:unvalidated-indices".to_owned()],
            elements: (0..device.discovery.led_count)
                .map(|index| ElementDescriptor {
                    id: element_id(index),
                    name: Some(format!("LED {index}")),
                    kind: ElementKind::Led,
                    geometry: None,
                    physical_tags: Vec::new(),
                    capabilities: colour_capabilities(),
                    notes: Vec::new(),
                    warnings: vec![warning.clone()],
                })
                .collect(),
            capabilities: persistent_capabilities(),
            notes: vec![format!(
                "Firmware {} reports {} RGB LEDs through Keychron protocol {}.",
                device.discovery.firmware_version,
                device.discovery.led_count,
                device.discovery.rgb_protocol_version
            )],
            warnings: vec![warning.clone()],
        }],
        groups: Vec::new(),
        capabilities: persistent_capabilities(),
        claims: vec![HardwareClaim {
            bus: HardwareBus::Hid,
            physical_identity: format!("hid:3434:{:04x}", device.product_id),
            control_domain: "keychron-qmk-raw-hid-rgb".to_owned(),
            exclusivity: ClaimExclusivity::Exclusive,
        }],
        category: Some(DeviceCategory::new(device_category::KEYBOARD)),
        physical_tags: vec!["shape:keyboard".to_owned(), "form:standalone".to_owned()],
        host_attached: true,
        notes: vec![
            "Uses the common RGB protocol discovered from stock Keychron QMK firmware.".to_owned(),
        ],
        warnings: vec![warning],
    }
}

fn colour_capabilities() -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        emission: true,
        off_is_wear_safe: true,
        ..CapabilitySet::default()
    }
}

fn persistent_capabilities() -> CapabilitySet {
    CapabilitySet {
        persistence: PersistenceCapability::CurrentState {
            requirement: PersistenceRequirement::Optional,
            explicit_commit: true,
            readback: false,
        },
        ..colour_capabilities()
    }
}

fn target_range(target: &PluginTarget, led_count: u8) -> Result<(u8, u8), PluginError> {
    match target {
        PluginTarget::Device { .. } => Ok((0, led_count)),
        PluginTarget::Surface { surface, .. } if surface == SURFACE_ID => Ok((0, led_count)),
        PluginTarget::Element {
            surface, element, ..
        } if surface == SURFACE_ID => parse_element_id(element)
            .filter(|index| *index < led_count)
            .map(|index| (index, 1))
            .ok_or_else(|| PluginError::InvalidArgument(format!("unknown Keychron LED {element}"))),
        PluginTarget::Surface { .. }
        | PluginTarget::Element { .. }
        | PluginTarget::Group { .. } => Err(PluginError::InvalidArgument(
            "target is not part of the experimental Keychron LED surface".to_owned(),
        )),
    }
}

fn ensure_whole_target(target: &PluginTarget) -> Result<(), PluginError> {
    match target {
        PluginTarget::Device { .. } => Ok(()),
        PluginTarget::Surface { surface, .. } if surface == SURFACE_ID => Ok(()),
        PluginTarget::Surface { .. }
        | PluginTarget::Element { .. }
        | PluginTarget::Group { .. } => Err(PluginError::InvalidArgument(
            "Keychron persistence applies to the complete device or LED surface".to_owned(),
        )),
    }
}

fn effect_hsv(effect: &Effect) -> Result<Hsv, PluginError> {
    match effect {
        Effect::Static { colour } => colour
            .try_as_rgb()
            .map(rgb_to_hsv)
            .map_err(|error| PluginError::InvalidArgument(error.to_string())),
        Effect::Off => Ok(Hsv {
            hue: 0,
            saturation: 0,
            value: 0,
        }),
        Effect::Breathe { .. }
        | Effect::Pulse { .. }
        | Effect::Strobe { .. }
        | Effect::Scanner { .. }
        | Effect::Morph { .. }
        | Effect::Spectrum { .. }
        | Effect::Rainbow { .. }
        | Effect::Hardware { .. } => Err(PluginError::Unsupported(
            "the experimental Keychron profile supports only static colour and off".to_owned(),
        )),
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    reason = "RGB components are exact u8-derived floats before comparison, then normalized, rounded, and clamped to Keychron's u8 HSV channels."
)]
fn rgb_to_hsv(rgb: Rgb) -> Hsv {
    let red = f32::from(rgb.r) / 255.0;
    let green = f32::from(rgb.g) / 255.0;
    let blue = f32::from(rgb.b) / 255.0;
    let maximum = red.max(green).max(blue);
    let minimum = red.min(green).min(blue);
    let delta = maximum - minimum;
    let hue = if delta == 0.0 {
        0.0
    } else if maximum == red {
        60.0 * ((green - blue) / delta).rem_euclid(6.0)
    } else if maximum == green {
        60.0 * ((blue - red) / delta + 2.0)
    } else {
        60.0 * ((red - green) / delta + 4.0)
    };
    let saturation = if maximum == 0.0 { 0.0 } else { delta / maximum };
    Hsv {
        hue: ((hue / 360.0) * 255.0).round().clamp(0.0, 255.0) as u8,
        saturation: (saturation * 255.0).round().clamp(0.0, 255.0) as u8,
        value: (maximum * 255.0).round().clamp(0.0, 255.0) as u8,
    }
}

fn protocol_plugin_error(error: protocol::ProtocolError) -> PluginError {
    match error {
        protocol::ProtocolError::Transport(message) => PluginError::Io(message),
        protocol::ProtocolError::InvalidColourRange { .. } => {
            PluginError::InvalidArgument(error.to_string())
        }
        protocol::ProtocolError::UnexpectedCommand { .. }
        | protocol::ProtocolError::UnexpectedSubcommand { .. }
        | protocol::ProtocolError::UnsupportedProtocol { .. }
        | protocol::ProtocolError::UnsupportedRgbProtocol(_)
        | protocol::ProtocolError::RgbUnsupported
        | protocol::ProtocolError::CommandFailed { .. }
        | protocol::ProtocolError::InvalidLedCount => PluginError::Unavailable(error.to_string()),
    }
}

fn device_id(product_id: u16) -> String {
    format!("keychron-3434-{product_id:04x}")
}

fn element_id(index: u8) -> String {
    format!("led-{index:03}")
}

fn parse_element_id(id: &str) -> Option<u8> {
    id.strip_prefix("led-")?.parse().ok()
}

luminate_export_plugin! {
    plugin: Keychron,
    name: NAME,
    version: VERSION,
    priority: 30,
    recommended_reconciliation: Some(ReconciliationPolicy::Leave),
    buses: BUSES,
    vendors: VENDORS,
    probe_hints: HINTS,
    start: none,
    rescan: none,
    batch: default,
    read_state: none,
    frame_upload: none,
    shm_frame: none,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_device(led_count: u8) -> Device {
        Device {
            product_id: 0x1234,
            product_name: Some("Keychron Test Board".to_owned()),
            path: c"mock".to_owned(),
            discovery: Discovery {
                firmware_version: "v1.2.3".to_owned(),
                rgb_protocol_version: 1,
                led_count,
            },
        }
    }

    #[test]
    fn unvalidated_topology_uses_numbered_leds_and_prominent_warnings() {
        let descriptor = device_descriptor(&mock_device(3));

        assert_eq!(descriptor.id, "keychron-3434-1234");
        assert_eq!(descriptor.surfaces[0].elements.len(), 3);
        assert_eq!(descriptor.surfaces[0].elements[2].id, "led-002");
        assert!(descriptor.surfaces[0].elements[0].geometry.is_none());
        assert_eq!(
            descriptor.physical_tags,
            ["shape:keyboard", "form:standalone"]
        );
        assert!(descriptor.warnings[0].contains("Experimental"));
    }

    #[test]
    fn target_ranges_preserve_unknown_physical_identity() {
        let device = "keychron-3434-1234".to_owned();
        assert_eq!(
            target_range(
                &PluginTarget::Element {
                    device,
                    surface: SURFACE_ID.to_owned(),
                    element: "led-080".to_owned(),
                },
                81,
            ),
            Ok((80, 1))
        );
    }

    #[test]
    fn rgb_primary_colours_convert_to_keychron_hsv() {
        assert_eq!(
            rgb_to_hsv(Rgb::new(255, 0, 0)),
            Hsv {
                hue: 0,
                saturation: 255,
                value: 255,
            }
        );
        assert_eq!(rgb_to_hsv(Rgb::new(0, 255, 0)).hue, 85);
        assert_eq!(rgb_to_hsv(Rgb::new(0, 0, 255)).hue, 170);
    }

    #[test]
    fn persistence_requires_a_complete_session_shadow() {
        let plugin = Keychron::new().expect("construct plugin");
        assert!(plugin.ensure_complete_shadow(0x1234, 2).is_err());
        plugin
            .record_colours(
                0x1234,
                2,
                0,
                &[Hsv {
                    hue: 0,
                    saturation: 0,
                    value: 0,
                }],
            )
            .expect("record first LED");
        assert!(plugin.ensure_complete_shadow(0x1234, 2).is_err());
        plugin
            .record_colours(
                0x1234,
                2,
                1,
                &[Hsv {
                    hue: 1,
                    saturation: 2,
                    value: 3,
                }],
            )
            .expect("record second LED");
        assert!(plugin.ensure_complete_shadow(0x1234, 2).is_ok());
    }

    #[test]
    fn duplicate_product_ids_are_omitted_instead_of_receiving_unstable_ids() {
        let first = mock_device(3);
        let mut duplicate = mock_device(3);
        duplicate.path = c"other-mock".to_owned();
        let mut distinct = mock_device(4);
        distinct.product_id = 0x5678;

        let devices = unique_product_devices(vec![first, duplicate, distinct]);

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].product_id, 0x5678);
    }
}
