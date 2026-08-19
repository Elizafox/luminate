// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use std::ptr;
use std::time::Duration;

use super::{
    PLUGIN_APPLY_DIAGNOSTIC_CAPACITY, PLUGIN_APPLY_DIAGNOSTIC_TRUNCATED, PluginApplyResult,
    PluginApplyStatus, PluginError, dispatch_update_batch_cbor, dispatch_update_cbor,
};
use crate::topology::{PluginTarget, PluginUpdate, PluginUpdateBatch, PluginUpdateOperation};

#[test]
fn typed_plugin_errors_preserve_abi_categories() {
    let cases = [
        (
            PluginError::InvalidTarget("target".to_owned()),
            PluginApplyStatus::InvalidArgument,
        ),
        (
            PluginError::InvalidArgument("argument".to_owned()),
            PluginApplyStatus::InvalidArgument,
        ),
        (
            PluginError::Unsupported("unsupported".to_owned()),
            PluginApplyStatus::Unsupported,
        ),
        (
            PluginError::Unavailable("offline".to_owned()),
            PluginApplyStatus::Unavailable,
        ),
        (
            PluginError::Io("transport".to_owned()),
            PluginApplyStatus::Io,
        ),
        (
            PluginError::Internal("invariant".to_owned()),
            PluginApplyStatus::Internal,
        ),
    ];

    for (error, expected) in cases {
        let result = error.into_apply_result();
        assert_eq!(result.decode().expect("valid typed result").0, expected);
    }
    let rate_limited = PluginError::RateLimited {
        diagnostic: "slow down".to_owned(),
        retry_after: Duration::from_secs(1),
    };
    assert_eq!(
        rate_limited.clone().into_apply_result().retry_after_ms(),
        Some(1_000)
    );
    assert_eq!(
        rate_limited
            .into_apply_result()
            .decode()
            .expect("valid rate-limit result")
            .0,
        PluginApplyStatus::RateLimited
    );
}

#[test]
fn apply_result_round_trips_status_and_diagnostic() {
    let result = PluginApplyResult::io("device timed out");
    assert_eq!(
        result.decode(),
        Ok((PluginApplyStatus::Io, "device timed out"))
    );
}

#[test]
fn apply_result_truncates_at_utf8_boundary() {
    let message = format!("{}é", "x".repeat(PLUGIN_APPLY_DIAGNOSTIC_CAPACITY - 1));
    let result = PluginApplyResult::internal(message);
    let (status, diagnostic) = result.decode().expect("decode truncated result");
    assert_eq!(status, PluginApplyStatus::Internal);
    assert_eq!(diagnostic.len(), PLUGIN_APPLY_DIAGNOSTIC_CAPACITY - 1);
    assert_eq!(result.flags, PLUGIN_APPLY_DIAGNOSTIC_TRUNCATED);
}

#[test]
fn apply_result_rejects_untrusted_invalid_fields() {
    let mut result = PluginApplyResult::applied();
    result.code = u8::MAX;
    assert!(result.decode().is_err());

    result = PluginApplyResult::applied();
    result.diagnostic_len = u16::MAX;
    assert!(result.decode().is_err());

    result = PluginApplyResult::applied();
    result.flags = 0x80;
    assert!(result.decode().is_err());
}

#[test]
fn update_dispatchers_decode_and_preserve_result_order() {
    let updates = vec![
        PluginUpdate {
            target: PluginTarget::Device {
                device: "first".to_owned(),
            },
            operation: PluginUpdateOperation::Clear,
        },
        PluginUpdate {
            target: PluginTarget::Device {
                device: "second".to_owned(),
            },
            operation: PluginUpdateOperation::SaveCurrent,
        },
    ];
    let mut batch = Vec::new();
    ciborium::into_writer(
        &PluginUpdateBatch {
            updates: updates.clone(),
        },
        &mut batch,
    )
    .expect("serialize update batch");
    let mut results = [PluginApplyResult::internal("unset"); 2];

    // SAFETY: `batch` is a live C string and `results` is a writable buffer
    // of the declared length.
    let written = unsafe {
        dispatch_update_batch_cbor(
            batch.as_ptr(),
            batch.len(),
            results.as_mut_ptr(),
            results.len(),
            |update| PluginApplyResult::io(update.target.device_id()),
        )
    };

    assert_eq!(written, 1);
    assert_eq!(results[0].decode(), Ok((PluginApplyStatus::Io, "first")));
    assert_eq!(results[1].decode(), Ok((PluginApplyStatus::Io, "second")));

    let mut update = Vec::new();
    ciborium::into_writer(&updates[0], &mut update).expect("serialize individual update");
    let mut result = PluginApplyResult::internal("unset");
    // SAFETY: `update` is a live C string and `result` is writable.
    let written = unsafe {
        dispatch_update_cbor(update.as_ptr(), update.len(), &raw mut result, |_| {
            PluginApplyResult::applied()
        })
    };
    assert_eq!(written, 1);
    assert_eq!(result.decode(), Ok((PluginApplyStatus::Applied, "")));
}

#[test]
fn update_dispatcher_rejects_bad_input_before_applying() {
    let mut result = PluginApplyResult::applied();
    // SAFETY: the malformed payload is still a valid C string, and
    // `result` is writable.
    let written = unsafe {
        dispatch_update_cbor([0xff].as_ptr(), 1, &raw mut result, |_| {
            panic!("malformed CBOR must not reach the plugin callback")
        })
    };
    assert_eq!(written, 1);
    assert_eq!(
        result.decode().map(|(status, _)| status),
        Ok(PluginApplyStatus::InvalidArgument)
    );

    let mut called = false;
    // SAFETY: null is an explicitly accepted result-pointer value.
    let written = unsafe {
        dispatch_update_cbor([0_u8].as_ptr(), 1, ptr::null_mut(), |_| {
            called = true;
            PluginApplyResult::applied()
        })
    };
    assert_eq!(written, 0);
    assert!(!called);
}
