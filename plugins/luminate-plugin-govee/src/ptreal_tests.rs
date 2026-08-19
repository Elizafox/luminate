// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Raw `ptReal` frame construction and validation tests.

use super::*;

#[test]
fn checksum_is_the_xor_of_the_preceding_bytes() {
    let built = frame(PROTYPE_WRITE, 0x04, &[50]).expect("build frame");
    let expected_checksum = built
        .get(..FRAME_LENGTH - 1)
        .expect("frame has 19 leading bytes")
        .iter()
        .fold(0_u8, |accumulator, byte| accumulator ^ byte);
    assert_eq!(built.last().copied(), Some(expected_checksum));
}

#[test]
fn frame_layout_matches_the_confirmed_brightness_shape() {
    let built = frame(PROTYPE_WRITE, 0x04, &[75]).expect("build brightness frame");
    assert_eq!(built.first().copied(), Some(PROTYPE_WRITE));
    assert_eq!(built.get(1).copied(), Some(0x04));
    assert_eq!(built.get(2).copied(), Some(75));
}

#[test]
fn oversized_payload_is_rejected() {
    let oversized = [0_u8; MAX_PAYLOAD_LENGTH + 1];
    assert!(frame(PROTYPE_WRITE, 0x00, &oversized).is_err());
}

#[test]
fn ptreal_command_base64_round_trips_the_frame_bytes() {
    let built = frame(PROTYPE_WRITE, 0x04, &[10]).expect("build frame");
    let command = ptreal_command(&[built]);
    let encoded = command["msg"]["data"]["command"][0]
        .as_str()
        .expect("command entry is a base64 string");
    let decoded = BASE64.decode(encoded).expect("decode base64 frame");
    assert_eq!(decoded, built);
}

#[test]
fn scene_frame_layout_matches_govee_lans_f_scene() {
    let built = scene_frame(0x2a).expect("build scene frame");
    assert_eq!(built.first().copied(), Some(PROTYPE_WRITE));
    assert_eq!(built.get(1).copied(), Some(0x05));
    assert_eq!(built.get(2).copied(), Some(0x04));
    assert_eq!(built.get(3).copied(), Some(0x2a));
    assert_eq!(built.get(4).copied(), Some(0x00));
    assert_eq!(built.get(5).copied(), Some(0x01));
}
