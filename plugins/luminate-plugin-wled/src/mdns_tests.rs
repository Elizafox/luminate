// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Bounded mDNS packet parsing and WLED service-discovery tests.

use super::*;

#[test]
fn query_requests_wled_service() {
    let packet = query();
    let mut offset = 12;
    assert_eq!(read_name(&packet, &mut offset).as_deref(), Some(SERVICE));
    assert_eq!(
        packet.get(offset..offset + 2),
        Some(12_u16.to_be_bytes().as_slice())
    );
}

#[test]
fn parses_srv_and_address_records() {
    let mut packet = vec![0, 0, 0x84, 0, 0, 0, 0, 2, 0, 0, 0, 0];
    append_name(&mut packet, "desk._wled._tcp.local");
    packet.extend_from_slice(&33_u16.to_be_bytes());
    packet.extend_from_slice(&1_u16.to_be_bytes());
    packet.extend_from_slice(&120_u32.to_be_bytes());
    let length_offset = packet.len();
    packet.extend_from_slice(&0_u16.to_be_bytes());
    let start = packet.len();
    packet.extend_from_slice(&0_u16.to_be_bytes());
    packet.extend_from_slice(&0_u16.to_be_bytes());
    packet.extend_from_slice(&80_u16.to_be_bytes());
    append_name(&mut packet, "wled-desk.local");
    let length = u16::try_from(packet.len() - start).expect("rdata length");
    packet[length_offset..length_offset + 2].copy_from_slice(&length.to_be_bytes());

    append_name(&mut packet, "wled-desk.local");
    packet.extend_from_slice(&1_u16.to_be_bytes());
    packet.extend_from_slice(&1_u16.to_be_bytes());
    packet.extend_from_slice(&120_u32.to_be_bytes());
    packet.extend_from_slice(&4_u16.to_be_bytes());
    packet.extend_from_slice(&[192, 0, 2, 10]);

    assert_eq!(
        parse_response(&packet, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10))),
        vec![SocketAddr::from(([192, 0, 2, 10], 80))]
    );
    assert!(parse_response(&packet, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 11))).is_empty());
}

fn append_name(packet: &mut Vec<u8>, name: &str) {
    for label in name.split('.') {
        packet.push(u8::try_from(label.len()).expect("label length"));
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
}

fn response_with_srv(instance: &str, target: &[u8], declared_length: u16) -> Vec<u8> {
    let mut packet = vec![0, 0, 0x84, 0, 0, 0, 0, 2, 0, 0, 0, 0];
    append_name(&mut packet, "wled-desk.local");
    packet.extend_from_slice(&1_u16.to_be_bytes());
    packet.extend_from_slice(&1_u16.to_be_bytes());
    packet.extend_from_slice(&120_u32.to_be_bytes());
    packet.extend_from_slice(&4_u16.to_be_bytes());
    packet.extend_from_slice(&[192, 0, 2, 10]);

    append_name(&mut packet, instance);
    packet.extend_from_slice(&33_u16.to_be_bytes());
    packet.extend_from_slice(&1_u16.to_be_bytes());
    packet.extend_from_slice(&120_u32.to_be_bytes());
    packet.extend_from_slice(&declared_length.to_be_bytes());
    packet.extend_from_slice(&[0, 0, 0, 0, 0, 80]);
    packet.extend_from_slice(target);
    packet
}

#[test]
fn service_suffix_requires_an_instance_label_boundary() {
    for instance in ["desk._wled._tcp.local", "desk._WLED._TCP.LOCAL"] {
        let packet = response_with_srv(instance, &[0xc0, 12], 8);
        assert_eq!(
            parse_response(&packet, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10))),
            vec![SocketAddr::from(([192, 0, 2, 10], 80))]
        );
    }
    for instance in ["desk.not_wled._tcp.local", "_wled._tcp.local"] {
        let packet = response_with_srv(instance, &[0xc0, 12], 8);
        assert!(parse_response(&packet, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10))).is_empty());
    }
}

#[test]
fn srv_target_must_fit_declared_record_data() {
    let mut target = Vec::new();
    append_name(&mut target, "wled-desk.local");
    // Both an uncompressed name and a compression pointer can otherwise read
    // beyond RDLENGTH into trailing bytes in the packet.
    for target in [target.as_slice(), &[0xc0, 12]] {
        let packet = response_with_srv("desk._wled._tcp.local", target, 7);
        assert!(parse_response(&packet, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10))).is_empty());
    }
}
