// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Converting Luminate domain values into their D-Bus wire shapes.
//!
//! The tuple aliases here define the published signatures of the
//! `Target2` properties, so their field order is part of the interface
//! contract rather than an implementation detail.

use luminate::capability::{
    ColourCapability, ColourChannel, EffectDirection, EffectParameter, HardwareEffectDescriptor,
};
use luminate::{AppearanceState, EffectiveAppearanceState, FacetValue};

use crate::error::MethodError;
pub(crate) type DbusRgb = (u8, u8, u8);
pub(crate) type DbusEffectChoice = (String, String);
pub(crate) type DbusEffectParameter = (String, u32, u32, u32, Vec<DbusEffectChoice>);
pub(crate) type DbusEffectDescriptor = (String, String, Vec<DbusEffectParameter>);
pub(crate) type DbusStaticColourCapability = (String, Vec<(String, u8)>);

pub(crate) fn static_colour_capability(
    capability: &ColourCapability,
) -> DbusStaticColourCapability {
    match capability {
        ColourCapability::Additive(channels) => (
            "additive".to_owned(),
            channels
                .iter()
                .map(|channel| {
                    (
                        colour_channel_name(channel.channel).to_owned(),
                        channel.bits,
                    )
                })
                .collect(),
        ),
        ColourCapability::Hsv {
            hue_bits,
            saturation_bits,
            value_bits,
        } => (
            "hsv".to_owned(),
            vec![
                ("hue".to_owned(), *hue_bits),
                ("saturation".to_owned(), *saturation_bits),
                ("value".to_owned(), *value_bits),
            ],
        ),
        ColourCapability::Hsl {
            hue_bits,
            saturation_bits,
            lightness_bits,
        } => (
            "hsl".to_owned(),
            vec![
                ("hue".to_owned(), *hue_bits),
                ("saturation".to_owned(), *saturation_bits),
                ("lightness".to_owned(), *lightness_bits),
            ],
        ),
        ColourCapability::Cct { bits } => {
            ("cct".to_owned(), vec![("temperature".to_owned(), *bits)])
        }
        ColourCapability::Monochrome { bits } => (
            "monochrome".to_owned(),
            vec![("intensity".to_owned(), *bits)],
        ),
    }
}

fn colour_channel_name(channel: ColourChannel) -> &'static str {
    match channel {
        ColourChannel::Red => "red",
        ColourChannel::Green => "green",
        ColourChannel::Blue => "blue",
        ColourChannel::White => "white",
        ColourChannel::WarmWhite => "warm-white",
        ColourChannel::CoolWhite => "cool-white",
        ColourChannel::Amber => "amber",
        ColourChannel::Ultraviolet => "ultraviolet",
        ColourChannel::Hue => "hue",
        ColourChannel::Saturation => "saturation",
        ColourChannel::Value => "value",
        ColourChannel::Lightness => "lightness",
        ColourChannel::Temperature => "temperature",
        ColourChannel::Intensity => "intensity",
    }
}

pub(crate) fn invalid_argument(message: impl Into<String>) -> MethodError {
    MethodError::InvalidArgument(message.into())
}

pub(crate) fn parse_direction(value: &str) -> Result<EffectDirection, MethodError> {
    match value {
        "forward" => Ok(EffectDirection::Forward),
        "reverse" => Ok(EffectDirection::Reverse),
        "clockwise" => Ok(EffectDirection::Clockwise),
        "counter-clockwise" => Ok(EffectDirection::CounterClockwise),
        "inward" => Ok(EffectDirection::Inward),
        "outward" => Ok(EffectDirection::Outward),
        "random" => Ok(EffectDirection::Random),
        _ => Err(invalid_argument(format!(
            "unknown hardware-effect direction {value:?}"
        ))),
    }
}

pub(crate) fn direction_id(direction: EffectDirection) -> &'static str {
    match direction {
        EffectDirection::Forward => "forward",
        EffectDirection::Reverse => "reverse",
        EffectDirection::Clockwise => "clockwise",
        EffectDirection::CounterClockwise => "counter-clockwise",
        EffectDirection::Inward => "inward",
        EffectDirection::Outward => "outward",
        EffectDirection::Random => "random",
    }
}

pub(crate) fn effect_descriptor(descriptor: &HardwareEffectDescriptor) -> DbusEffectDescriptor {
    (
        descriptor.id.as_str().to_owned(),
        descriptor.name.clone(),
        descriptor.parameters.iter().map(effect_parameter).collect(),
    )
}

pub(crate) fn effect_parameter(parameter: &EffectParameter) -> DbusEffectParameter {
    match parameter {
        EffectParameter::Colour {
            minimum_colours,
            maximum_colours,
        } => (
            "colour".into(),
            u32::from(*minimum_colours),
            u32::from(*maximum_colours),
            1,
            Vec::new(),
        ),
        EffectParameter::Speed { range } => (
            "speed".into(),
            u32::from(range.min),
            u32::from(range.max),
            u32::from(range.step),
            Vec::new(),
        ),
        EffectParameter::Direction { values } => (
            "direction".into(),
            0,
            0,
            0,
            values
                .iter()
                .map(|direction| {
                    let id = direction_id(*direction).to_owned();
                    (id.clone(), id)
                })
                .collect(),
        ),
        EffectParameter::Duration { milliseconds } => (
            "duration-ms".into(),
            milliseconds.min,
            milliseconds.max,
            milliseconds.step,
            Vec::new(),
        ),
        EffectParameter::Brightness { bits } => (
            "brightness".into(),
            0,
            match bits {
                0 => 0,
                1..=31 => (1_u32 << bits) - 1,
                32..=u8::MAX => u32::MAX,
            },
            1,
            Vec::new(),
        ),
        EffectParameter::Choice { options } => (
            "choice".into(),
            0,
            0,
            0,
            options
                .iter()
                .map(|option| (option.id.clone(), option.name.clone()))
                .collect(),
        ),
    }
}

pub(crate) fn facet_strings(value: &FacetValue) -> (String, String) {
    match value {
        FacetValue::Appearance(AppearanceState::Static(colour)) => {
            ("appearance".into(), format!("{colour:?}"))
        }
        FacetValue::Appearance(AppearanceState::Effect(_)) => {
            ("appearance".into(), "effect".into())
        }
        FacetValue::Appearance(AppearanceState::Mixed) => ("appearance".into(), "mixed".into()),
        FacetValue::Brightness(value) => ("brightness".into(), value.to_string()),
        FacetValue::Emission(value) => ("emission".into(), format!("{value:?}")),
        FacetValue::PhysicalPower(value) => ("physical-power".into(), format!("{value:?}")),
        FacetValue::EffectiveAppearance(EffectiveAppearanceState::Off) => {
            ("effective-appearance".into(), "off".into())
        }
        FacetValue::EffectiveAppearance(EffectiveAppearanceState::Static(colour)) => {
            ("effective-appearance".into(), format!("{colour:?}"))
        }
        FacetValue::EffectiveAppearance(EffectiveAppearanceState::Effect(_)) => {
            ("effective-appearance".into(), "effect".into())
        }
        FacetValue::EffectiveAppearance(EffectiveAppearanceState::Streaming) => {
            ("effective-appearance".into(), "streaming".into())
        }
        FacetValue::EffectiveAppearance(EffectiveAppearanceState::Mixed) => {
            ("effective-appearance".into(), "mixed".into())
        }
        FacetValue::AppearanceSlots(slots) => (
            "appearance-slots".into(),
            format!("{} known, complete={}", slots.values.len(), slots.complete),
        ),
    }
}
