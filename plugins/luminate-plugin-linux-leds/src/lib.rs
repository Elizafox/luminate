// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Linux LED-class keyboard-backlight plugin.

use std::collections::{BTreeMap, HashSet};
use std::env;
use std::ffi::CStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use thiserror::Error;

use luminate_core::capability::{
    BrightnessCapability, CapabilityScope, CapabilitySet, ColourCapability, ColourChannel,
    ReadableFacet, ReadbackFidelity, StateReadbackCapability,
};
use luminate_core::colour::{Colour, ColourChannelValue};
use luminate_core::control::ReconciliationPolicy;
use luminate_core::device::{DeviceCategory, device_category};
use luminate_core::effect::Effect;
use luminate_core::element::ElementKind;
use luminate_core::group::GroupKind;
use luminate_core::state::{AppearanceState, EmissionState, FacetValue, StateFacetKind};
use luminate_core::surface::SurfaceKind;
use luminate_plugin_api::notification;
use luminate_plugin_api::sdk::{BatchPlugin, LuminatePlugin, ReadablePlugin, StartPlugin};
use luminate_plugin_api::{
    ClaimExclusivity, DeviceDescriptor, ElementDescriptor, GroupDescriptor, GroupMemberDescriptor,
    HardwareBus, HardwareClaim, PluginBus, PluginError, PluginFacetObservation, PluginProbeHint,
    PluginReadError, PluginReadRequest, PluginRequestContext, PluginStateSnapshot, PluginTarget,
    PluginUpdate, PluginUpdateOperation, PluginVendorId, ProbeOutcome, SurfaceDescriptor,
    luminate_export_plugin,
};

const NAME: &CStr = c"luminate-plugin-linux-leds";
const VERSION: &CStr = c"0.1.0";
const DEFAULT_SYSFS_ROOT: &str = "/sys/class/leds";
const SYSFS_ROOT_ENV: &str = "LUMINATE_LED_SYSFS_ROOT";
const SURFACE_ID: &str = "backlight";
const ALL_ZONES_GROUP_ID: &str = "all-zones";
const POLL_INTERVAL: Duration = Duration::from_secs(2);

static BUSES: &[PluginBus] = &[PluginBus::Platform];
static VENDORS: &[PluginVendorId] = &[];
static HINTS: &[PluginProbeHint] = &[];

#[derive(Debug, Clone, PartialEq, Eq)]
struct Led {
    sysfs_name: String,
    path: PathBuf,
    device_key: String,
    stable_identity: String,
    zone: Option<String>,
    maximum: u32,
    rgb: Option<RgbAttributes>,
    trigger: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RgbAttributes {
    indices: Vec<String>,
    maxima: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Keyboard {
    key: String,
    stable_identity: String,
    leds: Vec<Led>,
}

#[derive(Debug, Error)]
enum ApplyError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Io(String),
}

impl From<ApplyError> for PluginError {
    fn from(error: ApplyError) -> Self {
        match error {
            ApplyError::Invalid(message) => Self::InvalidArgument(message),
            ApplyError::Unsupported(message) => Self::Unsupported(message),
            ApplyError::Io(message) => Self::Io(message),
        }
    }
}

/// Injectable sysfs writes for transport-error tests.
trait Transport {
    fn write(&self, path: &Path, contents: &str) -> Result<(), ApplyError>;
}

struct SysfsTransport;

impl Transport for SysfsTransport {
    fn write(&self, path: &Path, contents: &str) -> Result<(), ApplyError> {
        fs::write(path, contents)
            .map_err(|error| ApplyError::Io(format!("cannot write {}: {error}", path.display())))
    }
}

struct LinuxLeds {
    root: PathBuf,
}

impl LuminatePlugin for LinuxLeds {
    fn new() -> Result<Self, PluginError> {
        Ok(Self { root: sysfs_root() })
    }

    fn probe(&self) -> ProbeOutcome {
        match scan(&self.root) {
            Ok(leds) if !leds.is_empty() => {
                tracing::info!(count = leds.len(), "found Linux LED-class keyboard zones");
                ProbeOutcome::Ready
            }
            Ok(_) => {
                tracing::debug!(root = %self.root.display(), "no LED-class keyboard backlights found");
                ProbeOutcome::Dormant
            }
            Err(error) => {
                tracing::warn!(root = %self.root.display(), error = %error, "cannot scan LED class");
                ProbeOutcome::Unsupported
            }
        }
    }

    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
        let leds = scan(&self.root).map_err(|error| PluginError::Io(error.to_string()))?;
        Ok(keyboards(leds).iter().map(keyboard_descriptor).collect())
    }

    fn apply(
        &self,
        _context: &PluginRequestContext,
        update: &PluginUpdate,
    ) -> Result<(), PluginError> {
        let leds = scan(&self.root).map_err(|error| PluginError::Io(error.to_string()))?;
        apply_update(&SysfsTransport, &keyboards(leds), update).map_err(PluginError::from)
    }
}

impl StartPlugin for LinuxLeds {
    fn start(&self) {
        start_topology_poller(self.root.clone());
    }
}

impl BatchPlugin for LinuxLeds {
    fn apply_batch(
        &self,
        _context: &PluginRequestContext,
        updates: &[PluginUpdate],
    ) -> Vec<Result<(), PluginError>> {
        let keyboards = match scan(&self.root) {
            Ok(leds) => keyboards(leds),
            Err(error) => return vec![Err(PluginError::Io(error.to_string())); updates.len()],
        };
        updates
            .iter()
            .map(|update| {
                apply_update(&SysfsTransport, &keyboards, update).map_err(PluginError::from)
            })
            .collect()
    }
}

impl ReadablePlugin for LinuxLeds {
    fn read_state(
        &self,
        _context: &PluginRequestContext,
        request: &PluginReadRequest,
    ) -> Result<PluginStateSnapshot, PluginError> {
        Ok(read_snapshot(&self.root, request))
    }
}

fn start_topology_poller(root: PathBuf) {
    let _poller = thread::Builder::new()
        .name("luminate-linux-leds".to_owned())
        .spawn(move || topology_poller(&root));
}

fn topology_poller(root: &Path) -> ! {
    let mut previous = scan_fingerprint(root);
    loop {
        thread::sleep(POLL_INTERVAL);
        let current = scan_fingerprint(root);
        if current != previous {
            previous = current;
            tracing::info!("LED-class keyboard topology changed");
            notification::topology_changed();
        }
    }
}

fn scan_fingerprint(root: &Path) -> Vec<Keyboard> {
    keyboards(scan(root).unwrap_or_default())
}

fn sysfs_root() -> PathBuf {
    env::var_os(SYSFS_ROOT_ENV).map_or_else(|| PathBuf::from(DEFAULT_SYSFS_ROOT), PathBuf::from)
}

fn scan(root: &Path) -> io::Result<Vec<Led>> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut leds = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::debug!(error = %error, "cannot inspect LED-class entry");
                continue;
            }
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some((device_key, zone)) = parse_keyboard_name(&name) else {
            continue;
        };
        match inspect_led(entry.path(), name, device_key, zone) {
            Ok(led) => leds.push(led),
            Err(error) => {
                tracing::warn!(path = %entry.path().display(), error = %error, "ignoring invalid keyboard LED");
            }
        }
    }
    leds.sort_by(|left, right| left.sysfs_name.cmp(&right.sysfs_name));
    Ok(leds)
}

fn parse_keyboard_name(name: &str) -> Option<(String, Option<String>)> {
    if let Some(prefix) = name.strip_suffix(":kbd_backlight") {
        let device = prefix.trim_end_matches(':');
        return (!device.is_empty()).then(|| (device.to_owned(), None));
    }
    let (prefix, zone) = name.split_once(":kbd_zoned_backlight-")?;
    if zone.is_empty() {
        return None;
    }
    let (device, _colour) = prefix.rsplit_once(':')?;
    (!device.is_empty()).then(|| (device.to_owned(), Some(zone.to_owned())))
}

fn inspect_led(
    path: PathBuf,
    sysfs_name: String,
    device_key: String,
    zone: Option<String>,
) -> io::Result<Led> {
    let maximum = read_u32(&path.join("max_brightness"))?;
    if maximum == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "max_brightness is zero",
        ));
    }
    let rgb = inspect_rgb(&path, maximum)?;
    let stable_identity = stable_device_identity(&path, &device_key);
    let trigger = fs::read_to_string(path.join("trigger"))
        .ok()
        .and_then(|value| selected_trigger(&value));
    Ok(Led {
        sysfs_name,
        path,
        device_key,
        stable_identity,
        zone,
        maximum,
        rgb,
        trigger,
    })
}

fn inspect_rgb(path: &Path, fallback_maximum: u32) -> io::Result<Option<RgbAttributes>> {
    let index_path = path.join("multi_index");
    if !index_path.exists() {
        return Ok(None);
    }
    let indices: Vec<_> = fs::read_to_string(index_path)?
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();
    let unique: HashSet<_> = indices.iter().collect();
    if indices.len() != 3
        || unique.len() != 3
        || !["red", "green", "blue"]
            .iter()
            .all(|wanted| indices.iter().any(|value| value == wanted))
    {
        return Ok(None);
    }
    let maxima_path = path.join("multi_max_intensity");
    let maxima = if maxima_path.exists() {
        read_u32_list(&maxima_path)?
    } else {
        vec![fallback_maximum; indices.len()]
    };
    if maxima.len() != indices.len() || maxima.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid multi-channel maxima",
        ));
    }
    Ok(Some(RgbAttributes { indices, maxima }))
}

fn selected_trigger(value: &str) -> Option<String> {
    value.split_whitespace().find_map(|item| {
        item.strip_prefix('[')
            .and_then(|item| item.strip_suffix(']'))
            .map(str::to_owned)
    })
}

fn stable_device_identity(path: &Path, fallback: &str) -> String {
    let Ok(uevent) = fs::read_to_string(path.join("device/uevent")) else {
        return fallback.to_owned();
    };
    for key in ["UNIQ", "PHYS"] {
        if let Some(value) = uevent
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            && !value.is_empty()
        {
            return format!("{key}:{value}");
        }
    }
    let product = uevent
        .lines()
        .find_map(|line| line.strip_prefix("PRODUCT="));
    product.map_or_else(
        || fallback.to_owned(),
        |value| format!("PRODUCT:{value}:{fallback}"),
    )
}

fn read_u32(path: &Path) -> io::Result<u32> {
    let text = fs::read_to_string(path)?;
    text.trim()
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn read_u32_list(path: &Path) -> io::Result<Vec<u32>> {
    fs::read_to_string(path)?
        .split_whitespace()
        .map(|value| {
            value
                .parse()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        })
        .collect()
}

fn keyboards(leds: Vec<Led>) -> Vec<Keyboard> {
    let mut grouped: BTreeMap<String, Vec<Led>> = BTreeMap::new();
    for led in leds {
        grouped.entry(led.device_key.clone()).or_default().push(led);
    }
    grouped
        .into_iter()
        .map(|(key, leds)| {
            let stable_identity = leds
                .first()
                .map_or_else(|| key.clone(), |led| led.stable_identity.clone());
            Keyboard {
                key,
                stable_identity,
                leds,
            }
        })
        .collect()
}

fn keyboard_descriptor(keyboard: &Keyboard) -> DeviceDescriptor {
    let zoned = keyboard.leds.len() > 1 || keyboard.leds.iter().any(|led| led.zone.is_some());
    let all_rgb = keyboard.leds.iter().all(|led| led.rgb.is_some());
    let elements = if zoned {
        keyboard
            .leds
            .iter()
            .map(|led| ElementDescriptor {
                id: zone_id(led),
                name: Some(zone_name(led)),
                kind: ElementKind::Zone,
                geometry: None,
                physical_tags: Vec::new(),
                capabilities: capabilities(CapabilityScope::Element, led.rgb.is_some(), true),
                notes: trigger_notes(led),
                warnings: Vec::new(),
            })
            .collect()
    } else {
        Vec::new()
    };
    let broad_readback = !zoned;
    let groups = if zoned {
        vec![GroupDescriptor {
            id: ALL_ZONES_GROUP_ID.to_owned(),
            name: "All keyboard zones".to_owned(),
            description: Some("Every LED-class zone belonging to this keyboard".to_owned()),
            kind: GroupKind::Topology,
            members: keyboard
                .leds
                .iter()
                .map(|led| GroupMemberDescriptor::Element {
                    surface: SURFACE_ID.to_owned(),
                    element: zone_id(led),
                })
                .collect(),
            capabilities: capabilities(CapabilityScope::Device, all_rgb, false),
            notes: Vec::new(),
            warnings: Vec::new(),
        }]
    } else {
        Vec::new()
    };
    DeviceDescriptor {
        id: device_id(keyboard),
        name: format!("{} keyboard backlight", display_name(&keyboard.key)),
        vendor: None,
        model: None,
        surfaces: vec![SurfaceDescriptor {
            id: SURFACE_ID.to_owned(),
            name: "Keyboard backlight".to_owned(),
            kind: SurfaceKind::Opaque,
            physical_tags: Vec::new(),
            elements,
            capabilities: capabilities(CapabilityScope::Surface, all_rgb, broad_readback),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups,
        capabilities: capabilities(CapabilityScope::Device, all_rgb, broad_readback),
        claims: vec![HardwareClaim {
            bus: HardwareBus::Platform,
            physical_identity: keyboard.stable_identity.clone(),
            control_domain: "keyboard-backlight".to_owned(),
            exclusivity: ClaimExclusivity::Exclusive,
        }],
        category: Some(DeviceCategory::new(device_category::KEYBOARD)),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: vec![
            "Discovered through the Linux LED class; brightness uses a normalized 0–100 scale."
                .to_owned(),
        ],
        warnings: Vec::new(),
    }
}

fn capabilities(scope: CapabilityScope, rgb: bool, readback: bool) -> CapabilitySet {
    let mut readable_facets = vec![
        ReadableFacet {
            facet: StateFacetKind::Brightness,
            fidelity: ReadbackFidelity::BestEffort,
        },
        ReadableFacet {
            facet: StateFacetKind::Emission,
            fidelity: ReadbackFidelity::Exact,
        },
    ];
    if rgb {
        readable_facets.push(ReadableFacet {
            facet: StateFacetKind::Appearance,
            fidelity: ReadbackFidelity::BestEffort,
        });
    }
    CapabilitySet {
        colour: if rgb {
            vec![ColourCapability::rgb8()]
        } else {
            Vec::new()
        },
        brightness: BrightnessCapability::Independent {
            bits: 7,
            maximum: 100,
            scope,
        },
        state_readback: if readback {
            StateReadbackCapability::Readable {
                facets: readable_facets,
                read_disturbs_output: false,
                notifies_external_changes: false,
            }
        } else {
            StateReadbackCapability::None
        },
        emission: true,
        ..CapabilitySet::default()
    }
}

fn trigger_notes(led: &Led) -> Vec<String> {
    led.trigger
        .as_ref()
        .filter(|trigger| trigger.as_str() != "none")
        .map_or_else(Vec::new, |trigger| {
            vec![format!(
                "Kernel trigger '{trigger}' is active and is preserved by Luminate writes."
            )]
        })
}

fn device_id(keyboard: &Keyboard) -> String {
    format!(
        "linux-led-keyboard-{:016x}",
        stable_hash(&keyboard.stable_identity)
    )
}

fn stable_hash(value: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    value.bytes().fold(OFFSET, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(PRIME)
    })
}

fn zone_id(led: &Led) -> String {
    let identity = led
        .zone
        .as_ref()
        .map_or_else(|| "main".to_owned(), |zone| format!("zone:{zone}"));
    format!(
        "{}-{:016x}",
        stable_component(led.zone.as_deref().unwrap_or("main")),
        stable_hash(&identity)
    )
}

fn stable_component(value: &str) -> String {
    let mut output = String::new();
    let mut last_dash = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !output.is_empty() {
            output.push('-');
            last_dash = true;
        }
    }
    while output.ends_with('-') {
        let _ = output.pop();
    }
    if output.is_empty() {
        "unnamed".to_owned()
    } else {
        output
    }
}

fn display_name(value: &str) -> String {
    value.replace(['-', '_'], " ")
}

fn zone_name(led: &Led) -> String {
    led.zone
        .as_deref()
        .map_or_else(|| "Main".to_owned(), display_name)
}

fn apply_update(
    transport: &dyn Transport,
    keyboards: &[Keyboard],
    update: &PluginUpdate,
) -> Result<(), ApplyError> {
    let targets = resolve_target(keyboards, &update.target)?;
    for led in targets {
        apply_to_led(transport, led, &update.operation)?;
    }
    Ok(())
}

fn resolve_target<'a>(
    keyboards: &'a [Keyboard],
    target: &PluginTarget,
) -> Result<Vec<&'a Led>, ApplyError> {
    let (device, element) = match target {
        PluginTarget::Device { device } => (device, None),
        PluginTarget::Surface { device, surface } if surface == SURFACE_ID => (device, None),
        PluginTarget::Element {
            device,
            surface,
            element,
        } if surface == SURFACE_ID => (device, Some(element)),
        PluginTarget::Group { device, group } if group == ALL_ZONES_GROUP_ID => (device, None),
        PluginTarget::Surface { .. }
        | PluginTarget::Element { .. }
        | PluginTarget::Group { .. } => {
            return Err(ApplyError::Invalid(
                "unknown Linux LED-class target".to_owned(),
            ));
        }
    };
    let keyboard = keyboards
        .iter()
        .find(|keyboard| device_id(keyboard) == *device)
        .ok_or_else(|| ApplyError::Invalid(format!("unknown Linux LED-class device: {device}")))?;
    if let Some(element) = element {
        let led = keyboard
            .leds
            .iter()
            .find(|led| zone_id(led) == *element)
            .ok_or_else(|| ApplyError::Invalid(format!("unknown keyboard zone: {element}")))?;
        Ok(vec![led])
    } else {
        Ok(keyboard.leds.iter().collect())
    }
}

fn apply_to_led(
    transport: &dyn Transport,
    led: &Led,
    operation: &PluginUpdateOperation,
) -> Result<(), ApplyError> {
    match operation {
        PluginUpdateOperation::SetBrightness { value } => write_brightness(transport, led, *value),
        PluginUpdateOperation::SetEffect {
            effect: Effect::Static { colour },
        } => write_colour(transport, led, colour),
        PluginUpdateOperation::SetEffect { .. } => Err(ApplyError::Unsupported(
            "LED class supports only static colour".to_owned(),
        )),
        PluginUpdateOperation::Clear => write_brightness(transport, led, 0),
        PluginUpdateOperation::SaveCurrent => Err(ApplyError::Unsupported(
            "LED class has no save-current operation".to_owned(),
        )),
        PluginUpdateOperation::SetAppearanceSlots { .. } => Err(ApplyError::Unsupported(
            "LED class targets do not advertise appearance slots".to_owned(),
        )),
    }
}

fn write_brightness(transport: &dyn Transport, led: &Led, value: u32) -> Result<(), ApplyError> {
    if value > 100 {
        return Err(ApplyError::Invalid(format!(
            "brightness {value} exceeds 100"
        )));
    }
    let raw = scale(value, 100, led.maximum);
    transport.write(&led.path.join("brightness"), &raw.to_string())
}

fn write_colour(transport: &dyn Transport, led: &Led, colour: &Colour) -> Result<(), ApplyError> {
    use ColourChannel::{Blue, Green, Red};

    let rgb = led
        .rgb
        .as_ref()
        .ok_or_else(|| ApplyError::Unsupported(format!("{} is brightness-only", led.sysfs_name)))?;
    let channel = |wanted| {
        colour
            .channel(wanted)
            .ok_or_else(|| ApplyError::Invalid(format!("missing {wanted:?} colour channel")))
    };
    let red = channel(Red)?;
    let green = channel(Green)?;
    let blue = channel(Blue)?;
    if [red, green, blue]
        .iter()
        .any(|value| *value > u8::MAX.into())
    {
        return Err(ApplyError::Invalid("RGB channel exceeds 255".to_owned()));
    }
    let encoded: Vec<_> = rgb
        .indices
        .iter()
        .zip(&rgb.maxima)
        .map(|(index, maximum)| {
            let value = match index.as_str() {
                "red" => red,
                "green" => green,
                "blue" => blue,
                _ => 0,
            };
            scale(value, u8::MAX.into(), *maximum).to_string()
        })
        .collect();
    transport.write(&led.path.join("multi_intensity"), &encoded.join(" "))
}

fn scale(value: u32, from_maximum: u32, to_maximum: u32) -> u32 {
    let scaled = (u64::from(value) * u64::from(to_maximum) + u64::from(from_maximum / 2))
        / u64::from(from_maximum);
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

fn read_snapshot(root: &Path, request: &PluginReadRequest) -> PluginStateSnapshot {
    let keyboards = match scan(root) {
        Ok(leds) => keyboards(leds),
        Err(error) => {
            return PluginStateSnapshot {
                observations: Vec::new(),
                errors: request
                    .targets
                    .iter()
                    .map(|target| PluginReadError {
                        target: target.target.clone(),
                        diagnostic: error.to_string(),
                    })
                    .collect(),
            };
        }
    };
    let mut snapshot = PluginStateSnapshot::default();
    for requested in &request.targets {
        match resolve_target(&keyboards, &requested.target)
            .and_then(|leds| read_target(&requested.target, &requested.facets, &leds))
        {
            Ok(observations) => snapshot.observations.extend(observations),
            Err(error) => snapshot.errors.push(PluginReadError {
                target: requested.target.clone(),
                diagnostic: error.to_string(),
            }),
        }
    }
    snapshot
}

fn read_target(
    target: &PluginTarget,
    facets: &[StateFacetKind],
    leds: &[&Led],
) -> Result<Vec<PluginFacetObservation>, ApplyError> {
    let brightness: Vec<_> = leds
        .iter()
        .map(|led| {
            read_u32(&led.path.join("brightness"))
                .map(|value| scale(value.min(led.maximum), led.maximum, 100))
                .map_err(|error| {
                    ApplyError::Io(format!(
                        "cannot read {} brightness: {error}",
                        led.sysfs_name
                    ))
                })
        })
        .collect::<Result<_, _>>()?;
    let first = *brightness
        .first()
        .ok_or_else(|| ApplyError::Invalid("target has no LED zones".to_owned()))?;
    if brightness.iter().any(|value| *value != first) {
        return Err(ApplyError::Unsupported(
            "zones have different brightness values; read them individually".to_owned(),
        ));
    }
    let mut observations = Vec::new();
    for facet in facets {
        let value = match facet {
            StateFacetKind::Brightness => FacetValue::Brightness(first),
            StateFacetKind::Emission => FacetValue::Emission(if first == 0 {
                EmissionState::Dark
            } else {
                EmissionState::Emitting
            }),
            StateFacetKind::Appearance => read_appearance(leds)?,
            StateFacetKind::PhysicalPower => {
                return Err(ApplyError::Unsupported(
                    "LED class has no physical-power facet".to_owned(),
                ));
            }
            StateFacetKind::EffectiveAppearance => {
                return Err(ApplyError::Unsupported(
                    "effective appearance is daemon-synthesized and cannot be requested from a \
                        plugin"
                        .to_owned(),
                ));
            }
            StateFacetKind::AppearanceSlots => {
                return Err(ApplyError::Unsupported(
                    "LED class targets do not have appearance slots".to_owned(),
                ));
            }
        };
        observations.push(PluginFacetObservation {
            target: target.clone(),
            value,
        });
    }
    Ok(observations)
}

fn read_appearance(leds: &[&Led]) -> Result<FacetValue, ApplyError> {
    let colours: Vec<_> = leds
        .iter()
        .map(|led| read_colour(led))
        .collect::<Result<_, _>>()?;
    let first = colours
        .first()
        .ok_or_else(|| ApplyError::Invalid("target has no LED zones".to_owned()))?;
    if colours.iter().any(|colour| colour != first) {
        return Err(ApplyError::Unsupported(
            "zones have different colours; read them individually".to_owned(),
        ));
    }
    Ok(FacetValue::Appearance(AppearanceState::Static(
        first.clone(),
    )))
}

fn read_colour(led: &Led) -> Result<Colour, ApplyError> {
    use ColourChannel::{Blue, Green, Red};

    let rgb = led
        .rgb
        .as_ref()
        .ok_or_else(|| ApplyError::Unsupported(format!("{} is brightness-only", led.sysfs_name)))?;
    let raw = read_u32_list(&led.path.join("multi_intensity")).map_err(|error| {
        ApplyError::Io(format!("cannot read {} colour: {error}", led.sysfs_name))
    })?;
    if raw.len() != rgb.indices.len() {
        return Err(ApplyError::Io(format!(
            "{} returned the wrong channel count",
            led.sysfs_name
        )));
    }
    let value = |name: &str| {
        rgb.indices
            .iter()
            .zip(&raw)
            .zip(&rgb.maxima)
            .find(|((index, _), _)| index.as_str() == name)
            .map(|((_index, raw), maximum)| scale((*raw).min(*maximum), *maximum, u8::MAX.into()))
            .ok_or_else(|| ApplyError::Io(format!("{} has no {name} channel", led.sysfs_name)))
    };
    Colour::additive(vec![
        ColourChannelValue::new(Red, value("red")?),
        ColourChannelValue::new(Green, value("green")?),
        ColourChannelValue::new(Blue, value("blue")?),
    ])
    .map_err(|error| ApplyError::Invalid(error.to_string()))
}

luminate_export_plugin! {
    plugin: LinuxLeds,
    name: NAME,
    version: VERSION,
    priority: 20,
    recommended_reconciliation: Some(ReconciliationPolicy::Adopt),
    buses: BUSES,
    vendors: VENDORS,
    probe_hints: HINTS,
    start: native,
    // Topology scans sysfs itself; the poller only notices changes.
    rescan: none,
    batch: native,
    read_state: native,
    frame_upload: none,
    shm_frame: none,
}

#[cfg(test)]
#[cfg(target_os = "linux")]
#[path = "lib_tests.rs"]
mod tests;
