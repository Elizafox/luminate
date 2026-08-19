// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Typed C access to named appearance slots.

use super::effects::read_target;
use super::*;

/// One slot assignment supplied to a mutation or scene definition.
#[repr(C)]
pub struct LuminateAppearanceSlotInput {
    pub slot_id: *const c_char,
    pub effect: *const LuminateEffect,
}

pub(crate) unsafe fn read_appearance_slot_inputs(
    values: *const LuminateAppearanceSlotInput,
    count: usize,
) -> Result<Vec<AppearanceSlotValue>, LuminateStatus> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if values.is_null() {
        crate::ffi::set_last_error("appearance slot values pointer is null");
        return Err(LuminateStatus::NullPointer);
    }
    // SAFETY: upheld by the enclosing public function's pointer contract.
    unsafe { std::slice::from_raw_parts(values, count) }
        .iter()
        .map(|value| {
            // SAFETY: each input's strings are readable NUL-terminated strings.
            let slot = unsafe { read_required_str(value.slot_id, "slot_id") }?;
            let effect = effect_ref(value.effect).cloned().ok_or_else(|| {
                crate::ffi::set_last_error("appearance slot effect pointer is null");
                LuminateStatus::NullPointer
            })?;
            Ok(AppearanceSlotValue {
                slot: AppearanceSlotId::new(slot),
                effect,
            })
        })
        .collect()
}

/// Applies one logical appearance-slot mutation to a concrete surface.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_set_appearance_slots(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    values: *const LuminateAppearanceSlotInput,
    value_count: usize,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        // SAFETY: upheld by the enclosing function's pointer contract.
        let target = match unsafe { read_target(target) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        // SAFETY: upheld by the enclosing function's pointer contract.
        let values = match unsafe { read_appearance_slot_inputs(values, value_count) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        let result = match call_client(client, move |client| async move {
            client.set_appearance_slots(target, values).await
        }) {
            Ok(value) => value,
            Err(error) => return error,
        };
        match result {
            Ok(()) => {
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}

/// Borrowed appearance-slots capability, or null when absent.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_capability_set_appearance_slots(
    value: *const LuminateCapabilitySet,
) -> *const LuminateAppearanceSlotsCapability {
    native_ref!(value, CapabilitySet)
        .and_then(|value| value.appearance_slots.as_ref())
        .map_or(ptr::null(), cast_ref)
}

/// Completeness policy; one of `LUMINATE_APPEARANCE_SLOT_UPDATE_*`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_appearance_slots_update_policy(
    value: *const LuminateAppearanceSlotsCapability,
) -> LuminateAppearanceSlotUpdatePolicy {
    native_ref!(value, AppearanceSlotsCapability)
        .map_or(u32::MAX, |value| value.update_policy as u32)
}

vec_accessors!(
    "Number of appearance slots advertised.",
    "Borrowed slot descriptor at `index`, or null.",
    luminate_appearance_slot_count,
    luminate_appearance_slot_at,
    LuminateAppearanceSlotsCapability,
    AppearanceSlotsCapability,
    LuminateAppearanceSlotDescriptor,
    slots
);

str_accessor!(
    "Stable slot identifier.",
    luminate_appearance_slot_id,
    LuminateAppearanceSlotDescriptor,
    AppearanceSlotDescriptor,
    |value: &AppearanceSlotDescriptor| value.id.as_str()
);
str_accessor!(
    "Human-readable slot name.",
    luminate_appearance_slot_name,
    LuminateAppearanceSlotDescriptor,
    AppearanceSlotDescriptor,
    |value: &AppearanceSlotDescriptor| value.name.as_str()
);

/// Appearance operations accepted by this slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_appearance_slot_appearance(
    value: *const LuminateAppearanceSlotDescriptor,
) -> *const LuminateAppearanceCapability {
    native_ref!(value, AppearanceSlotDescriptor)
        .map_or(ptr::null(), |value| cast_ref(&value.appearance))
}

/// Persistence kind for this slot; one of `LUMINATE_PERSISTENCE_*`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_appearance_slot_persistence_kind(
    value: *const LuminateAppearanceSlotDescriptor,
) -> u32 {
    native_ref!(value, AppearanceSlotDescriptor).map_or(u32::MAX, |value| match value.persistence {
        PersistenceCapability::None => 0,
        PersistenceCapability::CurrentState { .. } => 1,
        PersistenceCapability::Profiles { .. } => 2,
    })
}

string_vec_accessors!(
    "Number of slot notes.",
    "Slot note at `index`.",
    luminate_appearance_slot_note_count,
    luminate_appearance_slot_note_at,
    LuminateAppearanceSlotDescriptor,
    AppearanceSlotDescriptor,
    notes
);
string_vec_accessors!(
    "Number of slot warnings.",
    "Slot warning at `index`.",
    luminate_appearance_slot_warning_count,
    luminate_appearance_slot_warning_at,
    LuminateAppearanceSlotDescriptor,
    AppearanceSlotDescriptor,
    warnings
);

vec_accessors!(
    "Number of accepted colour models.",
    "Borrowed colour capability at `index`.",
    luminate_appearance_capability_colour_count,
    luminate_appearance_capability_colour_at,
    LuminateAppearanceCapability,
    AppearanceCapability,
    LuminateColourCapability,
    colour
);

/// Slot CCT emulation policy; one of `LUMINATE_CCT_EMULATION_*`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_appearance_capability_cct_emulation(
    value: *const LuminateAppearanceCapability,
) -> u32 {
    native_ref!(value, AppearanceCapability).map_or(u32::MAX, |value| value.cct_emulation as u32)
}

/// Borrowed hardware-effects capability accepted by the slot, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_appearance_capability_hardware_effects(
    value: *const LuminateAppearanceCapability,
) -> *const LuminateHardwareEffectsCapability {
    native_ref!(value, AppearanceCapability)
        .and_then(|value| value.hardware_effects.as_ref())
        .map_or(ptr::null(), cast_ref)
}

/// Slot identifier in one observed or scene value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_appearance_slot_value_id(
    value: *const LuminateAppearanceSlotValue,
) -> LuminateStringView {
    native_ref!(value, AppearanceSlotValue)
        .map_or(optional_sv(None), |value| sv(value.slot.as_str()))
}

/// Borrowed view of a slot value's effect.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_appearance_slot_value_effect(
    value: *const LuminateAppearanceSlotValue,
) -> *const LuminateEffectView {
    native_ref!(value, AppearanceSlotValue).map_or(ptr::null(), |value| cast_ref(&value.effect))
}

#[cfg(test)]
mod tests {
    use super::state::luminate_effect_view_kind;
    use super::*;
    use luminate_core::capability::ColourCapability;
    use luminate_core::rgb::Rgb;

    #[test]
    fn capability_and_value_accessors_preserve_order_metadata_and_null_sentinels() {
        let slots = AppearanceSlotsCapability {
            slots: vec![AppearanceSlotDescriptor {
                id: AppearanceSlotId::new("ac"),
                name: "AC".to_owned(),
                appearance: AppearanceCapability {
                    colour: vec![ColourCapability::rgb8()],
                    ..AppearanceCapability::default()
                },
                persistence: PersistenceCapability::None,
                notes: vec!["Stored by firmware".to_owned()],
                warnings: vec!["Write sparingly".to_owned()],
            }],
            update_policy: AppearanceSlotUpdatePolicy::PartialIfKnown,
        };
        let capabilities = CapabilitySet {
            appearance_slots: Some(slots),
            ..CapabilitySet::default()
        };
        let value = AppearanceSlotValue {
            slot: AppearanceSlotId::new("ac"),
            effect: Effect::Static {
                colour: Colour::rgb(Rgb::new(1, 2, 3)),
            },
        };

        // SAFETY: every opaque pointer views a live value of the matching type.
        unsafe {
            let capability = luminate_capability_set_appearance_slots(cast_ref(&capabilities));
            assert_eq!(luminate_appearance_slots_update_policy(capability), 1);
            assert_eq!(luminate_appearance_slot_count(capability), 1);
            let descriptor = luminate_appearance_slot_at(capability, 0);
            assert_eq!(luminate_appearance_slot_note_count(descriptor), 1);
            assert_eq!(luminate_appearance_slot_warning_count(descriptor), 1);
            assert_eq!(
                luminate_appearance_capability_colour_count(luminate_appearance_slot_appearance(
                    descriptor
                )),
                1
            );
            assert!(luminate_appearance_slot_at(capability, 1).is_null());
            assert_eq!(
                luminate_appearance_slots_update_policy(ptr::null()),
                u32::MAX
            );

            let value = cast_ref::<_, LuminateAppearanceSlotValue>(&value);
            let effect = luminate_appearance_slot_value_effect(value);
            assert!(!effect.is_null());
            assert_eq!(luminate_effect_view_kind(effect), 1);
        }
    }
}
