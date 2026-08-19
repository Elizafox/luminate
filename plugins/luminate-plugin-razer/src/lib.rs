// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unnecessary_wraps,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Native Razer lighting plugin.

use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::Mutex;

use luminate_core::capability::{ColourEncoding, EffectDirection};
use luminate_core::colour::{Colour, Rgb8Error};
use luminate_core::control::ReconciliationPolicy;
use luminate_core::effect::{Effect, EffectArguments};
use luminate_core::frame::{FrameEnvelope, FramePayload};
use luminate_core::rgb::Rgb as CoreRgb;
use luminate_plugin_api::configuration;
use luminate_plugin_api::sdk::{
    BatchPlugin, CompleteShadow, CompleteShadowError, FrameStreamingPlugin, LuminatePlugin,
    stage_frame_updates,
};
use luminate_plugin_api::{
    DeviceDescriptor, PluginBus, PluginError, PluginProbeHint, PluginRequestContext,
    PluginSettingApplyMode, PluginSettingDescriptor, PluginSettingKind, PluginTarget, PluginUpdate,
    PluginUpdateOperation, PluginVendorId, ProbeHintKind, ProbeOutcome, luminate_export_plugin,
};
use serde::Deserialize;

use protocol::commands::{
    ColourMode, Command, Direction, LedTarget, MatrixEffect, ReactiveSpeed, Rgb, StarlightSpeed,
    Storage,
};
use protocol::transport::{DeviceOpener as _, HidApiOpener};

mod devices;
mod protocol;
mod topology;

const NAME: &CStr = c"luminate-plugin-razer";
const VERSION: &CStr = c"0.1.0";
static BUSES: &[PluginBus] = &[PluginBus::Hid];
static VENDORS: &[PluginVendorId] = &[
    PluginVendorId {
        vendor: 0x1532,
        product: 0x028d,
    },
    PluginVendorId {
        vendor: 0x1532,
        product: 0x0287,
    },
    PluginVendorId {
        vendor: 0x1532,
        product: 0x02a5,
    },
    PluginVendorId {
        vendor: 0x1532,
        product: 0x02b9,
    },
    PluginVendorId {
        vendor: 0x1532,
        product: 0x02d7,
    },
    PluginVendorId {
        vendor: 0x1532,
        product: 0x0258,
    },
    PluginVendorId {
        vendor: 0x1532,
        product: 0x0282,
    },
    PluginVendorId {
        vendor: 0x1532,
        product: 0x0266,
    },
    PluginVendorId {
        vendor: 0x1532,
        product: 0x026b,
    },
    PluginVendorId {
        vendor: 0x1532,
        product: 0x026c,
    },
    PluginVendorId {
        vendor: 0x1532,
        product: 0x02a6,
    },
    PluginVendorId {
        vendor: 0x1532,
        product: 0x02a7,
    },
    PluginVendorId {
        vendor: 0x1532,
        product: 0x0295,
    },
    PluginVendorId {
        vendor: 0x1532,
        product: 0x0292,
    },
    PluginVendorId {
        vendor: 0x1532,
        product: 0x0298,
    },
];
static HINTS: &[PluginProbeHint] = &[
    hid_hint(c"1532:028d"),
    hid_hint(c"1532:0287"),
    hid_hint(c"1532:02a5"),
    hid_hint(c"1532:02b9"),
    hid_hint(c"1532:02d7"),
    hid_hint(c"1532:0258"),
    hid_hint(c"1532:0282"),
    hid_hint(c"1532:0266"),
    hid_hint(c"1532:026b"),
    hid_hint(c"1532:026c"),
    hid_hint(c"1532:02a6"),
    hid_hint(c"1532:02a7"),
    hid_hint(c"1532:0295"),
    hid_hint(c"1532:0292"),
    hid_hint(c"1532:0298"),
];

const fn hid_hint(value: &'static CStr) -> PluginProbeHint {
    PluginProbeHint {
        kind: ProbeHintKind::HidVidPid,
        value: value.as_ptr().cast(),
    }
}
static SETTINGS: &[PluginSettingDescriptor] = &[PluginSettingDescriptor {
    key: c"enable_untested_devices".as_ptr(),
    label: c"Enable untested devices".as_ptr(),
    description:
        c"Discover exact imported Razer profiles that have not yet received hardware validation."
            .as_ptr(),
    kind: PluginSettingKind::Boolean as u32,
    default_toml: c"true".as_ptr(),
    required: false,
    sensitive: false,
    apply_mode: PluginSettingApplyMode::RestartRequired as u32,
    minimum: 0.0,
    maximum: 0.0,
    has_minimum: false,
    has_maximum: false,
    constraints: ptr::null(),
}];

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RazerConfig {
    enable_untested_devices: bool,
}

impl Default for RazerConfig {
    fn default() -> Self {
        Self {
            enable_untested_devices: true,
        }
    }
}

struct Razer {
    enable_untested_devices: bool,
    frame_shadows: Mutex<HashMap<String, Vec<Rgb>>>,
    current_states: Mutex<HashMap<String, CurrentState>>,
    warned_untested: Mutex<HashSet<&'static str>>,
}

#[derive(Debug, Default)]
struct CurrentState {
    effect: Option<MatrixEffect>,
    brightness: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiscoveredRazer {
    id: String,
    profile: devices::Profile,
    path: CString,
    physical_identity: String,
}

impl LuminatePlugin for Razer {
    fn new() -> Result<Self, PluginError> {
        for profile in devices::PROFILES {
            profile
                .validate()
                .map_err(|error| PluginError::Internal(error.to_string()))?;
        }
        let config = configuration::deserialize::<RazerConfig>().map_err(|error| {
            PluginError::InvalidArgument(format!("invalid Razer plugin configuration: {error}"))
        })?;
        Ok(Self {
            enable_untested_devices: config.enable_untested_devices,
            frame_shadows: Mutex::new(HashMap::new()),
            current_states: Mutex::new(HashMap::new()),
            warned_untested: Mutex::new(HashSet::new()),
        })
    }

    fn probe(&self) -> ProbeOutcome {
        tracing::debug!(
            enable_untested_devices = self.enable_untested_devices,
            "probing exact Razer HID profiles"
        );
        match hidapi::HidApi::new() {
            Ok(api)
                if discovered_devices(&api, self.enable_untested_devices)
                    .next()
                    .is_some() =>
            {
                ProbeOutcome::Ready
            }
            Ok(_) => ProbeOutcome::Unsupported,
            Err(error) => {
                tracing::warn!(error = %error, "failed to enumerate Razer HID interfaces");
                ProbeOutcome::Unsupported
            }
        }
    }

    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
        self.frame_shadows
            .lock()
            .expect("Razer frame shadow lock poisoned")
            .clear();
        let api =
            hidapi::HidApi::new().map_err(|error| PluginError::Unavailable(error.to_string()))?;
        let devices = discovered_devices(&api, self.enable_untested_devices).collect::<Vec<_>>();
        let mut warned = self
            .warned_untested
            .lock()
            .expect("Razer warning-state lock poisoned");
        Ok(devices.into_iter().map(|device| {
                let profile = device.profile;
                if profile.validation == devices::ValidationStatus::Untested {
                    if warned.insert(profile.slug) {
                        tracing::warn!(model = profile.name, vid = format_args!("{:04x}", profile.vendor_id), pid = format_args!("{:04x}", profile.product_id), "Razer device uses an untested Luminate profile; lighting is enabled, but hardware reports are welcome");
                    }
                    descriptor_for(&device, topology::untested_matrix_keyboard(profile))
                } else {
                    descriptor_for(&device, topology::blackwidow_v4_pro(profile))
                }
            }).collect())
    }

    fn apply(
        &self,
        _context: &PluginRequestContext,
        update: &PluginUpdate,
    ) -> Result<(), PluginError> {
        let selected = device_for_target(&update.target, self.enable_untested_devices)?;
        let profile = selected.profile;
        if let PluginTarget::Element {
            device,
            surface,
            element,
        } = &update.target
        {
            if device != &selected.id || surface != topology::SURFACE_ID {
                return Err(PluginError::InvalidTarget(
                    "target is not an element of the Razer lighting surface".to_owned(),
                ));
            }
            let (row, column) = topology::coordinate_for(profile, element).ok_or_else(|| {
                PluginError::InvalidTarget(format!("unknown Razer lighting element {element}"))
            })?;
            let colour = lower_element_operation(&update.operation)?;
            self.invalidate_persistable_effect(&selected)?;
            let mut shadows = self
                .frame_shadows
                .lock()
                .expect("Razer frame shadow lock poisoned");
            let pixels = frame_with_element(
                profile,
                shadows.get(&selected.id).map(Vec::as_slice),
                row,
                column,
                colour,
            )?;
            shadows.remove(&selected.id);
            upload_rgb_frame(&selected, &pixels)?;
            shadows.insert(selected.id.clone(), pixels);
            return Ok(());
        }

        ensure_target(&selected, &update.target)?;
        if matches!(update.operation, PluginUpdateOperation::SaveCurrent) {
            return self.save_current(&selected);
        }
        let command = lower_operation(profile, &update.operation, Storage::Volatile)?;
        if !matches!(
            update.operation,
            PluginUpdateOperation::SetBrightness { .. }
        ) {
            self.frame_shadows
                .lock()
                .expect("Razer frame shadow lock poisoned")
                .remove(&selected.id);
        }
        let channel = HidApiOpener
            .open_path(&selected.path)
            .map_err(PluginError::Unavailable)?;
        protocol::transport::execute(channel.as_ref(), &command, profile.response_delay)
            .map_err(|error| PluginError::Io(error.to_string()))?;
        self.remember_update(&selected, &update.operation)?;
        Ok(())
    }
}

impl Razer {
    fn invalidate_persistable_effect(&self, device: &DiscoveredRazer) -> Result<(), PluginError> {
        self.current_states
            .lock()
            .expect("Razer current-state lock poisoned")
            .entry(device.id.clone())
            .or_default()
            .effect = None;
        Ok(())
    }

    fn remember_update(
        &self,
        device: &DiscoveredRazer,
        operation: &PluginUpdateOperation,
    ) -> Result<(), PluginError> {
        let mut states = self
            .current_states
            .lock()
            .expect("Razer current-state lock poisoned");
        let state = states.entry(device.id.clone()).or_default();
        match operation {
            PluginUpdateOperation::SetBrightness { value } => {
                state.brightness = Some(u8::try_from(*value).map_err(|_| {
                    PluginError::InvalidArgument(format!("Razer brightness {value} exceeds 255"))
                })?);
            }
            PluginUpdateOperation::SetEffect { .. } | PluginUpdateOperation::Clear => {
                state.effect = Some(lower_effect(device.profile, operation)?);
            }
            PluginUpdateOperation::SetAppearanceSlots { .. }
            | PluginUpdateOperation::SaveCurrent => {}
        }
        Ok(())
    }

    fn save_current(&self, device: &DiscoveredRazer) -> Result<(), PluginError> {
        let profile = device.profile;
        if profile.validation == devices::ValidationStatus::Untested {
            return Err(PluginError::Unsupported(
                "persistence is not enabled for untested Razer profiles".to_owned(),
            ));
        }
        let (effect, brightness) = {
            let states = self
                .current_states
                .lock()
                .expect("Razer current-state lock poisoned");
            let state = states.get(&device.id).ok_or_else(|| {
                PluginError::Unsupported(
                    "Razer cannot save a state that was not established in this plugin session"
                        .to_owned(),
                )
            })?;
            let effect = state.effect.ok_or_else(|| {
                PluginError::Unsupported(
                    "Razer cannot save a state that was not established in this plugin session"
                        .to_owned(),
                )
            })?;
            (effect, state.brightness)
        };
        let channel = HidApiOpener
            .open_path(&device.path)
            .map_err(PluginError::Unavailable)?;
        let effect = Command::extended_matrix_effect(
            profile.transaction_id,
            Storage::Variable,
            LedTarget::Backlight,
            effect,
        )
        .map_err(|error| PluginError::Internal(error.to_string()))?;
        protocol::transport::execute(channel.as_ref(), &effect, profile.response_delay)
            .map_err(|error| PluginError::Io(error.to_string()))?;
        if let Some(brightness) = brightness {
            let brightness = Command::set_brightness(
                profile.transaction_id,
                Storage::Variable,
                LedTarget::Backlight,
                brightness,
            )
            .map_err(|error| PluginError::Internal(error.to_string()))?;
            protocol::transport::execute(channel.as_ref(), &brightness, profile.response_delay)
                .map_err(|error| PluginError::Io(error.to_string()))?;
        }
        Ok(())
    }
}

impl BatchPlugin for Razer {
    fn apply_batch(
        &self,
        context: &PluginRequestContext,
        updates: &[PluginUpdate],
    ) -> Vec<Result<(), PluginError>> {
        let batch_device = updates.first().and_then(|update| {
            device_for_target(&update.target, self.enable_untested_devices).ok()
        });
        if batch_device.is_none()
            || updates.iter().any(|update| {
                !matches!(update.target, PluginTarget::Element { .. })
                    || device_for_target(&update.target, self.enable_untested_devices).ok()
                        != batch_device
            })
        {
            return updates
                .iter()
                .map(|update| self.apply(context, update))
                .collect();
        }

        let Some(device) = batch_device else {
            return Vec::new();
        };
        let profile = device.profile;
        let mut shadows = self
            .frame_shadows
            .lock()
            .expect("Razer frame shadow lock poisoned");
        let Some(mut pixels) = shadows.get(&device.id).cloned() else {
            return repeated_batch_error(
                updates,
                &PluginError::Unsupported(
                    "Razer element updates require a complete frame written in this plugin session"
                        .to_owned(),
                ),
            );
        };
        if pixels.len() != usize::from(profile.rows) * usize::from(profile.columns) {
            return repeated_batch_error(
                updates,
                &PluginError::Internal("Razer frame shadow has the wrong size".to_owned()),
            );
        }
        let mut outcomes = Vec::with_capacity(updates.len());

        for update in updates {
            let PluginTarget::Element {
                device: target_device,
                surface,
                element,
            } = &update.target
            else {
                outcomes.push(Err(PluginError::Internal(
                    "non-element update reached Razer element batch".to_owned(),
                )));
                continue;
            };
            let outcome = (|| {
                if target_device != &device.id || surface != topology::SURFACE_ID {
                    return Err(PluginError::InvalidTarget(
                        "target is not an element of the Razer lighting surface".to_owned(),
                    ));
                }
                let (row, column) =
                    topology::coordinate_for(profile, element).ok_or_else(|| {
                        PluginError::InvalidTarget(format!(
                            "unknown Razer lighting element {element}"
                        ))
                    })?;
                let colour = lower_element_operation(&update.operation)?;
                let index = usize::from(row) * usize::from(profile.columns) + usize::from(column);
                let pixel = pixels.get_mut(index).ok_or_else(|| {
                    PluginError::Internal("Razer element coordinate exceeds the frame".to_owned())
                })?;
                *pixel = colour;
                Ok(())
            })();
            outcomes.push(outcome);
        }

        if outcomes.iter().any(Result::is_ok) {
            if let Err(error) = self.invalidate_persistable_effect(&device) {
                for outcome in &mut outcomes {
                    if outcome.is_ok() {
                        *outcome = Err(copy_plugin_error(&error));
                    }
                }
                return outcomes;
            }
            shadows.remove(&device.id);
            match upload_rgb_frame(&device, &pixels) {
                Ok(()) => {
                    shadows.insert(device.id.clone(), pixels);
                }
                Err(error) => {
                    for outcome in &mut outcomes {
                        if outcome.is_ok() {
                            *outcome = Err(copy_plugin_error(&error));
                        }
                    }
                }
            }
        }

        outcomes
    }
}

impl FrameStreamingPlugin for Razer {
    fn upload_frame(
        &self,
        _context: &PluginRequestContext,
        target: &PluginTarget,
        envelope: &FrameEnvelope,
    ) -> Result<(), PluginError> {
        let PluginTarget::Surface { surface, .. } = target else {
            return Err(PluginError::InvalidTarget(
                "Razer frame upload requires the lighting surface".to_owned(),
            ));
        };
        let device = device_for_target(target, self.enable_untested_devices)?;
        let profile = device.profile;
        if surface != topology::SURFACE_ID {
            return Err(PluginError::InvalidTarget(
                "target is not the Razer matrix surface".to_owned(),
            ));
        }
        let FramePayload::Full(pixels) = &envelope.payload else {
            return Err(PluginError::InvalidArgument(
                "Razer matrix accepts only complete frames".to_owned(),
            ));
        };
        let expected = usize::from(profile.rows) * usize::from(profile.columns);
        if pixels.len() != expected {
            return Err(PluginError::InvalidArgument(format!(
                "Razer matrix frame has {} pixels, expected {expected}",
                pixels.len()
            )));
        }

        let colours = pixels
            .iter()
            .map(rgb_from_colour)
            .collect::<Result<Vec<_>, _>>()?;
        self.invalidate_persistable_effect(&device)?;
        let mut shadows = self
            .frame_shadows
            .lock()
            .expect("Razer frame shadow lock poisoned");
        shadows.remove(&device.id);
        upload_rgb_frame(&device, &colours)?;
        shadows.insert(device.id.clone(), colours);
        Ok(())
    }
}

fn frame_with_element(
    profile: devices::Profile,
    shadow: Option<&[Rgb]>,
    row: u8,
    column: u8,
    colour: Rgb,
) -> Result<Vec<Rgb>, PluginError> {
    let index = usize::from(row) * usize::from(profile.columns) + usize::from(column);
    stage_frame_updates(
        &shadow.map_or(CompleteShadow::Unknown, CompleteShadow::Complete),
        usize::from(profile.rows) * usize::from(profile.columns),
        [(index, colour)],
    )
    .map_err(|error| match error {
        CompleteShadowError::Unknown => PluginError::Unsupported(
            "Razer element updates require a complete frame written in this plugin session"
                .to_owned(),
        ),
        CompleteShadowError::WrongLength => {
            PluginError::Internal("Razer frame shadow has the wrong size".to_owned())
        }
        CompleteShadowError::UnknownElement => {
            PluginError::Internal("Razer element coordinate exceeds the frame".to_owned())
        }
    })
}

fn lower_element_operation(operation: &PluginUpdateOperation) -> Result<Rgb, PluginError> {
    match operation {
        PluginUpdateOperation::SetEffect {
            effect: Effect::Static { colour },
        } => rgb_from_colour(colour),
        PluginUpdateOperation::SetEffect {
            effect: Effect::Off,
        }
        | PluginUpdateOperation::Clear => Ok(Rgb {
            red: 0,
            green: 0,
            blue: 0,
        }),
        PluginUpdateOperation::SetEffect { .. }
        | PluginUpdateOperation::SetBrightness { .. }
        | PluginUpdateOperation::SetAppearanceSlots { .. }
        | PluginUpdateOperation::SaveCurrent => Err(PluginError::Unsupported(
            "Razer elements support only static RGB, off, and clear".to_owned(),
        )),
    }
}

fn upload_rgb_frame(device: &DiscoveredRazer, colours: &[Rgb]) -> Result<(), PluginError> {
    let profile = device.profile;
    let expected = usize::from(profile.rows) * usize::from(profile.columns);
    if colours.len() != expected {
        return Err(PluginError::InvalidArgument(format!(
            "Razer matrix frame has {} pixels, expected {expected}",
            colours.len()
        )));
    }

    let channel = HidApiOpener
        .open_path(&device.path)
        .map_err(PluginError::Unavailable)?;
    let custom_mode = if profile.custom_frame_response {
        Command::custom_mode_required(profile.transaction_id)
    } else {
        Command::custom_mode(profile.transaction_id)
    }
    .map_err(|error| PluginError::Internal(error.to_string()))?;
    protocol::transport::execute(channel.as_ref(), &custom_mode, profile.response_delay)
        .map_err(|error| PluginError::Io(error.to_string()))?;
    for (row, row_colours) in colours
        .chunks_exact(usize::from(profile.columns))
        .enumerate()
    {
        let row = u8::try_from(row).map_err(|_| {
            PluginError::Internal("Razer frame row is not representable".to_owned())
        })?;
        let command = if profile.custom_frame_response {
            Command::custom_frame_row_required(profile.transaction_id, row, 0, row_colours)
        } else {
            Command::custom_frame_row(profile.transaction_id, row, 0, row_colours)
        }
        .map_err(|error| PluginError::InvalidArgument(error.to_string()))?;
        protocol::transport::execute(channel.as_ref(), &command, profile.response_delay)
            .map_err(|error| PluginError::Io(error.to_string()))?;
    }
    Ok(())
}

fn copy_plugin_error(error: &PluginError) -> PluginError {
    match error {
        PluginError::InvalidTarget(diagnostic) => PluginError::InvalidTarget(diagnostic.clone()),
        PluginError::InvalidArgument(diagnostic) => {
            PluginError::InvalidArgument(diagnostic.clone())
        }
        PluginError::Unsupported(diagnostic) => PluginError::Unsupported(diagnostic.clone()),
        PluginError::Unavailable(diagnostic) => PluginError::Unavailable(diagnostic.clone()),
        PluginError::RateLimited {
            diagnostic,
            retry_after,
        } => PluginError::RateLimited {
            diagnostic: diagnostic.clone(),
            retry_after: *retry_after,
        },
        PluginError::Io(diagnostic) => PluginError::Io(diagnostic.clone()),
        PluginError::Internal(diagnostic) => PluginError::Internal(diagnostic.clone()),
    }
}

fn repeated_batch_error(
    updates: &[PluginUpdate],
    error: &PluginError,
) -> Vec<Result<(), PluginError>> {
    updates
        .iter()
        .map(|_| Err(copy_plugin_error(error)))
        .collect()
}

fn device_for_target(
    target: &PluginTarget,
    enable_untested: bool,
) -> Result<DiscoveredRazer, PluginError> {
    let device = match target {
        PluginTarget::Device { device }
        | PluginTarget::Surface { device, .. }
        | PluginTarget::Element { device, .. }
        | PluginTarget::Group { device, .. } => device,
    };
    let api = hidapi::HidApi::new().map_err(|error| PluginError::Unavailable(error.to_string()))?;
    discovered_devices(&api, enable_untested)
        .find(|candidate| candidate.id == *device)
        .ok_or_else(|| PluginError::Unavailable(format!("Razer device {device} is unavailable")))
}

fn discovered_devices(
    api: &hidapi::HidApi,
    enable_untested: bool,
) -> impl Iterator<Item = DiscoveredRazer> + '_ {
    let candidates = api
        .device_list()
        .filter_map(move |info| {
            let profile = devices::PROFILES.iter().copied().find(|profile| {
                profile_enabled(*profile, enable_untested)
                    && info.vendor_id() == profile.vendor_id
                    && info.product_id() == profile.product_id
                    && info.interface_number() == profile.control_interface
            })?;
            let identity = info
                .serial_number()
                .map(str::trim)
                .filter(|serial| !serial.is_empty())
                .map_or_else(
                    || format!("path:{:016x}", stable_hash(info.path().to_bytes())),
                    |serial| format!("serial:{serial}"),
                );
            Some((profile, info.path().to_owned(), identity))
        })
        .collect::<Vec<_>>();
    prepare_discovered(candidates).into_iter()
}

fn prepare_discovered(
    mut candidates: Vec<(devices::Profile, CString, String)>,
) -> Vec<DiscoveredRazer> {
    retain_unique_identities(&mut candidates);
    candidates
        .into_iter()
        .map(|(profile, path, physical_identity)| {
            let id = format!(
                "razer-{}-{:016x}",
                profile.slug,
                stable_hash(physical_identity.as_bytes())
            );
            DiscoveredRazer {
                id,
                profile,
                path,
                physical_identity,
            }
        })
        .collect()
}

fn profile_enabled(profile: devices::Profile, enable_untested: bool) -> bool {
    enable_untested || profile.validation != devices::ValidationStatus::Untested
}

fn retain_unique_identities(candidates: &mut Vec<(devices::Profile, CString, String)>) {
    let mut counts = HashMap::new();
    for (_, _, identity) in candidates.iter() {
        *counts.entry(identity.clone()).or_insert(0_usize) += 1;
    }
    candidates.retain(|(_, _, identity)| counts.get(identity) == Some(&1));
}

fn stable_hash(value: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    value.iter().fold(OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
    })
}

fn descriptor_for(device: &DiscoveredRazer, mut descriptor: DeviceDescriptor) -> DeviceDescriptor {
    descriptor.id.clone_from(&device.id);
    if let Some(claim) = descriptor.claims.first_mut() {
        claim
            .physical_identity
            .clone_from(&device.physical_identity);
    }
    descriptor
}

fn ensure_target(device: &DiscoveredRazer, target: &PluginTarget) -> Result<(), PluginError> {
    let expected_device = &device.id;
    match target {
        PluginTarget::Device { device } if device == expected_device => Ok(()),
        PluginTarget::Surface { device, surface }
            if device == expected_device && surface == topology::SURFACE_ID =>
        {
            Ok(())
        }
        PluginTarget::Device { .. }
        | PluginTarget::Surface { .. }
        | PluginTarget::Element { .. }
        | PluginTarget::Group { .. } => Err(PluginError::InvalidTarget(
            "target is not the exposed Razer whole-device lighting surface".to_owned(),
        )),
    }
}

fn lower_effect(
    profile: devices::Profile,
    operation: &PluginUpdateOperation,
) -> Result<MatrixEffect, PluginError> {
    match operation {
        PluginUpdateOperation::SetEffect {
            effect: Effect::Off,
        }
        | PluginUpdateOperation::Clear => Ok(MatrixEffect::Off),
        PluginUpdateOperation::SetEffect {
            effect: Effect::Static { colour },
        } => Ok(MatrixEffect::Static(rgb_from_colour(colour)?)),
        PluginUpdateOperation::SetEffect {
            effect: Effect::Hardware { id, arguments },
        } => {
            if id.as_str() == topology::WHEEL_EFFECT_ID && !profile.supports_wheel {
                return Err(PluginError::Unsupported(format!(
                    "{} does not declare the Razer wheel effect",
                    profile.name
                )));
            }
            if id.as_str() == topology::STARLIGHT_EFFECT_ID && !profile.supports_starlight {
                return Err(PluginError::Unsupported(format!(
                    "{} does not declare the Razer starlight effect",
                    profile.name
                )));
            }
            lower_hardware_effect(id.as_str(), arguments)
        }
        PluginUpdateOperation::SetBrightness { .. }
        | PluginUpdateOperation::SetAppearanceSlots { .. }
        | PluginUpdateOperation::SaveCurrent => Err(PluginError::Unsupported(
            "Razer operation does not select a firmware effect".to_owned(),
        )),
        PluginUpdateOperation::SetEffect { .. } => Err(PluginError::Unsupported(
            "Razer operation is not yet hardware-validated".to_owned(),
        )),
    }
}

fn lower_operation(
    profile: devices::Profile,
    operation: &PluginUpdateOperation,
    storage: Storage,
) -> Result<Command, PluginError> {
    if let PluginUpdateOperation::SetBrightness { value } = operation {
        let brightness = u8::try_from(*value).map_err(|_| {
            PluginError::InvalidArgument(format!("Razer brightness {value} exceeds 255"))
        })?;
        return Command::set_brightness(
            profile.transaction_id,
            storage,
            LedTarget::Backlight,
            brightness,
        )
        .map_err(|error| PluginError::Internal(error.to_string()));
    }
    let effect = lower_effect(profile, operation)?;

    Command::extended_matrix_effect(
        profile.transaction_id,
        storage,
        LedTarget::Backlight,
        effect,
    )
    .map_err(|error| PluginError::Internal(error.to_string()))
}

fn lower_hardware_effect(
    id: &str,
    arguments: &EffectArguments,
) -> Result<MatrixEffect, PluginError> {
    match id {
        topology::SPECTRUM_EFFECT_ID => Ok(MatrixEffect::Spectrum),
        topology::WAVE_EFFECT_ID => Ok(MatrixEffect::Wave(match arguments.direction {
            Some(EffectDirection::Forward) => Direction::Forward,
            Some(EffectDirection::Reverse) => Direction::Reverse,
            _ => return Err(invalid_hardware_arguments(id)),
        })),
        topology::WHEEL_EFFECT_ID => Ok(MatrixEffect::Wheel(match arguments.direction {
            Some(EffectDirection::Clockwise) => Direction::Forward,
            Some(EffectDirection::CounterClockwise) => Direction::Reverse,
            _ => return Err(invalid_hardware_arguments(id)),
        })),
        topology::REACTIVE_EFFECT_ID => Ok(MatrixEffect::Reactive {
            speed: match arguments.speed {
                Some(1) => ReactiveSpeed::Fast,
                Some(2) => ReactiveSpeed::Medium,
                Some(3) => ReactiveSpeed::Slow,
                Some(4) => ReactiveSpeed::Slowest,
                _ => return Err(invalid_hardware_arguments(id)),
            },
            colour: one_hardware_colour(id, &arguments.colours)?,
        }),
        topology::BREATHING_EFFECT_ID => Ok(MatrixEffect::Breathing(one_or_two_colour_mode(
            id,
            &arguments.colours,
        )?)),
        topology::STARLIGHT_EFFECT_ID => Ok(MatrixEffect::Starlight {
            speed: match arguments.speed {
                Some(1) => StarlightSpeed::Fast,
                Some(2) => StarlightSpeed::Medium,
                Some(3) => StarlightSpeed::Slow,
                _ => return Err(invalid_hardware_arguments(id)),
            },
            colours: one_or_two_colour_mode(id, &arguments.colours)?,
        }),
        _ => Err(PluginError::Unsupported(format!(
            "unknown Razer hardware effect {id}"
        ))),
    }
}

fn one_hardware_colour(id: &str, colours: &[CoreRgb]) -> Result<Rgb, PluginError> {
    let [colour] = colours else {
        return Err(invalid_hardware_arguments(id));
    };
    Ok(protocol_rgb(*colour))
}

fn one_or_two_colour_mode(id: &str, colours: &[CoreRgb]) -> Result<ColourMode, PluginError> {
    match colours {
        [colour] => Ok(ColourMode::Single(protocol_rgb(*colour))),
        [first, second] => Ok(ColourMode::Dual(
            protocol_rgb(*first),
            protocol_rgb(*second),
        )),
        _ => Err(invalid_hardware_arguments(id)),
    }
}

const fn protocol_rgb(colour: CoreRgb) -> Rgb {
    Rgb {
        red: colour.r,
        green: colour.g,
        blue: colour.b,
    }
}

fn invalid_hardware_arguments(id: &str) -> PluginError {
    PluginError::InvalidArgument(format!("invalid arguments for Razer hardware effect {id}"))
}

fn rgb_from_colour(colour: &Colour) -> Result<Rgb, PluginError> {
    if colour.encoding() != ColourEncoding::Additive {
        return Err(PluginError::InvalidArgument(
            "Razer static effect requires additive RGB colour".to_owned(),
        ));
    }
    let Colour::Additive(channels) = colour else {
        return Err(PluginError::InvalidArgument(
            "Razer static effect requires additive RGB colour".to_owned(),
        ));
    };
    for channel in channels {
        u8::try_from(channel.value).map_err(|_| {
            PluginError::InvalidArgument("Razer RGB channel exceeds 8 bits".to_owned())
        })?;
    }
    colour.try_as_rgb().map(protocol_rgb).map_err(|error| {
        let diagnostic = match error {
            Rgb8Error::MissingChannel(channel) => {
                format!("missing {} channel", channel.as_str())
            }
            Rgb8Error::ChannelOutOfRange { .. } => "Razer RGB channel exceeds 8 bits".to_owned(),
            Rgb8Error::NonAdditive(_) => {
                "Razer static effect requires additive RGB colour".to_owned()
            }
        };
        PluginError::InvalidArgument(diagnostic)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgb = Rgb {
        red: 255,
        green: 0,
        blue: 0,
    };
    const BLUE: Rgb = Rgb {
        red: 0,
        green: 0,
        blue: 255,
    };

    #[test]
    fn first_element_update_rejects_an_unknown_shadow() {
        assert!(matches!(
            frame_with_element(devices::BLACKWIDOW_V4_PRO, None, 0, 1, RED),
            Err(PluginError::Unsupported(_))
        ));
    }

    #[test]
    fn element_update_preserves_the_previous_complete_shadow() {
        let first = vec![RED; 184];
        let second = frame_with_element(devices::BLACKWIDOW_V4_PRO, Some(&first), 0, 2, BLUE)
            .expect("valid second element coordinate");

        assert_eq!(second[1], RED);
        assert_eq!(second[2], BLUE);
    }

    #[test]
    fn element_update_rejects_invalid_or_corrupt_shadow_coordinates() {
        assert!(frame_with_element(devices::BLACKWIDOW_V4_PRO, None, 8, 0, RED).is_err());
        assert!(frame_with_element(devices::BLACKWIDOW_V4_PRO, Some(&[RED]), 0, 0, BLUE).is_err());
    }

    #[test]
    fn ordinary_updates_lower_to_volatile_storage() {
        let brightness = lower_operation(
            devices::BLACKWIDOW_V4_PRO,
            &PluginUpdateOperation::SetBrightness { value: 42 },
            Storage::Volatile,
        )
        .expect("brightness should lower");
        let off = lower_operation(
            devices::BLACKWIDOW_V4_PRO,
            &PluginUpdateOperation::SetEffect {
                effect: Effect::Off,
            },
            Storage::Volatile,
        )
        .expect("off should lower");

        assert_eq!(brightness.report().arguments(), &[0x00, 0x05, 42]);
        assert_eq!(off.report().arguments(), &[0x00, 0x05, 0, 0, 0, 0]);
    }

    #[test]
    fn disabled_untested_profiles_are_not_discovered() {
        assert!(!profile_enabled(devices::PROFILES[1], false));
        assert!(profile_enabled(devices::PROFILES[1], true));
        assert!(profile_enabled(devices::BLACKWIDOW_V4_PRO, false));
    }

    #[test]
    fn duplicate_physical_identities_are_omitted() {
        let profile = devices::BLACKWIDOW_V4_PRO;
        let mut candidates = vec![
            (profile, c"path-one".to_owned(), "serial:same".to_owned()),
            (profile, c"path-two".to_owned(), "serial:same".to_owned()),
            (
                profile,
                c"path-three".to_owned(),
                "serial:unique".to_owned(),
            ),
        ];

        retain_unique_identities(&mut candidates);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].2, "serial:unique");
    }

    #[test]
    fn physical_ids_and_routes_are_stable_across_enumeration_order() {
        let profile = devices::BLACKWIDOW_V4_PRO;
        let first = (profile, c"path-one".to_owned(), "serial:first".to_owned());
        let second = (profile, c"path-two".to_owned(), "serial:second".to_owned());

        let forward = prepare_discovered(vec![first.clone(), second.clone()]);
        let reverse = prepare_discovered(vec![second, first]);

        for device in &forward {
            let reordered = reverse
                .iter()
                .find(|candidate| candidate.id == device.id)
                .expect("physical identity remains discoverable");
            assert_eq!(reordered.path, device.path);
        }
    }

    #[test]
    fn path_identity_distinguishes_devices_without_serials() {
        let profile = devices::BLACKWIDOW_V4_PRO;
        let devices = prepare_discovered(vec![
            (profile, c"path-one".to_owned(), "path:1111".to_owned()),
            (profile, c"path-two".to_owned(), "path:2222".to_owned()),
        ]);

        assert_eq!(devices.len(), 2);
        assert_ne!(devices[0].id, devices[1].id);
        assert_ne!(devices[0].path, devices[1].path);
    }

    #[test]
    fn plugin_metadata_covers_every_imported_usb_identity() {
        assert_eq!(VENDORS.len(), devices::PROFILES.len());
        for profile in devices::PROFILES {
            assert!(VENDORS.iter().any(|identity| {
                identity.vendor == u32::from(profile.vendor_id)
                    && identity.product == u32::from(profile.product_id)
            }));
        }
    }
}

luminate_export_plugin! {
    plugin: Razer,
    name: NAME,
    version: VERSION,
    priority: 200,
    recommended_reconciliation: Some(ReconciliationPolicy::Restore),
    buses: BUSES,
    vendors: VENDORS,
    probe_hints: HINTS,
    settings: SETTINGS,
    start: none,
    rescan: none,
    batch: native,
    read_state: none,
    frame_upload: native,
    shm_frame: none,
}
