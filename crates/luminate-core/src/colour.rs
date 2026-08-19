// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Device-independent, structurally valid colour values.

use std::collections::HashSet;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::capability::{ColourChannel, ColourEncoding};
use crate::rgb::Rgb;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One named additive-emitter channel in a colour value.
pub struct ColourChannelValue {
    /// Emitter represented by this value.
    pub channel: ColourChannel,

    /// Observed or requested value.
    pub value: u32,
}

impl ColourChannelValue {
    /// Pairs an additive emitter with its requested or observed value.
    #[must_use]
    pub const fn new(channel: ColourChannel, value: u32) -> Self {
        Self { channel, value }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Why an additive colour could not be constructed.
pub enum ColourError {
    /// An additive colour must contain at least one emitter.
    #[error("additive colour has no channels")]
    EmptyAdditive,

    /// An emitter appeared more than once.
    #[error("additive colour repeats the {0:?} channel")]
    DuplicateAdditiveChannel(ColourChannel),

    /// A non-emitter channel was supplied to the additive model.
    #[error("{0:?} is not an additive emitter channel")]
    NonAdditiveChannel(ColourChannel),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Why a colour could not be represented as an 8-bit RGB triplet.
pub enum Rgb8Error {
    /// The colour uses a model other than additive emitters.
    #[error("{0:?} is not an additive colour model")]
    NonAdditive(ColourEncoding),

    /// One of the three RGB emitters was absent.
    #[error("additive colour is missing the {0:?} channel")]
    MissingChannel(ColourChannel),

    /// One of the three RGB emitter values did not fit in eight bits.
    #[error("additive colour's {channel:?} channel value {value} exceeds 8 bits")]
    ChannelOutOfRange {
        /// Emitter whose value was out of range.
        channel: ColourChannel,
        /// Value that could not be represented.
        value: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// A device-independent static colour.
pub enum Colour {
    /// Independent additive emitter channels, such as RGB, RGBW, or RGBWW.
    Additive(Vec<ColourChannelValue>),

    /// Hue, saturation, and value.
    Hsv {
        /// Hue component.
        hue: u32,
        /// Saturation component.
        saturation: u32,
        /// Value component.
        value: u32,
    },

    /// Hue, saturation, and lightness.
    Hsl {
        /// Hue component.
        hue: u32,
        /// Saturation component.
        saturation: u32,
        /// Lightness component.
        lightness: u32,
    },

    /// Correlated colour temperature.
    Cct {
        /// Temperature in kelvin.
        kelvin: u32,
    },

    /// A single intensity channel.
    Monochrome {
        /// Requested intensity.
        intensity: u32,
    },
}

impl Colour {
    /// Returns this value's colour model.
    #[must_use]
    pub const fn encoding(&self) -> ColourEncoding {
        match self {
            Self::Additive(_) => ColourEncoding::Additive,
            Self::Hsv { .. } => ColourEncoding::Hsv,
            Self::Hsl { .. } => ColourEncoding::Hsl,
            Self::Cct { .. } => ColourEncoding::Cct,
            Self::Monochrome { .. } => ColourEncoding::Monochrome,
        }
    }

    /// Converts an 8-bit RGB triplet into an additive colour value.
    #[must_use]
    pub fn rgb(rgb: Rgb) -> Self {
        Self::Additive(vec![
            ColourChannelValue::new(ColourChannel::Red, rgb.r.into()),
            ColourChannelValue::new(ColourChannel::Green, rgb.g.into()),
            ColourChannelValue::new(ColourChannel::Blue, rgb.b.into()),
        ])
    }

    /// Creates a validated additive colour.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection is empty, repeats an emitter, or
    /// contains a channel belonging to another colour model.
    pub fn additive(channels: Vec<ColourChannelValue>) -> Result<Self, ColourError> {
        validate_additive(&channels)?;
        Ok(Self::Additive(channels))
    }

    /// Creates an HSV colour.
    #[must_use]
    pub const fn hsv(hue: u32, saturation: u32, value: u32) -> Self {
        Self::Hsv {
            hue,
            saturation,
            value,
        }
    }

    /// Creates an HSL colour.
    #[must_use]
    pub const fn hsl(hue: u32, saturation: u32, lightness: u32) -> Self {
        Self::Hsl {
            hue,
            saturation,
            lightness,
        }
    }

    /// Creates a correlated-colour-temperature colour.
    #[must_use]
    pub const fn cct(kelvin: u32) -> Self {
        Self::Cct { kelvin }
    }

    /// Creates a monochrome colour.
    #[must_use]
    pub const fn monochrome(intensity: u32) -> Self {
        Self::Monochrome { intensity }
    }

    /// Returns whether every output channel is zero.
    #[must_use]
    pub fn is_dark(&self) -> bool {
        match self {
            Self::Additive(channels) => channels.iter().all(|channel| channel.value == 0),
            Self::Hsv { value, .. } => *value == 0,
            Self::Hsl { lightness, .. } => *lightness == 0,
            Self::Cct { .. } => false,
            Self::Monochrome { intensity } => *intensity == 0,
        }
    }

    /// Returns one component by its canonical channel name.
    #[must_use]
    pub fn channel(&self, wanted: ColourChannel) -> Option<u32> {
        match self {
            Self::Additive(channels) => channels
                .iter()
                .find(|channel| channel.channel == wanted)
                .map(|channel| channel.value),
            Self::Hsv {
                hue,
                saturation,
                value,
            } => match wanted {
                ColourChannel::Hue => Some(*hue),
                ColourChannel::Saturation => Some(*saturation),
                ColourChannel::Value => Some(*value),
                ColourChannel::Red
                | ColourChannel::Green
                | ColourChannel::Blue
                | ColourChannel::White
                | ColourChannel::WarmWhite
                | ColourChannel::CoolWhite
                | ColourChannel::Amber
                | ColourChannel::Ultraviolet
                | ColourChannel::Lightness
                | ColourChannel::Temperature
                | ColourChannel::Intensity => None,
            },
            Self::Hsl {
                hue,
                saturation,
                lightness,
            } => match wanted {
                ColourChannel::Hue => Some(*hue),
                ColourChannel::Saturation => Some(*saturation),
                ColourChannel::Lightness => Some(*lightness),
                ColourChannel::Red
                | ColourChannel::Green
                | ColourChannel::Blue
                | ColourChannel::White
                | ColourChannel::WarmWhite
                | ColourChannel::CoolWhite
                | ColourChannel::Amber
                | ColourChannel::Ultraviolet
                | ColourChannel::Value
                | ColourChannel::Temperature
                | ColourChannel::Intensity => None,
            },
            Self::Cct { kelvin } if wanted == ColourChannel::Temperature => Some(*kelvin),
            Self::Monochrome { intensity } if wanted == ColourChannel::Intensity => {
                Some(*intensity)
            }
            Self::Cct { .. } | Self::Monochrome { .. } => None,
        }
    }

    /// Extracts an 8-bit additive RGB triplet when present and in range.
    #[must_use]
    pub fn as_rgb(&self) -> Option<Rgb> {
        self.try_as_rgb().ok()
    }

    /// Extracts an 8-bit additive RGB triplet with a precise failure reason.
    ///
    /// Additive channels other than red, green, and blue are ignored. This
    /// permits RGBW and other additive values to be lowered for RGB-only
    /// hardware without treating the additional emitters as RGB data.
    ///
    /// # Errors
    ///
    /// Returns an error when this is not an additive colour, an RGB emitter is
    /// absent, or an RGB emitter's value exceeds eight bits.
    pub fn try_as_rgb(&self) -> Result<Rgb, Rgb8Error> {
        let Self::Additive(_) = self else {
            return Err(Rgb8Error::NonAdditive(self.encoding()));
        };

        let channel = |wanted| {
            let value = self
                .channel(wanted)
                .ok_or(Rgb8Error::MissingChannel(wanted))?;
            u8::try_from(value).map_err(|_| Rgb8Error::ChannelOutOfRange {
                channel: wanted,
                value,
            })
        };

        Ok(Rgb::new(
            channel(ColourChannel::Red)?,
            channel(ColourChannel::Green)?,
            channel(ColourChannel::Blue)?,
        ))
    }
}

impl<'de> Deserialize<'de> for Colour {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum Repr {
            Additive(Vec<ColourChannelValue>),
            Hsv {
                hue: u32,
                saturation: u32,
                value: u32,
            },
            Hsl {
                hue: u32,
                saturation: u32,
                lightness: u32,
            },
            Cct {
                kelvin: u32,
            },
            Monochrome {
                intensity: u32,
            },
        }

        match Repr::deserialize(deserializer)? {
            Repr::Additive(channels) => Self::additive(channels).map_err(D::Error::custom),
            Repr::Hsv {
                hue,
                saturation,
                value,
            } => Ok(Self::hsv(hue, saturation, value)),
            Repr::Hsl {
                hue,
                saturation,
                lightness,
            } => Ok(Self::hsl(hue, saturation, lightness)),
            Repr::Cct { kelvin } => Ok(Self::cct(kelvin)),
            Repr::Monochrome { intensity } => Ok(Self::monochrome(intensity)),
        }
    }
}

fn validate_additive(channels: &[ColourChannelValue]) -> Result<(), ColourError> {
    if channels.is_empty() {
        return Err(ColourError::EmptyAdditive);
    }

    let mut seen = HashSet::new();
    for value in channels {
        if !value.channel.is_additive() {
            return Err(ColourError::NonAdditiveChannel(value.channel));
        }
        if !seen.insert(value.channel) {
            return Err(ColourError::DuplicateAdditiveChannel(value.channel));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "colour_tests.rs"]
mod tests;
