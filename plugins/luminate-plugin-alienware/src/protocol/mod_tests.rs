// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Shared HID selection and colour-conversion tests.

use super::*;
use luminate_core::capability::ColourChannel;
use luminate_core::colour::ColourChannelValue;
use luminate_plugin_api::PluginUpdateOperation;

fn additive(channels: &[(ColourChannel, u32)]) -> Colour {
    Colour::additive(
        channels
            .iter()
            .map(|&(channel, value)| ColourChannelValue::new(channel, value))
            .collect(),
    )
    .expect("test additive colour is valid")
}

#[test]
fn rgb_from_colour_reads_the_named_channels() {
    let colour = additive(&[
        (ColourChannel::Red, 0x10),
        (ColourChannel::Green, 0x20),
        (ColourChannel::Blue, 0x30),
    ]);

    let rgb = rgb_from_colour(&colour).expect("all three channels are present");

    assert_eq!(rgb, Rgb::new(0x10, 0x20, 0x30));
}

#[test]
fn rgb_from_colour_ignores_channels_outside_the_rgb_triplet() {
    let colour = additive(&[
        (ColourChannel::Red, 1),
        (ColourChannel::Green, 2),
        (ColourChannel::Blue, 3),
        (ColourChannel::Amber, 255),
        (ColourChannel::White, 255),
    ]);

    let rgb = rgb_from_colour(&colour).expect("extra channels should be ignored");

    assert_eq!(rgb, Rgb::new(1, 2, 3));
}

#[test]
fn rgb_from_colour_rejects_non_additive_encoding() {
    let colour = Colour::cct(3500);

    let error = rgb_from_colour(&colour).expect_err("Cct is not additive RGB");

    assert!(error.contains("only additive RGB"));
}

#[test]
fn rgb_from_colour_rejects_a_channel_value_outside_eight_bits() {
    let colour = additive(&[
        (ColourChannel::Red, 256),
        (ColourChannel::Green, 0),
        (ColourChannel::Blue, 0),
    ]);

    let error = rgb_from_colour(&colour).expect_err("256 does not fit in u8");

    assert!(error.contains("exceeds 8-bit RGB"));
}

#[test]
fn rgb_from_colour_reports_which_channel_is_missing() {
    let missing_green = additive(&[(ColourChannel::Red, 1), (ColourChannel::Blue, 3)]);
    assert!(
        rgb_from_colour(&missing_green)
            .expect_err("green is absent")
            .contains("missing green channel")
    );

    let missing_blue = additive(&[(ColourChannel::Red, 1), (ColourChannel::Green, 2)]);
    assert!(
        rgb_from_colour(&missing_blue)
            .expect_err("blue is absent")
            .contains("missing blue channel")
    );

    let missing_red = additive(&[(ColourChannel::Green, 2), (ColourChannel::Blue, 3)]);
    assert!(
        rgb_from_colour(&missing_red)
            .expect_err("red is absent")
            .contains("missing red channel")
    );
}

#[test]
fn effect_to_rgb_static_lowers_static_effect_through_rgb_from_colour() {
    let colour = additive(&[
        (ColourChannel::Red, 4),
        (ColourChannel::Green, 5),
        (ColourChannel::Blue, 6),
    ]);
    let operation = PluginUpdateOperation::SetEffect {
        effect: Effect::Static { colour },
    };

    let rgb = effect_to_rgb_static(&operation).expect("additive colour lowers cleanly");

    assert_eq!(rgb, Rgb::new(4, 5, 6));
}

#[test]
fn effect_to_rgb_static_passes_through_a_static_effect_colour() {
    let operation = PluginUpdateOperation::SetEffect {
        effect: Effect::Static {
            colour: Colour::rgb(Rgb::new(7, 8, 9)),
        },
    };

    let rgb = effect_to_rgb_static(&operation).expect("static effect carries its own colour");

    assert_eq!(rgb, Rgb::new(7, 8, 9));
}

#[test]
fn effect_to_rgb_static_treats_off_and_clear_as_black() {
    let off = PluginUpdateOperation::SetEffect {
        effect: Effect::Off,
    };
    let clear = PluginUpdateOperation::Clear;

    assert_eq!(
        effect_to_rgb_static(&off).expect("off lowers to black"),
        Rgb::new(0, 0, 0)
    );
    assert_eq!(
        effect_to_rgb_static(&clear).expect("clear lowers to black"),
        Rgb::new(0, 0, 0)
    );
}

#[test]
fn effect_to_rgb_static_rejects_operations_with_no_single_colour() {
    let brightness = PluginUpdateOperation::SetBrightness { value: 50 };
    let save = PluginUpdateOperation::SaveCurrent;
    let animated = PluginUpdateOperation::SetEffect {
        effect: Effect::Breathe {
            colour: Rgb::new(0, 0, 0),
            period_ms: 500,
        },
    };

    for operation in [&brightness, &save, &animated] {
        assert!(
            effect_to_rgb_static(operation)
                .expect_err("operation has no single static colour")
                .contains("cannot be lowered")
        );
    }
}
