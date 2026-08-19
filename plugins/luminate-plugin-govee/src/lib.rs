// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Govee LAN plugin: power, brightness, RGB colour, CCT, H6022 matrix cells
//! and frame streaming, and firmware scene selection for Govee smart lights
//! over their LAN UDP API (no BLE, no cloud account).
//!
//! Discovery multicasts a `scan` request to `239.255.255.250:4001` and
//! listens on the fixed reply port `:4002` (Govee unicasts scan replies
//! there, not to an ephemeral sender port, which is why this plugin binds
//! that port with `SO_REUSEADDR` via `socket2` rather than using an
//! ephemeral socket the way `luminate-plugin-lifx`/`luminate-plugin-wled`
//! do). Control commands go to `<device-ip>:4003`.
//!
//! H6022 matrix images and firmware scenes go through [`ptreal`]'s raw-frame
//! `ptReal` passthrough, since Govee has no documented LAN command for either.
//! The H6022 accepts client-timed effects as full, row-major 12×11 matrix
//! frames. BLE remains out of scope.

mod h6022_matrix;
mod protocol;
mod ptreal;
mod sku;
mod transport;

use std::collections::{HashMap, HashSet};
use std::ffi::CStr;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol as SocketProtocol, Socket, Type};

use luminate_core::capability::{
    BrightnessCapability, BufferingMode, CapabilityScope, CapabilitySet, CctEmulation,
    ColourCapability, ColourChannel, ColourEncoding, FrameUpdateMode, FrameUploadCapability,
    HardwareEffectDescriptor, HardwareEffectId, HardwareEffectsCapability, PersistenceCapability,
    PersistenceRequirement, PhysicalPowerCapability, PowerDomainRef, StateReadbackCapability,
};
use luminate_core::colour::{Colour, Rgb8Error};
use luminate_core::control::ReconciliationPolicy;
use luminate_core::device::{DeviceCategory, device_category};
use luminate_core::effect::Effect;
use luminate_core::element::{ElementGeometry, ElementKind};
use luminate_core::frame::{FrameEnvelope, FramePayload};
use luminate_core::rgb::{Rgb, kelvin_to_rgb};
use luminate_core::surface::SurfaceKind;
use luminate_plugin_api::sdk::{
    CompleteShadow, CompleteShadowError, DiscoveryPacer, DynamicDeviceRegistry,
    FrameStreamingPlugin, LuminatePlugin, RegistryExpiry, RegistryPoisonError, RescanPlugin,
    StartPlugin, stage_frame_updates,
};
use luminate_plugin_api::{
    ClaimExclusivity, DeviceDescriptor, ElementDescriptor, HardwareBus, HardwareClaim, PluginBus,
    PluginError, PluginRequestContext, PluginTarget, PluginUpdate, PluginUpdateOperation,
    PluginVendorId, ProbeOutcome, RescanReason, SurfaceDescriptor, luminate_export_plugin,
};
use serde_json::Value;
use thiserror::Error;

use sku::SkuProfile;
use transport::{Transport, UdpTransport};

const NAME: &CStr = c"luminate-plugin-govee";
const VERSION: &CStr = c"0.1.0";

const DISCOVERY_MULTICAST_ADDR: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
const DISCOVERY_SEND_PORT: u16 = 4001;
const DISCOVERY_LISTEN_PORT: u16 = 4002;
const CONTROL_PORT: u16 = 4003;
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(10);
const DISCOVERY_WINDOW: Duration = Duration::from_millis(1200);
const DEVICE_EXPIRY: Duration = Duration::from_secs(75);
const MAX_DISCOVERED_DEVICES: usize = 256;
const MAX_DISCOVERED_DEVICES_PER_SOURCE: usize = 16;
const H6022_MAX_FRAME_RATE_HZ: u16 = 15;

const MATRIX_SURFACE_ID: &str = "matrix";

static BUSES: &[PluginBus] = &[PluginBus::Network];
static VENDORS: &[PluginVendorId] = &[];
static HINTS: &[luminate_plugin_api::PluginProbeHint] = &[];

#[derive(Debug, Clone)]
struct DiscoveredDevice {
    id: String,
    control_address: SocketAddr,
    sku: String,
    profile: &'static SkuProfile,
    ble_version: String,
}

type TopologyFingerprint = (String, SocketAddr);

struct Runtime {
    devices: DynamicDeviceRegistry<DiscoveredDevice, TopologyFingerprint>,
    matrix_shadows: Mutex<HashMap<String, Vec<Rgb>>>,
    /// Lets a rescan cut short the discovery thread's inter-cycle wait.
    discovery: DiscoveryPacer,
}

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime {
        devices: DynamicDeviceRegistry::new(
            MAX_DISCOVERED_DEVICES,
            DEVICE_EXPIRY,
            RegistryExpiry::AtOrAfter,
        ),
        matrix_shadows: Mutex::new(HashMap::new()),
        discovery: DiscoveryPacer::new(),
    })
}

fn govee_init() {
    match thread::Builder::new()
        .name("luminate-govee-discovery".to_owned())
        .spawn(discovery_loop)
    {
        Ok(_thread) => tracing::info!("Govee discovery thread started"),
        Err(error) => tracing::error!(error = %error, "failed to start Govee discovery thread"),
    }
}

struct Govee;

impl LuminatePlugin for Govee {
    fn new() -> Result<Self, PluginError> {
        let _ = runtime();
        Ok(Self)
    }

    fn probe(&self) -> ProbeOutcome {
        // A LAN device may appear after daemon startup; loading must not
        // depend on one answering the first discovery scan.
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
        apply_update(update, &UdpTransport)
            .map_err(classify_govee_error)
            .inspect_err(|error| {
                tracing::warn!(error = %error, "Govee update failed");
            })
    }
}

impl FrameStreamingPlugin for Govee {
    fn upload_frame(
        &self,
        _context: &PluginRequestContext,
        target: &PluginTarget,
        envelope: &FrameEnvelope,
    ) -> Result<(), PluginError> {
        apply_frame(target, envelope, &UdpTransport)
            .map_err(classify_govee_error)
            .inspect_err(|error| {
                tracing::warn!(error = %error, "Govee frame upload failed");
            })
    }
}

impl StartPlugin for Govee {
    fn start(&self) {
        govee_init();
    }
}

impl RescanPlugin for Govee {
    fn rescan(&self, reason: RescanReason) {
        // Refresh before topology is pulled again. Registry expiry removes
        // absent devices without making every device briefly disappear.
        tracing::debug!(reason = ?reason, "Govee rescan requested; waking discovery");
        runtime().discovery.wake();
    }
}

#[derive(Debug, Error)]
enum GoveeError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Internal(String),
}

fn classify_govee_error(error: GoveeError) -> PluginError {
    match error {
        GoveeError::Unavailable(message) => PluginError::Unavailable(message),
        GoveeError::Unsupported(message) => PluginError::Unsupported(message),
        GoveeError::Invalid(message) => PluginError::InvalidArgument(message),
        GoveeError::Internal(message) => PluginError::Internal(message),
        // Fire-and-forget sends (see `transport`) rarely time out; most
        // `Unavailable`s come from a registry miss, handled above. A
        // transport-level timeout most likely means the destination itself
        // is unreachable, not that an ack was merely slow, so it is folded
        // into `Unavailable` too rather than left as a generic `Io` error.
        GoveeError::Io(message) => {
            if message.contains("timed out") || message.contains("deadline expired") {
                PluginError::Unavailable(message)
            } else {
                PluginError::Io(message)
            }
        }
    }
}

fn apply_update(update: &PluginUpdate, transport: &impl Transport) -> Result<(), GoveeError> {
    match &update.target {
        PluginTarget::Device { device: device_id } => {
            let device = lookup_device(device_id)?;
            apply_device_operation(&device, &update.operation, transport)
        }
        PluginTarget::Element {
            device: device_id,
            surface,
            element,
        } => {
            let device = lookup_device(device_id)?;
            apply_matrix_operation(&device, surface, element, &update.operation, transport)
        }
        PluginTarget::Surface {
            device: device_id,
            surface,
        } => {
            let device = lookup_device(device_id)?;
            if surface != MATRIX_SURFACE_ID || device.profile.matrix.is_none() {
                return Err(GoveeError::Invalid(format!(
                    "unknown Govee surface: {surface}"
                )));
            }
            apply_device_operation(&device, &update.operation, transport)
        }
        PluginTarget::Group { .. } => Err(GoveeError::Invalid(
            "Govee does not support group-scoped targets".to_owned(),
        )),
    }
}

fn lookup_device(device_id: &str) -> Result<DiscoveredDevice, GoveeError> {
    runtime()
        .devices
        .get(device_id)
        .map_err(|error| GoveeError::Internal(error.to_string()))?
        .ok_or_else(|| GoveeError::Unavailable(format!("Govee device unavailable: {device_id}")))
}

fn apply_device_operation(
    device: &DiscoveredDevice,
    operation: &PluginUpdateOperation,
    transport: &impl Transport,
) -> Result<(), GoveeError> {
    if device.profile.matrix.is_some()
        && matches!(
            operation,
            PluginUpdateOperation::SetEffect { .. } | PluginUpdateOperation::Clear
        )
    {
        runtime()
            .matrix_shadows
            .lock()
            .expect("Govee matrix shadow lock poisoned")
            .remove(&device.id);
    }

    match operation {
        PluginUpdateOperation::SetBrightness { value } => set_brightness(device, *value, transport),
        PluginUpdateOperation::SetEffect { effect } => apply_effect(device, effect, transport),
        PluginUpdateOperation::Clear => send_turn(device, false, transport),
        PluginUpdateOperation::SaveCurrent => {
            // Govee LAN lights persist their last-set state in firmware as a
            // side effect of ordinary operation (matching
            // `PersistenceRequirement::Required` below); there is no
            // separate commit command, so this is a durable no-op rather
            // than an unsupported operation. H6022-specific confirmation is
            // still pending real-hardware verification.
            tracing::info!(device = %device.id, "Govee save-current accepted as a no-op: LAN devices persist state in firmware already");
            Ok(())
        }
        PluginUpdateOperation::SetAppearanceSlots { .. } => Err(GoveeError::Unsupported(
            "Govee targets do not advertise appearance slots".to_owned(),
        )),
    }?;

    update_matrix_shadow_after_device_operation(device, operation)
}

fn update_matrix_shadow_after_device_operation(
    device: &DiscoveredDevice,
    operation: &PluginUpdateOperation,
) -> Result<(), GoveeError> {
    if device.profile.matrix.is_none() {
        return Ok(());
    }

    let replacement = match operation {
        PluginUpdateOperation::SetEffect {
            effect: Effect::Static { colour },
        } if colour.encoding() == ColourEncoding::Additive => {
            Some(vec![rgb_from_colour(colour)?; h6022_matrix::CELL_COUNT])
        }
        PluginUpdateOperation::SetEffect { .. }
        | PluginUpdateOperation::Clear
        | PluginUpdateOperation::SetBrightness { value: 0 } => None,
        PluginUpdateOperation::SetBrightness { .. }
        | PluginUpdateOperation::SetAppearanceSlots { .. }
        | PluginUpdateOperation::SaveCurrent => {
            return Ok(());
        }
    };
    let mut shadows = runtime()
        .matrix_shadows
        .lock()
        .expect("Govee matrix shadow lock poisoned");
    if let Some(framebuffer) = replacement {
        shadows.insert(device.id.clone(), framebuffer);
    } else {
        shadows.remove(&device.id);
    }
    Ok(())
}

fn apply_frame(
    target: &PluginTarget,
    envelope: &FrameEnvelope,
    transport: &impl Transport,
) -> Result<(), GoveeError> {
    let PluginTarget::Surface {
        device: device_id,
        surface,
    } = target
    else {
        return Err(GoveeError::Unsupported(
            "Govee frame streaming is only supported at matrix-surface scope".to_owned(),
        ));
    };
    if surface != MATRIX_SURFACE_ID {
        return Err(GoveeError::Invalid(format!(
            "unknown Govee surface: {surface}"
        )));
    }

    let device = lookup_device(device_id)?;
    let Some((rows, cols)) = device.profile.matrix else {
        return Err(GoveeError::Unsupported(format!(
            "{} does not support matrix frame streaming",
            device.profile.model_name
        )));
    };
    let FramePayload::Full(pixels) = &envelope.payload else {
        return Err(GoveeError::Unsupported(
            "Govee matrix frame streaming only supports full frames".to_owned(),
        ));
    };
    let expected = usize::from(rows) * usize::from(cols);
    if pixels.len() != expected {
        return Err(GoveeError::Invalid(format!(
            "expected {expected} pixels for a {cols}×{rows} Govee matrix frame, got {}",
            pixels.len()
        )));
    }

    let framebuffer = pixels
        .iter()
        .map(rgb_from_colour)
        .collect::<Result<Vec<_>, _>>()?;
    let frames = h6022_matrix::encode(&framebuffer).map_err(GoveeError::Invalid)?;
    runtime()
        .matrix_shadows
        .lock()
        .expect("Govee matrix shadow lock poisoned")
        .remove(&device.id);
    send_json(&device, &ptreal::ptreal_command(&frames), transport)?;
    runtime()
        .matrix_shadows
        .lock()
        .expect("Govee matrix shadow lock poisoned")
        .insert(device.id, framebuffer);
    Ok(())
}

fn apply_matrix_operation(
    device: &DiscoveredDevice,
    surface: &str,
    element: &str,
    operation: &PluginUpdateOperation,
    transport: &impl Transport,
) -> Result<(), GoveeError> {
    if surface != MATRIX_SURFACE_ID {
        return Err(GoveeError::Invalid(format!(
            "unknown Govee surface: {surface}"
        )));
    }
    let index = matrix_cell_index(device.profile, element)?;
    match operation {
        PluginUpdateOperation::SetEffect {
            effect: Effect::Static { colour },
        } => {
            let rgb = rgb_from_colour(colour)?;
            let mut shadows = runtime()
                .matrix_shadows
                .lock()
                .expect("Govee matrix shadow lock poisoned");
            let next = stage_frame_updates(
                &shadows.get(&device.id).map_or(
                    CompleteShadow::Unknown,
                    |frame| CompleteShadow::Complete(frame.as_slice()),
                ),
                h6022_matrix::CELL_COUNT,
                [(index, rgb)],
            )
            .map_err(|error| match error {
                CompleteShadowError::Unknown => GoveeError::Unsupported(
                    "Govee matrix-cell updates require a complete frame written in this plugin session"
                        .to_owned(),
                ),
                CompleteShadowError::WrongLength => {
                    GoveeError::Internal("Govee matrix shadow has the wrong size".to_owned())
                }
                CompleteShadowError::UnknownElement => GoveeError::Internal(format!(
                    "H6022 matrix cell index {index} is outside the shadow framebuffer"
                )),
            })?;
            let frames = h6022_matrix::encode(&next).map_err(GoveeError::Invalid)?;
            shadows.remove(&device.id);
            send_json(device, &ptreal::ptreal_command(&frames), transport)?;
            shadows.insert(device.id.clone(), next);
            Ok(())
        }
        PluginUpdateOperation::SetBrightness { .. }
        | PluginUpdateOperation::SetAppearanceSlots { .. }
        | PluginUpdateOperation::SetEffect { .. }
        | PluginUpdateOperation::Clear
        | PluginUpdateOperation::SaveCurrent => Err(GoveeError::Unsupported(
            "Govee matrix cells only support the Static effect".to_owned(),
        )),
    }
}

fn matrix_cell_index(profile: &SkuProfile, element: &str) -> Result<usize, GoveeError> {
    let (rows, cols) = profile.matrix.ok_or_else(|| {
        GoveeError::Invalid(format!(
            "{} does not advertise a matrix",
            profile.model_name
        ))
    })?;
    let coordinates = element
        .strip_prefix("cell-r")
        .and_then(|suffix| suffix.split_once("-c"))
        .and_then(|(row, col)| Some((row.parse::<u16>().ok()?, col.parse::<u16>().ok()?)));
    let (row, col) = coordinates
        .filter(|(row, col)| *row < rows && *col < cols)
        .ok_or_else(|| {
            GoveeError::Invalid(format!(
                "unrecognized or out-of-range Govee matrix cell id: {element}"
            ))
        })?;
    Ok(usize::from(row) * usize::from(cols) + usize::from(col))
}

fn set_brightness(
    device: &DiscoveredDevice,
    value: u32,
    transport: &impl Transport,
) -> Result<(), GoveeError> {
    // Govee's documented range is 1..=100 with no representation for 0;
    // treat a request for zero brightness as "turn off" instead, mirroring
    // how luminate-plugin-wled maps `bri: 0` onto `on: false`.
    if value == 0 {
        return send_turn(device, false, transport);
    }
    let percent = u8::try_from(value)
        .ok()
        .filter(|percent| (1..=100).contains(percent))
        .ok_or_else(|| {
            GoveeError::Invalid(format!("Govee brightness {value} is outside 0..=100"))
        })?;
    let command = protocol::brightness_command(percent).map_err(GoveeError::Invalid)?;
    send_json(device, &command, transport)
}

fn set_colour(
    device: &DiscoveredDevice,
    colour: &Colour,
    transport: &impl Transport,
) -> Result<(), GoveeError> {
    let command = match colour.encoding() {
        ColourEncoding::Additive => protocol::colorwc_rgb_command(rgb_from_colour(colour)?),
        ColourEncoding::Cct if device.profile.supports_cct => {
            let kelvin = kelvin_from_colour(colour)?;
            match device.profile.native_cct_range {
                Some((minimum, maximum)) if !(minimum..=maximum).contains(&kelvin) => {
                    // The H6022's native white-channel control is limited to
                    // 2700-6500 K. Preserve the requested CCT by using the
                    // same RGB approximation as daemon-side CCT emulation.
                    protocol::colorwc_rgb_command(kelvin_to_rgb(kelvin))
                }
                _ => protocol::colorwc_cct_command(kelvin),
            }
        }
        ColourEncoding::Cct => {
            return Err(GoveeError::Unsupported(format!(
                "{} does not advertise CCT support",
                device.profile.model_name
            )));
        }
        ColourEncoding::Hsv | ColourEncoding::Hsl | ColourEncoding::Monochrome => {
            return Err(GoveeError::Invalid(format!(
                "unsupported colour encoding {:?}",
                colour.encoding()
            )));
        }
    };
    send_json(device, &command, transport)?;
    send_turn(device, true, transport)
}

fn apply_effect(
    device: &DiscoveredDevice,
    effect: &Effect,
    transport: &impl Transport,
) -> Result<(), GoveeError> {
    match effect {
        Effect::Off => send_turn(device, false, transport),
        Effect::Static { colour } => set_colour(device, colour, transport),
        Effect::Hardware { id, .. } => apply_scene(device, id, transport),
        Effect::Breathe { .. }
        | Effect::Pulse { .. }
        | Effect::Strobe { .. }
        | Effect::Scanner { .. }
        | Effect::Morph { .. }
        | Effect::Spectrum { .. }
        | Effect::Rainbow { .. } => Err(GoveeError::Unsupported(format!(
            "{} advertises no client-timed hardware effects",
            device.profile.model_name
        ))),
    }
}

fn apply_scene(
    device: &DiscoveredDevice,
    id: &HardwareEffectId,
    transport: &impl Transport,
) -> Result<(), GoveeError> {
    let code = device
        .profile
        .scenes
        .iter()
        .find(|(_code, name)| scene_slug(name) == id.as_str())
        .map(|(code, _name)| *code)
        .ok_or_else(|| GoveeError::Invalid(format!("unknown Govee scene: {}", id.as_str())))?;
    let frame = ptreal::scene_frame(code).map_err(GoveeError::Invalid)?;
    send_json(device, &ptreal::ptreal_command(&[frame]), transport)?;
    send_turn(device, true, transport)
}

/// Converts the profile's simple scene names into stable effect IDs.
fn scene_slug(name: &str) -> String {
    name.to_ascii_lowercase().replace(' ', "-")
}

fn send_turn(
    device: &DiscoveredDevice,
    on: bool,
    transport: &impl Transport,
) -> Result<(), GoveeError> {
    send_json(device, &protocol::turn_command(on), transport)
}

fn send_json(
    device: &DiscoveredDevice,
    command: &Value,
    transport: &impl Transport,
) -> Result<(), GoveeError> {
    let body =
        serde_json::to_vec(command).map_err(|error| GoveeError::Invalid(error.to_string()))?;
    transport
        .send(device.control_address, &body)
        .map_err(|error| GoveeError::Io(error.to_string()))
}

fn rgb_from_colour(colour: &Colour) -> Result<Rgb, GoveeError> {
    colour.try_as_rgb().map_err(|error| {
        let channel = match error {
            Rgb8Error::MissingChannel(channel) | Rgb8Error::ChannelOutOfRange { channel, .. } => {
                channel
            }
            Rgb8Error::NonAdditive(_) => ColourChannel::Red,
        };
        GoveeError::Invalid(format!("missing {} channel", channel.as_str()))
    })
}

fn kelvin_from_colour(colour: &Colour) -> Result<u32, GoveeError> {
    colour
        .channel(ColourChannel::Temperature)
        .ok_or_else(|| GoveeError::Invalid("missing temperature channel".to_owned()))
}

fn appearance_capabilities(profile: &SkuProfile, scope: CapabilityScope) -> CapabilitySet {
    let mut colour = vec![ColourCapability::rgb8()];
    if profile.supports_cct {
        colour.push(ColourCapability::cct(16));
    }
    let hardware_effects = profile.supports_scenes.then(|| HardwareEffectsCapability {
        scope,
        ..scene_effects_capability(profile)
    });
    CapabilitySet {
        colour,
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            // Govee's documented range is 1..=100, not an 8-bit 0..=255
            // scale like LIFX/WLED.
            bits: 7,
            maximum: 100,
            scope,
        },
        persistence: PersistenceCapability::CurrentState {
            requirement: PersistenceRequirement::Required,
            explicit_commit: false,
            readback: false,
        },
        state_readback: StateReadbackCapability::None,
        emission: true,
        off_is_wear_safe: true,
        physical_power: (scope == CapabilityScope::Device).then_some(PhysicalPowerCapability {
            scope: CapabilityScope::Device,
        }),
        power_domain: Some(PowerDomainRef::Device),
        hardware_effects,
        ..CapabilitySet::default()
    }
}

fn device_capabilities(profile: &SkuProfile) -> CapabilitySet {
    appearance_capabilities(profile, CapabilityScope::Device)
}

fn scene_effects_capability(profile: &SkuProfile) -> HardwareEffectsCapability {
    HardwareEffectsCapability {
        effects: profile
            .scenes
            .iter()
            .map(|(_code, name)| HardwareEffectDescriptor {
                id: HardwareEffectId::new(scene_slug(name)),
                name: (*name).to_owned(),
                parameters: Vec::new(),
            })
            .collect(),
        scope: CapabilityScope::Device,
        concurrent_with_streaming: false,
    }
}

fn matrix_surface(profile: &SkuProfile) -> Option<SurfaceDescriptor> {
    let (rows, cols) = profile.matrix?;
    let mut capabilities = appearance_capabilities(profile, CapabilityScope::Surface);
    capabilities.frame_upload = Some(FrameUploadCapability {
        scope: CapabilityScope::Surface,
        update_mode: FrameUpdateMode::FullFrameOnly,
        max_rate_hz: Some(H6022_MAX_FRAME_RATE_HZ),
        atomic: true,
        buffering: BufferingMode::Immediate,
        shm: None,
    });
    Some(SurfaceDescriptor {
        id: MATRIX_SURFACE_ID.to_owned(),
        name: "Matrix".to_owned(),
        kind: SurfaceKind::Matrix { rows, cols },
        physical_tags: vec!["layout:wrapped-horizontal".to_owned()],
        elements: (0..rows)
            .flat_map(|row| (0..cols).map(move |col| matrix_element(row, col)))
            .collect(),
        capabilities,
        notes: vec!["Full streamed frames use row-major order across the 12×11 matrix.".to_owned()],
        warnings: Vec::new(),
    })
}

fn matrix_element(row: u16, col: u16) -> ElementDescriptor {
    ElementDescriptor {
        id: format!("cell-r{row}-c{col}"),
        name: Some(format!("Cell {row}.{col}")),
        kind: ElementKind::Led,
        geometry: Some(ElementGeometry::MatrixCell { row, col }),
        physical_tags: Vec::new(),
        capabilities: CapabilitySet {
            colour: vec![ColourCapability::rgb8()],
            emission: true,
            ..CapabilitySet::default()
        },
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

fn device_descriptor(device: &DiscoveredDevice) -> DeviceDescriptor {
    let mut notes = vec![format!(
        "LAN device at {}; SKU {}.",
        device.control_address, device.sku
    )];
    if !device.ble_version.is_empty() {
        notes.push(format!(
            "Device-reported BLE firmware version {}.",
            device.ble_version
        ));
    }

    let mut warnings = vec![
        "Govee's LAN commands have no application-level acknowledgement; reachability is judged only by whether the device answered a recent discovery scan.".to_owned(),
    ];
    if !sku::is_known(&device.sku) {
        warnings.push(format!(
            "Unrecognized Govee SKU {}; using a conservative capability profile with no matrix, scenes, or hardware effects.",
            device.sku
        ));
    }
    if device.profile.supports_scenes {
        warnings.push(format!(
            "The firmware scene table covers only {} confirmed-named scenes; additional unnamed scene codes are known to exist for this device family.",
            device.profile.scenes.len()
        ));
    }

    DeviceDescriptor {
        id: device.id.clone(),
        name: format!("Govee {} ({})", device.profile.model_name, device.sku),
        vendor: Some("Govee".to_owned()),
        model: Some(device.profile.model_name.to_owned()),
        surfaces: matrix_surface(device.profile).into_iter().collect(),
        groups: Vec::new(),
        capabilities: device_capabilities(device.profile),
        claims: vec![HardwareClaim {
            bus: HardwareBus::Network,
            physical_identity: device.id.clone(),
            control_domain: "light-output".to_owned(),
            exclusivity: ClaimExclusivity::Exclusive,
        }],
        category: Some(DeviceCategory::new(device_category::LED_STRIP)),
        physical_tags: device
            .profile
            .matrix
            .is_some()
            .then(|| "shape:cylinder".to_owned())
            .into_iter()
            .collect(),
        host_attached: false,
        notes,
        warnings,
    }
}

#[derive(Debug, Error)]
enum GoveeDiscoveryError {
    #[error(transparent)]
    Io(io::Error),
    #[error(transparent)]
    Registry(RegistryPoisonError),
}

impl From<io::Error> for GoveeDiscoveryError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<RegistryPoisonError> for GoveeDiscoveryError {
    fn from(error: RegistryPoisonError) -> Self {
        Self::Registry(error)
    }
}

fn discovery_loop() {
    loop {
        match discovery_cycle() {
            Ok(()) => {}
            Err(error @ GoveeDiscoveryError::Io(_)) => {
                tracing::warn!(error = %error, "Govee discovery cycle failed");
            }
            Err(error @ GoveeDiscoveryError::Registry(_)) => {
                tracing::error!(error = %error, "Govee discovery stopped after registry invariant failure");
                return;
            }
        }
        runtime().discovery.wait(DISCOVERY_INTERVAL);
    }
}

fn discovery_cycle() -> Result<(), GoveeDiscoveryError> {
    // Govee unicasts scan replies to the fixed port 4002 rather than the
    // sender's ephemeral port, so (unlike LIFX/WLED) this socket must be
    // bound up front and kept open for the whole collection window.
    // SO_REUSEADDR lets this coexist with other Govee-aware software
    // (govee2mqtt, Home Assistant, ...) that might already be bound here.
    let listen_socket = bind_reuseaddr(DISCOVERY_LISTEN_PORT)?;

    let send_socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    let scan = serde_json::to_vec(&protocol::scan_command()).map_err(io::Error::other)?;
    send_socket.send_to(
        &scan,
        SocketAddrV4::new(DISCOVERY_MULTICAST_ADDR, DISCOVERY_SEND_PORT),
    )?;

    let deadline = Instant::now() + DISCOVERY_WINDOW;
    let mut discovered: HashMap<String, DiscoveredDevice> = HashMap::new();
    let mut ambiguous = HashSet::new();
    let mut buffer = [0_u8; 2048];
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        listen_socket.set_read_timeout(Some(remaining))?;
        match listen_socket.recv_from(&mut buffer) {
            Ok((length, sender)) => {
                if let Some(payload) = buffer.get(..length)
                    && let Some(device) = parse_candidate(payload, sender)
                {
                    record_candidate(&mut discovered, &mut ambiguous, device);
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

    let refresh = runtime().devices.refresh_with_source_limit(
        Instant::now(),
        discovered.into_values().map(|device| {
            let id = device.id.clone();
            let fingerprint = topology_fingerprint(&device);
            let source = Some(device.control_address.ip());
            (id, device, fingerprint, source)
        }),
        MAX_DISCOVERED_DEVICES_PER_SOURCE,
    )?;
    if refresh.expired > 0 {
        prune_matrix_shadows()?;
    }
    if refresh.topology_changed {
        runtime()
            .matrix_shadows
            .lock()
            .expect("Govee matrix shadow lock poisoned")
            .clear();
        tracing::info!("Govee topology changed");
    }
    Ok(())
}

fn prune_matrix_shadows() -> Result<(), GoveeDiscoveryError> {
    let active_ids: HashSet<String> = runtime()
        .devices
        .snapshot()?
        .into_iter()
        .map(|device| device.id)
        .collect();
    runtime()
        .matrix_shadows
        .lock()
        .expect("Govee matrix shadow lock poisoned")
        .retain(|device_id, _framebuffer| active_ids.contains(device_id));
    Ok(())
}

fn bind_reuseaddr(port: u16) -> io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(SocketProtocol::UDP))?;
    socket.set_reuse_address(true)?;
    let address = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
    socket.bind(&address.into())?;
    Ok(socket.into())
}

fn parse_candidate(payload: &[u8], sender: SocketAddr) -> Option<DiscoveredDevice> {
    let data = protocol::parse_scan_reply(payload)?;
    let profile = sku::profile_for(&data.sku);
    Some(DiscoveredDevice {
        id: device_id(&data.device)?,
        control_address: SocketAddr::new(sender.ip(), CONTROL_PORT),
        sku: data.sku,
        profile,
        ble_version: data.ble_version_soft,
    })
}

fn device_id(raw: &str) -> Option<String> {
    let mut normalized = String::with_capacity(12);
    for character in raw.chars() {
        if character.is_ascii_hexdigit() {
            normalized.push(character.to_ascii_lowercase());
        } else if !matches!(character, ':' | '-') {
            return None;
        }
    }
    (normalized.len() == 12).then(|| format!("govee-{normalized}"))
}

fn record_candidate(
    discovered: &mut HashMap<String, DiscoveredDevice>,
    ambiguous: &mut HashSet<String>,
    candidate: DiscoveredDevice,
) {
    if ambiguous.contains(&candidate.id) {
        return;
    }
    if let Some(existing) = discovered.get(&candidate.id)
        && existing.control_address.ip() != candidate.control_address.ip()
    {
        tracing::warn!(
            device = %candidate.id,
            "omitting Govee identity reported by multiple network addresses"
        );
        discovered.remove(&candidate.id);
        ambiguous.insert(candidate.id);
        return;
    }
    discovered.insert(candidate.id.clone(), candidate);
}

fn topology_fingerprint(device: &DiscoveredDevice) -> TopologyFingerprint {
    (device.sku.clone(), device.control_address)
}

luminate_export_plugin! {
    plugin: Govee,
    name: NAME,
    version: VERSION,
    priority: 80,
    // Govee reports only its plain colour register, not the scene or matrix
    // state that may actually be visible, so it cannot safely adopt state.
    recommended_reconciliation: Some(ReconciliationPolicy::Leave),
    buses: BUSES,
    vendors: VENDORS,
    probe_hints: HINTS,
    start: native,
    rescan: native,
    batch: default,
    read_state: none,
    frame_upload: native,
    shm_frame: none,
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
