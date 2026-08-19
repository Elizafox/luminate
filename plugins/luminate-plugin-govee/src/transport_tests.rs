// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Retry, acknowledgement, timeout, and destination-selection transport tests.

use std::time::Duration;

use super::{
    IpAddr, Ipv4Addr, Ipv6Addr, RETRANSMIT_COUNT, SocketAddr, Transport as _, UdpSocket,
    UdpTransport, address_family_unspecified,
};

#[test]
fn udp_transport_send_retransmits_the_same_payload_over_loopback() {
    let receiver = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind receiver");
    receiver
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set a bounded read timeout");
    let address = SocketAddr::new(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        receiver.local_addr().expect("receiver address").port(),
    );

    UdpTransport
        .send(address, b"turn-on")
        .expect("send over loopback should succeed");

    let mut buffer = [0_u8; 32];
    for attempt in 0..=RETRANSMIT_COUNT {
        let (length, _) = receiver
            .recv_from(&mut buffer)
            .unwrap_or_else(|error| panic!("expected transmission {attempt}: {error}"));
        assert_eq!(&buffer[..length], b"turn-on");
    }
}

#[test]
fn address_family_unspecified_matches_an_ipv4_target() {
    let address: SocketAddr = "192.168.1.10:4003".parse().expect("valid IPv4 address");

    assert_eq!(
        address_family_unspecified(address),
        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    );
}

#[test]
fn address_family_unspecified_matches_an_ipv6_target() {
    let address: SocketAddr = "[fe80::1]:4003".parse().expect("valid IPv6 address");

    assert_eq!(
        address_family_unspecified(address),
        IpAddr::V6(Ipv6Addr::UNSPECIFIED)
    );
}
