// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Packet encoding for Alienware AW-ELC lighting controllers.

#![allow(
    clippy::indexing_slicing,
    reason = "AW-ELC reports are fixed-layout HID packets with checked offsets."
)]

use std::cell::LazyCell;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Mutex, OnceLock};
use std::thread::sleep;
use std::time::{Duration, Instant};

#[cfg(test)]
use luminate_core::appearance_slot::AppearanceSlotId;
use luminate_core::appearance_slot::AppearanceSlotValue;
#[cfg(test)]
use luminate_core::colour::Colour;
use luminate_core::effect::Effect;
use luminate_core::rgb::Rgb;
use luminate_plugin_api::{PluginTarget, PluginUpdateOperation, ShadowState};

use crate::aw_elc_profile::{
    AwElcIdentity, AwElcProfile, AwElcZone, M16_R2, ProfileResolutionError, ZoneOperationClass,
    resolve,
};
use crate::topology::{
    AW_ELC_BREATHE_EFFECT_ID, AW_ELC_DEVICE_ID, AW_ELC_PULSE_EFFECT_ID, AW_ELC_RAINBOW_EFFECT_ID,
    AW_ELC_SLOT_AC, AW_ELC_SLOT_BATTERY, AW_ELC_SPECTRUM_EFFECT_ID, AW_ELC_SURFACE_POWER_BUTTON,
};

use super::{HidChannel, HidTransport, HidUsage, effect_to_rgb_static, write_feature_report};

/// AW-ELC exposes exactly one top-level collection (confirmed by real-device
/// enumeration), so unlike the keyboard there is no ambiguity to resolve.
/// This is filtered explicitly anyway so both devices go through the same
/// collection-aware open path rather than one of them quietly depending on
/// there only ever being one match.
const VENDOR_USAGE: HidUsage = HidUsage {
    page: 0xff00,
    usage: 0x0001,
};

const REPORT_LEN: usize = 34;
const WIRE_OFFSET: usize = 1;
const WIRE_ID: u8 = 0x03;
const OP_REPORT_CONFIG: u8 = 0x20;
const REPORT_CONFIG: u8 = 0x02;
const OP_USER_ANIMATION: u8 = 0x21;
const OP_POWER_ANIMATION: u8 = 0x22;
const OP_SELECT_ZONES: u8 = 0x23;
const OP_ADD_ACTION: u8 = 0x24;
const LIVE_ANIMATION_ID: u16 = 0xffff;
const USER_ANIM_START_NEW: u16 = 0x0001;
const USER_ANIM_FINISH_SAVE: u16 = 0x0002;
const USER_ANIM_FINISH_PLAY: u16 = 0x0003;
const USER_ANIM_REMOVE: u16 = 0x0004;
const USER_ANIM_SET_DEFAULT: u16 = 0x0006;
const SAVED_ANIMATION_SLOT: u16 = 0x0061;

/// Duration/tempo used for the keyframe-typed power-button slots, taken from
/// the protocol spec's slot `0x5b` example (§13.6): `03 e8` (1000 ms) /
/// `00 64` (fast tempo).
const POWER_SLOT_KEYFRAME_DURATION_MS: u16 = 0x03e8;
const POWER_SLOT_KEYFRAME_TEMPO: u16 = 0x0064;

/// Fast pulse tempo (§14: "Pulse tempo mapping (`00 64` fast, `00 fa`
/// slow)"), used for the one pulse-typed power-button slot.
const POWER_SLOT_PULSE_TEMPO: u16 = 0x0064;

/// Firmware-managed power conditions. The slot identity determines both its
/// colour source and its complete action sequence; representing only the
/// primitive record type loses the second keyframe required by sleep and
/// charging states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PowerButtonSlot {
    AcSleep,
    AcActive,
    Charging,
    BatterySleep,
    BatteryActive,
    BatteryCritical,
}

impl PowerButtonSlot {
    const fn id(self) -> u16 {
        match self {
            Self::AcSleep => 0x005b,
            Self::AcActive => 0x005c,
            Self::Charging => 0x005d,
            Self::BatterySleep => 0x005e,
            Self::BatteryActive => 0x005f,
            Self::BatteryCritical => 0x0060,
        }
    }
}

const POWER_SLOTS_AC: &[PowerButtonSlot] = &[PowerButtonSlot::AcSleep, PowerButtonSlot::AcActive];
const POWER_SLOT_CHARGING: &[PowerButtonSlot] = &[PowerButtonSlot::Charging];
const POWER_SLOTS_BATTERY: &[PowerButtonSlot] = &[
    PowerButtonSlot::BatterySleep,
    PowerButtonSlot::BatteryActive,
    PowerButtonSlot::BatteryCritical,
];
const POWER_WRITE_MIN_INTERVAL: Duration = Duration::from_secs(5);

/// Settling delay around opening/closing the saved-animation slot
/// transaction (`REMOVE`/`FINISH_N_SAVE`). Confirmed necessary against real
/// hardware by an independent reference implementation: real AWCC traffic
/// shows a comparable gap here, and running a save transaction without it
/// was the suspected cause of a corrupted adjacent write despite every
/// individual command acknowledging fine over USB.
const SAVE_SETTLE: Duration = Duration::from_millis(350);

/// Upper bound on user-supplied morph keyframes.
///
/// A live morph is materialized into one `03:24` action record per colour,
/// packed three-per-HID-report (`live_animation_reports`), and every report is
/// a synchronous feature-report write issued while the daemon holds its single
/// state lock. The `colours` vector arrives straight off the wire, bounded only
/// by the 8 MiB frame cap (roughly 2.7M colours, i.e. ~900k HID writes), so an
/// uncapped morph is a request-to-hardware amplification vector that can pin the
/// state lock effectively forever. The built-in spectrum/rainbow palettes use
/// seven keyframes and no larger firmware animation buffer is documented (AW-ELC
/// protocol spec §5), so this cap sits generously above every legitimate use
/// while bounding a single `SetEffect` to a handful of reports.
const MAX_MORPH_KEYFRAMES: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConfigurationQueryError {
    Transport(String),
    ShortReport {
        length: usize,
    },
    WrongMarkers {
        wire_id: u8,
        opcode: u8,
        subcommand: u8,
    },
    Profile(ProfileResolutionError),
}

impl fmt::Display for ConfigurationQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => formatter.write_str(error),
            Self::ShortReport { length } => {
                write!(
                    formatter,
                    "AW-ELC configuration report was too short: {length} bytes"
                )
            }
            Self::WrongMarkers {
                wire_id,
                opcode,
                subcommand,
            } => write!(
                formatter,
                "unexpected AW-ELC configuration response markers: {wire_id:02x}:{opcode:02x}:{subcommand:02x}"
            ),
            Self::Profile(error) => error.fmt(formatter),
        }
    }
}

pub(crate) fn read_identity(
    transport: &dyn HidTransport,
    vendor_id: u16,
    product_id: u16,
) -> Result<AwElcIdentity, ConfigurationQueryError> {
    let device = transport
        .open(vendor_id, product_id, VENDOR_USAGE)
        .map_err(ConfigurationQueryError::Transport)?;
    let mut query = empty_report(OP_REPORT_CONFIG);
    query[WIRE_OFFSET + 2] = REPORT_CONFIG;
    write_feature_report(device.as_ref(), &query).map_err(ConfigurationQueryError::Transport)?;

    let mut report = [0_u8; REPORT_LEN];
    let length = device.get_feature_report(&mut report).map_err(|error| {
        ConfigurationQueryError::Transport(format!("failed to read AW-ELC configuration: {error}"))
    })?;

    parse_configuration_report(vendor_id, product_id, &report[..length])
}

fn parse_configuration_report(
    vendor_id: u16,
    product_id: u16,
    report: &[u8],
) -> Result<AwElcIdentity, ConfigurationQueryError> {
    const MINIMUM_LENGTH: usize = 7;
    if report.len() < MINIMUM_LENGTH {
        return Err(ConfigurationQueryError::ShortReport {
            length: report.len(),
        });
    }

    let wire_id = report[WIRE_OFFSET];
    let opcode = report[WIRE_OFFSET + 1];
    let subcommand = report[WIRE_OFFSET + 2];
    if (wire_id, opcode, subcommand) != (WIRE_ID, OP_REPORT_CONFIG, REPORT_CONFIG) {
        return Err(ConfigurationQueryError::WrongMarkers {
            wire_id,
            opcode,
            subcommand,
        });
    }

    // The protocol notes number response offsets in the complete hidapi
    // buffer, including its leading zero report-ID placeholder.
    let Some(&[platform_high, platform_low, reported_zone_count]) = report.get(4..7) else {
        return Err(ConfigurationQueryError::ShortReport {
            length: report.len(),
        });
    };
    let platform_id = u16::from_be_bytes([platform_high, platform_low]);
    resolve(vendor_id, product_id, platform_id, reported_zone_count)
        .map_err(ConfigurationQueryError::Profile)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PowerButtonZone {
    Ac,
    Battery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zone {
    Live(u8),
    PowerButton(PowerButtonZone),
}

impl Zone {
    #[cfg(test)]
    #[allow(
        non_upper_case_globals,
        reason = "compatibility aliases keep packet regression tests readable"
    )]
    const TrackpadRing: Self = Self::Live(0x00);

    #[cfg(test)]
    #[allow(
        non_upper_case_globals,
        reason = "compatibility aliases keep packet regression tests readable"
    )]
    const RearLogo: Self = Self::Live(0x02);

    const fn id(self) -> u8 {
        match self {
            Self::Live(id) => id,
            Self::PowerButton(_) => 0x04,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActionRecord {
    mode: u8,
    duration_ms: u16,
    tempo: u16,
    rgb: Rgb,
}

#[derive(Debug, Default)]
struct PowerButtonShadow {
    rgb_ac: Option<Rgb>,
    rgb_battery: Option<Rgb>,
    last_write_ac: Option<Instant>,
    last_write_battery: Option<Instant>,
    last_write_charging: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct LiveZoneKey {
    vendor: u16,
    product: u16,
    platform: u16,
    zone: u8,
}

/// In-process shadow of the action records last applied live to each zone,
/// keyed by exact controller/profile identity and zone ID. There is no hardware readback for the live
/// animation state, so `SaveCurrent` replays whatever this process itself
/// last wrote rather than querying the controller. Persists for the lifetime
/// of the plugin, same as the keyboard's per-key shadow.
static LIVE_ZONE_SHADOW: ShadowState<LiveZoneKey, Vec<ActionRecord>> = ShadowState::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PowerButtonColours {
    ac: Rgb,
    battery: Rgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PowerButtonWrite {
    colours: PowerButtonColours,
    ac: bool,
    battery: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PowerButtonWritePlan {
    SkipAlreadyCurrent,
    Write(PowerButtonWrite),
}

pub fn apply(
    transport: &dyn HidTransport,
    identity: AwElcIdentity,
    target: &PluginTarget,
    operation: &PluginUpdateOperation,
) -> Result<(), String> {
    if matches!(operation, PluginUpdateOperation::SetBrightness { .. }) {
        return Err("AW-ELC does not expose an independent brightness control".to_owned());
    }

    if let PluginUpdateOperation::SetAppearanceSlots { values } = operation {
        if identity.profile != &M16_R2 {
            return Err(format!(
                "AW-ELC profile {} does not expose appearance slots",
                identity.profile.model
            ));
        }
        return apply_power_button_slots(
            transport,
            identity.vendor_id,
            identity.product_id,
            target,
            values,
        );
    }

    let zones = zones_for_target(identity.profile, target)?;

    if matches!(operation, PluginUpdateOperation::SaveCurrent) {
        if identity.profile != &M16_R2 {
            return Err(format!(
                "AW-ELC profile {} exposes volatile state only and cannot save the current appearance",
                identity.profile.model
            ));
        }
        return apply_save_current(transport, identity, &zones);
    }

    ensure_power_button_operation_supported(&zones, operation)?;

    let device = transport.open(identity.vendor_id, identity.product_id, VENDOR_USAGE)?;

    let live_zones = zones
        .iter()
        .copied()
        .filter(|zone| !matches!(zone, Zone::PowerButton(_)))
        .collect::<Vec<_>>();
    if !live_zones.is_empty() {
        let records = records_for_operation(operation)?;
        send_live_animation(device.as_ref(), &live_zones, &records)?;
        remember_live_zone_records(identity, &live_zones, &records);
    }

    let rgb = LazyCell::new(|| effect_to_rgb_static(operation));
    for zone in zones.iter().copied() {
        if let Zone::PowerButton(button) = zone {
            apply_power_button_static(button, device.as_ref(), (*rgb).clone()?)?;
        }
    }

    Ok(())
}

pub(crate) fn apply_live_batch(
    transport: &dyn HidTransport,
    identity: AwElcIdentity,
    entries: &[(&PluginTarget, &PluginUpdateOperation)],
) -> Option<Vec<Result<(), String>>> {
    if identity.profile == &M16_R2 {
        return None;
    }

    let prepare = |target: &PluginTarget, operation: &PluginUpdateOperation| {
        if matches!(operation, PluginUpdateOperation::SetBrightness { .. }) {
            return Err("AW-ELC does not expose an independent brightness control".to_owned());
        }
        if matches!(operation, PluginUpdateOperation::SetAppearanceSlots { .. }) {
            return Err(format!(
                "AW-ELC profile {} does not expose appearance slots",
                identity.profile.model
            ));
        }
        if matches!(operation, PluginUpdateOperation::SaveCurrent) {
            return Err(format!(
                "AW-ELC profile {} exposes volatile state only and cannot save the current appearance",
                identity.profile.model
            ));
        }

        let zones = zones_for_target(identity.profile, target)?;
        if zones
            .iter()
            .any(|zone| matches!(zone, Zone::PowerButton(_)))
        {
            return Err("imported AW-ELC profiles cannot target power-button slots".to_owned());
        }
        let records = records_for_operation(operation)?;
        Ok((zones, records))
    };

    let mut outcomes = Vec::with_capacity(entries.len());
    let mut index = 0;
    while index < entries.len() {
        let Some(&(target, operation)) = entries.get(index) else {
            break;
        };
        let (mut zones, records) = match prepare(target, operation) {
            Ok(prepared) => prepared,
            Err(error) => {
                outcomes.push(Err(error));
                index += 1;
                continue;
            }
        };

        let mut end = index + 1;
        while let Some(&(next_target, next_operation)) = entries.get(end) {
            let Ok((next_zones, next_records)) = prepare(next_target, next_operation) else {
                break;
            };
            if next_records != records {
                break;
            }
            for zone in next_zones {
                if !zones.contains(&zone) {
                    zones.push(zone);
                }
            }
            end += 1;
        }

        let outcome = transport
            .open(identity.vendor_id, identity.product_id, VENDOR_USAGE)
            .and_then(|device| send_live_animation(device.as_ref(), &zones, &records));
        if outcome.is_ok() {
            remember_live_zone_records(identity, &zones, &records);
        }
        outcomes.extend((index..end).map(|_| outcome.clone()));
        index = end;
    }

    Some(outcomes)
}

/// Persists the currently live ring/rear-logo animation into the non-volatile
/// saved-animation slot (protocol spec §6.2/§11), confirmed by an independent
/// reference implementation to survive a full reboot, auto-restore at boot,
/// and replay via `USER_ANIM PLAY`. The power button never needs this: every
/// ordinary write already lands directly in a persistent firmware slot.
fn apply_save_current(
    transport: &dyn HidTransport,
    identity: AwElcIdentity,
    zones: &[Zone],
) -> Result<(), String> {
    if zones
        .iter()
        .any(|zone| matches!(zone, Zone::PowerButton(_)))
    {
        if zones.len() != 1 {
            return Err(
                "AW-ELC target mixes transient live zones with the persistent power button states; \
                 target group:live-zones for temporary control or surface:power-button for firmware slots".to_owned(),
            );
        }
        return Err(
            "AW-ELC power-button slots are already persistent on every write; save-current is not needed"
                .to_owned(),
        );
    }

    let per_zone_records = {
        let shadow = LIVE_ZONE_SHADOW
            .lock()
            .expect("AW-ELC live-zone shadow lock poisoned");
        resolve_zone_records(&shadow, identity, zones)?
    };

    let device = transport.open(identity.vendor_id, identity.product_id, VENDOR_USAGE)?;
    save_live_zones(device.as_ref(), &per_zone_records)
}

/// Looks up each zone's last-applied live records in the shadow. Pure and
/// side-effect-free so it can be tested against a local map instead of the
/// process-global shadow.
fn resolve_zone_records(
    shadow: &BTreeMap<LiveZoneKey, Vec<ActionRecord>>,
    identity: AwElcIdentity,
    zones: &[Zone],
) -> Result<Vec<(Zone, Vec<ActionRecord>)>, String> {
    zones
        .iter()
        .map(|&zone| {
            shadow
                .get(&live_zone_key(identity, zone.id()))
                .cloned()
                .map(|records| (zone, records))
                .ok_or_else(|| {
                    format!(
                        "no live AW-ELC animation has been applied to zone 0x{:02x} yet to save",
                        zone.id()
                    )
                })
        })
        .collect()
}

fn zones_for_target(
    profile: &'static AwElcProfile,
    target: &PluginTarget,
) -> Result<Vec<Zone>, String> {
    let all_zones = || {
        profile
            .zones
            .iter()
            .flat_map(profile_zone)
            .collect::<Vec<_>>()
    };
    match target {
        PluginTarget::Device { .. } => Ok(all_zones()),
        PluginTarget::Group { group, .. } if group == "all" => Ok(all_zones()),
        PluginTarget::Group { group, .. } if group == "live-zones" && profile == &M16_R2 => {
            Ok(profile
                .zones
                .iter()
                .filter(|zone| zone.operation_class == ZoneOperationClass::Live)
                .map(|zone| Zone::Live(zone.firmware_id))
                .collect())
        }
        PluginTarget::Surface { surface, .. } => {
            surface_to_zone(profile, surface).map(|zone| vec![zone])
        }
        PluginTarget::Element {
            surface, element, ..
        } if profile == &M16_R2 && element == "led" => {
            surface_to_zone(profile, surface).map(|zone| vec![zone])
        }
        PluginTarget::Element {
            surface, element, ..
        } => Err(format!(
            "unknown AW-ELC element target: {surface}/{element}"
        )),
        PluginTarget::Group { group, .. } => Err(format!("unknown AW-ELC group target: {group}")),
    }
}

fn profile_zone(zone: &AwElcZone) -> Vec<Zone> {
    match zone.operation_class {
        ZoneOperationClass::Live => vec![Zone::Live(zone.firmware_id)],
        ZoneOperationClass::PowerButton => vec![
            Zone::PowerButton(PowerButtonZone::Ac),
            Zone::PowerButton(PowerButtonZone::Battery),
        ],
    }
}

fn surface_to_zone(profile: &'static AwElcProfile, surface: &str) -> Result<Zone, String> {
    let zone = profile
        .zones
        .iter()
        .find(|zone| zone.surface_id == surface)
        .ok_or_else(|| format!("unknown AW-ELC surface target: {surface}"))?;
    match zone.operation_class {
        ZoneOperationClass::Live => Ok(Zone::Live(zone.firmware_id)),
        ZoneOperationClass::PowerButton => {
            Err("AW-ELC power-button requires a set-appearance-slots operation".to_owned())
        }
    }
}

fn operation_supports_power_button(operation: &PluginUpdateOperation) -> bool {
    matches!(
        operation,
        PluginUpdateOperation::SetEffect {
            effect: Effect::Static { .. } | Effect::Off,
        } | PluginUpdateOperation::Clear
    )
}

fn ensure_power_button_operation_supported(
    zones: &[Zone],
    operation: &PluginUpdateOperation,
) -> Result<(), String> {
    let includes_power_button = zones
        .iter()
        .any(|zone| matches!(zone, Zone::PowerButton(_)));
    if !includes_power_button {
        return Ok(());
    }

    if zones.len() != 1 {
        return Err(
            "AW-ELC target mixes transient live zones with the persistent power button states; \
             target group:live-zones for temporary control or surface:power-button for firmware slots".to_owned(),
        );
    }

    if !operation_supports_power_button(operation) {
        return Err(
            "AW-ELC power button supports only static RGB/off/clear persistent slot writes; \
             target group:live-zones or a live-zone surface for animated effects"
                .to_owned(),
        );
    }

    Ok(())
}

fn records_for_operation(operation: &PluginUpdateOperation) -> Result<Vec<ActionRecord>, String> {
    match operation {
        PluginUpdateOperation::SetEffect {
            effect: Effect::Static { .. } | Effect::Off,
        }
        | PluginUpdateOperation::Clear => Ok(vec![static_record(effect_to_rgb_static(operation)?)]),
        PluginUpdateOperation::SetEffect {
            effect: Effect::Hardware { id, arguments },
        } if matches!(id.as_str(), AW_ELC_BREATHE_EFFECT_ID) => {
            let colour = hardware_colour(arguments.colours.first(), id.as_str())?;
            Ok(vec![
                morph_record(colour, 1500, 0x0064),
                morph_record(Rgb::new(0, 0, 0), 1500, 0x0064),
            ])
        }
        PluginUpdateOperation::SetEffect {
            effect: Effect::Hardware { id, arguments },
        } if matches!(id.as_str(), AW_ELC_PULSE_EFFECT_ID) => Ok(vec![pulse_record(
            hardware_colour(arguments.colours.first(), id.as_str())?,
            0x0064,
        )]),
        PluginUpdateOperation::SetEffect {
            effect: Effect::Morph { colours, period_ms },
        } => {
            if colours.is_empty() {
                return Err("morph effect requires at least one colour".to_owned());
            }
            if colours.len() > MAX_MORPH_KEYFRAMES {
                return Err(format!(
                    "morph effect has {} colours, but AW-ELC accepts at most {MAX_MORPH_KEYFRAMES} keyframes",
                    colours.len()
                ));
            }
            let duration = u16::try_from(*period_ms)
                .map_err(|_| format!("period {period_ms}ms exceeds AW-ELC duration field"))?;
            Ok(colours
                .iter()
                .copied()
                .map(|colour| morph_record(colour, duration, 0x0064))
                .collect())
        }
        PluginUpdateOperation::SetEffect {
            effect: Effect::Hardware { id, .. },
        } if matches!(id.as_str(), AW_ELC_SPECTRUM_EFFECT_ID) => Ok(spectrum_records()),
        PluginUpdateOperation::SetEffect {
            effect: Effect::Hardware { id, .. },
        } if matches!(id.as_str(), AW_ELC_RAINBOW_EFFECT_ID) => Ok(rainbow_records()),
        PluginUpdateOperation::SetEffect {
            effect:
                Effect::Breathe { .. }
                | Effect::Pulse { .. }
                | Effect::Strobe { .. }
                | Effect::Spectrum { .. }
                | Effect::Rainbow { .. }
                | Effect::Scanner { .. },
        } => Err(
            "AW-ELC fixed-cadence animations are exposed as Alienware hardware effects".to_owned(),
        ),
        PluginUpdateOperation::SetEffect {
            effect: Effect::Hardware { .. },
        } => Err("AW-ELC advertises no vendor-specific hardware effects".to_owned()),
        PluginUpdateOperation::SetBrightness { .. } => {
            Err("AW-ELC does not expose an independent brightness control".to_owned())
        }
        PluginUpdateOperation::SetAppearanceSlots { .. } => unreachable!("handled by apply"),
        // Handled by `apply`; this arm keeps the operation match exhaustive.
        PluginUpdateOperation::SaveCurrent => {
            Err("AW-ELC save-current has no live-animation records to send".to_owned())
        }
    }
}

fn hardware_colour(colour: Option<&Rgb>, effect_id: &str) -> Result<Rgb, String> {
    colour
        .copied()
        .ok_or_else(|| format!("{effect_id} requires one colour"))
}

fn static_record(rgb: Rgb) -> ActionRecord {
    ActionRecord {
        mode: 0x00,
        duration_ms: 2000,
        tempo: 0x00fa,
        rgb,
    }
}

fn morph_record(rgb: Rgb, duration_ms: u16, tempo: u16) -> ActionRecord {
    ActionRecord {
        mode: 0x02,
        duration_ms,
        tempo,
        rgb,
    }
}

fn pulse_record(rgb: Rgb, tempo: u16) -> ActionRecord {
    ActionRecord {
        mode: 0x01,
        duration_ms: 2000,
        tempo,
        rgb,
    }
}

fn spectrum_records() -> Vec<ActionRecord> {
    palette_records(642)
}

fn rainbow_records() -> Vec<ActionRecord> {
    palette_records(428)
}

fn palette_records(duration_ms: u16) -> Vec<ActionRecord> {
    [
        Rgb::new(0xff, 0x00, 0x00),
        Rgb::new(0xff, 0xa5, 0x00),
        Rgb::new(0xff, 0xff, 0x00),
        Rgb::new(0x00, 0x80, 0x00),
        Rgb::new(0x00, 0xbf, 0xff),
        Rgb::new(0x00, 0x00, 0xff),
        Rgb::new(0x80, 0x00, 0x80),
    ]
    .into_iter()
    .map(|rgb| morph_record(rgb, duration_ms, 0x000f))
    .collect()
}

fn send_live_animation(
    device: &dyn HidChannel,
    zones: &[Zone],
    records: &[ActionRecord],
) -> Result<(), String> {
    for report in live_animation_reports(zones, records)? {
        write_feature_report(device, &report)?;
    }
    Ok(())
}

fn live_animation_reports(
    zones: &[Zone],
    records: &[ActionRecord],
) -> Result<Vec<[u8; REPORT_LEN]>, String> {
    let mut reports = vec![
        animation_envelope_report(OP_USER_ANIMATION, USER_ANIM_START_NEW, LIVE_ANIMATION_ID, 0),
        select_zones_report(zones)?,
    ];

    for chunk in records.chunks(3) {
        reports.push(action_records_report(chunk)?);
    }

    reports.push(animation_envelope_report(
        OP_USER_ANIMATION,
        USER_ANIM_FINISH_PLAY,
        LIVE_ANIMATION_ID,
        0,
    ));
    Ok(reports)
}

const fn live_zone_key(identity: AwElcIdentity, zone_id: u8) -> LiveZoneKey {
    LiveZoneKey {
        vendor: identity.vendor_id,
        product: identity.product_id,
        platform: identity.platform_id,
        zone: zone_id,
    }
}

fn remember_live_zone_records(identity: AwElcIdentity, zones: &[Zone], records: &[ActionRecord]) {
    let mut shadow = LIVE_ZONE_SHADOW
        .lock()
        .expect("AW-ELC live-zone shadow lock poisoned");
    for zone in zones {
        shadow.insert(live_zone_key(identity, zone.id()), records.to_vec());
    }
}

pub(crate) fn retain_live_shadow_for(identity: Option<AwElcIdentity>) {
    let mut shadow = LIVE_ZONE_SHADOW
        .lock()
        .expect("AW-ELC live-zone shadow lock poisoned");
    retain_live_shadow_entries(&mut shadow, identity);
}

fn retain_live_shadow_entries(
    shadow: &mut BTreeMap<LiveZoneKey, Vec<ActionRecord>>,
    identity: Option<AwElcIdentity>,
) {
    match identity {
        Some(identity) => shadow.retain(|key, _| {
            key.vendor == identity.vendor_id
                && key.product == identity.product_id
                && key.platform == identity.platform_id
        }),
        None => shadow.clear(),
    }
}

/// Replays the confirmed save-transaction choreography against the saved
/// slot: remove any existing saved animation, then select each zone in turn
/// and send its last-applied action records inside one `START_NEW`/
/// `FINISH_N_SAVE` bracket, then mark the slot as the default. Matches the
/// sequence an independent reference implementation confirmed on hardware
/// (`REMOVE` → `START_NEW` → per-zone `SELECT_ZONES`/`ADD_ACTION` →
/// `FINISH_N_SAVE` → `SET_DEFAULT`), including the settling delay around the
/// slot open/close that real AWCC traffic also uses.
fn save_live_zones(
    device: &dyn HidChannel,
    per_zone_records: &[(Zone, Vec<ActionRecord>)],
) -> Result<(), String> {
    write_feature_report(
        device,
        &animation_envelope_report(OP_USER_ANIMATION, USER_ANIM_REMOVE, SAVED_ANIMATION_SLOT, 0),
    )?;
    sleep(SAVE_SETTLE);

    write_feature_report(
        device,
        &animation_envelope_report(
            OP_USER_ANIMATION,
            USER_ANIM_START_NEW,
            SAVED_ANIMATION_SLOT,
            0,
        ),
    )?;
    for (zone, records) in per_zone_records {
        write_feature_report(device, &select_zones_report(&[*zone])?)?;
        for chunk in records.chunks(3) {
            write_feature_report(device, &action_records_report(chunk)?)?;
        }
    }
    write_feature_report(
        device,
        &animation_envelope_report(
            OP_USER_ANIMATION,
            USER_ANIM_FINISH_SAVE,
            SAVED_ANIMATION_SLOT,
            0,
        ),
    )?;
    sleep(SAVE_SETTLE);

    write_feature_report(
        device,
        &animation_envelope_report(
            OP_USER_ANIMATION,
            USER_ANIM_SET_DEFAULT,
            SAVED_ANIMATION_SLOT,
            0,
        ),
    )
}

fn send_power_button_slots(
    slots: &[PowerButtonSlot],
    device: &dyn HidChannel,
    colours: PowerButtonColours,
) -> Result<(), String> {
    for &power_slot in slots {
        let slot = power_slot.id();
        send_animation_envelope(device, OP_POWER_ANIMATION, 0x0004, slot, 0)?;
        send_animation_envelope(device, OP_POWER_ANIMATION, 0x0001, slot, 0)?;
        send_select_zones(device, &[Zone::PowerButton(PowerButtonZone::Ac)])?;
        send_action_records(device, &power_slot_records(power_slot, colours))?;
        send_animation_envelope(device, OP_POWER_ANIMATION, 0x0002, slot, 0)?;
    }

    Ok(())
}

/// Builds the complete action sequence for a firmware power condition. Sleep
/// fades to black, while charging morphs between the configured AC and battery
/// colours rather than treating `0x5d` as another single-colour AC slot.
fn power_slot_records(slot: PowerButtonSlot, colours: PowerButtonColours) -> Vec<ActionRecord> {
    let keyframe = |rgb| {
        morph_record(
            rgb,
            POWER_SLOT_KEYFRAME_DURATION_MS,
            POWER_SLOT_KEYFRAME_TEMPO,
        )
    };
    match slot {
        PowerButtonSlot::AcSleep => vec![keyframe(colours.ac), keyframe(Rgb::new(0, 0, 0))],
        PowerButtonSlot::AcActive => vec![static_record(colours.ac)],
        PowerButtonSlot::Charging => {
            vec![keyframe(colours.ac), keyframe(colours.battery)]
        }
        PowerButtonSlot::BatterySleep => {
            vec![keyframe(colours.battery), keyframe(Rgb::new(0, 0, 0))]
        }
        PowerButtonSlot::BatteryActive => vec![static_record(colours.battery)],
        PowerButtonSlot::BatteryCritical => {
            vec![pulse_record(colours.battery, POWER_SLOT_PULSE_TEMPO)]
        }
    }
}

fn apply_power_button_static(
    button_zone: PowerButtonZone,
    device: &dyn HidChannel,
    rgb: Rgb,
) -> Result<(), String> {
    match plan_power_button_write(button_zone, rgb)? {
        PowerButtonWritePlan::SkipAlreadyCurrent => Ok(()),
        PowerButtonWritePlan::Write(write) => {
            send_power_button_write(device, write)?;
            remember_power_button_write(write);
            Ok(())
        }
    }
}

fn apply_power_button_slots(
    transport: &dyn HidTransport,
    vendor_id: u16,
    product_id: u16,
    target: &PluginTarget,
    values: &[AppearanceSlotValue],
) -> Result<(), String> {
    if !matches!(target, PluginTarget::Surface { device, surface }
        if device == AW_ELC_DEVICE_ID && surface == AW_ELC_SURFACE_POWER_BUTTON)
    {
        return Err("AW-ELC appearance slots require surface:power-button".to_owned());
    }
    let (ac, battery) = power_button_colours_from_slots(values)?;
    let plan = plan_power_button_updates(ac, battery)?;
    let PowerButtonWritePlan::Write(write) = plan else {
        return Ok(());
    };

    let device = transport.open(vendor_id, product_id, VENDOR_USAGE)?;
    send_power_button_write(device.as_ref(), write)?;
    remember_power_button_write(write);
    Ok(())
}

fn power_button_colours_from_slots(
    values: &[AppearanceSlotValue],
) -> Result<(Option<Rgb>, Option<Rgb>), String> {
    if values.is_empty() {
        return Err("AW-ELC appearance-slot update must not be empty".to_owned());
    }

    let mut ac = None;
    let mut battery = None;
    for value in values {
        let operation = PluginUpdateOperation::SetEffect {
            effect: value.effect.clone(),
        };
        if !operation_supports_power_button(&operation) {
            return Err(
                "AW-ELC power button supports only static RGB/off persistent slot writes"
                    .to_owned(),
            );
        }
        let rgb = effect_to_rgb_static(&operation)?;
        let destination = match value.slot.as_str() {
            AW_ELC_SLOT_AC => &mut ac,
            AW_ELC_SLOT_BATTERY => &mut battery,
            other => return Err(format!("unknown AW-ELC power-button slot: {other}")),
        };
        if destination.replace(rgb).is_some() {
            return Err(format!(
                "duplicate AW-ELC power-button slot: {}",
                value.slot.as_str()
            ));
        }
    }

    Ok((ac, battery))
}

fn send_power_button_write(device: &dyn HidChannel, write: PowerButtonWrite) -> Result<(), String> {
    if write.ac {
        send_power_button_slots(POWER_SLOTS_AC, device, write.colours)?;
    }
    if write.ac || write.battery {
        send_power_button_slots(POWER_SLOT_CHARGING, device, write.colours)?;
    }
    if write.battery {
        send_power_button_slots(POWER_SLOTS_BATTERY, device, write.colours)?;
    }

    Ok(())
}

fn plan_power_button_write(
    button_zone: PowerButtonZone,
    rgb: Rgb,
) -> Result<PowerButtonWritePlan, String> {
    let shadow = power_button_shadow()
        .lock()
        .expect("AW-ELC power-button shadow lock poisoned");
    plan_power_button_write_with_shadow(button_zone, &shadow, rgb, Instant::now())
}

fn plan_power_button_updates(
    ac: Option<Rgb>,
    battery: Option<Rgb>,
) -> Result<PowerButtonWritePlan, String> {
    let shadow = power_button_shadow()
        .lock()
        .expect("AW-ELC power-button shadow lock poisoned");
    plan_power_button_updates_with_shadow(&shadow, ac, battery, Instant::now())
}

fn plan_power_button_write_with_shadow(
    button_zone: PowerButtonZone,
    shadow: &PowerButtonShadow,
    rgb: Rgb,
    now: Instant,
) -> Result<PowerButtonWritePlan, String> {
    match button_zone {
        PowerButtonZone::Ac => plan_power_button_updates_with_shadow(shadow, Some(rgb), None, now),
        PowerButtonZone::Battery => {
            plan_power_button_updates_with_shadow(shadow, None, Some(rgb), now)
        }
    }
}

fn plan_power_button_updates_with_shadow(
    shadow: &PowerButtonShadow,
    ac: Option<Rgb>,
    battery: Option<Rgb>,
    now: Instant,
) -> Result<PowerButtonWritePlan, String> {
    let write_ac = ac.is_some_and(|rgb| shadow.rgb_ac != Some(rgb));
    let write_battery = battery.is_some_and(|rgb| shadow.rgb_battery != Some(rgb));
    if !write_ac && !write_battery {
        return Ok(PowerButtonWritePlan::SkipAlreadyCurrent);
    }

    check_power_button_rate_limit(write_ac, shadow.last_write_ac, now)?;
    check_power_button_rate_limit(write_battery, shadow.last_write_battery, now)?;
    check_power_button_rate_limit(write_ac || write_battery, shadow.last_write_charging, now)?;

    let ac = ac.or(shadow.rgb_ac).ok_or_else(|| {
        "cannot update AW-ELC charging slot 0x5d because the AC power-button colour is unknown; \
         submit AC and battery power-button updates together"
            .to_owned()
    })?;
    let battery = battery.or(shadow.rgb_battery).ok_or_else(|| {
        "cannot update AW-ELC charging slot 0x5d because the battery power-button colour is unknown; \
         submit AC and battery power-button updates together"
            .to_owned()
    })?;

    Ok(PowerButtonWritePlan::Write(PowerButtonWrite {
        colours: PowerButtonColours { ac, battery },
        ac: write_ac,
        battery: write_battery,
    }))
}

fn check_power_button_rate_limit(
    write: bool,
    last_write: Option<Instant>,
    now: Instant,
) -> Result<(), String> {
    if write
        && let Some(last_write) = last_write
        && now.duration_since(last_write) < POWER_WRITE_MIN_INTERVAL
    {
        return Err(format!(
            "AW-ELC power-button firmware slots were updated less than {}s ago; \
             refusing another non-volatile rewrite",
            POWER_WRITE_MIN_INTERVAL.as_secs()
        ));
    }

    Ok(())
}

fn remember_power_button_write(write: PowerButtonWrite) {
    let mut shadow = power_button_shadow()
        .lock()
        .expect("AW-ELC power-button shadow lock poisoned");
    remember_power_button_write_in(&mut shadow, write, Instant::now());
}

fn remember_power_button_write_in(
    shadow: &mut PowerButtonShadow,
    write: PowerButtonWrite,
    now: Instant,
) {
    let now = Some(now);
    if write.ac {
        shadow.rgb_ac = Some(write.colours.ac);
        shadow.last_write_ac = now;
    }
    if write.battery {
        shadow.rgb_battery = Some(write.colours.battery);
        shadow.last_write_battery = now;
    }
    shadow.last_write_charging = now;
}

fn power_button_shadow() -> &'static Mutex<PowerButtonShadow> {
    static SHADOW: OnceLock<Mutex<PowerButtonShadow>> = OnceLock::new();
    SHADOW.get_or_init(|| Mutex::new(PowerButtonShadow::default()))
}

fn send_animation_envelope(
    device: &dyn HidChannel,
    opcode: u8,
    subcommand: u16,
    animation_id: u16,
    duration: u16,
) -> Result<(), String> {
    let report = animation_envelope_report(opcode, subcommand, animation_id, duration);
    write_feature_report(device, &report)
}

fn animation_envelope_report(
    opcode: u8,
    subcommand: u16,
    animation_id: u16,
    duration: u16,
) -> [u8; REPORT_LEN] {
    let mut report = empty_report(opcode);
    write_be_u16(&mut report, 2, subcommand);
    write_be_u16(&mut report, 4, animation_id);
    write_be_u16(&mut report, 6, duration);
    report
}

fn send_select_zones(device: &dyn HidChannel, zones: &[Zone]) -> Result<(), String> {
    let report = select_zones_report(zones)?;
    write_feature_report(device, &report)
}

fn select_zones_report(zones: &[Zone]) -> Result<[u8; REPORT_LEN], String> {
    const MAX_SELECTED_ZONES: usize = REPORT_LEN - (WIRE_OFFSET + 5);
    if zones.len() > MAX_SELECTED_ZONES {
        return Err(format!(
            "AW-ELC zone selection contains {} zones; at most {MAX_SELECTED_ZONES} fit in one report",
            zones.len()
        ));
    }
    let count =
        u8::try_from(zones.len()).map_err(|_| "too many AW-ELC zones selected".to_owned())?;
    let mut report = empty_report(OP_SELECT_ZONES);
    report[WIRE_OFFSET + 2] = 0x01;
    report[WIRE_OFFSET + 4] = count;
    for (index, zone) in zones.iter().enumerate() {
        report[WIRE_OFFSET + 5 + index] = zone.id();
    }
    Ok(report)
}

fn send_action_records(device: &dyn HidChannel, records: &[ActionRecord]) -> Result<(), String> {
    let report = action_records_report(records)?;
    write_feature_report(device, &report)
}

fn action_records_report(records: &[ActionRecord]) -> Result<[u8; REPORT_LEN], String> {
    if records.len() > 3 {
        return Err("one AW-ELC action report can carry at most three records".to_owned());
    }

    let mut report = empty_report(OP_ADD_ACTION);
    for (index, record) in records.iter().enumerate() {
        let base = 2 + (index * 8);
        report[WIRE_OFFSET + base] = record.mode;
        write_be_u16(&mut report, base + 1, record.duration_ms);
        write_be_u16(&mut report, base + 3, record.tempo);
        report[WIRE_OFFSET + base + 5] = record.rgb.r;
        report[WIRE_OFFSET + base + 6] = record.rgb.g;
        report[WIRE_OFFSET + base + 7] = record.rgb.b;
    }
    Ok(report)
}

fn empty_report(opcode: u8) -> [u8; REPORT_LEN] {
    let mut report = [0_u8; REPORT_LEN];
    report[WIRE_OFFSET] = WIRE_ID;
    report[WIRE_OFFSET + 1] = opcode;
    report
}

fn write_be_u16(report: &mut [u8; REPORT_LEN], offset: usize, value: u16) {
    let [high, low] = value.to_be_bytes();
    report[WIRE_OFFSET + offset] = high;
    report[WIRE_OFFSET + offset + 1] = low;
}

#[cfg(test)]
#[path = "aw_elc_tests.rs"]
mod tests;
