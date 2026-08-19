// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Raw 20-byte Govee protocol frames, carried over LAN via the undocumented
//! `ptReal` passthrough command (the same frame shape BLE uses).
//!
//! [`crate::protocol`]'s documented `turn`/`brightness`/`colorwc` commands
//! cover whole-device power/brightness/colour; matrix images and firmware
//! scenes have no documented LAN command at all, so they go through this
//! module's raw frames instead.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};

pub(crate) const PROTYPE_WRITE: u8 = 0x33;

const FRAME_LENGTH: usize = 20;
const MAX_PAYLOAD_LENGTH: usize = FRAME_LENGTH - 3; // protype + cmd_type + checksum
pub(crate) type Frame = [u8; FRAME_LENGTH];

/// Builds one 20-byte frame: `protype`, `cmd_type`, zero-padded `payload`,
/// then an XOR checksum over bytes `0..19` as the final byte.
pub(crate) fn frame(protype: u8, cmd_type: u8, payload: &[u8]) -> Result<Frame, String> {
    if payload.len() > MAX_PAYLOAD_LENGTH {
        return Err(format!(
            "ptReal payload of {} bytes exceeds the {MAX_PAYLOAD_LENGTH}-byte limit",
            payload.len()
        ));
    }
    let mut frame = [0_u8; FRAME_LENGTH];
    if let Some(slot) = frame.first_mut() {
        *slot = protype;
    }
    if let Some(slot) = frame.get_mut(1) {
        *slot = cmd_type;
    }
    if let Some(destination) = frame.get_mut(2..2 + payload.len()) {
        destination.copy_from_slice(payload);
    }
    let checksum = frame
        .get(..FRAME_LENGTH - 1)
        .unwrap_or_default()
        .iter()
        .fold(0_u8, |accumulator, byte| accumulator ^ byte);
    if let Some(slot) = frame.last_mut() {
        *slot = checksum;
    }
    Ok(frame)
}

/// `33 05 <sub_mode> <payload...>`: the "mode" command family. The third
/// byte selects a sub-mode (`0x04` scene select, `0x0d` whole-device colour,
/// `0x0a` DIY commit).
fn mode_frame(sub_mode: u8, payload: &[u8]) -> Result<[u8; FRAME_LENGTH], String> {
    let mut framed = Vec::with_capacity(1 + payload.len());
    framed.push(sub_mode);
    framed.extend_from_slice(payload);
    frame(PROTYPE_WRITE, 0x05, &framed)
}

/// Firmware scene select, sub-mode `0x04`: `33 05 04 <code> 00 01`. The
/// trailing `00 01` is confirmed by a working implementation despite being
/// absent from the prose documentation.
pub(crate) fn scene_frame(code: u8) -> Result<[u8; FRAME_LENGTH], String> {
    mode_frame(0x04, &[code, 0x00, 0x01])
}

/// One `0xa3` multipart frame with up to 17 bytes of payload.
pub(crate) fn multipart_frame(sequence: u8, payload: &[u8]) -> Result<Frame, String> {
    frame(0xa3, sequence, payload)
}

/// Commits an uploaded H6022 DIY/graffiti image.
pub(crate) fn diy_commit_frame() -> Result<Frame, String> {
    mode_frame(0x0a, &[0x20, 0x03, 0x5a])
}

/// Wraps raw frames for `ptReal` passthrough:
/// `{"msg":{"cmd":"ptReal","data":{"command":[<base64 frame>, ...]}}}`.
pub(crate) fn ptreal_command(frames: &[Frame]) -> Value {
    let encoded: Vec<String> = frames.iter().map(|frame| BASE64.encode(frame)).collect();
    json!({"msg": {"cmd": "ptReal", "data": {"command": encoded}}})
}

#[cfg(test)]
#[path = "ptreal_tests.rs"]
mod tests;
