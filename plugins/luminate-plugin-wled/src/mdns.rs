// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Bounded mDNS discovery for WLED HTTP endpoints.

#![allow(
    clippy::indexing_slicing,
    clippy::missing_asserts_for_indexing,
    reason = "DNS fields are indexed only after their enclosing fixed-size region is checked"
)]

use std::collections::{HashMap, HashSet};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::str;
use std::time::{Duration, Instant};

use crate::http::Endpoint;

const MDNS_ADDRESS: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(224, 0, 0, 251)), 5353);
const SERVICE: &str = "_wled._tcp.local";
const WINDOW: Duration = Duration::from_millis(900);
const MAX_PACKETS: usize = 64;

pub(crate) fn discover() -> io::Result<Vec<Endpoint>> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.set_write_timeout(Some(WINDOW))?;
    socket.send_to(&query(), MDNS_ADDRESS)?;
    let deadline = Instant::now() + WINDOW;
    let mut endpoints = HashSet::new();
    for _ in 0..MAX_PACKETS {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        socket.set_read_timeout(Some(remaining))?;
        let mut packet = [0_u8; 9000];
        match socket.recv_from(&mut packet) {
            Ok((length, source)) => {
                if let Some(packet) = packet.get(..length) {
                    endpoints.extend(parse_response(packet, source.ip()));
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(error) => return Err(error),
        }
    }
    let mut endpoints: Vec<_> = endpoints.into_iter().map(Endpoint::new).collect();
    endpoints.sort_by_key(|endpoint| endpoint.address);
    Ok(endpoints)
}

fn query() -> Vec<u8> {
    let mut packet = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in SERVICE.split('.') {
        packet.push(u8::try_from(label.len()).unwrap_or(0));
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&12_u16.to_be_bytes()); // PTR
    packet.extend_from_slice(&0x8001_u16.to_be_bytes()); // IN, unicast response requested
    packet
}

#[derive(Debug)]
struct Record {
    name: String,
    kind: u16,
    data_offset: usize,
    data_length: usize,
}

fn parse_response(packet: &[u8], source: IpAddr) -> Vec<SocketAddr> {
    let Some(header) = packet.get(..12) else {
        return Vec::new();
    };
    let questions = usize::from(u16::from_be_bytes([header[4], header[5]]));
    let records = usize::from(u16::from_be_bytes([header[6], header[7]]))
        + usize::from(u16::from_be_bytes([header[8], header[9]]))
        + usize::from(u16::from_be_bytes([header[10], header[11]]));
    let mut offset = 12;
    for _ in 0..questions {
        if read_name(packet, &mut offset).is_none() || packet.get(offset..offset + 4).is_none() {
            return Vec::new();
        }
        offset += 4;
    }
    let mut parsed = Vec::new();
    for _ in 0..records.min(512) {
        let Some(name) = read_name(packet, &mut offset) else {
            break;
        };
        let Some(header) = packet.get(offset..offset + 10) else {
            break;
        };
        let kind = u16::from_be_bytes([header[0], header[1]]);
        let data_length = usize::from(u16::from_be_bytes([header[8], header[9]]));
        offset += 10;
        if packet.get(offset..offset + data_length).is_none() {
            break;
        }
        parsed.push(Record {
            name: name.to_ascii_lowercase(),
            kind,
            data_offset: offset,
            data_length,
        });
        offset += data_length;
    }

    let mut addresses: HashMap<String, Vec<IpAddr>> = HashMap::new();
    for record in &parsed {
        let data = &packet[record.data_offset..record.data_offset + record.data_length];
        let address = match (record.kind, data) {
            (1, [a, b, c, d]) => Some(IpAddr::V4(Ipv4Addr::new(*a, *b, *c, *d))),
            (28, bytes) if bytes.len() == 16 => <[u8; 16]>::try_from(bytes)
                .ok()
                .map(Ipv6Addr::from)
                .map(IpAddr::V6),
            _ => None,
        };
        // An unauthenticated mDNS response must not redirect HTTP probes to a
        // different host. Explicitly configured endpoints remain unrestricted.
        if let Some(address) = address.filter(|address| *address == source) {
            addresses
                .entry(record.name.clone())
                .or_default()
                .push(address);
        }
    }

    let mut output = HashSet::new();
    for record in &parsed {
        if record.kind != 33 || !record.name.ends_with(SERVICE) || record.data_length < 7 {
            continue;
        }
        let data = &packet[record.data_offset..record.data_offset + record.data_length];
        let port = u16::from_be_bytes([data[4], data[5]]);
        let mut target_offset = record.data_offset + 6;
        let Some(target) = read_name(packet, &mut target_offset) else {
            continue;
        };
        if let Some(target_addresses) = addresses.get(&target.to_ascii_lowercase()) {
            output.extend(
                target_addresses
                    .iter()
                    .copied()
                    .map(|address| SocketAddr::new(address, port)),
            );
        }
    }
    output.into_iter().collect()
}

fn read_name(packet: &[u8], offset: &mut usize) -> Option<String> {
    let mut labels = Vec::new();
    let mut cursor = *offset;
    let mut jumped = false;
    for _ in 0..128 {
        let length = *packet.get(cursor)?;
        if length & 0xc0 == 0xc0 {
            let next = *packet.get(cursor + 1)?;
            let pointer = (usize::from(length & 0x3f) << 8) | usize::from(next);
            if !jumped {
                *offset = cursor + 2;
                jumped = true;
            }
            cursor = pointer;
            continue;
        }
        if length & 0xc0 != 0 {
            return None;
        }
        cursor += 1;
        if length == 0 {
            if !jumped {
                *offset = cursor;
            }
            return Some(labels.join("."));
        }
        let end = cursor.checked_add(usize::from(length))?;
        let label = str::from_utf8(packet.get(cursor..end)?).ok()?;
        labels.push(label.to_owned());
        cursor = end;
    }
    None
}

#[cfg(test)]
#[path = "mdns_tests.rs"]
mod tests;
