// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! C bindings for wear-safe all-off planning.

use super::*;

null_safe_free!(
    "Releases a `LuminateAllOffPlan` returned by `luminate_all_off_plan`. Null \
     is a no-op.",
    luminate_all_off_plan_free,
    LuminateAllOffPlan
);

/// Builds a wear-safe all-off plan for a borrowed device.
///
/// # Safety
///
/// `device` must be a valid borrowed device pointer and `out` a valid non-null
/// out-pointer.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_all_off_plan(
    device: *const LuminateDevice,
    out: *mut *mut LuminateAllOffPlan,
) -> LuminateStatus {
    ffi_guard(|| {
        let Some(device) = native_ref!(device, Device) else {
            crate::ffi::set_last_error("device pointer is null");
            return LuminateStatus::NullPointer;
        };
        if out.is_null() {
            crate::ffi::set_last_error("all-off plan output pointer is null");
            return LuminateStatus::NullPointer;
        }
        // SAFETY: `out` was checked for null above.
        unsafe {
            *out = Box::into_raw(Box::new(LuminateAllOffPlan(crate::all_off_plan(device))));
        }
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Number of wear-safe targets in the plan.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_all_off_plan_target_count(
    plan: *const LuminateAllOffPlan,
) -> usize {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { plan.as_ref() }.map_or(0, |plan| plan.0.targets().len())
}

/// Borrowed wear-safe target at `index`, or null if out of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_all_off_plan_target_at(
    plan: *const LuminateAllOffPlan,
    index: usize,
) -> *const LuminateTargetView {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { plan.as_ref() }
        .and_then(|plan| plan.0.targets().get(index))
        .map_or(ptr::null(), cast_ref)
}

/// Number of off-capable wear-limited targets skipped by the plan.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_all_off_plan_skipped_persistent_count(
    plan: *const LuminateAllOffPlan,
) -> usize {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { plan.as_ref() }.map_or(0, |plan| plan.0.skipped_persistent().len())
}

/// Borrowed skipped wear-limited target at `index`, or null if out of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_all_off_plan_skipped_persistent_at(
    plan: *const LuminateAllOffPlan,
    index: usize,
) -> *const LuminateTargetView {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { plan.as_ref() }
        .and_then(|plan| plan.0.skipped_persistent().get(index))
        .map_or(ptr::null(), cast_ref)
}

#[cfg(test)]
#[path = "all_off_tests.rs"]
mod tests;
