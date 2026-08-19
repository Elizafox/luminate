// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unnecessary_wraps,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! LIFX LAN plugin for plain colour bulbs and linear multizone products.

#![allow(
    clippy::indexing_slicing,
    reason = "LIFX packet handling uses fixed protocol offsets after length checks."
)]

mod products;
mod protocol;

use luminate_core::control::ReconciliationPolicy;
use luminate_plugin_api::sdk::{
    DiscoveryPacer, DynamicDeviceRegistry, LuminatePlugin, ReadablePlugin, RegistryExpiry,
    RegistryPoisonError, RescanPlugin, StartPlugin,
};
use std::process;
use std::thread;

use std::collections::{HashMap, HashSet};
use std::ffi::CStr;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use thiserror::Error;

use luminate_core::capability::{
    BrightnessCapability, CapabilityScope, CapabilitySet, ColourCapability, ColourChannel,
    ColourEncoding, EffectParameter, HardwareEffectDescriptor, HardwareEffectId,
    HardwareEffectsCapability, PhysicalPowerCapability, PowerDomainRef, ReadableFacet,
    ReadbackFidelity, StateReadbackCapability,
};
use luminate_core::colour::{Colour, Rgb8Error};
use luminate_core::device::DeviceCategory;
use luminate_core::device::device_category;
use luminate_core::effect::Effect;
use luminate_core::element::{ElementGeometry, ElementKind};
use luminate_core::rgb::Rgb;
use luminate_core::state::{
    AppearanceState, EmissionState, FacetValue, PhysicalPowerState, StateFacetKind,
};
use luminate_core::surface::SurfaceKind;
use luminate_core::util::DiscreteRange;
use luminate_plugin_api::{
    ClaimExclusivity, DeviceDescriptor, ElementDescriptor, HardwareBus, HardwareClaim, PluginBus,
    PluginError, PluginFacetObservation, PluginProbeHint, PluginReadError, PluginReadRequest,
    PluginRequestContext, PluginSettingDescriptor, PluginStateSnapshot, PluginTarget, PluginUpdate,
    PluginUpdateOperation, PluginVendorId, ProbeOutcome, RescanReason, SurfaceDescriptor,
    configuration, luminate_export_plugin,
};

use products::{ProductTopology, product as lookup_product};
use protocol::Hsbk;

const NAME: &CStr = c"luminate-plugin-lifx";
const VERSION: &CStr = c"0.1.0";
const DISCOVERY_PORT: u16 = 56_700;
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(10);
const DISCOVERY_WINDOW: Duration = Duration::from_millis(900);
const DISCOVERY_REFRESH_BUDGET: Duration = Duration::from_secs(5);
const DEVICE_EXPIRY: Duration = Duration::from_secs(75);
const MAX_CANDIDATES_PER_CYCLE: usize = 64;
const MAX_DISCOVERED_DEVICES: usize = 256;
const MAX_DISCOVERED_DEVICES_PER_SOURCE: usize = 16;
const REQUEST_TIMEOUT: Duration = Duration::from_millis(800);
const REQUEST_ATTEMPTS: usize = 3;
const DEFAULT_KELVIN: u16 = 3500;
const MAX_LINEAR_ZONES: u16 = 255;
const ZONES_SURFACE_ID: &str = "zones";

static BUSES: &[PluginBus] = &[PluginBus::Network];
static VENDORS: &[PluginVendorId] = &[PluginVendorId {
    vendor: 1,
    product: 0,
}];
static HINTS: &[PluginProbeHint] = &[];
static SETTINGS: &[PluginSettingDescriptor] = &[PluginSettingDescriptor::string(
    c"discovery_address",
    c"Discovery address",
    c"IPv4 address and UDP port used for LIFX LAN discovery.",
    c"\"255.255.255.255:56700\"",
    false,
    false,
)];

#[derive(Debug, Clone, Copy)]
struct LifxConfig {
    discovery_address: SocketAddrV4,
}

fn lifx_configuration() -> Result<&'static LifxConfig, &'static str> {
    static CONFIGURATION: OnceLock<Result<LifxConfig, String>> = OnceLock::new();
    CONFIGURATION
        .get_or_init(|| {
            let value = configuration::deserialize::<serde_json::Value>()
                .map_err(|error| error.to_string())?;
            parse_lifx_configuration(&value)
        })
        .as_ref()
        .map_err(String::as_str)
}

fn parse_lifx_configuration(value: &serde_json::Value) -> Result<LifxConfig, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "LIFX configuration must be an object".to_owned())?;
    if let Some(key) = object
        .keys()
        .find(|key| key.as_str() != "discovery_address")
    {
        return Err(format!("unknown LIFX configuration field {key:?}"));
    }
    let discovery_address = match object.get("discovery_address") {
        Some(serde_json::Value::String(address)) => address
            .parse::<SocketAddrV4>()
            .map_err(|error| format!("invalid LIFX discovery_address: {error}"))?,
        Some(_) => {
            return Err("LIFX discovery_address must be an IPv4 socket address".to_owned());
        }
        None => SocketAddrV4::new(Ipv4Addr::BROADCAST, DISCOVERY_PORT),
    };
    Ok(LifxConfig { discovery_address })
}

#[derive(Debug, Clone)]
struct DiscoveredDevice {
    id: String,
    target: [u8; 8],
    address: SocketAddr,

    vendor: u32,
    product: u32,
    product_name: &'static str,
    topology: ProductTopology,
    zone_count: Option<u16>,

    label: String,
}

type TopologyFingerprint = (String, u32, Option<u16>);

struct RuntimeState {
    source: u32,

    devices: DynamicDeviceRegistry<DiscoveredDevice, TopologyFingerprint>,
    sequences: Mutex<HashMap<[u8; 8], u8>>,
    /// Lets a rescan cut short the discovery thread's inter-cycle wait.
    discovery: DiscoveryPacer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("LIFX sequence lock poisoned")]
struct SequencePoisonError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetScope {
    Whole,
    Zone(u16),
}

fn runtime() -> &'static RuntimeState {
    static RUNTIME: OnceLock<RuntimeState> = OnceLock::new();
    RUNTIME.get_or_init(|| RuntimeState {
        source: source_identifier(),
        devices: DynamicDeviceRegistry::new(
            MAX_DISCOVERED_DEVICES,
            DEVICE_EXPIRY,
            RegistryExpiry::AtOrAfter,
        ),
        sequences: Mutex::new(HashMap::new()),
        discovery: DiscoveryPacer::new(),
    })
}

fn lifx_init() {
    let state = runtime();
    let source = state.source;
    match thread::Builder::new()
        .name("luminate-lifx-discovery".to_owned())
        .spawn(move || discovery_loop(source))
    {
        Ok(_thread) => tracing::info!(source, "LIFX discovery thread started"),
        Err(error) => tracing::error!(error = %error, "failed to start LIFX discovery thread"),
    }
}

struct Lifx;

impl LuminatePlugin for Lifx {
    fn new() -> Result<Self, PluginError> {
        lifx_configuration().map_err(|error| PluginError::InvalidArgument(error.to_owned()))?;
        let _ = runtime();
        Ok(Self)
    }

    fn probe(&self) -> ProbeOutcome {
        // A network device may appear after daemon startup. Loading must not
        // depend on a bulb answering the first broadcast.
        ProbeOutcome::Dormant
    }

    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
        let devices = runtime()
            .devices
            .snapshot()
            .map_err(|error| PluginError::Internal(error.to_string()))?;
        Ok(devices.iter().map(device_descriptor).collect())
    }

    fn apply(
        &self,
        _context: &PluginRequestContext,
        update: &PluginUpdate,
    ) -> Result<(), PluginError> {
        apply_update(update).inspect_err(|error| {
            tracing::warn!(error = %error, "LIFX update failed");
        })
    }
}

impl StartPlugin for Lifx {
    fn start(&self) {
        lifx_init();
    }
}

impl RescanPlugin for Lifx {
    fn rescan(&self, reason: RescanReason) {
        // Refresh before topology is pulled again. Registry expiry removes
        // absent devices without making every device briefly disappear.
        tracing::debug!(reason = ?reason, "LIFX rescan requested; waking discovery");
        runtime().discovery.wake();
    }
}

impl ReadablePlugin for Lifx {
    fn read_state(
        &self,
        _context: &PluginRequestContext,
        request: &PluginReadRequest,
    ) -> Result<PluginStateSnapshot, PluginError> {
        read_state_snapshot(request)
    }
}

fn read_state_snapshot(requested: &PluginReadRequest) -> Result<PluginStateSnapshot, PluginError> {
    let mut snapshot = PluginStateSnapshot::default();
    for requested in &requested.targets {
        let Some(device_id) = plugin_target_device(&requested.target) else {
            snapshot.errors.push(PluginReadError {
                target: requested.target.clone(),
                diagnostic: "group targets are not physical readback scopes".to_owned(),
            });
            continue;
        };
        // Discovery owns this map too, so never hold its lock across network I/O.
        let device = runtime()
            .devices
            .get(device_id)
            .map_err(|error| PluginError::Internal(error.to_string()))?;
        let Some(device) = device else {
            snapshot.errors.push(PluginReadError {
                target: requested.target.clone(),
                diagnostic: "LIFX device is unavailable".to_owned(),
            });
            continue;
        };
        match read_target_facets(&device, &requested.target, &requested.facets) {
            Ok(observations) => snapshot.observations.extend(observations),
            Err(diagnostic) => snapshot.errors.push(PluginReadError {
                target: requested.target.clone(),
                diagnostic,
            }),
        }
    }
    Ok(snapshot)
}

fn plugin_target_device(target: &PluginTarget) -> Option<&str> {
    match target {
        PluginTarget::Device { device }
        | PluginTarget::Surface { device, .. }
        | PluginTarget::Element { device, .. } => Some(device),
        PluginTarget::Group { .. } => None,
    }
}

fn read_target_facets(
    device: &DiscoveredDevice,
    target: &PluginTarget,
    facets: &[StateFacetKind],
) -> Result<Vec<PluginFacetObservation>, String> {
    let scope = resolve_scope(device, target)?;
    let (colour, power) = match scope {
        TargetScope::Whole => read_light_state(device)?,
        TargetScope::Zone(zone) => {
            let colour = get_zone_colour(device, zone)?;
            let (_, power) = read_light_state(device)?;
            (colour, power)
        }
    };
    // Appearance is chromatic state; brightness is reported independently.
    let appearance = appearance_from_hsbk(colour);
    let emitting = power && colour.brightness != 0;
    let mut observations = Vec::new();
    for facet in facets {
        let value = match facet {
            StateFacetKind::Appearance => {
                FacetValue::Appearance(AppearanceState::Static(appearance.clone()))
            }
            StateFacetKind::Brightness => {
                FacetValue::Brightness(u32::from(colour.brightness / 257))
            }
            StateFacetKind::Emission => FacetValue::Emission(if emitting {
                EmissionState::Emitting
            } else {
                EmissionState::Dark
            }),
            StateFacetKind::PhysicalPower if matches!(scope, TargetScope::Whole) => {
                FacetValue::PhysicalPower(if power {
                    PhysicalPowerState::On
                } else {
                    PhysicalPowerState::Off
                })
            }
            // `EffectiveAppearance` is daemon-synthesized only and never
            // requested from a plugin.
            StateFacetKind::PhysicalPower
            | StateFacetKind::EffectiveAppearance
            | StateFacetKind::AppearanceSlots => continue,
        };
        observations.push(PluginFacetObservation {
            target: target.clone(),
            value,
        });
    }
    Ok(observations)
}

fn read_light_state(device: &DiscoveredDevice) -> Result<(Hsbk, bool), String> {
    let response = request(
        device.address,
        device.target,
        protocol::GET_COLOR,
        &[],
        protocol::LIGHT_STATE,
        false,
    )?;
    let (_, payload) = protocol::parse(&response).map_err(|error| error.to_string())?;
    let colour = protocol::parse_hsbk(payload).map_err(|error| error.to_string())?;
    let power = payload
        .get(10..12)
        .and_then(|bytes| <[u8; 2]>::try_from(bytes).ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| "short LIFX LightState power field".to_owned())?
        != 0;
    Ok((colour, power))
}

fn hsbk_to_rgb(colour: Hsbk) -> Rgb {
    let hue = f64::from(colour.hue) / 65_535.0;
    let saturation = f64::from(colour.saturation) / 65_535.0;
    let value = f64::from(colour.brightness) / 65_535.0;
    let sector = hue * 6.0;
    let chroma = value * saturation;
    let x = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match sector {
        value if value < 1.0 => (chroma, x, 0.0),
        value if value < 2.0 => (x, chroma, 0.0),
        value if value < 3.0 => (0.0, chroma, x),
        value if value < 4.0 => (0.0, x, chroma),
        value if value < 5.0 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let offset = value - chroma;
    let channel = |value: f64| {
        let value = ((value + offset) * 255.0).round().clamp(0.0, 255.0);
        // The clamp proves this narrowing conversion is within `u8`'s range.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the value is rounded and clamped to the complete u8 range above"
        )]
        let value = value as u8;
        value
    };
    Rgb::new(channel(red), channel(green), channel(blue))
}

/// Converts a readback `Hsbk` into the device-independent `Colour` it
/// represents. Zero saturation means the device is in white-balance mode:
/// that's reported as `Cct` rather than converted through [`hsbk_to_rgb`],
/// which would collapse any kelvin value to flat white once saturation drops
/// to zero.
fn appearance_from_hsbk(colour: Hsbk) -> Colour {
    if colour.saturation == 0 {
        Colour::cct(u32::from(colour.kelvin))
    } else {
        let rgb = hsbk_to_rgb(Hsbk {
            brightness: u16::MAX,
            ..colour
        });
        Colour::rgb(rgb)
    }
}

fn apply_update(update: &PluginUpdate) -> Result<(), PluginError> {
    let device_id = update.target.device_id();
    let device = runtime()
        .devices
        .get(device_id)
        .map_err(|error| PluginError::Internal(error.to_string()))?
        .ok_or_else(|| {
            PluginError::Unavailable(format!("LIFX device is unavailable: {device_id}"))
        })?;
    let scope = resolve_scope(&device, &update.target).map_err(PluginError::InvalidTarget)?;

    if matches!(update.operation, PluginUpdateOperation::SaveCurrent) {
        return Err(PluginError::Unsupported(
            "LIFX phase 1 does not expose firmware persistence".to_owned(),
        ));
    }

    let result: Result<(), String> = (|| match (&update.operation, scope) {
        (PluginUpdateOperation::SetEffect { effect }, TargetScope::Whole) => {
            apply_effect(&device, effect)
        }
        (PluginUpdateOperation::SetEffect { effect }, TargetScope::Zone(zone)) => {
            apply_zone_effect(&device, zone, effect)
        }
        (PluginUpdateOperation::SetBrightness { value }, TargetScope::Whole) => {
            set_brightness(&device, *value)
        }
        (PluginUpdateOperation::SetBrightness { value }, TargetScope::Zone(zone)) => {
            set_zone_brightness(&device, zone, *value)
        }
        (PluginUpdateOperation::Clear, TargetScope::Whole) => transition_to_off(&device),
        (PluginUpdateOperation::Clear, TargetScope::Zone(zone)) => {
            stop_move(&device)?;
            set_zone_colour(&device, zone, off_hsbk(), 1)?;
            set_power(&device, true)
        }
        (PluginUpdateOperation::SaveCurrent, _) => {
            Err("LIFX phase 1 does not expose firmware persistence".to_owned())
        }
        (PluginUpdateOperation::SetAppearanceSlots { .. }, _) => {
            Err("LIFX targets do not advertise appearance slots".to_owned())
        }
    })();
    result.map_err(classify_lifx_error)
}

fn classify_lifx_error(diagnostic: String) -> PluginError {
    if diagnostic.contains("lock poisoned") {
        return PluginError::Internal(diagnostic);
    }
    if diagnostic.contains("timed out") || diagnostic.contains("deadline expired") {
        return PluginError::Unavailable(diagnostic);
    }
    if diagnostic.contains("not supported by this LIFX shape")
        || diagnostic.contains("animated effects require")
        || diagnostic.contains("advertises no hardware effects")
    {
        return PluginError::Unsupported(diagnostic);
    }
    if diagnostic.contains("no zone count") || diagnostic.contains("no usable zones") {
        return PluginError::Internal(diagnostic);
    }
    if diagnostic.contains("brightness exceeds")
        || diagnostic.contains("only additive RGB")
        || diagnostic.contains("colour channel")
        || diagnostic.contains("missing red channel")
        || diagnostic.contains("missing green channel")
        || diagnostic.contains("missing blue channel")
        || diagnostic.contains("zone index exceeds")
    {
        return PluginError::InvalidArgument(diagnostic);
    }
    PluginError::Io(diagnostic)
}

fn resolve_scope(device: &DiscoveredDevice, target: &PluginTarget) -> Result<TargetScope, String> {
    match target {
        PluginTarget::Device { .. } => Ok(TargetScope::Whole),
        PluginTarget::Surface { surface, .. }
            if device.topology == ProductTopology::Linear && surface == ZONES_SURFACE_ID =>
        {
            Ok(TargetScope::Whole)
        }
        PluginTarget::Element {
            surface, element, ..
        } if device.topology == ProductTopology::Linear && surface == ZONES_SURFACE_ID => {
            let zone = element
                .strip_prefix("zone-")
                .ok_or_else(|| format!("invalid LIFX zone element: {element}"))?
                .parse::<u16>()
                .map_err(|error| format!("invalid LIFX zone index: {error}"))?;
            let count = device
                .zone_count
                .ok_or_else(|| "LIFX linear device has no zone count".to_owned())?;
            if zone >= count {
                return Err(format!("LIFX zone {zone} is outside 0..{count}"));
            }
            Ok(TargetScope::Zone(zone))
        }
        PluginTarget::Surface { .. }
        | PluginTarget::Element { .. }
        | PluginTarget::Group { .. } => Err("target is not part of this LIFX device".to_owned()),
    }
}

fn apply_effect(device: &DiscoveredDevice, effect: &Effect) -> Result<(), String> {
    match effect {
        Effect::Off => transition_to_off(device),
        Effect::Static { colour } => {
            stop_move(device)?;
            set_colour(device, colour)?;
            set_power(device, true)
        }
        Effect::Breathe { colour, period_ms } => {
            stop_move(device)?;
            set_waveform(device, *colour, *period_ms, 1)?;
            set_power(device, true)
        }
        Effect::Pulse { colour, period_ms } | Effect::Strobe { colour, period_ms } => {
            stop_move(device)?;
            set_waveform(device, *colour, *period_ms, 4)?;
            set_power(device, true)
        }
        Effect::Scanner { colour, period_ms } if device.topology == ProductTopology::Linear => {
            start_scanner(device, *colour, *period_ms)
        }
        Effect::Spectrum { period_ms } | Effect::Rainbow { period_ms }
            if device.topology == ProductTopology::Linear =>
        {
            start_rainbow(device, *period_ms)
        }
        Effect::Scanner { .. }
        | Effect::Morph { .. }
        | Effect::Spectrum { .. }
        | Effect::Rainbow { .. } => Err("effect is not supported by this LIFX shape".to_owned()),
        // This plugin advertises only the typed effects above, so the daemon
        // never routes a `Hardware` effect here; reject defensively.
        Effect::Hardware { .. } => {
            Err("this LIFX device advertises no hardware effects".to_owned())
        }
    }
}

fn apply_zone_effect(device: &DiscoveredDevice, zone: u16, effect: &Effect) -> Result<(), String> {
    match effect {
        Effect::Off => {
            stop_move(device)?;
            set_zone_colour(device, zone, off_hsbk(), 1)?;
            set_power(device, true)
        }
        Effect::Static { colour } => {
            stop_move(device)?;
            let hsbk = hsbk_from_zone_colour(device, zone, colour)?;
            set_zone_colour(device, zone, hsbk, 1)?;
            set_power(device, true)
        }
        Effect::Breathe { .. }
        | Effect::Pulse { .. }
        | Effect::Strobe { .. }
        | Effect::Scanner { .. }
        | Effect::Morph { .. }
        | Effect::Spectrum { .. }
        | Effect::Rainbow { .. } => {
            Err("animated effects require the whole linear surface".to_owned())
        }
        Effect::Hardware { .. } => {
            Err("this LIFX device advertises no hardware effects".to_owned())
        }
    }
}

fn set_power(device: &DiscoveredDevice, on: bool) -> Result<(), String> {
    acknowledged_request(
        device,
        protocol::SET_POWER,
        &protocol::set_power_payload(on),
    )
}

fn set_colour(device: &DiscoveredDevice, colour: &Colour) -> Result<(), String> {
    let hsbk = hsbk_from_colour(device, colour)?;
    let payload = protocol::set_color_payload(hsbk, 0);
    acknowledged_request(device, protocol::SET_COLOR, &payload)
}

/// Converts a device-independent `Colour` into the wire `Hsbk` LIFX expects,
/// natively: `Additive` RGB is converted via `rgb_to_hsbk`, and `Cct` sets a
/// desaturated white balance at the requested kelvin, preserving the
/// device's current brightness (mirroring `set_brightness`'s read-modify-write).
fn hsbk_from_colour(device: &DiscoveredDevice, colour: &Colour) -> Result<Hsbk, String> {
    match colour.encoding() {
        ColourEncoding::Additive => Ok(rgb_to_hsbk(rgb_from_colour(colour)?)),
        ColourEncoding::Cct => {
            let kelvin = kelvin_from_colour(colour)?;
            let (current, _) = read_light_state(device)?;
            Ok(Hsbk {
                hue: 0,
                saturation: 0,
                brightness: current.brightness,
                kelvin,
            })
        }
        ColourEncoding::Hsv | ColourEncoding::Hsl | ColourEncoding::Monochrome => Err(format!(
            "unsupported colour encoding {:?}",
            colour.encoding()
        )),
    }
}

/// As [`hsbk_from_colour`], but for a single zone: `Cct` preserves that
/// zone's current brightness rather than the whole device's.
fn hsbk_from_zone_colour(
    device: &DiscoveredDevice,
    zone: u16,
    colour: &Colour,
) -> Result<Hsbk, String> {
    match colour.encoding() {
        ColourEncoding::Additive => Ok(rgb_to_hsbk(rgb_from_colour(colour)?)),
        ColourEncoding::Cct => {
            let kelvin = kelvin_from_colour(colour)?;
            let current = get_zone_colour(device, zone)?;
            Ok(Hsbk {
                hue: 0,
                saturation: 0,
                brightness: current.brightness,
                kelvin,
            })
        }
        ColourEncoding::Hsv | ColourEncoding::Hsl | ColourEncoding::Monochrome => Err(format!(
            "unsupported colour encoding {:?}",
            colour.encoding()
        )),
    }
}

fn kelvin_from_colour(colour: &Colour) -> Result<u16, String> {
    colour
        .channel(ColourChannel::Temperature)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| "missing temperature channel".to_owned())
}

fn set_brightness(device: &DiscoveredDevice, value: u32) -> Result<(), String> {
    let value = u8::try_from(value).map_err(|_| "brightness exceeds 8-bit range".to_owned())?;
    stop_move(device)?;
    let response = request(
        device.address,
        device.target,
        protocol::GET_COLOR,
        &[],
        protocol::LIGHT_STATE,
        false,
    )?;
    let (_, payload) = protocol::parse(&response).map_err(|error| error.to_string())?;
    let mut colour = protocol::parse_hsbk(payload).map_err(|error| error.to_string())?;
    colour.brightness = u16::from(value) * 257;
    let payload = protocol::set_color_payload(colour, 0);
    acknowledged_request(device, protocol::SET_COLOR, &payload)?;
    set_power(device, true)
}

fn set_zone_brightness(device: &DiscoveredDevice, zone: u16, value: u32) -> Result<(), String> {
    let value = u8::try_from(value).map_err(|_| "brightness exceeds 8-bit range".to_owned())?;
    stop_move(device)?;
    let mut colour = get_zone_colour(device, zone)?;
    colour.brightness = u16::from(value) * 257;
    set_zone_colour(device, zone, colour, 1)?;
    set_power(device, true)
}

fn get_zone_colour(device: &DiscoveredDevice, zone: u16) -> Result<Hsbk, String> {
    let zone = u8::try_from(zone).map_err(|_| "zone index exceeds legacy limit".to_owned())?;
    let payload = protocol::get_color_zones_payload(zone, zone);
    let response = request(
        device.address,
        device.target,
        protocol::GET_COLOR_ZONES,
        &payload,
        protocol::STATE_ZONE,
        false,
    )?;
    let (_, payload) = protocol::parse(&response).map_err(|error| error.to_string())?;
    protocol::parse_hsbk(
        payload
            .get(2..)
            .ok_or_else(|| "short StateZone payload".to_owned())?,
    )
    .map_err(|error| error.to_string())
}

fn set_zone_colour(
    device: &DiscoveredDevice,
    zone: u16,
    colour: Hsbk,
    apply: u8,
) -> Result<(), String> {
    let zone = u8::try_from(zone).map_err(|_| "zone index exceeds legacy limit".to_owned())?;
    let payload = protocol::set_color_zones_payload(zone, zone, colour, 0, apply);
    acknowledged_request(device, protocol::SET_COLOR_ZONES, &payload)
}

fn set_zone_range(
    device: &DiscoveredDevice,
    start: u16,
    end: u16,
    colour: Hsbk,
    apply: u8,
) -> Result<(), String> {
    let start = u8::try_from(start).map_err(|_| "zone index exceeds legacy limit".to_owned())?;
    let end = u8::try_from(end).map_err(|_| "zone index exceeds legacy limit".to_owned())?;
    let payload = protocol::set_color_zones_payload(start, end, colour, 0, apply);
    acknowledged_request(device, protocol::SET_COLOR_ZONES, &payload)
}

fn start_scanner(device: &DiscoveredDevice, rgb: Rgb, period_ms: u32) -> Result<(), String> {
    stop_move(device)?;
    let count = linear_zone_count(device)?;
    set_zone_range(device, 0, count - 1, off_hsbk(), 0)?;
    set_zone_colour(device, 0, rgb_to_hsbk(rgb), 1)?;
    start_move(device, period_ms)?;
    set_power(device, true)
}

fn start_rainbow(device: &DiscoveredDevice, period_ms: u32) -> Result<(), String> {
    stop_move(device)?;
    let count = linear_zone_count(device)?;
    for zone in 0..count {
        let hue = u16::try_from((u32::from(zone) * 65_535) / u32::from(count))
            .map_err(|error| error.to_string())?;
        let colour = Hsbk {
            hue,
            saturation: u16::MAX,
            brightness: u16::MAX,
            kelvin: DEFAULT_KELVIN,
        };
        let apply = u8::from(zone + 1 == count);
        set_zone_colour(device, zone, colour, apply)?;
    }
    start_move(device, period_ms)?;
    set_power(device, true)
}

fn start_move(device: &DiscoveredDevice, period_ms: u32) -> Result<(), String> {
    let payload = protocol::set_multi_zone_effect_payload(period_ms, false);
    acknowledged_request(device, protocol::SET_MULTI_ZONE_EFFECT, &payload)
}

fn stop_move(device: &DiscoveredDevice) -> Result<(), String> {
    if device.topology != ProductTopology::Linear {
        return Ok(());
    }
    let payload = protocol::set_multi_zone_effect_off_payload();
    acknowledged_request(device, protocol::SET_MULTI_ZONE_EFFECT, &payload)
}

fn transition_to_off(device: &DiscoveredDevice) -> Result<(), String> {
    stop_move(device)?;
    set_power(device, false)
}

fn linear_zone_count(device: &DiscoveredDevice) -> Result<u16, String> {
    device
        .zone_count
        .filter(|count| *count > 0)
        .ok_or_else(|| "LIFX linear device has no usable zones".to_owned())
}

fn off_hsbk() -> Hsbk {
    Hsbk {
        hue: 0,
        saturation: 0,
        brightness: 0,
        kelvin: DEFAULT_KELVIN,
    }
}

fn set_waveform(
    device: &DiscoveredDevice,
    rgb: Rgb,
    period_ms: u32,
    waveform: u8,
) -> Result<(), String> {
    let payload = protocol::set_waveform_payload(rgb_to_hsbk(rgb), period_ms, waveform);
    acknowledged_request(device, protocol::SET_WAVEFORM, &payload)
}

fn acknowledged_request(
    device: &DiscoveredDevice,
    message_type: u16,
    payload: &[u8],
) -> Result<(), String> {
    request(
        device.address,
        device.target,
        message_type,
        payload,
        protocol::ACKNOWLEDGEMENT,
        true,
    )?;
    Ok(())
}

fn request(
    address: SocketAddr,
    target: [u8; 8],
    message_type: u16,
    payload: &[u8],
    expected_type: u16,
    acknowledgement_required: bool,
) -> Result<Vec<u8>, String> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|error| error.to_string())?;
    request_with_socket(
        &socket,
        address,
        LifxRequest {
            target,
            message_type,
            payload,
            expected_types: &[expected_type],
            acknowledgement_required,
        },
    )
    .map_err(|error| error.to_string())
}

#[derive(Clone, Copy)]
struct LifxRequest<'a> {
    target: [u8; 8],
    message_type: u16,
    payload: &'a [u8],
    expected_types: &'a [u16],
    acknowledgement_required: bool,
}

fn request_with_socket(
    socket: &UdpSocket,
    address: SocketAddr,
    request: LifxRequest<'_>,
) -> io::Result<Vec<u8>> {
    let overall_deadline = luminate_plugin_api::current_request_deadline()
        .and_then(luminate_plugin_api::PluginDeadline::remaining)
        .and_then(|remaining| Instant::now().checked_add(remaining));
    request_with_socket_until(socket, address, request, overall_deadline, true)
}

fn request_with_socket_until(
    socket: &UdpSocket,
    address: SocketAddr,
    request: LifxRequest<'_>,
    overall_deadline: Option<Instant>,
    persist_sequence: bool,
) -> io::Result<Vec<u8>> {
    let sequence =
        sequence_for_request(request.target, persist_sequence).map_err(io::Error::other)?;
    let packet = protocol::packet(
        runtime().source,
        request.target,
        sequence,
        request.message_type,
        request.payload,
        false,
        request.acknowledgement_required,
    )?;
    let mut buffer = [0; 2048];
    for _attempt in 0..REQUEST_ATTEMPTS {
        if overall_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            break;
        }
        socket.send_to(&packet, address)?;
        let request_deadline = Instant::now() + REQUEST_TIMEOUT;
        let deadline =
            overall_deadline.map_or(request_deadline, |overall| request_deadline.min(overall));
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            socket.set_read_timeout(Some(remaining))?;
            match socket.recv_from(&mut buffer) {
                Ok((length, sender)) => {
                    if sender != address {
                        continue;
                    }
                    let candidate = &buffer[..length];
                    if response_matches_any(
                        candidate,
                        runtime().source,
                        sequence,
                        request.target,
                        request.expected_types,
                    ) {
                        return Ok(candidate.to_vec());
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    break;
                }
                Err(error) => return Err(error),
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "no LIFX response type in {:?} from {address}",
            request.expected_types
        ),
    ))
}

fn response_matches_any(
    packet: &[u8],
    source: u32,
    sequence: u8,
    target: [u8; 8],
    expected_types: &[u16],
) -> bool {
    protocol::parse(packet).is_ok_and(|(header, _)| {
        header.source == source
            && header.sequence == sequence
            && header.target == target
            && expected_types.contains(&header.message_type)
    })
}

#[derive(Debug, Error)]
enum LifxDiscoveryError {
    #[error(transparent)]
    Io(io::Error),
    #[error(transparent)]
    Registry(RegistryPoisonError),
    #[error(transparent)]
    Sequence(SequencePoisonError),
}

impl From<io::Error> for LifxDiscoveryError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<RegistryPoisonError> for LifxDiscoveryError {
    fn from(error: RegistryPoisonError) -> Self {
        Self::Registry(error)
    }
}

impl From<SequencePoisonError> for LifxDiscoveryError {
    fn from(error: SequencePoisonError) -> Self {
        Self::Sequence(error)
    }
}

fn discovery_loop(source: u32) {
    loop {
        match discovery_cycle(source) {
            Ok(()) => {}
            Err(error @ LifxDiscoveryError::Io(_)) => {
                tracing::warn!(error = %error, "LIFX discovery cycle failed");
            }
            Err(error @ (LifxDiscoveryError::Registry(_) | LifxDiscoveryError::Sequence(_))) => {
                tracing::error!(error = %error, "LIFX discovery stopped after invariant failure");
                return;
            }
        }
        runtime().discovery.wait(DISCOVERY_INTERVAL);
    }
}

fn discovery_cycle(source: u32) -> Result<(), LifxDiscoveryError> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    let discovery_address = lifx_configuration()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?
        .discovery_address;
    socket.set_broadcast(discovery_address.ip().is_broadcast())?;
    let packet = protocol::packet(source, [0; 8], 0, protocol::GET_SERVICE, &[], true, false)?;
    socket.send_to(&packet, discovery_address)?;

    let deadline = Instant::now() + DISCOVERY_WINDOW;
    let mut candidates = HashMap::<[u8; 8], SocketAddr>::new();
    let mut buffer = [0; 2048];
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        socket.set_read_timeout(Some(remaining))?;
        match socket.recv_from(&mut buffer) {
            Ok((length, sender)) => {
                if let Some((target, address)) = parse_service(&buffer[..length], source, sender) {
                    insert_discovery_candidate(&mut candidates, target, address);
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(error) => return Err(error.into()),
        }
    }

    let mut candidates = candidates.into_iter().collect::<Vec<_>>();
    candidates.sort_by_key(|(target, _)| *target);
    let refresh_deadline = Instant::now() + DISCOVERY_REFRESH_BUDGET;
    let mut refreshed = Vec::new();
    for (target, address) in candidates {
        if Instant::now() >= refresh_deadline {
            tracing::debug!("LIFX discovery refresh budget exhausted");
            break;
        }
        let can_refresh = {
            runtime().devices.contains(&device_id(target))?
                || runtime().devices.len()? < MAX_DISCOVERED_DEVICES
        };
        if can_refresh
            && let Some(device) = refresh_device(&socket, target, address, refresh_deadline)?
        {
            refreshed.push(device);
        }
    }
    runtime().devices.refresh_with_source_limit(
        Instant::now(),
        refreshed.into_iter().map(|device| {
            let id = device.id.clone();
            let fingerprint = topology_fingerprint(&device);
            let source = Some(device.address.ip());
            (id, device, fingerprint, source)
        }),
        MAX_DISCOVERED_DEVICES_PER_SOURCE,
    )?;
    let active_targets = runtime()
        .devices
        .snapshot()?
        .into_iter()
        .map(|device| device.target)
        .collect::<HashSet<_>>();
    let mut sequences = lock_sequences(&runtime().sequences)?;
    prune_sequences(&mut sequences, &active_targets);
    drop(sequences);
    Ok(())
}

fn insert_discovery_candidate(
    candidates: &mut HashMap<[u8; 8], SocketAddr>,
    target: [u8; 8],
    address: SocketAddr,
) {
    let source_count = candidates
        .values()
        .filter(|candidate| candidate.ip() == address.ip())
        .count();
    if candidates.contains_key(&target)
        || (candidates.len() < MAX_CANDIDATES_PER_CYCLE
            && source_count < MAX_DISCOVERED_DEVICES_PER_SOURCE)
    {
        candidates.insert(target, address);
    }
}

fn prune_sequences(sequences: &mut HashMap<[u8; 8], u8>, active_targets: &HashSet<[u8; 8]>) {
    sequences.retain(|target, _| active_targets.contains(target));
}

fn parse_service(packet: &[u8], source: u32, sender: SocketAddr) -> Option<([u8; 8], SocketAddr)> {
    let (header, payload) = protocol::parse(packet).ok()?;
    if header.source != source
        || header.message_type != protocol::STATE_SERVICE
        || payload.len() < 5
        || payload[0] != 1
    {
        return None;
    }
    let port = u32::from_le_bytes(payload.get(1..5)?.try_into().ok()?);
    let port = u16::try_from(port).ok().filter(|port| *port != 0)?;
    Some((header.target, SocketAddr::new(sender.ip(), port)))
}

fn refresh_device(
    socket: &UdpSocket,
    target: [u8; 8],
    address: SocketAddr,
    deadline: Instant,
) -> Result<Option<DiscoveredDevice>, RegistryPoisonError> {
    let id = device_id(target);
    if let Some(mut existing) = runtime().devices.get(&id)? {
        let refreshed_zone_count = (existing.topology == ProductTopology::Linear)
            .then(|| query_zone_count(socket, address, target, deadline).ok())
            .flatten();
        existing.address = address;
        if refreshed_zone_count.is_some() {
            existing.zone_count = refreshed_zone_count;
        }
        return Ok(Some(existing));
    }

    let version = request_with_socket_until(
        socket,
        address,
        LifxRequest {
            target,
            message_type: protocol::GET_VERSION,
            payload: &[],
            expected_types: &[protocol::STATE_VERSION],
            acknowledgement_required: false,
        },
        Some(deadline),
        false,
    );
    let Ok(version) = version else {
        return Ok(None);
    };
    let Ok((_, payload)) = protocol::parse(&version) else {
        return Ok(None);
    };
    let Some((vendor, product)) = parse_version(payload) else {
        return Ok(None);
    };
    let Some(product_info) = lookup_product(vendor, product) else {
        tracing::debug!(vendor, product, %id, "LIFX product has an unsupported topology");
        return Ok(None);
    };
    let zone_count = if product_info.topology == ProductTopology::Linear {
        query_zone_count(socket, address, target, deadline).ok()
    } else {
        None
    };
    if product_info.topology == ProductTopology::Linear && zone_count.is_none() {
        tracing::debug!(vendor, product, %id, "LIFX linear product did not report a usable zone count");
        return Ok(None);
    }
    let label = query_label(socket, address, target, deadline);
    let device = DiscoveredDevice {
        id: id.clone(),
        target,
        address,
        vendor,
        product,
        product_name: product_info.name,
        topology: product_info.topology,
        zone_count,
        label,
    };
    tracing::info!(
        %id,
        %address,
        product,
        model = product_info.name,
        ?zone_count,
        "discovered LIFX light"
    );
    Ok(Some(device))
}

fn query_label(
    socket: &UdpSocket,
    address: SocketAddr,
    target: [u8; 8],
    deadline: Instant,
) -> String {
    request_with_socket_until(
        socket,
        address,
        LifxRequest {
            target,
            message_type: protocol::GET_LABEL,
            payload: &[],
            expected_types: &[protocol::STATE_LABEL],
            acknowledgement_required: false,
        },
        Some(deadline),
        true,
    )
    .ok()
    .and_then(|packet| {
        protocol::parse(&packet)
            .ok()
            .map(|(_, payload)| parse_label(payload))
    })
    .unwrap_or_default()
}

fn query_zone_count(
    socket: &UdpSocket,
    address: SocketAddr,
    target: [u8; 8],
    deadline: Instant,
) -> io::Result<u16> {
    let payload = protocol::get_color_zones_payload(0, u8::MAX);
    let response = request_with_socket_until(
        socket,
        address,
        LifxRequest {
            target,
            message_type: protocol::GET_COLOR_ZONES,
            payload: &payload,
            expected_types: &[protocol::STATE_ZONE, protocol::STATE_MULTI_ZONE],
            acknowledgement_required: false,
        },
        Some(deadline),
        true,
    )?;
    let (_, payload) = protocol::parse(&response)?;
    parse_legacy_zone_count(payload)
}

fn parse_legacy_zone_count(payload: &[u8]) -> io::Result<u16> {
    let count = payload.first().copied().map(u16::from).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "multizone response has no count",
        )
    })?;
    if count == 0 || count > MAX_LINEAR_ZONES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("LIFX zone count {count} is outside 1..={MAX_LINEAR_ZONES}"),
        ));
    }
    Ok(count)
}

fn parse_version(payload: &[u8]) -> Option<(u32, u32)> {
    let vendor = u32::from_le_bytes(payload.get(..4)?.try_into().ok()?);
    let product = u32::from_le_bytes(payload.get(4..8)?.try_into().ok()?);
    Some((vendor, product))
}

fn parse_label(payload: &[u8]) -> String {
    let end = payload
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(payload.len().min(32));
    String::from_utf8_lossy(&payload[..end.min(32)])
        .trim()
        .to_owned()
}

fn topology_fingerprint(device: &DiscoveredDevice) -> TopologyFingerprint {
    (device.label.clone(), device.product, device.zone_count)
}

fn device_descriptor(device: &DiscoveredDevice) -> DeviceDescriptor {
    let serial = serial_string(device.target);
    let name = if device.label.is_empty() {
        format!("{} {serial}", device.product_name)
    } else {
        // StateLabel is user-controlled and duplicate labels are normal. Keep
        // it recognizable while appending the stable physical serial so the
        // daemon's global presentation-name invariant cannot hide the plugin's
        // entire topology.
        format!("{} ({serial})", device.label)
    };
    let (surfaces, category, capabilities) = match device.topology {
        ProductTopology::Plain => (
            Vec::new(),
            DeviceCategory::new("light-bulb"),
            bulb_capabilities(),
        ),
        ProductTopology::Linear => (
            vec![linear_surface(device.zone_count.unwrap_or(0))],
            DeviceCategory::new(device_category::LED_STRIP),
            linear_capabilities(CapabilityScope::Device),
        ),
    };
    DeviceDescriptor {
        id: device.id.clone(),
        name,
        vendor: Some("LIFX".to_owned()),
        model: Some(device.product_name.to_owned()),
        surfaces,
        groups: Vec::new(),
        capabilities,
        claims: vec![HardwareClaim {
            bus: HardwareBus::Network,
            physical_identity: device.id.clone(),
            control_domain: "light-output".to_owned(),
            exclusivity: ClaimExclusivity::Exclusive,
        }],
        category: Some(category),
        physical_tags: lookup_product(device.vendor, device.product)
            .map_or(&[] as &[&str], |product| product.physical_tags)
            .iter()
            .map(|tag| (*tag).to_owned())
            .collect(),
        host_attached: false,
        notes: vec![format!(
            "LAN device at {}; LIFX vendor/product {}/{}.",
            device.address, device.vendor, device.product
        )],
        warnings: Vec::new(),
    }
}

fn linear_surface(zone_count: u16) -> SurfaceDescriptor {
    let denominator = f32::from(zone_count.saturating_sub(1).max(1));
    let elements = (0..zone_count)
        .map(|zone| ElementDescriptor {
            id: format!("zone-{zone}"),
            name: Some(format!("Zone {}", zone + 1)),
            kind: ElementKind::Zone,
            geometry: Some(ElementGeometry::Linear {
                position: f32::from(zone) / denominator,
            }),
            physical_tags: Vec::new(),
            capabilities: zone_capabilities(),
            notes: Vec::new(),
            warnings: Vec::new(),
        })
        .collect();
    SurfaceDescriptor {
        id: ZONES_SURFACE_ID.to_owned(),
        name: "Zones".to_owned(),
        kind: SurfaceKind::Linear {
            length: f32::from(zone_count),
        },
        physical_tags: Vec::new(),
        elements,
        capabilities: linear_capabilities(CapabilityScope::Surface),
        notes: vec![format!(
            "Zone count is device-reported at discovery time ({zone_count}); support is untested on physical hardware."
        )],
        warnings: vec!["LIFX Z/Beam support is implemented from the published protocol but has not been live-tested.".to_owned()],
    }
}

fn bulb_capabilities() -> CapabilitySet {
    let duration = || EffectParameter::Duration {
        milliseconds: DiscreteRange::new(100, 60_000, 1),
    };
    let colour = || EffectParameter::Colour {
        minimum_colours: 1,
        maximum_colours: 1,
    };
    CapabilitySet {
        colour: vec![ColourCapability::rgb8(), ColourCapability::cct(16)],
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: u8::MAX.into(),
            scope: CapabilityScope::Device,
        },
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![
                HardwareEffectDescriptor {
                    id: HardwareEffectId::new("breathe"),
                    name: "Breathe".to_owned(),
                    parameters: vec![colour(), duration()],
                },
                HardwareEffectDescriptor {
                    id: HardwareEffectId::new("pulse"),
                    name: "Pulse".to_owned(),
                    parameters: vec![colour(), duration()],
                },
                HardwareEffectDescriptor {
                    id: HardwareEffectId::new("strobe"),
                    name: "Strobe".to_owned(),
                    parameters: vec![colour(), duration()],
                },
            ],
            scope: CapabilityScope::Device,
            concurrent_with_streaming: false,
        }),
        state_readback: StateReadbackCapability::Readable {
            facets: vec![
                ReadableFacet {
                    facet: StateFacetKind::Appearance,
                    fidelity: ReadbackFidelity::BestEffort,
                },
                ReadableFacet {
                    facet: StateFacetKind::Brightness,
                    fidelity: ReadbackFidelity::Exact,
                },
                ReadableFacet {
                    facet: StateFacetKind::Emission,
                    fidelity: ReadbackFidelity::Exact,
                },
                ReadableFacet {
                    facet: StateFacetKind::PhysicalPower,
                    fidelity: ReadbackFidelity::Exact,
                },
            ],
            read_disturbs_output: false,
            notifies_external_changes: false,
        },
        emission: true,
        physical_power: Some(PhysicalPowerCapability {
            scope: CapabilityScope::Device,
        }),
        power_domain: Some(PowerDomainRef::Device),
        ..CapabilitySet::default()
    }
}

fn linear_capabilities(scope: CapabilityScope) -> CapabilitySet {
    let mut capabilities = bulb_capabilities();
    capabilities.brightness = BrightnessCapability::Independent {
        bits: 8,
        maximum: u8::MAX.into(),
        scope,
    };
    capabilities.physical_power =
        (scope == CapabilityScope::Device).then_some(PhysicalPowerCapability {
            scope: CapabilityScope::Device,
        });
    if scope != CapabilityScope::Device {
        let StateReadbackCapability::Readable { facets, .. } = &mut capabilities.state_readback
        else {
            unreachable!("bulb capabilities always advertise readback");
        };
        facets.retain(|facet| facet.facet != StateFacetKind::PhysicalPower);
    }
    let effects = capabilities
        .hardware_effects
        .get_or_insert(HardwareEffectsCapability {
            effects: Vec::new(),
            scope,
            concurrent_with_streaming: false,
        });
    effects.scope = scope;
    effects.effects.extend([
        HardwareEffectDescriptor {
            id: HardwareEffectId::new("scanner"),
            name: "Scanner".to_owned(),
            parameters: vec![
                EffectParameter::Colour {
                    minimum_colours: 1,
                    maximum_colours: 1,
                },
                EffectParameter::Duration {
                    milliseconds: DiscreteRange::new(100, 60_000, 1),
                },
            ],
        },
        HardwareEffectDescriptor {
            id: HardwareEffectId::new("spectrum"),
            name: "Spectrum".to_owned(),
            parameters: vec![EffectParameter::Duration {
                milliseconds: DiscreteRange::new(100, 60_000, 1),
            }],
        },
        HardwareEffectDescriptor {
            id: HardwareEffectId::new("rainbow"),
            name: "Rainbow".to_owned(),
            parameters: vec![EffectParameter::Duration {
                milliseconds: DiscreteRange::new(100, 60_000, 1),
            }],
        },
    ]);
    capabilities
}

fn zone_capabilities() -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8(), ColourCapability::cct(16)],
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: u8::MAX.into(),
            scope: CapabilityScope::Element,
        },
        state_readback: StateReadbackCapability::Readable {
            facets: vec![
                ReadableFacet {
                    facet: StateFacetKind::Appearance,
                    fidelity: ReadbackFidelity::BestEffort,
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
        emission: true,
        power_domain: Some(PowerDomainRef::Device),
        ..CapabilitySet::default()
    }
}

fn rgb_from_colour(colour: &Colour) -> Result<Rgb, String> {
    colour.try_as_rgb().map_err(|error| {
        let channel = match error {
            Rgb8Error::MissingChannel(channel) | Rgb8Error::ChannelOutOfRange { channel, .. } => {
                channel
            }
            Rgb8Error::NonAdditive(_) => ColourChannel::Red,
        };
        format!("missing {} channel", channel.as_str())
    })
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    reason = "Colour conversion clamps normalized floats into the LIFX integer wire format."
)]
fn rgb_to_hsbk(rgb: Rgb) -> Hsbk {
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
    Hsbk {
        hue: ((hue / 360.0) * 65_536.0).round() as u16,
        saturation: (saturation * f32::from(u16::MAX)).round() as u16,
        brightness: (maximum * f32::from(u16::MAX)).round() as u16,
        kelvin: DEFAULT_KELVIN,
    }
}

fn lock_sequences(
    sequences: &Mutex<HashMap<[u8; 8], u8>>,
) -> Result<MutexGuard<'_, HashMap<[u8; 8], u8>>, SequencePoisonError> {
    Ok(sequences.lock().expect("LIFX sequence lock poisoned"))
}

fn next_sequence(target: [u8; 8]) -> Result<u8, SequencePoisonError> {
    let mut sequences = lock_sequences(&runtime().sequences)?;
    let sequence = sequences.entry(target).or_default();
    let current = *sequence;
    *sequence = sequence.wrapping_add(1);
    Ok(current)
}

fn sequence_for_request(target: [u8; 8], persist: bool) -> Result<u8, SequencePoisonError> {
    if persist {
        next_sequence(target)
    } else {
        Ok(0)
    }
}

fn device_id(target: [u8; 8]) -> String {
    format!("lifx-{}", serial_string(target))
}

fn serial_string(target: [u8; 8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut serial = String::with_capacity(12);
    for byte in target.iter().take(6) {
        serial.push(char::from(HEX[usize::from(byte >> 4)]));
        serial.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    serial
}

fn source_identifier() -> u32 {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.subsec_nanos());
    let source = time ^ process::id().rotate_left(13) ^ 0x4c55_4d49;
    if source <= 1 { 0x4c49_4658 } else { source }
}

luminate_export_plugin! {
    plugin: Lifx,
    name: NAME,
    version: VERSION,
    priority: 100,
    recommended_reconciliation: Some(ReconciliationPolicy::Adopt),
    buses: BUSES,
    vendors: VENDORS,
    probe_hints: HINTS,
    settings: SETTINGS,
    start: native,
    rescan: native,
    batch: default,
    read_state: native,
    frame_upload: none,
    shm_frame: none,
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
