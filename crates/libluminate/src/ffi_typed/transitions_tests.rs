// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use super::*;

#[test]
fn c_options_map_easing_colour_and_hue_choices() {
    let parsed = options(LuminateTransitionOptions {
        duration_ms: 1_000,
        step_interval_ms: 20,
        function: 3,
        colour_interpolation: 0,
        hue_direction: 2,
    })
    .unwrap();
    assert_eq!(parsed.function, TransitionFunction::EaseInOut);
    assert_eq!(
        parsed.colour_interpolation,
        TransitionColourInterpolation::Encoded {
            hue_direction: HueDirection::Decreasing,
        }
    );

    let oklab = options(LuminateTransitionOptions {
        duration_ms: 1_000,
        step_interval_ms: 0,
        function: 0,
        colour_interpolation: 1,
        hue_direction: 0,
    })
    .unwrap();
    assert_eq!(
        oklab.colour_interpolation,
        TransitionColourInterpolation::Oklab
    );
}

#[test]
fn c_options_reject_direction_for_oklab() {
    assert_eq!(
        options(LuminateTransitionOptions {
            duration_ms: 1_000,
            step_interval_ms: 0,
            function: 0,
            colour_interpolation: 1,
            hue_direction: 1,
        }),
        Err(LuminateStatus::InvalidArgument)
    );
}
