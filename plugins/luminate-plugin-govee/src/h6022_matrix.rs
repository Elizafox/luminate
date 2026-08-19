// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Static H6022 matrix images encoded through Govee's DIY/graffiti protocol.

use std::collections::BTreeMap;

use luminate_core::rgb::Rgb;

use crate::ptreal::{self, Frame};

pub(crate) const ROWS: u16 = 11;
pub(crate) const COLS: u16 = 12;
pub(crate) const CELL_COUNT: usize = ROWS as usize * COLS as usize;

const CHUNK_LENGTH: usize = 17;
const MAX_FIELD_LENGTH: usize = u16::MAX as usize;
const MAX_GROUP_COUNT: usize = u8::MAX as usize;
const MAX_GROUP_CELLS: usize = u8::MAX as usize;
#[cfg(test)]
const MAX_SAFE_UDP_PAYLOAD: usize = 1_400;
const BRIGHTNESS: u8 = 100;
const ACTION_DOWN: u8 = 2;

/// Encodes one complete, static 12×11 framebuffer and its DIY commit frame.
pub(crate) fn encode(framebuffer: &[Rgb]) -> Result<Vec<Frame>, String> {
    let payload = encode_payload(framebuffer)?;
    let mut frames = chunk_payload(&payload)?;
    frames.push(ptreal::diy_commit_frame()?);
    Ok(frames)
}

fn encode_payload(framebuffer: &[Rgb]) -> Result<Vec<u8>, String> {
    if framebuffer.len() != CELL_COUNT {
        return Err(format!(
            "H6022 matrix framebuffer has {} cells; expected {CELL_COUNT}",
            framebuffer.len()
        ));
    }

    let background = choose_background(framebuffer);
    let mut groups = Vec::<(Rgb, Vec<u8>)>::new();
    for (index, colour) in framebuffer.iter().enumerate() {
        if *colour == background {
            continue;
        }
        let index = u8::try_from(index)
            .map_err(|_| format!("H6022 matrix cell index {index} does not fit in one byte"))?;
        if let Some((_colour, indices)) = groups
            .iter_mut()
            .find(|(group_colour, _indices)| group_colour == colour)
        {
            indices.push(index);
        } else {
            groups.push((*colour, vec![index]));
        }
    }
    if groups.len() > MAX_GROUP_COUNT {
        return Err(format!(
            "H6022 matrix image has {} colour groups; protocol supports at most {MAX_GROUP_COUNT}",
            groups.len()
        ));
    }

    let group_bytes = groups
        .iter()
        .try_fold(0_usize, |total, (_colour, indices)| {
        if indices.len() > MAX_GROUP_CELLS {
            return Err(format!(
                "H6022 matrix colour group has {} cells; protocol supports at most {MAX_GROUP_CELLS}",
                indices.len()
            ));
        }
        total
            .checked_add(4 + indices.len())
            .ok_or_else(|| "H6022 matrix group length overflowed".to_owned())
    })?;
    let group_section_length = group_bytes
        .checked_add(1)
        .ok_or_else(|| "H6022 matrix group-section length overflowed".to_owned())?;
    let layer_length = group_bytes
        .checked_add(15)
        .ok_or_else(|| "H6022 matrix layer length overflowed".to_owned())?;
    let group_section_length = u16::try_from(group_section_length).map_err(|_| {
        format!("H6022 matrix group section exceeds the {MAX_FIELD_LENGTH}-byte protocol limit")
    })?;
    let layer_length = u16::try_from(layer_length).map_err(|_| {
        format!("H6022 matrix layer exceeds the {MAX_FIELD_LENGTH}-byte protocol limit")
    })?;

    let payload_length = 16_usize
        .checked_add(group_bytes)
        .and_then(|length| length.checked_add(7))
        .ok_or_else(|| "H6022 matrix payload length overflowed".to_owned())?;
    let packet_count = payload_length.div_ceil(CHUNK_LENGTH);
    let packet_count = u8::try_from(packet_count)
        .map_err(|_| "H6022 matrix upload needs more than 255 packets".to_owned())?;

    let mut payload = Vec::with_capacity(payload_length);
    payload.extend_from_slice(&[
        0x01,
        packet_count,
        0x58,
        0x5a,
        background.r,
        background.g,
        background.b,
        BRIGHTNESS,
        0x00,
        0x01,
    ]);
    payload.extend_from_slice(&layer_length.to_le_bytes());
    payload.push(0x03);
    payload.extend_from_slice(&group_section_length.to_le_bytes());
    payload.push(u8::try_from(groups.len()).map_err(|_| {
        format!(
            "H6022 matrix image has {} colour groups; protocol supports at most {MAX_GROUP_COUNT}",
            groups.len()
        )
    })?);

    for (colour, indices) in groups {
        payload.push(u8::try_from(indices.len()).map_err(|_| {
            format!(
                "H6022 matrix colour group has {} cells; protocol supports at most {MAX_GROUP_CELLS}",
                indices.len()
            )
        })?);
        payload.extend_from_slice(&[colour.r, colour.g, colour.b]);
        payload.extend_from_slice(&indices);
    }

    // Speed zero is the hardware-verified static-image control. Duration zero
    // does not stop animation, so retain the firmware's "forever" sentinel.
    payload.extend_from_slice(&[0x00, BRIGHTNESS, ACTION_DOWN, 0x00, 0x01, 0xff, 0xff]);
    Ok(payload)
}

fn choose_background(framebuffer: &[Rgb]) -> Rgb {
    let mut counts = BTreeMap::<(u8, u8, u8), usize>::new();
    for colour in framebuffer {
        *counts.entry((colour.r, colour.g, colour.b)).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|(left_colour, left_count), (right_colour, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| right_colour.cmp(left_colour))
        })
        .map_or(Rgb::BLACK, |((red, green, blue), _count)| {
            Rgb::new(red, green, blue)
        })
}

fn chunk_payload(payload: &[u8]) -> Result<Vec<Frame>, String> {
    let packet_count = payload.len().div_ceil(CHUNK_LENGTH);
    if packet_count == 0 || packet_count > u8::MAX as usize {
        return Err(format!(
            "H6022 matrix upload needs {packet_count} packets; protocol supports 1..=255"
        ));
    }

    payload
        .chunks(CHUNK_LENGTH)
        .enumerate()
        .map(|(index, chunk)| {
            let sequence = if index + 1 == packet_count {
                0xff
            } else {
                u8::try_from(index)
                    .map_err(|_| format!("H6022 matrix packet index {index} is unrepresentable"))?
            };
            ptreal::multipart_frame(sequence, chunk)
        })
        .collect()
}

#[cfg(test)]
#[path = "h6022_matrix_tests.rs"]
mod tests;
