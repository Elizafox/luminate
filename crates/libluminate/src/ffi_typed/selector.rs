// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Selector-based operations shared by the unified C client.

#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "This exported C boundary validates pointers at entry; each remaining unsafe expression is governed by the function's caller-owned pointer contract."
)]

use super::effects::read_target;
use super::management::LuminateUnsupportedPolicy;
use super::*;

#[cfg(test)]
#[path = "selector_tests.rs"]
mod tests;

/// One target, collection, or explicit target list. `kind` selects `target`
/// (0), `collection_id` (1), or `targets`/`target_count` (2).
#[repr(C)]
pub struct LuminateSelectorInput {
    pub kind: u32,
    pub target: LuminateTarget,
    pub collection_id: *const c_char,
    pub targets: *const LuminateTarget,
    pub target_count: usize,
}

unsafe fn read_selector(value: &LuminateSelectorInput) -> Result<Selector, LuminateStatus> {
    match value.kind {
        0 => unsafe { read_target(ptr::from_ref(&value.target)) }.map(Selector::Target),
        1 => unsafe { read_required_str(value.collection_id, "selector collection_id") }
            .map(|id| Selector::Collection(CollectionId::new(id))),
        2 if value.target_count == 0 => Ok(Selector::Targets(Vec::new())),
        2 if value.targets.is_null() => {
            crate::ffi::set_last_error("selector targets pointer is null");
            Err(LuminateStatus::NullPointer)
        }
        2 => {
            // SAFETY: the C contract requires `target_count` initialized entries.
            let values = unsafe { std::slice::from_raw_parts(value.targets, value.target_count) };
            values
                .iter()
                .map(|target| unsafe { read_target(ptr::from_ref(target)) })
                .collect::<Result<Vec<_>, _>>()
                .map(Selector::Targets)
        }
        _ => {
            crate::ffi::set_last_error("invalid LuminateSelector kind");
            Err(LuminateStatus::InvalidArgument)
        }
    }
}

pub struct LuminateCollectionOutcome(pub(crate) CollectionOutcome);

null_safe_free!(
    "Releases a selector-operation outcome. Null is a no-op.",
    luminate_collection_outcome_free,
    LuminateCollectionOutcome
);

fn finish(result: Result<(), Error>) -> LuminateStatus {
    match result {
        Ok(()) => {
            clear_last_error();
            LuminateStatus::Ok
        }
        Err(error) => store_error(&error),
    }
}

fn write_outcome(
    result: Result<CollectionOutcome, Error>,
    output: *mut *mut LuminateCollectionOutcome,
) -> LuminateStatus {
    match result {
        Ok(outcome) => {
            unsafe { *output = Box::into_raw(Box::new(LuminateCollectionOutcome(outcome))) };
            clear_last_error();
            LuminateStatus::Ok
        }
        Err(error) => store_error(&error),
    }
}

pub(crate) unsafe fn selector_input(
    value: *const LuminateSelectorInput,
) -> Result<Selector, LuminateStatus> {
    let Some(value) = (unsafe { value.as_ref() }) else {
        crate::ffi::set_last_error("selector input pointer is null");
        return Err(LuminateStatus::NullPointer);
    };
    unsafe { read_selector(value) }
}

pub(crate) fn unsupported(
    has_value: bool,
    value: u32,
) -> Result<Option<UnsupportedPolicy>, LuminateStatus> {
    match (has_value, value) {
        (false, _) => Ok(None),
        (true, 0) => Ok(Some(UnsupportedPolicy::Skip)),
        (true, 1) => Ok(Some(UnsupportedPolicy::Reject)),
        (true, _) => {
            crate::ffi::set_last_error("invalid LuminateUnsupportedPolicy");
            Err(LuminateStatus::InvalidArgument)
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_refresh_state(
    client: *mut LuminateClient,
    device_id: *const c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let id = match unsafe { read_required_str(device_id, "device id") } {
            Ok(value) => DeviceId::new(value),
            Err(status) => return status,
        };
        match call_client(client, move |client| async move {
            client.refresh_state(id).await
        }) {
            Ok(result) => finish(result),
            Err(status) => status,
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_save_current(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let target = match unsafe { read_target(target) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        match call_client(client, move |client| async move {
            client.save_current(target).await
        }) {
            Ok(result) => finish(result),
            Err(status) => status,
        }
    })
}

macro_rules! selector_operation {
    ($name:ident, $method:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(
            client: *mut LuminateClient,
            selector: *const LuminateSelectorInput,
            output: *mut *mut LuminateCollectionOutcome,
        ) -> LuminateStatus {
            ffi_guard(|| {
                let client = match unsafe { client_ref(client) } {
                    Ok(value) => value,
                    Err(status) => return status,
                };
                let selector = match unsafe { selector_input(selector) } {
                    Ok(value) => value,
                    Err(status) => return status,
                };
                if output.is_null() {
                    crate::ffi::set_last_error("collection outcome output pointer is null");
                    return LuminateStatus::NullPointer;
                }
                match call_client(client, move |client| async move {
                    client.$method(selector).await
                }) {
                    Ok(result) => write_outcome(result, output),
                    Err(status) => status,
                }
            })
        }
    };
}

selector_operation!(luminate_client_clear_target_selector, clear_target_selector);
selector_operation!(luminate_client_save_current_selector, save_current_selector);
selector_operation!(
    luminate_client_restore_appearance_selector,
    restore_appearance_selector
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_set_brightness_selector(
    client: *mut LuminateClient,
    selector: *const LuminateSelectorInput,
    value: u32,
    has_policy: bool,
    policy: LuminateUnsupportedPolicy,
    output: *mut *mut LuminateCollectionOutcome,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let selector = match unsafe { selector_input(selector) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let policy = match unsupported(has_policy, policy) {
            Ok(value) => value,
            Err(status) => return status,
        };
        if output.is_null() {
            crate::ffi::set_last_error("collection outcome output pointer is null");
            return LuminateStatus::NullPointer;
        }
        match call_client(client, move |client| async move {
            client
                .set_brightness_selector(selector, value, policy)
                .await
        }) {
            Ok(result) => write_outcome(result, output),
            Err(status) => status,
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_set_effect_selector(
    client: *mut LuminateClient,
    selector: *const LuminateSelectorInput,
    effect: *const LuminateEffect,
    has_policy: bool,
    policy: LuminateUnsupportedPolicy,
    output: *mut *mut LuminateCollectionOutcome,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let selector = match unsafe { selector_input(selector) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let Some(effect) = (unsafe { effect.cast::<Effect>().as_ref() }) else {
            crate::ffi::set_last_error("effect pointer is null");
            return LuminateStatus::NullPointer;
        };
        let policy = match unsupported(has_policy, policy) {
            Ok(value) => value,
            Err(status) => return status,
        };
        if output.is_null() {
            crate::ffi::set_last_error("collection outcome output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let effect = effect.clone();
        match call_client(client, move |client| async move {
            client.set_effect_selector(selector, effect, policy).await
        }) {
            Ok(result) => write_outcome(result, output),
            Err(status) => status,
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_set_emission_selector(
    client: *mut LuminateClient,
    selector: *const LuminateSelectorInput,
    state: u32,
    output: *mut *mut LuminateCollectionOutcome,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let selector = match unsafe { selector_input(selector) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let state = match state {
            value if value == EmissionState::Dark as u32 => EmissionState::Dark,
            value if value == EmissionState::Emitting as u32 => EmissionState::Emitting,
            _ => {
                crate::ffi::set_last_error("invalid LuminateEmissionState");
                return LuminateStatus::InvalidArgument;
            }
        };
        if output.is_null() {
            crate::ffi::set_last_error("collection outcome output pointer is null");
            return LuminateStatus::NullPointer;
        }
        match call_client(client, move |client| async move {
            client.set_emission_selector(selector, state).await
        }) {
            Ok(result) => write_outcome(result, output),
            Err(status) => status,
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_outcome_applied_count(
    value: *const LuminateCollectionOutcome,
) -> usize {
    unsafe { value.as_ref() }.map_or(0, |value| value.0.applied.len())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_outcome_applied_at(
    value: *const LuminateCollectionOutcome,
    index: usize,
) -> *const LuminateTargetView {
    unsafe { value.as_ref() }
        .and_then(|value| value.0.applied.get(index))
        .map_or(ptr::null(), cast_ref)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_outcome_denied_count(
    value: *const LuminateCollectionOutcome,
) -> usize {
    unsafe { value.as_ref() }.map_or(0, |value| value.0.denied.len())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_outcome_denied_at(
    value: *const LuminateCollectionOutcome,
    index: usize,
) -> *const LuminateTargetView {
    unsafe { value.as_ref() }
        .and_then(|value| value.0.denied.get(index))
        .map_or(ptr::null(), cast_ref)
}
