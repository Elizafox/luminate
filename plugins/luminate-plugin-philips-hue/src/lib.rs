// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Experimental local Philips Hue Bridge API v2 plugin.

mod api;
mod colour;
mod configuration;
mod http;
mod mdns;
mod mutation;
mod readback;
mod setup;
mod tls;
mod topology;

use std::ffi::CStr;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use luminate_core::control::ReconciliationPolicy;
use luminate_plugin_api::configuration as host_configuration;
use luminate_plugin_api::sdk::{
    DiscoveryPacer, DynamicDeviceRegistry, LuminatePlugin, ReadablePlugin, RegistryExpiry,
    RegistryPoisonError, RegistryRefresh, RescanPlugin, SetupPlugin, StartPlugin,
};
use luminate_plugin_api::{
    DeviceDescriptor, PluginBus, PluginError, PluginProbeHint, PluginReadRequest,
    PluginSettingDescriptor, PluginSetupRequest, PluginSetupStep, PluginSetupWorkflowDescriptor,
    PluginSetupWorkflowKind, PluginStateSnapshot, PluginUpdate, PluginVendorId, ProbeOutcome,
    RescanReason, luminate_export_plugin,
};

use crate::configuration::HueConfiguration;

const NAME: &CStr = c"luminate-plugin-philips-hue";
const VERSION: &CStr = c"0.1.0";
const MAX_LIGHTS: usize = 4_096;
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(30);
const DEVICE_EXPIRY: Duration = Duration::from_secs(300);

static BUSES: &[PluginBus] = &[PluginBus::Network];
static VENDORS: &[PluginVendorId] = &[];
static HINTS: &[PluginProbeHint] = &[];
static SETTINGS: &[PluginSettingDescriptor] = &[
    string_setting_without_default(
        c"bridge_id",
        c"Bridge ID",
        c"The 16-digit identity printed or reported by the intended Hue Bridge.",
        true,
        false,
    ),
    string_setting_without_default(
        c"application_key",
        c"Application key",
        c"A pre-created local Hue application key. This value is never included in diagnostics.",
        true,
        true,
    ),
    PluginSettingDescriptor {
        key: c"mdns".as_ptr(),
        label: c"mDNS discovery".as_ptr(),
        description: c"Discover the configured bridge on the local network when no endpoint is configured."
            .as_ptr(),
        kind: luminate_plugin_api::PluginSettingKind::Boolean as u32,
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
    string_setting_without_default(
        c"endpoint",
        c"Bridge endpoint",
        c"Optional hostname or IP address and port used for routing without weakening bridge authentication.",
        false,
        false,
    ),
];

const fn string_setting_without_default(
    key: &'static CStr,
    label: &'static CStr,
    description: &'static CStr,
    required: bool,
    sensitive: bool,
) -> PluginSettingDescriptor {
    let mut setting =
        PluginSettingDescriptor::string(key, label, description, c"", required, sensitive);
    setting.default_toml = ptr::null();
    setting
}

static SETUP_WORKFLOWS: &[PluginSetupWorkflowDescriptor] = &[PluginSetupWorkflowDescriptor::new(
    c"push-link",
    c"Connect Hue Bridge",
    c"Discover a local Hue Bridge and create an application key using its link button.",
    PluginSetupWorkflowKind::Provision,
)];

struct PhilipsHue {
    runtime: Arc<Runtime>,
}

struct Runtime {
    configuration: HueConfiguration,
    http: http::HttpClient,
    rate_limiter: mutation::RateLimiter,
    devices: DynamicDeviceRegistry<topology::HueDevice, topology::TopologyFingerprint>,
    discovery: DiscoveryPacer,
    discovery_started: AtomicBool,
}

impl ReadablePlugin for PhilipsHue {
    fn read_state(
        &self,
        _context: &luminate_plugin_api::PluginRequestContext,
        request: &PluginReadRequest,
    ) -> Result<PluginStateSnapshot, PluginError> {
        let devices = self
            .runtime
            .devices
            .snapshot()
            .map_err(|error| PluginError::Internal(error.to_string()))?;
        Ok(readback::read(&self.runtime.http, &devices, request))
    }
}

impl LuminatePlugin for PhilipsHue {
    fn new() -> Result<Self, PluginError> {
        let configuration = host_configuration::deserialize::<HueConfiguration>()
            .map_err(|error| PluginError::InvalidArgument(error.to_string()))?;
        let http = http::HttpClient::new(
            configuration.bridge_id.clone(),
            configuration.application_key.clone(),
        )
        .map_err(|error| PluginError::Internal(error.to_string()))?;

        Ok(Self {
            runtime: Arc::new(Runtime {
                configuration,
                http,
                rate_limiter: mutation::RateLimiter::hue_lights(),
                devices: DynamicDeviceRegistry::new(
                    MAX_LIGHTS,
                    DEVICE_EXPIRY,
                    RegistryExpiry::After,
                ),
                discovery: DiscoveryPacer::new(),
                discovery_started: AtomicBool::new(false),
            }),
        })
    }

    fn probe(&self) -> ProbeOutcome {
        // The configured bridge may appear after daemon startup or receive a
        // new address. Discovery starts only after the host accepts the plugin.
        ProbeOutcome::Dormant
    }

    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
        let devices = self
            .runtime
            .devices
            .snapshot()
            .map_err(|error| PluginError::Internal(error.to_string()))?;
        Ok(devices.iter().map(topology::descriptor).collect())
    }

    fn apply(
        &self,
        _context: &luminate_plugin_api::PluginRequestContext,
        update: &PluginUpdate,
    ) -> Result<(), PluginError> {
        let device_id = update.target.device_id();
        let device = self
            .runtime
            .devices
            .get(device_id)
            .map_err(|error| PluginError::Internal(error.to_string()))?
            .ok_or_else(|| {
                PluginError::Unavailable(format!("Hue light is unavailable: {device_id}"))
            })?;
        mutation::apply(
            &self.runtime.http,
            &self.runtime.rate_limiter,
            &device,
            update,
        )
    }
}

impl SetupPlugin for PhilipsHue {
    fn setup(request: PluginSetupRequest) -> Result<PluginSetupStep, PluginError> {
        setup::run(request)
    }
}

impl StartPlugin for PhilipsHue {
    fn start(&self) {
        start_discovery_thread(&self.runtime);
    }
}

impl RescanPlugin for PhilipsHue {
    fn rescan(&self, _reason: RescanReason) {
        self.runtime.discovery.wake();
    }
}

fn start_discovery_thread(runtime: &Arc<Runtime>) {
    if runtime
        .discovery_started
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let thread_runtime = Arc::clone(runtime);
    match thread::Builder::new()
        .name("luminate-hue-discovery".to_owned())
        .spawn(move || discovery_loop(&thread_runtime))
    {
        Ok(_thread) => {}
        Err(_error) => {
            runtime.discovery_started.store(false, Ordering::Release);
        }
    }
}

fn discovery_loop(runtime: &Runtime) {
    loop {
        if discovery_cycle(runtime).is_err() {
            return;
        }
        runtime.discovery.wait(DISCOVERY_INTERVAL);
    }
}

fn discovery_cycle(runtime: &Runtime) -> Result<(), RegistryPoisonError> {
    let endpoints = runtime.configuration.endpoint.clone().map_or_else(
        || {
            if runtime.configuration.mdns {
                mdns::discover(&runtime.configuration.bridge_id).unwrap_or_default()
            } else {
                Vec::new()
            }
        },
        |endpoint| vec![endpoint],
    );
    refresh_from_endpoints(
        &runtime.http,
        &runtime.configuration.bridge_id,
        &runtime.devices,
        &endpoints,
        Instant::now(),
    )
    .map(|_| ())
}

fn refresh_from_endpoints(
    transport: &impl api::Transport,
    bridge_id: &configuration::BridgeId,
    devices: &DynamicDeviceRegistry<topology::HueDevice, topology::TopologyFingerprint>,
    endpoints: &[configuration::Endpoint],
    now: Instant,
) -> Result<RegistryRefresh, RegistryPoisonError> {
    for endpoint in endpoints {
        match api::enumerate(transport, endpoint, bridge_id) {
            Ok(snapshot) => {
                let entries = topology::registry_entries(&snapshot, endpoint, bridge_id);
                return devices.refresh(now, entries);
            }
            Err(_error) => {}
        }
    }
    devices.refresh(now, Vec::new())
}

luminate_export_plugin! {
    plugin: PhilipsHue,
    name: NAME,
    version: VERSION,
    priority: 80,
    recommended_reconciliation: Some(ReconciliationPolicy::Adopt),
    buses: BUSES,
    vendors: VENDORS,
    probe_hints: HINTS,
    settings: SETTINGS,
    setup_workflows: SETUP_WORKFLOWS,
    setup: native,
    start: native,
    rescan: native,
    batch: default,
    read_state: native,
    frame_upload: none,
    shm_frame: none,
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::slice;

    use super::*;

    struct FakeTransport {
        failing_host: Option<String>,
    }

    impl api::Transport for FakeTransport {
        fn get_json(
            &self,
            endpoint: &configuration::Endpoint,
            path: &str,
        ) -> Result<Vec<u8>, http::HttpError> {
            if self.failing_host.as_deref() == Some(endpoint.host()) {
                return Err(http::HttpError::TimedOut);
            }
            let response = match path {
                "/clip/v2/resource/bridge" => BRIDGE_RESPONSE,
                "/clip/v2/resource/device" => DEVICE_RESPONSE,
                "/clip/v2/resource/light" => LIGHT_RESPONSE,
                _ => return Err(http::HttpError::Protocol("unexpected test path".to_owned())),
            };
            Ok(response.to_vec())
        }

        fn put_json(
            &self,
            _endpoint: &configuration::Endpoint,
            _path: &str,
            _body: &[u8],
        ) -> Result<Vec<u8>, http::HttpError> {
            Err(http::HttpError::Protocol("unexpected test PUT".to_owned()))
        }
    }

    const BRIDGE_RESPONSE: &[u8] = br#"{"errors":[],"data":[{"id":"11111111-1111-4111-8111-111111111111","type":"bridge","bridge_id":"001788fffe123456"}]}"#;
    const DEVICE_RESPONSE: &[u8] = br#"{"errors":[],"data":[{"id":"22222222-2222-4222-8222-222222222222","type":"device","product_data":{"model_id":"test-model","manufacturer_name":"Test Vendor","product_name":"Test Lamp","certified":true,"software_version":"1.2.3"},"metadata":{"name":"Test light"},"services":[{"rid":"33333333-3333-4333-8333-333333333333","rtype":"light"}]}]}"#;
    const LIGHT_RESPONSE: &[u8] = br#"{"errors":[],"data":[{"id":"33333333-3333-4333-8333-333333333333","owner":{"rid":"22222222-2222-4222-8222-222222222222","rtype":"device"},"type":"light","on":{"on":true},"dimming":{"brightness":50.0}}]}"#;

    fn endpoint(last_octet: u8) -> configuration::Endpoint {
        configuration::Endpoint::from_ip(IpAddr::V4(Ipv4Addr::new(192, 0, 2, last_octet)), 443)
            .expect("endpoint")
    }

    fn registry() -> DynamicDeviceRegistry<topology::HueDevice, topology::TopologyFingerprint> {
        DynamicDeviceRegistry::new(MAX_LIGHTS, DEVICE_EXPIRY, RegistryExpiry::After)
    }

    #[test]
    fn settings_mark_the_application_key_sensitive() {
        let application_key = SETTINGS.get(1).expect("application key setting");

        assert!(application_key.required);
        assert!(application_key.sensitive);
    }

    #[test]
    fn string_settings_without_defaults_use_null_pointers() {
        for index in [0, 1, 3] {
            let setting = SETTINGS.get(index).expect("string setting");
            assert!(setting.default_toml.is_null());
        }
    }

    #[test]
    fn plugin_recommends_adopt_reconciliation() {
        assert_eq!(
            luminate_plugin_api::reconciliation_policy_from_abi(
                LUMINATE_PLUGIN_DESCRIPTOR.recommended_reconciliation
            ),
            Some(Some(ReconciliationPolicy::Adopt))
        );
    }

    #[test]
    fn refresh_tries_candidates_and_updates_routes_without_topology_churn() {
        let registry = registry();
        let bridge_id = configuration::BridgeId::parse("001788fffe123456").expect("bridge ID");
        let first = endpoint(1);
        let second = endpoint(2);
        let transport = FakeTransport {
            failing_host: Some(first.host().to_owned()),
        };
        let now = Instant::now();

        let initial = refresh_from_endpoints(
            &transport,
            &bridge_id,
            &registry,
            &[first, second.clone()],
            now,
        )
        .expect("initial refresh");
        assert!(initial.topology_changed);
        assert_eq!(registry.snapshot().expect("snapshot")[0].endpoint, second);

        let replacement = endpoint(3);
        let refreshed = refresh_from_endpoints(
            &FakeTransport { failing_host: None },
            &bridge_id,
            &registry,
            slice::from_ref(&replacement),
            now + Duration::from_secs(1),
        )
        .expect("route refresh");
        assert!(!refreshed.topology_changed);
        assert_eq!(
            registry.snapshot().expect("refreshed snapshot")[0].endpoint,
            replacement
        );
    }

    #[test]
    fn missed_cycles_retain_then_expire_the_light() {
        let registry = registry();
        let bridge_id = configuration::BridgeId::parse("001788fffe123456").expect("bridge ID");
        let now = Instant::now();
        refresh_from_endpoints(
            &FakeTransport { failing_host: None },
            &bridge_id,
            &registry,
            &[endpoint(1)],
            now,
        )
        .expect("initial refresh");

        let boundary = refresh_from_endpoints(
            &FakeTransport { failing_host: None },
            &bridge_id,
            &registry,
            &[],
            now + DEVICE_EXPIRY,
        )
        .expect("boundary refresh");
        assert_eq!(boundary.expired, 0);
        assert_eq!(registry.snapshot().expect("retained snapshot").len(), 1);

        let expired = refresh_from_endpoints(
            &FakeTransport { failing_host: None },
            &bridge_id,
            &registry,
            &[],
            now + DEVICE_EXPIRY + Duration::from_nanos(1),
        )
        .expect("expiry refresh");
        assert_eq!(expired.expired, 1);
        assert!(expired.topology_changed);
        assert!(registry.snapshot().expect("expired snapshot").is_empty());
    }
}
