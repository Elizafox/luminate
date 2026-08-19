// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Per-SKU capability profiles.
//!
//! Govee's LAN API has no analogue of WLED's `/json/info` self-description:
//! a device reports its SKU string but not what it can do with it. Matrix
//! capabilities, layouts, and scene support are all per-model facts that
//! have to be captured by hand from a real device, so this is a static
//! table rather than something discovered live. Unknown SKUs fall back to a
//! conservative profile (RGB colour and brightness only) rather than
//! guessing at matrix/scene support that would fail against the real
//! hardware.

use std::ptr;

/// What one Govee LAN device model is known to support.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SkuProfile {
    pub(crate) model_name: &'static str,
    pub(crate) supports_cct: bool,
    /// Native CCT range, when verified for this SKU. Values outside this
    /// range are sent as an RGB approximation by the plugin.
    pub(crate) native_cct_range: Option<(u32, u32)>,
    pub(crate) supports_scenes: bool,
    /// Firmware-native scenes: `(code, name)`, keyed by a kebab-case slug of
    /// `name` when advertised as a `HardwareEffectDescriptor` id.
    pub(crate) scenes: &'static [(u8, &'static str)],
    /// `(rows, columns)` for matrix-style devices, if known.
    pub(crate) matrix: Option<(u16, u16)>,
}

/// Confirmed firmware scene codes for the H6022, cross-checked between
/// `h6022_firmware_effects.json`, `h6022_scene_effects_full.json`'s
/// `sceneType: 0` entries (a plain byte code, no upload payload needed), and
/// the owned lamp.
///
/// The two Rainbow codes are genuinely distinct scenes rather than conflicting
/// sources: hardware testing showed `0x16` as a drawn-looking animation and
/// `0x2a` as a striped animation.
const H6022_SCENES: &[(u8, &str)] = &[
    (0x16, "Rainbow Drawing"),
    (0x23, "Reading"),
    (0x26, "Night Light"),
    (0x28, "Fire"),
    (0x29, "Snow flake"),
    (0x2a, "Rainbow Striped"),
    (0x2b, "Ocean"),
    (0x2c, "Forest"),
    (0x2d, "Joyful"),
    (0x2e, "Wave"),
    (0x2f, "Starry Sky"),
    (0x63, "White Light"),
];

const H6022: SkuProfile = SkuProfile {
    model_name: "H6022",
    supports_cct: true,
    native_cct_range: Some((2700, 6500)),
    supports_scenes: true,
    scenes: H6022_SCENES,
    matrix: Some((11, 12)),
};

/// Conservative profile for a SKU this plugin has not been taught about.
/// The LAN wire format accepts a `colorTemInKelvin` field on devices that may
/// ignore it. That establishes wire compatibility, not native CCT support, so
/// unknown devices use the daemon's RGB CCT emulation instead. Matrices and
/// scenes also stay off because their model-specific contracts are unknown.
const FALLBACK: SkuProfile = SkuProfile {
    model_name: "Unknown Govee LAN device",
    supports_cct: false,
    native_cct_range: None,
    supports_scenes: false,
    scenes: &[],
    matrix: None,
};

pub(crate) fn profile_for(sku: &str) -> &'static SkuProfile {
    match sku {
        "H6022" => &H6022,
        _ => &FALLBACK,
    }
}

/// Whether `sku` has a device-specific profile.
pub(crate) fn is_known(sku: &str) -> bool {
    !ptr::eq(profile_for(sku), &FALLBACK)
}

#[cfg(test)]
#[path = "sku_tests.rs"]
mod tests;
