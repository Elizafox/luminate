// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Capabilities through which targets describe their supported operations.

use std::collections::HashSet;
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::appearance_slot::AppearanceSlotsCapability;
use crate::shm_frame::ShmPixelFormat;
use crate::state;
use crate::util::DiscreteRange;
use crate::util::declare_opaque_id;

// Capability set

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Operations and state semantics advertised for one target.
pub struct CapabilitySet {
    /// The colour encodings this target accepts, as independent alternatives
    /// a caller may pick between (for example, a LIFX bulb accepts both
    /// `Additive` RGB and `Cct` kelvin as separate colour values it can
    /// apply natively). An empty list means no drivable colour output at
    /// all: a device whose only lighting behaviour is a single fixed
    /// hardware effect toggled on/off (see `hardware_effects`), with
    /// nothing to set via colour channels.
    pub colour: Vec<ColourCapability>,

    /// Whether the daemon may emulate a `Cct` (colour temperature) request
    /// by mapping it onto this target's `Additive`/`Hsv`/`Hsl` channels when
    /// no `Cct` capability is advertised natively. Devices for which a
    /// kelvin tint is meaningless (e.g. a fixed-colour indicator LED) should
    /// set this to `Disabled` rather than rely on validation alone, since a
    /// target may have `Additive` colour for unrelated reasons.
    pub cct_emulation: CctEmulation,

    /// Separate brightness control, if supported.
    pub brightness: BrightnessCapability,

    /// Frame-upload support, if present.
    pub frame_upload: Option<FrameUploadCapability>,

    /// Hardware effects advertised at this target.
    pub hardware_effects: Option<HardwareEffectsCapability>,

    /// Named firmware-stored appearances owned by this surface.
    #[serde(default)]
    pub appearance_slots: Option<AppearanceSlotsCapability>,

    /// Non-volatile storage behaviour.
    pub persistence: PersistenceCapability,

    /// Live-state readback support.
    pub state_readback: StateReadbackCapability,

    /// Whether this target has a meaningful local emitting/dark state.
    pub emission: bool,

    /// Whether turning this target off is safe even when ordinary appearance
    /// updates are written through to non-volatile storage.
    pub off_is_wear_safe: bool,

    /// Present when this target owns an independently controlled physical
    /// power domain.
    pub physical_power: Option<PhysicalPowerCapability>,

    /// Broader power domain that visible mutations at this target require.
    pub power_domain: Option<PowerDomainRef>,
}

impl Default for CapabilitySet {
    fn default() -> Self {
        Self {
            colour: Vec::new(),
            cct_emulation: CctEmulation::Auto,
            brightness: BrightnessCapability::None,
            frame_upload: None,
            hardware_effects: None,
            appearance_slots: None,
            persistence: PersistenceCapability::None,
            state_readback: StateReadbackCapability::None,
            emission: false,
            off_is_wear_safe: false,
            physical_power: None,
            power_domain: None,
        }
    }
}

/// Why an advertised capability set is not a usable request contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityValidationError(String);

impl fmt::Display for CapabilityValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for CapabilityValidationError {}

impl CapabilitySet {
    /// Validates the construction-time invariants shared by every capability
    /// producer and consumer.
    ///
    /// # Errors
    ///
    /// Returns an error if any advertised value cannot form an unambiguous,
    /// satisfiable request contract.
    pub fn validate(&self) -> Result<(), CapabilityValidationError> {
        validate_colours(&self.colour)?;
        if let BrightnessCapability::Independent { bits, maximum, .. } = self.brightness {
            validate_bits("independent brightness", bits)?;
            let representable = if bits == 32 {
                u32::MAX
            } else {
                (1_u32 << bits) - 1
            };
            ensure(
                maximum != 0 && maximum <= representable,
                "independent brightness maximum is zero or exceeds its bit width",
            )?;
        }
        if let Some(upload) = &self.frame_upload
            && let Some(shm) = &upload.shm
        {
            ensure(
                !shm.pixel_formats.is_empty(),
                "shared-memory frame has no pixel formats",
            )?;
            ensure(
                shm.shape.pixel_count().is_some_and(|count| count != 0),
                "shared-memory frame shape has an invalid pixel count",
            )?;
        }
        if let Some(effects) = &self.hardware_effects {
            validate_effects(effects)?;
        }
        if let Some(slots) = &self.appearance_slots {
            let mut ids = HashSet::new();
            for slot in &slots.slots {
                ensure(
                    ids.insert(slot.id.as_str()),
                    "appearance slot ids are not unique",
                )?;
                validate_colours(&slot.appearance.colour).map_err(|error| {
                    CapabilityValidationError(format!(
                        "appearance slot {}: {error}",
                        slot.id.as_str()
                    ))
                })?;
                if let Some(effects) = &slot.appearance.hardware_effects {
                    validate_effects(effects).map_err(|error| {
                        CapabilityValidationError(format!(
                            "appearance slot {}: {error}",
                            slot.id.as_str()
                        ))
                    })?;
                }
            }
        }
        Ok(())
    }
}

fn ensure(condition: bool, message: &str) -> Result<(), CapabilityValidationError> {
    condition
        .then_some(())
        .ok_or_else(|| CapabilityValidationError(message.to_owned()))
}

fn validate_bits(name: &str, bits: u8) -> Result<(), CapabilityValidationError> {
    ensure(
        (1..=32).contains(&bits),
        &format!("{name} bit width is outside 1..=32"),
    )
}

fn validate_colours(colours: &[ColourCapability]) -> Result<(), CapabilityValidationError> {
    for colour in colours {
        match colour {
            ColourCapability::Additive(channels) => {
                ensure(!channels.is_empty(), "additive colour has no channels")?;
                let mut names = HashSet::new();
                for channel in channels {
                    validate_bits("colour channel", channel.bits)?;
                    ensure(
                        names.insert(channel.channel),
                        "additive colour repeats a channel",
                    )?;
                }
            }
            ColourCapability::Hsv {
                hue_bits,
                saturation_bits,
                value_bits,
            } => {
                for bits in [*hue_bits, *saturation_bits, *value_bits] {
                    validate_bits("HSV channel", bits)?;
                }
            }
            ColourCapability::Hsl {
                hue_bits,
                saturation_bits,
                lightness_bits,
            } => {
                for bits in [*hue_bits, *saturation_bits, *lightness_bits] {
                    validate_bits("HSL channel", bits)?;
                }
            }
            ColourCapability::Cct { bits } | ColourCapability::Monochrome { bits } => {
                validate_bits("colour channel", *bits)?;
            }
        }
    }
    Ok(())
}

fn validate_effects(effects: &HardwareEffectsCapability) -> Result<(), CapabilityValidationError> {
    let mut ids = HashSet::new();
    for effect in &effects.effects {
        ensure(
            !effect.id.as_str().is_empty(),
            "hardware effect has an empty id",
        )?;
        ensure(
            ids.insert(effect.id.as_str()),
            "duplicate hardware effect id",
        )?;
        ensure(!effect.name.is_empty(), "hardware effect has an empty name")?;
        let mut kinds = HashSet::new();
        for parameter in &effect.parameters {
            let kind = match parameter {
                EffectParameter::Colour {
                    minimum_colours,
                    maximum_colours,
                } => {
                    ensure(
                        minimum_colours <= maximum_colours,
                        "hardware effect has an inverted colour-count range",
                    )?;
                    0
                }
                EffectParameter::Speed { range } => {
                    ensure(
                        range.step != 0 && range.min <= range.max,
                        "effect speed range is invalid",
                    )?;
                    1
                }
                EffectParameter::Direction { values } => {
                    ensure(!values.is_empty(), "effect direction list is empty")?;
                    let unique = values.iter().copied().collect::<HashSet<_>>();
                    ensure(
                        unique.len() == values.len(),
                        "effect direction values are not unique",
                    )?;
                    2
                }
                EffectParameter::Duration { milliseconds } => {
                    ensure(
                        milliseconds.step != 0 && milliseconds.min <= milliseconds.max,
                        "effect duration range is invalid",
                    )?;
                    3
                }
                EffectParameter::Brightness { bits } => {
                    validate_bits("effect brightness", *bits)?;
                    4
                }
                EffectParameter::Choice { options } => {
                    ensure(!options.is_empty(), "effect choice list is empty")?;
                    let mut option_ids = HashSet::new();
                    for option in options {
                        ensure(
                            !option.id.is_empty()
                                && !option.name.is_empty()
                                && option_ids.insert(option.id.as_str()),
                            "hardware effect has invalid or duplicate choices",
                        )?;
                    }
                    5
                }
            };
            ensure(
                kinds.insert(kind),
                "hardware effect repeats a parameter kind",
            )?;
        }
    }
    Ok(())
}

// Colour

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// A supported colour channel and its bit width.
pub struct ColourChannelCapability {
    /// Channel represented by this value.
    pub channel: ColourChannel,

    /// Bit width reported by the hardware.
    pub bits: u8,
}

impl ColourChannelCapability {
    /// Creates a channel capability with the hardware's native bit width.
    #[must_use]
    pub const fn new(channel: ColourChannel, bits: u8) -> Self {
        Self { channel, bits }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One structurally complete colour model accepted by a target.
pub enum ColourCapability {
    /// Independent additive emitter channels.
    Additive(Vec<ColourChannelCapability>),

    /// Hue, saturation, and value channel widths.
    Hsv {
        /// Hue channel width.
        hue_bits: u8,
        /// Saturation channel width.
        saturation_bits: u8,
        /// Value channel width.
        value_bits: u8,
    },

    /// Hue, saturation, and lightness channel widths.
    Hsl {
        /// Hue channel width.
        hue_bits: u8,
        /// Saturation channel width.
        saturation_bits: u8,
        /// Lightness channel width.
        lightness_bits: u8,
    },

    /// Correlated-colour-temperature channel width.
    Cct {
        /// Kelvin channel width.
        bits: u8,
    },

    /// Monochrome-intensity channel width.
    Monochrome {
        /// Intensity channel width.
        bits: u8,
    },
}

impl ColourCapability {
    /// Creates an additive red, green, and blue capability with 8-bit channels.
    #[must_use]
    pub fn rgb8() -> Self {
        Self::Additive(vec![
            ColourChannelCapability::new(ColourChannel::Red, 8),
            ColourChannelCapability::new(ColourChannel::Green, 8),
            ColourChannelCapability::new(ColourChannel::Blue, 8),
        ])
    }

    /// Creates a single-channel intensity capability.
    #[must_use]
    pub fn monochrome(bits: u8) -> Self {
        Self::Monochrome { bits }
    }

    /// Creates a correlated-colour-temperature capability with the
    /// hardware's native bit width for its kelvin channel.
    #[must_use]
    pub fn cct(bits: u8) -> Self {
        Self::Cct { bits }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Whether the daemon may emulate `Cct` colour requests on a target that has
/// no native `Cct` capability, by mapping kelvin onto its other advertised
/// colour channels.
pub enum CctEmulation {
    /// Emulate `Cct` via the target's other colour channels when it has no
    /// native `Cct` capability. The default: most additive/HSx-capable
    /// devices can render an approximate warm/cool tint even without a true
    /// white channel.
    Auto,

    /// Never emulate; a `Cct` request against a target with no native `Cct`
    /// capability is rejected as unsupported.
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
/// How a set of colour channels should be interpreted.
pub enum ColourEncoding {
    /// Independent additive emitter channels, such as RGB or RGBW.
    Additive = 0,

    /// Hue, saturation, and value.
    Hsv = 1,

    /// Hue, saturation, and lightness.
    Hsl = 2,

    /// Correlated colour temperature.
    Cct = 3,

    /// A single intensity channel.
    Monochrome = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
/// Named channels used by supported colour models.
pub enum ColourChannel {
    // Additive emitter channels
    /// Red additive channel.
    Red = 0,

    /// Green additive channel.
    Green = 1,

    /// Blue additive channel.
    Blue = 2,

    /// Broad-spectrum white channel.
    White = 3,

    /// Warm-white emitter channel.
    WarmWhite = 4,

    /// Cool-white emitter channel.
    CoolWhite = 5,

    /// Amber emitter channel.
    Amber = 6,

    /// Ultraviolet emitter channel.
    Ultraviolet = 7,

    // Abstract colour-model channels
    /// Hue component.
    Hue = 8,

    /// Saturation component.
    Saturation = 9,

    /// HSV value component.
    Value = 10,

    /// HSL lightness component.
    Lightness = 11,

    /// Correlated colour-temperature component.
    Temperature = 12,

    /// Monochrome intensity.
    Intensity = 13,
}

impl ColourChannel {
    /// Returns the canonical lower-case channel name used in diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::White => "white",
            Self::WarmWhite => "warm-white",
            Self::CoolWhite => "cool-white",
            Self::Amber => "amber",
            Self::Ultraviolet => "ultraviolet",
            Self::Hue => "hue",
            Self::Saturation => "saturation",
            Self::Value => "value",
            Self::Lightness => "lightness",
            Self::Temperature => "temperature",
            Self::Intensity => "intensity",
        }
    }

    /// Returns whether this channel names a physical additive emitter.
    #[must_use]
    pub const fn is_additive(self) -> bool {
        matches!(
            self,
            Self::Red
                | Self::Green
                | Self::Blue
                | Self::White
                | Self::WarmWhite
                | Self::CoolWhite
                | Self::Amber
                | Self::Ultraviolet
        )
    }
}

// Brightness

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Whether brightness has a separate hardware control.
pub enum BrightnessCapability {
    /// No separate hardware brightness control.
    ///
    /// Brightness may still be emulated by scaling colour channels.
    None,

    /// A dedicated hardware brightness control.
    Independent {
        /// Hardware bit width.
        bits: u8,

        /// Largest accepted value.
        maximum: u32,

        /// Scope at which the control applies.
        scope: CapabilityScope,
    },
}

// Shared capability scope

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
/// Smallest addressable scope of a capability.
pub enum CapabilityScope {
    /// One addressable element on a surface.
    Element = 0,

    /// One surface on a device.
    Surface = 1,

    /// An entire device.
    Device = 2,

    /// A shared controller above one device.
    Controller = 3,
}

// Frame upload

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Limits and pixel format for uploading complete lighting frames.
pub struct FrameUploadCapability {
    /// Smallest scope at which the capability applies.
    pub scope: CapabilityScope,

    /// Whether uploads are full, partial, or both.
    pub update_mode: FrameUpdateMode,

    /// Maximum intended frame rate, if known.
    pub max_rate_hz: Option<u16>,

    /// Whether an uploaded frame becomes visible as one indivisible update.
    pub atomic: bool,

    /// How uploaded frames become visible.
    pub buffering: BufferingMode,

    /// Present when this target additionally supports the opt-in,
    /// zero-copy shared-memory frame fast path, on top of the
    /// always-available request/response path described by the fields
    /// above. `None` is correct for a target that only supports the
    /// ordinary path, which is every bundled plugin today.
    #[serde(default)]
    pub shm: Option<ShmFrameCapability>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
/// The pixel layout of a target's shared-memory frame buffer.
pub enum ShmFrameShape {
    /// A flat run of pixels in plugin-defined order, matching how
    /// [`crate::frame::FramePayload::Full`] is already ordered today. The
    /// only shape any bundled plugin needs.
    Linear {
        /// Number of pixels in the buffer.
        pixel_count: u32,
    },

    /// A row-major two-dimensional matrix, so a producer computing a 2D
    /// effect can address pixels by row and column instead of guessing at
    /// plugin-defined linear order.
    Matrix {
        /// Number of columns.
        width: u32,
        /// Number of rows.
        height: u32,
    },
}

impl ShmFrameShape {
    /// The total number of pixels this shape describes.
    ///
    /// For [`Self::Matrix`], this is `width * height`; a plugin whose
    /// dimensions would overflow `u32` cannot advertise a valid shape and
    /// must not report `Matrix` for such a target.
    #[must_use]
    pub const fn pixel_count(self) -> Option<u32> {
        match self {
            Self::Linear { pixel_count } => Some(pixel_count),
            Self::Matrix { width, height } => width.checked_mul(height),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Limits and pixel format for a target's shared-memory frame fast path.
pub struct ShmFrameCapability {
    /// Pixel formats this target accepts, in the plugin's preference
    /// order. The daemon picks the first entry it recognizes.
    pub pixel_formats: Vec<ShmPixelFormat>,

    /// The pixel buffer's layout.
    pub shape: ShmFrameShape,

    /// Maximum intended frame rate for the fast path specifically. `None`
    /// means the same ceiling as [`FrameUploadCapability::max_rate_hz`]
    /// applies to both paths.
    pub max_rate_hz: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
/// Amount of state required in each frame upload.
pub enum FrameUpdateMode {
    /// Every upload must contain the complete state of the scope.
    FullFrameOnly = 0,

    /// Only changed elements may be uploaded.
    Partial = 1,

    /// Both complete and partial uploads are supported.
    Both = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
/// How uploaded frames are staged and displayed.
pub enum BufferingMode {
    /// Updates become visible while or immediately after they are written.
    Immediate = 0,

    /// Updates are staged and made visible by an explicit commit operation.
    ExplicitCommit = 1,

    /// A new frame may be prepared while the previous frame remains visible.
    DoubleBuffered = 2,
}

// Hardware effects

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Firmware effects exposed by a target.
pub struct HardwareEffectsCapability {
    /// Hardware effects advertised by the target.
    pub effects: Vec<HardwareEffectDescriptor>,

    /// Smallest scope at which the capability applies.
    pub scope: CapabilityScope,

    /// Whether hardware effects can remain active while frames are uploaded.
    pub concurrent_with_streaming: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One firmware effect and the arguments it accepts.
pub struct HardwareEffectDescriptor {
    /// Stable identifier.
    pub id: HardwareEffectId,

    /// Human-readable name.
    pub name: String,

    /// Arguments accepted by the effect.
    pub parameters: Vec<EffectParameter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
/// Stable identifier for a firmware effect.
pub struct HardwareEffectId(pub String);

declare_opaque_id!(
    HardwareEffectId,
    "Creates an effect identifier from its plugin-defined string."
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Arguments accepted by an advertised hardware effect.
pub enum EffectParameter {
    /// A fixed colour value.
    Colour {
        /// Minimum number of colours required.
        minimum_colours: u8,

        /// Maximum number of colours accepted.
        maximum_colours: u8,
    },

    /// A bounded speed value.
    Speed {
        /// Accepted discrete range.
        range: DiscreteRange<u16>,
    },

    /// A direction chosen from an advertised set.
    Direction {
        /// Accepted directions.
        values: Vec<EffectDirection>,
    },

    /// A bounded duration in milliseconds.
    Duration {
        /// Accepted duration range in milliseconds.
        milliseconds: DiscreteRange<u32>,
    },

    /// A separate brightness value.
    Brightness {
        /// Hardware bit width.
        bits: u8,
    },

    /// A pick-one-of-named parameter: the effect takes a single choice from a
    /// fixed set of advertised options. This is how a plugin models a large
    /// scene tail (e.g. a bulb's hundreds of named presets) as one grouped
    /// effect rather than one descriptor per scene. The invocation supplies the
    /// selected option's `id` in `EffectArguments::choice`.
    Choice {
        /// Named choices advertised for this parameter.
        options: Vec<EffectChoice>,
    },
}

/// One selectable option of an `EffectParameter::Choice`. `id` is the stable
/// value an invocation references; `name` is human-facing display text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectChoice {
    /// Stable identifier.
    pub id: String,

    /// Human-readable name.
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
/// Directions commonly exposed by hardware effects.
pub enum EffectDirection {
    /// Forward along the target topology.
    Forward = 0,

    /// Reverse along the target topology.
    Reverse = 1,

    /// Clockwise around a ring.
    Clockwise = 2,

    /// Counter-clockwise around a ring.
    CounterClockwise = 3,

    /// Toward the centre.
    Inward = 4,

    /// Away from the centre.
    Outward = 5,

    /// Hardware-selected random direction.
    Random = 6,
}

// Persistence

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// How target state interacts with non-volatile hardware storage.
pub enum PersistenceCapability {
    /// No hardware persistence support.
    None,

    /// The device can persist its current lighting state.
    CurrentState {
        /// Whether ordinary operation requires persistent writes.
        requirement: PersistenceRequirement,

        /// Whether persistence requires a separate save/commit operation.
        explicit_commit: bool,

        /// Whether the persisted state can be read back.
        readback: bool,
    },

    /// The device supports multiple persistent profiles.
    Profiles {
        /// Whether ordinary operation requires persistent writes.
        requirement: PersistenceRequirement,

        /// Number of hardware profile slots.
        slots: u16,

        /// Whether saving requires a separate commit.
        explicit_commit: bool,

        /// Whether saved state can be read back.
        readback: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
/// Whether an ordinary update must write persistent storage.
pub enum PersistenceRequirement {
    /// The device works fully without ever persisting. An explicit save is
    /// an opt-in convenience so state survives a power cycle without the
    /// daemon needing to replay its cache at startup.
    Optional = 0,

    /// The device always writes state through to non-volatile storage as a
    /// side effect of ordinary operation, so there is no separate "unsaved"
    /// state. An explicit save request is meaningless here; the daemon may
    /// treat it as a trivial no-op success. Restore reconciliation should also
    /// avoid redundantly re-pushing cached state to hardware that already
    /// durably holds it, since real flash/EEPROM writes are not free and are
    /// often wear-limited.
    Required = 1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Live hardware-state facets available for observation.
pub enum StateReadbackCapability {
    /// The target cannot report live state.
    None,

    /// The target can report the listed facets at their declared fidelity.
    Readable {
        /// Facets available through readback.
        facets: Vec<ReadableFacet>,

        /// Reading may visibly alter or interrupt output.
        read_disturbs_output: bool,

        /// The device can proactively report changes made outside Luminate.
        notifies_external_changes: bool,
    },
}

/// Whether a hardware value is suitable for exact comparison and adoption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
pub enum ReadbackFidelity {
    /// Useful for display, but not safe for adoption or exact verification.
    BestEffort = 0,

    /// Faithful enough for adoption and exact verification.
    Exact = 1,
}

/// One readable facet and the fidelity guaranteed by the plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReadableFacet {
    /// Facet that can be read.
    pub facet: state::StateFacetKind,

    /// Accuracy guaranteed for that facet.
    pub fidelity: ReadbackFidelity,
}

/// An independently controlled hardware power domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalPowerCapability {
    /// Smallest scope at which the capability applies.
    pub scope: CapabilityScope,
}

/// The ancestor power domain required by a subtarget's visible operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PowerDomainRef {
    /// An entire device.
    Device,

    /// A named surface on the same device owns the required power domain.
    Surface {
        /// Stable surface identifier within the device.
        surface: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effect(parameters: Vec<EffectParameter>) -> HardwareEffectsCapability {
        HardwareEffectsCapability {
            effects: vec![HardwareEffectDescriptor {
                id: HardwareEffectId::new("test"),
                name: "Test".to_owned(),
                parameters,
            }],
            scope: CapabilityScope::Device,
            concurrent_with_streaming: false,
        }
    }

    #[test]
    fn independent_brightness_must_fit_its_bit_width() {
        for (bits, maximum, valid) in [
            (0, 1, false),
            (1, 1, true),
            (8, 255, true),
            (8, 256, false),
            (32, u32::MAX, true),
            (33, u32::MAX, false),
        ] {
            let capabilities = CapabilitySet {
                brightness: BrightnessCapability::Independent {
                    bits,
                    maximum,
                    scope: CapabilityScope::Device,
                },
                ..CapabilitySet::default()
            };
            assert_eq!(
                capabilities.validate().is_ok(),
                valid,
                "bits={bits}, maximum={maximum}"
            );
        }
    }

    #[test]
    fn effect_parameter_kinds_and_directions_are_unique() {
        let duplicate_kind = CapabilitySet {
            hardware_effects: Some(effect(vec![
                EffectParameter::Brightness { bits: 8 },
                EffectParameter::Brightness { bits: 16 },
            ])),
            ..CapabilitySet::default()
        };
        assert!(duplicate_kind.validate().is_err());

        let duplicate_direction = CapabilitySet {
            hardware_effects: Some(effect(vec![EffectParameter::Direction {
                values: vec![EffectDirection::Forward, EffectDirection::Forward],
            }])),
            ..CapabilitySet::default()
        };
        assert!(duplicate_direction.validate().is_err());
    }

    #[test]
    fn matrix_pixel_count_reports_overflow() {
        assert_eq!(
            ShmFrameShape::Matrix {
                width: 3,
                height: 4
            }
            .pixel_count(),
            Some(12)
        );
        assert_eq!(
            ShmFrameShape::Matrix {
                width: u32::MAX,
                height: 2
            }
            .pixel_count(),
            None
        );
    }
}
