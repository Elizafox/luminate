// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use std::ptr;

use super::{PluginApplyResult, dispatch_shm_frame, dispatch_shm_stream_begin};
use crate::apply::PluginApplyStatus;
use crate::codec::encode_cbor;
use crate::topology::PluginTarget;
use luminate_core::shm_frame::{SHM_FRAME_HEADER_VERSION, ShmFrameHeader, ShmPixelFormat};

fn sample_target() -> PluginTarget {
    PluginTarget::Device {
        device: "matrix-1".to_owned(),
    }
}

fn sample_header() -> ShmFrameHeader {
    ShmFrameHeader {
        sequence: 1,
        generation: 1,
        pixel_count: 3,
        pixel_format: ShmPixelFormat::Rgb8.to_abi(),
        header_version: SHM_FRAME_HEADER_VERSION,
        flags: 0,
        reserved: 0,
    }
}

fn encode_target(target: &PluginTarget) -> Vec<u8> {
    encode_cbor(target).expect("encode target")
}

#[test]
fn begin_decodes_target_and_format_then_forwards_to_the_closure() {
    let encoded = encode_target(&sample_target());
    let mut handle = 0xAAAA_AAAA_u64;
    let mut result = PluginApplyResult::applied();

    // SAFETY: `encoded` is valid for its own length, and
    // `handle`/`result` are valid, writable local variables for this call.
    let status = unsafe {
        dispatch_shm_stream_begin(
            encoded.as_ptr(),
            encoded.len(),
            ShmPixelFormat::Rgb8.to_abi(),
            16,
            7,
            &raw mut handle,
            &raw mut result,
            |target, format, pixel_count, generation| {
                assert!(matches!(target, PluginTarget::Device { device } if device == "matrix-1"));
                assert_eq!(format, ShmPixelFormat::Rgb8);
                assert_eq!(pixel_count, 16);
                assert_eq!(generation, 7);
                Ok(42)
            },
        )
    };

    assert_eq!(status, 1);
    assert_eq!(handle, 42);
    assert_eq!(
        result.decode().expect("decode").0,
        PluginApplyStatus::Applied
    );
}

#[test]
fn begin_writes_zero_handle_and_propagates_a_closure_failure() {
    let encoded = encode_target(&sample_target());
    let mut handle = 0xAAAA_AAAA_u64;
    let mut result = PluginApplyResult::applied();

    // SAFETY: `encoded` is valid for its own length, and
    // `handle`/`result` are valid, writable local variables for this call.
    let status = unsafe {
        dispatch_shm_stream_begin(
            encoded.as_ptr(),
            encoded.len(),
            ShmPixelFormat::Rgb8.to_abi(),
            16,
            7,
            &raw mut handle,
            &raw mut result,
            |_target, _format, _pixel_count, _generation| {
                Err(Box::new(PluginApplyResult::unavailable("hardware offline")))
            },
        )
    };

    assert_eq!(status, 1);
    assert_eq!(handle, 0);
    assert_eq!(
        result.decode().expect("decode").0,
        PluginApplyStatus::Unavailable
    );
}

#[test]
fn begin_rejects_malformed_cbor_without_calling_the_closure() {
    let garbage = [0xff_u8; 4];
    let mut handle = 0xAAAA_AAAA_u64;
    let mut result = PluginApplyResult::applied();

    // SAFETY: `encoded` is valid for its own length, and
    // `handle`/`result` are valid, writable local variables for this call.
    let status = unsafe {
        dispatch_shm_stream_begin(
            garbage.as_ptr(),
            garbage.len(),
            ShmPixelFormat::Rgb8.to_abi(),
            16,
            7,
            &raw mut handle,
            &raw mut result,
            |_, _, _, _| panic!("closure must not run for malformed CBOR"),
        )
    };

    assert_eq!(status, 1);
    assert_eq!(handle, 0);
    assert_eq!(
        result.decode().expect("decode").0,
        PluginApplyStatus::InvalidArgument
    );
}

#[test]
fn begin_rejects_an_unrecognized_pixel_format_without_calling_the_closure() {
    let encoded = encode_target(&sample_target());
    let mut handle = 0xAAAA_AAAA_u64;
    let mut result = PluginApplyResult::applied();

    // SAFETY: `encoded` is valid for its own length, and
    // `handle`/`result` are valid, writable local variables for this call.
    let status = unsafe {
        dispatch_shm_stream_begin(
            encoded.as_ptr(),
            encoded.len(),
            999,
            16,
            7,
            &raw mut handle,
            &raw mut result,
            |_, _, _, _| panic!("closure must not run for an unknown pixel format"),
        )
    };

    assert_eq!(status, 1);
    assert_eq!(handle, 0);
    assert_eq!(
        result.decode().expect("decode").0,
        PluginApplyStatus::InvalidArgument
    );
}

#[test]
fn begin_returns_zero_and_writes_nothing_when_result_is_null() {
    let encoded = encode_target(&sample_target());
    let mut handle = 0xAAAA_AAAA_u64;

    // SAFETY: `encoded` is valid for its own length, and
    // `handle`/`result` are valid, writable local variables for this call.
    let status = unsafe {
        dispatch_shm_stream_begin(
            encoded.as_ptr(),
            encoded.len(),
            ShmPixelFormat::Rgb8.to_abi(),
            16,
            7,
            &raw mut handle,
            ptr::null_mut(),
            |_, _, _, _| panic!("closure must not run when result is null"),
        )
    };

    assert_eq!(status, 0);
    assert_eq!(
        handle, 0xAAAA_AAAA,
        "handle slot must be untouched, not just zeroed"
    );
}

#[test]
fn begin_returns_zero_when_handle_slot_is_null() {
    let encoded = encode_target(&sample_target());
    let mut result = PluginApplyResult::applied();

    // SAFETY: `encoded`/`garbage` are valid for their own length, and
    // `handle`/`result` are valid, writable local variables for this call.
    let status = unsafe {
        dispatch_shm_stream_begin(
            encoded.as_ptr(),
            encoded.len(),
            ShmPixelFormat::Rgb8.to_abi(),
            16,
            7,
            ptr::null_mut(),
            &raw mut result,
            |_, _, _, _| panic!("closure must not run when the handle slot is null"),
        )
    };

    assert_eq!(status, 0);
}

#[test]
fn frame_forwards_header_and_pixels_to_the_closure() {
    let pixels = [1_u8, 2, 3];
    let mut result = PluginApplyResult::applied();
    let header = sample_header();

    // SAFETY: `pixels` is valid for its own length; `result` is a valid,
    // writable local variable for this call.
    let status = unsafe {
        dispatch_shm_frame(
            header,
            pixels.as_ptr(),
            pixels.len(),
            &raw mut result,
            |seen_header, seen_pixels| {
                assert_eq!(seen_header.sequence, header.sequence);
                assert_eq!(seen_pixels, &pixels);
                PluginApplyResult::applied()
            },
        )
    };

    assert_eq!(status, 1);
    assert_eq!(
        result.decode().expect("decode").0,
        PluginApplyStatus::Applied
    );
}

#[test]
fn frame_accepts_a_null_pointer_paired_with_a_zero_length() {
    let mut result = PluginApplyResult::applied();

    // SAFETY: a null pointer paired with a `0` length is explicitly
    // permitted by `dispatch_shm_frame`'s own safety contract; `result`
    // is a valid, writable local variable for this call.
    let status = unsafe {
        dispatch_shm_frame(
            sample_header(),
            ptr::null(),
            0,
            &raw mut result,
            |_header, pixels| {
                assert!(pixels.is_empty());
                PluginApplyResult::applied()
            },
        )
    };

    assert_eq!(status, 1);
}

#[test]
fn frame_rejects_a_null_pointer_with_a_nonzero_length_without_calling_the_closure() {
    let mut result = PluginApplyResult::applied();

    // SAFETY: `result` is a valid, writable local variable for this
    // call; the null pixel pointer with a nonzero length is exactly the
    // rejected case this test exercises.
    let status = unsafe {
        dispatch_shm_frame(sample_header(), ptr::null(), 4, &raw mut result, |_, _| {
            panic!("closure must not run for a null buffer with a nonzero length")
        })
    };

    assert_eq!(status, 1);
    assert_eq!(
        result.decode().expect("decode").0,
        PluginApplyStatus::InvalidArgument
    );
}

#[test]
fn frame_returns_zero_when_result_is_null() {
    let pixels = [1_u8, 2, 3];

    // SAFETY: `pixels` is valid for its own length; a null `result` is
    // explicitly permitted by `dispatch_shm_frame`'s own safety contract.
    let status = unsafe {
        dispatch_shm_frame(
            sample_header(),
            pixels.as_ptr(),
            pixels.len(),
            ptr::null_mut(),
            |_, _| panic!("closure must not run when result is null"),
        )
    };

    assert_eq!(status, 0);
}
