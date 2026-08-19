// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Eight-bit red, green, and blue colour components.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
/// An 8-bit additive RGB triplet.
pub struct Rgb {
    /// Red channel, from 0 to 255.
    pub r: u8,

    /// Green channel, from 0 to 255.
    pub g: u8,

    /// Blue channel, from 0 to 255.
    pub b: u8,
}

impl Rgb {
    /// All channels off.
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0 };

    /// All channels at full intensity.
    pub const WHITE: Self = Self {
        r: 255,
        g: 255,
        b: 255,
    };

    /// Creates a colour from red, green, and blue channel values.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// Approximates the RGB appearance of a correlated colour temperature, for
/// devices with no native white/kelvin channel.
///
/// Uses Tanner Helland's polynomial fit to the Planckian locus
/// (<https://tannerhelland.com/2012/09/18/convert-temperature-rgb-algorithm.html>),
/// clamped to its documented working range of 1000-40000 K. This is a
/// visual approximation, not a colorimetric conversion.
#[must_use]
pub fn kelvin_to_rgb(kelvin: u32) -> Rgb {
    let k = f64::from(kelvin.clamp(1000, 40_000)) / 100.0;

    let r = if k <= 66.0 {
        255.0
    } else {
        329.698_727_446 * (k - 60.0).powf(-0.133_204_759_2)
    };

    let g = if k <= 66.0 {
        99.470_802_586_1 * k.ln() - 161.119_568_166_1
    } else {
        288.122_169_528_3 * (k - 60.0).powf(-0.075_514_849_2)
    };

    let b = if k >= 66.0 {
        255.0
    } else if k <= 19.0 {
        0.0
    } else {
        138.517_731_223_1 * (k - 10.0).ln() - 305.044_792_730_7
    };

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is rounded and clamped to the complete u8 range above"
    )]
    let clamp = |value: f64| value.round().clamp(0.0, 255.0) as u8;
    Rgb::new(clamp(r), clamp(g), clamp(b))
}

#[cfg(test)]
#[path = "rgb_tests.rs"]
mod tests;
