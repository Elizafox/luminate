// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Native-ABI adapters for the opt-in, zero-copy shared-memory frame fast
//! path.
//!
//! Only [`dispatch_shm_stream_begin`] decodes CBOR, once per stream. Frame
//! delivery uses validated shared-memory inputs and avoids per-frame decoding.
//! Stream teardown carries no pointers that need a shared validation adapter.

use std::slice;

use luminate_core::shm_frame::{ShmFrameHeader, ShmPixelFormat};

use crate::apply::{PluginApplyResult, write_apply_result};
use crate::topology::PluginTarget;

/// Decodes a CBOR-encoded target and validates the requested pixel format,
/// then hands both to `begin`, which returns either an opaque stream handle
/// or a failure result.
///
/// `*handle_out` is always written: `0` on any failure, including a
/// malformed envelope this function rejects before ever calling `begin`;
/// otherwise the handle `begin` returned. A handle of `0` is never valid:
/// callers may treat it as "no stream" the same way a null pointer would be.
///
/// # Safety
///
/// Any pointer may be null. Otherwise: `target_cbor` must point to
/// `target_len` bytes valid for this call; `handle_out` must point to one
/// writable `u64`; `result` must point to one writable, properly aligned
/// [`PluginApplyResult`].
#[allow(
    clippy::too_many_arguments,
    reason = "mirrors the raw ABI signature it adapts (PluginShmStreamBeginFn), which the plugin ABI's compatibility contract fixes"
)]
pub unsafe fn dispatch_shm_stream_begin(
    target_cbor: *const u8,
    target_len: usize,
    pixel_format: u32,
    pixel_count: u32,
    generation: u32,
    handle_out: *mut u64,
    result: *mut PluginApplyResult,
    begin: impl FnOnce(&PluginTarget, ShmPixelFormat, u32, u32) -> Result<u64, Box<PluginApplyResult>>,
) -> u8 {
    if result.is_null() || handle_out.is_null() {
        tracing::warn!("received null shm-stream-begin result or handle slot");
        return 0;
    }

    // SAFETY: checked non-null above; the caller's contract guarantees it's
    // writable for the duration of this call.
    unsafe { handle_out.write(0) };

    if target_cbor.is_null() {
        tracing::warn!("received null shm-stream target");
        // SAFETY: `result` was checked non-null above.
        return unsafe {
            write_apply_result(
                result,
                PluginApplyResult::invalid_argument("received null shm-stream target"),
            )
        };
    }

    // SAFETY: upheld by the caller; null was checked above.
    let payload = unsafe { slice::from_raw_parts(target_cbor, target_len) };
    let target: PluginTarget = match ciborium::from_reader(payload) {
        Ok(target) => target,
        Err(error) => {
            tracing::warn!(error = %error, "failed to decode CBOR shm-stream target");
            // SAFETY: `result` was checked non-null above.
            return unsafe {
                write_apply_result(
                    result,
                    PluginApplyResult::invalid_argument(error.to_string()),
                )
            };
        }
    };

    let Some(format) = ShmPixelFormat::from_abi(pixel_format) else {
        tracing::warn!(pixel_format, "unrecognized shm pixel format");
        // SAFETY: `result` was checked non-null above.
        return unsafe {
            write_apply_result(
                result,
                PluginApplyResult::invalid_argument("unrecognized shm pixel format"),
            )
        };
    };

    let outcome = match begin(&target, format, pixel_count, generation) {
        Ok(handle) => {
            // SAFETY: checked non-null above.
            unsafe { handle_out.write(handle) };
            PluginApplyResult::applied()
        }
        Err(failure) => *failure,
    };

    // SAFETY: `result` was checked non-null above.
    unsafe { write_apply_result(result, outcome) }
}

/// Builds a pixel slice from `pixels`/`pixels_len` and hands it, along with
/// `header`, to `apply`. There is no CBOR to decode on this path.
///
/// # Safety
///
/// `pixels` may be null only if `pixels_len` is `0`. Otherwise it must point
/// to `pixels_len` bytes valid for this call. `result` must point to one
/// writable, properly aligned [`PluginApplyResult`]; it may be null, in
/// which case this returns `0` without calling `apply`.
pub unsafe fn dispatch_shm_frame(
    header: ShmFrameHeader,
    pixels: *const u8,
    pixels_len: usize,
    result: *mut PluginApplyResult,
    apply: impl FnOnce(&ShmFrameHeader, &[u8]) -> PluginApplyResult,
) -> u8 {
    if result.is_null() {
        tracing::warn!("received null shm-frame result buffer");
        return 0;
    }

    if pixels.is_null() && pixels_len != 0 {
        tracing::warn!("received null shm-frame pixel buffer with a nonzero length");
        // SAFETY: `result` was checked non-null above.
        return unsafe {
            write_apply_result(
                result,
                PluginApplyResult::invalid_argument("received null shm-frame pixel buffer"),
            )
        };
    }

    // SAFETY: upheld by the caller; the null-with-nonzero-length case was
    // rejected above, so a null pointer here only ever pairs with a `0`
    // length, which `from_raw_parts` would otherwise still reject on its
    // own (it requires non-null even for empty slices).
    let pixels = unsafe {
        if pixels.is_null() {
            &[]
        } else {
            slice::from_raw_parts(pixels, pixels_len)
        }
    };

    let outcome = apply(&header, pixels);

    // SAFETY: `result` was checked non-null above.
    unsafe { write_apply_result(result, outcome) }
}

#[cfg(test)]
#[path = "shm_tests.rs"]
mod tests;
