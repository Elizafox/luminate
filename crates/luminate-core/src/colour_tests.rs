// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn additive_rejects_empty_duplicate_and_non_emitter_channels() {
    assert_eq!(
        Colour::additive(Vec::new()),
        Err(ColourError::EmptyAdditive)
    );
    assert_eq!(
        Colour::additive(vec![
            ColourChannelValue::new(ColourChannel::Red, 1),
            ColourChannelValue::new(ColourChannel::Red, 2),
        ]),
        Err(ColourError::DuplicateAdditiveChannel(ColourChannel::Red))
    );
    assert_eq!(
        Colour::additive(vec![ColourChannelValue::new(ColourChannel::Hue, 1)]),
        Err(ColourError::NonAdditiveChannel(ColourChannel::Hue))
    );
}

#[test]
fn malformed_additive_serialization_is_rejected() {
    let malformed = r#"{"Additive":[{"channel":"Red","value":1},{"channel":"Red","value":2}]}"#;
    assert!(serde_json::from_str::<Colour>(malformed).is_err());
}

#[test]
fn every_model_round_trips() {
    let values = [
        Colour::rgb(Rgb::new(1, 2, 3)),
        Colour::hsv(4, 5, 6),
        Colour::hsl(7, 8, 9),
        Colour::cct(4_000),
        Colour::monochrome(10),
    ];
    for value in values {
        let serialized = serde_json::to_string(&value).expect("colour should serialize");
        let decoded = serde_json::from_str(&serialized).expect("colour should deserialize");
        assert_eq!(value, decoded);
    }
}

#[test]
fn colour_queries_cover_darkness_channels_and_rgb_conversion_boundaries() {
    let cases = [
        (Colour::rgb(Rgb::new(0, 0, 0)), true),
        (Colour::hsv(120, 50, 0), true),
        (Colour::hsl(120, 50, 0), true),
        (Colour::cct(2_700), false),
        (Colour::monochrome(0), true),
        (Colour::monochrome(1), false),
    ];
    for (colour, dark) in cases {
        assert_eq!(colour.is_dark(), dark, "unexpected darkness for {colour:?}");
    }

    let hsv = Colour::hsv(10, 20, 30);
    assert_eq!(hsv.encoding(), ColourEncoding::Hsv);
    assert_eq!(hsv.channel(ColourChannel::Hue), Some(10));
    assert_eq!(hsv.channel(ColourChannel::Saturation), Some(20));
    assert_eq!(hsv.channel(ColourChannel::Value), Some(30));
    assert_eq!(hsv.channel(ColourChannel::Red), None);

    let hsl = Colour::hsl(40, 50, 60);
    assert_eq!(hsl.channel(ColourChannel::Lightness), Some(60));
    assert_eq!(hsl.channel(ColourChannel::Value), None);
    assert_eq!(
        Colour::cct(4_000).channel(ColourChannel::Temperature),
        Some(4_000)
    );
    assert_eq!(Colour::cct(4_000).channel(ColourChannel::Intensity), None);
    assert_eq!(
        Colour::monochrome(70).channel(ColourChannel::Intensity),
        Some(70)
    );

    assert_eq!(
        Colour::rgb(Rgb::new(1, 2, 3)).as_rgb(),
        Some(Rgb::new(1, 2, 3))
    );
    let out_of_range = Colour::additive(vec![
        ColourChannelValue::new(ColourChannel::Red, 256),
        ColourChannelValue::new(ColourChannel::Green, 2),
        ColourChannelValue::new(ColourChannel::Blue, 3),
    ])
    .expect("valid additive shape");
    assert_eq!(out_of_range.as_rgb(), None);
    assert_eq!(Colour::monochrome(1).as_rgb(), None);
}

#[test]
fn detailed_rgb_conversion_preserves_failure_reasons_and_ignores_extra_emitters() {
    let missing_green = Colour::additive(vec![
        ColourChannelValue::new(ColourChannel::Red, 1),
        ColourChannelValue::new(ColourChannel::Blue, 3),
    ])
    .expect("valid additive shape");
    assert_eq!(
        missing_green.try_as_rgb(),
        Err(Rgb8Error::MissingChannel(ColourChannel::Green))
    );

    let red_out_of_range = Colour::additive(vec![
        ColourChannelValue::new(ColourChannel::Red, 256),
        ColourChannelValue::new(ColourChannel::Green, 2),
        ColourChannelValue::new(ColourChannel::Blue, 3),
    ])
    .expect("valid additive shape");
    assert_eq!(
        red_out_of_range.try_as_rgb(),
        Err(Rgb8Error::ChannelOutOfRange {
            channel: ColourChannel::Red,
            value: 256,
        })
    );

    let rgbw = Colour::additive(vec![
        ColourChannelValue::new(ColourChannel::Red, 1),
        ColourChannelValue::new(ColourChannel::Green, 2),
        ColourChannelValue::new(ColourChannel::Blue, 3),
        ColourChannelValue::new(ColourChannel::White, 1_000),
    ])
    .expect("valid additive shape");
    assert_eq!(rgbw.try_as_rgb(), Ok(Rgb::new(1, 2, 3)));

    assert_eq!(
        Colour::cct(4_000).try_as_rgb(),
        Err(Rgb8Error::NonAdditive(ColourEncoding::Cct))
    );
}
