// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! C API for the client → daemon shared-memory frame-streaming fast path.
//!
//! Unlike the ordinary [`super::frame`] functions, this leg needs a
//! persistent client-side handle: once negotiated, every uploaded frame is
//! published directly into shared memory rather than sent as a request, so
//! something has to hold the open iceoryx2 publisher/notifier between calls.
//! [`LuminateShmFrameStream`] is that handle, an owned root, following the
//! same single-field-newtype convention as `LuminateEvent` and friends (see
//! `ffi_typed`'s module doc), not a worker-thread handle like
//! [`crate::ffi::FfiClient`]: every iceoryx2 operation here is synchronous
//! and needs no Tokio runtime, so there is nothing for a dedicated thread to
//! do. Negotiation and teardown (`BeginShmFrameStream`/`EndShmFrameStream`)
//! do need the daemon connection, so those two calls are dispatched onto the
//! *originating* [`LuminateClient`]'s execution domain via [`call_client`],
//! exactly like [`super::frame`]'s calls. The stream retains that domain, so
//! it remains usable after the public client handle is freed.

#![allow(
    unsafe_code,
    reason = "none of this module uses unsafe directly, but it links iceoryx2, which the workspace lint config treats as unsafe-adjacent FFI"
)]

use super::effects::read_target;
use super::*;

use iceoryx2::port::notifier::Notifier;
use iceoryx2::port::publisher::Publisher;
use iceoryx2::prelude::*;

use luminate_core::shm_frame::{SHM_CLIENT_FRAME_HEADER_LEN, ShmClientFrameHeader, ShmPixelFormat};

use crate::ShmStreamReady;
use crate::ffi::FfiClient;

/// The daemon's response to one published sample, mirroring
/// [`super::frame::LuminateFrameAck`]. There is no `dropped` field: the fast
/// path is fire-and-forget, so this only ever confirms the local publish
/// succeeded, never that the daemon (let alone the plugin) has applied it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LuminateShmFrameAck {
    /// The sequence number this client assigned and just published.
    pub sequence: u64,
}

pub(crate) struct FfiShmStream {
    /// Retains the connection and execution domain this stream was negotiated
    /// on so teardown remains safe after the public client handle is freed.
    client: FfiClient,

    target: TargetId,
    generation: u32,
    pixel_format: ShmPixelFormat,
    pixel_count: usize,
    stream_nonce: u64,

    sequence: u64,

    publisher: Publisher<ipc_threadsafe::Service, [u8], ()>,
    notifier: Notifier<ipc_threadsafe::Service>,
}

/// Owned client-published shared-memory frame stream, returned by
/// `luminate_client_begin_shm_frame_stream`; ended (and freed) with
/// `luminate_client_end_shm_frame_stream`.
pub struct LuminateShmFrameStream(pub(crate) FfiShmStream);

type PublisherPorts = (
    Publisher<ipc_threadsafe::Service, [u8], ()>,
    Notifier<ipc_threadsafe::Service>,
);

/// Attaches to the two iceoryx2 services the daemon already created for
/// `ready`, and creates this side's publisher/notifier ports on them.
/// Deliberately opens rather than creates-or-opens: unlike the daemon, this
/// side is never the resource owner, so a missing service is a genuine
/// setup failure, not something to paper over.
fn open_publisher(ready: &ShmStreamReady) -> Result<PublisherPorts, String> {
    let node = luminate_host_supervisor::create_node()
        .map_err(|error| format!("creating iceoryx2 node: {error}"))?;

    let pubsub_name = ServiceName::new(&ready.service_name)
        .map_err(|error| format!("invalid publish-subscribe service name: {error}"))?;
    let pubsub = node
        .service_builder(&pubsub_name)
        .publish_subscribe::<[u8]>()
        .open()
        .map_err(|error| format!("opening shared-memory segment: {error}"))?;
    let publisher = pubsub
        .publisher_builder()
        .initial_max_slice_len(usize::try_from(ready.segment_bytes).unwrap_or(usize::MAX))
        .create()
        .map_err(|error| format!("creating shared-memory publisher: {error}"))?;

    let event_name = ServiceName::new(&ready.event_service_name)
        .map_err(|error| format!("invalid event service name: {error}"))?;
    let event = node
        .service_builder(&event_name)
        .event()
        .open()
        .map_err(|error| format!("opening shared-memory event channel: {error}"))?;
    let notifier = event
        .notifier_builder()
        .create()
        .map_err(|error| format!("creating shared-memory notifier: {error}"))?;

    Ok((publisher, notifier))
}

impl FfiShmStream {
    pub(crate) fn client(&self) -> &FfiClient {
        &self.client
    }

    pub(crate) fn into_teardown(self) -> (FfiClient, TargetId, u32) {
        (self.client, self.target, self.generation)
    }
}

pub(crate) fn create_ffi_shm_stream(
    client: FfiClient,
    target: TargetId,
    ready: &ShmStreamReady,
) -> Result<LuminateShmFrameStream, Error> {
    let segment_bytes = usize::try_from(ready.segment_bytes).unwrap_or(usize::MAX);
    let per_pixel = ready.pixel_format.bytes_per_pixel();
    let pixel_count = segment_bytes
        .checked_sub(SHM_CLIENT_FRAME_HEADER_LEN)
        .filter(|payload_bytes| payload_bytes % per_pixel == 0)
        .map(|payload_bytes| payload_bytes / per_pixel)
        .ok_or_else(|| {
            Error::Protocol(
                "negotiated shared-memory segment does not contain a whole packed frame".to_owned(),
            )
        })?;
    let (publisher, notifier) = open_publisher(ready).map_err(Error::Io)?;

    Ok(LuminateShmFrameStream(FfiShmStream {
        client,
        target,
        generation: ready.generation,
        pixel_format: ready.pixel_format,
        pixel_count,
        stream_nonce: ready.stream_nonce,
        sequence: 0,
        publisher,
        notifier,
    }))
}

/// Negotiates and begins a client-published shared-memory frame stream on
/// the given target, writing the ready handle to `out_stream`.
/// [`crate::LuminateStatusUnsupported`]-equivalent
/// (`LUMINATE_STATUS_UNSUPPORTED`) means the fast path isn't offered to
/// this connection or target; the caller's fallback is
/// `luminate_client_begin_frame_stream`. The returned stream retains its
/// originating connection and may outlive the public client handle.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_begin_shm_frame_stream(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    out_stream: *mut *mut LuminateShmFrameStream,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let client_ref_value = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let target = match unsafe { read_target(target) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let out_stream = match unsafe { out_ptr_mut(out_stream, "shm frame stream") } {
            Ok(v) => v,
            Err(e) => return e,
        };

        let negotiated_target = target.clone();
        let ready = match call_client(client_ref_value, move |c| async move {
            c.begin_shm_frame_stream(negotiated_target).await
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let ready = match ready {
            Ok(ready) => ready,
            Err(e) => return store_error(&e),
        };
        let segment_bytes = usize::try_from(ready.segment_bytes).unwrap_or(usize::MAX);
        let per_pixel = ready.pixel_format.bytes_per_pixel();
        let pixel_count = segment_bytes
            .checked_sub(SHM_CLIENT_FRAME_HEADER_LEN)
            .filter(|payload_bytes| payload_bytes % per_pixel == 0)
            .map(|payload_bytes| payload_bytes / per_pixel);
        let Some(pixel_count) = pixel_count else {
            let cleanup_target = target.clone();
            let generation = ready.generation;
            let _ = call_client(client_ref_value, move |c| async move {
                c.end_shm_frame_stream(cleanup_target, generation).await
            });
            crate::ffi::set_last_error(
                "negotiated shared-memory segment does not contain a whole packed frame",
            );
            return LuminateStatus::Protocol;
        };

        let (publisher, notifier) = match open_publisher(&ready) {
            Ok(ports) => ports,
            Err(message) => {
                // The daemon already accepted and stood up its half; tell it
                // to tear that down rather than leaving an orphaned
                // subscriber waiting for a publisher that will never appear.
                let cleanup_target = target.clone();
                let generation = ready.generation;
                let _ = call_client(client_ref_value, move |c| async move {
                    c.end_shm_frame_stream(cleanup_target, generation).await
                });
                crate::ffi::set_last_error(message);
                return LuminateStatus::Io;
            }
        };

        let stream = FfiShmStream {
            client: client_ref_value.clone(),
            target,
            generation: ready.generation,
            pixel_format: ready.pixel_format,
            pixel_count,
            stream_nonce: ready.stream_nonce,
            sequence: 0,
            publisher,
            notifier,
        };
        *out_stream = Box::into_raw(Box::new(LuminateShmFrameStream(stream)));
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Publishes one full frame of colours to an active client-published
/// shared-memory stream. `colours` must have exactly the pixel count the
/// stream's target advertised; a mismatched count is reported as
/// [`crate::LuminateStatusInvalidArgument`]-equivalent
/// (`LUMINATE_STATUS_INVALID_ARGUMENT`), not silently truncated or padded.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_shm_upload_frame_full(
    stream: *mut LuminateShmFrameStream,
    colours: *const LuminateRgb,
    count: usize,
    commit: u8,
    out_ack: *mut LuminateShmFrameAck,
) -> LuminateStatus {
    ffi_guard(|| {
        if colours.is_null() && count != 0 {
            crate::ffi::set_last_error("frame colours pointer is null");
            return LuminateStatus::NullPointer;
        }
        let Some(stream) = native_mut!(stream, FfiShmStream) else {
            crate::ffi::set_last_error("shm frame stream handle is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let out_ack = match unsafe { out_ptr_mut(out_ack, "shm frame ack") } {
            Ok(v) => v,
            Err(e) => return e,
        };
        if count != stream.pixel_count {
            crate::ffi::set_last_error(format!(
                "frame has {count} pixels, expected {}",
                stream.pixel_count
            ));
            return LuminateStatus::InvalidArgument;
        }

        let per_pixel = stream.pixel_format.bytes_per_pixel();
        let Ok(pixel_count) = u32::try_from(count) else {
            crate::ffi::set_last_error("frame pixel count overflowed u32");
            return LuminateStatus::InvalidArgument;
        };
        let Some(total_len) =
            SHM_CLIENT_FRAME_HEADER_LEN.checked_add(match count.checked_mul(per_pixel) {
                Some(v) => v,
                None => {
                    crate::ffi::set_last_error("frame payload size overflowed");
                    return LuminateStatus::InvalidArgument;
                }
            })
        else {
            crate::ffi::set_last_error("frame payload size overflowed");
            return LuminateStatus::InvalidArgument;
        };

        let sequence = stream.sequence;
        let header = ShmClientFrameHeader {
            sequence,
            generation: stream.generation,
            pixel_count,
            pixel_format: stream.pixel_format.to_abi(),
            header_version: luminate_core::shm_frame::SHM_CLIENT_FRAME_HEADER_VERSION,
            flags: 0,
            reserved: 0,
            stream_nonce: stream.stream_nonce,
        }
        .with_commit(commit != 0);

        let mut sample = match stream.publisher.loan_slice(total_len) {
            Ok(sample) => sample,
            Err(error) => {
                crate::ffi::set_last_error(format!("loaning a shared-memory sample: {error}"));
                return LuminateStatus::Io;
            }
        };
        let payload = sample.payload_mut();
        let Some((header_bytes, pixel_bytes)) =
            payload.split_at_mut_checked(SHM_CLIENT_FRAME_HEADER_LEN)
        else {
            crate::ffi::set_last_error(
                "loaned shared-memory sample was shorter than the frame header",
            );
            return LuminateStatus::Internal;
        };
        header_bytes.copy_from_slice(&header.to_bytes());

        let colours = if count == 0 {
            &[][..]
        } else {
            // SAFETY: `colours` was null-checked above whenever `count != 0`.
            unsafe { std::slice::from_raw_parts(colours, count) }
        };
        for (chunk, colour) in pixel_bytes.chunks_mut(per_pixel).zip(colours) {
            let colour = Colour::rgb(Rgb::new(colour.r, colour.g, colour.b));
            if let Err(error) = stream.pixel_format.pack(&colour, chunk) {
                crate::ffi::set_last_error(format!(
                    "packing a pixel into the shared-memory buffer: {error}"
                ));
                return LuminateStatus::InvalidArgument;
            }
        }

        if let Err(error) = sample.send() {
            crate::ffi::set_last_error(format!("sending a shared-memory sample: {error}"));
            return LuminateStatus::Io;
        }
        if let Err(error) = stream.notifier.notify() {
            crate::ffi::set_last_error(format!("notifying the shared-memory subscriber: {error}"));
            return LuminateStatus::Io;
        }
        stream.sequence = stream.sequence.wrapping_add(1);

        *out_ack = LuminateShmFrameAck { sequence };
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Ends the client-published shared-memory stream, telling the daemon over
/// its original connection, then frees the handle. Idempotent-safe to call
/// with null (a no-op), but not safe to call twice on the same non-null
/// pointer: like every other owned root in this crate, the handle is
/// consumed and invalidated by this call.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_end_shm_frame_stream(
    stream: *mut LuminateShmFrameStream,
) -> LuminateStatus {
    ffi_guard(|| {
        if stream.is_null() {
            clear_last_error();
            return LuminateStatus::Ok;
        }
        // SAFETY: upheld by the enclosing function's documented C pointer
        // contract: a non-null pointer references a live, uniquely-owned
        // `LuminateShmFrameStream` this call consumes.
        let stream = unsafe { Box::from_raw(stream) }.0;
        let target = stream.target;
        let generation = stream.generation;
        let result = match call_client(&stream.client, move |c| async move {
            c.end_shm_frame_stream(target, generation).await
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
#[path = "shm_tests.rs"]
mod tests;
