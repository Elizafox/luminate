// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Packet construction for the nine-byte legacy `AlienFX` v2 protocol.
//!
//! This module deliberately contains no device discovery or generic zone
//! scanning.  Masks are profile data and must be selected by an exact
//! controller identity before a packet reaches a transport.

use luminate_core::rgb::Rgb;

pub(crate) const REPORT_LEN: usize = 9;
pub(crate) const REPORT_ID: u8 = 0x02;

const SET_COLOUR: u8 = 0x03;
const LOOP_BLOCK_END: u8 = 0x04;
const TRANSMIT_EXECUTE: u8 = 0x05;
const GET_STATUS: u8 = 0x06;

/// The `M14xR3` masks reported by the pinned `trackmastersteve/alienfx`
/// controller profile.  The source marks these as requiring correct zone
/// codes; keep them as evidence rather than publishing them as validated
/// topology until hardware confirms the mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct M14xZone {
    pub(crate) surface_id: &'static str,
    pub(crate) mask: u16,
}

pub(crate) const M14X_ZONES: &[M14xZone] = &[
    M14xZone {
        surface_id: "left-keyboard",
        mask: 0x0001,
    },
    M14xZone {
        surface_id: "centre-left-keyboard",
        mask: 0x0002,
    },
    M14xZone {
        surface_id: "centre-right-keyboard",
        mask: 0x0004,
    },
    M14xZone {
        surface_id: "right-keyboard",
        mask: 0x0008,
    },
    M14xZone {
        surface_id: "right-speaker",
        mask: 0x0020,
    },
    M14xZone {
        surface_id: "left-speaker",
        mask: 0x0040,
    },
    M14xZone {
        surface_id: "logo",
        mask: 0x0100,
    },
    M14xZone {
        surface_id: "touchpad",
        mask: 0x0200,
    },
    M14xZone {
        surface_id: "status-leds",
        mask: 0x0800,
    },
    M14xZone {
        surface_id: "power-button",
        mask: 0x2000,
    },
    M14xZone {
        surface_id: "hdd-leds",
        mask: 0x4000,
    },
];

const fn empty(command: u8) -> [u8; REPORT_LEN] {
    [REPORT_ID, command, 0, 0, 0, 0, 0, 0, 0]
}

fn packed_colour(colour: Rgb) -> [u8; 2] {
    let red = colour.r >> 4;
    let green = colour.g >> 4;
    let blue = colour.b >> 4;
    [(red << 4) | green, blue << 4]
}

/// Builds the non-mutating status query used before a legacy transaction.
pub(crate) const fn get_status() -> [u8; REPORT_LEN] {
    empty(GET_STATUS)
}

/// Builds a volatile static-colour command for one or more profile zones.
pub(crate) fn set_colour(block: u8, zone_mask: u16, colour: Rgb) -> [u8; REPORT_LEN] {
    let mut report = empty(SET_COLOUR);
    report[2] = block;
    let [high, low] = zone_mask.to_be_bytes();
    report[3] = 0;
    report[4] = high;
    report[5] = low;
    let [first, second] = packed_colour(colour);
    report[6] = first;
    report[7] = second;
    report
}

pub(crate) const fn loop_block_end() -> [u8; REPORT_LEN] {
    empty(LOOP_BLOCK_END)
}

pub(crate) const fn transmit_execute() -> [u8; REPORT_LEN] {
    empty(TRANSMIT_EXECUTE)
}

/// Returns the minimal volatile static transaction.  It intentionally omits
/// reset, save, power-state, and firmware-animation commands.
pub(crate) fn static_transaction(zone_mask: u16, colour: Rgb) -> [[u8; REPORT_LEN]; 4] {
    [
        get_status(),
        set_colour(1, zone_mask, colour),
        loop_block_end(),
        transmit_execute(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_m14x_red_at_the_expected_quantisation() {
        assert_eq!(
            set_colour(1, 0x0101, Rgb::new(255, 128, 15)),
            [0x02, 0x03, 0x01, 0x00, 0x01, 0x01, 0xf8, 0x00, 0x00]
        );
    }

    #[test]
    fn static_transaction_has_no_persistence_or_reset_command() {
        let transaction = static_transaction(0x0001, Rgb::new(0, 0, 0));
        assert_eq!(transaction[0], [0x02, 0x06, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(transaction[1][1], 0x03);
        assert_eq!(transaction[2][1], 0x04);
        assert_eq!(transaction[3][1], 0x05);
        assert!(transaction.iter().all(|report| report[1] != 0x07));
        assert!(transaction.iter().all(|report| report[1] != 0x09));
    }

    #[test]
    fn m14x_masks_are_bounded_to_the_profile_width() {
        assert!(M14X_ZONES.iter().all(|zone| zone.mask <= 0x7fff));
    }
}
