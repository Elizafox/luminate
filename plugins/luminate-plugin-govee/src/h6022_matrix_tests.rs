// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! H6022 matrix image validation and encoding tests.

use super::*;

fn four_corners() -> Vec<Rgb> {
    let mut framebuffer = vec![Rgb::WHITE; CELL_COUNT];
    framebuffer[0] = Rgb::new(255, 0, 0);
    framebuffer[11] = Rgb::new(0, 255, 0);
    framebuffer[120] = Rgb::new(0, 0, 255);
    framebuffer[131] = Rgb::new(255, 255, 0);
    framebuffer
}

#[test]
fn four_corner_capture_layout_is_reproduced_with_static_speed() {
    let frames = encode(&four_corners()).expect("encode captured image");
    let expected = [
        "a3000103585affffff6400012300031500040109",
        "a301ff0000000100ff000b010000ff7801ffff2f",
        "a3ff00830064020001ffff0000000000000000b8",
        "33050a20035a0000000000000000000000000045",
    ];
    assert_eq!(frames.len(), expected.len());
    for (frame, expected) in frames.iter().zip(expected) {
        assert_eq!(hex(frame), expected);
    }
}

#[test]
fn row_major_boundaries_have_the_expected_indices() {
    let payload = encode_payload(&four_corners()).expect("encode image");
    assert!(
        payload
            .windows(5)
            .any(|window| window == [1, 0, 255, 0, 11])
    );
    assert!(
        payload
            .windows(5)
            .any(|window| window == [1, 0, 0, 255, 120])
    );
    assert!(
        payload
            .windows(5)
            .any(|window| window == [1, 255, 255, 0, 131])
    );
}

#[test]
fn all_black_image_uses_black_as_the_background_without_groups() {
    let payload = encode_payload(&vec![Rgb::BLACK; CELL_COUNT]).expect("encode black");
    assert_eq!(&payload[4..7], &[0, 0, 0]);
    assert_eq!(payload[15], 0);
    assert_eq!(&payload[payload.len() - 7..], &[0, 100, 2, 0, 1, 255, 255]);
}

#[test]
fn most_common_colour_is_background_and_groups_follow_first_cell_order() {
    let mut framebuffer = vec![Rgb::new(9, 9, 9); CELL_COUNT];
    framebuffer[0] = Rgb::new(2, 0, 0);
    framebuffer[1] = Rgb::new(1, 0, 0);
    let payload = encode_payload(&framebuffer).expect("encode groups");
    assert_eq!(&payload[4..7], &[9, 9, 9]);
    assert_eq!(&payload[16..21], &[1, 2, 0, 0, 0]);
    assert_eq!(&payload[21..26], &[1, 1, 0, 0, 1]);
}

#[test]
fn maximum_colour_framebuffer_is_representable_and_bounded() {
    let framebuffer: Vec<Rgb> = (0..CELL_COUNT)
        .map(|index| {
            Rgb::new(
                u8::try_from(index).expect("cell index fits"),
                u8::try_from(index + 1).expect("cell index plus one fits"),
                u8::try_from(index + 2).expect("cell index plus two fits"),
            )
        })
        .collect();
    let frames = encode(&framebuffer).expect("encode maximum-colour image");
    let command = ptreal::ptreal_command(&frames);
    let datagram = serde_json::to_vec(&command).expect("serialize command");
    assert!(
        datagram.len() <= MAX_SAFE_UDP_PAYLOAD,
        "{}-byte UDP datagram exceeds the {MAX_SAFE_UDP_PAYLOAD}-byte safe bound",
        datagram.len()
    );
}

#[test]
fn invalid_framebuffer_lengths_are_rejected() {
    assert!(encode(&[]).is_err());
    assert!(encode(&vec![Rgb::BLACK; CELL_COUNT - 1]).is_err());
    assert!(encode(&vec![Rgb::BLACK; CELL_COUNT + 1]).is_err());
}

#[test]
fn chunks_are_padded_sequenced_and_checksummed() {
    let frames = encode(&vec![Rgb::BLACK; CELL_COUNT]).expect("encode black");
    assert_eq!(frames[0][1], 0);
    assert_eq!(frames[1][1], 0xff);
    assert_eq!(&frames[1][8..19], &[0; 11]);
    for frame in &frames {
        assert_eq!(frame.iter().fold(0_u8, |sum, byte| sum ^ byte), 0);
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes.iter().fold(String::new(), |mut output, byte| {
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
        output
    })
}
