// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Parsing and validation for the `a{sv}` dictionary accepted by
//! `Target2.SetEffect`.
//!
//! Each effect accepts a specific set of fields. Inapplicable fields are
//! rejected instead of silently ignored, so callers receive a useful error for
//! mistakes such as supplying a period for an effect that does not use one.

use luminate::capability::HardwareEffectId;
use luminate::effect::EffectArguments;
use std::collections::HashMap;

use luminate::capability::ColourChannel;
use luminate::colour::ColourChannelValue;
use luminate::{Colour, Effect, Rgb};
use zbus::zvariant::{DeserializeDict, Type};

use crate::convert::{DbusRgb, invalid_argument, parse_direction};
use crate::error::MethodError;

pub(crate) const MAX_EFFECT_COLOURS: usize = u8::MAX as usize;

#[derive(Debug, Default, DeserializeDict, Type)]
#[zvariant(signature = "a{sv}", rename_all = "PascalCase", deny_unknown_fields)]
pub(crate) struct EffectRequest {
    pub(crate) kind: String,
    pub(crate) static_colour: Option<StaticColourRequest>,
    pub(crate) colours: Option<Vec<DbusRgb>>,
    pub(crate) period_ms: Option<u32>,
    pub(crate) hardware_id: Option<String>,
    pub(crate) speed: Option<u16>,
    pub(crate) direction: Option<String>,
    pub(crate) duration_ms: Option<u32>,
    pub(crate) brightness: Option<u32>,
    pub(crate) choice: Option<String>,
}

#[derive(Debug, Default, DeserializeDict, Type)]
#[zvariant(signature = "a{sv}", rename_all = "PascalCase", deny_unknown_fields)]
pub(crate) struct StaticColourRequest {
    pub(crate) model: String,
    pub(crate) channels: HashMap<String, u32>,
}

impl EffectRequest {
    pub(crate) fn into_effect(self) -> Result<Effect, MethodError> {
        if self.colours().len() > MAX_EFFECT_COLOURS {
            return Err(invalid_argument(format!(
                "effect accepts at most {MAX_EFFECT_COLOURS} colours"
            )));
        }
        if self.kind != "static" && self.static_colour.is_some() {
            return Err(invalid_argument(format!(
                "{} does not accept StaticColour",
                self.kind
            )));
        }

        match self.kind.as_str() {
            "off" => {
                self.require_no_colours_or_period()?;
                self.require_no_hardware_arguments()?;
                Ok(Effect::Off)
            }
            "static" => {
                if !self.colours().is_empty() {
                    return Err(invalid_argument(
                        "Static uses StaticColour rather than the RGB Colours field",
                    ));
                }
                let colour = self
                    .static_colour
                    .as_ref()
                    .ok_or_else(|| invalid_argument("Static requires StaticColour"))?
                    .to_colour()?;
                self.require_no_period()?;
                self.require_no_hardware_arguments()?;
                Ok(Effect::Static { colour })
            }
            "breathe" | "pulse" | "strobe" | "scanner" => {
                let colour = self.one_colour()?;
                let period_ms = self.required_period()?;
                self.require_no_hardware_arguments()?;
                Ok(match self.kind.as_str() {
                    "breathe" => Effect::Breathe { colour, period_ms },
                    "pulse" => Effect::Pulse { colour, period_ms },
                    "strobe" => Effect::Strobe { colour, period_ms },
                    "scanner" => Effect::Scanner { colour, period_ms },
                    other => {
                        return Err(invalid_argument(format!("unknown effect kind {other:?}")));
                    }
                })
            }
            "morph" => {
                if self.colours().is_empty() {
                    return Err(invalid_argument("Morph requires at least one colour"));
                }
                let colours = self.rgb_colours();
                let period_ms = self.required_period()?;
                self.require_no_hardware_arguments()?;
                Ok(Effect::Morph { colours, period_ms })
            }
            "spectrum" | "rainbow" => {
                if !self.colours().is_empty() {
                    return Err(invalid_argument(format!(
                        "{} does not accept colours",
                        self.kind
                    )));
                }
                let period_ms = self.required_period()?;
                self.require_no_hardware_arguments()?;
                Ok(if self.kind == "spectrum" {
                    Effect::Spectrum { period_ms }
                } else {
                    Effect::Rainbow { period_ms }
                })
            }
            "hardware" => {
                self.require_no_period()?;
                let id = self
                    .hardware_id
                    .clone()
                    .ok_or_else(|| invalid_argument("Hardware effects require HardwareId"))?;
                let direction = self.direction.as_deref().map(parse_direction).transpose()?;
                Ok(Effect::Hardware {
                    id: HardwareEffectId::new(id),
                    arguments: EffectArguments {
                        colours: self.rgb_colours(),
                        speed: self.speed,
                        direction,
                        duration_ms: self.duration_ms,
                        brightness: self.brightness,
                        choice: self.choice,
                    },
                })
            }
            other => Err(invalid_argument(format!("unknown effect kind {other:?}"))),
        }
    }

    fn rgb_colours(&self) -> Vec<Rgb> {
        self.colours()
            .iter()
            .map(|&(red, green, blue)| Rgb::new(red, green, blue))
            .collect()
    }

    fn one_colour(&self) -> Result<Rgb, MethodError> {
        let [colour] = self.colours() else {
            return Err(invalid_argument(format!(
                "{} requires exactly one colour",
                self.kind
            )));
        };
        Ok(Rgb::new(colour.0, colour.1, colour.2))
    }

    fn required_period(&self) -> Result<u32, MethodError> {
        self.period_ms
            .ok_or_else(|| invalid_argument(format!("{} requires PeriodMs", self.kind)))
    }

    fn require_no_period(&self) -> Result<(), MethodError> {
        if self.period_ms.is_some() {
            return Err(invalid_argument(format!(
                "{} does not accept PeriodMs",
                self.kind
            )));
        }
        Ok(())
    }

    fn require_no_colours_or_period(&self) -> Result<(), MethodError> {
        if !self.colours().is_empty() {
            return Err(invalid_argument(format!(
                "{} does not accept colours",
                self.kind
            )));
        }
        self.require_no_period()
    }

    fn colours(&self) -> &[DbusRgb] {
        self.colours.as_deref().unwrap_or_default()
    }

    fn require_no_hardware_arguments(&self) -> Result<(), MethodError> {
        if self.hardware_id.is_some()
            || self.speed.is_some()
            || self.direction.is_some()
            || self.duration_ms.is_some()
            || self.brightness.is_some()
            || self.choice.is_some()
        {
            return Err(invalid_argument(format!(
                "{} does not accept hardware-effect arguments",
                self.kind
            )));
        }
        Ok(())
    }
}

impl StaticColourRequest {
    pub(crate) fn to_colour(&self) -> Result<Colour, MethodError> {
        let required = |name: &str| {
            self.channels.get(name).copied().ok_or_else(|| {
                invalid_argument(format!("{} requires channel {name:?}", self.model))
            })
        };
        let reject_unknown = |allowed: &[&str]| {
            if let Some(name) = self
                .channels
                .keys()
                .find(|name| !allowed.contains(&name.as_str()))
            {
                return Err(invalid_argument(format!(
                    "{} does not accept channel {name:?}",
                    self.model
                )));
            }
            Ok(())
        };

        match self.model.as_str() {
            "additive" => {
                if self.channels.is_empty() {
                    return Err(invalid_argument(
                        "additive colour requires at least one channel",
                    ));
                }
                let channels = self
                    .channels
                    .iter()
                    .map(|(name, value)| {
                        Ok(ColourChannelValue::new(
                            parse_additive_channel(name)?,
                            *value,
                        ))
                    })
                    .collect::<Result<Vec<_>, MethodError>>()?;
                Colour::additive(channels).map_err(|error| invalid_argument(error.to_string()))
            }
            "hsv" => {
                reject_unknown(&["hue", "saturation", "value"])?;
                Ok(Colour::hsv(
                    required("hue")?,
                    required("saturation")?,
                    required("value")?,
                ))
            }
            "hsl" => {
                reject_unknown(&["hue", "saturation", "lightness"])?;
                Ok(Colour::hsl(
                    required("hue")?,
                    required("saturation")?,
                    required("lightness")?,
                ))
            }
            "cct" => {
                reject_unknown(&["temperature"])?;
                Ok(Colour::cct(required("temperature")?))
            }
            "monochrome" => {
                reject_unknown(&["intensity"])?;
                Ok(Colour::monochrome(required("intensity")?))
            }
            model => Err(invalid_argument(format!(
                "unknown Static colour model {model:?}"
            ))),
        }
    }
}

fn parse_additive_channel(name: &str) -> Result<ColourChannel, MethodError> {
    match name {
        "red" => Ok(ColourChannel::Red),
        "green" => Ok(ColourChannel::Green),
        "blue" => Ok(ColourChannel::Blue),
        "white" => Ok(ColourChannel::White),
        "warm-white" => Ok(ColourChannel::WarmWhite),
        "cool-white" => Ok(ColourChannel::CoolWhite),
        "amber" => Ok(ColourChannel::Amber),
        "ultraviolet" => Ok(ColourChannel::Ultraviolet),
        _ => Err(invalid_argument(format!(
            "unknown additive channel {name:?}"
        ))),
    }
}

#[cfg(test)]
#[path = "effect_request_tests.rs"]
mod tests;
