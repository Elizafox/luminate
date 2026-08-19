// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Portable and hardware-defined lighting effects and their arguments.

use serde::{Deserialize, Serialize};

use crate::capability::EffectDirection;
use crate::capability::HardwareEffectId;
use crate::colour::Colour;
use crate::rgb::Rgb;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Portable effects plus plugin-advertised hardware effects.
pub enum Effect {
    /// No light output.
    Off,

    /// A fixed colour.
    Static {
        /// Primary effect colour.
        colour: Colour,
    },

    /// A smooth fade in and out.
    Breathe {
        /// Primary effect colour.
        colour: Rgb,

        /// Animation period in milliseconds.
        period_ms: u32,
    },

    /// A repeating pulse.
    Pulse {
        /// Primary effect colour.
        colour: Rgb,

        /// Animation period in milliseconds.
        period_ms: u32,
    },

    /// A rapid, sharply-timed on/off flash, distinct from [`Effect::Pulse`]'s
    /// smooth 50% duty cycle.
    Strobe {
        /// Primary effect colour.
        colour: Rgb,

        /// Animation period in milliseconds.
        period_ms: u32,
    },

    /// A moving scanner animation.
    Scanner {
        /// Primary effect colour.
        colour: Rgb,

        /// Animation period in milliseconds.
        period_ms: u32,
    },

    /// A transition through several colours.
    Morph {
        /// Ordered effect colours.
        colours: Vec<Rgb>,

        /// Animation period in milliseconds.
        period_ms: u32,
    },

    /// A repeating spectrum animation.
    Spectrum {
        /// Animation period in milliseconds.
        period_ms: u32,
    },

    /// A spatial rainbow animation.
    Rainbow {
        /// Animation period in milliseconds.
        period_ms: u32,
    },

    /// A vendor-specific effect advertised by a plugin through a
    /// `HardwareEffectDescriptor` and invoked by its `id`.
    ///
    /// This represents proprietary scene presets and animations that have no
    /// portable cross-device meaning. The daemon validates [`EffectArguments`]
    /// against the matched descriptor's declared parameters.
    Hardware {
        /// Advertised effect identifier.
        id: HardwareEffectId,

        /// Arguments validated against the advertised descriptor.
        arguments: EffectArguments,
    },
}

/// The generic argument values for an [`Effect::Hardware`] invocation, each
/// paired with the `EffectParameter` of the same shape in the target's
/// advertised descriptor. A field is `Some`/non-empty only when the descriptor
/// declares the corresponding parameter; the daemon rejects arguments that the
/// descriptor doesn't advertise or that fall outside its bounds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectArguments {
    /// Values for a `Colour` parameter, in order.
    pub colours: Vec<Rgb>,

    /// Value for a `Speed` parameter.
    pub speed: Option<u16>,

    /// Value for a `Direction` parameter.
    pub direction: Option<EffectDirection>,

    /// Value for a `Duration` parameter, in milliseconds.
    pub duration_ms: Option<u32>,

    /// Value for a `Brightness` parameter.
    pub brightness: Option<u32>,

    /// Selected option `id` for a `Choice` parameter.
    pub choice: Option<String>,
}

impl Effect {
    /// Canonical capability descriptor ID for this wire effect. For the typed
    /// variants this is a fixed well-known string; for [`Effect::Hardware`] it
    /// is the advertised descriptor `id` the caller invoked.
    #[must_use]
    pub fn capability_id(&self) -> &str {
        match self {
            Self::Off => "off",
            Self::Static { .. } => "static",
            Self::Breathe { .. } => "breathe",
            Self::Pulse { .. } => "pulse",
            Self::Strobe { .. } => "strobe",
            Self::Scanner { .. } => "scanner",
            Self::Morph { .. } => "morph",
            Self::Spectrum { .. } => "spectrum",
            Self::Rainbow { .. } => "rainbow",
            Self::Hardware { id, .. } => id.as_str(),
        }
    }
}

#[cfg(test)]
#[path = "effect_tests.rs"]
mod tests;
