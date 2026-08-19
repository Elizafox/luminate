// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Hue DNS-SD query and hostile-response regression tests.

use super::*;

const INSTANCE: &str = "Philips Hue - 012345._hue._tcp.local";
const TARGET: &str = "hue-bridge.local";
const SOURCE: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10));

#[test]
fn query_requests_registered_hue_service() {
    let packet = query();
    let mut offset = 12;
    assert_eq!(read_name(&packet, &mut offset).as_deref(), Some(SERVICE));
    assert_eq!(
        packet.get(offset..offset + 2),
        Some(12_u16.to_be_bytes().as_slice())
    );
}

#[test]
fn parses_identity_endpoint_and_model_from_related_records() {
    let packet = response_packet(SOURCE);
    let candidates = parse_response(&packet, SOURCE);

    assert_eq!(candidates.len(), 1);
    let candidate = candidates.first().expect("Hue candidate");
    assert_eq!(candidate.bridge_id.as_str(), "001788fffe012345");
    assert_eq!(candidate.model_id.as_deref(), Some("BSB002"));
    assert_eq!(candidate.address, SOURCE);
    assert_eq!(candidate.port, 443);
}

#[test]
fn rejects_unrelated_source_address_injection() {
    let packet = response_packet(SOURCE);

    assert!(parse_response(&packet, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 99))).is_empty());
}

#[test]
fn requires_ptr_srv_txt_and_address_to_join_on_one_instance() {
    let mut packet = response_packet(SOURCE);
    replace_ascii(&mut packet, b"bridgeid", b"bridgexd");

    assert!(parse_response(&packet, SOURCE).is_empty());
}

#[test]
fn rejects_conflicting_duplicate_identity_fields() {
    let packet = response_packet_with_txt(
        &[
            "bridgeid=001788fffe012345",
            "bridgeid=001788fffe999999",
            "modelid=BSB002",
        ],
        SOURCE,
    );

    assert!(parse_response(&packet, SOURCE).is_empty());
}

#[test]
fn rejects_truncated_records_and_compression_cycles() {
    let mut truncated = response_packet(SOURCE);
    let _last = truncated.pop();
    assert!(parse_response(&truncated, SOURCE).is_empty());

    let cycle = vec![0xc0, 0x00];
    let mut offset = 0;
    assert!(read_name(&cycle, &mut offset).is_none());
}

#[test]
fn rejects_queries_error_responses_and_excessive_record_counts() {
    let mut query_packet = response_packet(SOURCE);
    query_packet[2] = 0;
    assert!(parse_response(&query_packet, SOURCE).is_empty());

    let mut error_packet = response_packet(SOURCE);
    error_packet[3] = 3;
    assert!(parse_response(&error_packet, SOURCE).is_empty());

    let mut excessive = response_packet(SOURCE);
    excessive[6..8].copy_from_slice(
        &u16::try_from(MAX_RECORDS + 1)
            .expect("record count")
            .to_be_bytes(),
    );
    assert!(parse_response(&excessive, SOURCE).is_empty());
}

fn response_packet(address: IpAddr) -> Vec<u8> {
    response_packet_with_txt(&["bridgeid=001788FFFE012345", "modelid=BSB002"], address)
}

fn response_packet_with_txt(fields: &[&str], address: IpAddr) -> Vec<u8> {
    let mut packet = vec![0, 0, 0x84, 0, 0, 0, 0, 4, 0, 0, 0, 0];
    append_record(&mut packet, SERVICE, 12, 1, |data| {
        append_name(data, INSTANCE);
    });
    append_record(&mut packet, INSTANCE, 33, 1, |data| {
        data.extend_from_slice(&0_u16.to_be_bytes());
        data.extend_from_slice(&0_u16.to_be_bytes());
        data.extend_from_slice(&443_u16.to_be_bytes());
        append_name(data, TARGET);
    });
    append_record(&mut packet, INSTANCE, 16, 1, |data| {
        for field in fields {
            data.push(u8::try_from(field.len()).expect("TXT field length"));
            data.extend_from_slice(field.as_bytes());
        }
    });
    append_record(
        &mut packet,
        TARGET,
        match address {
            IpAddr::V4(_) => 1,
            IpAddr::V6(_) => 28,
        },
        1,
        |data| match address {
            IpAddr::V4(address) => data.extend_from_slice(&address.octets()),
            IpAddr::V6(address) => data.extend_from_slice(&address.octets()),
        },
    );
    packet
}

fn append_record(
    packet: &mut Vec<u8>,
    name: &str,
    kind: u16,
    class: u16,
    append_data: impl FnOnce(&mut Vec<u8>),
) {
    append_name(packet, name);
    packet.extend_from_slice(&kind.to_be_bytes());
    packet.extend_from_slice(&class.to_be_bytes());
    packet.extend_from_slice(&120_u32.to_be_bytes());
    let length_offset = packet.len();
    packet.extend_from_slice(&0_u16.to_be_bytes());
    let data_offset = packet.len();
    append_data(packet);
    let length = u16::try_from(packet.len() - data_offset).expect("record data length");
    packet[length_offset..length_offset + 2].copy_from_slice(&length.to_be_bytes());
}

fn append_name(packet: &mut Vec<u8>, name: &str) {
    for label in name.split('.') {
        packet.push(u8::try_from(label.len()).expect("DNS label length"));
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
}

fn replace_ascii(packet: &mut [u8], old: &[u8], new: &[u8]) {
    assert_eq!(old.len(), new.len());
    let offset = packet
        .windows(old.len())
        .position(|window| window == old)
        .expect("text to replace");
    packet[offset..offset + old.len()].copy_from_slice(new);
}
