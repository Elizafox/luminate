// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! WLED JSON-API plugin.

mod http;
mod mdns;

use std::collections::{HashMap, HashSet};
use std::ffi::CStr;
use std::io::ErrorKind;
use std::net::{IpAddr, ToSocketAddrs as _};
use std::ptr;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use luminate_core::capability::{
    BrightnessCapability, BufferingMode, CapabilityScope, CapabilitySet, CctEmulation,
    ColourCapability, EffectChoice, EffectParameter, FrameUpdateMode, FrameUploadCapability,
    HardwareEffectDescriptor, HardwareEffectId, HardwareEffectsCapability, PhysicalPowerCapability,
    PowerDomainRef, ReadableFacet, ReadbackFidelity, StateReadbackCapability,
};
use luminate_core::colour::{Colour, Rgb8Error};
use luminate_core::control::ReconciliationPolicy;
use luminate_core::device::{DeviceCategory, device_category};
use luminate_core::effect::{Effect, EffectArguments};
use luminate_core::element::{ElementGeometry, ElementKind};
use luminate_core::frame::{FrameEnvelope, FramePayload};
use luminate_core::rgb::Rgb;
use luminate_core::state::{
    AppearanceState, EmissionState, FacetValue, PhysicalPowerState, StateFacetKind,
};
use luminate_core::surface::SurfaceKind;
use luminate_core::util::DiscreteRange;
use luminate_plugin_api::configuration;
use luminate_plugin_api::sdk::{
    DiscoveryPacer, DynamicDeviceRegistry, FrameStreamingPlugin, LuminatePlugin, ReadablePlugin,
    RegistryExpiry, RegistryPoisonError, RescanPlugin, StartPlugin,
};
use luminate_plugin_api::{
    ClaimExclusivity, DeviceDescriptor, ElementDescriptor, HardwareBus, HardwareClaim, PluginBus,
    PluginError, PluginFacetObservation, PluginProbeHint, PluginReadError, PluginReadRequest,
    PluginRequestContext, PluginSettingDescriptor, PluginSettingKind, PluginStateSnapshot,
    PluginTarget, PluginUpdate, PluginUpdateOperation, PluginVendorId, ProbeOutcome, RescanReason,
    SurfaceDescriptor, luminate_export_plugin,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use thiserror::Error;

use crate::http::Endpoint;

const NAME: &CStr = c"luminate-plugin-wled";
const VERSION: &CStr = c"0.1.0";
const SEGMENTS_SURFACE: &str = "segments";
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(15);
const DEVICE_EXPIRY: Duration = Duration::from_secs(90);
const MAX_ENDPOINTS: usize = 256;
const MAX_DISCOVERED_DEVICES_PER_SOURCE: usize = 16;

static BUSES: &[PluginBus] = &[PluginBus::Network];
static VENDORS: &[PluginVendorId] = &[];
static HINTS: &[PluginProbeHint] = &[];
static SETTINGS: &[PluginSettingDescriptor] = &[
    PluginSettingDescriptor {
        key: c"endpoints".as_ptr(),
        label: c"Controller endpoints".as_ptr(),
        description: c"HTTP host or host:port values used for explicit WLED discovery.".as_ptr(),
        kind: PluginSettingKind::Array as u32,
        default_toml: c"[]".as_ptr(),
        required: false,
        sensitive: false,
        apply_mode: luminate_plugin_api::PluginSettingApplyMode::RestartRequired as u32,
        minimum: 0.0,
        maximum: 256.0,
        has_minimum: false,
        has_maximum: true,
        constraints: c"{ element_kind = \"string\" }".as_ptr(),
    },
    PluginSettingDescriptor {
        key: c"mdns".as_ptr(),
        label: c"Enable mDNS discovery".as_ptr(),
        description: c"Discover WLED controllers advertised on the local network.".as_ptr(),
        kind: PluginSettingKind::Boolean as u32,
        default_toml: c"true".as_ptr(),
        required: false,
        sensitive: false,
        apply_mode: luminate_plugin_api::PluginSettingApplyMode::RestartRequired as u32,
        minimum: 0.0,
        maximum: 0.0,
        has_minimum: false,
        has_maximum: false,
        constraints: ptr::null(),
    },
    PluginSettingDescriptor {
        key: c"physical_tags".as_ptr(),
        label: c"Physical surface tags".as_ptr(),
        description: c"Open presentation hints applied to each WLED segments surface.".as_ptr(),
        kind: PluginSettingKind::Array as u32,
        default_toml: c"[]".as_ptr(),
        required: false,
        sensitive: false,
        apply_mode: luminate_plugin_api::PluginSettingApplyMode::RestartRequired as u32,
        minimum: 0.0,
        maximum: 0.0,
        has_minimum: false,
        has_maximum: false,
        constraints: c"{ element_kind = \"string\" }".as_ptr(),
    },
];

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct WledConfig {
    /// Explicit HTTP endpoints. An empty array disables configured endpoints.
    endpoints: Vec<String>,
    mdns: bool,
    /// Presentation hints for each WLED segments surface.
    physical_tags: Vec<String>,
}

impl Default for WledConfig {
    fn default() -> Self {
        Self {
            endpoints: Vec::new(),
            mdns: true,
            physical_tags: Vec::new(),
        }
    }
}

fn wled_configuration() -> Result<&'static WledConfig, &'static str> {
    static CONFIGURATION: OnceLock<Result<WledConfig, String>> = OnceLock::new();
    CONFIGURATION
        .get_or_init(|| {
            let configuration =
                configuration::deserialize::<WledConfig>().map_err(|error| error.to_string())?;
            validate_configuration(configuration)
        })
        .as_ref()
        .map_err(String::as_str)
}

fn validate_configuration(configuration: WledConfig) -> Result<WledConfig, String> {
    if configuration.endpoints.len() > MAX_ENDPOINTS {
        return Err(format!(
            "WLED endpoints contains {} entries; at most {MAX_ENDPOINTS} are allowed",
            configuration.endpoints.len()
        ));
    }
    for endpoint in &configuration.endpoints {
        validate_endpoint_syntax(endpoint)?;
    }
    Ok(configuration)
}

#[derive(Debug, Clone)]
struct Device {
    id: String,
    endpoint: Endpoint,
    name: String,
    version: String,
    mac: String,
    led_count: u32,
    segments: Vec<Segment>,
    effects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct Segment {
    id: u16,
    #[serde(default)]
    start: u32,
    #[serde(default)]
    stop: u32,
    #[serde(default, rename = "n")]
    name: String,
    #[serde(default = "default_true")]
    on: bool,
    #[serde(default = "maximum_byte")]
    bri: u8,
    #[serde(default)]
    col: Vec<Vec<u8>>,
    #[serde(default)]
    fx: u16,
    #[serde(default = "mid_byte")]
    sx: u8,
    #[serde(default = "mid_byte")]
    ix: u8,
    #[serde(default)]
    pal: u16,
}

const fn default_true() -> bool {
    true
}

const fn maximum_byte() -> u8 {
    u8::MAX
}

const fn mid_byte() -> u8 {
    128
}

#[derive(Debug, Deserialize)]
struct Info {
    #[serde(default)]
    name: String,
    #[serde(default)]
    ver: String,
    #[serde(default)]
    mac: String,
    #[serde(default)]
    leds: LedInfo,
}

#[derive(Debug, Default, Deserialize)]
struct LedInfo {
    #[serde(default)]
    count: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct State {
    #[serde(default)]
    on: bool,
    #[serde(default)]
    bri: u8,
    #[serde(default, rename = "seg")]
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TopologyFingerprint {
    name: String,
    version: String,
    led_count: u32,
    segments: Vec<(u16, u32, u32, String)>,
    effects: Vec<String>,
}

struct Runtime {
    devices: DynamicDeviceRegistry<Device, TopologyFingerprint>,
    /// Lets a rescan cut short the discovery thread's inter-cycle wait.
    discovery: DiscoveryPacer,
}

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime {
        devices: DynamicDeviceRegistry::new(MAX_ENDPOINTS, DEVICE_EXPIRY, RegistryExpiry::After),
        discovery: DiscoveryPacer::new(),
    })
}

#[derive(Debug, Error)]
enum ApplyError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("{message}")]
    RateLimited {
        message: String,
        retry_after: Duration,
    },
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Internal(String),
}

#[derive(Debug, PartialEq, Eq)]
struct CommandPlan {
    endpoint: Endpoint,
    path: &'static str,
    body: Vec<u8>,
}

trait Transport {
    fn execute(&self, plan: &CommandPlan) -> Result<(), ApplyError>;
}

struct HttpTransport;

impl Transport for HttpTransport {
    fn execute(&self, plan: &CommandPlan) -> Result<(), ApplyError> {
        http::post(&plan.endpoint, plan.path, &plan.body)
            .map(|_response| ())
            .map_err(classify_transport_error)
    }
}

fn classify_transport_error(error: http::HttpError) -> ApplyError {
    const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(1);

    match error {
        http::HttpError::Status {
            code: 429,
            reason,
            retry_after,
        } => ApplyError::RateLimited {
            message: format!("HTTP rate limit: {reason}"),
            retry_after: retry_after.unwrap_or(DEFAULT_RETRY_AFTER),
        },
        http::HttpError::Status {
            code: 503,
            reason,
            retry_after: Some(retry_after),
        } => ApplyError::RateLimited {
            message: format!("HTTP rate limit: {reason}"),
            retry_after,
        },
        http::HttpError::Status {
            code: 502..=504,
            reason,
            ..
        } => ApplyError::Unavailable(format!("WLED controller unavailable: {reason}")),
        http::HttpError::Io(error)
            if matches!(
                error.kind(),
                ErrorKind::ConnectionRefused
                    | ErrorKind::ConnectionReset
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::NotConnected
                    | ErrorKind::AddrNotAvailable
                    | ErrorKind::BrokenPipe
                    | ErrorKind::UnexpectedEof
            ) =>
        {
            ApplyError::Unavailable(error.to_string())
        }
        http::HttpError::Io(error)
            if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) =>
        {
            ApplyError::Io(format!("WLED request timed out: {error}"))
        }
        error @ (http::HttpError::Io(_)
        | http::HttpError::Protocol(_)
        | http::HttpError::Status { .. }) => ApplyError::Io(error.to_string()),
    }
}

impl From<ApplyError> for PluginError {
    fn from(error: ApplyError) -> Self {
        match error {
            ApplyError::Invalid(message) => Self::InvalidArgument(message),
            ApplyError::Unsupported(message) => Self::Unsupported(message),
            ApplyError::Unavailable(message) => Self::Unavailable(message),
            ApplyError::RateLimited {
                message,
                retry_after,
            } => Self::RateLimited {
                diagnostic: message,
                retry_after,
            },
            ApplyError::Io(message) => Self::Io(message),
            ApplyError::Internal(message) => Self::Internal(message),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Scope {
    Device,
    Segments,
    Segment(u16),
}

fn start_discovery_thread() {
    match thread::Builder::new()
        .name("luminate-wled-discovery".to_owned())
        .spawn(discovery_loop)
    {
        Ok(_thread) => tracing::info!("WLED discovery thread started"),
        Err(error) => tracing::error!(error = %error, "failed to start WLED discovery thread"),
    }
}

fn discovery_loop() {
    loop {
        if let Err(error) = refresh_devices() {
            tracing::error!(error = %error, "WLED discovery stopped after registry invariant failure");
            return;
        }
        runtime().discovery.wait(DISCOVERY_INTERVAL);
    }
}

fn refresh_devices() -> Result<(), RegistryPoisonError> {
    let mut endpoints = configured_endpoints()
        .into_iter()
        .map(|endpoint| (endpoint, None))
        .collect::<Vec<_>>();
    if wled_configuration().is_ok_and(|configuration| configuration.mdns) {
        match mdns::discover() {
            Ok(discovered) => endpoints.extend(discovered.into_iter().map(|endpoint| {
                let source = Some(endpoint.address.ip());
                (endpoint, source)
            })),
            Err(error) => tracing::debug!(error = %error, "WLED mDNS discovery failed"),
        }
    }
    deduplicate_endpoints(&mut endpoints);
    limit_discovered_endpoints_per_source(&mut endpoints);
    endpoints.truncate(MAX_ENDPOINTS);

    let mut refreshed = Vec::new();
    for (endpoint, discovery_source) in endpoints {
        match inspect(&endpoint) {
            Ok(device) => refreshed.push((device, discovery_source)),
            Err(error) => {
                tracing::debug!(address = %endpoint.address, error = %error, "WLED endpoint did not respond");
            }
        }
    }

    let refresh = runtime().devices.refresh_with_source_limit(
        Instant::now(),
        refreshed.into_iter().map(|(device, discovery_source)| {
            let id = device.id.clone();
            let fingerprint = topology_fingerprint(&device);
            (id, device, fingerprint, discovery_source)
        }),
        MAX_DISCOVERED_DEVICES_PER_SOURCE,
    )?;
    if refresh.topology_changed {
        tracing::info!("WLED topology changed");
    }
    Ok(())
}

fn configured_endpoints() -> Vec<Endpoint> {
    wled_configuration()
        .map(|configuration| configuration.endpoints.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|value| match parse_endpoint(value.trim()) {
            Ok(endpoints) => Some(endpoints),
            Err(error) => {
                tracing::warn!(endpoint = value.trim(), error = %error, "ignoring invalid configured WLED endpoint");
                None
            }
        })
        .flatten()
        .take(MAX_ENDPOINTS)
        .collect()
}

fn parse_endpoint(value: &str) -> Result<Vec<Endpoint>, String> {
    validate_endpoint_syntax(value)?;
    let value = value.strip_prefix("http://").unwrap_or(value);
    let (host, port) = split_host_port(value)?;
    let addresses = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|error| error.to_string())?;
    let mut host_header = if host.contains(':') {
        format!("[{host}]")
    } else {
        host
    };
    if port != 80 {
        host_header.push(':');
        host_header.push_str(&port.to_string());
    }
    let endpoints: Vec<_> = addresses
        .map(|address| Endpoint {
            address,
            host: host_header.clone(),
        })
        .collect();
    if endpoints.is_empty() {
        Err("endpoint resolved to no addresses".to_owned())
    } else {
        Ok(endpoints)
    }
}

fn validate_endpoint_syntax(value: &str) -> Result<(), String> {
    let value = value.trim();
    let value = value.strip_prefix("http://").unwrap_or(value);
    if value.is_empty() || value.starts_with("https://") || value.contains('/') {
        return Err(format!(
            "invalid WLED endpoint {value:?}: expected an HTTP host or host:port without a path"
        ));
    }
    let _ = split_host_port(value)?;
    Ok(())
}

fn split_host_port(value: &str) -> Result<(String, u16), String> {
    if let Some(rest) = value.strip_prefix('[') {
        let (host, suffix) = rest
            .split_once(']')
            .ok_or_else(|| "unterminated IPv6 address".to_owned())?;
        let port = suffix.strip_prefix(':').map_or(Ok(80), |value| {
            value.parse::<u16>().map_err(|error| error.to_string())
        })?;
        return Ok((host.to_owned(), port));
    }
    if value.matches(':').count() == 1 {
        let (host, port) = value
            .split_once(':')
            .ok_or_else(|| "invalid endpoint".to_owned())?;
        return Ok((
            host.to_owned(),
            port.parse::<u16>().map_err(|error| error.to_string())?,
        ));
    }
    Ok((value.to_owned(), 80))
}

fn deduplicate_endpoints(endpoints: &mut Vec<(Endpoint, Option<IpAddr>)>) {
    let mut seen = HashSet::new();
    endpoints.retain(|(endpoint, _)| seen.insert(endpoint.address));
}

fn limit_discovered_endpoints_per_source(endpoints: &mut Vec<(Endpoint, Option<IpAddr>)>) {
    let mut counts = HashMap::new();
    endpoints.retain(|(_, source)| {
        source.is_none_or(|source| {
            let count = counts.entry(source).or_insert(0);
            if *count >= MAX_DISCOVERED_DEVICES_PER_SOURCE {
                return false;
            }
            *count += 1;
            true
        })
    });
}

fn inspect(endpoint: &Endpoint) -> Result<Device, ApplyError> {
    let info: Info = get_json(endpoint, "/json/info")?;
    let state: State = get_json(endpoint, "/json/state")?;
    let mut effects: Vec<String> = get_json(endpoint, "/json/eff")?;
    effects.truncate(256);
    let mac = normalize_mac(&info.mac)
        .ok_or_else(|| ApplyError::Io("WLED returned no valid MAC identity".to_owned()))?;
    Ok(Device {
        id: format!("wled-{mac}"),
        endpoint: endpoint.clone(),
        name: if info.name.trim().is_empty() {
            format!("WLED {mac}")
        } else {
            info.name
        },
        version: info.ver,
        mac,
        led_count: info.leds.count,
        segments: valid_segments(state.segments, info.leds.count),
        effects,
    })
}

fn valid_segments(mut segments: Vec<Segment>, led_count: u32) -> Vec<Segment> {
    if led_count == 0 {
        return Vec::new();
    }
    segments.retain(|segment| segment.stop > segment.start && segment.stop <= led_count);
    segments.sort_by_key(|segment| segment.id);
    segments.dedup_by_key(|segment| segment.id);
    segments.truncate(32);
    segments
}

fn normalize_mac(value: &str) -> Option<String> {
    let normalized: String = value
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|character| character.to_ascii_lowercase())
        .collect();
    (normalized.len() == 12).then_some(normalized)
}

fn get_json<T: DeserializeOwned>(endpoint: &Endpoint, path: &str) -> Result<T, ApplyError> {
    let response = http::get(endpoint, path).map_err(|error| ApplyError::Io(error.to_string()))?;
    serde_json::from_slice(&response)
        .map_err(|error| ApplyError::Io(format!("invalid WLED JSON from {path}: {error}")))
}

fn topology_fingerprint(device: &Device) -> TopologyFingerprint {
    TopologyFingerprint {
        name: device.name.clone(),
        version: device.version.clone(),
        led_count: device.led_count,
        segments: device
            .segments
            .iter()
            .map(|segment| {
                (
                    segment.id,
                    segment.start,
                    segment.stop,
                    segment.name.clone(),
                )
            })
            .collect(),
        effects: device.effects.clone(),
    }
}

struct Wled;

impl LuminatePlugin for Wled {
    fn new() -> Result<Self, PluginError> {
        wled_configuration().map_err(|error| PluginError::InvalidArgument(error.to_owned()))?;
        let _ = runtime();
        Ok(Self)
    }

    fn probe(&self) -> ProbeOutcome {
        // Network controllers can appear after startup.
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
        apply_update(update).map_err(PluginError::from)
    }
}

impl StartPlugin for Wled {
    fn start(&self) {
        start_discovery_thread();
    }
}

impl RescanPlugin for Wled {
    fn rescan(&self, reason: RescanReason) {
        // Refresh before topology is pulled again. Registry expiry removes
        // absent devices without making every device briefly disappear.
        tracing::debug!(reason = ?reason, "WLED rescan requested; waking discovery");
        runtime().discovery.wake();
    }
}

impl ReadablePlugin for Wled {
    fn read_state(
        &self,
        _context: &PluginRequestContext,
        request: &PluginReadRequest,
    ) -> Result<PluginStateSnapshot, PluginError> {
        read_snapshot(request)
    }
}

impl FrameStreamingPlugin for Wled {
    fn upload_frame(
        &self,
        _context: &PluginRequestContext,
        target: &PluginTarget,
        envelope: &FrameEnvelope,
    ) -> Result<(), PluginError> {
        apply_frame(target, envelope).map_err(PluginError::from)
    }
}

fn device_descriptor(device: &Device) -> DeviceDescriptor {
    let physical_tags = wled_configuration()
        .map(|configuration| configuration.physical_tags.clone())
        .unwrap_or_default();
    let elements = device
        .segments
        .iter()
        .map(|segment| ElementDescriptor {
            id: segment_id(segment.id),
            name: Some(segment_name(segment)),
            kind: ElementKind::Zone,
            geometry: segment_geometry(segment, device.led_count),
            physical_tags: Vec::new(),
            capabilities: capabilities(device, CapabilityScope::Element, false),
            notes: vec![format!("WLED pixels {}..{}", segment.start, segment.stop)],
            warnings: Vec::new(),
        })
        .collect();
    DeviceDescriptor {
        id: device.id.clone(),
        name: format!("{} ({})", device.name, device.mac),
        vendor: Some("WLED".to_owned()),
        model: (!device.version.is_empty()).then(|| format!("WLED {}", device.version)),
        surfaces: vec![SurfaceDescriptor {
            id: SEGMENTS_SURFACE.to_owned(),
            name: "Segments".to_owned(),
            kind: SurfaceKind::Linear {
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "surface lengths are presentation-only f32 coordinates"
                )]
                length: device.led_count.max(1) as f32,
            },
            physical_tags,
            elements,
            capabilities: capabilities(device, CapabilityScope::Surface, false),
            notes: vec![
                "Segments are firmware-defined addressable regions; frame streaming targets the whole device, not individual segments."
                    .to_owned(),
            ],
            warnings: Vec::new(),
        }],
        groups: Vec::new(),
        capabilities: capabilities(device, CapabilityScope::Device, true),
        claims: vec![HardwareClaim {
            bus: HardwareBus::Network,
            physical_identity: device.mac.clone(),
            control_domain: "led-output".to_owned(),
            exclusivity: ClaimExclusivity::Exclusive,
        }],
        category: Some(DeviceCategory::new(device_category::CONTROLLER)),
        physical_tags: Vec::new(),
        host_attached: false,
        notes: vec![format!(
            "Discovered through WLED mDNS or a configured endpoint; controller reports {} LEDs.",
            device.led_count
        )],
        warnings: vec![
            "WLED's HTTP API is unauthenticated unless protected by the surrounding network."
                .to_owned(),
        ],
    }
}

fn segment_name(segment: &Segment) -> String {
    if segment.name.trim().is_empty() {
        format!("Segment {}", segment.id)
    } else {
        segment.name.clone()
    }
}

fn segment_geometry(segment: &Segment, led_count: u32) -> Option<ElementGeometry> {
    if led_count == 0 {
        return None;
    }
    let midpoint = f64::midpoint(f64::from(segment.start), f64::from(segment.stop));
    #[allow(
        clippy::cast_possible_truncation,
        reason = "normalized topology coordinates intentionally use f32"
    )]
    let position = (midpoint / f64::from(led_count)).clamp(0.0, 1.0) as f32;
    Some(ElementGeometry::Linear { position })
}

fn capabilities(device: &Device, scope: CapabilityScope, physical_power: bool) -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: u8::MAX.into(),
            scope,
        },
        hardware_effects: Some(HardwareEffectsCapability {
            effects: effect_descriptors(&device.effects),
            scope,
            concurrent_with_streaming: false,
        }),
        frame_upload: matches!(scope, CapabilityScope::Device).then_some(FrameUploadCapability {
            scope: CapabilityScope::Device,
            update_mode: FrameUpdateMode::FullFrameOnly,
            max_rate_hz: None,
            atomic: true,
            buffering: BufferingMode::Immediate,
            shm: None,
        }),
        state_readback: StateReadbackCapability::Readable {
            facets: readable_facets(physical_power),
            read_disturbs_output: false,
            notifies_external_changes: false,
        },
        emission: true,
        physical_power: physical_power.then_some(PhysicalPowerCapability {
            scope: CapabilityScope::Device,
        }),
        power_domain: Some(PowerDomainRef::Device),
        ..CapabilitySet::default()
    }
}

fn readable_facets(physical_power: bool) -> Vec<ReadableFacet> {
    let mut facets = vec![
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
    ];
    if physical_power {
        facets.push(ReadableFacet {
            facet: StateFacetKind::PhysicalPower,
            fidelity: ReadbackFidelity::Exact,
        });
    }
    facets
}

fn effect_descriptors(effects: &[String]) -> Vec<HardwareEffectDescriptor> {
    let mut descriptors = Vec::new();
    for (id, name, parameters) in [
        ("breathe", "Breathe", colour_duration_parameters()),
        ("pulse", "Pulse", colour_duration_parameters()),
        ("strobe", "Strobe", colour_duration_parameters()),
        ("scanner", "Scanner", colour_duration_parameters()),
        ("spectrum", "Spectrum", duration_parameters()),
        ("rainbow", "Rainbow", duration_parameters()),
    ] {
        if find_effect(effects, id).is_some() {
            descriptors.push(HardwareEffectDescriptor {
                id: HardwareEffectId::new(id),
                name: name.to_owned(),
                parameters,
            });
        }
    }
    let choices = effects
        .iter()
        .enumerate()
        .filter(|(_index, name)| !name.trim().is_empty() && !is_reserved_effect(name))
        .map(|(index, name)| EffectChoice {
            id: index.to_string(),
            name: name.clone(),
        })
        .collect::<Vec<_>>();
    if !choices.is_empty() {
        descriptors.push(HardwareEffectDescriptor {
            id: HardwareEffectId::new("wled-effect"),
            name: "WLED firmware effect".to_owned(),
            parameters: vec![
                EffectParameter::Choice { options: choices },
                speed_parameter(),
            ],
        });
    }
    descriptors
}

fn is_reserved_effect(name: &str) -> bool {
    let name = name.trim();
    name == "-" || name.eq_ignore_ascii_case("RSVD")
}

fn colour_duration_parameters() -> Vec<EffectParameter> {
    vec![
        EffectParameter::Colour {
            minimum_colours: 1,
            maximum_colours: 1,
        },
        duration_parameter(),
    ]
}

fn duration_parameters() -> Vec<EffectParameter> {
    vec![duration_parameter()]
}

fn duration_parameter() -> EffectParameter {
    EffectParameter::Duration {
        milliseconds: DiscreteRange::new(100, 60_000, 1),
    }
}

fn speed_parameter() -> EffectParameter {
    EffectParameter::Speed {
        range: DiscreteRange::new(0, u16::from(u8::MAX), 1),
    }
}

fn apply_update(update: &PluginUpdate) -> Result<(), ApplyError> {
    apply_update_with_transport(update, &HttpTransport)
}

fn apply_update_with_transport(
    update: &PluginUpdate,
    transport: &impl Transport,
) -> Result<(), ApplyError> {
    let device_id = update.target.device_id();
    let device = runtime()
        .devices
        .get(device_id)
        .map_err(|error| ApplyError::Internal(error.to_string()))?
        .ok_or_else(|| {
            ApplyError::Invalid(format!("unknown or unavailable WLED device: {device_id}"))
        })?;
    execute_update(&device, update, transport)
}

fn execute_update(
    device: &Device,
    update: &PluginUpdate,
    transport: &impl Transport,
) -> Result<(), ApplyError> {
    let plan = plan_update(device, update)?;
    transport.execute(&plan)
}

fn plan_update(device: &Device, update: &PluginUpdate) -> Result<CommandPlan, ApplyError> {
    let scope = resolve_scope(device, &update.target)?;
    let payload = operation_payload(device, scope, &update.operation)?;
    let body =
        serde_json::to_vec(&payload).map_err(|error| ApplyError::Invalid(error.to_string()))?;
    Ok(CommandPlan {
        endpoint: device.endpoint.clone(),
        path: "/json/state",
        body,
    })
}

fn resolve_scope(device: &Device, target: &PluginTarget) -> Result<Scope, ApplyError> {
    match target {
        PluginTarget::Device { device: target } if target == &device.id => Ok(Scope::Device),
        PluginTarget::Surface {
            device: target,
            surface,
        } if target == &device.id && surface == SEGMENTS_SURFACE => Ok(Scope::Segments),
        PluginTarget::Element {
            device: target,
            surface,
            element,
        } if target == &device.id && surface == SEGMENTS_SURFACE => {
            let id = parse_segment_id(element)?;
            device
                .segments
                .iter()
                .any(|segment| segment.id == id)
                .then_some(Scope::Segment(id))
                .ok_or_else(|| ApplyError::Invalid(format!("unknown WLED segment: {element}")))
        }
        PluginTarget::Device { .. }
        | PluginTarget::Surface { .. }
        | PluginTarget::Element { .. }
        | PluginTarget::Group { .. } => Err(ApplyError::Invalid("unknown WLED target".to_owned())),
    }
}

fn apply_frame(target: &PluginTarget, envelope: &FrameEnvelope) -> Result<(), ApplyError> {
    apply_frame_with_transport(target, envelope, &HttpTransport)
}

fn apply_frame_with_transport(
    target: &PluginTarget,
    envelope: &FrameEnvelope,
    transport: &impl Transport,
) -> Result<(), ApplyError> {
    let PluginTarget::Device { device: device_id } = target else {
        return Err(ApplyError::Unsupported(
            "WLED frame streaming is only supported at device scope".to_owned(),
        ));
    };
    let device = runtime()
        .devices
        .get(device_id)
        .map_err(|error| ApplyError::Internal(error.to_string()))?
        .ok_or_else(|| {
            ApplyError::Invalid(format!("unknown or unavailable WLED device: {device_id}"))
        })?;
    execute_frame(&device, envelope, transport)
}

fn execute_frame(
    device: &Device,
    envelope: &FrameEnvelope,
    transport: &impl Transport,
) -> Result<(), ApplyError> {
    let plan = plan_frame(device, envelope)?;
    transport.execute(&plan)
}

/// Builds one device-wide `seg.i` command in topology pixel order.
fn plan_frame(device: &Device, envelope: &FrameEnvelope) -> Result<CommandPlan, ApplyError> {
    let FramePayload::Full(pixels) = &envelope.payload else {
        return Err(ApplyError::Unsupported(
            "WLED frame streaming only supports full frames".to_owned(),
        ));
    };
    if pixels.len() != device.led_count as usize {
        return Err(ApplyError::Invalid(format!(
            "expected {} pixels for a full WLED frame, got {}",
            device.led_count,
            pixels.len()
        )));
    }
    let segments = device
        .segments
        .iter()
        .map(|segment| segment_frame_payload(segment, pixels))
        .collect::<Result<Vec<_>, ApplyError>>()?;
    let body = serde_json::to_vec(&json!({"on": true, "seg": segments}))
        .map_err(|error| ApplyError::Invalid(error.to_string()))?;
    Ok(CommandPlan {
        endpoint: device.endpoint.clone(),
        path: "/json/state",
        body,
    })
}

fn segment_frame_payload(segment: &Segment, pixels: &[Colour]) -> Result<Value, ApplyError> {
    let start = (segment.start as usize).min(pixels.len());
    let stop = (segment.stop as usize).clamp(start, pixels.len());
    let hex_pixels = pixels
        .get(start..stop)
        .unwrap_or_default()
        .iter()
        .map(|colour| {
            rgb_from_colour(colour).map(|rgb| format!("{:02X}{:02X}{:02X}", rgb.r, rgb.g, rgb.b))
        })
        .collect::<Result<Vec<_>, ApplyError>>()?;
    Ok(json!({"id": segment.id, "i": hex_pixels}))
}

fn segment_id(id: u16) -> String {
    format!("segment-{id}")
}

fn parse_segment_id(value: &str) -> Result<u16, ApplyError> {
    value
        .strip_prefix("segment-")
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ApplyError::Invalid(format!("invalid WLED segment ID: {value}")))
}

fn operation_payload(
    device: &Device,
    scope: Scope,
    operation: &PluginUpdateOperation,
) -> Result<Value, ApplyError> {
    match operation {
        PluginUpdateOperation::SetBrightness { value } => {
            let brightness = u8::try_from(*value)
                .map_err(|_| ApplyError::Invalid(format!("WLED brightness {value} exceeds 255")))?;
            if matches!(scope, Scope::Device) {
                Ok(json!({"bri": brightness, "on": brightness > 0}))
            } else {
                Ok(scoped_payload(
                    device,
                    scope,
                    json!({"bri": brightness, "on": brightness > 0}),
                ))
            }
        }
        PluginUpdateOperation::SetEffect { effect } => effect_payload(device, scope, effect),
        PluginUpdateOperation::Clear if matches!(scope, Scope::Device) => Ok(json!({"on": false})),
        PluginUpdateOperation::Clear => Ok(scoped_payload(device, scope, json!({"on": false}))),
        PluginUpdateOperation::SaveCurrent => Err(ApplyError::Unsupported(
            "WLED preset persistence is not exposed as save-current".to_owned(),
        )),
        PluginUpdateOperation::SetAppearanceSlots { .. } => Err(ApplyError::Unsupported(
            "WLED targets do not advertise appearance slots".to_owned(),
        )),
    }
}

fn scoped_payload(device: &Device, scope: Scope, segment: Value) -> Value {
    match scope {
        Scope::Device | Scope::Segments => {
            let updates: Vec<_> = device
                .segments
                .iter()
                .map(|known| {
                    let mut update = segment.clone();
                    if let Value::Object(fields) = &mut update {
                        fields.insert("id".to_owned(), json!(known.id));
                    }
                    update
                })
                .collect();
            let mut payload = json!({"seg": updates});
            if let Value::Object(fields) = &mut payload {
                if segment.get("on") == Some(&Value::Bool(true)) {
                    fields.insert("on".to_owned(), json!(true));
                } else if matches!(scope, Scope::Device) {
                    fields.insert(
                        "on".to_owned(),
                        segment.get("on").cloned().unwrap_or(json!(true)),
                    );
                }
            }
            payload
        }
        Scope::Segment(id) => {
            let mut update = segment;
            if let Value::Object(fields) = &mut update {
                fields.insert("id".to_owned(), json!(id));
            }
            if update.get("on") == Some(&Value::Bool(true)) {
                json!({"on": true, "seg": update})
            } else {
                json!({"seg": update})
            }
        }
    }
}

fn effect_payload(device: &Device, scope: Scope, effect: &Effect) -> Result<Value, ApplyError> {
    match effect {
        Effect::Off if matches!(scope, Scope::Device) => Ok(json!({"on": false})),
        Effect::Off => Ok(scoped_payload(device, scope, json!({"on": false}))),
        Effect::Static { colour } => {
            let rgb = rgb_from_colour(colour)?;
            Ok(scoped_payload(
                device,
                scope,
                json!({"on": true, "fx": solid_effect(&device.effects), "col": [[rgb.r, rgb.g, rgb.b]]}),
            ))
        }
        Effect::Breathe { colour, period_ms } => named_effect_payload(
            device,
            scope,
            "breathe",
            Some(*colour),
            period_speed(*period_ms),
        ),
        Effect::Pulse { colour, period_ms } => named_effect_payload(
            device,
            scope,
            "pulse",
            Some(*colour),
            period_speed(*period_ms),
        ),
        Effect::Strobe { colour, period_ms } => named_effect_payload(
            device,
            scope,
            "strobe",
            Some(*colour),
            period_speed(*period_ms),
        ),
        Effect::Scanner { colour, period_ms } => named_effect_payload(
            device,
            scope,
            "scanner",
            Some(*colour),
            period_speed(*period_ms),
        ),
        Effect::Spectrum { period_ms } => {
            named_effect_payload(device, scope, "spectrum", None, period_speed(*period_ms))
        }
        Effect::Rainbow { period_ms } => {
            named_effect_payload(device, scope, "rainbow", None, period_speed(*period_ms))
        }
        Effect::Morph { .. } => Err(ApplyError::Unsupported(
            "WLED does not expose a portable morph mapping".to_owned(),
        )),
        Effect::Hardware { id, arguments } if id.as_str() == "wled-effect" => {
            hardware_effect_payload(device, scope, arguments)
        }
        Effect::Hardware { id, .. } => Err(ApplyError::Invalid(format!(
            "unknown WLED hardware effect: {}",
            id.as_str()
        ))),
    }
}

fn named_effect_payload(
    device: &Device,
    scope: Scope,
    name: &str,
    colour: Option<Rgb>,
    speed: u8,
) -> Result<Value, ApplyError> {
    let effect = find_effect(&device.effects, name).ok_or_else(|| {
        ApplyError::Unsupported(format!("this WLED firmware does not provide {name}"))
    })?;
    let mut fields = serde_json::Map::from_iter([
        ("on".to_owned(), json!(true)),
        ("fx".to_owned(), json!(effect)),
        ("sx".to_owned(), json!(speed)),
    ]);
    if let Some(colour) = colour {
        fields.insert("col".to_owned(), json!([[colour.r, colour.g, colour.b]]));
    }
    Ok(scoped_payload(device, scope, Value::Object(fields)))
}

fn hardware_effect_payload(
    device: &Device,
    scope: Scope,
    arguments: &EffectArguments,
) -> Result<Value, ApplyError> {
    let choice = arguments
        .choice
        .as_deref()
        .ok_or_else(|| ApplyError::Invalid("WLED effect choice is required".to_owned()))?;
    let effect = choice
        .parse::<usize>()
        .ok()
        .filter(|index| *index < device.effects.len())
        .ok_or_else(|| ApplyError::Invalid(format!("unknown WLED effect choice: {choice}")))?;
    let speed = arguments
        .speed
        .map(|speed| {
            u8::try_from(speed)
                .map_err(|_| ApplyError::Invalid("WLED effect speed exceeds 255".to_owned()))
        })
        .transpose()?
        .unwrap_or(128);
    Ok(scoped_payload(
        device,
        scope,
        json!({"on": true, "fx": effect, "sx": speed}),
    ))
}

fn period_speed(period_ms: u32) -> u8 {
    let period = period_ms.clamp(100, 60_000);
    let inverse = 60_000_u64.saturating_sub(u64::from(period));
    u8::try_from((inverse * u64::from(u8::MAX)) / 59_900).unwrap_or(u8::MAX)
}

fn find_effect(effects: &[String], wanted: &str) -> Option<usize> {
    let aliases: &[&str] = match wanted {
        "breathe" => &["breathe", "breath"],
        "pulse" => &["blink", "pulse"],
        "strobe" => &["strobe"],
        "scanner" => &["scanner", "larson scanner"],
        "spectrum" => &["colorloop", "colourloop", "colorwaves"],
        "rainbow" => &["rainbow", "rainbow runner"],
        _ => &[wanted],
    };
    effects.iter().position(|effect| {
        aliases
            .iter()
            .any(|alias| effect.eq_ignore_ascii_case(alias))
    })
}

fn solid_effect(effects: &[String]) -> usize {
    effects
        .iter()
        .position(|effect| effect.eq_ignore_ascii_case("solid"))
        .unwrap_or(0)
}

fn rgb_from_colour(colour: &Colour) -> Result<Rgb, ApplyError> {
    colour.try_as_rgb().map_err(|error| match error {
        Rgb8Error::MissingChannel(channel) => {
            ApplyError::Invalid(format!("missing {channel:?} colour channel"))
        }
        Rgb8Error::ChannelOutOfRange { channel, .. } => {
            ApplyError::Invalid(format!("{channel:?} colour channel exceeds 255"))
        }
        Rgb8Error::NonAdditive(encoding) => {
            ApplyError::Invalid(format!("{encoding:?} colour does not contain RGB channels"))
        }
    })
}

fn read_snapshot(request: &PluginReadRequest) -> Result<PluginStateSnapshot, PluginError> {
    let devices = runtime()
        .devices
        .snapshot()
        .map_err(|error| PluginError::Internal(error.to_string()))?
        .into_iter()
        .map(|device| (device.id.clone(), device))
        .collect::<HashMap<_, _>>();
    let mut states: HashMap<String, Result<State, String>> = HashMap::new();
    let mut snapshot = PluginStateSnapshot::default();
    for requested in &request.targets {
        let id = requested.target.device_id();
        let Some(device) = devices.get(id) else {
            snapshot.errors.push(PluginReadError {
                target: requested.target.clone(),
                diagnostic: "WLED device is unavailable".to_owned(),
            });
            continue;
        };
        let state = states.entry(id.to_owned()).or_insert_with(|| {
            get_json::<State>(&device.endpoint, "/json/state").map_err(|error| error.to_string())
        });
        match state {
            Ok(state) => match read_target(device, state, &requested.target, &requested.facets) {
                Ok(observations) => snapshot.observations.extend(observations),
                Err(error) => snapshot.errors.push(PluginReadError {
                    target: requested.target.clone(),
                    diagnostic: error.to_string(),
                }),
            },
            Err(error) => snapshot.errors.push(PluginReadError {
                target: requested.target.clone(),
                diagnostic: error.clone(),
            }),
        }
    }
    Ok(snapshot)
}

fn read_target(
    device: &Device,
    state: &State,
    target: &PluginTarget,
    facets: &[StateFacetKind],
) -> Result<Vec<PluginFacetObservation>, ApplyError> {
    let scope = resolve_scope(device, target)?;
    let segments = valid_segments(state.segments.clone(), device.led_count);
    let selected: Vec<&Segment> = match scope {
        Scope::Device | Scope::Segments => segments.iter().collect(),
        Scope::Segment(id) => vec![
            segments
                .iter()
                .find(|segment| segment.id == id)
                .ok_or_else(|| ApplyError::Io(format!("WLED omitted segment {id} from state")))?,
        ],
    };
    let mut observations = Vec::new();
    for facet in facets {
        let value = match facet {
            StateFacetKind::Appearance => appearance(device, &selected)?,
            StateFacetKind::Brightness => {
                let value = match scope {
                    Scope::Device => state.bri,
                    Scope::Segments => common_brightness(&selected)?,
                    Scope::Segment(_) => selected.first().map_or(0, |segment| segment.bri),
                };
                FacetValue::Brightness(u32::from(value))
            }
            StateFacetKind::Emission => {
                let emitting = state.on
                    && state.bri != 0
                    && selected
                        .iter()
                        .any(|segment| segment.on && segment.bri != 0);
                FacetValue::Emission(if emitting {
                    EmissionState::Emitting
                } else {
                    EmissionState::Dark
                })
            }
            StateFacetKind::PhysicalPower if matches!(scope, Scope::Device) => {
                FacetValue::PhysicalPower(if state.on {
                    PhysicalPowerState::On
                } else {
                    PhysicalPowerState::Off
                })
            }
            StateFacetKind::PhysicalPower => {
                return Err(ApplyError::Unsupported(
                    "WLED segments do not own physical power".to_owned(),
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
                    "WLED targets do not have appearance slots".to_owned(),
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

fn common_brightness(segments: &[&Segment]) -> Result<u8, ApplyError> {
    let first = segments
        .first()
        .ok_or_else(|| ApplyError::Io("WLED state contains no active segments".to_owned()))?
        .bri;
    if segments.iter().any(|segment| segment.bri != first) {
        Err(ApplyError::Unsupported(
            "WLED segments have different brightness values; read them individually".to_owned(),
        ))
    } else {
        Ok(first)
    }
}

fn appearance(device: &Device, segments: &[&Segment]) -> Result<FacetValue, ApplyError> {
    let first = segments
        .first()
        .ok_or_else(|| ApplyError::Io("WLED state contains no active segments".to_owned()))?;
    if segments.iter().any(|segment| {
        segment.fx != first.fx
            || segment.sx != first.sx
            || first_colour(segment) != first_colour(first)
    }) {
        return Err(ApplyError::Unsupported(
            "WLED segments have different appearances; read them individually".to_owned(),
        ));
    }
    let effect_index = usize::from(first.fx);
    let static_fallback = device
        .effects
        .get(effect_index)
        .is_none_or(|name| is_reserved_effect(name));
    if effect_index == solid_effect(&device.effects) || static_fallback {
        let colour = first_colour(first).unwrap_or(Rgb::new(0, 0, 0));
        Ok(FacetValue::Appearance(AppearanceState::Static(
            Colour::rgb(colour),
        )))
    } else {
        Ok(FacetValue::Appearance(AppearanceState::Effect(
            Effect::Hardware {
                id: HardwareEffectId::new("wled-effect"),
                arguments: EffectArguments {
                    speed: Some(u16::from(first.sx)),
                    choice: Some(first.fx.to_string()),
                    ..EffectArguments::default()
                },
            },
        )))
    }
}

fn first_colour(segment: &Segment) -> Option<Rgb> {
    let colour = segment.col.first()?;
    Some(Rgb::new(*colour.first()?, *colour.get(1)?, *colour.get(2)?))
}

luminate_export_plugin! {
    plugin: Wled,
    name: NAME,
    version: VERSION,
    priority: 80,
    recommended_reconciliation: Some(ReconciliationPolicy::Adopt),
    buses: BUSES,
    vendors: VENDORS,
    probe_hints: HINTS,
    settings: SETTINGS,
    start: native,
    rescan: native,
    batch: default,
    read_state: native,
    frame_upload: native,
    shm_frame: none,
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
