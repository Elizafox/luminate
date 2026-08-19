// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Generic colour inputs shared by the unified C client surface.

use luminate_core::colour::{Colour, ColourChannelValue};

use super::*;

#[cfg(test)]
#[path = "colour_input_tests.rs"]
mod tests;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LuminateColourChannelInput {
    pub channel: LuminateColourChannel,
    pub value: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LuminateColourInput {
    pub encoding: LuminateColourEncoding,
    pub channels: *const LuminateColourChannelInput,
    pub channel_count: usize,
}

fn colour_encoding(value: u32) -> Result<ColourEncoding, LuminateStatus> {
    match value {
        0 => Ok(ColourEncoding::Additive),
        1 => Ok(ColourEncoding::Hsv),
        2 => Ok(ColourEncoding::Hsl),
        3 => Ok(ColourEncoding::Cct),
        4 => Ok(ColourEncoding::Monochrome),
        _ => {
            crate::ffi::set_last_error("invalid LuminateColourEncoding");
            Err(LuminateStatus::InvalidArgument)
        }
    }
}

pub(super) fn colour_channel(value: u32) -> Result<ColourChannel, LuminateStatus> {
    match value {
        0 => Ok(ColourChannel::Red),
        1 => Ok(ColourChannel::Green),
        2 => Ok(ColourChannel::Blue),
        3 => Ok(ColourChannel::White),
        4 => Ok(ColourChannel::WarmWhite),
        5 => Ok(ColourChannel::CoolWhite),
        6 => Ok(ColourChannel::Amber),
        7 => Ok(ColourChannel::Ultraviolet),
        8 => Ok(ColourChannel::Hue),
        9 => Ok(ColourChannel::Saturation),
        10 => Ok(ColourChannel::Value),
        11 => Ok(ColourChannel::Lightness),
        12 => Ok(ColourChannel::Temperature),
        13 => Ok(ColourChannel::Intensity),
        _ => {
            crate::ffi::set_last_error("invalid LuminateColourChannel");
            Err(LuminateStatus::InvalidArgument)
        }
    }
}

pub(super) unsafe fn read_colour(value: &LuminateColourInput) -> Result<Colour, LuminateStatus> {
    let encoding = colour_encoding(value.encoding)?;
    let channels = if value.channel_count == 0 {
        Vec::new()
    } else {
        if value.channels.is_null() {
            crate::ffi::set_last_error("colour channels pointer is null");
            return Err(LuminateStatus::NullPointer);
        }
        // SAFETY: the C contract requires `channels` to reference
        // `channel_count` initialized entries; the null case is rejected above.
        unsafe { std::slice::from_raw_parts(value.channels, value.channel_count) }
            .iter()
            .map(|channel| {
                Ok(ColourChannelValue::new(
                    colour_channel(channel.channel)?,
                    channel.value,
                ))
            })
            .collect::<Result<Vec<_>, LuminateStatus>>()?
    };
    let component = |wanted| {
        channels
            .iter()
            .find(|channel| channel.channel == wanted)
            .map(|channel| channel.value)
            .ok_or_else(|| {
                crate::ffi::set_last_error("colour input is missing a required channel");
                LuminateStatus::InvalidArgument
            })
    };
    match encoding {
        ColourEncoding::Additive => Colour::additive(channels).map_err(|error| {
            crate::ffi::set_last_error(error.to_string());
            LuminateStatus::InvalidArgument
        }),
        ColourEncoding::Hsv => Ok(Colour::hsv(
            component(ColourChannel::Hue)?,
            component(ColourChannel::Saturation)?,
            component(ColourChannel::Value)?,
        )),
        ColourEncoding::Hsl => Ok(Colour::hsl(
            component(ColourChannel::Hue)?,
            component(ColourChannel::Saturation)?,
            component(ColourChannel::Lightness)?,
        )),
        ColourEncoding::Cct => Ok(Colour::cct(component(ColourChannel::Temperature)?)),
        ColourEncoding::Monochrome => Ok(Colour::monochrome(component(ColourChannel::Intensity)?)),
    }
}
