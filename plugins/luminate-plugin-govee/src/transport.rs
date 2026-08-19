// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Outbound command transport for Govee's LAN control port.
//!
//! Govee's documented LAN commands (`turn`, `brightness`, `colorwc`) have no
//! application-level acknowledgement: a device applies or silently ignores a
//! command, with no reply to distinguish the two. This is weaker than
//! LIFX's ack-based writes or WLED's HTTP status codes, so [`UdpTransport`]
//! does not wait for a reply at all; it only retransmits a couple of times
//! as a best-effort guard against ordinary UDP packet loss on the LAN.
//! Liveness is instead judged by whether the device answered a recent
//! discovery scan (see `crate::runtime().devices`), not by the outcome of a
//! command send.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::thread;
use std::time::Duration;

/// Gap between best-effort retransmissions of the same command.
const RETRANSMIT_GAP: Duration = Duration::from_millis(50);
/// Total sends per command: the original plus this many retransmissions.
const RETRANSMIT_COUNT: usize = 1;

pub(crate) trait Transport {
    fn send(&self, address: SocketAddr, payload: &[u8]) -> io::Result<()>;
}

pub(crate) struct UdpTransport;

impl Transport for UdpTransport {
    fn send(&self, address: SocketAddr, payload: &[u8]) -> io::Result<()> {
        let socket = UdpSocket::bind((address_family_unspecified(address), 0))?;
        socket.send_to(payload, address)?;
        for _retransmission in 0..RETRANSMIT_COUNT {
            thread::sleep(RETRANSMIT_GAP);
            socket.send_to(payload, address)?;
        }
        Ok(())
    }
}

fn address_family_unspecified(address: SocketAddr) -> IpAddr {
    if address.is_ipv6() {
        Ipv6Addr::UNSPECIFIED.into()
    } else {
        Ipv4Addr::UNSPECIFIED.into()
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "transport_fakes.rs"]
pub(crate) mod fakes;
