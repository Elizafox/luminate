// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! JSON command serialization and response parsing tests.

use super::*;

#[test]
fn brightness_command_rejects_out_of_range_values() {
    assert!(brightness_command(0).is_err());
    assert!(brightness_command(101).is_err());
    assert_eq!(
        brightness_command(50).expect("valid brightness"),
        json!({"msg": {"cmd": "brightness", "data": {"value": 50}}})
    );
}

#[test]
fn turn_command_encodes_boolean_as_zero_or_one() {
    assert_eq!(
        turn_command(true),
        json!({"msg": {"cmd": "turn", "data": {"value": 1}}})
    );
    assert_eq!(
        turn_command(false),
        json!({"msg": {"cmd": "turn", "data": {"value": 0}}})
    );
}

#[test]
fn colorwc_commands_encode_rgb_and_kelvin_separately() {
    assert_eq!(
        colorwc_rgb_command(Rgb::new(1, 2, 3)),
        json!({"msg": {"cmd": "colorwc", "data": {"color": {"r": 1, "g": 2, "b": 3}}}})
    );
    assert_eq!(
        colorwc_cct_command(4000),
        json!({
            "msg": {
                "cmd": "colorwc",
                "data": {"color": {"r": 0, "g": 0, "b": 0}, "colorTemInKelvin": 4000},
            }
        })
    );
}

#[test]
fn parses_a_well_formed_scan_reply() {
    let payload = json!({
        "msg": {
            "cmd": "scan",
            "data": {
                "ip": "192.0.2.10",
                "device": "AA:BB:CC:DD:EE:FF",
                "sku": "H6022",
                "bleVersionHard": "1",
                "bleVersionSoft": "1.2",
            }
        }
    });
    let data = parse_scan_reply(&serde_json::to_vec(&payload).expect("serialize fixture"))
        .expect("parse scan reply");
    assert_eq!(data.device, "AA:BB:CC:DD:EE:FF");
    assert_eq!(data.sku, "H6022");
}

#[test]
fn ignores_malformed_or_non_scan_payloads() {
    assert!(parse_scan_reply(b"not json").is_none());
    assert!(parse_scan_reply(br#"{"msg":{"cmd":"turn","data":{}}}"#).is_none());
}
