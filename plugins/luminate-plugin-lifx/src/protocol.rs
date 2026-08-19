// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Encoding and decoding for the subset of the LIFX LAN protocol used here.

#![allow(
    clippy::indexing_slicing,
    clippy::missing_asserts_for_indexing,
    reason = "The LIFX wire protocol uses fixed offsets after explicit packet length validation."
)]

use std::io;

pub const HEADER_LEN: usize = 36;
pub const GET_SERVICE: u16 = 2;
pub const STATE_SERVICE: u16 = 3;
pub const SET_POWER: u16 = 21;
pub const GET_LABEL: u16 = 23;
pub const STATE_LABEL: u16 = 25;
pub const GET_VERSION: u16 = 32;
pub const STATE_VERSION: u16 = 33;
pub const ACKNOWLEDGEMENT: u16 = 45;
pub const GET_COLOR: u16 = 101;
pub const SET_COLOR: u16 = 102;
pub const SET_WAVEFORM: u16 = 103;
pub const LIGHT_STATE: u16 = 107;
pub const SET_COLOR_ZONES: u16 = 501;
pub const GET_COLOR_ZONES: u16 = 502;
pub const STATE_ZONE: u16 = 503;
pub const STATE_MULTI_ZONE: u16 = 506;
pub const SET_MULTI_ZONE_EFFECT: u16 = 508;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub source: u32,
    pub target: [u8; 8],
    pub sequence: u8,
    pub message_type: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hsbk {
    pub hue: u16,
    pub saturation: u16,
    pub brightness: u16,
    pub kelvin: u16,
}

pub fn packet(
    source: u32,
    target: [u8; 8],
    sequence: u8,
    message_type: u16,
    payload: &[u8],
    tagged: bool,
    acknowledgement_required: bool,
) -> io::Result<Vec<u8>> {
    let size = HEADER_LEN
        .checked_add(payload.len())
        .and_then(|size| u16::try_from(size).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "LIFX packet too large"))?;
    let mut bytes = vec![0; HEADER_LEN];
    bytes[0..2].copy_from_slice(&size.to_le_bytes());
    bytes[2] = 0;
    bytes[3] = 0x14 | (u8::from(tagged) << 5); // protocol 1024 + addressable
    bytes[4..8].copy_from_slice(&source.to_le_bytes());
    bytes[8..16].copy_from_slice(&target);
    bytes[22] = u8::from(acknowledgement_required) << 1;
    bytes[23] = sequence;
    bytes[32..34].copy_from_slice(&message_type.to_le_bytes());
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

pub fn parse(packet: &[u8]) -> io::Result<(Header, &[u8])> {
    if packet.len() < HEADER_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "short LIFX packet header",
        ));
    }
    let size = usize::from(u16::from_le_bytes([packet[0], packet[1]]));
    if size < HEADER_LEN || size > packet.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid LIFX packet size",
        ));
    }
    let protocol = u16::from_le_bytes([packet[2], packet[3]]) & 0x0fff;
    if protocol != 1024 || packet[3] & 0x10 == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported LIFX header",
        ));
    }
    let mut target = [0; 8];
    target.copy_from_slice(&packet[8..16]);
    Ok((
        Header {
            source: u32::from_le_bytes([packet[4], packet[5], packet[6], packet[7]]),
            target,
            sequence: packet[23],
            message_type: u16::from_le_bytes([packet[32], packet[33]]),
        },
        &packet[HEADER_LEN..size],
    ))
}

pub fn set_power_payload(on: bool) -> [u8; 2] {
    if on { u16::MAX } else { 0 }.to_le_bytes()
}

pub fn set_color_payload(colour: Hsbk, duration_ms: u32) -> [u8; 13] {
    let mut payload = [0; 13];
    write_hsbk(&mut payload[1..9], colour);
    payload[9..13].copy_from_slice(&duration_ms.to_le_bytes());
    payload
}

pub fn set_waveform_payload(colour: Hsbk, period_ms: u32, waveform: u8) -> [u8; 21] {
    let mut payload = [0; 21];
    payload[1] = 1; // transient: restore the original colour after each cycle
    write_hsbk(&mut payload[2..10], colour);
    payload[10..14].copy_from_slice(&period_ms.to_le_bytes());
    // The protocol documents a finite float cycle count but no infinite
    // sentinel. One billion cycles lasts more than three years even at the
    // minimum advertised 100ms period and is replaced by any later command.
    payload[14..18].copy_from_slice(&1_000_000_000_f32.to_le_bytes());
    payload[18..20].copy_from_slice(&0_i16.to_le_bytes()); // 50% pulse duty cycle
    payload[20] = waveform;
    payload
}

pub fn get_color_zones_payload(start: u8, end: u8) -> [u8; 2] {
    [start, end]
}

pub fn set_color_zones_payload(
    start: u8,
    end: u8,
    colour: Hsbk,
    duration_ms: u32,
    apply: u8,
) -> [u8; 15] {
    let mut payload = [0; 15];
    payload[0] = start;
    payload[1] = end;
    write_hsbk(&mut payload[2..10], colour);
    payload[10..14].copy_from_slice(&duration_ms.to_le_bytes());
    payload[14] = apply;
    payload
}

pub fn set_multi_zone_effect_payload(period_ms: u32, reverse: bool) -> [u8; 59] {
    let mut payload = [0; 59];
    payload[4] = 1; // MOVE
    payload[7..11].copy_from_slice(&period_ms.to_le_bytes());
    payload[11..19].copy_from_slice(&u64::MAX.to_le_bytes());
    // Parameter 1 is the Direction enum: 0 reversed, 1 away from zone zero.
    payload[31..35].copy_from_slice(&u32::from(!reverse).to_le_bytes());
    payload
}

/// Stops the currently running firmware multizone effect. The LIFX protocol
/// uses the same packet as `MOVE`, with the effect type set to `OFF` (zero).
pub const fn set_multi_zone_effect_off_payload() -> [u8; 59] {
    [0; 59]
}

pub fn parse_hsbk(payload: &[u8]) -> io::Result<Hsbk> {
    if payload.len() < 8 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "short LIFX HSBK payload",
        ));
    }
    Ok(Hsbk {
        hue: u16::from_le_bytes([payload[0], payload[1]]),
        saturation: u16::from_le_bytes([payload[2], payload[3]]),
        brightness: u16::from_le_bytes([payload[4], payload[5]]),
        kelvin: u16::from_le_bytes([payload[6], payload[7]]),
    })
}

fn write_hsbk(bytes: &mut [u8], colour: Hsbk) {
    bytes[0..2].copy_from_slice(&colour.hue.to_le_bytes());
    bytes[2..4].copy_from_slice(&colour.saturation.to_le_bytes());
    bytes[4..6].copy_from_slice(&colour.brightness.to_le_bytes());
    bytes[6..8].copy_from_slice(&colour.kelvin.to_le_bytes());
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
