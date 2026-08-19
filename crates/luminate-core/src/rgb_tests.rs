// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn kelvin_to_rgb_daylight_is_roughly_neutral() {
    let rgb = kelvin_to_rgb(6600);
    assert!(rgb.r.abs_diff(255) <= 2);
    assert!(rgb.g.abs_diff(255) <= 2);
    assert!(rgb.b.abs_diff(255) <= 2);
}

#[test]
fn kelvin_to_rgb_low_temperature_is_warm() {
    let rgb = kelvin_to_rgb(2000);
    assert!(rgb.r > rgb.b, "warm light should be red-heavy: {rgb:?}");
    assert_eq!(rgb.r, 255);
}

#[test]
fn kelvin_to_rgb_high_temperature_is_cool() {
    let rgb = kelvin_to_rgb(15_000);
    assert!(rgb.b > rgb.r, "cool light should be blue-heavy: {rgb:?}");
    assert_eq!(rgb.b, 255);
}

#[test]
fn kelvin_to_rgb_clamps_out_of_range_input() {
    assert_eq!(kelvin_to_rgb(100), kelvin_to_rgb(1000));
    assert_eq!(kelvin_to_rgb(1_000_000), kelvin_to_rgb(40_000));
}
