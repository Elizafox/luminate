// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Mutation validation: colour resolution/emulation, brightness bounds,
//! effect and hardware-effect-argument validation, and the shared
//! `DaemonError` constructors used throughout validation.

use std::collections::HashSet;

use luminate_core::appearance_slot::{
    AppearanceCapability, AppearanceSlotUpdatePolicy, AppearanceSlotValue,
};
use luminate_core::capability::{
    BrightnessCapability, CapabilityScope, CapabilitySet, CctEmulation, ColourCapability,
    ColourChannel, ColourEncoding, EffectParameter, HardwareEffectDescriptor,
    HardwareEffectsCapability,
};
use luminate_core::colour::Colour;
use luminate_core::effect::{Effect, EffectArguments};
use luminate_core::rgb;
use luminate_core::target::TargetId;

use crate::error::DaemonError;

use super::DaemonState;
use super::target_state::TargetState;

trait AppearanceCapabilities {
    fn colour(&self) -> &[ColourCapability];
    fn cct_emulation(&self) -> CctEmulation;
    fn hardware_effects(&self) -> Option<&HardwareEffectsCapability>;
}

impl AppearanceCapabilities for CapabilitySet {
    fn colour(&self) -> &[ColourCapability] {
        &self.colour
    }

    fn cct_emulation(&self) -> CctEmulation {
        self.cct_emulation
    }

    fn hardware_effects(&self) -> Option<&HardwareEffectsCapability> {
        self.hardware_effects.as_ref()
    }
}

impl AppearanceCapabilities for AppearanceCapability {
    fn colour(&self) -> &[ColourCapability] {
        &self.colour
    }

    fn cct_emulation(&self) -> CctEmulation {
        self.cct_emulation
    }

    fn hardware_effects(&self) -> Option<&HardwareEffectsCapability> {
        self.hardware_effects.as_ref()
    }
}

impl DaemonState {
    pub(crate) fn validate_observed_appearance_slot_values(
        &self,
        target: &TargetId,
        values: &[AppearanceSlotValue],
    ) -> Result<bool, DaemonError> {
        if values.is_empty() {
            return Err(invalid(target, "appearance-slot observation is empty"));
        }
        let slots = self
            .capabilities_for_target_or_error(target)?
            .appearance_slots
            .as_ref()
            .ok_or_else(|| unsupported(target, "target does not advertise appearance slots"))?;
        let mut seen = HashSet::with_capacity(values.len());
        for value in values {
            if !seen.insert(value.slot.clone()) {
                return Err(invalid(
                    target,
                    format!("appearance slot {:?} occurs more than once", value.slot),
                ));
            }
            let descriptor = slots
                .slots
                .iter()
                .find(|descriptor| descriptor.id == value.slot)
                .ok_or_else(|| {
                    invalid(target, format!("unknown appearance slot {:?}", value.slot))
                })?;
            validate_effect_for_appearance(
                target,
                &value.effect,
                &descriptor.appearance,
                self.cct_emulation_override,
            )?;
        }
        Ok(seen.len() == slots.slots.len())
    }

    pub(crate) fn validate_appearance_slot_values(
        &self,
        target: &TargetId,
        values: &[AppearanceSlotValue],
    ) -> Result<Vec<AppearanceSlotValue>, DaemonError> {
        if !matches!(target, TargetId::Surface { .. }) {
            return Err(DaemonError::InvalidArgument {
                target: target.clone(),
                reason: "appearance slots require one concrete surface".to_owned(),
            });
        }
        if values.is_empty() {
            return Err(invalid(target, "appearance-slot mutation is empty"));
        }

        let capabilities = self.capabilities_for_target_or_error(target)?;
        let slots = capabilities
            .appearance_slots
            .as_ref()
            .ok_or_else(|| unsupported(target, "target does not advertise appearance slots"))?;
        let mut seen = HashSet::with_capacity(values.len());
        let mut resolved = Vec::with_capacity(values.len());
        for value in values {
            if !seen.insert(value.slot.clone()) {
                return Err(invalid(
                    target,
                    format!("appearance slot {:?} occurs more than once", value.slot),
                ));
            }
            let descriptor = slots
                .slots
                .iter()
                .find(|descriptor| descriptor.id == value.slot)
                .ok_or_else(|| {
                    invalid(target, format!("unknown appearance slot {:?}", value.slot))
                })?;
            let effect = match &value.effect {
                Effect::Static { colour } => Effect::Static {
                    colour: resolve_colour_for_appearance(
                        target,
                        colour,
                        &descriptor.appearance,
                        self.cct_emulation_override,
                    )?,
                },
                effect @ (Effect::Off
                | Effect::Breathe { .. }
                | Effect::Pulse { .. }
                | Effect::Strobe { .. }
                | Effect::Scanner { .. }
                | Effect::Morph { .. }
                | Effect::Spectrum { .. }
                | Effect::Rainbow { .. }
                | Effect::Hardware { .. }) => {
                    validate_effect_for_appearance(
                        target,
                        effect,
                        &descriptor.appearance,
                        self.cct_emulation_override,
                    )?;
                    effect.clone()
                }
            };
            resolved.push(AppearanceSlotValue {
                slot: value.slot.clone(),
                effect,
            });
        }

        if seen.len() != slots.slots.len() {
            match slots.update_policy {
                AppearanceSlotUpdatePolicy::Independent => {}
                AppearanceSlotUpdatePolicy::PartialIfKnown => {
                    let known = self.known_appearance_slot_values(target);
                    for descriptor in &slots.slots {
                        if seen.contains(&descriptor.id) {
                            continue;
                        }
                        let value = known
                            .iter()
                            .find(|value| value.slot == descriptor.id)
                            .ok_or_else(|| DaemonError::UnknownState {
                                target: target.clone(),
                            })?;
                        resolved.push(value.clone());
                    }
                }
                AppearanceSlotUpdatePolicy::CompleteSet => {
                    return Err(invalid(
                        target,
                        "appearance-slot mutation must include every slot",
                    ));
                }
            }
        }
        Ok(resolved)
    }

    /// Sets the daemon-wide `Cct` emulation override (from `DaemonConfig`).
    pub fn set_cct_emulation_override(&mut self, override_: Option<CctEmulation>) {
        self.cct_emulation_override = override_;
    }

    /// Determine if the colour is valid for the given `target`.
    pub fn validate_colour(&self, target: &TargetId, colour: &Colour) -> Result<(), DaemonError> {
        self.resolve_colour(target, colour).map(|_| ())
    }

    /// Resolves the colour to apply at `target`.
    ///
    /// If the target has a native capability matching the requested
    /// encoding, the colour is returned unchanged. Otherwise, if the request
    /// is `Cct` and emulation is permitted, an approximate `Additive` colour
    /// is returned instead. Returns an error if neither a native nor an
    /// emulated representation is available.
    pub fn resolve_colour(
        &self,
        target: &TargetId,
        colour: &Colour,
    ) -> Result<Colour, DaemonError> {
        let capabilities = self.capabilities_for_target_or_error(target)?;

        resolve_colour_for_appearance(target, colour, capabilities, self.cct_emulation_override)
    }

    /// Determine if the given effect is valid for the given `target`.
    pub fn validate_effect(&self, target: &TargetId, effect: &Effect) -> Result<(), DaemonError> {
        let capabilities = self.capabilities_for_target_or_error(target)?;

        validate_effect_for_appearance(target, effect, capabilities, self.cct_emulation_override)
    }

    pub(crate) fn validate_target_state(
        &self,
        target: &TargetId,
        target_state: &TargetState,
    ) -> Result<(), DaemonError> {
        match target_state {
            TargetState::Effect(effect) => self.validate_effect(target, effect),
            TargetState::Brightness(value) => self.validate_brightness(target, *value),
            TargetState::AppearanceSlots(values) => self
                .validate_appearance_slot_values(target, values)
                .map(|_| ()),
            TargetState::Clear => self.ensure_target_exists(target),
        }
    }
}

fn resolve_colour_for_appearance(
    target: &TargetId,
    colour: &Colour,
    capabilities: &impl AppearanceCapabilities,
    cct_emulation_override: Option<CctEmulation>,
) -> Result<Colour, DaemonError> {
    if let Some(capability) = capabilities
        .colour()
        .iter()
        .find(|capability| capability_matches_colour(capability, colour))
    {
        validate_colour_channels(target, colour, capability)?;
        return Ok(colour.clone());
    }

    if colour.encoding() == ColourEncoding::Cct {
        let emulation = if capabilities.cct_emulation() == CctEmulation::Disabled {
            CctEmulation::Disabled
        } else {
            cct_emulation_override.unwrap_or(CctEmulation::Auto)
        };
        let kelvin = colour.channel(ColourChannel::Temperature);
        let additive = capabilities
            .colour()
            .iter()
            .find(|capability| matches!(capability, ColourCapability::Additive(_)));
        if let (CctEmulation::Auto, Some(kelvin), Some(capability)) = (emulation, kelvin, additive)
        {
            let emulated = Colour::rgb(rgb::kelvin_to_rgb(kelvin));
            validate_colour_channels(target, &emulated, capability)?;
            return Ok(emulated);
        }
    }

    if capabilities.colour().is_empty() {
        return Err(unsupported(target, "target has no colour capability"));
    }
    Err(invalid(
        target,
        format!(
            "colour encoding {:?} does not match any advertised encoding",
            colour.encoding()
        ),
    ))
}

fn validate_colour_channels(
    target: &TargetId,
    colour: &Colour,
    capability: &ColourCapability,
) -> Result<(), DaemonError> {
    let (values, channels) = match (colour, capability) {
        (Colour::Additive(values), ColourCapability::Additive(channels)) => {
            (values.as_slice(), channels.as_slice())
        }
        (
            Colour::Hsv {
                hue,
                saturation,
                value,
            },
            ColourCapability::Hsv {
                hue_bits,
                saturation_bits,
                value_bits,
            },
        ) => {
            return validate_fixed_channels(
                target,
                &[
                    ("hue", *hue, *hue_bits),
                    ("saturation", *saturation, *saturation_bits),
                    ("value", *value, *value_bits),
                ],
            );
        }
        (
            Colour::Hsl {
                hue,
                saturation,
                lightness,
            },
            ColourCapability::Hsl {
                hue_bits,
                saturation_bits,
                lightness_bits,
            },
        ) => {
            return validate_fixed_channels(
                target,
                &[
                    ("hue", *hue, *hue_bits),
                    ("saturation", *saturation, *saturation_bits),
                    ("lightness", *lightness, *lightness_bits),
                ],
            );
        }
        (Colour::Cct { kelvin }, ColourCapability::Cct { bits }) => {
            return validate_fixed_channels(target, &[("temperature", *kelvin, *bits)]);
        }
        (Colour::Monochrome { intensity }, ColourCapability::Monochrome { bits }) => {
            return validate_fixed_channels(target, &[("intensity", *intensity, *bits)]);
        }
        _ => return Err(invalid(target, "colour model does not match capability")),
    };

    if values.len() != channels.len() {
        return Err(invalid(
            target,
            format!(
                "colour requires exactly {} channels, received {}",
                channels.len(),
                values.len()
            ),
        ));
    }
    for value in values {
        let Some(channel) = channels
            .iter()
            .find(|channel| channel.channel == value.channel)
        else {
            return Err(invalid(
                target,
                format!("channel {:?} is not advertised", value.channel),
            ));
        };
        let Some(maximum) = maximum_for_bits(channel.bits) else {
            return Err(unsupported(
                target,
                format!(
                    "advertised channel {:?} has invalid bit width {}",
                    value.channel, channel.bits
                ),
            ));
        };
        if value.value > maximum {
            return Err(invalid(
                target,
                format!(
                    "channel {:?} value {} exceeds {maximum} for {} bits",
                    value.channel, value.value, channel.bits
                ),
            ));
        }
    }

    Ok(())
}

impl DaemonState {
    /// Validate the brightness value for the given target.
    pub fn validate_brightness(&self, target: &TargetId, value: u32) -> Result<(), DaemonError> {
        let capabilities = self.capabilities_for_target_or_error(target)?;
        let BrightnessCapability::Independent {
            bits,
            maximum,
            scope,
        } = capabilities.brightness
        else {
            return Err(unsupported(
                target,
                "target has no independent brightness capability",
            ));
        };

        validate_scope(target, scope)?;
        let Some(representable_maximum) = maximum_for_bits(bits) else {
            return Err(unsupported(
                target,
                format!("invalid brightness bit width {bits}"),
            ));
        };
        if maximum > representable_maximum {
            return Err(unsupported(
                target,
                format!("brightness maximum {maximum} exceeds {bits}-bit representation"),
            ));
        }
        if value > maximum {
            return Err(invalid(
                target,
                format!("brightness {value} exceeds advertised maximum {maximum}"),
            ));
        }

        Ok(())
    }
}

fn validate_effect_for_appearance(
    target: &TargetId,
    effect: &Effect,
    capabilities: &impl AppearanceCapabilities,
    cct_emulation_override: Option<CctEmulation>,
) -> Result<(), DaemonError> {
    if matches!(effect, Effect::Off) {
        if !capabilities.colour().is_empty()
            || find_effect(capabilities, effect.capability_id()).is_some()
        {
            return Ok(());
        }
        return Err(unsupported(target, "target cannot be turned off"));
    }

    if let Effect::Static { colour } = effect {
        return resolve_colour_for_appearance(target, colour, capabilities, cct_emulation_override)
            .map(|_| ());
    }

    let Some(effects) = capabilities.hardware_effects() else {
        return Err(unsupported(
            target,
            format!("effect {} is not advertised", effect.capability_id()),
        ));
    };
    let Some(descriptor) = effects
        .effects
        .iter()
        .find(|descriptor| descriptor.id.as_str() == effect.capability_id())
    else {
        return Err(unsupported(
            target,
            format!("effect {} is not advertised", effect.capability_id()),
        ));
    };
    validate_scope(target, effects.scope)?;
    if let Effect::Hardware { arguments, .. } = effect {
        return validate_hardware_arguments(target, descriptor, arguments);
    }
    validate_effect_parameters(target, descriptor, effect)
}

fn maximum_for_bits(bits: u8) -> Option<u32> {
    match bits {
        1..=31 => Some((1_u32 << bits) - 1),
        32 => Some(u32::MAX),
        0 | 33..=u8::MAX => None,
    }
}

fn capability_matches_colour(capability: &ColourCapability, colour: &Colour) -> bool {
    matches!(
        (capability, colour),
        (ColourCapability::Additive(_), Colour::Additive(_))
            | (ColourCapability::Hsv { .. }, Colour::Hsv { .. })
            | (ColourCapability::Hsl { .. }, Colour::Hsl { .. })
            | (ColourCapability::Cct { .. }, Colour::Cct { .. })
            | (
                ColourCapability::Monochrome { .. },
                Colour::Monochrome { .. }
            )
    )
}

fn validate_fixed_channels(
    target: &TargetId,
    channels: &[(&str, u32, u8)],
) -> Result<(), DaemonError> {
    for (name, value, bits) in channels {
        let Some(maximum) = maximum_for_bits(*bits) else {
            return Err(unsupported(
                target,
                format!("advertised {name} channel has invalid bit width {bits}"),
            ));
        };
        if *value > maximum {
            return Err(invalid(
                target,
                format!("{name} value {value} exceeds {maximum} for {bits} bits"),
            ));
        }
    }
    Ok(())
}

fn find_effect<'a>(
    capabilities: &'a impl AppearanceCapabilities,
    id: &str,
) -> Option<&'a HardwareEffectDescriptor> {
    capabilities
        .hardware_effects()?
        .effects
        .iter()
        .find(|effect| effect.id.as_str() == id)
}

fn validate_scope(target: &TargetId, scope: CapabilityScope) -> Result<(), DaemonError> {
    let expected = match target {
        TargetId::Device(_) | TargetId::Group { .. } => CapabilityScope::Device,
        TargetId::Surface { .. } => CapabilityScope::Surface,
        TargetId::Element { .. } => CapabilityScope::Element,
    };
    if scope != expected {
        return Err(unsupported(
            target,
            format!("capability scope is {scope:?}, target requires {expected:?}"),
        ));
    }
    Ok(())
}

fn validate_effect_parameters(
    target: &TargetId,
    descriptor: &HardwareEffectDescriptor,
    effect: &Effect,
) -> Result<(), DaemonError> {
    let (colour_count, period_ms) = match effect {
        Effect::Breathe { period_ms, .. }
        | Effect::Pulse { period_ms, .. }
        | Effect::Strobe { period_ms, .. }
        | Effect::Scanner { period_ms, .. } => (Some(1_usize), *period_ms),
        Effect::Morph { colours, period_ms } => (Some(colours.len()), *period_ms),
        Effect::Spectrum { period_ms } | Effect::Rainbow { period_ms } => (None, *period_ms),
        // A `Hardware` effect carries a collection of `EffectArguments`, not the typed
        // fields inspected here; it is diverted to `validate_hardware_arguments` in
        // `validate_effect` before reaching this typed path.
        Effect::Off | Effect::Static { .. } | Effect::Hardware { .. } => return Ok(()),
    };

    let mut saw_duration = false;
    let mut saw_colour = colour_count.is_none();
    for parameter in &descriptor.parameters {
        match parameter {
            EffectParameter::Colour {
                minimum_colours,
                maximum_colours,
            } => {
                if saw_colour {
                    return Err(unsupported(target, "duplicate colour effect parameter"));
                }
                let Some(count) = colour_count else {
                    return Err(invalid(target, "effect does not accept colour parameters"));
                };
                saw_colour = true;
                if count < usize::from(*minimum_colours) || count > usize::from(*maximum_colours) {
                    return Err(invalid(
                        target,
                        format!(
                            "effect requires {minimum_colours}..={maximum_colours} colours, received {count}"
                        ),
                    ));
                }
            }
            EffectParameter::Duration { milliseconds } => {
                if saw_duration {
                    return Err(unsupported(target, "duplicate duration effect parameter"));
                }
                saw_duration = true;
                if !milliseconds.contains(period_ms) {
                    return Err(invalid(
                        target,
                        format!(
                            "effect duration {period_ms}ms is outside {}..={} step {}",
                            milliseconds.min, milliseconds.max, milliseconds.step
                        ),
                    ));
                }
            }
            EffectParameter::Speed { .. }
            | EffectParameter::Direction { .. }
            | EffectParameter::Brightness { .. }
            | EffectParameter::Choice { .. } => {
                return Err(unsupported(
                    target,
                    "effect descriptor requires a parameter absent from the wire effect",
                ));
            }
        }
    }

    if !saw_colour || !saw_duration {
        return Err(unsupported(
            target,
            "effect descriptor does not bound all wire parameters",
        ));
    }
    Ok(())
}

/// Validates the generic argument bag for an [`Effect::Hardware`]
/// invocation against its advertised descriptor.
///
/// The descriptor is treated as the complete contract: every declared
/// argument must be present and within bounds, and no undeclared
/// arguments may be supplied.
///
/// Built-in effects with statically known parameter types are validated
/// by dedicated typed code instead. This path is reserved for
/// descriptor-driven vendor effects whose parameters are only known at
/// runtime.
fn validate_hardware_arguments(
    target: &TargetId,
    descriptor: &HardwareEffectDescriptor,
    arguments: &EffectArguments,
) -> Result<(), DaemonError> {
    for parameter in &descriptor.parameters {
        match parameter {
            EffectParameter::Colour {
                minimum_colours,
                maximum_colours,
            } => {
                let count = arguments.colours.len();
                if count < usize::from(*minimum_colours) || count > usize::from(*maximum_colours) {
                    return Err(invalid(
                        target,
                        format!(
                            "effect requires {minimum_colours}..={maximum_colours} colours, received {count}"
                        ),
                    ));
                }
            }
            EffectParameter::Speed { range } => {
                let Some(value) = arguments.speed else {
                    return Err(invalid(target, "effect requires a speed argument"));
                };
                if !range.contains(value) {
                    return Err(invalid(
                        target,
                        format!(
                            "speed {value} is outside {}..={} step {}",
                            range.min, range.max, range.step
                        ),
                    ));
                }
            }
            EffectParameter::Direction { values } => {
                let Some(value) = arguments.direction else {
                    return Err(invalid(target, "effect requires a direction argument"));
                };
                if !values.contains(&value) {
                    return Err(invalid(
                        target,
                        "direction is not among the advertised values",
                    ));
                }
            }
            EffectParameter::Duration { milliseconds } => {
                let Some(value) = arguments.duration_ms else {
                    return Err(invalid(target, "effect requires a duration argument"));
                };
                if !milliseconds.contains(value) {
                    return Err(invalid(
                        target,
                        format!(
                            "duration {value}ms is outside {}..={} step {}",
                            milliseconds.min, milliseconds.max, milliseconds.step
                        ),
                    ));
                }
            }
            EffectParameter::Brightness { bits } => {
                let Some(value) = arguments.brightness else {
                    return Err(invalid(target, "effect requires a brightness argument"));
                };
                let Some(maximum) = maximum_for_bits(*bits) else {
                    return Err(unsupported(
                        target,
                        format!("advertised brightness has invalid bit width {bits}"),
                    ));
                };
                if value > maximum {
                    return Err(invalid(
                        target,
                        format!("brightness {value} exceeds {maximum} for {bits} bits"),
                    ));
                }
            }
            EffectParameter::Choice { options } => {
                let Some(value) = &arguments.choice else {
                    return Err(invalid(target, "effect requires a choice argument"));
                };
                if !options.iter().any(|option| &option.id == value) {
                    return Err(invalid(
                        target,
                        format!("choice {value:?} is not an advertised option"),
                    ));
                }
            }
        }
    }

    reject_undeclared_arguments(target, descriptor, arguments)
}

/// Rejects any argument supplied for a parameter the descriptor did not
/// declare, so a caller cannot smuggle in values a device never advertised.
fn reject_undeclared_arguments(
    target: &TargetId,
    descriptor: &HardwareEffectDescriptor,
    arguments: &EffectArguments,
) -> Result<(), DaemonError> {
    let declares =
        |matches: fn(&EffectParameter) -> bool| descriptor.parameters.iter().any(matches);

    if !arguments.colours.is_empty()
        && !declares(|parameter| matches!(parameter, EffectParameter::Colour { .. }))
    {
        return Err(invalid(target, "effect does not accept colour arguments"));
    }
    if arguments.speed.is_some()
        && !declares(|parameter| matches!(parameter, EffectParameter::Speed { .. }))
    {
        return Err(invalid(target, "effect does not accept a speed argument"));
    }
    if arguments.direction.is_some()
        && !declares(|parameter| matches!(parameter, EffectParameter::Direction { .. }))
    {
        return Err(invalid(
            target,
            "effect does not accept a direction argument",
        ));
    }
    if arguments.duration_ms.is_some()
        && !declares(|parameter| matches!(parameter, EffectParameter::Duration { .. }))
    {
        return Err(invalid(
            target,
            "effect does not accept a duration argument",
        ));
    }
    if arguments.brightness.is_some()
        && !declares(|parameter| matches!(parameter, EffectParameter::Brightness { .. }))
    {
        return Err(invalid(
            target,
            "effect does not accept a brightness argument",
        ));
    }
    if arguments.choice.is_some()
        && !declares(|parameter| matches!(parameter, EffectParameter::Choice { .. }))
    {
        return Err(invalid(target, "effect does not accept a choice argument"));
    }
    Ok(())
}

pub(super) fn unsupported(target: &TargetId, reason: impl Into<String>) -> DaemonError {
    DaemonError::UnsupportedCapability {
        target: target.clone(),
        reason: reason.into(),
    }
}

pub(super) fn invalid(target: &TargetId, reason: impl Into<String>) -> DaemonError {
    DaemonError::InvalidArgument {
        target: target.clone(),
        reason: reason.into(),
    }
}

#[cfg(test)]
#[path = "validation_tests.rs"]
pub(crate) mod tests;
