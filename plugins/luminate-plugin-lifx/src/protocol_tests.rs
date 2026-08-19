// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! LIFX LAN packet layout and payload encoding tests.

use super::*;

const TARGET: [u8; 8] = [0xd0, 0x73, 0xd5, 0, 0x13, 0x37, 0, 0];

#[test]
fn set_color_matches_official_packet_shape() {
    let payload = set_color_payload(
        Hsbk {
            hue: 21_845,
            saturation: u16::MAX,
            brightness: u16::MAX,
            kelvin: 3500,
        },
        0,
    );
    let bytes =
        packet(2, TARGET, 1, SET_COLOR, &payload, false, true).expect("packet should encode");
    assert_eq!(bytes.len(), 49);
    assert_eq!(&bytes[0..2], &49_u16.to_le_bytes());
    assert_eq!(bytes[3], 0x14);
    assert_eq!(bytes[22], 0x02);
    assert_eq!(&bytes[32..34], &SET_COLOR.to_le_bytes());
    assert_eq!(bytes[37..39], 21_845_u16.to_le_bytes());
}

#[test]
fn discovery_packet_is_tagged_and_broadcast_targeted() {
    let bytes =
        packet(0x1234, [0; 8], 7, GET_SERVICE, &[], true, false).expect("packet should encode");
    assert_eq!(bytes.len(), HEADER_LEN);
    assert_eq!(bytes[3], 0x34);
    assert_eq!(&bytes[8..16], &[0; 8]);
    assert_eq!(
        parse(&bytes).expect("packet should parse").0.message_type,
        2
    );
}

#[test]
fn waveform_payload_has_official_field_offsets() {
    let payload = set_waveform_payload(
        Hsbk {
            hue: 1,
            saturation: 2,
            brightness: 3,
            kelvin: 3500,
        },
        1200,
        4,
    );
    assert_eq!(payload.len(), 21);
    assert_eq!(payload[1], 1);
    assert_eq!(&payload[10..14], &1200_u32.to_le_bytes());
    assert_eq!(payload[20], 4);
}

#[test]
fn parser_rejects_truncated_and_wrong_protocol_packets() {
    assert!(parse(&[0; HEADER_LEN - 1]).is_err());
    let mut bytes =
        packet(2, TARGET, 1, GET_COLOR, &[], false, false).expect("packet should encode");
    bytes[2] = 1;
    assert!(parse(&bytes).is_err());
}

#[test]
fn legacy_multizone_payloads_match_official_layouts() {
    let colour = Hsbk {
        hue: 1,
        saturation: 2,
        brightness: 3,
        kelvin: 3500,
    };
    let set = set_color_zones_payload(2, 4, colour, 500, 1);
    assert_eq!(set.len(), 15);
    assert_eq!(&set[0..2], &[2, 4]);
    assert_eq!(&set[2..4], &1_u16.to_le_bytes());
    assert_eq!(&set[10..14], &500_u32.to_le_bytes());
    assert_eq!(set[14], 1);
    assert_eq!(get_color_zones_payload(0, u8::MAX), [0, u8::MAX]);
}

#[test]
fn move_effect_uses_direction_parameter_one() {
    let payload = set_multi_zone_effect_payload(1200, false);
    assert_eq!(payload.len(), 59);
    assert_eq!(payload[4], 1);
    assert_eq!(&payload[7..11], &1200_u32.to_le_bytes());
    assert_eq!(&payload[31..35], &1_u32.to_le_bytes());
}

#[test]
fn multizone_off_effect_has_zero_effect_type() {
    let payload = set_multi_zone_effect_off_payload();
    assert_eq!(payload.len(), 59);
    assert_eq!(payload[4], 0);
    assert!(payload.iter().all(|byte| *byte == 0));
}
