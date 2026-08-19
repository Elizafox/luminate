// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Daemon-managed transition inputs, identifiers, and status snapshots.

use std::time::Duration;

use palette::convert::FromColorUnclamped as _;
use palette::{Clamp as _, FromColor as _, Okhsv, Oklab, Srgb};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::capability::ColourChannel;
use crate::colour::{Colour, ColourChannelValue};
use crate::effect::{Effect, EffectArguments};
use crate::rgb::Rgb;
use crate::scene::SceneTargetState;
use crate::target::TargetId;
use crate::util::DiscreteRange;
use crate::util::declare_opaque_id;

/// Default transition step interval, approximately 30 Hz.
pub const DEFAULT_STEP_INTERVAL: Duration = Duration::from_millis(33);

/// Smallest accepted transition step interval.
pub const MINIMUM_STEP_INTERVAL: Duration = Duration::from_millis(10);

/// Opaque identifier minted by the daemon for one transition.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TransitionId(String);

declare_opaque_id!(
    TransitionId,
    "Wraps an existing transition identifier. Prefer [`Self::generate`] when \
     minting a fresh transition identifier."
);

impl TransitionId {
    /// Mints a fresh transition identifier.
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

/// Curve used to interpolate a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionFunction {
    /// Interpolate every numeric component at a constant rate.
    Linear,
    /// Begin gradually and accelerate cubically.
    EaseIn,
    /// Begin quickly and decelerate cubically.
    EaseOut,
    /// Accelerate and decelerate with smooth cubic endpoints.
    EaseInOut,
}

/// Direction used when encoded HSV or HSL hue crosses its circular boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HueDirection {
    /// Follow the shorter path. Exact half-turns increase.
    Shortest,
    /// Always increase the encoded hue, wrapping at its maximum.
    Increasing,
    /// Always decrease the encoded hue, wrapping at zero.
    Decreasing,
}

/// Colour space used for intermediate transition colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionColourInterpolation {
    /// Interpolate the target's encoded channels directly.
    Encoded {
        /// Circular path for HSV and HSL hue.
        hue_direction: HueDirection,
    },
    /// Interpolate assumed-sRGB colours through the perceptual `OKLab` space.
    Oklab,
}

impl Default for TransitionColourInterpolation {
    fn default() -> Self {
        Self::Encoded {
            hue_direction: HueDirection::Shortest,
        }
    }
}

/// Timing configuration for a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionOptions {
    duration_ms: u64,
    step_interval_ms: Option<u64>,
    /// Interpolation curve.
    pub function: TransitionFunction,
    /// Colour interpolation model.
    pub colour_interpolation: TransitionColourInterpolation,
}

impl TransitionOptions {
    /// Creates validated transition timing.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionValidationError`] for zero duration or a step
    /// interval shorter than 10 ms.
    pub fn new(
        duration: Duration,
        step_interval: Option<Duration>,
    ) -> Result<Self, TransitionValidationError> {
        if duration.is_zero() {
            return Err(TransitionValidationError::ZeroDuration);
        }
        if step_interval.is_some_and(|interval| interval < MINIMUM_STEP_INTERVAL) {
            return Err(TransitionValidationError::StepIntervalTooShort);
        }
        let duration_ms = u64::try_from(duration.as_millis())
            .map_err(|_| TransitionValidationError::DurationTooLong)?;
        let step_interval_ms = step_interval
            .map(|interval| u64::try_from(interval.as_millis()))
            .transpose()
            .map_err(|_| TransitionValidationError::DurationTooLong)?;
        if duration_ms == 0 || step_interval_ms == Some(0) {
            return Err(TransitionValidationError::SubMillisecondTiming);
        }
        Ok(Self {
            duration_ms,
            step_interval_ms,
            function: TransitionFunction::Linear,
            colour_interpolation: TransitionColourInterpolation::default(),
        })
    }

    /// Selects the time-easing function.
    #[must_use]
    pub const fn with_function(mut self, function: TransitionFunction) -> Self {
        self.function = function;
        self
    }

    /// Selects encoded or perceptual colour interpolation.
    #[must_use]
    pub const fn with_colour_interpolation(
        mut self,
        colour_interpolation: TransitionColourInterpolation,
    ) -> Self {
        self.colour_interpolation = colour_interpolation;
        self
    }

    /// Total transition duration.
    #[must_use]
    pub const fn duration(&self) -> Duration {
        Duration::from_millis(self.duration_ms)
    }

    /// Requested step interval, or the default of approximately 30 Hz.
    #[must_use]
    pub const fn step_interval(&self) -> Duration {
        match self.step_interval_ms {
            Some(milliseconds) => Duration::from_millis(milliseconds),
            None => DEFAULT_STEP_INTERVAL,
        }
    }
}

/// One concrete target and its sparse destination facets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionTargetState {
    /// Concrete target controlled by this transition.
    pub target: TargetId,
    /// Sparse destination. Omitted facets retain their source values.
    pub state: SceneTargetState,
}

/// Why an active transition stopped without completing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionCancellation {
    /// Explicitly aborted by its owner.
    Aborted,
    /// Replaced by an overlapping transition.
    Replaced,
    /// Cancelled by another overlapping mutation.
    ConflictingMutation,
    /// Renewable delegated authorization expired or was denied.
    AuthorizationExpired,
}

/// Terminal transition result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionOutcome {
    /// The exact destination was applied.
    Completed,
    /// The transition stopped deliberately.
    Cancelled(TransitionCancellation),
    /// Hardware failed after preflight succeeded.
    Failed {
        /// Human-readable provider or daemon diagnostic.
        diagnostic: String,
        /// Targets successfully updated by the last attempted step.
        applied_targets: Vec<TargetId>,
    },
}

/// Authoritative transition status snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionStatus {
    /// Transition identifier.
    pub id: TransitionId,
    /// Concrete controlled targets.
    pub targets: Vec<TargetId>,
    /// Elapsed duration, capped at the requested duration.
    pub elapsed_ms: u64,
    /// Requested duration.
    pub duration_ms: u64,
    /// Terminal result, or `None` while active.
    pub outcome: Option<TransitionOutcome>,
}

impl TransitionStatus {
    /// Returns whether this status is terminal.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        self.outcome.is_some()
    }
}

/// Invalid transition timing or input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TransitionValidationError {
    /// Immediate mutations use the existing appearance and scene APIs.
    #[error("transition duration must be greater than zero")]
    ZeroDuration,
    /// Sub-millisecond values cannot be represented on the wire.
    #[error("transition timing must be at least one millisecond")]
    SubMillisecondTiming,
    /// Transition intervals below 10 ms would allow abusive write rates.
    #[error("transition step interval must be at least 10 milliseconds")]
    StepIntervalTooShort,
    /// Millisecond timing exceeds the wire representation.
    #[error("transition timing is too long")]
    DurationTooLong,
}

/// Structural mismatch between two transition endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransitionInterpolationError {
    /// Effects must retain the same identity.
    #[error("transition effects have different identities")]
    EffectIdentity,
    /// Colours must retain their encoding and channel order.
    #[error("transition colours have incompatible structure")]
    ColourStructure,
    /// Hardware effect arguments must retain their discrete structure.
    #[error("transition hardware-effect arguments have incompatible structure")]
    HardwareStructure,
    /// The requested perceptual interpolation does not support this encoding.
    #[error("OKLab interpolation requires an RGB colour encoding")]
    OklabUnsupported,
}

/// Applies a transition's easing function to elapsed progress.
///
/// The returned fraction uses a fixed denominator and retains exact endpoints.
///
/// # Errors
///
/// Returns [`TransitionInterpolationError::HardwareStructure`] when
/// `denominator` is zero.
pub fn ease_progress(
    function: TransitionFunction,
    numerator: u64,
    denominator: u64,
) -> Result<(u64, u64), TransitionInterpolationError> {
    const SCALE: u128 = 1_000_000_000;
    if denominator == 0 {
        return Err(TransitionInterpolationError::HardwareStructure);
    }
    let numerator = numerator.min(denominator);
    let x = (u128::from(numerator) * SCALE + u128::from(denominator / 2)) / u128::from(denominator);
    let complement = SCALE - x;
    let eased = match function {
        TransitionFunction::Linear => x,
        TransitionFunction::EaseIn => x * x * x / (SCALE * SCALE),
        TransitionFunction::EaseOut => {
            SCALE - complement * complement * complement / (SCALE * SCALE)
        }
        TransitionFunction::EaseInOut => {
            (3 * x * x / SCALE).saturating_sub(2 * x * x * x / (SCALE * SCALE))
        }
    };
    Ok((
        u64::try_from(eased).map_err(|_| TransitionInterpolationError::HardwareStructure)?,
        u64::try_from(SCALE).map_err(|_| TransitionInterpolationError::HardwareStructure)?,
    ))
}

/// Linearly interpolates two unsigned values with checked integer rounding.
///
/// `numerator / denominator` is clamped to the inclusive unit interval.
///
/// # Errors
///
/// Returns [`TransitionInterpolationError::HardwareStructure`] when
/// `denominator` is zero.
pub fn interpolate_u32(
    start: u32,
    end: u32,
    numerator: u64,
    denominator: u64,
) -> Result<u32, TransitionInterpolationError> {
    if denominator == 0 {
        return Err(TransitionInterpolationError::HardwareStructure);
    }
    let numerator = numerator.min(denominator);
    let start = i128::from(start);
    let delta = i128::from(end) - start;
    let scaled = delta * i128::from(numerator);
    let half = i128::from(denominator / 2);
    let rounded = if scaled >= 0 {
        (scaled + half) / i128::from(denominator)
    } else {
        (scaled - half) / i128::from(denominator)
    };
    u32::try_from(start + rounded).map_err(|_| TransitionInterpolationError::HardwareStructure)
}

/// Snaps a value to the nearest advertised discrete range step.
///
/// Ties round upward. A zero-step range is rejected.
///
/// # Errors
///
/// Returns [`TransitionInterpolationError::HardwareStructure`] for a zero
/// step or an inverted range.
pub fn snap_to_discrete_range_u32(
    value: u32,
    range: DiscreteRange<u32>,
) -> Result<u32, TransitionInterpolationError> {
    if range.step == 0 || range.min > range.max {
        return Err(TransitionInterpolationError::HardwareStructure);
    }
    let value = value.clamp(range.min, range.max);
    let offset = value - range.min;
    let lower = range.min + (offset / range.step) * range.step;
    let upper = lower.saturating_add(range.step).min(range.max);
    if value - lower < upper - value {
        Ok(lower)
    } else {
        Ok(upper)
    }
}

fn interpolate_u16(
    start: u16,
    end: u16,
    numerator: u64,
    denominator: u64,
) -> Result<u16, TransitionInterpolationError> {
    u16::try_from(interpolate_u32(
        u32::from(start),
        u32::from(end),
        numerator,
        denominator,
    )?)
    .map_err(|_| TransitionInterpolationError::HardwareStructure)
}

fn interpolate_u8(
    start: u8,
    end: u8,
    numerator: u64,
    denominator: u64,
) -> Result<u8, TransitionInterpolationError> {
    u8::try_from(interpolate_u32(
        u32::from(start),
        u32::from(end),
        numerator,
        denominator,
    )?)
    .map_err(|_| TransitionInterpolationError::HardwareStructure)
}

fn interpolate_rgb_encoded(
    start: Rgb,
    end: Rgb,
    numerator: u64,
    denominator: u64,
) -> Result<Rgb, TransitionInterpolationError> {
    Ok(Rgb::new(
        interpolate_u8(start.r, end.r, numerator, denominator)?,
        interpolate_u8(start.g, end.g, numerator, denominator)?,
        interpolate_u8(start.b, end.b, numerator, denominator)?,
    ))
}

#[allow(
    clippy::cast_precision_loss,
    reason = "normalized interpolation progress is intentionally evaluated as f64"
)]
fn interpolate_rgb_oklab(
    start: Rgb,
    end: Rgb,
    numerator: u64,
    denominator: u64,
) -> Result<Rgb, TransitionInterpolationError> {
    if denominator == 0 {
        return Err(TransitionInterpolationError::HardwareStructure);
    }
    if numerator == 0 {
        return Ok(start);
    }
    if numerator >= denominator {
        return Ok(end);
    }
    let progress = numerator as f64 / denominator as f64;
    let start = Oklab::<f64>::from_color(Srgb::new(
        f64::from(start.r) / 255.0,
        f64::from(start.g) / 255.0,
        f64::from(start.b) / 255.0,
    ));
    let end = Oklab::<f64>::from_color(Srgb::new(
        f64::from(end.r) / 255.0,
        f64::from(end.g) / 255.0,
        f64::from(end.b) / 255.0,
    ));
    let mixed = Oklab::new(
        start.l + (end.l - start.l) * progress,
        start.a + (end.a - start.a) * progress,
        start.b + (end.b - start.b) * progress,
    );
    let mapped = Okhsv::from_color_unclamped(mixed).clamp();
    let rgb: Srgb<u8> = Srgb::<f64>::from_color(mapped).into_format();
    Ok(Rgb::new(rgb.red, rgb.green, rgb.blue))
}

fn interpolate_rgb(
    start: Rgb,
    end: Rgb,
    numerator: u64,
    denominator: u64,
    colour_interpolation: TransitionColourInterpolation,
) -> Result<Rgb, TransitionInterpolationError> {
    match colour_interpolation {
        TransitionColourInterpolation::Encoded { .. } => {
            interpolate_rgb_encoded(start, end, numerator, denominator)
        }
        TransitionColourInterpolation::Oklab => {
            interpolate_rgb_oklab(start, end, numerator, denominator)
        }
    }
}

fn interpolate_hue(
    start: u32,
    end: u32,
    maximum: u32,
    numerator: u64,
    denominator: u64,
    direction: HueDirection,
) -> Result<u32, TransitionInterpolationError> {
    let modulus = u64::from(maximum) + 1;
    let start = u64::from(start) % modulus;
    let end = u64::from(end) % modulus;
    let forward = (end + modulus - start) % modulus;
    let signed = match direction {
        HueDirection::Shortest if forward * 2 <= modulus => i128::from(forward),
        HueDirection::Shortest => i128::from(forward) - i128::from(modulus),
        HueDirection::Increasing => i128::from(forward),
        HueDirection::Decreasing => {
            let reverse = (start + modulus - end) % modulus;
            -i128::from(reverse)
        }
    };
    let numerator = numerator.min(denominator);
    if denominator == 0 {
        return Err(TransitionInterpolationError::HardwareStructure);
    }
    let scaled = signed * i128::from(numerator);
    let half = i128::from(denominator / 2);
    let rounded = if scaled >= 0 {
        (scaled + half) / i128::from(denominator)
    } else {
        (scaled - half) / i128::from(denominator)
    };
    let value = (i128::from(start) + rounded).rem_euclid(i128::from(modulus));
    u32::try_from(value).map_err(|_| TransitionInterpolationError::HardwareStructure)
}

fn interpolate_additive_oklab(
    start: &[ColourChannelValue],
    end: &[ColourChannelValue],
    numerator: u64,
    denominator: u64,
) -> Result<Colour, TransitionInterpolationError> {
    let ([start_red, start_green, start_blue], [end_red, end_green, end_blue]) = (start, end)
    else {
        return Err(TransitionInterpolationError::OklabUnsupported);
    };
    if [
        (start_red, end_red, ColourChannel::Red),
        (start_green, end_green, ColourChannel::Green),
        (start_blue, end_blue, ColourChannel::Blue),
    ]
    .iter()
    .any(|(start, end, channel)| {
        start.channel != *channel || end.channel != *channel || start.value > 255 || end.value > 255
    }) {
        return Err(TransitionInterpolationError::OklabUnsupported);
    }
    let start_rgb = Rgb::new(
        u8::try_from(start_red.value)
            .map_err(|_| TransitionInterpolationError::OklabUnsupported)?,
        u8::try_from(start_green.value)
            .map_err(|_| TransitionInterpolationError::OklabUnsupported)?,
        u8::try_from(start_blue.value)
            .map_err(|_| TransitionInterpolationError::OklabUnsupported)?,
    );
    let end_rgb = Rgb::new(
        u8::try_from(end_red.value).map_err(|_| TransitionInterpolationError::OklabUnsupported)?,
        u8::try_from(end_green.value)
            .map_err(|_| TransitionInterpolationError::OklabUnsupported)?,
        u8::try_from(end_blue.value).map_err(|_| TransitionInterpolationError::OklabUnsupported)?,
    );
    let rgb = interpolate_rgb_oklab(start_rgb, end_rgb, numerator, denominator)?;
    Colour::additive(vec![
        ColourChannelValue::new(start_red.channel, u32::from(rgb.r)),
        ColourChannelValue::new(start_green.channel, u32::from(rgb.g)),
        ColourChannelValue::new(start_blue.channel, u32::from(rgb.b)),
    ])
    .map_err(|_| TransitionInterpolationError::ColourStructure)
}

/// Interpolates compatible colours.
///
/// `hue_maximum` is the largest value of the target's encoded hue channel.
/// Hue follows the shortest circular path.
///
/// # Errors
///
/// Returns [`TransitionInterpolationError::ColourStructure`] when encodings,
/// additive channel order, or channel counts differ.
pub fn interpolate_colour(
    start: &Colour,
    end: &Colour,
    numerator: u64,
    denominator: u64,
    hue_maximum: u32,
    colour_interpolation: TransitionColourInterpolation,
) -> Result<Colour, TransitionInterpolationError> {
    let hue_direction = match colour_interpolation {
        TransitionColourInterpolation::Encoded { hue_direction } => hue_direction,
        TransitionColourInterpolation::Oklab => {
            let (Colour::Additive(start), Colour::Additive(end)) = (start, end) else {
                return Err(TransitionInterpolationError::OklabUnsupported);
            };
            return interpolate_additive_oklab(start, end, numerator, denominator);
        }
    };
    match (start, end) {
        (Colour::Additive(start), Colour::Additive(end))
            if start.len() == end.len()
                && start
                    .iter()
                    .zip(end)
                    .all(|(left, right)| left.channel == right.channel) =>
        {
            let channels = start
                .iter()
                .zip(end)
                .map(|(left, right)| {
                    Ok(ColourChannelValue::new(
                        left.channel,
                        interpolate_u32(left.value, right.value, numerator, denominator)?,
                    ))
                })
                .collect::<Result<Vec<_>, TransitionInterpolationError>>()?;
            Colour::additive(channels).map_err(|_| TransitionInterpolationError::ColourStructure)
        }
        (
            Colour::Hsv {
                hue: start_hue,
                saturation: start_saturation,
                value: start_value,
            },
            Colour::Hsv {
                hue: end_hue,
                saturation: end_saturation,
                value: end_value,
            },
        ) => Ok(Colour::hsv(
            interpolate_hue(
                *start_hue,
                *end_hue,
                hue_maximum,
                numerator,
                denominator,
                hue_direction,
            )?,
            interpolate_u32(*start_saturation, *end_saturation, numerator, denominator)?,
            interpolate_u32(*start_value, *end_value, numerator, denominator)?,
        )),
        (
            Colour::Hsl {
                hue: start_hue,
                saturation: start_saturation,
                lightness: start_lightness,
            },
            Colour::Hsl {
                hue: end_hue,
                saturation: end_saturation,
                lightness: end_lightness,
            },
        ) => Ok(Colour::hsl(
            interpolate_hue(
                *start_hue,
                *end_hue,
                hue_maximum,
                numerator,
                denominator,
                hue_direction,
            )?,
            interpolate_u32(*start_saturation, *end_saturation, numerator, denominator)?,
            interpolate_u32(*start_lightness, *end_lightness, numerator, denominator)?,
        )),
        (Colour::Cct { kelvin: start }, Colour::Cct { kelvin: end }) => Ok(Colour::cct(
            interpolate_u32(*start, *end, numerator, denominator)?,
        )),
        (Colour::Monochrome { intensity: start }, Colour::Monochrome { intensity: end }) => Ok(
            Colour::monochrome(interpolate_u32(*start, *end, numerator, denominator)?),
        ),
        _ => Err(TransitionInterpolationError::ColourStructure),
    }
}

fn interpolate_arguments(
    start: &EffectArguments,
    end: &EffectArguments,
    numerator: u64,
    denominator: u64,
    colour_interpolation: TransitionColourInterpolation,
) -> Result<EffectArguments, TransitionInterpolationError> {
    if start.direction != end.direction
        || start.choice != end.choice
        || start.colours.len() != end.colours.len()
        || start.speed.is_some() != end.speed.is_some()
        || start.duration_ms.is_some() != end.duration_ms.is_some()
        || start.brightness.is_some() != end.brightness.is_some()
    {
        return Err(TransitionInterpolationError::HardwareStructure);
    }
    Ok(EffectArguments {
        colours: start
            .colours
            .iter()
            .zip(&end.colours)
            .map(|(left, right)| {
                interpolate_rgb(*left, *right, numerator, denominator, colour_interpolation)
            })
            .collect::<Result<Vec<_>, _>>()?,
        speed: start
            .speed
            .zip(end.speed)
            .map(|(left, right)| interpolate_u16(left, right, numerator, denominator))
            .transpose()?,
        direction: start.direction,
        duration_ms: start
            .duration_ms
            .zip(end.duration_ms)
            .map(|(left, right)| interpolate_u32(left, right, numerator, denominator))
            .transpose()?,
        brightness: start
            .brightness
            .zip(end.brightness)
            .map(|(left, right)| interpolate_u32(left, right, numerator, denominator))
            .transpose()?,
        choice: start.choice.clone(),
    })
}

/// Interpolates compatible effects without changing discrete structure.
///
/// # Errors
///
/// Returns a structural error when variants, hardware effect identifiers,
/// discrete arguments, or colour-list lengths differ.
#[allow(
    clippy::too_many_lines,
    reason = "the exhaustive effect-variant match keeps transition compatibility visible"
)]
pub fn interpolate_effect(
    start: &Effect,
    end: &Effect,
    numerator: u64,
    denominator: u64,
    hue_maximum: u32,
    colour_interpolation: TransitionColourInterpolation,
) -> Result<Effect, TransitionInterpolationError> {
    let period = |start, end| interpolate_u32(start, end, numerator, denominator);
    let rgb =
        |start, end| interpolate_rgb(start, end, numerator, denominator, colour_interpolation);
    match (start, end) {
        (Effect::Static { colour: start }, Effect::Static { colour: end }) => Ok(Effect::Static {
            colour: interpolate_colour(
                start,
                end,
                numerator,
                denominator,
                hue_maximum,
                colour_interpolation,
            )?,
        }),
        (
            Effect::Breathe {
                colour: start_colour,
                period_ms: start_period,
            },
            Effect::Breathe {
                colour: end_colour,
                period_ms: end_period,
            },
        ) => Ok(Effect::Breathe {
            colour: rgb(*start_colour, *end_colour)?,
            period_ms: period(*start_period, *end_period)?,
        }),
        (
            Effect::Pulse {
                colour: start_colour,
                period_ms: start_period,
            },
            Effect::Pulse {
                colour: end_colour,
                period_ms: end_period,
            },
        ) => Ok(Effect::Pulse {
            colour: rgb(*start_colour, *end_colour)?,
            period_ms: period(*start_period, *end_period)?,
        }),
        (
            Effect::Strobe {
                colour: start_colour,
                period_ms: start_period,
            },
            Effect::Strobe {
                colour: end_colour,
                period_ms: end_period,
            },
        ) => Ok(Effect::Strobe {
            colour: rgb(*start_colour, *end_colour)?,
            period_ms: period(*start_period, *end_period)?,
        }),
        (
            Effect::Scanner {
                colour: start_colour,
                period_ms: start_period,
            },
            Effect::Scanner {
                colour: end_colour,
                period_ms: end_period,
            },
        ) => Ok(Effect::Scanner {
            colour: rgb(*start_colour, *end_colour)?,
            period_ms: period(*start_period, *end_period)?,
        }),
        (
            Effect::Morph {
                colours: start_colours,
                period_ms: start_period,
            },
            Effect::Morph {
                colours: end_colours,
                period_ms: end_period,
            },
        ) if start_colours.len() == end_colours.len() => Ok(Effect::Morph {
            colours: start_colours
                .iter()
                .zip(end_colours)
                .map(|(left, right)| rgb(*left, *right))
                .collect::<Result<Vec<_>, _>>()?,
            period_ms: period(*start_period, *end_period)?,
        }),
        (
            Effect::Spectrum {
                period_ms: start_period,
            },
            Effect::Spectrum {
                period_ms: end_period,
            },
        ) => Ok(Effect::Spectrum {
            period_ms: period(*start_period, *end_period)?,
        }),
        (
            Effect::Rainbow {
                period_ms: start_period,
            },
            Effect::Rainbow {
                period_ms: end_period,
            },
        ) => Ok(Effect::Rainbow {
            period_ms: period(*start_period, *end_period)?,
        }),
        (
            Effect::Hardware {
                id: start_id,
                arguments: start_arguments,
            },
            Effect::Hardware {
                id: end_id,
                arguments: end_arguments,
            },
        ) if start_id == end_id => Ok(Effect::Hardware {
            id: start_id.clone(),
            arguments: interpolate_arguments(
                start_arguments,
                end_arguments,
                numerator,
                denominator,
                colour_interpolation,
            )?,
        }),
        _ => Err(TransitionInterpolationError::EffectIdentity),
    }
}

#[cfg(test)]
#[path = "transition_tests.rs"]
mod tests;
