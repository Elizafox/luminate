// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Bounded DNS-SD discovery for local Hue Bridges.

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

use crate::configuration::{BridgeId, Endpoint};

const MDNS_ADDRESS: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(224, 0, 0, 251)), 5353);
const SERVICE: &str = "_hue._tcp.local";
const WINDOW: Duration = Duration::from_millis(900);
const MAX_PACKET_SIZE: usize = 9_000;
const MAX_PACKETS: usize = 64;
const MAX_RECORDS: usize = 512;
const MAX_CANDIDATES: usize = 64;
const MAX_NAME_STEPS: usize = 128;
const MAX_TXT_FIELDS: usize = 64;

pub(crate) fn discover(bridge_id: &BridgeId) -> io::Result<Vec<Endpoint>> {
    let mut endpoints = discover_all()?
        .into_iter()
        .filter(|candidate| candidate.bridge_id == *bridge_id)
        .map(|candidate| candidate.endpoint)
        .collect::<Vec<_>>();
    endpoints.sort_by(|left, right| {
        left.host()
            .cmp(right.host())
            .then_with(|| left.port().cmp(&right.port()))
    });
    endpoints.dedup();
    endpoints.truncate(MAX_CANDIDATES);
    Ok(endpoints)
}

pub(crate) fn discover_all() -> io::Result<Vec<DiscoveredBridge>> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.set_write_timeout(Some(WINDOW))?;
    socket.send_to(&query(), MDNS_ADDRESS)?;
    let deadline = Instant::now() + WINDOW;
    let mut bridges = HashSet::new();
    for _ in 0..MAX_PACKETS {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        socket.set_read_timeout(Some(remaining))?;
        let mut packet = [0_u8; MAX_PACKET_SIZE];
        match socket.recv_from(&mut packet) {
            Ok((length, source)) => {
                let Some(packet) = packet.get(..length) else {
                    continue;
                };
                bridges.extend(parse_response(packet, source.ip()));
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

    let mut bridges = bridges
        .into_iter()
        .filter_map(|candidate| {
            Some(DiscoveredBridge {
                bridge_id: candidate.bridge_id,
                model_id: candidate.model_id,
                endpoint: Endpoint::from_ip(candidate.address, candidate.port).ok()?,
            })
        })
        .collect::<Vec<_>>();
    bridges.sort_by(|left, right| {
        left.bridge_id
            .as_str()
            .cmp(right.bridge_id.as_str())
            .then_with(|| left.endpoint.host().cmp(right.endpoint.host()))
            .then_with(|| left.endpoint.port().cmp(&right.endpoint.port()))
    });
    bridges.truncate(MAX_CANDIDATES);
    Ok(bridges)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoveredBridge {
    pub(crate) bridge_id: BridgeId,
    pub(crate) model_id: Option<String>,
    pub(crate) endpoint: Endpoint,
}

fn query() -> Vec<u8> {
    let mut packet = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    append_query_name(&mut packet, SERVICE);
    packet.extend_from_slice(&12_u16.to_be_bytes()); // PTR
    packet.extend_from_slice(&0x8001_u16.to_be_bytes()); // IN, unicast response requested
    packet
}

fn append_query_name(packet: &mut Vec<u8>, name: &str) {
    for label in name.split('.') {
        packet.push(u8::try_from(label.len()).unwrap_or(0));
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
}

#[derive(Debug)]
struct Record {
    name: String,
    kind: u16,
    class: u16,
    data_offset: usize,
    data_length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Candidate {
    bridge_id: BridgeId,
    model_id: Option<String>,
    address: IpAddr,
    port: u16,
}

fn parse_response(packet: &[u8], source: IpAddr) -> Vec<Candidate> {
    let Some(header) = packet.get(..12) else {
        return Vec::new();
    };
    let flags = u16::from_be_bytes([header[2], header[3]]);
    if flags & 0x8000 == 0 || flags & 0x000f != 0 {
        return Vec::new();
    }
    let questions = usize::from(u16::from_be_bytes([header[4], header[5]]));
    let records = usize::from(u16::from_be_bytes([header[6], header[7]]))
        + usize::from(u16::from_be_bytes([header[8], header[9]]))
        + usize::from(u16::from_be_bytes([header[10], header[11]]));
    if records > MAX_RECORDS {
        return Vec::new();
    }

    let mut offset = 12;
    for _ in 0..questions {
        if read_name(packet, &mut offset).is_none() || packet.get(offset..offset + 4).is_none() {
            return Vec::new();
        }
        offset += 4;
    }
    let mut parsed = Vec::with_capacity(records);
    for _ in 0..records {
        let Some(name) = read_name(packet, &mut offset) else {
            return Vec::new();
        };
        let Some(record_header) = packet.get(offset..offset + 10) else {
            return Vec::new();
        };
        let kind = u16::from_be_bytes([record_header[0], record_header[1]]);
        let class = u16::from_be_bytes([record_header[2], record_header[3]]);
        let data_length = usize::from(u16::from_be_bytes([record_header[8], record_header[9]]));
        offset += 10;
        if packet.get(offset..offset + data_length).is_none() {
            return Vec::new();
        }
        parsed.push(Record {
            name: normalize_name(&name),
            kind,
            class,
            data_offset: offset,
            data_length,
        });
        offset += data_length;
    }

    assemble_candidates(packet, &parsed, source)
}

fn assemble_candidates(packet: &[u8], records: &[Record], source: IpAddr) -> Vec<Candidate> {
    let instances = records
        .iter()
        .filter(|record| record.kind == 12 && is_in_class(record) && record.name == SERVICE)
        .filter_map(|record| {
            let mut offset = record.data_offset;
            let name = normalize_name(&read_name(packet, &mut offset)?);
            let consumed = offset.checked_sub(record.data_offset)?;
            (consumed <= record.data_length && name.ends_with(&format!(".{SERVICE}")))
                .then_some(name)
        })
        .collect::<HashSet<_>>();

    let mut text = HashMap::new();
    let mut invalid_text = HashSet::new();
    for record in records.iter().filter(|record| {
        record.kind == 16 && is_in_class(record) && instances.contains(&record.name)
    }) {
        match parse_txt(record_data(packet, record)) {
            Some(fields) if !text.contains_key(&record.name) => {
                text.insert(record.name.clone(), fields);
            }
            _ => {
                invalid_text.insert(record.name.clone());
            }
        }
    }

    let mut addresses: HashMap<String, HashSet<IpAddr>> = HashMap::new();
    for record in records.iter().filter(|record| is_in_class(record)) {
        let data = record_data(packet, record);
        let address = match (record.kind, data) {
            (1, [a, b, c, d]) => Some(IpAddr::V4(Ipv4Addr::new(*a, *b, *c, *d))),
            (28, bytes) if bytes.len() == 16 => <[u8; 16]>::try_from(bytes)
                .ok()
                .map(Ipv6Addr::from)
                .map(IpAddr::V6),
            _ => None,
        };
        // DNS-SD is only a routing hint. A response cannot use an address RR
        // to redirect the eventual authenticated probe to a different host.
        if let Some(address) = address.filter(|address| *address == source) {
            addresses
                .entry(record.name.clone())
                .or_default()
                .insert(address);
        }
    }

    let mut candidates = HashSet::new();
    for record in records.iter().filter(|record| {
        record.kind == 33 && is_in_class(record) && instances.contains(&record.name)
    }) {
        if invalid_text.contains(&record.name) || record.data_length < 7 {
            continue;
        }
        let Some(fields) = text.get(&record.name) else {
            continue;
        };
        let Some(bridge_id) = fields
            .get("bridgeid")
            .and_then(|value| BridgeId::parse(value).ok())
        else {
            continue;
        };
        let data = record_data(packet, record);
        let port = u16::from_be_bytes([data[4], data[5]]);
        if port == 0 {
            continue;
        }
        let mut target_offset = record.data_offset + 6;
        let Some(target) = read_name(packet, &mut target_offset).map(|name| normalize_name(&name))
        else {
            continue;
        };
        let Some(consumed) = target_offset.checked_sub(record.data_offset) else {
            continue;
        };
        if consumed > record.data_length {
            continue;
        }
        let model_id = fields.get("modelid").cloned();
        if let Some(target_addresses) = addresses.get(&target) {
            let mut target_addresses = target_addresses.iter().copied().collect::<Vec<_>>();
            target_addresses.sort_by_key(ToString::to_string);
            for address in target_addresses {
                if candidates.len() == MAX_CANDIDATES {
                    break;
                }
                candidates.insert(Candidate {
                    bridge_id: bridge_id.clone(),
                    model_id: model_id.clone(),
                    address,
                    port,
                });
            }
        }
    }

    candidates.into_iter().collect()
}

fn is_in_class(record: &Record) -> bool {
    record.class & 0x7fff == 1
}

fn record_data<'a>(packet: &'a [u8], record: &Record) -> &'a [u8] {
    &packet[record.data_offset..record.data_offset + record.data_length]
}

fn parse_txt(data: &[u8]) -> Option<HashMap<String, String>> {
    let mut fields = HashMap::new();
    let mut offset = 0;
    for _ in 0..MAX_TXT_FIELDS {
        if offset == data.len() {
            return Some(fields);
        }
        let length = usize::from(*data.get(offset)?);
        offset += 1;
        let end = offset.checked_add(length)?;
        let field = str::from_utf8(data.get(offset..end)?).ok()?;
        offset = end;
        let (key, value) = field.split_once('=')?;
        let key = key.to_ascii_lowercase();
        if key.is_empty() || fields.insert(key, value.to_owned()).is_some() {
            return None;
        }
    }
    None
}

fn normalize_name(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}

fn read_name(packet: &[u8], offset: &mut usize) -> Option<String> {
    let mut labels = Vec::new();
    let mut cursor = *offset;
    let mut jumped = false;
    let mut visited = HashSet::new();
    for _ in 0..MAX_NAME_STEPS {
        if !visited.insert(cursor) {
            return None;
        }
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
        if length & 0xc0 != 0 || length > 63 {
            return None;
        }
        cursor += 1;
        if length == 0 {
            if !jumped {
                *offset = cursor;
            }
            let name = labels.join(".");
            return (name.len() <= 253).then_some(name);
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
