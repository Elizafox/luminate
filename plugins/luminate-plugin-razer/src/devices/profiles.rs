// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Validated exact-product profile registry.

use std::error::Error;
use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "validation states are part of the registry schema before all states have profiles"
)]
pub(crate) enum ValidationStatus {
    Untested,
    PartiallyValidated,
    Validated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Profile {
    pub(crate) slug: &'static str,
    pub(crate) name: &'static str,
    pub(crate) vendor_id: u16,
    pub(crate) product_id: u16,
    pub(crate) control_interface: i32,
    pub(crate) transaction_id: u8,
    pub(crate) response_delay: Duration,
    pub(crate) rows: u8,
    pub(crate) columns: u8,
    pub(crate) supports_wheel: bool,
    pub(crate) supports_starlight: bool,
    pub(crate) custom_frame_response: bool,
    pub(crate) validation: ValidationStatus,
}

impl Profile {
    pub(crate) const fn validate(self) -> Result<(), ProfileError> {
        if self.vendor_id == 0 || self.product_id == 0 {
            return Err(ProfileError::MissingUsbIdentity);
        }
        if self.control_interface < 0 {
            return Err(ProfileError::InvalidControlInterface);
        }
        if self.transaction_id == 0 {
            return Err(ProfileError::MissingTransactionId);
        }
        if self.rows == 0 || self.columns == 0 || self.columns > 25 {
            return Err(ProfileError::InvalidMatrix);
        }
        Ok(())
    }
}

pub(crate) const BLACKWIDOW_V4_PRO: Profile = Profile {
    slug: "blackwidow-v4-pro",
    name: "Razer BlackWidow V4 Pro",
    vendor_id: 0x1532,
    product_id: 0x028d,
    control_interface: 3,
    transaction_id: 0x1f,
    response_delay: Duration::from_micros(600),
    rows: 8,
    columns: 23,
    supports_wheel: true,
    supports_starlight: true,
    custom_frame_response: false,
    validation: ValidationStatus::PartiallyValidated,
};

pub(crate) const BLACKWIDOW_V4: Profile = Profile {
    slug: "blackwidow-v4",
    name: "Razer BlackWidow V4",
    vendor_id: 0x1532,
    product_id: 0x0287,
    control_interface: 3,
    transaction_id: 0x1f,
    response_delay: Duration::from_micros(600),
    rows: 8,
    columns: 23,
    supports_wheel: true,
    supports_starlight: true,
    custom_frame_response: false,
    validation: ValidationStatus::Untested,
};

pub(crate) const BLACKWIDOW_V4_75: Profile = Profile {
    slug: "blackwidow-v4-75",
    name: "Razer BlackWidow V4 75%",
    vendor_id: 0x1532,
    product_id: 0x02a5,
    control_interface: 3,
    transaction_id: 0x1f,
    response_delay: Duration::from_micros(600),
    rows: 6,
    columns: 16,
    supports_wheel: true,
    supports_starlight: true,
    custom_frame_response: false,
    validation: ValidationStatus::Untested,
};

pub(crate) const BLACKWIDOW_V4_MINI_HYPERSPEED_WIRED: Profile = Profile {
    slug: "blackwidow-v4-mini-hyperspeed-wired",
    name: "Razer BlackWidow V4 Mini HyperSpeed (Wired)",
    vendor_id: 0x1532,
    product_id: 0x02b9,
    control_interface: 3,
    transaction_id: 0x1f,
    response_delay: Duration::from_micros(600),
    rows: 5,
    columns: 14,
    supports_wheel: false,
    supports_starlight: true,
    custom_frame_response: true,
    validation: ValidationStatus::Untested,
};

pub(crate) const BLACKWIDOW_V4_TKL_HYPERSPEED_WIRED: Profile = Profile {
    slug: "blackwidow-v4-tenkeyless-hyperspeed-wired",
    name: "Razer BlackWidow V4 Tenkeyless HyperSpeed (Wired)",
    vendor_id: 0x1532,
    product_id: 0x02d7,
    control_interface: 3,
    transaction_id: 0x1f,
    response_delay: Duration::from_micros(600),
    rows: 6,
    columns: 18,
    supports_wheel: false,
    supports_starlight: true,
    custom_frame_response: true,
    validation: ValidationStatus::Untested,
};

const fn untested_keyboard(
    slug: &'static str,
    name: &'static str,
    product_id: u16,
    transaction_id: u8,
    rows: u8,
    columns: u8,
    supports_starlight: bool,
) -> Profile {
    Profile {
        slug,
        name,
        vendor_id: 0x1532,
        product_id,
        control_interface: 3,
        transaction_id,
        response_delay: Duration::from_micros(600),
        rows,
        columns,
        supports_wheel: false,
        supports_starlight,
        custom_frame_response: true,
        validation: ValidationStatus::Untested,
    }
}

pub(crate) const BLACKWIDOW_V3_MINI_HYPERSPEED_WIRED: Profile = untested_keyboard(
    "blackwidow-v3-mini-hyperspeed-wired",
    "Razer BlackWidow V3 Mini HyperSpeed (Wired)",
    0x0258,
    0x1f,
    5,
    16,
    true,
);

pub(crate) const HUNTSMAN_MINI_ANALOG: Profile = untested_keyboard(
    "huntsman-mini-analog",
    "Razer Huntsman Mini Analog",
    0x0282,
    0x1f,
    5,
    15,
    false,
);

pub(crate) const HUNTSMAN_V2_ANALOG: Profile = untested_keyboard(
    "huntsman-v2-analog",
    "Razer Huntsman V2 Analog",
    0x0266,
    0x1f,
    9,
    22,
    false,
);

pub(crate) const HUNTSMAN_V2_TKL: Profile = untested_keyboard(
    "huntsman-v2-tenkeyless",
    "Razer Huntsman V2 Tenkeyless",
    0x026b,
    0x1f,
    6,
    17,
    true,
);

pub(crate) const HUNTSMAN_V2: Profile = untested_keyboard(
    "huntsman-v2",
    "Razer Huntsman V2",
    0x026c,
    0x1f,
    6,
    22,
    true,
);

pub(crate) const HUNTSMAN_V3_PRO: Profile = untested_keyboard(
    "huntsman-v3-pro",
    "Razer Huntsman V3 Pro",
    0x02a6,
    0x1f,
    6,
    22,
    true,
);

pub(crate) const HUNTSMAN_V3_PRO_TKL: Profile = untested_keyboard(
    "huntsman-v3-pro-tenkeyless",
    "Razer Huntsman V3 Pro Tenkeyless",
    0x02a7,
    0x1f,
    6,
    19,
    true,
);

pub(crate) const DEATHSTALKER_V2: Profile = untested_keyboard(
    "deathstalker-v2",
    "Razer DeathStalker V2",
    0x0295,
    0x1f,
    6,
    22,
    true,
);

pub(crate) const DEATHSTALKER_V2_PRO_WIRED: Profile = untested_keyboard(
    "deathstalker-v2-pro-wired",
    "Razer DeathStalker V2 Pro (Wired)",
    0x0292,
    0x1f,
    6,
    22,
    true,
);

pub(crate) const DEATHSTALKER_V2_PRO_TKL_WIRED: Profile = untested_keyboard(
    "deathstalker-v2-pro-tenkeyless-wired",
    "Razer DeathStalker V2 Pro Tenkeyless (Wired)",
    0x0298,
    0x1f,
    6,
    17,
    true,
);

pub(crate) static PROFILES: &[Profile] = &[
    BLACKWIDOW_V4_PRO,
    BLACKWIDOW_V4,
    BLACKWIDOW_V4_75,
    BLACKWIDOW_V4_MINI_HYPERSPEED_WIRED,
    BLACKWIDOW_V4_TKL_HYPERSPEED_WIRED,
    BLACKWIDOW_V3_MINI_HYPERSPEED_WIRED,
    HUNTSMAN_MINI_ANALOG,
    HUNTSMAN_V2_ANALOG,
    HUNTSMAN_V2_TKL,
    HUNTSMAN_V2,
    HUNTSMAN_V3_PRO,
    HUNTSMAN_V3_PRO_TKL,
    DEATHSTALKER_V2,
    DEATHSTALKER_V2_PRO_WIRED,
    DEATHSTALKER_V2_PRO_TKL_WIRED,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileError {
    MissingUsbIdentity,
    InvalidControlInterface,
    MissingTransactionId,
    InvalidMatrix,
}

impl fmt::Display for ProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingUsbIdentity => {
                formatter.write_str("Razer profile has no exact USB identity")
            }
            Self::InvalidControlInterface => {
                formatter.write_str("Razer profile control interface is invalid")
            }
            Self::MissingTransactionId => {
                formatter.write_str("Razer profile transaction ID is zero")
            }
            Self::InvalidMatrix => {
                formatter.write_str("Razer profile matrix dimensions are invalid")
            }
        }
    }
}

impl Error for ProfileError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_profiles_are_internally_consistent_and_unique() {
        let profiles = PROFILES;
        for profile in profiles {
            profile.validate().expect("imported profile must be valid");
        }
        for (index, profile) in profiles.iter().enumerate() {
            assert!(
                profiles.iter().skip(index + 1).all(|other| {
                    (profile.vendor_id, profile.product_id) != (other.vendor_id, other.product_id)
                        && profile.slug != other.slug
                }),
                "Razer profile identities and slugs must be unique"
            );
        }

        let no_response = profiles
            .iter()
            .filter(|profile| !profile.custom_frame_response)
            .map(|profile| profile.slug)
            .collect::<Vec<_>>();
        assert_eq!(
            no_response,
            ["blackwidow-v4-pro", "blackwidow-v4", "blackwidow-v4-75"]
        );

        assert!(
            profiles
                .iter()
                .all(|profile| profile.transaction_id & 0x07 == 0x07)
        );
    }
}
