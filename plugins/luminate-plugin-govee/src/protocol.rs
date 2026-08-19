// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Govee's documented LAN JSON commands.
//!
//! These are Govee's own published LAN API commands (`turn`, `brightness`,
//! `colorwc`, `scan`), distinct from the undocumented `ptReal` raw-frame
//! passthrough in [`crate::ptreal`]. Whole-device power, brightness, RGB
//! colour, and CCT use these commands without constructing or guessing at raw
//! protocol bytes.

use serde::Deserialize;
use serde_json::{Value, json};

use luminate_core::rgb::Rgb;

pub(crate) fn scan_command() -> Value {
    json!({"msg": {"cmd": "scan", "data": {"account_topic": "reserve"}}})
}

pub(crate) fn turn_command(on: bool) -> Value {
    json!({"msg": {"cmd": "turn", "data": {"value": u8::from(on)}}})
}

/// Builds a `brightness` command. Govee's documented range is `1..=100`;
/// `0` has no representation here; callers should send [`turn_command`]
/// instead of `brightness_command(0)`.
pub(crate) fn brightness_command(percent: u8) -> Result<Value, String> {
    if !(1..=100).contains(&percent) {
        return Err(format!("Govee brightness {percent} is outside 1..=100"));
    }
    Ok(json!({"msg": {"cmd": "brightness", "data": {"value": percent}}}))
}

pub(crate) fn colorwc_rgb_command(rgb: Rgb) -> Value {
    json!({
        "msg": {
            "cmd": "colorwc",
            "data": {"color": {"r": rgb.r, "g": rgb.g, "b": rgb.b}},
        }
    })
}

pub(crate) fn colorwc_cct_command(kelvin: u32) -> Value {
    json!({
        "msg": {
            "cmd": "colorwc",
            "data": {"color": {"r": 0, "g": 0, "b": 0}, "colorTemInKelvin": kelvin},
        }
    })
}

/// A device's reply to [`scan_command`].
///
/// Field names follow community-documented Govee LAN API shapes and have been
/// verified against a reply from the owned H6022. Unknown fields are ignored
/// rather than rejected (no `deny_unknown_fields`) so firmware additions
/// degrade gracefully instead of dropping every discovered device.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ScanReplyEnvelope {
    pub(crate) msg: ScanReplyMsg,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ScanReplyMsg {
    #[serde(default)]
    pub(crate) cmd: String,
    pub(crate) data: ScanReplyData,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScanReplyData {
    pub(crate) device: String,
    pub(crate) sku: String,
    #[serde(default)]
    pub(crate) ble_version_soft: String,
}

/// Parses one UDP datagram as a [`ScanReplyEnvelope`], returning `None` for
/// anything that doesn't parse or isn't a scan reply. Discovery treats a
/// parse failure as "not a scan reply" rather than a hard error, since the
/// discovery socket may see traffic from other Govee-aware software on the
/// same LAN.
pub(crate) fn parse_scan_reply(payload: &[u8]) -> Option<ScanReplyData> {
    let envelope: ScanReplyEnvelope = serde_json::from_slice(payload).ok()?;
    (envelope.msg.cmd == "scan").then_some(envelope.msg.data)
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
