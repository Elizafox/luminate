// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;
use luminate_core::shm_frame::ShmPixelFormat;

use super::management::LuminateCctEmulation;

// Capability accessors. Optional nested capabilities are represented by null.
vec_accessors!(
    "Number of colour capabilities advertised.",
    "Borrowed colour capability at `index`, or null if out of range.",
    luminate_capability_set_colour_count,
    luminate_capability_set_colour_at,
    LuminateCapabilitySet,
    CapabilitySet,
    LuminateColourCapability,
    colour
);
/// Whether the device/surface/element can be dark or emitting.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_emission(v: *const LuminateCapabilitySet) -> bool {
    native_ref!(v, CapabilitySet).is_some_and(|v| v.emission)
}

/// Whether turning the target off is safe even when ordinary appearance
/// updates are written through to non-volatile storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_off_is_wear_safe(
    v: *const LuminateCapabilitySet,
) -> bool {
    native_ref!(v, CapabilitySet).is_some_and(|v| v.off_is_wear_safe)
}

/// How CCT requests are emulated when no native CCT channels are advertised;
/// one of the `LUMINATE_CCT_EMULATION_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_cct_emulation(
    v: *const LuminateCapabilitySet,
) -> LuminateCctEmulation {
    native_ref!(v, CapabilitySet).map_or(u32::MAX, |v| v.cct_emulation as u32)
}

/// Whether brightness is independently controllable; one of the
/// `LUMINATE_CAPABILITY_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_brightness_kind(
    v: *const LuminateCapabilitySet,
) -> u32 {
    native_ref!(v, CapabilitySet).map_or(u32::MAX, |v| match v.brightness {
        BrightnessCapability::None => 0,
        BrightnessCapability::Independent { .. } => 1,
    })
}

/// Writes the bit depth of independent brightness control.
///
/// Returns false and leaves `out_bits` unchanged if brightness is not
/// independent or either pointer is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_brightness_bits(
    v: *const LuminateCapabilitySet,
    out_bits: *mut u8,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(bits), Some(out_bits)) = (
        native_ref!(v, CapabilitySet).and_then(|v| match v.brightness {
            BrightnessCapability::Independent { bits, .. } => Some(bits),
            BrightnessCapability::None => None,
        }),
        unsafe { out_bits.as_mut() },
    ) else {
        return false;
    };
    *out_bits = bits;
    true
}

/// Writes the maximum independent brightness value.
///
/// Returns false and leaves `out_maximum` unchanged if brightness is not
/// independent or either pointer is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_brightness_maximum(
    v: *const LuminateCapabilitySet,
    out_maximum: *mut u32,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(maximum), Some(out_maximum)) = (
        native_ref!(v, CapabilitySet).and_then(|v| match v.brightness {
            BrightnessCapability::Independent { maximum, .. } => Some(maximum),
            BrightnessCapability::None => None,
        }),
        unsafe { out_maximum.as_mut() },
    ) else {
        return false;
    };
    *out_maximum = maximum;
    true
}

/// Writes the scope at which independent brightness applies.
///
/// Returns false and leaves `out_scope` unchanged if brightness is not
/// independent or either pointer is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_brightness_scope(
    v: *const LuminateCapabilitySet,
    out_scope: *mut LuminateCapabilityScope,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(scope), Some(out_scope)) = (
        native_ref!(v, CapabilitySet).and_then(|v| match v.brightness {
            BrightnessCapability::Independent { scope, .. } => Some(scope as u32),
            BrightnessCapability::None => None,
        }),
        unsafe { out_scope.as_mut() },
    ) else {
        return false;
    };
    *out_scope = scope;
    true
}

/// Borrowed frame-upload capability, or null if frame upload is not
/// supported.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_frame_upload(
    v: *const LuminateCapabilitySet,
) -> *const LuminateFrameUploadCapability {
    native_ref!(v, CapabilitySet)
        .and_then(|v| v.frame_upload.as_ref())
        .map_or(ptr::null(), cast_ref)
}

/// Borrowed hardware-effects capability, or null if hardware effects are not
/// supported.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_hardware_effects(
    v: *const LuminateCapabilitySet,
) -> *const LuminateHardwareEffectsCapability {
    native_ref!(v, CapabilitySet)
        .and_then(|v| v.hardware_effects.as_ref())
        .map_or(ptr::null(), cast_ref)
}

/// What the device retains across power cycles; one of the
/// `LUMINATE_PERSISTENCE_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_persistence_kind(
    v: *const LuminateCapabilitySet,
) -> LuminatePersistenceKind {
    native_ref!(v, CapabilitySet).map_or(u32::MAX, |v| match v.persistence {
        PersistenceCapability::None => 0,
        PersistenceCapability::CurrentState { .. } => 1,
        PersistenceCapability::Profiles { .. } => 2,
    })
}

/// Writes the persistence requirement.
///
/// Returns false and leaves `out_requirement` unchanged if nothing is
/// persisted or either pointer is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_persistence_requirement(
    v: *const LuminateCapabilitySet,
    out_requirement: *mut LuminatePersistenceRequirement,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(requirement), Some(out_requirement)) = (
        native_ref!(v, CapabilitySet).and_then(|v| match v.persistence {
            PersistenceCapability::CurrentState { requirement, .. }
            | PersistenceCapability::Profiles { requirement, .. } => Some(requirement as u32),
            PersistenceCapability::None => None,
        }),
        unsafe { out_requirement.as_mut() },
    ) else {
        return false;
    };
    *out_requirement = requirement;
    true
}

/// Writes the number of named profile slots.
///
/// Returns false and leaves `out_slots` unchanged unless persistence kind is
/// profiles or either pointer is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_persistence_slots(
    v: *const LuminateCapabilitySet,
    out_slots: *mut u16,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(slots), Some(out_slots)) = (
        native_ref!(v, CapabilitySet).and_then(|v| match v.persistence {
            PersistenceCapability::Profiles { slots, .. } => Some(slots),
            PersistenceCapability::None | PersistenceCapability::CurrentState { .. } => None,
        }),
        unsafe { out_slots.as_mut() },
    ) else {
        return false;
    };
    *out_slots = slots;
    true
}

/// Writes whether persistence requires an explicit commit call.
///
/// Returns false and leaves `out_explicit_commit` unchanged if nothing is
/// persisted or either pointer is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_persistence_explicit_commit(
    v: *const LuminateCapabilitySet,
    out_explicit_commit: *mut bool,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(explicit_commit), Some(out_explicit_commit)) = (
        native_ref!(v, CapabilitySet).and_then(|v| match v.persistence {
            PersistenceCapability::CurrentState {
                explicit_commit, ..
            }
            | PersistenceCapability::Profiles {
                explicit_commit, ..
            } => Some(explicit_commit),
            PersistenceCapability::None => None,
        }),
        unsafe { out_explicit_commit.as_mut() },
    ) else {
        return false;
    };
    *out_explicit_commit = explicit_commit;
    true
}

/// Writes whether persisted state can be read back.
///
/// Returns false and leaves `out_readback` unchanged if nothing is persisted
/// or either pointer is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_persistence_readback(
    v: *const LuminateCapabilitySet,
    out_readback: *mut bool,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(readback), Some(out_readback)) = (
        native_ref!(v, CapabilitySet).and_then(|v| match v.persistence {
            PersistenceCapability::CurrentState { readback, .. }
            | PersistenceCapability::Profiles { readback, .. } => Some(readback),
            PersistenceCapability::None => None,
        }),
        unsafe { out_readback.as_mut() },
    ) else {
        return false;
    };
    *out_readback = readback;
    true
}

/// Whether device state can be read back; one of the
/// `LUMINATE_STATE_READBACK_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_state_readback_kind(
    v: *const LuminateCapabilitySet,
) -> LuminateStateReadbackKind {
    native_ref!(v, CapabilitySet).map_or(u32::MAX, |v| match v.state_readback {
        StateReadbackCapability::None => 0,
        StateReadbackCapability::Readable { .. } => 1,
    })
}

/// Number of readable facets, or 0 if state is not readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_readable_facet_count(
    v: *const LuminateCapabilitySet,
) -> usize {
    native_ref!(v, CapabilitySet).map_or(0, |v| match &v.state_readback {
        StateReadbackCapability::Readable { facets, .. } => facets.len(),
        _ => 0,
    })
}

/// Borrowed readable facet at `index`, or null if out of range or state is
/// not readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_readable_facet_at(
    v: *const LuminateCapabilitySet,
    i: usize,
) -> *const LuminateReadableFacet {
    native_ref!(v, CapabilitySet)
        .and_then(|v| match &v.state_readback {
            StateReadbackCapability::Readable { facets, .. } => facets.get(i),
            _ => None,
        })
        .map_or(ptr::null(), cast_ref)
}

/// Whether reading state disturbs the device's visible output.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_read_disturbs_output(
    v: *const LuminateCapabilitySet,
) -> bool {
    native_ref!(v, CapabilitySet).is_some_and(|v| {
        matches!(
            v.state_readback,
            StateReadbackCapability::Readable {
                read_disturbs_output: true,
                ..
            }
        )
    })
}

/// Whether the device notifies of state changes made outside luminate.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_notifies_external_changes(
    v: *const LuminateCapabilitySet,
) -> bool {
    native_ref!(v, CapabilitySet).is_some_and(|v| {
        matches!(
            v.state_readback,
            StateReadbackCapability::Readable {
                notifies_external_changes: true,
                ..
            }
        )
    })
}

/// Borrowed physical-power capability, or null if not supported.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_physical_power(
    v: *const LuminateCapabilitySet,
) -> *const LuminatePhysicalPowerCapability {
    native_ref!(v, CapabilitySet)
        .and_then(|v| v.physical_power.as_ref())
        .map_or(ptr::null(), cast_ref)
}

/// Borrowed power domain reference, or null if not set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_power_domain(
    v: *const LuminateCapabilitySet,
) -> *const LuminatePowerDomainRef {
    native_ref!(v, CapabilitySet)
        .and_then(|v| v.power_domain.as_ref())
        .map_or(ptr::null(), cast_ref)
}

/// How this colour capability's channels are interpreted; one of the
/// `LUMINATE_COLOUR_ENCODING_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_colour_capability_encoding(
    v: *const LuminateColourCapability,
) -> LuminateColourEncoding {
    native_ref!(v, ColourCapability).map_or(u32::MAX, |v| match v {
        ColourCapability::Additive(_) => ColourEncoding::Additive as u32,
        ColourCapability::Hsv { .. } => ColourEncoding::Hsv as u32,
        ColourCapability::Hsl { .. } => ColourEncoding::Hsl as u32,
        ColourCapability::Cct { .. } => ColourEncoding::Cct as u32,
        ColourCapability::Monochrome { .. } => ColourEncoding::Monochrome as u32,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_colour_capability_channel_count(
    v: *const LuminateColourCapability,
) -> usize {
    native_ref!(v, ColourCapability).map_or(0, |v| match v {
        ColourCapability::Additive(channels) => channels.len(),
        ColourCapability::Hsv { .. }
        | ColourCapability::Hsl { .. }
        | ColourCapability::Cct { .. }
        | ColourCapability::Monochrome { .. } => 0,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_colour_capability_channel_at(
    v: *const LuminateColourCapability,
    index: usize,
) -> *const LuminateColourChannelCapability {
    native_ref!(v, ColourCapability)
        .and_then(|v| match v {
            ColourCapability::Additive(channels) => channels.get(index),
            _ => None,
        })
        .map_or(ptr::null(), cast_ref)
}

/// Looks up a channel bit width by `LuminateColourChannel`.
///
/// Returns false for a null capability, invalid or absent channel, or null
/// output.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_colour_capability_bits(
    v: *const LuminateColourCapability,
    channel: LuminateColourChannel,
    out_bits: *mut u8,
) -> bool {
    let Some(capability) = native_ref!(v, ColourCapability) else {
        return false;
    };
    let Ok(channel) = colour_input::colour_channel(channel) else {
        return false;
    };
    let bits = match capability {
        ColourCapability::Additive(channels) => channels
            .iter()
            .find(|value| value.channel == channel)
            .map(|value| value.bits),
        ColourCapability::Hsv {
            hue_bits,
            saturation_bits,
            value_bits,
        } => match channel {
            ColourChannel::Hue => Some(*hue_bits),
            ColourChannel::Saturation => Some(*saturation_bits),
            ColourChannel::Value => Some(*value_bits),
            _ => None,
        },
        ColourCapability::Hsl {
            hue_bits,
            saturation_bits,
            lightness_bits,
        } => match channel {
            ColourChannel::Hue => Some(*hue_bits),
            ColourChannel::Saturation => Some(*saturation_bits),
            ColourChannel::Lightness => Some(*lightness_bits),
            _ => None,
        },
        ColourCapability::Cct { bits } if channel == ColourChannel::Temperature => Some(*bits),
        ColourCapability::Monochrome { bits } if channel == ColourChannel::Intensity => Some(*bits),
        ColourCapability::Cct { .. } | ColourCapability::Monochrome { .. } => None,
    };
    let Some(bits) = bits else {
        return false;
    };
    if out_bits.is_null() {
        return false;
    }
    // SAFETY: the caller supplied a writable output pointer.
    unsafe { *out_bits = bits };
    true
}

/// Which channel this capability describes; one of the
/// `LUMINATE_COLOUR_CHANNEL_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_colour_channel_capability_channel(
    v: *const LuminateColourChannelCapability,
) -> LuminateColourChannel {
    native_ref!(v, ColourChannelCapability).map_or(u32::MAX, |v| v.channel as u32)
}

/// Bit depth of this channel.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_colour_channel_capability_bits(
    v: *const LuminateColourChannelCapability,
) -> u8 {
    native_ref!(v, ColourChannelCapability).map_or(0, |v| v.bits)
}

/// Granularity at which frame upload applies; one of the `LUMINATE_SCOPE_*`
/// values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_frame_upload_scope(
    v: *const LuminateFrameUploadCapability,
) -> LuminateCapabilityScope {
    native_ref!(v, FrameUploadCapability).map_or(u32::MAX, |v| v.scope as u32)
}

/// Which frame upload styles are accepted; one of the
/// `LUMINATE_FRAME_UPDATE_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_frame_upload_update_mode(
    v: *const LuminateFrameUploadCapability,
) -> LuminateFrameUpdateMode {
    native_ref!(v, FrameUploadCapability).map_or(u32::MAX, |v| v.update_mode as u32)
}

/// Whether a maximum upload rate is specified.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_frame_upload_has_max_rate_hz(
    v: *const LuminateFrameUploadCapability,
) -> bool {
    native_ref!(v, FrameUploadCapability).is_some_and(|v| v.max_rate_hz.is_some())
}

/// Maximum upload rate in Hz, or 0 if unspecified.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_frame_upload_max_rate_hz(
    v: *const LuminateFrameUploadCapability,
) -> u16 {
    native_ref!(v, FrameUploadCapability)
        .and_then(|v| v.max_rate_hz)
        .unwrap_or(0)
}

/// Whether uploads are applied atomically.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_frame_upload_atomic(
    v: *const LuminateFrameUploadCapability,
) -> bool {
    native_ref!(v, FrameUploadCapability).is_some_and(|v| v.atomic)
}

/// How uploaded frames are committed to output; one of the
/// `LUMINATE_BUFFERING_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_frame_upload_buffering(
    v: *const LuminateFrameUploadCapability,
) -> LuminateBufferingMode {
    native_ref!(v, FrameUploadCapability).map_or(u32::MAX, |v| v.buffering as u32)
}

/// Borrowed shared-memory fast-path capability, or null when only ordinary
/// request/response frame uploads are supported.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_frame_upload_shm(
    v: *const LuminateFrameUploadCapability,
) -> *const LuminateShmFrameCapability {
    native_ref!(v, FrameUploadCapability)
        .and_then(|v| v.shm.as_ref())
        .map_or(ptr::null(), cast_ref)
}

/// Number of packed pixel formats accepted by this shared-memory capability.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_shm_frame_pixel_format_count(
    v: *const LuminateShmFrameCapability,
) -> usize {
    native_ref!(v, ShmFrameCapability).map_or(0, |v| v.pixel_formats.len())
}

/// Packed pixel format at `index`, or `LUMINATE_DISCRIMINANT_INVALID` if out
/// of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_shm_frame_pixel_format_at(
    v: *const LuminateShmFrameCapability,
    index: usize,
) -> LuminateShmPixelFormat {
    native_ref!(v, ShmFrameCapability)
        .and_then(|v| v.pixel_formats.get(index))
        .map_or(u32::MAX, |format| format.to_abi())
}

/// Number of bytes occupied by one pixel in `format`, or `0` for an unknown
/// discriminant.
#[unsafe(no_mangle)]
pub extern "C" fn luminate_shm_pixel_format_bytes_per_pixel(
    format: LuminateShmPixelFormat,
) -> usize {
    ShmPixelFormat::from_abi(format).map_or(0, ShmPixelFormat::bytes_per_pixel)
}

/// The buffer layout; one of the `LUMINATE_SHM_FRAME_SHAPE_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_shm_frame_shape_kind(
    v: *const LuminateShmFrameCapability,
) -> LuminateShmFrameShapeKind {
    native_ref!(v, ShmFrameCapability).map_or(u32::MAX, |v| match v.shape {
        ShmFrameShape::Linear { .. } => 0,
        ShmFrameShape::Matrix { .. } => 1,
    })
}

/// Writes the total number of pixels in the shared-memory buffer.
///
/// Returns false and leaves `out_pixel_count` unchanged if either pointer is
/// null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_shm_frame_pixel_count(
    v: *const LuminateShmFrameCapability,
    out_pixel_count: *mut u32,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(pixel_count), Some(out_pixel_count)) = (
        native_ref!(v, ShmFrameCapability).and_then(|value| value.shape.pixel_count()),
        unsafe { out_pixel_count.as_mut() },
    ) else {
        return false;
    };
    *out_pixel_count = pixel_count;
    true
}

/// Writes the matrix width.
///
/// Returns false and leaves `out_width` unchanged unless the buffer shape is
/// matrix or either pointer is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_shm_frame_matrix_width(
    v: *const LuminateShmFrameCapability,
    out_width: *mut u32,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(width), Some(out_width)) = (
        native_ref!(v, ShmFrameCapability).and_then(|v| match v.shape {
            ShmFrameShape::Matrix { width, .. } => Some(width),
            ShmFrameShape::Linear { .. } => None,
        }),
        unsafe { out_width.as_mut() },
    ) else {
        return false;
    };
    *out_width = width;
    true
}

/// Writes the matrix height.
///
/// Returns false and leaves `out_height` unchanged unless the buffer shape is
/// matrix or either pointer is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_shm_frame_matrix_height(
    v: *const LuminateShmFrameCapability,
    out_height: *mut u32,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(height), Some(out_height)) = (
        native_ref!(v, ShmFrameCapability).and_then(|v| match v.shape {
            ShmFrameShape::Matrix { height, .. } => Some(height),
            ShmFrameShape::Linear { .. } => None,
        }),
        unsafe { out_height.as_mut() },
    ) else {
        return false;
    };
    *out_height = height;
    true
}

/// Whether a shared-memory-specific maximum upload rate is specified.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_shm_frame_has_max_rate_hz(
    v: *const LuminateShmFrameCapability,
) -> bool {
    native_ref!(v, ShmFrameCapability).is_some_and(|v| v.max_rate_hz.is_some())
}

/// Shared-memory-specific maximum upload rate in Hz, or `0` when the ordinary
/// frame-upload limit applies.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_shm_frame_max_rate_hz(
    v: *const LuminateShmFrameCapability,
) -> u16 {
    native_ref!(v, ShmFrameCapability)
        .and_then(|v| v.max_rate_hz)
        .unwrap_or(0)
}

vec_accessors!(
    "Number of hardware effects advertised by a hardware-effects capability.",
    "Borrowed hardware effect descriptor at `index`, or null if out of \
     range.",
    luminate_hardware_effects_effect_count,
    luminate_hardware_effects_effect_at,
    LuminateHardwareEffectsCapability,
    HardwareEffectsCapability,
    LuminateHardwareEffectDescriptor,
    effects
);
/// Granularity at which hardware effects apply; one of the
/// `LUMINATE_SCOPE_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_hardware_effects_scope(
    v: *const LuminateHardwareEffectsCapability,
) -> u32 {
    native_ref!(v, HardwareEffectsCapability).map_or(u32::MAX, |v| v.scope as u32)
}

/// Whether hardware effects can run concurrently with frame streaming.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_hardware_effects_concurrent_with_streaming(
    v: *const LuminateHardwareEffectsCapability,
) -> bool {
    native_ref!(v, HardwareEffectsCapability).is_some_and(|v| v.concurrent_with_streaming)
}

str_accessor!(
    "The hardware effect's stable identifier, passed to \
     `luminate_effect_create_hardware`.",
    luminate_hardware_effect_descriptor_id,
    LuminateHardwareEffectDescriptor,
    HardwareEffectDescriptor,
    |v: &HardwareEffectDescriptor| v.id.as_str()
);
str_accessor!(
    "The hardware effect's human-readable display name.",
    luminate_hardware_effect_descriptor_name,
    LuminateHardwareEffectDescriptor,
    HardwareEffectDescriptor,
    |v: &HardwareEffectDescriptor| v.name.as_str()
);
vec_accessors!(
    "Number of configurable parameters the hardware effect exposes.",
    "Borrowed parameter descriptor at `index`, or null if out of range.",
    luminate_hardware_effect_descriptor_parameter_count,
    luminate_hardware_effect_descriptor_parameter_at,
    LuminateHardwareEffectDescriptor,
    HardwareEffectDescriptor,
    LuminateEffectParameter,
    parameters
);
/// Which variant of the effect parameter union is populated; one of the
/// `LUMINATE_EFFECT_PARAMETER_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_parameter_kind(
    v: *const LuminateEffectParameter,
) -> LuminateEffectParameterKind {
    native_ref!(v, EffectParameter).map_or(u32::MAX, |v| match v {
        EffectParameter::Colour { .. } => 0,
        EffectParameter::Speed { .. } => 1,
        EffectParameter::Direction { .. } => 2,
        EffectParameter::Duration { .. } => 3,
        EffectParameter::Brightness { .. } => 4,
        EffectParameter::Choice { .. } => 5,
    })
}

/// Writes the inclusive colour-count range for a colour parameter.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_parameter_colour_count_range(
    v: *const LuminateEffectParameter,
    out_minimum: *mut u8,
    out_maximum: *mut u8,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some((minimum, maximum)), Some(out_minimum), Some(out_maximum)) = (
        native_ref!(v, EffectParameter).and_then(|v| match v {
            EffectParameter::Colour {
                minimum_colours,
                maximum_colours,
            } => Some((*minimum_colours, *maximum_colours)),
            _ => None,
        }),
        unsafe { out_minimum.as_mut() },
        unsafe { out_maximum.as_mut() },
    ) else {
        return false;
    };
    *out_minimum = minimum;
    *out_maximum = maximum;
    true
}

/// The allowed speed range, or a zeroed range unless this is a speed
/// parameter.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_parameter_speed_range(
    v: *const LuminateEffectParameter,
    out_range: *mut LuminateU16Range,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(range), Some(out_range)) = (
        native_ref!(v, EffectParameter).and_then(|v| match v {
            EffectParameter::Speed { range } => Some(range),
            _ => None,
        }),
        unsafe { out_range.as_mut() },
    ) else {
        return false;
    };
    *out_range = LuminateU16Range {
        minimum: range.min,
        maximum: range.max,
        step: range.step,
    };
    true
}

/// Number of supported directions, or 0 unless this is a direction
/// parameter.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_parameter_direction_count(
    v: *const LuminateEffectParameter,
) -> usize {
    native_ref!(v, EffectParameter).map_or(0, |v| {
        if let EffectParameter::Direction { values } = v {
            values.len()
        } else {
            0
        }
    })
}

/// Direction at `index`; one of the `LUMINATE_DIRECTION_*` values, or
/// `LUMINATE_DISCRIMINANT_INVALID` if out of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_parameter_direction_at(
    v: *const LuminateEffectParameter,
    i: usize,
) -> LuminateEffectDirection {
    native_ref!(v, EffectParameter)
        .and_then(|v| {
            if let EffectParameter::Direction { values } = v {
                values.get(i).map(|x| *x as u32)
            } else {
                None
            }
        })
        .unwrap_or(u32::MAX)
}

/// The allowed duration-in-milliseconds range, or a zeroed range unless this
/// is a duration parameter.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_parameter_duration_range(
    v: *const LuminateEffectParameter,
    out_range: *mut LuminateU32Range,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(range), Some(out_range)) = (
        native_ref!(v, EffectParameter).and_then(|v| match v {
            EffectParameter::Duration { milliseconds } => Some(milliseconds),
            _ => None,
        }),
        unsafe { out_range.as_mut() },
    ) else {
        return false;
    };
    *out_range = LuminateU32Range {
        minimum: range.min,
        maximum: range.max,
        step: range.step,
    };
    true
}

/// Bit depth of the brightness parameter, or 0 unless this is a brightness
/// parameter.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_parameter_brightness_bits(
    v: *const LuminateEffectParameter,
    out_bits: *mut u8,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(bits), Some(out_bits)) = (
        native_ref!(v, EffectParameter).and_then(|v| match v {
            EffectParameter::Brightness { bits } => Some(*bits),
            _ => None,
        }),
        unsafe { out_bits.as_mut() },
    ) else {
        return false;
    };
    *out_bits = bits;
    true
}

/// Number of choice options, or 0 unless this is a choice parameter.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_parameter_choice_count(
    v: *const LuminateEffectParameter,
) -> usize {
    native_ref!(v, EffectParameter).map_or(0, |v| {
        if let EffectParameter::Choice { options } = v {
            options.len()
        } else {
            0
        }
    })
}

/// Borrowed choice option at `index`, or null if out of range or not a
/// choice parameter.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_parameter_choice_at(
    v: *const LuminateEffectParameter,
    i: usize,
) -> *const LuminateEffectChoice {
    native_ref!(v, EffectParameter)
        .and_then(|v| {
            if let EffectParameter::Choice { options } = v {
                options.get(i)
            } else {
                None
            }
        })
        .map_or(ptr::null(), cast_ref)
}

str_accessor!(
    "The choice option's stable identifier.",
    luminate_effect_choice_id,
    LuminateEffectChoice,
    EffectChoice,
    |v: &EffectChoice| v.id.as_str()
);
str_accessor!(
    "The choice option's human-readable display name.",
    luminate_effect_choice_name,
    LuminateEffectChoice,
    EffectChoice,
    |v: &EffectChoice| v.name.as_str()
);
/// Which facet is readable; one of the `LUMINATE_FACET_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_readable_facet_kind(
    v: *const LuminateReadableFacet,
) -> LuminateStateFacetKind {
    native_ref!(v, ReadableFacet).map_or(u32::MAX, |v| v.facet as u32)
}

/// How trustworthy readback of this facet is; one of the
/// `LUMINATE_READBACK_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_readable_facet_fidelity(
    v: *const LuminateReadableFacet,
) -> LuminateReadbackFidelity {
    native_ref!(v, ReadableFacet).map_or(u32::MAX, |v| v.fidelity as u32)
}

/// Granularity at which physical power control applies; one of the
/// `LUMINATE_SCOPE_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_physical_power_scope(
    v: *const LuminatePhysicalPowerCapability,
) -> LuminateCapabilityScope {
    native_ref!(v, PhysicalPowerCapability).map_or(u32::MAX, |v| v.scope as u32)
}

/// What this power domain reference addresses; one of the
/// `LUMINATE_POWER_DOMAIN_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_power_domain_kind(
    v: *const LuminatePowerDomainRef,
) -> LuminatePowerDomainKind {
    native_ref!(v, PowerDomainRef).map_or(u32::MAX, |v| match v {
        PowerDomainRef::Device => 0,
        PowerDomainRef::Surface { .. } => 1,
    })
}

/// The referenced surface's id, or an absent view unless this domain refers
/// to a surface.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_power_domain_surface_id(
    v: *const LuminatePowerDomainRef,
) -> LuminateStringView {
    native_ref!(v, PowerDomainRef).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| match v {
            PowerDomainRef::Surface { surface } => sv(surface),
            _ => optional_sv(None),
        },
    )
}

#[cfg(test)]
#[path = "capabilities_tests.rs"]
mod tests;
