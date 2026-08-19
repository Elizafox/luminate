// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::slice;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::plugin_host::lock_fixture;
use luminate_core::capability::ShmFrameShape;
use luminate_core::shm_frame::SHM_FRAME_HEADER_VERSION;
use luminate_plugin_api::write_apply_result;

use super::*;

type RecordedFrame = (ShmFrameHeader, Vec<u8>);

/// Serializes tests in this module: they share the `static` recording
/// state the synthetic ABI callbacks below write into, and iceoryx2
/// service names must stay unique across concurrently running tests
/// (see `unique_target`), not just within one test. Always taken
/// through [`lock_fixture`](crate::plugin_host::lock_fixture), never
/// `.unwrap()`.
static FIXTURE_LOCK: Mutex<()> = Mutex::new(());
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
static RECEIVED_HANDLE: OnceLock<Mutex<Option<u64>>> = OnceLock::new();
static RECEIVED_FRAMES: OnceLock<Mutex<Vec<RecordedFrame>>> = OnceLock::new();
static ENDED: OnceLock<Mutex<Option<(u64, u32)>>> = OnceLock::new();

fn reset_fixture_state() {
    RECEIVED_HANDLE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .take();
    RECEIVED_FRAMES
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .clear();
    ENDED
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .take();
}

/// A process-wide-unique iceoryx2-service-safe target string, so
/// concurrently running tests (in this module or, via `cargo test`'s
/// default parallelism, any other) never collide on the same
/// publish-subscribe/event service names.
fn unique_target(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    format!("shm-runtime-test-{label}-{nanos}")
}

fn test_callbacks() -> ShmCallbacks {
    ShmCallbacks {
        begin: test_shm_begin,
        apply: test_shm_apply,
        end: test_shm_end,
    }
}

unsafe extern "C" fn test_shm_begin(
    _context: PluginRequestContext,
    target_cbor: *const u8,
    _target_len: usize,
    _pixel_format: u32,
    _pixel_count: u32,
    _generation: u32,
    handle_out: *mut u64,
    result: *mut PluginApplyResult,
) -> u8 {
    if target_cbor.is_null() || handle_out.is_null() || result.is_null() {
        return 0;
    }
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
    RECEIVED_HANDLE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .replace(handle);
    // SAFETY: the test host supplies a writable `u64` slot per the ABI contract.
    unsafe { handle_out.write(handle) };
    // SAFETY: the test host supplies one writable result slot.
    unsafe { write_apply_result(result, PluginApplyResult::applied()) }
}

unsafe extern "C" fn test_shm_apply(
    _context: PluginRequestContext,
    _handle: u64,
    header: ShmFrameHeader,
    pixels: *const u8,
    pixels_len: usize,
    result: *mut PluginApplyResult,
) -> u8 {
    if result.is_null() {
        return 0;
    }
    let bytes = if pixels.is_null() || pixels_len == 0 {
        Vec::new()
    } else {
        // SAFETY: the test host guarantees `pixels_len` readable bytes
        // at `pixels` for the duration of this call.
        unsafe { slice::from_raw_parts(pixels, pixels_len) }.to_vec()
    };
    RECEIVED_FRAMES
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .push((header, bytes));
    // SAFETY: the test host supplies one writable result slot.
    unsafe { write_apply_result(result, PluginApplyResult::applied()) }
}

unsafe extern "C" fn test_shm_end(_context: PluginRequestContext, handle: u64, generation: u32) {
    ENDED
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .replace((handle, generation));
}

/// Publishes one test frame through the same shared-memory services used by
/// `plugins::shm`.
///
/// Service names are derived from `plugin_name` and the rendered `target`.
/// A fresh publisher and notifier are created, then `header` and `pixels`
/// are published until the stream thread reports receiving the frame.
///
/// A newly created publisher and subscriber can briefly race while iceoryx2
/// establishes their connection, so the first sample may be lost during
/// otherwise normal startup. Production frame streams tolerate this because
/// delivery is fire-and-forget and a later frame follows shortly afterward.
///
/// This helper retries every 50 ms for at most five seconds solely to make
/// that startup race deterministic in tests. The retry interval is not an
/// assumption about steady-state delivery latency.
fn publish_until_received(
    plugin_name: &str,
    target: &PluginTarget,
    segment_bytes: u32,
    header: ShmFrameHeader,
    pixels: &[u8],
) {
    let names = luminate_host_supervisor::service_names(plugin_name, &target.to_string())
        .expect("derive service names");
    let node = luminate_host_supervisor::create_node().expect("create iceoryx2 node");
    let pubsub = node
        .service_builder(&names.publish_subscribe)
        .publish_subscribe::<[u8]>()
        .open_or_create()
        .expect("open publish-subscribe service");
    let publisher = pubsub
        .publisher_builder()
        .initial_max_slice_len(segment_bytes as usize)
        .create()
        .expect("create publisher");
    let event = node
        .service_builder(&names.event)
        .event()
        .open_or_create()
        .expect("open event service");
    let notifier = event.notifier_builder().create().expect("create notifier");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut sample = publisher
            .loan_slice(segment_bytes as usize)
            .expect("loan a sample");
        let payload = sample.payload_mut();
        let (header_bytes, pixel_bytes) = payload.split_at_mut(size_of::<ShmFrameHeader>());
        header_bytes.copy_from_slice(&header.to_bytes());
        pixel_bytes.copy_from_slice(pixels);
        sample.send().expect("send the sample");
        notifier.notify().expect("notify the listener");

        thread::sleep(Duration::from_millis(50));
        if !RECEIVED_FRAMES
            .get()
            .expect("initialized by reset_fixture_state")
            .lock()
            .unwrap()
            .is_empty()
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the shared-memory frame to be applied"
        );
    }
}

#[test]
fn begin_apply_end_round_trips_through_real_iceoryx2_services() {
    let _fixture = lock_fixture(&FIXTURE_LOCK);
    reset_fixture_state();

    let runtime = ShmRuntime::default();
    let plugin_name = "shm-runtime-test-plugin";
    let target = PluginTarget::Device {
        device: unique_target("round-trip"),
    };
    let request = BeginShmStreamRequest {
        target: target.clone(),
        generation: 7,
        pixel_format: ShmPixelFormat::Rgb8.to_abi(),
        shape: ShmFrameShape::Linear { pixel_count: 2 },
    };
    let context = PluginRequestContext::new(Duration::from_secs(5));

    let outcome = runtime
        .begin(plugin_name, &request, test_callbacks(), context)
        .expect("begin should not error");
    let advertised_segment_bytes = match outcome {
        ShmStreamOutcome::Ready { segment_bytes } => segment_bytes,
        ShmStreamOutcome::Acknowledged
        | ShmStreamOutcome::Unsupported(_)
        | ShmStreamOutcome::Io(_)
        | ShmStreamOutcome::Internal(_) => {
            panic!("expected Ready, got {outcome:?}")
        }
    };
    assert_eq!(
        Some(advertised_segment_bytes),
        segment_bytes(ShmPixelFormat::Rgb8, 2)
    );
    let handle = RECEIVED_HANDLE
        .get()
        .expect("initialized by reset_fixture_state")
        .lock()
        .unwrap()
        .expect("plugin should have received a begin call");

    let header = ShmFrameHeader {
        sequence: 42,
        generation: 7,
        pixel_count: 2,
        pixel_format: ShmPixelFormat::Rgb8.to_abi(),
        header_version: SHM_FRAME_HEADER_VERSION,
        flags: 0,
        reserved: 0,
    };
    let pixels = [10_u8, 20, 30, 40, 50, 60];
    publish_until_received(
        plugin_name,
        &target,
        advertised_segment_bytes,
        header,
        &pixels,
    );

    let frames = RECEIVED_FRAMES.get().unwrap().lock().unwrap();
    assert!(!frames.is_empty());
    let (received_header, received_pixels) = &frames[0];
    assert_eq!(received_header.sequence, 42);
    assert_eq!(received_header.generation, 7);
    assert_eq!(received_pixels.as_slice(), pixels);
    drop(frames);

    let end_outcome = runtime.end(&target, 7, context);
    assert!(matches!(end_outcome, ShmStreamOutcome::Acknowledged));
    let (ended_handle, ended_generation) = ENDED
        .get()
        .expect("initialized by reset_fixture_state")
        .lock()
        .unwrap()
        .expect("plugin should have received an end call");
    assert_eq!(ended_handle, handle);
    assert_eq!(ended_generation, 7);
}

#[test]
fn begin_rejects_an_unrecognized_pixel_format() {
    let _fixture = lock_fixture(&FIXTURE_LOCK);
    reset_fixture_state();

    let runtime = ShmRuntime::default();
    let request = BeginShmStreamRequest {
        target: PluginTarget::Device {
            device: unique_target("bad-format"),
        },
        generation: 1,
        pixel_format: 999,
        shape: ShmFrameShape::Linear { pixel_count: 1 },
    };
    let outcome = runtime
        .begin(
            "shm-runtime-test-plugin",
            &request,
            test_callbacks(),
            PluginRequestContext::new(Duration::from_secs(1)),
        )
        .expect("an unrecognized format is a decline, not an error");
    assert!(matches!(outcome, ShmStreamOutcome::Unsupported(_)));
}

#[test]
fn end_with_a_stale_generation_is_a_no_op() {
    let runtime = ShmRuntime::default();
    let target = PluginTarget::Device {
        device: unique_target("no-such-stream"),
    };
    let outcome = runtime.end(
        &target,
        5,
        PluginRequestContext::new(Duration::from_secs(1)),
    );
    assert!(matches!(outcome, ShmStreamOutcome::Acknowledged));
}

#[test]
fn reset_on_an_unknown_target_is_a_harmless_no_op() {
    let runtime = ShmRuntime::default();
    let target = PluginTarget::Device {
        device: unique_target("no-such-stream"),
    };
    assert!(matches!(
        runtime.reset(&target, 9),
        ShmStreamOutcome::Acknowledged
    ));
}
