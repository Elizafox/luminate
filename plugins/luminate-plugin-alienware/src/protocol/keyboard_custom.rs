// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! `cc:8c`/`cc:8b` custom-animation (per-key) packet builders and the
//! full-table apply sequence documented in
//! `docs/design/hardware/alienware/alienware-keyboard-rgb-hid-protocol-spec.md` §5.
//!
//! There is no per-key readback (§6.2), so per §11 a complete resend of the
//! assignment + RGB tables is the documented approach for every change, not
//! an optimization to avoid. Packet builders here are layout-agnostic: they
//! take matrix indices and colours, with no knowledge of key names or which
//! `KeyboardLayoutId` produced them.
#![allow(
    clippy::indexing_slicing,
    reason = "Keyboard custom reports are fixed-layout HID packets with checked offsets."
)]

use std::collections::{BTreeMap, BTreeSet};
use std::thread;
use std::time::Duration;

use luminate_core::rgb::Rgb;

use crate::keyboard_layout::KEYBOARD_MATRIX_POSITIONS;

use super::{HidChannel, write_feature_report};

const REPORT_LEN: usize = 64;
const REPORT_ID: u8 = 0xcc;
const OP_CUSTOM_ANIMATION: u8 = 0x8c;
const OP_COMMIT: u8 = 0x8b;

const SUB_RESET: u8 = 0x10;
const SUB_SLOT_CONFIG: u8 = 0x01;
const SUB_ASSIGNMENT_0_59: u8 = 0x05;
const SUB_ASSIGNMENT_60_119: u8 = 0x06;
const SUB_ASSIGNMENT_120_139: u8 = 0x07;
const SUB_RGB_TABLE: u8 = 0x02;
const SUB_FINALIZE: u8 = 0x13;

const STATIC_SLOT: u8 = 0x01;
const ASSIGNMENT_ENTRIES_PER_REPORT: usize = 60;
const RGB_RECORDS_PER_REPORT: usize = 15;

/// Firmware effect slots cleared/reinitialized before configuring the static
/// slot. Matches a known-working independent implementation's
/// `CUSTOM_SLOTS`; the meaning of most of these beyond `STATIC_SLOT` (`0x01`)
/// is not fully documented (protocol spec §5.1), but clearing them removes a
/// source of stale cross-session state.
const CUSTOM_SLOTS_TO_CLEAR: &[u8] = &[0x01, 0x02, 0x05, 0x08, 0x09, 0x0e];

/// Gap held between successive feature-report writes in
/// `apply_custom_colours`. The firmware processes each custom-animation
/// report asynchronously (notably after `reset_report`); sending the next
/// one immediately has been observed to silently drop some writes on real
/// hardware. This is an experimentally-chosen value, not a documented
/// firmware requirement.
const INTER_REPORT_DELAY: Duration = Duration::from_millis(15);

fn write_custom_animation_report(
    device: &dyn HidChannel,
    report: &[u8; REPORT_LEN],
) -> Result<(), String> {
    write_feature_report(device, report)?;
    thread::sleep(INTER_REPORT_DELAY);
    Ok(())
}

/// Sends the full documented sequence (reset, slot clears, slot config,
/// assignment table, RGB table, finalize, commit) so that exactly `colours`
/// are lit and every other matrix position is unassigned. Callers own the
/// shadow state; this function only performs one complete apply from
/// whatever map it is given.
///
/// `colours` is keyed by the same matrix index carried in each `cc:8c:02`
/// RGB record's `<key_index>` byte (matching `KeyboardKeyMap`'s indices, and
/// the protocol spec's §8 key-index table). The assignment table's array
/// position for a given key is one less than that index. Confirmed against
/// a known-working independent implementation and directly against hardware
/// (an unshifted assignment table produces exactly the flaky-looking,
/// key-dependent behaviour seen while diagnosing this: a key only lit if some
/// other key at `index - 1` happened to already be in the shadow, since
/// each key's own (incorrectly unshifted) assignment entry would coincidentally
/// satisfy its successor's requirement).
pub fn apply_custom_colours(
    device: &dyn HidChannel,
    colours: &BTreeMap<u8, Rgb>,
) -> Result<(), String> {
    let assigned_positions = assignment_positions(colours.keys().copied())?;

    write_custom_animation_report(device, &reset_report())?;

    for slot in CUSTOM_SLOTS_TO_CLEAR {
        write_custom_animation_report(device, &slot_clear_report(*slot))?;
    }

    let placeholder = colours
        .values()
        .copied()
        .next()
        .unwrap_or(Rgb::new(0xff, 0xff, 0xff));
    write_custom_animation_report(device, &static_slot_config_report(placeholder))?;

    for report in assignment_table_reports(&assigned_positions) {
        write_custom_animation_report(device, &report)?;
    }

    let entries = colours
        .iter()
        .map(|(index, rgb)| (*index, *rgb))
        .collect::<Vec<_>>();
    for report in rgb_table_reports(&entries) {
        write_custom_animation_report(device, &report)?;
    }

    write_custom_animation_report(device, &finalize_report())?;
    write_feature_report(device, &commit_report())
}

fn assignment_positions(key_indices: impl IntoIterator<Item = u8>) -> Result<BTreeSet<u8>, String> {
    key_indices
        .into_iter()
        .map(|key_index| {
            key_index.checked_sub(1).ok_or_else(|| {
                "keyboard key index 0 has no corresponding assignment-table position".to_owned()
            })
        })
        .collect()
}

/// Lightweight follow-up write for one or more keys that are already in the
/// firmware's assignment table: just `cc:8c:02` RGB record(s), with none of
/// `apply_custom_colours`'s reset/slot-clear/assignment-table/finalize/commit
/// steps. Matches a known-working independent implementation's low-level
/// `upload_rgb_records` path, and (confirmed against hardware) does not
/// cause the whole-board flicker that a full `apply_custom_colours` does.
/// Only valid when every entry's assignment-table slot was already
/// programmed by a prior `apply_custom_colours` call; callers must track
/// that. Accepting a slice (rather than one key at a time) lets a batch of
/// already-assigned recolours share `rgb_table_reports`' 15-per-report
/// chunking instead of one write per key.
pub fn update_keys_colour(device: &dyn HidChannel, entries: &[(u8, Rgb)]) -> Result<(), String> {
    for report in rgb_table_reports(entries) {
        write_feature_report(device, &report)?;
    }
    Ok(())
}

fn header(subcommand: u8) -> [u8; REPORT_LEN] {
    let mut report = [0_u8; REPORT_LEN];
    report[0] = REPORT_ID;
    report[1] = OP_CUSTOM_ANIMATION;
    report[2] = subcommand;
    report
}

fn reset_report() -> [u8; REPORT_LEN] {
    header(SUB_RESET)
}

/// Clears/reinitializes one firmware effect slot: `cc 8c 01 <slot>` with the
/// rest of the report zero-filled, distinct from `static_slot_config_report`
/// which fully configures `STATIC_SLOT` for rendering.
fn slot_clear_report(slot: u8) -> [u8; REPORT_LEN] {
    let mut report = header(SUB_SLOT_CONFIG);
    report[3] = slot;
    report
}

fn static_slot_config_report(rgb: Rgb) -> [u8; REPORT_LEN] {
    let mut report = header(SUB_SLOT_CONFIG);
    report[3] = STATIC_SLOT;
    report[4] = 0x01; // timing selector A
    report[7] = 0x01; // direction: none
    report[8] = 0x01; // trigger: immediate/static
    report[9] = 0x01; // enabled
    report[10] = 0x00; // effect-specific flag
    report[11] = rgb.r;
    report[12] = rgb.g;
    report[13] = rgb.b;
    report[14] = rgb.r;
    report[15] = rgb.g;
    report[16] = rgb.b;
    report[17] = 0x01; // timing selector B
    report
}

fn assignment_table_reports(assigned_indices: &BTreeSet<u8>) -> [[u8; REPORT_LEN]; 3] {
    let subcommands = [
        SUB_ASSIGNMENT_0_59,
        SUB_ASSIGNMENT_60_119,
        SUB_ASSIGNMENT_120_139,
    ];
    let mut reports = [
        header(subcommands[0]),
        header(subcommands[1]),
        header(subcommands[2]),
    ];

    for (report_index, report) in reports.iter_mut().enumerate() {
        let base = report_index * ASSIGNMENT_ENTRIES_PER_REPORT;
        let remaining = usize::from(KEYBOARD_MATRIX_POSITIONS).saturating_sub(base);
        let count = ASSIGNMENT_ENTRIES_PER_REPORT.min(remaining);
        for offset in 0..count {
            let Ok(index) = u8::try_from(base + offset) else {
                continue;
            };
            if assigned_indices.contains(&index) {
                report[4 + offset] = STATIC_SLOT;
            }
        }
    }

    reports
}

fn rgb_table_reports(entries: &[(u8, Rgb)]) -> Vec<[u8; REPORT_LEN]> {
    entries
        .chunks(RGB_RECORDS_PER_REPORT)
        .map(rgb_table_report)
        .collect()
}

fn rgb_table_report(records: &[(u8, Rgb)]) -> [u8; REPORT_LEN] {
    let mut report = header(SUB_RGB_TABLE);
    for (record_index, (index, rgb)) in records.iter().enumerate() {
        let base = 4 + (record_index * 4);
        report[base] = *index;
        report[base + 1] = rgb.r;
        report[base + 2] = rgb.g;
        report[base + 3] = rgb.b;
    }
    report
}

fn finalize_report() -> [u8; REPORT_LEN] {
    header(SUB_FINALIZE)
}

fn commit_report() -> [u8; REPORT_LEN] {
    let mut report = [0_u8; REPORT_LEN];
    report[0] = REPORT_ID;
    report[1] = OP_COMMIT;
    report[2] = 0x01;
    report[3] = 0xff;
    report
}

#[cfg(test)]
#[path = "keyboard_custom_tests.rs"]
mod tests;
