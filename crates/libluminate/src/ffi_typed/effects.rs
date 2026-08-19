// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::colour_input::{LuminateColourInput, read_colour};
use super::management::LuminateManagementChangeSetView;
use super::*;

fn boxed_effect(effect: Effect, out: *mut *mut LuminateEffect) -> LuminateStatus {
    if out.is_null() {
        crate::ffi::set_last_error("effect output pointer is null");
        return LuminateStatus::NullPointer;
    }
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { *out = Box::into_raw(Box::new(effect)).cast() }
    clear_last_error();
    LuminateStatus::Ok
}

/// Deep-copies a borrowed effect view into an owned effect. Release the result
/// with `luminate_effect_free`.
///
/// Returns `LUMINATE_STATUS_NULL_POINTER` when either pointer is null.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_view_clone(
    view: *const LuminateEffectView,
    out_effect: *mut *mut LuminateEffect,
) -> LuminateStatus {
    ffi_guard(|| {
        let Some(effect) = effect_view_ref(view) else {
            crate::ffi::set_last_error("effect view pointer is null");
            return LuminateStatus::NullPointer;
        };
        boxed_effect(effect.clone(), out_effect)
    })
}

/// Deep-copies a borrowed generic colour into an owned static effect. Release
/// the result with `luminate_effect_free`.
///
/// Returns `LUMINATE_STATUS_NULL_POINTER` when either pointer is null.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_create_static_from_colour(
    colour: *const LuminateColour,
    out_effect: *mut *mut LuminateEffect,
) -> LuminateStatus {
    ffi_guard(|| {
        let Some(colour) = (native_ref!(colour, Colour)) else {
            crate::ffi::set_last_error("colour pointer is null");
            return LuminateStatus::NullPointer;
        };
        boxed_effect(
            Effect::Static {
                colour: colour.clone(),
            },
            out_effect,
        )
    })
}

/// Creates an owned off effect. Release with `luminate_effect_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_create_off(
    out: *mut *mut LuminateEffect,
) -> LuminateStatus {
    boxed_effect(Effect::Off, out)
}

/// Defines an owned-effect constructor for a colour+period effect variant.
/// `$doc` becomes the generated function's doc comment.
macro_rules! colour_ctor {
    ($doc:literal, $fn:ident,$variant:ident) => {
        #[doc = $doc]
        #[must_use]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn(
            c: LuminateRgb,
            period: u32,
            out: *mut *mut LuminateEffect,
        ) -> LuminateStatus {
            boxed_effect(
                Effect::$variant {
                    colour: Rgb::new(c.r, c.g, c.b),
                    period_ms: period,
                },
                out,
            )
        }
    };
}

/// Creates an owned static-colour effect. Release with `luminate_effect_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_create_static(
    colour: *const LuminateColourInput,
    out: *mut *mut LuminateEffect,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let Some(colour) = (unsafe { colour.as_ref() }) else {
            crate::ffi::set_last_error("colour input pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: nested pointers are governed by the input contract.
        let colour = match unsafe { read_colour(colour) } {
            Ok(value) => value,
            Err(status) => return status,
        };

        boxed_effect(Effect::Static { colour }, out)
    })
}

colour_ctor!(
    "Creates an owned breathe effect. Release with `luminate_effect_free`.",
    luminate_effect_create_breathe,
    Breathe
);
colour_ctor!(
    "Creates an owned pulse effect. Release with `luminate_effect_free`.",
    luminate_effect_create_pulse,
    Pulse
);
colour_ctor!(
    "Creates an owned strobe effect. Release with `luminate_effect_free`.",
    luminate_effect_create_strobe,
    Strobe
);
colour_ctor!(
    "Creates an owned scanner effect. Release with `luminate_effect_free`.",
    luminate_effect_create_scanner,
    Scanner
);
/// Creates an owned spectrum effect. Release with `luminate_effect_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_create_spectrum(
    period: u32,
    out: *mut *mut LuminateEffect,
) -> LuminateStatus {
    boxed_effect(Effect::Spectrum { period_ms: period }, out)
}

/// Creates an owned rainbow effect. Release with `luminate_effect_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_create_rainbow(
    period: u32,
    out: *mut *mut LuminateEffect,
) -> LuminateStatus {
    boxed_effect(Effect::Rainbow { period_ms: period }, out)
}

/// Creates an owned morph effect over the given colours. Release with
/// `luminate_effect_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_create_morph(
    colours: *const LuminateRgb,
    count: usize,
    period: u32,
    out: *mut *mut LuminateEffect,
) -> LuminateStatus {
    ffi_guard(|| {
        if colours.is_null() && count != 0 {
            crate::ffi::set_last_error("morph colours pointer is null");
            return LuminateStatus::NullPointer;
        }
        let values = if count == 0 {
            Vec::new()
        } else {
            // SAFETY: upheld by the enclosing function's documented C pointer contract.
            unsafe { std::slice::from_raw_parts(colours, count) }
                .iter()
                .map(|v| Rgb::new(v.r, v.g, v.b))
                .collect()
        };
        boxed_effect(
            Effect::Morph {
                colours: values,
                period_ms: period,
            },
            out,
        )
    })
}

/// Creates an owned hardware effect selected by `id`, with no arguments set.
/// Release with `luminate_effect_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_create_hardware(
    id: *const c_char,
    out: *mut *mut LuminateEffect,
) -> LuminateStatus {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let id = match unsafe { read_required_str(id, "effect_id") } {
        Ok(v) => v.to_owned(),
        Err(e) => return e,
    };
    boxed_effect(
        Effect::Hardware {
            id: HardwareEffectId::new(id),
            arguments: EffectArguments::default(),
        },
        out,
    )
}

fn effect_mut(v: *mut LuminateEffect) -> Result<&'static mut Effect, LuminateStatus> {
    if v.is_null() {
        crate::ffi::set_last_error("effect pointer is null");
        Err(LuminateStatus::NullPointer)
    } else {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        Ok(unsafe { &mut *v.cast::<Effect>() })
    }
}

fn hardware_args(v: *mut LuminateEffect) -> Result<&'static mut EffectArguments, LuminateStatus> {
    match effect_mut(v)? {
        Effect::Hardware { arguments, .. } => Ok(arguments),
        _ => {
            crate::ffi::set_last_error("effect is not a hardware effect");
            Err(LuminateStatus::InvalidArgument)
        }
    }
}

/// Appends a colour to a hardware effect's colour list.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_hardware_add_colour(
    v: *mut LuminateEffect,
    c: LuminateRgb,
) -> LuminateStatus {
    match hardware_args(v) {
        Ok(a) => {
            a.colours.push(Rgb::new(c.r, c.g, c.b));
            clear_last_error();
            LuminateStatus::Ok
        }
        Err(e) => e,
    }
}

/// Defines a setter for one optional hardware-effect argument field. `$doc`
/// becomes the generated function's doc comment.
macro_rules! setter {
    ($doc:literal, $fn:ident,$field:ident,$ty:ty) => {
        #[doc = $doc]
        #[must_use]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn(v: *mut LuminateEffect, x: $ty) -> LuminateStatus {
            match hardware_args(v) {
                Ok(a) => {
                    a.$field = Some(x);
                    clear_last_error();
                    LuminateStatus::Ok
                }
                Err(e) => e,
            }
        }
    };
}

setter!(
    "Sets the speed parameter on a hardware effect created by \
     `luminate_effect_create_hardware`.",
    luminate_effect_hardware_set_speed,
    speed,
    u16
);
setter!(
    "Sets the duration-in-milliseconds parameter on a hardware effect \
     created by `luminate_effect_create_hardware`.",
    luminate_effect_hardware_set_duration_ms,
    duration_ms,
    u32
);
setter!(
    "Sets the brightness parameter on a hardware effect created by \
     `luminate_effect_create_hardware`.",
    luminate_effect_hardware_set_brightness,
    brightness,
    u32
);
/// Sets the direction parameter on a hardware effect created by
/// `luminate_effect_create_hardware`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_hardware_set_direction(
    v: *mut LuminateEffect,
    x: u32,
) -> LuminateStatus {
    let d = match x {
        0 => EffectDirection::Forward,
        1 => EffectDirection::Reverse,
        2 => EffectDirection::Clockwise,
        3 => EffectDirection::CounterClockwise,
        4 => EffectDirection::Inward,
        5 => EffectDirection::Outward,
        6 => EffectDirection::Random,
        _ => {
            crate::ffi::set_last_error("invalid effect direction");
            return LuminateStatus::InvalidArgument;
        }
    };
    match hardware_args(v) {
        Ok(a) => {
            a.direction = Some(d);
            clear_last_error();
            LuminateStatus::Ok
        }
        Err(e) => e,
    }
}

/// Sets the choice parameter on a hardware effect created by
/// `luminate_effect_create_hardware`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_hardware_set_choice(
    v: *mut LuminateEffect,
    x: *const c_char,
) -> LuminateStatus {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let x = match unsafe { read_required_str(x, "choice") } {
        Ok(v) => v.to_owned(),
        Err(e) => return e,
    };
    match hardware_args(v) {
        Ok(a) => {
            a.choice = Some(x);
            clear_last_error();
            LuminateStatus::Ok
        }
        Err(e) => e,
    }
}

pub(crate) unsafe fn read_target(v: *const LuminateTarget) -> Result<TargetId, LuminateStatus> {
    if v.is_null() {
        crate::ffi::set_last_error("target pointer is null");
        return Err(LuminateStatus::NullPointer);
    }
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let v = unsafe { &*v };
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let read = |p: *const c_char, n: &'static str| unsafe {
        if p.is_null() {
            Ok(None)
        } else {
            read_required_str(p, n).map(Some)
        }
    };
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let device = unsafe { read_required_str(v.device_id, "device_id") }?;
    let surface = read(v.surface_id, "surface_id")?;
    let element = read(v.element_id, "element_id")?;
    let group = read(v.group_id, "group_id")?;
    TargetId::from_parts(device, surface, element, group)
        .map_err(|e| store_error(&Error::InvalidArgument(e.to_owned())))
}

/// Defines a target-addressed client operation that blocks on the async
/// client method `$method`. `$doc` becomes the generated function's doc
/// comment.
///
// cbindgen can't see through this macro, which is why its expansions
// (`luminate_client_set_brightness`, `_clear_target`, `_set_off`, and
// `_restore_appearance`) are
// hand-declared again in the `trailer` of `cbindgen.toml`. Keep the two in
// sync if this macro's signature changes.
macro_rules! target_operation {
    ($doc:literal, $fn:ident, $method:ident $(, $arg:ident: $ty:ty)*) => {
        #[doc = $doc]
        #[must_use]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn(
            client: *mut LuminateClient,
            target: *const LuminateTarget,
            $($arg: $ty,)*
        ) -> LuminateStatus {
            ffi_guard(|| {
                // SAFETY: upheld by the enclosing function's documented C pointer contract.
                let client = match unsafe { client_ref(client) } {
                    Ok(v) => v,
                    Err(e) => return e,
                };
                // SAFETY: upheld by the enclosing function's documented C pointer contract.
                let target = match unsafe { read_target(target) } {
                    Ok(v) => v,
                    Err(e) => return e,
                };
                let result = match call_client(client, move |c| async move {
                    c.$method(target $(, $arg)*).await
                }) {
                    Ok(v) => v,
                    Err(e) => return e,
                };
                match result {
                    Ok(()) => {
                        clear_last_error();
                        LuminateStatus::Ok
                    }
                    Err(e) => store_error(&e),
                }
            })
        }
    };
}
target_operation!(
    "Sets brightness on the given target, in the raw units of its advertised \
     brightness capability (0 to `luminate_capability_set_brightness_maximum`, \
     not a 0-100 percentage).",
    luminate_client_set_brightness,
    set_brightness,
    value: u32
);
target_operation!(
    "Clears any desired-state override for the given target, returning it \
     to its default.",
    luminate_client_clear_target,
    clear_target
);
target_operation!(
    "Turns off (dark) the given target.",
    luminate_client_set_off,
    set_off
);
target_operation!(
    "Reapplies the given target's last-known configured appearance.",
    luminate_client_restore_appearance,
    restore_appearance
);

/// Sets whether the given target emits light.
///
/// cbindgen:ignore
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_set_emission(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    state: u32,
) -> LuminateStatus {
    ffi_guard(|| {
        let state = match state {
            value if value == EmissionState::Dark as u32 => EmissionState::Dark,
            value if value == EmissionState::Emitting as u32 => EmissionState::Emitting,
            _ => {
                crate::ffi::set_last_error("invalid LuminateEmissionState");
                return LuminateStatus::InvalidArgument;
            }
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let target = match unsafe { read_target(target) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        let result = match call_client(client, move |client| async move {
            client.set_emission(target, state).await
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
/// Applies the given effect to the target.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_set_effect(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    effect: *const LuminateEffect,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let target = match unsafe { read_target(target) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let effect = match effect_ref(effect) {
            Some(v) => v.clone(),
            None => {
                crate::ffi::set_last_error("effect pointer is null");
                return LuminateStatus::NullPointer;
            }
        };
        let result = match call_client(client, move |c| async move {
            c.set_effect(target, effect).await
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(()) => {
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(e) => store_error(&e),
        }
    })
}

/// Which payload this event carries; one of the `LUMINATE_EVENT_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_event_kind(v: *const LuminateEvent) -> LuminateEventKind {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(u32::MAX, |v| match v.0 {
        Event::TopologyChanged { .. } => 0,
        Event::StateChanged { .. } => 1,
        Event::ShmStreamEnded { .. } => 2,
        Event::ConfigurationChanged { .. } => 3,
        Event::ScenesChanged => 4,
        Event::TransitionsChanged { .. } => 5,
        Event::ResyncRequired => 6,
    })
}

/// Number of devices in a topology-changed event, or 0 unless this is a
/// topology event.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_event_topology_device_count(v: *const LuminateEvent) -> usize {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(0, |v| match &v.0 {
        Event::TopologyChanged { devices } => devices.len(),
        Event::ResyncRequired
        | Event::StateChanged { .. }
        | Event::ShmStreamEnded { .. }
        | Event::ConfigurationChanged { .. }
        | Event::ScenesChanged
        | Event::TransitionsChanged { .. } => 0,
    })
}

/// Device id at `index` in a topology-changed event, or an absent view if
/// out of range or not a topology event.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_event_topology_device_at(
    v: *const LuminateEvent,
    i: usize,
) -> LuminateStringView {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }
        .and_then(|v| match &v.0 {
            Event::TopologyChanged { devices } => devices.get(i),
            Event::ResyncRequired
            | Event::StateChanged { .. }
            | Event::ShmStreamEnded { .. }
            | Event::ConfigurationChanged { .. }
            | Event::ScenesChanged
            | Event::TransitionsChanged { .. } => None,
        })
        .map_or(
            LuminateStringView {
                data: ptr::null(),
                len: 0,
            },
            |v| sv(v.as_str()),
        )
}

/// Number of devices in a state-changed event, or 0 unless this is a state
/// event.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_event_state_device_count(v: *const LuminateEvent) -> usize {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(0, |v| match &v.0 {
        Event::ResyncRequired
        | Event::TopologyChanged { .. }
        | Event::ShmStreamEnded { .. }
        | Event::ConfigurationChanged { .. }
        | Event::ScenesChanged
        | Event::TransitionsChanged { .. } => 0,
        Event::StateChanged { devices } => devices.len(),
    })
}

/// Device id at `index` in a state-changed event, or an absent view if out
/// of range or not a state event.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_event_state_device_at(
    v: *const LuminateEvent,
    i: usize,
) -> LuminateStringView {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }
        .and_then(|v| match &v.0 {
            Event::ResyncRequired
            | Event::TopologyChanged { .. }
            | Event::ShmStreamEnded { .. }
            | Event::ConfigurationChanged { .. }
            | Event::ScenesChanged
            | Event::TransitionsChanged { .. } => None,
            Event::StateChanged { devices } => devices.get(i),
        })
        .map_or(
            LuminateStringView {
                data: ptr::null(),
                len: 0,
            },
            |v| sv(v.as_str()),
        )
}

/// Borrowed target of a shared-memory-stream-ended event, or null unless
/// this is one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_event_shm_stream_target(
    v: *const LuminateEvent,
) -> *const LuminateTargetView {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(ptr::null(), |v| match &v.0 {
        Event::ShmStreamEnded { target, .. } => cast_ref(target),
        Event::ResyncRequired
        | Event::TopologyChanged { .. }
        | Event::StateChanged { .. }
        | Event::ConfigurationChanged { .. }
        | Event::ScenesChanged
        | Event::TransitionsChanged { .. } => ptr::null(),
    })
}

/// Writes the generation the ended stream was negotiated under.
///
/// Returns false and leaves `out_generation` unchanged unless this is a
/// shared-memory-stream-ended event or either pointer is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_event_shm_stream_generation(
    v: *const LuminateEvent,
    out_generation: *mut u32,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(generation), Some(out_generation)) = (
        unsafe { v.as_ref() }.and_then(|v| match &v.0 {
            Event::ShmStreamEnded { generation, .. } => Some(*generation),
            Event::ResyncRequired
            | Event::TopologyChanged { .. }
            | Event::StateChanged { .. }
            | Event::ConfigurationChanged { .. }
            | Event::ScenesChanged
            | Event::TransitionsChanged { .. } => None,
        }),
        unsafe { out_generation.as_mut() },
    ) else {
        return false;
    };
    *out_generation = generation;
    true
}

/// Number of dirty transition identifiers, or 0 unless this is a transition
/// event. Zero in a transition event requests a full refresh.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_event_transition_count(v: *const LuminateEvent) -> usize {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(0, |v| match &v.0 {
        Event::TransitionsChanged { transitions } => transitions.len(),
        _ => 0,
    })
}

/// Borrowed dirty transition identifier at `index`, or an absent view.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_event_transition_id_at(
    v: *const LuminateEvent,
    index: usize,
) -> LuminateStringView {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }
        .and_then(|v| match &v.0 {
            Event::TransitionsChanged { transitions } => transitions.get(index),
            _ => None,
        })
        .map_or(
            LuminateStringView {
                data: ptr::null(),
                len: 0,
            },
            |id| sv(id.as_str()),
        )
}

/// Borrowed redacted change set carried by a configuration-changed event, or
/// null unless this is one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_event_configuration_changes(
    v: *const LuminateEvent,
) -> *const LuminateManagementChangeSetView {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(ptr::null(), |v| match &v.0 {
        Event::ConfigurationChanged { changes } => cast_ref(changes),
        Event::ResyncRequired
        | Event::TopologyChanged { .. }
        | Event::StateChanged { .. }
        | Event::ShmStreamEnded { .. }
        | Event::ScenesChanged
        | Event::TransitionsChanged { .. } => ptr::null(),
    })
}

#[cfg(test)]
#[path = "effects_tests.rs"]
mod tests;
