// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Alienware keyboard discovery and lighting-update application.

#![allow(
    clippy::indexing_slicing,
    reason = "Keyboard reports are fixed-layout HID packets with checked offsets."
)]

use std::collections::{BTreeMap, BTreeSet};

use luminate_core::effect::Effect;
use luminate_core::frame::{FrameEnvelope, FramePayload};
use luminate_core::rgb::Rgb;
use luminate_plugin_api::sdk::{CompleteShadow, stage_map_updates};
use luminate_plugin_api::{PluginTarget, PluginUpdateOperation, ShadowState};

use crate::keyboard_layout::{self, KeyboardIdentity};
use crate::topology::{
    AW_KEYBOARD_BREATHE_EFFECT_ID, AW_KEYBOARD_PULSE_EFFECT_ID, AW_KEYBOARD_RAINBOW_EFFECT_ID,
    AW_KEYBOARD_SPECTRUM_EFFECT_ID, KEYBOARD_DEVICE_ID,
};

use super::keyboard_custom;
use super::{
    HidChannel, HidTransport, HidUsage, effect_to_rgb_static, rgb_from_colour, write_feature_report,
};

/// The keyboard's vendor-defined `AlienFX` control collection. Confirmed
/// against real hardware on Windows: of the keyboard's four top-level
/// collections (a second vendor-defined one of unknown purpose, this one,
/// the standard keyboard collection, and standard consumer control), only
/// this one works.
const VENDOR_USAGE: HidUsage = HidUsage {
    page: 0xff89,
    usage: 0x00cc,
};

/// In-process shadow of applied per-key colours, keyed by canonical matrix
/// index. There is no per-key hardware readback (protocol spec §6.2), so
/// partial writes are rejected until a complete frame has established every
/// named key. This avoids replacing unknown key colours with black.
/// Persists for the lifetime of the plugin (plugins are never unloaded), so
/// it survives across mutations after a complete frame initializes it.
static KEY_COLOURS: ShadowState<u8, Rgb> = ShadowState::new();

const REPORT_LEN: usize = 64;
const REPORT_ID: u8 = 0xcc;
const OP_BUILTIN_EFFECT: u8 = 0x80;
const OP_BRIGHTNESS_FINALIZER: u8 = 0x83;
const OP_PERSIST_ACTIVE_EFFECT: u8 = 0x84;
const OP_LAYOUT_IDENTITY: u8 = 0x93;
#[cfg(test)]
const OFF_RGB: Rgb = Rgb::new(0, 0, 0);

#[derive(Debug, Clone)]
struct ResolvedPerKey {
    index: u8,
    rgb: Rgb,
    assignment_indices: Vec<u8>,
}

pub fn read_layout_identity(
    transport: &dyn HidTransport,
    vendor_id: u16,
    product_id: u16,
) -> Result<KeyboardIdentity, String> {
    let device = transport.open(vendor_id, product_id, VENDOR_USAGE)?;

    // `GET_FEATURE` only uses the leading report-id byte as input and
    // overwrites the rest of the buffer with whatever the device currently
    // holds for that report id. It is not a way to pass an argument. Select
    // the identity sub-report with an explicit `SET_FEATURE` write first,
    // then fetch the prepared response.
    let mut query = [0_u8; REPORT_LEN];
    query[0] = REPORT_ID;
    query[1] = OP_LAYOUT_IDENTITY;
    write_feature_report(device.as_ref(), &query)?;

    let mut report = [0_u8; REPORT_LEN];
    report[0] = REPORT_ID;
    let length = device
        .get_feature_report(&mut report)
        .map_err(|error| format!("failed to read keyboard layout identity: {error}"))?;

    keyboard_layout::parse_identity_report(&report[..length])
        .ok_or_else(|| "keyboard layout identity report was too short".to_owned())
}

#[allow(
    clippy::too_many_lines,
    reason = "The keyboard topology table is a dense hardware layout declaration."
)]
pub fn apply(
    transport: &dyn HidTransport,
    vendor_id: u16,
    product_id: u16,
    target: &PluginTarget,
    operation: &PluginUpdateOperation,
) -> Result<(), String> {
    if let PluginTarget::Element {
        surface, element, ..
    } = target
    {
        return apply_per_key(
            transport, vendor_id, product_id, surface, element, operation,
        );
    }

    ensure_whole_keyboard_target(target)?;

    let device = transport.open(vendor_id, product_id, VENDOR_USAGE)?;

    match operation {
        PluginUpdateOperation::SetEffect {
            effect: Effect::Static { .. } | Effect::Off,
        }
        | PluginUpdateOperation::Clear => {
            let rgb = effect_to_rgb_static(operation)?;
            send_builtin_effect(
                device.as_ref(),
                0x01,
                0x01,
                0x01,
                [0x01, 0x01, 0x01, 0x00],
                rgb,
                rgb,
            )?;
            invalidate_per_key_shadow(operation);
            send_brightness_finalizer(device.as_ref(), 100)
        }
        PluginUpdateOperation::SetEffect {
            effect: Effect::Hardware { id, arguments },
        } if matches!(
            id.as_str(),
            AW_KEYBOARD_BREATHE_EFFECT_ID | AW_KEYBOARD_PULSE_EFFECT_ID
        ) =>
        {
            let colour = hardware_colour(arguments.colours.first(), id.as_str())?;
            send_builtin_effect(
                device.as_ref(),
                0x02,
                0x07,
                0x05,
                [0x01, 0x01, 0x01, 0x00],
                colour,
                colour,
            )?;
            invalidate_per_key_shadow(operation);
            send_brightness_finalizer(device.as_ref(), 100)
        }
        PluginUpdateOperation::SetEffect {
            effect: Effect::Hardware { id, .. },
        } if id.as_str() == AW_KEYBOARD_SPECTRUM_EFFECT_ID => {
            let placeholder = Rgb::new(0xff, 0x00, 0x00);
            send_builtin_effect(
                device.as_ref(),
                0x0e,
                0x02,
                0x06,
                [0x01, 0x01, 0x01, 0x01],
                placeholder,
                placeholder,
            )?;
            invalidate_per_key_shadow(operation);
            send_brightness_finalizer(device.as_ref(), 100)
        }
        PluginUpdateOperation::SetEffect {
            effect: Effect::Hardware { id, .. },
        } if id.as_str() == AW_KEYBOARD_RAINBOW_EFFECT_ID => {
            let placeholder = Rgb::new(0xff, 0x00, 0x00);
            send_builtin_effect(
                device.as_ref(),
                0x03,
                0x05,
                0x05,
                [0x01, 0x01, 0x01, 0x01],
                placeholder,
                placeholder,
            )?;
            invalidate_per_key_shadow(operation);
            send_brightness_finalizer(device.as_ref(), 100)
        }
        PluginUpdateOperation::SetEffect {
            effect:
                Effect::Breathe { .. }
                | Effect::Pulse { .. }
                | Effect::Strobe { .. }
                | Effect::Spectrum { .. }
                | Effect::Rainbow { .. }
                | Effect::Scanner { .. },
        } => {
            Err("keyboard built-in animations are exposed as Alienware hardware effects".to_owned())
        }
        PluginUpdateOperation::SetEffect {
            effect: Effect::Morph { .. },
        } => {
            Err("keyboard morph requires the custom-animation path, not implemented yet".to_owned())
        }
        PluginUpdateOperation::SetEffect {
            effect: Effect::Hardware { id, arguments },
        } if id.as_str() == "aw-sweeper" => {
            // The firmware side-to-side sweep the daemon validated against
            // `sweeper_effect`'s single-colour schema, driven by the built-in
            // 0x0a "scanner" animation.
            let colour = arguments
                .colours
                .first()
                .copied()
                .ok_or_else(|| "aw-sweeper requires one colour".to_owned())?;
            send_builtin_effect(
                device.as_ref(),
                0x0a,
                0x07,
                0x06,
                [0x01, 0x01, 0x01, 0x00],
                colour,
                colour,
            )?;
            invalidate_per_key_shadow(operation);
            send_brightness_finalizer(device.as_ref(), 100)
        }
        PluginUpdateOperation::SetEffect {
            effect: Effect::Hardware { .. },
        } => Err("unknown keyboard hardware effect".to_owned()),
        PluginUpdateOperation::SetBrightness { value } => {
            let percent = u8::try_from((*value).min(100))
                .map_err(|_| "brightness value could not be represented".to_owned())?;
            send_brightness_finalizer(device.as_ref(), percent)
        }
        PluginUpdateOperation::SaveCurrent => send_persist_active_effect(device.as_ref()),
        PluginUpdateOperation::SetAppearanceSlots { .. } => {
            Err("Alienware keyboards do not advertise appearance slots".to_owned())
        }
    }
}

fn hardware_colour(colour: Option<&Rgb>, effect_id: &str) -> Result<Rgb, String> {
    colour
        .copied()
        .ok_or_else(|| format!("{effect_id} requires one colour"))
}

/// Validates and resolves a per-key target/operation down to the matrix
/// index and static colour it addresses, without touching hardware or the
/// shadow map. Shared by the single-update path (`apply_per_key`) and the
/// batch path (`apply_per_key_batch`).
fn resolve_static_per_key(
    transport: &dyn HidTransport,
    vendor_id: u16,
    product_id: u16,
    surface: &str,
    element: &str,
    operation: &PluginUpdateOperation,
) -> Result<ResolvedPerKey, String> {
    if surface != "keyboard" {
        return Err(format!("unknown keyboard surface target: {surface}"));
    }
    if matches!(operation, PluginUpdateOperation::SaveCurrent) {
        return Err("keyboard firmware save-current is not implemented yet".to_owned());
    }
    if matches!(operation, PluginUpdateOperation::SetBrightness { .. }) {
        return Err("per-key targets do not support independent brightness".to_owned());
    }
    if matches!(
        operation,
        PluginUpdateOperation::SetEffect {
            effect: Effect::Breathe { .. }
                | Effect::Pulse { .. }
                | Effect::Strobe { .. }
                | Effect::Spectrum { .. }
                | Effect::Rainbow { .. }
                | Effect::Scanner { .. }
                | Effect::Morph { .. },
        }
    ) {
        return Err(
            "per-key targets only support a static colour, not animated effects".to_owned(),
        );
    }

    let rgb = effect_to_rgb_static(operation)?;

    let identity = read_layout_identity(transport, vendor_id, product_id)?;
    let key_map = identity.layout_id().key_map().ok_or_else(|| {
        "keyboard layout not identified; per-key targets are unavailable".to_owned()
    })?;
    let index = key_map
        .index_for_name(element)
        .ok_or_else(|| format!("unknown keyboard key: {element}"))?;
    let assignment_indices = key_map
        .named_positions()
        .map(|(_, matrix_index)| matrix_index)
        .collect();

    Ok(ResolvedPerKey {
        index,
        rgb,
        assignment_indices,
    })
}

fn apply_per_key(
    transport: &dyn HidTransport,
    vendor_id: u16,
    product_id: u16,
    surface: &str,
    element: &str,
    operation: &PluginUpdateOperation,
) -> Result<(), String> {
    let resolved = resolve_static_per_key(
        transport, vendor_id, product_id, surface, element, operation,
    )?;

    let mut colours = KEY_COLOURS.lock().expect("lock poisoned");
    let entry = (resolved.index, resolved.rgb);
    let staged_colours = stage_per_key_shadow(&colours, &[entry], &resolved.assignment_indices)?;

    let device = transport.open(vendor_id, product_id, VENDOR_USAGE)?;
    let hardware_result = keyboard_custom::update_keys_colour(device.as_ref(), &[entry]);

    if hardware_result.is_ok() {
        *colours = staged_colours;
    } else {
        colours.clear();
    }

    hardware_result
}

/// Batched form of `apply_per_key`: resolves every entry independently (so
/// one unknown key name doesn't fail its siblings), then stages every
/// successfully-resolved `(index, colour)` pair under one lock acquisition.
/// A complete frame must already have established every named key; otherwise
/// the batch is rejected without opening the device. The staged shadow is
/// committed only after the one lightweight hardware update succeeds.
///
/// The hardware outcome (success or failure) is shared across every
/// successfully-resolved entry in the group, since they were applied by the
/// same HID transaction. This batching API has no per-entry atomicity beyond
/// that.
pub fn apply_per_key_batch(
    transport: &dyn HidTransport,
    vendor_id: u16,
    product_id: u16,
    entries: &[(&str, &str, &PluginUpdateOperation)],
) -> Vec<Result<(), String>> {
    let resolved = entries
        .iter()
        .map(|&(surface, element, operation)| {
            resolve_static_per_key(
                transport, vendor_id, product_id, surface, element, operation,
            )
        })
        .collect::<Vec<_>>();

    let ok_entries = resolved
        .iter()
        .filter_map(|result| result.as_ref().ok())
        .map(|resolved| (resolved.index, resolved.rgb))
        .collect::<Vec<_>>();
    let assignment_indices = resolved
        .iter()
        .filter_map(|result| result.as_ref().ok())
        .flat_map(|resolved| resolved.assignment_indices.iter().copied())
        .collect::<BTreeSet<_>>();

    if ok_entries.is_empty() {
        return resolved
            .into_iter()
            .map(|result| result.map(|_| ()))
            .collect();
    }

    let mut colours = KEY_COLOURS.lock().expect("lock poisoned");
    let assignment_indices = assignment_indices.into_iter().collect::<Vec<_>>();
    let staged_colours = match stage_per_key_shadow(&colours, &ok_entries, &assignment_indices) {
        Ok(staged) => staged,
        Err(error) => {
            return resolved
                .into_iter()
                .map(|result| match result {
                    Ok(_) => Err(error.clone()),
                    Err(error) => Err(error),
                })
                .collect();
        }
    };

    let device = match transport.open(vendor_id, product_id, VENDOR_USAGE) {
        Ok(device) => device,
        Err(error) => return resolved.into_iter().map(|_| Err(error.clone())).collect(),
    };
    let hardware_result = keyboard_custom::update_keys_colour(device.as_ref(), &ok_entries);

    if hardware_result.is_ok() {
        *colours = staged_colours;
    } else {
        colours.clear();
    }

    resolved
        .into_iter()
        .map(|result| match result {
            Ok(_) => hardware_result.clone(),
            Err(error) => Err(error),
        })
        .collect()
}

/// Applies a complete volatile frame in the topology's published key order.
///
/// The first frame initializes the firmware's in-memory assignment table.
/// Later frames send only changed RGB records. Neither path issues the
/// separate `cc:84` persistence command.
pub fn apply_frame(
    transport: &dyn HidTransport,
    vendor_id: u16,
    product_id: u16,
    layout: keyboard_layout::KeyboardLayoutId,
    target: &PluginTarget,
    envelope: &FrameEnvelope,
) -> Result<(), String> {
    match target {
        PluginTarget::Surface { device, surface }
            if device == KEYBOARD_DEVICE_ID && surface == "keyboard" => {}
        PluginTarget::Device { .. }
        | PluginTarget::Surface { .. }
        | PluginTarget::Element { .. }
        | PluginTarget::Group { .. } => {
            return Err(
                "keyboard frame streaming is only supported on the keyboard surface".to_owned(),
            );
        }
    }

    let key_map = layout.key_map().ok_or_else(|| {
        "keyboard layout not identified; frame streaming is unavailable".to_owned()
    })?;
    let FramePayload::Full(pixels) = &envelope.payload else {
        return Err("keyboard frame streaming only supports full frames".to_owned());
    };
    let positions = key_map.named_positions().collect::<Vec<_>>();
    if pixels.len() != positions.len() {
        return Err(format!(
            "expected {} pixels for a full keyboard frame, got {}",
            positions.len(),
            pixels.len()
        ));
    }

    let entries = positions
        .iter()
        .zip(pixels)
        .map(|((_, index), colour)| rgb_from_colour(colour).map(|rgb| (*index, rgb)))
        .collect::<Result<Vec<_>, _>>()?;
    let staged_colours = entries.iter().copied().collect::<BTreeMap<_, _>>();
    let device = transport.open(vendor_id, product_id, VENDOR_USAGE)?;
    let mut colours = KEY_COLOURS.lock().expect("lock poisoned");

    let initialized = entries.iter().all(|(index, _)| colours.contains_key(index));
    let changed = entries
        .iter()
        .copied()
        .filter(|(index, rgb)| colours.get(index) != Some(rgb))
        .collect::<Vec<_>>();
    colours.clear();
    let hardware_result = if initialized {
        keyboard_custom::update_keys_colour(device.as_ref(), &changed)
    } else {
        keyboard_custom::apply_custom_colours(device.as_ref(), &staged_colours)
    };

    if hardware_result.is_ok() {
        *colours = staged_colours;
    }

    hardware_result
}

fn stage_per_key_shadow(
    current: &BTreeMap<u8, Rgb>,
    entries: &[(u8, Rgb)],
    assignment_indices: &[u8],
) -> Result<BTreeMap<u8, Rgb>, String> {
    stage_map_updates(
        &if current.is_empty() {
            CompleteShadow::Unknown
        } else {
            CompleteShadow::Complete(current)
        },
        assignment_indices.iter().copied(),
        entries.iter().copied(),
    )
    .map_err(|error| format!(
        "per-key updates require a complete keyboard frame written in this plugin session: {error}"
    ))
}

fn invalidate_per_key_shadow(operation: &PluginUpdateOperation) {
    if !operation_invalidates_per_key_shadow(operation) {
        return;
    }

    KEY_COLOURS.lock().expect("lock poisoned").clear();
}

pub(crate) fn invalidate_per_key_shadow_for_discovery_change() {
    KEY_COLOURS.lock().expect("lock poisoned").clear();
}

fn operation_invalidates_per_key_shadow(operation: &PluginUpdateOperation) -> bool {
    matches!(
        operation,
        PluginUpdateOperation::SetEffect { .. } | PluginUpdateOperation::Clear
    )
}

fn ensure_whole_keyboard_target(target: &PluginTarget) -> Result<(), String> {
    match target {
        PluginTarget::Device { .. } => Ok(()),
        PluginTarget::Surface { surface, .. } if surface == "keyboard" => Ok(()),
        PluginTarget::Group { group, .. } if group == "all" => Ok(()),
        PluginTarget::Surface { .. }
        | PluginTarget::Group { .. }
        | PluginTarget::Element { .. } => {
            Err("keyboard plugin currently supports whole-keyboard targets only".to_owned())
        }
    }
}

fn send_builtin_effect(
    device: &dyn HidChannel,
    effect: u8,
    timing_a: u8,
    timing_b: u8,
    flags: [u8; 4],
    primary: Rgb,
    secondary: Rgb,
) -> Result<(), String> {
    let mut report = [0_u8; REPORT_LEN];
    report[0] = REPORT_ID;
    report[1] = OP_BUILTIN_EFFECT;
    report[2] = effect;
    report[3] = timing_a;
    report[6..10].copy_from_slice(&flags);
    report[10] = primary.r;
    report[11] = primary.g;
    report[12] = primary.b;
    report[13] = secondary.r;
    report[14] = secondary.g;
    report[15] = secondary.b;
    report[16] = timing_b;

    write_feature_report(device, &report)
}

fn send_brightness_finalizer(device: &dyn HidChannel, percent: u8) -> Result<(), String> {
    let mut report = [0_u8; REPORT_LEN];
    report[0] = REPORT_ID;
    report[1] = OP_BRIGHTNESS_FINALIZER;
    report[2] = 0x38;
    report[3] = 0x9c;
    report[4] = scale_percent_to_keyboard_brightness(percent);

    write_feature_report(device, &report)
}

/// Persists the currently active whole-keyboard effect to non-volatile
/// firmware storage (protocol spec §7, `cc:84:03:00`). AWCC calls this from
/// `SaveCurrentEffectsToLastEffect()` right after applying an effect;
/// confirmed experimentally to survive a full power-off/reboot cycle. The
/// command has no colour/effect payload of its own; it commits whatever the
/// keyboard is already rendering, so there is nothing to stage or shadow here.
fn send_persist_active_effect(device: &dyn HidChannel) -> Result<(), String> {
    let mut report = [0_u8; REPORT_LEN];
    report[0] = REPORT_ID;
    report[1] = OP_PERSIST_ACTIVE_EFFECT;
    report[2] = 0x03;

    write_feature_report(device, &report)
}

fn scale_percent_to_keyboard_brightness(percent: u8) -> u8 {
    let clamped = u16::from(percent.min(100));
    u8::try_from(((clamped * 254) + 50) / 100).unwrap_or(0xfe)
}

#[cfg(test)]
#[path = "keyboard_tests.rs"]
mod tests;
