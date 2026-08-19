// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

/// Collection id described by this aggregate state snapshot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_state_id(
    value: *const LuminateCollectionStateSnapshot,
) -> LuminateStringView {
    native_ref!(value, CollectionStateStatus).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |value| sv(value.collection.as_str()),
    )
}

/// Whether configured appearance is known for every collection constituent.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_state_has_appearance(
    value: *const LuminateCollectionStateSnapshot,
) -> bool {
    native_ref!(value, CollectionStateStatus).is_some_and(|value| value.appearance.is_some())
}

/// Configured appearance kind, including `LUMINATE_APPEARANCE_MIXED`, or
/// `LUMINATE_DISCRIMINANT_INVALID` when unknown.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_state_appearance_kind(
    value: *const LuminateCollectionStateSnapshot,
) -> u32 {
    native_ref!(value, CollectionStateStatus)
        .and_then(|value| value.appearance.as_ref())
        .map_or(u32::MAX, |observation| match observation.value {
            AppearanceState::Static(_) => 0,
            AppearanceState::Effect(_) => 1,
            AppearanceState::Mixed => 2,
        })
}

/// Borrowed configured static colour, or null when configured appearance is
/// unknown or is not static. The view remains valid while `value` remains
/// alive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_state_appearance_colour(
    value: *const LuminateCollectionStateSnapshot,
) -> *const LuminateColour {
    native_ref!(value, CollectionStateStatus)
        .and_then(|value| value.appearance.as_ref())
        .and_then(|observation| match &observation.value {
            AppearanceState::Static(colour) => Some(colour),
            AppearanceState::Effect(_) | AppearanceState::Mixed => None,
        })
        .map_or(ptr::null(), cast_ref)
}

/// Borrowed configured effect, or null when configured appearance is unknown
/// or is not an effect. The view remains valid while `value` remains alive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_state_appearance_effect(
    value: *const LuminateCollectionStateSnapshot,
) -> *const LuminateEffectView {
    native_ref!(value, CollectionStateStatus)
        .and_then(|value| value.appearance.as_ref())
        .and_then(|observation| match &observation.value {
            AppearanceState::Effect(effect) => Some(effect),
            AppearanceState::Static(_) | AppearanceState::Mixed => None,
        })
        .map_or(ptr::null(), cast_ref)
}

/// Whether effective appearance is known for every collection constituent.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_state_has_effective_appearance(
    value: *const LuminateCollectionStateSnapshot,
) -> bool {
    native_ref!(value, CollectionStateStatus)
        .is_some_and(|value| value.effective_appearance.is_some())
}

/// Effective appearance kind, including
/// `LUMINATE_EFFECTIVE_APPEARANCE_MIXED`, or
/// `LUMINATE_DISCRIMINANT_INVALID` when unknown.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_state_effective_appearance_kind(
    value: *const LuminateCollectionStateSnapshot,
) -> u32 {
    native_ref!(value, CollectionStateStatus)
        .and_then(|value| value.effective_appearance.as_ref())
        .map_or(u32::MAX, |observation| match observation.value {
            EffectiveAppearanceState::Off => 0,
            EffectiveAppearanceState::Static(_) => 1,
            EffectiveAppearanceState::Effect(_) => 2,
            EffectiveAppearanceState::Streaming => 3,
            EffectiveAppearanceState::Mixed => 4,
        })
}

/// Borrowed effective static colour, or null when effective appearance is
/// unknown, off, streaming, mixed, or is not static. The view remains valid
/// while `value` remains alive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_state_effective_appearance_colour(
    value: *const LuminateCollectionStateSnapshot,
) -> *const LuminateColour {
    native_ref!(value, CollectionStateStatus)
        .and_then(|value| value.effective_appearance.as_ref())
        .and_then(|observation| match &observation.value {
            EffectiveAppearanceState::Static(colour) => Some(colour),
            EffectiveAppearanceState::Off
            | EffectiveAppearanceState::Effect(_)
            | EffectiveAppearanceState::Streaming
            | EffectiveAppearanceState::Mixed => None,
        })
        .map_or(ptr::null(), cast_ref)
}

/// Borrowed effective effect, or null when effective appearance is unknown,
/// off, streaming, mixed, or is not an effect. The view remains valid while
/// `value` remains alive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_state_effective_appearance_effect(
    value: *const LuminateCollectionStateSnapshot,
) -> *const LuminateEffectView {
    native_ref!(value, CollectionStateStatus)
        .and_then(|value| value.effective_appearance.as_ref())
        .and_then(|observation| match &observation.value {
            EffectiveAppearanceState::Effect(effect) => Some(effect),
            EffectiveAppearanceState::Off
            | EffectiveAppearanceState::Static(_)
            | EffectiveAppearanceState::Streaming
            | EffectiveAppearanceState::Mixed => None,
        })
        .map_or(ptr::null(), cast_ref)
}

// State, target, colour and effect views.
str_accessor!(
    "The device id this state snapshot belongs to.",
    luminate_state_device_id,
    LuminateState,
    DeviceStateStatus,
    |v: &DeviceStateStatus| v.device.as_str()
);
vec_accessors!(
    "Number of facet observations recorded for the device.",
    "Borrowed observation at `index`, or null if out of range.",
    luminate_state_observation_count,
    luminate_state_observation_at,
    LuminateState,
    DeviceStateStatus,
    LuminateFacetObservation,
    observations
);

/// Finds the observation for a specific target and facet kind, or null if
/// none matches. `kind` is one of the `LUMINATE_FACET_*` values. `target` is
/// a borrowed target view from an observation or adoption record of this
/// same state; this is the one supported way to look up a specific facet.
/// `observations` carries no ordering contract, so a caller assuming a fixed
/// position (for example that `Appearance` is always first) sees whatever a
/// given daemon build happens to sort by, not a guarantee.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_state_find_observation(
    state: *const LuminateState,
    target: *const LuminateTargetView,
    kind: u32,
) -> *const LuminateFacetObservation {
    let Some(state) = (native_ref!(state, DeviceStateStatus)) else {
        return ptr::null();
    };
    let Some(target) = (native_ref!(target, TargetId)) else {
        return ptr::null();
    };
    state
        .observations
        .iter()
        .find(|observation| {
            &observation.target == target && observation.value.kind() as u32 == kind
        })
        .map_or(ptr::null(), cast_ref)
}

vec_accessors!(
    "Number of facet adoption records for the device.",
    "Borrowed adoption record at `index`, or null if out of range.",
    luminate_state_adoption_count,
    luminate_state_adoption_at,
    LuminateState,
    DeviceStateStatus,
    LuminateAdoption,
    adoption
);
/// Whether the device is currently reachable; one of the
/// `LUMINATE_REACHABILITY_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_state_reachability(
    v: *const LuminateState,
) -> LuminateReachability {
    native_ref!(v, DeviceStateStatus).map_or(u32::MAX, |v| v.reachability as u32)
}

/// Progress of reconciling desired state with hardware; one of the
/// `LUMINATE_RECONCILIATION_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_state_reconciliation(
    v: *const LuminateState,
) -> LuminateReconciliationStatus {
    native_ref!(v, DeviceStateStatus).map_or(u32::MAX, |v| v.reconciliation as u32)
}

/// The most recent reconciliation error message, or an absent view if there
/// was none.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_state_latest_error(
    v: *const LuminateState,
) -> LuminateStringView {
    native_ref!(v, DeviceStateStatus).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| optional_sv(v.latest_error.as_deref()),
    )
}

/// Whether a most-recent reconciliation attempt timestamp is recorded.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_state_has_latest_attempt_ms(v: *const LuminateState) -> bool {
    native_ref!(v, DeviceStateStatus).is_some_and(|v| v.latest_attempt_ms.is_some())
}

/// Timestamp of the most recent reconciliation attempt in milliseconds, or 0
/// if none is recorded.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_state_latest_attempt_ms(v: *const LuminateState) -> u64 {
    native_ref!(v, DeviceStateStatus)
        .and_then(|v| v.latest_attempt_ms)
        .unwrap_or(0)
}

/// Borrowed target this observation describes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_observation_target(
    v: *const LuminateFacetObservation,
) -> *const LuminateTargetView {
    native_ref!(v, FacetObservation).map_or(ptr::null(), |v| cast_ref(&v.target))
}

/// Borrowed observed facet value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_observation_value(
    v: *const LuminateFacetObservation,
) -> *const LuminateFacetValue {
    native_ref!(v, FacetObservation).map_or(ptr::null(), |v| cast_ref(&v.value))
}

/// How this observation was obtained; one of the `LUMINATE_CONFIDENCE_*`
/// values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_observation_confidence(
    v: *const LuminateFacetObservation,
) -> LuminateObservationConfidence {
    native_ref!(v, FacetObservation).map_or(u32::MAX, |v| v.confidence as u32)
}

/// Where this observation's value came from; one of the
/// `LUMINATE_SOURCE_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_observation_source(
    v: *const LuminateFacetObservation,
) -> LuminateObservationSource {
    native_ref!(v, FacetObservation).map_or(u32::MAX, |v| v.source as u32)
}

/// Timestamp this observation was made, in milliseconds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_observation_observed_at_ms(
    v: *const LuminateFacetObservation,
) -> u64 {
    native_ref!(v, FacetObservation).map_or(0, |v| v.observed_at_ms)
}

/// Whether this observation is considered stale.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_observation_stale(v: *const LuminateFacetObservation) -> bool {
    native_ref!(v, FacetObservation).is_some_and(|v| v.stale)
}

/// Borrowed target this adoption record describes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_adoption_target(
    v: *const LuminateAdoption,
) -> *const LuminateTargetView {
    native_ref!(v, (TargetId, StateFacetKind, AdoptionStatus))
        .map_or(ptr::null(), |v| cast_ref(&v.0))
}

/// Which facet this adoption record covers; one of the `LUMINATE_FACET_*`
/// values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_adoption_facet(
    v: *const LuminateAdoption,
) -> LuminateStateFacetKind {
    native_ref!(v, (TargetId, StateFacetKind, AdoptionStatus)).map_or(u32::MAX, |v| v.1 as u32)
}

/// Whether the facet's pre-existing hardware state was adopted; one of the
/// `LUMINATE_ADOPTION_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_adoption_status(
    v: *const LuminateAdoption,
) -> LuminateAdoptionStatus {
    native_ref!(v, (TargetId, StateFacetKind, AdoptionStatus)).map_or(u32::MAX, |v| v.2 as u32)
}

/// Which addressing level this target names; one of the `LUMINATE_TARGET_*`
/// values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_target_view_kind(
    v: *const LuminateTargetView,
) -> LuminateTargetKind {
    native_ref!(v, TargetId).map_or(u32::MAX, |v| match v {
        TargetId::Device(_) => 0,
        TargetId::Surface { .. } => 1,
        TargetId::Element { .. } => 2,
        TargetId::Group { .. } => 3,
    })
}

/// The target's device id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_target_view_device_id(
    v: *const LuminateTargetView,
) -> LuminateStringView {
    native_ref!(v, TargetId).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| sv(v.device_id().as_str()),
    )
}

/// The target's surface id, or an absent view unless the target addresses a
/// surface or element.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_target_view_surface_id(
    v: *const LuminateTargetView,
) -> LuminateStringView {
    native_ref!(v, TargetId).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| match v {
            TargetId::Surface { surface, .. } | TargetId::Element { surface, .. } => {
                sv(surface.as_str())
            }
            _ => optional_sv(None),
        },
    )
}

/// The target's element id, or an absent view unless the target addresses an
/// element.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_target_view_element_id(
    v: *const LuminateTargetView,
) -> LuminateStringView {
    native_ref!(v, TargetId).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| match v {
            TargetId::Element { element, .. } => sv(element.as_str()),
            _ => optional_sv(None),
        },
    )
}

/// The target's group id, or an absent view unless the target addresses a
/// group.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_target_view_group_id(
    v: *const LuminateTargetView,
) -> LuminateStringView {
    native_ref!(v, TargetId).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| match v {
            TargetId::Group { group, .. } => sv(group.as_str()),
            _ => optional_sv(None),
        },
    )
}

/// Which state facet this value represents; one of the `LUMINATE_FACET_*`
/// values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_facet_value_kind(
    v: *const LuminateFacetValue,
) -> LuminateStateFacetKind {
    native_ref!(v, FacetValue).map_or(u32::MAX, |v| v.kind() as u32)
}

/// Whether the appearance value is a static colour or a running effect; one
/// of the `LUMINATE_APPEARANCE_*` values, or `LUMINATE_DISCRIMINANT_INVALID`
/// unless this is an appearance facet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_facet_value_appearance_kind(
    v: *const LuminateFacetValue,
) -> LuminateAppearanceKind {
    native_ref!(v, FacetValue)
        .and_then(|v| {
            if let FacetValue::Appearance(a) = v {
                Some(match a {
                    AppearanceState::Static(_) => 0,
                    AppearanceState::Effect(_) => 1,
                    AppearanceState::Mixed => 2,
                })
            } else {
                None
            }
        })
        .unwrap_or(u32::MAX)
}

/// Whether the effective-appearance value is off, a static colour, a running
/// effect, or an active frame stream; one of the
/// `LUMINATE_EFFECTIVE_APPEARANCE_*` values, or `LUMINATE_DISCRIMINANT_INVALID`
/// unless this is an effective-appearance facet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_facet_value_effective_appearance_kind(
    v: *const LuminateFacetValue,
) -> LuminateEffectiveAppearanceKind {
    native_ref!(v, FacetValue)
        .and_then(|v| {
            if let FacetValue::EffectiveAppearance(a) = v {
                Some(match a {
                    EffectiveAppearanceState::Off => 0,
                    EffectiveAppearanceState::Static(_) => 1,
                    EffectiveAppearanceState::Effect(_) => 2,
                    EffectiveAppearanceState::Streaming => 3,
                    EffectiveAppearanceState::Mixed => 4,
                })
            } else {
                None
            }
        })
        .unwrap_or(u32::MAX)
}

/// Borrowed static colour, or null unless this is a static appearance or
/// effective-appearance facet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_facet_value_colour(
    v: *const LuminateFacetValue,
) -> *const LuminateColour {
    native_ref!(v, FacetValue)
        .and_then(|v| match v {
            FacetValue::Appearance(AppearanceState::Static(c))
            | FacetValue::EffectiveAppearance(EffectiveAppearanceState::Static(c)) => Some(c),
            _ => None,
        })
        .map_or(ptr::null(), cast_ref)
}

/// Borrowed running effect, or null unless this is an effect appearance or
/// effective-appearance facet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_facet_value_effect(
    v: *const LuminateFacetValue,
) -> *const LuminateEffectView {
    native_ref!(v, FacetValue)
        .and_then(|v| match v {
            FacetValue::Appearance(AppearanceState::Effect(e))
            | FacetValue::EffectiveAppearance(EffectiveAppearanceState::Effect(e)) => Some(e),
            _ => None,
        })
        .map_or(ptr::null(), cast_ref)
}

/// Writes whether an appearance-slots facet contains every advertised slot.
///
/// Returns false and leaves `out_complete` unchanged unless this is an
/// appearance-slots facet or either pointer is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_facet_value_appearance_slots_complete(
    v: *const LuminateFacetValue,
    out_complete: *mut bool,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(complete), Some(out_complete)) = (
        native_ref!(v, FacetValue).and_then(|value| match value {
            FacetValue::AppearanceSlots(slots) => Some(slots.complete),
            _ => None,
        }),
        unsafe { out_complete.as_mut() },
    ) else {
        return false;
    };
    *out_complete = complete;
    true
}

/// Number of known values in an appearance-slots facet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_facet_value_appearance_slot_count(
    v: *const LuminateFacetValue,
) -> usize {
    native_ref!(v, FacetValue).map_or(0, |value| match value {
        FacetValue::AppearanceSlots(slots) => slots.values.len(),
        _ => 0,
    })
}

/// Borrowed appearance-slot value at `index`, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_facet_value_appearance_slot_at(
    v: *const LuminateFacetValue,
    index: usize,
) -> *const LuminateAppearanceSlotValue {
    native_ref!(v, FacetValue)
        .and_then(|value| match value {
            FacetValue::AppearanceSlots(slots) => slots.values.get(index),
            _ => None,
        })
        .map_or(ptr::null(), cast_ref)
}

/// Writes the brightness value.
///
/// Returns false and leaves `out_brightness` unchanged unless this is a
/// brightness facet or either pointer is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_facet_value_brightness(
    v: *const LuminateFacetValue,
    out_brightness: *mut u32,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(brightness), Some(out_brightness)) = (
        native_ref!(v, FacetValue).and_then(|v| match v {
            FacetValue::Brightness(brightness) => Some(*brightness),
            _ => None,
        }),
        unsafe { out_brightness.as_mut() },
    ) else {
        return false;
    };
    *out_brightness = brightness;
    true
}

/// Whether the target is emitting light; one of the `LUMINATE_EMISSION_*`
/// values, or `LUMINATE_DISCRIMINANT_INVALID` unless this is an emission
/// facet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_facet_value_emission(
    v: *const LuminateFacetValue,
) -> LuminateEmissionState {
    native_ref!(v, FacetValue)
        .and_then(|v| {
            if let FacetValue::Emission(x) = v {
                Some(*x as u32)
            } else {
                None
            }
        })
        .unwrap_or(u32::MAX)
}

/// Whether the target's physical power is on; one of the
/// `LUMINATE_PHYSICAL_POWER_*` values, or `LUMINATE_DISCRIMINANT_INVALID`
/// unless this is a physical-power facet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_facet_value_physical_power(
    v: *const LuminateFacetValue,
) -> LuminatePhysicalPowerState {
    native_ref!(v, FacetValue)
        .and_then(|v| {
            if let FacetValue::PhysicalPower(x) = v {
                Some(*x as u32)
            } else {
                None
            }
        })
        .unwrap_or(u32::MAX)
}

/// How this colour's channels are interpreted; one of the
/// `LUMINATE_COLOUR_ENCODING_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_colour_encoding(
    v: *const LuminateColour,
) -> LuminateColourEncoding {
    native_ref!(v, Colour).map_or(u32::MAX, |v| v.encoding() as u32)
}

#[unsafe(no_mangle)]
/// Number of channel values in this colour.
pub unsafe extern "C" fn luminate_colour_channel_count(v: *const LuminateColour) -> usize {
    native_ref!(v, Colour).map_or(0, |v| match v {
        Colour::Additive(channels) => channels.len(),
        Colour::Hsv { .. } | Colour::Hsl { .. } => 3,
        Colour::Cct { .. } | Colour::Monochrome { .. } => 1,
    })
}

#[unsafe(no_mangle)]
/// Copies the channel and raw value at `index` into caller-owned outputs.
///
/// Returns false for a null colour, null output, or out-of-range index. Both
/// outputs remain unchanged on failure.
pub unsafe extern "C" fn luminate_colour_channel_at(
    v: *const LuminateColour,
    index: usize,
    out_channel: *mut LuminateColourChannel,
    out_value: *mut u32,
) -> bool {
    if out_channel.is_null() || out_value.is_null() {
        return false;
    }
    let Some((channel, value)) = native_ref!(v, Colour).and_then(|v| match (v, index) {
        (Colour::Additive(channels), index) => channels
            .get(index)
            .map(|channel| (channel.channel, channel.value)),
        (Colour::Hsv { hue, .. } | Colour::Hsl { hue, .. }, 0) => Some((ColourChannel::Hue, *hue)),
        (Colour::Hsv { saturation, .. } | Colour::Hsl { saturation, .. }, 1) => {
            Some((ColourChannel::Saturation, *saturation))
        }
        (Colour::Hsv { value, .. }, 2) => Some((ColourChannel::Value, *value)),
        (Colour::Hsl { lightness, .. }, 2) => Some((ColourChannel::Lightness, *lightness)),
        (Colour::Cct { kelvin }, 0) => Some((ColourChannel::Temperature, *kelvin)),
        (Colour::Monochrome { intensity }, 0) => Some((ColourChannel::Intensity, *intensity)),
        _ => None,
    }) else {
        return false;
    };

    // SAFETY: both output pointers were checked before either is written.
    unsafe {
        *out_channel = channel as u32;
        *out_value = value;
    }
    true
}

/// Looks up a colour component by `LuminateColourChannel`.
///
/// Returns false for a null colour, invalid or absent channel, or null output.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_colour_value(
    v: *const LuminateColour,
    channel: LuminateColourChannel,
    out_value: *mut u32,
) -> bool {
    let Some(colour) = native_ref!(v, Colour) else {
        return false;
    };
    let Ok(channel) = colour_input::colour_channel(channel) else {
        return false;
    };
    let Some(value) = colour.channel(channel) else {
        return false;
    };
    if out_value.is_null() {
        return false;
    }
    // SAFETY: the caller supplied a writable output pointer.
    unsafe { *out_value = value };
    true
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;

fn effect_kind_value(effect: &Effect) -> LuminateEffectKind {
    match effect {
        Effect::Off => 0,
        Effect::Static { .. } => 1,
        Effect::Breathe { .. } => 2,
        Effect::Pulse { .. } => 3,
        Effect::Scanner { .. } => 4,
        Effect::Morph { .. } => 5,
        Effect::Spectrum { .. } => 6,
        Effect::Rainbow { .. } => 7,
        Effect::Hardware { .. } => 8,
        Effect::Strobe { .. } => 9,
    }
}

fn effect_rgb_value(effect: &Effect) -> Option<LuminateRgb> {
    match effect {
        Effect::Breathe { colour, .. }
        | Effect::Pulse { colour, .. }
        | Effect::Strobe { colour, .. }
        | Effect::Scanner { colour, .. } => Some(LuminateRgb {
            r: colour.r,
            g: colour.g,
            b: colour.b,
        }),
        _ => None,
    }
}

fn effect_static_colour_value(effect: &Effect) -> Option<&Colour> {
    match effect {
        Effect::Static { colour } => Some(colour),
        _ => None,
    }
}

fn effect_period_value(effect: &Effect) -> Option<u32> {
    match effect {
        Effect::Breathe { period_ms, .. }
        | Effect::Pulse { period_ms, .. }
        | Effect::Strobe { period_ms, .. }
        | Effect::Scanner { period_ms, .. }
        | Effect::Morph { period_ms, .. }
        | Effect::Spectrum { period_ms }
        | Effect::Rainbow { period_ms } => Some(*period_ms),
        _ => None,
    }
}

fn effect_rgbs(effect: &Effect) -> &[Rgb] {
    match effect {
        Effect::Morph { colours, .. } => colours,
        Effect::Hardware { arguments, .. } => &arguments.colours,
        _ => &[],
    }
}

fn effect_arguments(effect: &Effect) -> Option<&EffectArguments> {
    match effect {
        Effect::Hardware { arguments, .. } => Some(arguments),
        _ => None,
    }
}

macro_rules! effect_read_api {
    (
        $opaque:ty, $ref_fn:ident,
        $kind:ident, $rgb:ident, $static_colour:ident, $period:ident,
        $rgb_count:ident, $rgb_at:ident, $hardware_id:ident, $speed:ident,
        $duration:ident, $brightness:ident, $direction:ident, $choice:ident
    ) => {
        /// Which effect variant this value holds, or
        /// `LUMINATE_DISCRIMINANT_INVALID` for null.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $kind(value: *const $opaque) -> LuminateEffectKind {
            $ref_fn(value).map_or(u32::MAX, effect_kind_value)
        }

        /// Copies the effect's RGB colour into `out_colour`.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $rgb(value: *const $opaque, out_colour: *mut LuminateRgb) -> bool {
            let Some(colour) = $ref_fn(value).and_then(effect_rgb_value) else {
                return false;
            };
            if out_colour.is_null() {
                return false;
            }
            // SAFETY: the output pointer was checked before it was written.
            unsafe { *out_colour = colour };
            true
        }

        /// Borrowed generic colour, or null unless this is a static effect.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $static_colour(value: *const $opaque) -> *const LuminateColour {
            $ref_fn(value)
                .and_then(effect_static_colour_value)
                .map_or(ptr::null(), cast_ref)
        }

        /// Copies the animation period into `out_period_ms`.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $period(value: *const $opaque, out_period_ms: *mut u32) -> bool {
            let Some(period) = $ref_fn(value).and_then(effect_period_value) else {
                return false;
            };
            if out_period_ms.is_null() {
                return false;
            }
            // SAFETY: the output pointer was checked before it was written.
            unsafe { *out_period_ms = period };
            true
        }

        /// Number of RGB colours in a morph or hardware effect.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $rgb_count(value: *const $opaque) -> usize {
            $ref_fn(value).map_or(0, |effect| effect_rgbs(effect).len())
        }

        /// Copies the RGB colour at `index` into `out_colour`.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $rgb_at(
            value: *const $opaque,
            index: usize,
            out_colour: *mut LuminateRgb,
        ) -> bool {
            if out_colour.is_null() {
                return false;
            }
            let Some(colour) = $ref_fn(value).and_then(|effect| effect_rgbs(effect).get(index))
            else {
                return false;
            };
            // SAFETY: the output pointer was checked before it was written.
            unsafe {
                *out_colour = LuminateRgb {
                    r: colour.r,
                    g: colour.g,
                    b: colour.b,
                };
            }
            true
        }

        /// Hardware effect identifier, or an absent view.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $hardware_id(value: *const $opaque) -> LuminateStringView {
            $ref_fn(value)
                .and_then(|effect| match effect {
                    Effect::Hardware { id, .. } => Some(id.as_str()),
                    _ => None,
                })
                .map_or_else(|| optional_sv(None), sv)
        }

        /// Copies the optional hardware speed into `out_speed`.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $speed(value: *const $opaque, out_speed: *mut u16) -> bool {
            let Some(speed) = $ref_fn(value)
                .and_then(effect_arguments)
                .and_then(|arguments| arguments.speed)
            else {
                return false;
            };
            if out_speed.is_null() {
                return false;
            }
            // SAFETY: the output pointer was checked before it was written.
            unsafe { *out_speed = speed };
            true
        }

        /// Copies the optional hardware duration into `out_duration_ms`.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $duration(
            value: *const $opaque,
            out_duration_ms: *mut u32,
        ) -> bool {
            let Some(duration) = $ref_fn(value)
                .and_then(effect_arguments)
                .and_then(|arguments| arguments.duration_ms)
            else {
                return false;
            };
            if out_duration_ms.is_null() {
                return false;
            }
            // SAFETY: the output pointer was checked before it was written.
            unsafe { *out_duration_ms = duration };
            true
        }

        /// Copies the optional hardware brightness into `out_brightness`.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $brightness(
            value: *const $opaque,
            out_brightness: *mut u32,
        ) -> bool {
            let Some(brightness) = $ref_fn(value)
                .and_then(effect_arguments)
                .and_then(|arguments| arguments.brightness)
            else {
                return false;
            };
            if out_brightness.is_null() {
                return false;
            }
            // SAFETY: the output pointer was checked before it was written.
            unsafe { *out_brightness = brightness };
            true
        }

        /// Optional hardware direction, or `LUMINATE_DISCRIMINANT_INVALID`.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $direction(value: *const $opaque) -> LuminateEffectDirection {
            $ref_fn(value)
                .and_then(effect_arguments)
                .and_then(|arguments| arguments.direction)
                .map_or(u32::MAX, |direction| direction as u32)
        }

        /// Optional hardware choice, or an absent view.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $choice(value: *const $opaque) -> LuminateStringView {
            $ref_fn(value)
                .and_then(effect_arguments)
                .and_then(|arguments| arguments.choice.as_deref())
                .map_or_else(|| optional_sv(None), sv)
        }
    };
}

effect_read_api!(
    LuminateEffect,
    effect_ref,
    luminate_effect_kind,
    luminate_effect_rgb,
    luminate_effect_static_colour,
    luminate_effect_period_ms,
    luminate_effect_rgb_count,
    luminate_effect_rgb_at,
    luminate_effect_hardware_id,
    luminate_effect_speed,
    luminate_effect_duration_ms,
    luminate_effect_brightness,
    luminate_effect_direction,
    luminate_effect_choice
);

effect_read_api!(
    LuminateEffectView,
    effect_view_ref,
    luminate_effect_view_kind,
    luminate_effect_view_rgb,
    luminate_effect_view_static_colour,
    luminate_effect_view_period_ms,
    luminate_effect_view_rgb_count,
    luminate_effect_view_rgb_at,
    luminate_effect_view_hardware_id,
    luminate_effect_view_speed,
    luminate_effect_view_duration_ms,
    luminate_effect_view_brightness,
    luminate_effect_view_direction,
    luminate_effect_view_choice
);
