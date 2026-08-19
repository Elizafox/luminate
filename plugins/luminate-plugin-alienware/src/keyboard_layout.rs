// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Alienware keyboard identities and their addressable key layouts.

use std::sync;

use std::collections::BTreeSet;

pub const KEYBOARD_MATRIX_POSITIONS: u8 = 140;

const M16_R2_US_ANSI_RESERVED_INDICES: &[u8] = &[0x22, 0x37, 0x3c, 0x4a, 0x53, 0x6a, 0x6f];

/// Canonical lower-bank name/index pairs for the tested M16 R2 US-ANSI unit
/// (protocol spec §8). This table is specific to `KeyboardLayoutId::M16R2UsAnsi`
/// and lives only here; nothing outside this module should assume US-ANSI or
/// any particular index. Callers go through `KeyboardKeyMap::index_for_name`/
/// `named_positions` instead. Adding another layout means adding its own table
/// and a new `KeyboardLayoutId` variant, not touching this one.
const M16_R2_US_ANSI_KEY_NAMES: &[(&str, u8)] = &[
    ("escape", 0x01),
    ("f1", 0x02),
    ("f2", 0x03),
    ("f3", 0x04),
    ("f4", 0x05),
    ("f5", 0x06),
    ("f6", 0x07),
    ("f7", 0x08),
    ("f8", 0x09),
    ("f9", 0x0a),
    ("f10", 0x0b),
    ("f11", 0x0c),
    ("f12", 0x0d),
    ("home", 0x0e),
    ("end", 0x0f),
    ("delete", 0x10),
    ("mic-mute", 0x14),
    ("grave", 0x15),
    ("1", 0x16),
    ("2", 0x17),
    ("3", 0x18),
    ("4", 0x19),
    ("5", 0x1a),
    ("6", 0x1b),
    ("7", 0x1c),
    ("8", 0x1d),
    ("9", 0x1e),
    ("0", 0x1f),
    ("minus", 0x20),
    ("equals", 0x21),
    ("backspace", 0x24),
    ("volume-mute", 0x11),
    ("tab", 0x29),
    ("q", 0x2b),
    ("w", 0x2c),
    ("e", 0x2d),
    ("r", 0x2e),
    ("t", 0x2f),
    ("y", 0x30),
    ("u", 0x31),
    ("i", 0x32),
    ("o", 0x33),
    ("p", 0x34),
    ("left-bracket", 0x35),
    ("right-bracket", 0x36),
    ("backslash", 0x38),
    ("volume-down", 0x12),
    ("volume-up", 0x13),
    ("caps-lock", 0x3e),
    ("a", 0x3f),
    ("s", 0x40),
    ("d", 0x41),
    ("f", 0x42),
    ("g", 0x43),
    ("h", 0x44),
    ("j", 0x45),
    ("k", 0x46),
    ("l", 0x47),
    ("semicolon", 0x48),
    ("apostrophe", 0x49),
    ("enter", 0x4b),
    ("left-shift", 0x52),
    ("z", 0x54),
    ("x", 0x55),
    ("c", 0x56),
    ("v", 0x57),
    ("b", 0x58),
    ("n", 0x59),
    ("m", 0x5a),
    ("comma", 0x5b),
    ("period", 0x5c),
    ("slash", 0x5d),
    ("right-shift", 0x5f),
    ("up", 0x73),
    ("left-ctrl", 0x65),
    ("fn", 0x66),
    ("left-win", 0x68),
    ("left-alt", 0x69),
    ("space", 0x6c),
    ("right-win", 0x6e),
    ("right-alt", 0x70),
    ("right-ctrl", 0x71),
    ("left", 0x86),
    ("down", 0x87),
    ("right", 0x88),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardLayoutId {
    M16R2UsAnsi,
    Unknown {
        hardware_variant: [u8; 2],
        layout: u8,
        chassis_colour: u8,
    },
}

impl KeyboardLayoutId {
    pub const fn name(self) -> &'static str {
        match self {
            Self::M16R2UsAnsi => "m16-r2-us-ansi",
            Self::Unknown { .. } => "unknown",
        }
    }

    pub fn key_map(self) -> Option<&'static KeyboardKeyMap> {
        match self {
            Self::M16R2UsAnsi => Some(&M16_R2_US_ANSI_KEY_MAP),
            Self::Unknown { .. } => None,
        }
    }

    pub const fn physical_tags(self) -> &'static [&'static str] {
        match self {
            Self::M16R2UsAnsi => &["layout:us-ansi"],
            Self::Unknown { .. } => &[],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyboardIdentity {
    pub hardware_variant: [u8; 2],
    pub layout: u8,
    pub chassis_colour: u8,
}

/// `cc:93` identity captured live from the tested M16 R2 US-ANSI unit on
/// 2026-07-11 (`alienware keyboard layout identity read` trace line).
const M16_R2_US_ANSI_IDENTITY: KeyboardIdentity = KeyboardIdentity {
    hardware_variant: [0x17, 0x11],
    layout: 0x21,
    chassis_colour: 0x00,
};

impl KeyboardIdentity {
    pub const fn layout_id(self) -> KeyboardLayoutId {
        if self.hardware_variant[0] == M16_R2_US_ANSI_IDENTITY.hardware_variant[0]
            && self.hardware_variant[1] == M16_R2_US_ANSI_IDENTITY.hardware_variant[1]
            && self.layout == M16_R2_US_ANSI_IDENTITY.layout
            && self.chassis_colour == M16_R2_US_ANSI_IDENTITY.chassis_colour
        {
            return KeyboardLayoutId::M16R2UsAnsi;
        }

        KeyboardLayoutId::Unknown {
            hardware_variant: self.hardware_variant,
            layout: self.layout,
            chassis_colour: self.chassis_colour,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct KeyPosition {
    pub matrix_index: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPresenceMask {
    present_indices: BTreeSet<u8>,
}

impl KeyPresenceMask {
    pub fn from_reserved_indices(reserved_indices: &[u8]) -> Self {
        let reserved_indices = reserved_indices.iter().copied().collect::<BTreeSet<_>>();
        let present_indices = (0..KEYBOARD_MATRIX_POSITIONS)
            .filter(|index| !reserved_indices.contains(index))
            .collect();
        Self { present_indices }
    }

    pub fn positions(&self) -> impl Iterator<Item = KeyPosition> + '_ {
        self.present_indices
            .iter()
            .copied()
            .map(|matrix_index| KeyPosition { matrix_index })
    }

    pub fn len(&self) -> usize {
        self.present_indices.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardKeyMap {
    pub id: KeyboardLayoutId,
    pub presence: KeyPresenceMask,
    named_positions: &'static [(&'static str, u8)],
}

impl KeyboardKeyMap {
    pub fn position_indices(&self) -> impl Iterator<Item = u8> + '_ {
        self.presence
            .positions()
            .map(|position| position.matrix_index)
    }

    /// Named physical keys for this layout, as `(name, matrix_index)` pairs.
    /// Empty for layouts that have a presence mask but no captured name table.
    pub fn named_positions(&self) -> impl Iterator<Item = (&'static str, u8)> + '_ {
        self.named_positions.iter().copied()
    }

    pub fn index_for_name(&self, name: &str) -> Option<u8> {
        self.named_positions
            .iter()
            .find_map(|(candidate, index)| (*candidate == name).then_some(*index))
    }
}

static M16_R2_US_ANSI_KEY_MAP: sync::LazyLock<KeyboardKeyMap> =
    sync::LazyLock::new(|| KeyboardKeyMap {
        id: KeyboardLayoutId::M16R2UsAnsi,
        presence: KeyPresenceMask::from_reserved_indices(M16_R2_US_ANSI_RESERVED_INDICES),
        named_positions: M16_R2_US_ANSI_KEY_NAMES,
    });

pub fn parse_identity_report(report: &[u8]) -> Option<KeyboardIdentity> {
    let hardware_variant = [*report.get(2)?, *report.get(3)?];
    let layout = *report.get(4)?;
    let chassis_colour = *report.get(5)?;
    Some(KeyboardIdentity {
        hardware_variant,
        layout,
        chassis_colour,
    })
}

#[cfg(test)]
#[path = "keyboard_layout_tests.rs"]
mod tests;
