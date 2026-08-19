// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::borrow_as_ptr,
    reason = "these tests deliberately call the public C ABI with borrowed Rust fixtures"
)]

use super::*;
use crate::ffi::FfiClient;
use std::ffi::CString;

fn device_target(id: &CString) -> LuminateTarget {
    LuminateTarget {
        device_id: id.as_ptr(),
        surface_id: ptr::null(),
        element_id: ptr::null(),
        group_id: ptr::null(),
    }
}

#[test]
fn selectors_decode_single_collection_empty_and_multiple_targets() {
    let device = CString::new("desk").expect("C string");
    let collection = CString::new("room").expect("C string");
    let target = device_target(&device);

    let single = LuminateSelectorInput {
        kind: 0,
        target,
        collection_id: ptr::null(),
        targets: ptr::null(),
        target_count: 0,
    };
    assert!(matches!(
        unsafe { read_selector(&single) },
        Ok(Selector::Target(_))
    ));

    let collection = LuminateSelectorInput {
        kind: 1,
        target,
        collection_id: collection.as_ptr(),
        targets: ptr::null(),
        target_count: 0,
    };
    assert!(matches!(
        unsafe { read_selector(&collection) },
        Ok(Selector::Collection(_))
    ));

    let empty = LuminateSelectorInput {
        kind: 2,
        target,
        collection_id: ptr::null(),
        targets: ptr::null(),
        target_count: 0,
    };
    assert!(
        matches!(unsafe { read_selector(&empty) }, Ok(Selector::Targets(values)) if values.is_empty())
    );

    let targets = [target, target];
    let multiple = LuminateSelectorInput {
        kind: 2,
        target,
        collection_id: ptr::null(),
        targets: targets.as_ptr(),
        target_count: targets.len(),
    };
    assert!(
        matches!(unsafe { read_selector(&multiple) }, Ok(Selector::Targets(values)) if values.len() == 2)
    );
}

#[test]
fn selector_inputs_and_policies_reject_invalid_values() {
    let device = CString::new("desk").expect("C string");
    let target = device_target(&device);
    let null_targets = LuminateSelectorInput {
        kind: 2,
        target,
        collection_id: ptr::null(),
        targets: ptr::null(),
        target_count: 1,
    };
    assert_eq!(
        unsafe { read_selector(&null_targets) },
        Err(LuminateStatus::NullPointer)
    );
    let invalid = LuminateSelectorInput {
        kind: 99,
        ..null_targets
    };
    assert_eq!(
        unsafe { read_selector(&invalid) },
        Err(LuminateStatus::InvalidArgument)
    );
    assert_eq!(unsupported(false, 99), Ok(None));
    assert_eq!(unsupported(true, 0), Ok(Some(UnsupportedPolicy::Skip)));
    assert_eq!(unsupported(true, 1), Ok(Some(UnsupportedPolicy::Reject)));
    assert_eq!(unsupported(true, 2), Err(LuminateStatus::InvalidArgument));
}

#[test]
fn selector_operations_accept_typical_inputs_before_dispatch() {
    let client = FfiClient::stopped();
    let client = (&raw const client).cast::<LuminateClient>().cast_mut();
    let device = CString::new("desk").expect("C string");
    let selector = LuminateSelectorInput {
        kind: 0,
        target: device_target(&device),
        collection_id: ptr::null(),
        targets: ptr::null(),
        target_count: 0,
    };
    let mut outcome = ptr::null_mut();

    unsafe {
        assert_eq!(
            luminate_client_refresh_state(client, device.as_ptr()),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_save_current(client, &raw const selector.target),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_clear_target_selector(client, &raw const selector, &raw mut outcome),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_save_current_selector(client, &raw const selector, &raw mut outcome),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_restore_appearance_selector(
                client,
                &raw const selector,
                &raw mut outcome
            ),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_set_brightness_selector(
                client,
                &raw const selector,
                42,
                true,
                0,
                &raw mut outcome
            ),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_set_emission_selector(
                client,
                &raw const selector,
                EmissionState::Dark as u32,
                &raw mut outcome
            ),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_set_brightness_selector(
                client,
                &raw const selector,
                42,
                true,
                99,
                &raw mut outcome,
            ),
            LuminateStatus::InvalidArgument
        );
        assert_eq!(
            luminate_client_set_brightness_selector(
                client,
                &raw const selector,
                42,
                false,
                0,
                ptr::null_mut(),
            ),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_set_emission_selector(
                client,
                &raw const selector,
                u32::MAX,
                &raw mut outcome,
            ),
            LuminateStatus::InvalidArgument
        );
    }
}

#[test]
fn collection_outcome_accessors_preserve_applied_and_denied_targets() {
    let value = LuminateCollectionOutcome(CollectionOutcome {
        applied: vec![TargetId::device("desk")],
        denied: vec![TargetId::device("hall")],
    });
    assert_eq!(
        unsafe { luminate_collection_outcome_applied_count(&value) },
        1
    );
    assert!(!unsafe { luminate_collection_outcome_applied_at(&value, 0) }.is_null());
    assert!(unsafe { luminate_collection_outcome_applied_at(&value, 1) }.is_null());
    assert_eq!(
        unsafe { luminate_collection_outcome_denied_count(&value) },
        1
    );
    assert!(!unsafe { luminate_collection_outcome_denied_at(&value, 0) }.is_null());
    assert_eq!(
        unsafe { luminate_collection_outcome_denied_count(ptr::null()) },
        0
    );
}
