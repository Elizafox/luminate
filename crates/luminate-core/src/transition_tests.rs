// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;
use crate::capability::HardwareEffectId;

#[test]
fn options_validate_timing_and_supply_default_interval() {
    let options = TransitionOptions::new(Duration::from_secs(1), None).unwrap();
    assert_eq!(options.duration(), Duration::from_secs(1));
    assert_eq!(options.step_interval(), DEFAULT_STEP_INTERVAL);
    assert_eq!(
        TransitionOptions::new(Duration::ZERO, None),
        Err(TransitionValidationError::ZeroDuration)
    );
    assert_eq!(
        TransitionOptions::new(Duration::from_secs(1), Some(Duration::from_millis(9))),
        Err(TransitionValidationError::StepIntervalTooShort)
    );
}

#[test]
fn rounding_and_hue_take_expected_paths() {
    assert_eq!(interpolate_u32(0, 5, 1, 2).unwrap(), 3);
    assert_eq!(interpolate_u32(5, 0, 1, 2).unwrap(), 2);
    let start = Colour::hsv(350, 10, 20);
    let end = Colour::hsv(10, 30, 40);
    assert_eq!(
        interpolate_colour(
            &start,
            &end,
            1,
            2,
            359,
            TransitionColourInterpolation::default(),
        )
        .unwrap(),
        Colour::hsv(0, 20, 30)
    );
    assert_eq!(
        interpolate_colour(
            &Colour::hsv(10, 10, 20),
            &Colour::hsv(350, 30, 40),
            1,
            2,
            359,
            TransitionColourInterpolation::Encoded {
                hue_direction: HueDirection::Increasing,
            },
        )
        .unwrap(),
        Colour::hsv(180, 20, 30)
    );
    assert_eq!(
        interpolate_colour(
            &Colour::hsv(10, 10, 20),
            &Colour::hsv(350, 30, 40),
            1,
            2,
            359,
            TransitionColourInterpolation::Encoded {
                hue_direction: HueDirection::Decreasing,
            },
        )
        .unwrap(),
        Colour::hsv(0, 20, 30)
    );
}

#[test]
fn cubic_easing_retains_endpoints_and_expected_quarter_progress() {
    for function in [
        TransitionFunction::Linear,
        TransitionFunction::EaseIn,
        TransitionFunction::EaseOut,
        TransitionFunction::EaseInOut,
    ] {
        assert_eq!(ease_progress(function, 0, 4).unwrap(), (0, 1_000_000_000));
        assert_eq!(
            ease_progress(function, 4, 4).unwrap(),
            (1_000_000_000, 1_000_000_000)
        );
    }
    assert_eq!(
        ease_progress(TransitionFunction::Linear, 1, 4).unwrap().0,
        250_000_000
    );
    assert_eq!(
        ease_progress(TransitionFunction::EaseIn, 1, 4).unwrap().0,
        15_625_000
    );
    assert_eq!(
        ease_progress(TransitionFunction::EaseOut, 1, 4).unwrap().0,
        578_125_000
    );
    assert_eq!(
        ease_progress(TransitionFunction::EaseInOut, 1, 4)
            .unwrap()
            .0,
        156_250_000
    );
}

#[test]
fn oklab_rgb_interpolation_is_opt_in_and_preserves_exact_endpoints() {
    let start = Rgb::new(255, 0, 0);
    let end = Rgb::new(0, 0, 255);
    let encoded =
        interpolate_rgb(start, end, 1, 2, TransitionColourInterpolation::default()).unwrap();
    let perceptual =
        interpolate_rgb(start, end, 1, 2, TransitionColourInterpolation::Oklab).unwrap();
    assert_eq!(encoded, Rgb::new(127, 0, 128));
    assert_ne!(perceptual, encoded);
    assert_eq!(
        interpolate_rgb(start, end, 0, 2, TransitionColourInterpolation::Oklab,).unwrap(),
        start
    );
    assert_eq!(
        interpolate_rgb(start, end, 2, 2, TransitionColourInterpolation::Oklab,).unwrap(),
        end
    );
}

#[test]
fn incompatible_discrete_structure_is_rejected() {
    let start = Effect::Hardware {
        id: HardwareEffectId::new("demo"),
        arguments: EffectArguments {
            choice: Some("a".to_owned()),
            ..EffectArguments::default()
        },
    };
    let end = Effect::Hardware {
        id: HardwareEffectId::new("demo"),
        arguments: EffectArguments {
            choice: Some("b".to_owned()),
            ..EffectArguments::default()
        },
    };
    assert_eq!(
        interpolate_effect(
            &start,
            &end,
            1,
            2,
            255,
            TransitionColourInterpolation::default(),
        ),
        Err(TransitionInterpolationError::HardwareStructure)
    );
}

#[test]
fn advertised_ranges_snap_to_valid_steps() {
    let range = DiscreteRange::new(10, 30, 5);
    assert_eq!(snap_to_discrete_range_u32(12, range).unwrap(), 10);
    assert_eq!(snap_to_discrete_range_u32(13, range).unwrap(), 15);
    assert_eq!(snap_to_discrete_range_u32(99, range).unwrap(), 30);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "This table-driven test covers the public transition shape matrix."
)]
fn interpolation_covers_public_effect_and_colour_shapes() {
    let encoded = TransitionColourInterpolation::default();
    let rgb_start = Colour::rgb(Rgb::new(1, 2, 3));
    let rgb_end = Colour::rgb(Rgb::new(10, 20, 30));
    let additive_start = Colour::additive(vec![
        ColourChannelValue::new(ColourChannel::Red, 1),
        ColourChannelValue::new(ColourChannel::Green, 2),
        ColourChannelValue::new(ColourChannel::Blue, 3),
    ])
    .expect("valid additive colour");
    let additive_end = Colour::additive(vec![
        ColourChannelValue::new(ColourChannel::Red, 10),
        ColourChannelValue::new(ColourChannel::Green, 20),
        ColourChannelValue::new(ColourChannel::Blue, 30),
    ])
    .expect("valid additive colour");

    for (start, end) in [
        (rgb_start.clone(), rgb_end.clone()),
        (Colour::hsl(1, 2, 3), Colour::hsl(10, 20, 30)),
        (Colour::cct(2_700), Colour::cct(6_500)),
        (Colour::monochrome(1), Colour::monochrome(200)),
        (additive_start.clone(), additive_end.clone()),
    ] {
        assert!(interpolate_colour(&start, &end, 1, 2, 359, encoded).is_ok());
    }
    assert!(
        interpolate_colour(
            &additive_start,
            &additive_end,
            1,
            2,
            359,
            TransitionColourInterpolation::Oklab,
        )
        .is_ok()
    );
    assert_eq!(
        interpolate_colour(&Colour::hsv(1, 2, 3), &additive_end, 1, 2, 359, encoded,),
        Err(TransitionInterpolationError::ColourStructure)
    );

    let effects = [
        (
            Effect::Breathe {
                colour: Rgb::new(1, 2, 3),
                period_ms: 10,
            },
            Effect::Breathe {
                colour: Rgb::new(10, 20, 30),
                period_ms: 20,
            },
        ),
        (
            Effect::Pulse {
                colour: Rgb::new(1, 2, 3),
                period_ms: 10,
            },
            Effect::Pulse {
                colour: Rgb::new(10, 20, 30),
                period_ms: 20,
            },
        ),
        (
            Effect::Strobe {
                colour: Rgb::new(1, 2, 3),
                period_ms: 10,
            },
            Effect::Strobe {
                colour: Rgb::new(10, 20, 30),
                period_ms: 20,
            },
        ),
        (
            Effect::Scanner {
                colour: Rgb::new(1, 2, 3),
                period_ms: 10,
            },
            Effect::Scanner {
                colour: Rgb::new(10, 20, 30),
                period_ms: 20,
            },
        ),
        (
            Effect::Morph {
                colours: vec![Rgb::new(1, 2, 3)],
                period_ms: 10,
            },
            Effect::Morph {
                colours: vec![Rgb::new(10, 20, 30)],
                period_ms: 20,
            },
        ),
        (
            Effect::Spectrum { period_ms: 10 },
            Effect::Spectrum { period_ms: 20 },
        ),
        (
            Effect::Rainbow { period_ms: 10 },
            Effect::Rainbow { period_ms: 20 },
        ),
        (
            Effect::Hardware {
                id: HardwareEffectId::new("demo"),
                arguments: EffectArguments {
                    speed: Some(10),
                    duration_ms: Some(10),
                    brightness: Some(10),
                    colours: vec![Rgb::new(1, 2, 3)],
                    ..EffectArguments::default()
                },
            },
            Effect::Hardware {
                id: HardwareEffectId::new("demo"),
                arguments: EffectArguments {
                    speed: Some(20),
                    duration_ms: Some(20),
                    brightness: Some(20),
                    colours: vec![Rgb::new(10, 20, 30)],
                    ..EffectArguments::default()
                },
            },
        ),
    ];
    for (start, end) in effects {
        assert!(interpolate_effect(&start, &end, 1, 2, 359, encoded).is_ok());
    }
    assert_eq!(
        interpolate_effect(
            &Effect::Off,
            &Effect::Rainbow { period_ms: 20 },
            1,
            2,
            359,
            encoded,
        ),
        Err(TransitionInterpolationError::EffectIdentity)
    );
    assert_eq!(
        interpolate_effect(
            &Effect::Morph {
                colours: vec![Rgb::new(1, 2, 3)],
                period_ms: 10,
            },
            &Effect::Morph {
                colours: vec![],
                period_ms: 20,
            },
            1,
            2,
            359,
            encoded,
        ),
        Err(TransitionInterpolationError::EffectIdentity)
    );
}

#[test]
fn transition_helpers_reject_invalid_denominators_and_ranges() {
    assert_eq!(
        ease_progress(TransitionFunction::Linear, 1, 0),
        Err(TransitionInterpolationError::HardwareStructure)
    );
    assert_eq!(
        interpolate_u32(0, 1, 1, 0),
        Err(TransitionInterpolationError::HardwareStructure)
    );
    assert_eq!(
        snap_to_discrete_range_u32(1, DiscreteRange::new(0, 1, 0)),
        Err(TransitionInterpolationError::HardwareStructure)
    );
    assert_eq!(
        snap_to_discrete_range_u32(1, DiscreteRange::new(2, 1, 1)),
        Err(TransitionInterpolationError::HardwareStructure)
    );
    assert_eq!(
        TransitionOptions::new(Duration::from_nanos(1), None),
        Err(TransitionValidationError::SubMillisecondTiming)
    );
    assert_eq!(
        TransitionOptions::new(Duration::new(u64::MAX, 0), None),
        Err(TransitionValidationError::DurationTooLong)
    );
}
