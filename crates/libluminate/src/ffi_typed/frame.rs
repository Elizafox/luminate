// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

use luminate_core::frame::{FrameEnvelope, FramePayload};

use super::effects::read_target;

/// The daemon's response to one uploaded frame, mirroring [`crate::FrameAck`].
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LuminateFrameAck {
    /// The sequence number the daemon accepted (echoes the uploaded frame's).
    pub sequence: u64,

    /// Nonzero if the frame was accepted but rate-limited, not forwarded to
    /// the plugin.
    pub dropped: u8,
}

/// Begins a frame streaming session on the given target, writing the
/// session generation to `out_generation`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_begin_frame_stream(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    out_generation: *mut u32,
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
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let out_generation = match unsafe { out_ptr_mut(out_generation, "generation") } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let result = match call_client(client, move |c| async move {
            c.begin_frame_stream(target).await
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(generation) => {
                *out_generation = generation;
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(e) => store_error(&e),
        }
    })
}

fn upload_frame(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    envelope: FrameEnvelope,
    out_ack: *mut LuminateFrameAck,
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
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let out_ack = match unsafe { out_ptr_mut(out_ack, "frame ack") } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let result = match call_client(client, move |c| async move {
            c.upload_frame(target, envelope).await
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(ack) => {
                *out_ack = LuminateFrameAck {
                    sequence: ack.sequence,
                    dropped: u8::from(ack.dropped),
                };
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(e) => store_error(&e),
        }
    })
}

/// Uploads a full frame of colours to the given target's active streaming
/// session.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_upload_frame_full(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    generation: u32,
    sequence: u64,
    colours: *const LuminateRgb,
    count: usize,
    commit: u8,
    out_ack: *mut LuminateFrameAck,
) -> LuminateStatus {
    ffi_guard(|| {
        if colours.is_null() && count != 0 {
            crate::ffi::set_last_error("frame colours pointer is null");
            return LuminateStatus::NullPointer;
        }
        let pixels = if count == 0 {
            Vec::new()
        } else {
            // SAFETY: upheld by the enclosing function's documented C pointer contract.
            unsafe { std::slice::from_raw_parts(colours, count) }
                .iter()
                .map(|v| Colour::rgb(Rgb::new(v.r, v.g, v.b)))
                .collect()
        };
        let envelope = FrameEnvelope {
            generation,
            sequence,
            payload: FramePayload::Full(pixels),
            commit: commit != 0,
        };
        upload_frame(client, target, envelope, out_ack)
    })
}

/// Uploads a partial frame (sparse indices and colours) to the given
/// target's active streaming session.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_upload_frame_partial(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    generation: u32,
    sequence: u64,
    indices: *const u32,
    colours: *const LuminateRgb,
    count: usize,
    commit: u8,
    out_ack: *mut LuminateFrameAck,
) -> LuminateStatus {
    ffi_guard(|| {
        if count != 0 && (indices.is_null() || colours.is_null()) {
            crate::ffi::set_last_error("frame indices or colours pointer is null");
            return LuminateStatus::NullPointer;
        }
        let pixels = if count == 0 {
            Vec::new()
        } else {
            // SAFETY: upheld by the enclosing function's documented C pointer contract.
            let indices = unsafe { std::slice::from_raw_parts(indices, count) };
            // SAFETY: upheld by the enclosing function's documented C pointer contract.
            let colours = unsafe { std::slice::from_raw_parts(colours, count) };
            indices
                .iter()
                .zip(colours.iter())
                .map(|(&index, v)| (index, Colour::rgb(Rgb::new(v.r, v.g, v.b))))
                .collect()
        };
        let envelope = FrameEnvelope {
            generation,
            sequence,
            payload: FramePayload::Partial(pixels),
            commit: commit != 0,
        };
        upload_frame(client, target, envelope, out_ack)
    })
}

/// Ends the frame streaming session with the given generation on the
/// target.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_end_frame_stream(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    generation: u32,
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
            c.end_frame_stream(target, generation).await
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

#[cfg(test)]
#[path = "frame_tests.rs"]
mod tests;
