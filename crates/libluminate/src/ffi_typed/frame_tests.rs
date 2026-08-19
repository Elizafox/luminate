// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Frame projections and portable fake-daemon round trips.

use super::*;
use crate::ffi::{FfiClient, luminate_client_connect_path, luminate_client_free};
use crate::ffi_typed::test_support::{FakeDaemon, respond};
use luminate_protocol::{Request, ResponseStatus};
use std::ffi::CString;

/// A full frame-stream lifecycle (begin, full upload, partial upload, end)
/// against a live mock daemon, exercising the success paths that a null or
/// stopped client can never reach.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "The test exercises one continuous frame-stream lifecycle across the C boundary."
)]
async fn frame_stream_lifecycle_round_trips_through_a_live_daemon() {
    let daemon = FakeDaemon::bind("frame-ffi-lifecycle");
    let path_c = daemon.c_path();
    let server = tokio::spawn(async move {
        let mut daemon = daemon;
        let mut stream = daemon.accept("frame-test-daemon").await;

        let request = respond(
            &mut stream,
            ResponseStatus::FrameStreamStarted { generation: 7 },
        )
        .await;
        assert!(matches!(request, Request::BeginFrameStream { .. }));

        let request = respond(
            &mut stream,
            ResponseStatus::FrameAck {
                sequence: 1,
                dropped: false,
            },
        )
        .await;
        assert!(matches!(request, Request::UploadFrame { .. }));

        let request = respond(
            &mut stream,
            ResponseStatus::FrameAck {
                sequence: 2,
                dropped: true,
            },
        )
        .await;
        assert!(matches!(request, Request::UploadFrame { .. }));

        let request = respond(&mut stream, ResponseStatus::Ack).await;
        assert!(matches!(request, Request::EndFrameStream { .. }));
    });

    let device = CString::new("keyboard").expect("valid device id");
    let target = LuminateTarget {
        device_id: device.as_ptr(),
        surface_id: ptr::null(),
        element_id: ptr::null(),
        group_id: ptr::null(),
    };
    let mut client = ptr::null_mut();
    let mut generation = 0;
    let colour = LuminateRgb { r: 9, g: 8, b: 7 };
    let index = 3;
    let mut ack = LuminateFrameAck {
        sequence: 0,
        dropped: 0,
    };

    // SAFETY: every pointer refers to live local storage or a NUL-terminated
    // string kept alive for the duration of this test, and the client handle
    // is freed exactly once at the end.
    unsafe {
        assert_eq!(
            luminate_client_connect_path(path_c.as_ptr(), &raw mut client),
            LuminateStatus::Ok
        );

        assert_eq!(
            luminate_client_begin_frame_stream(client, &raw const target, &raw mut generation),
            LuminateStatus::Ok
        );
        assert_eq!(generation, 7);

        assert_eq!(
            luminate_client_upload_frame_full(
                client,
                &raw const target,
                generation,
                1,
                &raw const colour,
                1,
                1,
                &raw mut ack,
            ),
            LuminateStatus::Ok
        );
        assert_eq!(ack.sequence, 1);
        assert_eq!(ack.dropped, 0);

        assert_eq!(
            luminate_client_upload_frame_partial(
                client,
                &raw const target,
                generation,
                2,
                &raw const index,
                &raw const colour,
                1,
                0,
                &raw mut ack,
            ),
            LuminateStatus::Ok
        );
        assert_eq!(ack.sequence, 2);
        assert_eq!(ack.dropped, 1);

        assert_eq!(
            luminate_client_end_frame_stream(client, &raw const target, generation),
            LuminateStatus::Ok
        );

        luminate_client_free(client);
    }

    server.await.expect("frame server task");
}

#[test]
fn frame_uploads_validate_pointer_and_count_combinations() {
    let colour = LuminateRgb { r: 1, g: 2, b: 3 };
    let index = 7;
    let mut ack = LuminateFrameAck {
        sequence: 0,
        dropped: 0,
    };

    // SAFETY: every non-null pointer refers to live local storage. A null
    // client deliberately stops valid payloads before any client access.
    unsafe {
        assert_eq!(
            luminate_client_upload_frame_full(
                ptr::null_mut(),
                ptr::null(),
                1,
                2,
                ptr::null(),
                1,
                0,
                &raw mut ack,
            ),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_upload_frame_partial(
                ptr::null_mut(),
                ptr::null(),
                1,
                2,
                ptr::null(),
                &raw const colour,
                1,
                0,
                &raw mut ack,
            ),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_upload_frame_partial(
                ptr::null_mut(),
                ptr::null(),
                1,
                2,
                &raw const index,
                ptr::null(),
                1,
                0,
                &raw mut ack,
            ),
            LuminateStatus::NullPointer
        );

        assert_eq!(
            luminate_client_upload_frame_full(
                ptr::null_mut(),
                ptr::null(),
                3,
                4,
                ptr::null(),
                0,
                1,
                &raw mut ack,
            ),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_upload_frame_partial(
                ptr::null_mut(),
                ptr::null(),
                3,
                4,
                ptr::null(),
                ptr::null(),
                0,
                1,
                &raw mut ack,
            ),
            LuminateStatus::NullPointer
        );
    }
}

#[test]
fn frame_stream_operations_reject_null_clients() {
    let mut generation = 0;

    // SAFETY: null is a supported error input, and the output pointer
    // refers to live local storage.
    unsafe {
        assert_eq!(
            luminate_client_begin_frame_stream(ptr::null_mut(), ptr::null(), &raw mut generation,),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_end_frame_stream(ptr::null_mut(), ptr::null(), 9),
            LuminateStatus::NullPointer
        );
    }
}

#[test]
fn frame_stream_operations_validate_inputs_before_dispatch() {
    let device = CString::new("keyboard").expect("literal has no NUL");
    let target = LuminateTarget {
        device_id: device.as_ptr(),
        surface_id: ptr::null(),
        element_id: ptr::null(),
        group_id: ptr::null(),
    };
    let mut client = FfiClient::stopped();
    let client = ptr::from_mut(&mut client).cast::<LuminateClient>();
    let colour = LuminateRgb { r: 4, g: 5, b: 6 };
    let index = 7;
    let mut generation = 0;
    let mut ack = LuminateFrameAck {
        sequence: 0,
        dropped: 0,
    };

    // SAFETY: the opaque client points to a live FFI client fixture, the
    // target strings are NUL-terminated, and outputs are writable locals.
    unsafe {
        assert_eq!(
            luminate_client_begin_frame_stream(client, ptr::null(), &raw mut generation),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_begin_frame_stream(client, &raw const target, ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_begin_frame_stream(client, &raw const target, &raw mut generation),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_upload_frame_full(
                client,
                &raw const target,
                1,
                2,
                &raw const colour,
                1,
                0,
                ptr::null_mut(),
            ),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_upload_frame_full(
                client,
                &raw const target,
                1,
                2,
                &raw const colour,
                1,
                0,
                &raw mut ack,
            ),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_upload_frame_partial(
                client,
                &raw const target,
                1,
                2,
                &raw const index,
                &raw const colour,
                1,
                0,
                &raw mut ack,
            ),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_end_frame_stream(client, &raw const target, 1),
            LuminateStatus::Internal
        );
    }
}
