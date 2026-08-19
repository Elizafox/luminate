// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Known AW-ELC controller identities and their profile-owned lighting zones.

use std::fmt;

pub(crate) const AW_ELC_VENDOR_ID: u16 = 0x187c;
pub(crate) const AW_ELC_LEGACY_PRODUCT_ID: u16 = 0x0550;
pub(crate) const AW_ELC_M16_R2_PRODUCT_ID: u16 = 0x0551;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValidationStatus {
    HardwareValidated,
    OpenRgbCorroborated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ZoneOperationClass {
    Live,
    PowerButton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AwElcZone {
    pub(crate) surface_id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) firmware_id: u8,
    pub(crate) form: &'static str,
    pub(crate) shape: &'static str,
    pub(crate) operation_class: ZoneOperationClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AwElcProfile {
    pub(crate) platform_id: Option<u16>,
    pub(crate) product_id: u16,
    pub(crate) model: &'static str,
    pub(crate) validation: ValidationStatus,
    pub(crate) expected_raw_zone_count: u8,
    pub(crate) zones: &'static [AwElcZone],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AwElcIdentity {
    pub(crate) vendor_id: u16,
    pub(crate) product_id: u16,
    pub(crate) platform_id: u16,
    pub(crate) reported_zone_count: u8,
    pub(crate) profile: &'static AwElcProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileResolutionError {
    UnsupportedController {
        vendor_id: u16,
        product_id: u16,
    },
    UnknownPlatform {
        product_id: u16,
        platform_id: u16,
    },
    ZoneCountMismatch {
        platform_id: u16,
        expected: u8,
        reported: u8,
    },
}

impl fmt::Display for ProfileResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedController {
                vendor_id,
                product_id,
            } => write!(
                formatter,
                "unsupported AW-ELC controller {vendor_id:04x}:{product_id:04x}"
            ),
            Self::UnknownPlatform {
                product_id,
                platform_id,
            } => write!(
                formatter,
                "unknown AW-ELC platform {platform_id:#06x} for product {product_id:04x}"
            ),
            Self::ZoneCountMismatch {
                platform_id,
                expected,
                reported,
            } => write!(
                formatter,
                "AW-ELC platform {platform_id:#06x} reported {reported} zones; expected {expected}"
            ),
        }
    }
}

const fn live_zone(
    surface_id: &'static str,
    name: &'static str,
    firmware_id: u8,
    form: &'static str,
    shape: &'static str,
) -> AwElcZone {
    AwElcZone {
        surface_id,
        name,
        firmware_id,
        form,
        shape,
        operation_class: ZoneOperationClass::Live,
    }
}

const FOUR_ZONE_KEYBOARD: &[AwElcZone] = &[
    live_zone("left", "Left", 0x00, "laptop", "keyboard-zone"),
    live_zone("middle", "Middle", 0x01, "laptop", "keyboard-zone"),
    live_zone("right", "Right", 0x02, "laptop", "keyboard-zone"),
    live_zone("numpad", "Numpad", 0x03, "laptop", "keyboard-zone"),
];

const G7_15_7500_ZONES: &[AwElcZone] = &[
    live_zone("left", "Left", 0x00, "laptop", "keyboard-zone"),
    live_zone(
        "centre-left",
        "Centre Left",
        0x01,
        "laptop",
        "keyboard-zone",
    ),
    live_zone(
        "centre-right",
        "Centre Right",
        0x02,
        "laptop",
        "keyboard-zone",
    ),
    live_zone("right", "Right", 0x03, "laptop", "keyboard-zone"),
    live_zone(
        "light-bar-1",
        "Light Bar 1",
        0x04,
        "laptop-light-bar",
        "segment",
    ),
    live_zone(
        "light-bar-2",
        "Light Bar 2",
        0x05,
        "laptop-light-bar",
        "segment",
    ),
    live_zone(
        "light-bar-3",
        "Light Bar 3",
        0x06,
        "laptop-light-bar",
        "segment",
    ),
    live_zone(
        "light-bar-4",
        "Light Bar 4",
        0x07,
        "laptop-light-bar",
        "segment",
    ),
    live_zone(
        "light-bar-5",
        "Light Bar 5",
        0x08,
        "laptop-light-bar",
        "segment",
    ),
    live_zone(
        "light-bar-6",
        "Light Bar 6",
        0x09,
        "laptop-light-bar",
        "segment",
    ),
    live_zone(
        "light-bar-7",
        "Light Bar 7",
        0x0a,
        "laptop-light-bar",
        "segment",
    ),
    live_zone(
        "light-bar-8",
        "Light Bar 8",
        0x0b,
        "laptop-light-bar",
        "segment",
    ),
    live_zone(
        "light-bar-9",
        "Light Bar 9",
        0x0c,
        "laptop-light-bar",
        "segment",
    ),
    live_zone(
        "light-bar-10",
        "Light Bar 10",
        0x0d,
        "laptop-light-bar",
        "segment",
    ),
    live_zone(
        "light-bar-11",
        "Light Bar 11",
        0x0e,
        "laptop-light-bar",
        "segment",
    ),
    live_zone(
        "light-bar-12",
        "Light Bar 12",
        0x0f,
        "laptop-light-bar",
        "segment",
    ),
];

const M16_R2_ZONES: &[AwElcZone] = &[
    live_zone(
        "trackpad-ring",
        "Trackpad Ring",
        0x00,
        "laptop",
        "trackpad-ring",
    ),
    live_zone("rear-logo", "Rear Logo", 0x02, "laptop", "logo"),
    AwElcZone {
        surface_id: "power-button",
        name: "Power Button",
        firmware_id: 0x04,
        form: "laptop",
        shape: "logo",
        operation_class: ZoneOperationClass::PowerButton,
    },
];

pub(crate) static M16_R2: AwElcProfile = AwElcProfile {
    platform_id: Some(0x1102),
    product_id: AW_ELC_M16_R2_PRODUCT_ID,
    model: "Alienware m16 R2",
    validation: ValidationStatus::HardwareValidated,
    // The firmware reports five zones. Only the three independently validated
    // zone IDs are published; IDs 0x01 and 0x03 remain intentionally opaque.
    expected_raw_zone_count: 5,
    zones: M16_R2_ZONES,
};

pub(crate) static PROFILES: &[AwElcProfile] = &[
    AwElcProfile {
        platform_id: Some(0x0c01),
        product_id: AW_ELC_LEGACY_PRODUCT_ID,
        model: "Dell G5 SE 5505",
        validation: ValidationStatus::OpenRgbCorroborated,
        expected_raw_zone_count: 4,
        zones: FOUR_ZONE_KEYBOARD,
    },
    AwElcProfile {
        platform_id: Some(0x0a01),
        product_id: AW_ELC_LEGACY_PRODUCT_ID,
        model: "Dell G7 15 7500",
        validation: ValidationStatus::OpenRgbCorroborated,
        expected_raw_zone_count: 16,
        zones: G7_15_7500_ZONES,
    },
    AwElcProfile {
        platform_id: Some(0x0e03),
        product_id: AW_ELC_LEGACY_PRODUCT_ID,
        model: "Dell G15 5511",
        validation: ValidationStatus::OpenRgbCorroborated,
        expected_raw_zone_count: 4,
        zones: FOUR_ZONE_KEYBOARD,
    },
    AwElcProfile {
        platform_id: Some(0x0e0a),
        product_id: AW_ELC_LEGACY_PRODUCT_ID,
        model: "Dell G15 5530",
        validation: ValidationStatus::OpenRgbCorroborated,
        expected_raw_zone_count: 4,
        zones: FOUR_ZONE_KEYBOARD,
    },
];

pub(crate) fn resolve(
    vendor_id: u16,
    product_id: u16,
    platform_id: u16,
    reported_zone_count: u8,
) -> Result<AwElcIdentity, ProfileResolutionError> {
    if vendor_id != AW_ELC_VENDOR_ID
        || !matches!(
            product_id,
            AW_ELC_LEGACY_PRODUCT_ID | AW_ELC_M16_R2_PRODUCT_ID
        )
    {
        return Err(ProfileResolutionError::UnsupportedController {
            vendor_id,
            product_id,
        });
    }

    let profile = PROFILES
        .iter()
        .chain([&M16_R2])
        .find(|profile| {
            profile.product_id == product_id && profile.platform_id == Some(platform_id)
        })
        .ok_or(ProfileResolutionError::UnknownPlatform {
            product_id,
            platform_id,
        })?;
    let expected = profile.expected_raw_zone_count;
    if reported_zone_count != expected {
        return Err(ProfileResolutionError::ZoneCountMismatch {
            platform_id,
            expected,
            reported: reported_zone_count,
        });
    }

    Ok(AwElcIdentity {
        vendor_id,
        product_id,
        platform_id,
        reported_zone_count,
        profile,
    })
}

#[cfg(test)]
#[path = "aw_elc_profile_tests.rs"]
mod tests;
