// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use super::*;
use luminate_core::device::DeviceId;

#[test]
#[allow(
    clippy::undocumented_unsafe_blocks,
    reason = "The test deliberately exercises the C ABI pointer contract."
)]
fn c_plan_owns_results_and_handles_null_safely() {
    let device = Device {
        id: DeviceId::new("dark"),
        name: "Dark".to_owned(),
        vendor: None,
        model: None,
        provider_instance: None,
        surfaces: Vec::new(),
        groups: Vec::new(),
        capabilities: CapabilitySet::default(),
        category: None,
        physical_tags: Vec::new(),
        host_attached: false,
        notes: Vec::new(),
        warnings: Vec::new(),
    };
    let mut plan = ptr::null_mut();

    // SAFETY: the function validates both pointers before use.
    assert_eq!(
        unsafe { luminate_all_off_plan(ptr::null(), &raw mut plan) },
        LuminateStatus::NullPointer
    );
    // SAFETY: the function validates the null output pointer before use.
    assert_eq!(
        unsafe { luminate_all_off_plan(cast_ref(&device), ptr::null_mut()) },
        LuminateStatus::NullPointer
    );

    // SAFETY: the borrowed device and writable out-pointer remain valid.
    let status = unsafe { luminate_all_off_plan(cast_ref(&device), &raw mut plan) };
    assert_eq!(status, LuminateStatus::Ok);
    // SAFETY: `plan` is the live owned handle returned above.
    assert_eq!(unsafe { luminate_all_off_plan_target_count(plan) }, 0);
    // SAFETY: `plan` is live; the out-of-range index is handled by the accessor.
    assert!(unsafe { luminate_all_off_plan_target_at(plan, 0) }.is_null());
    // SAFETY: null is explicitly supported by the accessor.
    assert!(unsafe { luminate_all_off_plan_target_at(ptr::null(), 0) }.is_null());
    // SAFETY: null is explicitly supported by the accessors and free.
    let skipped = unsafe { luminate_all_off_plan_skipped_persistent_count(ptr::null()) };
    assert_eq!(skipped, 0);
    // SAFETY: `plan` is live; the out-of-range index is handled by the accessor.
    assert!(unsafe { luminate_all_off_plan_skipped_persistent_at(plan, 0) }.is_null());
    // SAFETY: null is explicitly supported by the accessor.
    assert!(unsafe { luminate_all_off_plan_skipped_persistent_at(ptr::null(), 0) }.is_null());
    // SAFETY: `plan` is freed exactly once.
    unsafe { luminate_all_off_plan_free(plan) };
}
