// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Hue's published RGB/CIE xy conversion and gamut clipping.

use luminate_core::rgb::Rgb;
use thiserror::Error;

use crate::api::{Gamut, MirekSchema, Xy};

pub(crate) fn rgb_to_xy(rgb: Rgb, gamut: Option<&Gamut>) -> Result<Xy, ColourError> {
    let red = gamma_correct(f64::from(rgb.r) / 255.0);
    let green = gamma_correct(f64::from(rgb.g) / 255.0);
    let blue = gamma_correct(f64::from(rgb.b) / 255.0);
    let tristimulus_x = red.mul_add(0.4124, green.mul_add(0.3576, blue * 0.1805));
    let tristimulus_y = red.mul_add(0.2126, green.mul_add(0.7152, blue * 0.0722));
    let tristimulus_z = red.mul_add(0.0193, green.mul_add(0.1192, blue * 0.9505));
    let total = tristimulus_x + tristimulus_y + tristimulus_z;
    if !total.is_finite() || total <= f64::EPSILON {
        return Err(ColourError::DarkRgb);
    }
    let point = Xy {
        x: tristimulus_x / total,
        y: tristimulus_y / total,
    };
    Ok(gamut.map_or(point.clone(), |gamut| clip_to_gamut(&point, gamut)))
}

pub(crate) fn kelvin_to_mirek(kelvin: u32, schema: &MirekSchema) -> Result<u16, ColourError> {
    if kelvin == 0 {
        return Err(ColourError::InvalidKelvin);
    }
    let rounded = 1_000_000_u32
        .checked_add(kelvin / 2)
        .ok_or(ColourError::InvalidKelvin)?
        / kelvin;
    let bounded = rounded.clamp(
        u32::from(schema.mirek_minimum),
        u32::from(schema.mirek_maximum),
    );
    u16::try_from(bounded).map_err(|_| ColourError::InvalidKelvin)
}

pub(crate) fn mirek_to_kelvin(mirek: u16) -> Result<u32, ColourError> {
    if mirek == 0 {
        return Err(ColourError::InvalidMirek);
    }
    Ok(1_000_000_u32 / u32::from(mirek))
}

pub(crate) fn xy_to_rgb(xy: &Xy, gamut: Option<&Gamut>) -> Result<Rgb, ColourError> {
    let point = gamut.map_or_else(|| xy.clone(), |gamut| clip_to_gamut(xy, gamut));
    if point.y <= f64::EPSILON {
        return Err(ColourError::InvalidXy);
    }
    let tristimulus_y = 1.0;
    let tristimulus_x = tristimulus_y / point.y * point.x;
    let tristimulus_z = tristimulus_y / point.y * (1.0 - point.x - point.y);
    let red = tristimulus_x.mul_add(
        1.656_492,
        tristimulus_y.mul_add(-0.354_851, tristimulus_z * -0.255_038),
    );
    let green = tristimulus_x.mul_add(
        -0.707_196,
        tristimulus_y.mul_add(1.655_397, tristimulus_z * 0.036_152),
    );
    let blue = tristimulus_x.mul_add(
        0.051_713,
        tristimulus_y.mul_add(-0.121_364, tristimulus_z * 1.011_530),
    );
    let maximum = red.max(green).max(blue).max(1.0);
    Ok(Rgb::new(
        rgb_channel(red / maximum),
        rgb_channel(green / maximum),
        rgb_channel(blue / maximum),
    ))
}

fn rgb_channel(linear: f64) -> u8 {
    let corrected = if linear <= 0.003_130_8 {
        12.92 * linear
    } else {
        1.055 * linear.max(0.0).powf(1.0 / 2.4) - 0.055
    };
    let scaled = (corrected * 255.0).round().clamp(0.0, 255.0);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the rounded RGB channel is clamped to the complete u8 range"
    )]
    let channel = scaled as u8;
    channel
}

fn gamma_correct(value: f64) -> f64 {
    if value > 0.04045 {
        ((value + 0.055) / 1.055).powf(2.4)
    } else {
        value / 12.92
    }
}

fn clip_to_gamut(point: &Xy, gamut: &Gamut) -> Xy {
    if point_in_triangle(point, &gamut.red, &gamut.green, &gamut.blue) {
        return point.clone();
    }
    let candidates = [
        closest_on_segment(point, &gamut.red, &gamut.green),
        closest_on_segment(point, &gamut.green, &gamut.blue),
        closest_on_segment(point, &gamut.blue, &gamut.red),
    ];
    candidates
        .into_iter()
        .min_by(|left, right| {
            squared_distance(point, left).total_cmp(&squared_distance(point, right))
        })
        .unwrap_or_else(|| gamut.red.clone())
}

fn point_in_triangle(point: &Xy, a: &Xy, b: &Xy, c: &Xy) -> bool {
    let denominator = (b.y - c.y).mul_add(a.x - c.x, (c.x - b.x) * (a.y - c.y));
    if denominator.abs() <= f64::EPSILON {
        return false;
    }
    let first = (b.y - c.y).mul_add(point.x - c.x, (c.x - b.x) * (point.y - c.y)) / denominator;
    let second = (c.y - a.y).mul_add(point.x - c.x, (a.x - c.x) * (point.y - c.y)) / denominator;
    let third = 1.0 - first - second;
    first >= 0.0 && second >= 0.0 && third >= 0.0
}

fn closest_on_segment(point: &Xy, start: &Xy, end: &Xy) -> Xy {
    let delta_x = end.x - start.x;
    let delta_y = end.y - start.y;
    let length_squared = delta_x.mul_add(delta_x, delta_y * delta_y);
    if length_squared <= f64::EPSILON {
        return start.clone();
    }
    let projection = ((point.x - start.x).mul_add(delta_x, (point.y - start.y) * delta_y)
        / length_squared)
        .clamp(0.0, 1.0);
    Xy {
        x: delta_x.mul_add(projection, start.x),
        y: delta_y.mul_add(projection, start.y),
    }
}

fn squared_distance(left: &Xy, right: &Xy) -> f64 {
    let delta_x = left.x - right.x;
    let delta_y = left.y - right.y;
    delta_x.mul_add(delta_x, delta_y * delta_y)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum ColourError {
    #[error("black RGB has no defined CIE xy chromaticity")]
    DarkRgb,
    #[error("colour temperature must be greater than zero kelvin")]
    InvalidKelvin,
    #[error("zero mirek has no defined colour temperature")]
    InvalidMirek,
    #[error("CIE xy with zero y cannot be converted to RGB")]
    InvalidXy,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gamut_c() -> Gamut {
        Gamut {
            red: Xy {
                x: 0.6915,
                y: 0.3083,
            },
            green: Xy { x: 0.17, y: 0.7 },
            blue: Xy {
                x: 0.1532,
                y: 0.0475,
            },
        }
    }

    #[test]
    fn converts_rgb_with_published_formula_and_clips_to_gamut() {
        let unclipped = rgb_to_xy(Rgb::new(255, 0, 0), None).expect("red xy");
        assert!((unclipped.x - 0.6400).abs() < 0.0002);
        assert!((unclipped.y - 0.3300).abs() < 0.0002);

        let clipped = rgb_to_xy(Rgb::new(255, 0, 0), Some(&gamut_c())).expect("clipped red");
        assert!(point_in_triangle(
            &clipped,
            &gamut_c().red,
            &gamut_c().green,
            &gamut_c().blue
        ));
        assert!(squared_distance(&clipped, &unclipped) < f64::EPSILON);

        let outside = Xy { x: 1.0, y: 1.0 };
        let clipped_outside = clip_to_gamut(&outside, &gamut_c());
        assert!(point_in_triangle(
            &clipped_outside,
            &gamut_c().red,
            &gamut_c().green,
            &gamut_c().blue
        ));
    }

    #[test]
    fn preserves_points_inside_gamut_and_rejects_black() {
        let white = rgb_to_xy(Rgb::new(255, 255, 255), Some(&gamut_c())).expect("white xy");
        assert!((white.x - 0.3127).abs() < 0.001);
        assert!((white.y - 0.3290).abs() < 0.001);
        assert!(matches!(
            rgb_to_xy(Rgb::new(0, 0, 0), None),
            Err(ColourError::DarkRgb)
        ));
    }

    #[test]
    fn converts_and_saturates_kelvin_at_reported_bounds() {
        let schema = MirekSchema {
            mirek_minimum: 153,
            mirek_maximum: 500,
        };
        assert_eq!(kelvin_to_mirek(4_000, &schema), Ok(250));
        assert_eq!(kelvin_to_mirek(20_000, &schema), Ok(153));
        assert_eq!(kelvin_to_mirek(1_000, &schema), Ok(500));
        assert_eq!(kelvin_to_mirek(0, &schema), Err(ColourError::InvalidKelvin));
    }
}
