// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Shared-memory projections and portable fake-daemon round trips.

use super::*;
use std::ffi::CString;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::ffi::{luminate_client_connect_path, luminate_client_free};
use crate::ffi_typed::test_support::{FakeDaemon, respond};
use luminate_protocol::{ErrorCode, OperationError, Request, ResponseStatus};

fn create_shm_services(service_name: &str, event_service_name: &str) -> (impl Sized, impl Sized) {
    let node = luminate_host_supervisor::create_node().expect("create iceoryx2 node");
    let pubsub_name = ServiceName::new(service_name).expect("valid service name");
    let pubsub = node
        .service_builder(&pubsub_name)
        .publish_subscribe::<[u8]>()
        .max_publishers(1)
        .max_subscribers(1)
        .subscriber_max_buffer_size(1)
        .history_size(0)
        .enable_safe_overflow(true)
        .open_or_create()
        .expect("create publish-subscribe service");
    let subscriber = pubsub
        .subscriber_builder()
        .create()
        .expect("create subscriber");

    let event_name = ServiceName::new(event_service_name).expect("valid event name");
    let event = node
        .service_builder(&event_name)
        .event()
        .max_notifiers(1)
        .max_listeners(1)
        .open_or_create()
        .expect("create event service");
    let listener = event.listener_builder().create().expect("create listener");
    (subscriber, listener)
}

/// The successful same-uid `BeginShmFrameStream` negotiation has a dedicated
/// cross-process integration test against a real daemon. This test exercises
/// the C entry point against a mock daemon that declines the fast path. Every
/// caller must handle the resulting `Unsupported` status by falling back to
/// `luminate_client_begin_frame_stream`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "The test exercises one continuous begin/fallback round trip across the C boundary."
)]
async fn begin_shm_frame_stream_reports_unsupported_from_a_live_daemon() {
    let daemon = FakeDaemon::bind("shm-ffi-unsupported");
    let path_c = daemon.c_path();
    let server = tokio::spawn(async move {
        let mut daemon = daemon;
        let mut stream = daemon.accept("shm-test-daemon").await;

        let request = respond(
            &mut stream,
            ResponseStatus::Error(OperationError {
                code: ErrorCode::Unsupported,
                message: "shared-memory streaming not offered".to_owned(),
                retry_after_ms: None,
                applied_targets: Vec::new(),
            }),
        )
        .await;
        assert!(matches!(request, Request::BeginShmFrameStream { .. }));
    });

    let device = CString::new("keyboard").expect("valid device id");
    let target = LuminateTarget {
        device_id: device.as_ptr(),
        surface_id: ptr::null(),
        element_id: ptr::null(),
        group_id: ptr::null(),
    };
    let mut client = ptr::null_mut();
    let mut stream_handle = ptr::null_mut();

    // SAFETY: every pointer refers to live local storage or a NUL-terminated
    // string kept alive for the duration of this test, and the client
    // handle is freed exactly once at the end. `stream_handle` stays null
    // on the `Unsupported` path, so there is nothing else to free.
    unsafe {
        assert_eq!(
            luminate_client_connect_path(path_c.as_ptr(), &raw mut client),
            LuminateStatus::Ok
        );

        assert_eq!(
            luminate_client_begin_shm_frame_stream(
                client,
                &raw const target,
                &raw mut stream_handle,
            ),
            LuminateStatus::Unsupported
        );
        assert!(stream_handle.is_null());

        luminate_client_free(client);
    }

    server.await.expect("shm server task");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "The test exercises one continuous shared-memory stream lifecycle across the C boundary."
)]
async fn shm_frame_stream_round_trips_through_live_services() {
    let daemon = FakeDaemon::bind("shm-ffi-round-trip");
    let path_c = daemon.c_path();
    let id = NEXT_SHM_SERVICE.fetch_add(1, Ordering::Relaxed);
    let service_name = format!("luminate-ffi-shm-pub-{}-{id}", process::id());
    let event_service_name = format!("luminate-ffi-shm-event-{}-{id}", process::id());
    let (_subscriber, _event_listener) = create_shm_services(&service_name, &event_service_name);
    let server_service_name = service_name.clone();
    let server_event_name = event_service_name.clone();
    let server = tokio::spawn(async move {
        let mut daemon = daemon;
        let mut stream = daemon.accept("shm-test-daemon").await;
        let request = respond(
            &mut stream,
            ResponseStatus::ShmFrameStreamReady {
                generation: 5,
                service_name: server_service_name,
                event_service_name: server_event_name,
                pixel_format: ShmPixelFormat::Rgb8,
                stream_nonce: 13,
                segment_bytes: u32::try_from(SHM_CLIENT_FRAME_HEADER_LEN + 3).expect("small frame"),
            },
        )
        .await;
        assert!(matches!(request, Request::BeginShmFrameStream { .. }));
        let request = respond(&mut stream, ResponseStatus::Ack).await;
        assert!(matches!(
            request,
            Request::EndShmFrameStream { generation: 5, .. }
        ));
    });

    let device = CString::new("keyboard").expect("valid device id");
    let target = LuminateTarget {
        device_id: device.as_ptr(),
        surface_id: ptr::null(),
        element_id: ptr::null(),
        group_id: ptr::null(),
    };
    let mut client = ptr::null_mut();
    let mut stream = ptr::null_mut();
    // SAFETY: all pointers refer to live local values or handles returned by
    // the preceding call. The stream and client are each released once.
    unsafe {
        assert_eq!(
            luminate_client_connect_path(path_c.as_ptr(), &raw mut client),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_begin_shm_frame_stream(client, &raw const target, &raw mut stream),
            LuminateStatus::Ok
        );
        luminate_client_free(client);

        let colours = [
            LuminateRgb { r: 1, g: 2, b: 3 },
            LuminateRgb { r: 4, g: 5, b: 6 },
        ];
        let mut ack = LuminateShmFrameAck { sequence: u64::MAX };
        assert_eq!(
            luminate_client_shm_upload_frame_full(stream, colours.as_ptr(), 1, 1, &raw mut ack,),
            LuminateStatus::Ok
        );
        assert_eq!(ack.sequence, 0);
        assert_eq!(
            luminate_client_shm_upload_frame_full(
                stream,
                colours.as_ptr(),
                colours.len(),
                1,
                &raw mut ack,
            ),
            LuminateStatus::InvalidArgument
        );
        assert_eq!(
            luminate_client_end_shm_frame_stream(stream),
            LuminateStatus::Ok
        );
    }

    server.await.expect("shm server task");
}

static NEXT_SHM_SERVICE: AtomicU64 = AtomicU64::new(0);

#[test]
fn shm_frame_stream_operations_reject_null_pointers() {
    let device = CString::new("keyboard").expect("valid device id");
    let target = LuminateTarget {
        device_id: device.as_ptr(),
        surface_id: ptr::null(),
        element_id: ptr::null(),
        group_id: ptr::null(),
    };
    let mut stream_handle = ptr::null_mut();
    let colour = LuminateRgb { r: 1, g: 2, b: 3 };
    let mut ack = LuminateShmFrameAck { sequence: 0 };

    // SAFETY: every non-null pointer refers to live local storage. A null
    // client/stream deliberately stops valid payloads before any dereference.
    unsafe {
        assert_eq!(
            luminate_client_begin_shm_frame_stream(
                ptr::null_mut(),
                &raw const target,
                &raw mut stream_handle,
            ),
            LuminateStatus::NullPointer
        );
        assert!(stream_handle.is_null());

        assert_eq!(
            luminate_client_shm_upload_frame_full(
                ptr::null_mut(),
                &raw const colour,
                1,
                0,
                &raw mut ack,
            ),
            LuminateStatus::NullPointer
        );

        // Null is a documented no-op, unlike every other combination.
        assert_eq!(
            luminate_client_end_shm_frame_stream(ptr::null_mut()),
            LuminateStatus::Ok
        );
    }
}
