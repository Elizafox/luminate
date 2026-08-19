// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "these tests pass live slices directly to the module's C input decoder"
)]

use super::*;

fn input(encoding: u32, channels: &[LuminateColourChannelInput]) -> LuminateColourInput {
    LuminateColourInput {
        encoding,
        channels: channels.as_ptr(),
        channel_count: channels.len(),
    }
}

#[test]
fn colour_inputs_decode_every_public_encoding() {
    let additive = [
        LuminateColourChannelInput {
            channel: 0,
            value: 10,
        },
        LuminateColourChannelInput {
            channel: 1,
            value: 20,
        },
        LuminateColourChannelInput {
            channel: 2,
            value: 30,
        },
        LuminateColourChannelInput {
            channel: 3,
            value: 40,
        },
    ];
    assert_eq!(
        unsafe { read_colour(&input(0, &additive)) }.expect("additive colour"),
        Colour::additive(vec![
            ColourChannelValue::new(ColourChannel::Red, 10),
            ColourChannelValue::new(ColourChannel::Green, 20),
            ColourChannelValue::new(ColourChannel::Blue, 30),
            ColourChannelValue::new(ColourChannel::White, 40),
        ])
        .expect("additive colour")
    );

    let hue = LuminateColourChannelInput {
        channel: 8,
        value: 120,
    };
    let saturation = LuminateColourChannelInput {
        channel: 9,
        value: 80,
    };
    let value = LuminateColourChannelInput {
        channel: 10,
        value: 70,
    };
    let lightness = LuminateColourChannelInput {
        channel: 11,
        value: 60,
    };
    assert_eq!(
        unsafe { read_colour(&input(1, &[hue, saturation, value])) }.expect("HSV colour"),
        Colour::hsv(120, 80, 70)
    );
    assert_eq!(
        unsafe { read_colour(&input(2, &[hue, saturation, lightness])) }.expect("HSL colour"),
        Colour::hsl(120, 80, 60)
    );
    assert_eq!(
        unsafe {
            read_colour(&input(
                3,
                &[LuminateColourChannelInput {
                    channel: 12,
                    value: 4_000,
                }],
            ))
        }
        .expect("CCT colour"),
        Colour::cct(4_000)
    );
    assert_eq!(
        unsafe {
            read_colour(&input(
                4,
                &[LuminateColourChannelInput {
                    channel: 13,
                    value: 55,
                }],
            ))
        }
        .expect("monochrome colour"),
        Colour::monochrome(55)
    );
}

#[test]
fn colour_inputs_reject_malformed_shapes_and_values() {
    let null_channels = LuminateColourInput {
        encoding: 0,
        channels: ptr::null(),
        channel_count: 1,
    };
    assert_eq!(
        unsafe { read_colour(&null_channels) },
        Err(LuminateStatus::NullPointer)
    );
    assert_eq!(
        unsafe { read_colour(&input(99, &[])) },
        Err(LuminateStatus::InvalidArgument)
    );
    assert_eq!(
        unsafe { read_colour(&input(1, &[])) },
        Err(LuminateStatus::InvalidArgument)
    );
    assert_eq!(
        unsafe {
            read_colour(&input(
                0,
                &[LuminateColourChannelInput {
                    channel: 99,
                    value: 1,
                }],
            ))
        },
        Err(LuminateStatus::InvalidArgument)
    );
}
