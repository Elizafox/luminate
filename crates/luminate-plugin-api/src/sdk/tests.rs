// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

use luminate_core::frame::FrameEnvelope;
use luminate_core::shm_frame::{SHM_FRAME_HEADER_VERSION, ShmFrameHeader, ShmPixelFormat};

use crate::DeviceDescriptor;
use crate::{
    PluginApplyStatus, PluginReadTarget, PluginTarget, PluginUpdateBatch, PluginUpdateOperation,
};
use luminate_core::frame::FramePayload;
use std::ffi::c_char;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

struct FailingPlugin;

impl LuminatePlugin for FailingPlugin {
    fn new() -> Result<Self, PluginError> {
        Err(PluginError::InvalidArgument("bad configuration".to_owned()))
    }

    fn probe(&self) -> ProbeOutcome {
        ProbeOutcome::Ready
    }

    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
        Ok(Vec::new())
    }

    fn apply(
        &self,
        _context: &PluginRequestContext,
        _update: &PluginUpdate,
    ) -> Result<(), PluginError> {
        Ok(())
    }
}

struct PanickingPlugin;

impl LuminatePlugin for PanickingPlugin {
    fn new() -> Result<Self, PluginError> {
        Ok(Self)
    }

    fn probe(&self) -> ProbeOutcome {
        ProbeOutcome::Ready
    }

    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
        Ok(Vec::new())
    }

    #[allow(clippy::panic_in_result_fn, reason = "exercises ABI panic containment")]
    fn apply(
        &self,
        _context: &PluginRequestContext,
        _update: &PluginUpdate,
    ) -> Result<(), PluginError> {
        panic!("test panic")
    }
}

struct FullPlugin {
    start_called: AtomicBool,
}

impl LuminatePlugin for FullPlugin {
    fn new() -> Result<Self, PluginError> {
        Ok(Self {
            start_called: AtomicBool::new(false),
        })
    }

    fn probe(&self) -> ProbeOutcome {
        ProbeOutcome::Ready
    }

    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
        Ok(Vec::new())
    }

    fn apply(
        &self,
        _context: &PluginRequestContext,
        _update: &PluginUpdate,
    ) -> Result<(), PluginError> {
        Ok(())
    }
}

impl StartPlugin for FullPlugin {
    fn start(&self) {
        self.start_called.store(true, Ordering::Release);
    }
}

impl RescanPlugin for FullPlugin {
    fn rescan(&self, _reason: RescanReason) {}
}

impl BatchPlugin for FullPlugin {
    fn apply_batch(
        &self,
        _context: &PluginRequestContext,
        updates: &[PluginUpdate],
    ) -> Vec<Result<(), PluginError>> {
        updates.iter().map(|_| Ok(())).collect()
    }
}

impl ReadablePlugin for FullPlugin {
    fn read_state(
        &self,
        _context: &PluginRequestContext,
        _request: &PluginReadRequest,
    ) -> Result<PluginStateSnapshot, PluginError> {
        Ok(PluginStateSnapshot::default())
    }
}

impl FrameStreamingPlugin for FullPlugin {
    fn upload_frame(
        &self,
        _context: &PluginRequestContext,
        _target: &PluginTarget,
        _envelope: &FrameEnvelope,
    ) -> Result<(), PluginError> {
        Ok(())
    }
}

impl ShmFrameStreamingPlugin for FullPlugin {
    type Stream = u32;

    fn shm_stream_begin(
        &self,
        _context: &PluginRequestContext,
        _target: &PluginTarget,
        _format: ShmPixelFormat,
        _pixel_count: u32,
        generation: u32,
    ) -> Result<Self::Stream, PluginError> {
        Ok(generation)
    }

    fn shm_frame(
        &self,
        _context: &PluginRequestContext,
        stream: &mut Self::Stream,
        _header: &ShmFrameHeader,
        _pixels: &[u8],
    ) -> Result<(), PluginError> {
        *stream += 1;
        Ok(())
    }

    fn shm_stream_end(&self, _stream: Self::Stream, _generation: u32) {}
}

struct ShortBatchPlugin;

impl LuminatePlugin for ShortBatchPlugin {
    fn new() -> Result<Self, PluginError> {
        Ok(Self)
    }

    fn probe(&self) -> ProbeOutcome {
        ProbeOutcome::Ready
    }

    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
        Ok(Vec::new())
    }

    fn apply(
        &self,
        _context: &PluginRequestContext,
        _update: &PluginUpdate,
    ) -> Result<(), PluginError> {
        Ok(())
    }
}

impl BatchPlugin for ShortBatchPlugin {
    fn apply_batch(
        &self,
        _context: &PluginRequestContext,
        _updates: &[PluginUpdate],
    ) -> Vec<Result<(), PluginError>> {
        Vec::new()
    }
}

fn context() -> PluginRequestContext {
    PluginRequestContext::new(Duration::from_secs(1))
}

fn update() -> PluginUpdate {
    PluginUpdate {
        target: PluginTarget::Device {
            device: "example".to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    }
}

#[test]
fn callback_before_initialization_is_rejected() {
    let instance = OnceLock::<Result<FailingPlugin, PluginError>>::new();
    assert_eq!(probe(&instance), ProbeOutcome::Unsupported.to_abi());
}

#[test]
fn target_type_remains_available_to_facade_users() {
    let target = PluginTarget::Device {
        device: "example".to_owned(),
    };
    assert_eq!(target.device_id(), "example");
}

#[test]
fn apply_contains_plugin_panics_and_writes_an_internal_result() {
    let instance = OnceLock::from(Ok(PanickingPlugin));
    let encoded = crate::encode_cbor(&update()).expect("serialize update");
    let mut result = PluginApplyResult::applied();

    // SAFETY: the encoded update and writable result remain valid for the call.
    let accepted = unsafe {
        apply(
            &instance,
            context(),
            encoded.as_ptr(),
            encoded.len(),
            &raw mut result,
        )
    };

    assert_eq!(accepted, 1);
    assert_eq!(
        result.decode().expect("decode result").0,
        PluginApplyStatus::Internal
    );
}

#[test]
fn native_batch_rejects_the_wrong_number_of_plugin_results() {
    let instance = OnceLock::from(Ok(ShortBatchPlugin));
    let encoded = crate::encode_cbor(&PluginUpdateBatch {
        updates: vec![update()],
    })
    .expect("serialize batch");
    let mut results = [PluginApplyResult::applied()];

    // SAFETY: the encoded batch and writable result remain valid for the call.
    let accepted = unsafe {
        apply_native_batch(
            &instance,
            context(),
            encoded.as_ptr(),
            encoded.len(),
            results.as_mut_ptr(),
            results.len(),
        )
    };

    assert_eq!(accepted, 0);
}

#[test]
fn read_state_rejects_null_and_malformed_requests() {
    struct Reader;
    impl LuminatePlugin for Reader {
        fn new() -> Result<Self, PluginError> {
            Ok(Self)
        }
        fn probe(&self) -> ProbeOutcome {
            ProbeOutcome::Ready
        }
        fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
            Ok(Vec::new())
        }
        fn apply(&self, _: &PluginRequestContext, _: &PluginUpdate) -> Result<(), PluginError> {
            Ok(())
        }
    }
    impl ReadablePlugin for Reader {
        fn read_state(
            &self,
            _: &PluginRequestContext,
            _: &PluginReadRequest,
        ) -> Result<PluginStateSnapshot, PluginError> {
            Ok(PluginStateSnapshot::default())
        }
    }
    let instance = OnceLock::from(Ok(Reader));
    let malformed = [0xff];

    // SAFETY: null input is intentional and rejected before dereferencing.
    let null_result =
        unsafe { read_state(&instance, context(), ptr::null(), 0, ptr::null_mut(), 0) };
    assert_eq!(null_result, usize::MAX);

    // SAFETY: `malformed` remains readable for the call; no output buffer is supplied.
    let malformed_result = unsafe {
        read_state(
            &instance,
            context(),
            malformed.as_ptr(),
            malformed.len(),
            ptr::null_mut(),
            0,
        )
    };
    assert_eq!(malformed_result, usize::MAX);
}

unsafe extern "C" fn noop_log(_plugin_name: *const c_char, _level: u8, _message: *const c_char) {}

unsafe extern "C" fn noop_notify(_plugin_name: *const c_char) {}

/// Calls `initialize` with no-op callbacks and an empty configuration.
#[allow(
    clippy::semicolon_if_nothing_returned,
    reason = "rustfmt wraps this call across lines, and in that shape clippy's \
              semicolon_if_nothing_returned and semicolon_outside_block lints \
              disagree about where the `;` belongs"
)]
fn call_initialize(instance: &OnceLock<Result<FullPlugin, PluginError>>, name: &'static CStr) {
    // SAFETY: the callbacks are no-ops and the configuration pointer is
    // null, which `configuration::init` treats as an empty object.
    unsafe { initialize::<FullPlugin>(instance, name, noop_log, 0, noop_notify, ptr::null(), 0) };
}

#[test]
fn initialize_constructs_and_installs_the_plugin_instance_once() {
    let instance = OnceLock::<Result<FullPlugin, PluginError>>::new();
    let name = c"initialize-test-plugin";

    call_initialize(&instance, name);
    assert!(matches!(instance.get(), Some(Ok(_))));

    // A second call must not overwrite the installed instance or panic;
    // it only logs that initialization ran twice.
    call_initialize(&instance, name);
    assert!(matches!(instance.get(), Some(Ok(_))));
}

#[test]
fn start_runs_the_plugins_start_method() {
    let instance = OnceLock::from(Ok(FullPlugin {
        start_called: AtomicBool::new(false),
    }));
    start(&instance);
    let Ok(plugin) = instance.get().expect("instance was installed") else {
        panic!("instance holds an error");
    };
    assert!(plugin.start_called.load(Ordering::Acquire));
}

#[test]
fn start_before_initialization_does_not_panic() {
    let instance = OnceLock::<Result<FullPlugin, PluginError>>::new();
    start(&instance);
}

#[test]
fn probe_and_rescan_handle_initialized_plugins_and_unknown_reasons() {
    let instance = OnceLock::from(Ok(FullPlugin {
        start_called: AtomicBool::new(false),
    }));
    assert_eq!(probe(&instance), ProbeOutcome::Ready.to_abi());
    rescan(&instance, RescanReason::Operator.to_abi());
    rescan(&instance, u8::MAX);
}

#[test]
fn topology_serializes_the_plugins_snapshot_into_the_cache() {
    let instance = OnceLock::from(Ok(FullPlugin {
        start_called: AtomicBool::new(false),
    }));
    let cache = OnceLock::new();
    let mut length = 0_usize;

    // SAFETY: `length` is a writable local for the duration of the call.
    let pointer = unsafe { topology(&instance, &cache, &raw mut length) };
    assert!(!pointer.is_null());
    assert!(length > 0);
    // SAFETY: the callback contract guarantees `length` readable bytes at `pointer`.
    let encoded = unsafe { slice::from_raw_parts(pointer, length) };
    let decoded: Vec<DeviceDescriptor> =
        ciborium::from_reader(encoded).expect("cached topology decodes");
    assert!(decoded.is_empty());

    // A null length is valid when the caller only needs the stable pointer.
    // SAFETY: the cache remains live for the duration of the call.
    assert!(!unsafe { topology(&instance, &cache, ptr::null_mut()) }.is_null());
}

#[test]
fn topology_serializes_an_empty_snapshot_when_the_plugin_is_unavailable() {
    let instance = OnceLock::<Result<FullPlugin, PluginError>>::from(Err(
        PluginError::Unavailable("device vanished".to_owned()),
    ));
    let cache = OnceLock::new();
    let mut length = 0_usize;

    // SAFETY: `length` is a writable local for the duration of the call.
    let pointer = unsafe { topology(&instance, &cache, &raw mut length) };
    assert!(!pointer.is_null());
    // SAFETY: the callback contract guarantees `length` readable bytes at `pointer`.
    let encoded = unsafe { slice::from_raw_parts(pointer, length) };
    let decoded: Vec<DeviceDescriptor> =
        ciborium::from_reader(encoded).expect("fallback topology decodes");
    assert!(decoded.is_empty());
}

#[test]
fn apply_applies_a_successful_update() {
    let instance = OnceLock::from(Ok(FullPlugin {
        start_called: AtomicBool::new(false),
    }));
    let encoded = crate::encode_cbor(&update()).expect("serialize update");
    let mut result = PluginApplyResult::internal("unwritten");

    // SAFETY: the encoded update and writable result remain valid for the call.
    let accepted = unsafe {
        apply(
            &instance,
            context(),
            encoded.as_ptr(),
            encoded.len(),
            &raw mut result,
        )
    };

    assert_eq!(accepted, 1);
    assert_eq!(
        result.decode().expect("decode result").0,
        PluginApplyStatus::Applied
    );
}

#[test]
fn ordered_apply_batch_applies_each_update_in_sequence() {
    let instance = OnceLock::from(Ok(FullPlugin {
        start_called: AtomicBool::new(false),
    }));
    let encoded = crate::encode_cbor(&PluginUpdateBatch {
        updates: vec![update(), update()],
    })
    .expect("serialize batch");
    let mut results = [
        PluginApplyResult::internal("unwritten"),
        PluginApplyResult::internal("unwritten"),
    ];

    // SAFETY: the encoded batch and writable results remain valid for the call.
    let accepted = unsafe {
        apply_batch(
            &instance,
            context(),
            encoded.as_ptr(),
            encoded.len(),
            results.as_mut_ptr(),
            results.len(),
        )
    };

    assert_eq!(accepted, 1);
    for result in &results {
        assert_eq!(
            result.decode().expect("decode result").0,
            PluginApplyStatus::Applied
        );
    }
}

#[test]
fn native_batch_succeeds_with_a_matching_result_count() {
    let instance = OnceLock::from(Ok(FullPlugin {
        start_called: AtomicBool::new(false),
    }));
    let encoded = crate::encode_cbor(&PluginUpdateBatch {
        updates: vec![update(), update()],
    })
    .expect("serialize batch");
    let mut results = [
        PluginApplyResult::internal("unwritten"),
        PluginApplyResult::internal("unwritten"),
    ];

    // SAFETY: the encoded batch and writable results remain valid for the call.
    let accepted = unsafe {
        apply_native_batch(
            &instance,
            context(),
            encoded.as_ptr(),
            encoded.len(),
            results.as_mut_ptr(),
            results.len(),
        )
    };

    assert_eq!(accepted, 1);
    for result in &results {
        assert_eq!(
            result.decode().expect("decode result").0,
            PluginApplyStatus::Applied
        );
    }
}

#[test]
fn native_batch_reports_an_unavailable_plugin_for_each_update() {
    let instance = OnceLock::<Result<FullPlugin, PluginError>>::from(Err(
        PluginError::Unavailable("transport is offline".to_owned()),
    ));
    let encoded = crate::encode_cbor(&PluginUpdateBatch {
        updates: vec![update(), update()],
    })
    .expect("serialize batch");
    let mut results = [
        PluginApplyResult::internal("unwritten"),
        PluginApplyResult::internal("unwritten"),
    ];

    // SAFETY: the encoded batch and writable results remain valid for the call.
    let accepted = unsafe {
        apply_native_batch(
            &instance,
            context(),
            encoded.as_ptr(),
            encoded.len(),
            results.as_mut_ptr(),
            results.len(),
        )
    };

    assert_eq!(accepted, 1);
    for result in &results {
        assert_eq!(
            result.decode().expect("decode result").0,
            PluginApplyStatus::Unavailable
        );
    }
}

#[test]
fn upload_frame_applies_a_successful_frame() {
    let instance = OnceLock::from(Ok(FullPlugin {
        start_called: AtomicBool::new(false),
    }));
    let frame_upload = crate::PluginFrameUpload {
        target: PluginTarget::Device {
            device: "example".to_owned(),
        },
        envelope: FrameEnvelope {
            generation: 1,
            sequence: 0,
            payload: FramePayload::Full(Vec::new()),
            commit: false,
        },
    };
    let encoded = crate::encode_cbor(&frame_upload).expect("serialize frame upload");
    let mut result = PluginApplyResult::internal("unwritten");

    // SAFETY: the encoded frame and writable result remain valid for the call.
    let accepted = unsafe {
        upload_frame(
            &instance,
            context(),
            encoded.as_ptr(),
            encoded.len(),
            &raw mut result,
        )
    };

    assert_eq!(accepted, 1);
    assert_eq!(
        result.decode().expect("decode result").0,
        PluginApplyStatus::Applied
    );
}

#[test]
fn shared_memory_stream_callbacks_preserve_the_opaque_stream_state() {
    let instance = OnceLock::from(Ok(FullPlugin {
        start_called: AtomicBool::new(false),
    }));
    let target = crate::encode_cbor(&PluginTarget::Device {
        device: "example".to_owned(),
    })
    .expect("serialize target");
    let mut handle = 0_u64;
    let mut result = PluginApplyResult::internal("unwritten");

    // SAFETY: the target, handle, and result remain valid for the call.
    let accepted = unsafe {
        shm_stream_begin(
            &instance,
            context(),
            target.as_ptr(),
            target.len(),
            ShmPixelFormat::Rgb8.to_abi(),
            1,
            7,
            &raw mut handle,
            &raw mut result,
        )
    };
    assert_eq!(accepted, 1);
    assert_ne!(handle, 0);

    let header = ShmFrameHeader {
        sequence: 1,
        generation: 7,
        pixel_count: 1,
        pixel_format: ShmPixelFormat::Rgb8.to_abi(),
        header_version: SHM_FRAME_HEADER_VERSION,
        flags: 0,
        reserved: 0,
    };
    let pixels = [1_u8, 2, 3];
    // SAFETY: `handle` came from the matching begin call and remains uniquely owned.
    let accepted = unsafe {
        shm_frame(
            &instance,
            context(),
            handle,
            header,
            pixels.as_ptr(),
            pixels.len(),
            &raw mut result,
        )
    };
    assert_eq!(accepted, 1);
    assert_eq!(
        result.decode().expect("decode result").0,
        PluginApplyStatus::Applied
    );

    // SAFETY: `handle` came from the matching begin call and is ended exactly once.
    unsafe { shm_stream_end(&instance, context(), handle, 7) };
    // SAFETY: zero is the documented no-active-stream sentinel and owns no allocation.
    unsafe { shm_stream_end(&instance, context(), 0, 7) };
}

#[test]
fn shared_memory_frame_rejects_the_zero_handle_sentinel() {
    let instance = OnceLock::from(Ok(FullPlugin {
        start_called: AtomicBool::new(false),
    }));
    let mut result = PluginApplyResult::internal("unwritten");

    // SAFETY: handle zero is rejected before any pointer is derived from it.
    let accepted = unsafe {
        shm_frame(
            &instance,
            context(),
            0,
            ShmFrameHeader {
                sequence: 0,
                generation: 0,
                pixel_count: 0,
                pixel_format: ShmPixelFormat::Rgb8.to_abi(),
                header_version: SHM_FRAME_HEADER_VERSION,
                flags: 0,
                reserved: 0,
            },
            ptr::null(),
            0,
            &raw mut result,
        )
    };

    assert_eq!(accepted, 1);
    assert_eq!(
        result.decode().expect("decode result").0,
        PluginApplyStatus::InvalidArgument
    );
}

#[test]
fn read_state_returns_the_plugins_snapshot() {
    let instance = OnceLock::from(Ok(FullPlugin {
        start_called: AtomicBool::new(false),
    }));
    let request = PluginReadRequest {
        targets: Vec::new(),
    };
    let encoded = crate::encode_cbor(&request).expect("serialize request");
    let mut output = vec![0_u8; 256];

    // SAFETY: the encoded request and output buffer remain valid for the call.
    let length = unsafe {
        read_state(
            &instance,
            context(),
            encoded.as_ptr(),
            encoded.len(),
            output.as_mut_ptr(),
            output.len(),
        )
    };

    assert_ne!(length, usize::MAX);
    assert!(length <= output.len());
    let snapshot: PluginStateSnapshot =
        ciborium::from_reader(&output[..length]).expect("decode snapshot");
    assert!(snapshot.observations.is_empty());
    assert!(snapshot.errors.is_empty());
}

#[test]
fn read_state_maps_a_whole_plugin_failure_to_each_requested_target() {
    let instance = OnceLock::<Result<FullPlugin, PluginError>>::from(Err(
        PluginError::Unavailable("readback transport is offline".to_owned()),
    ));
    let target = PluginTarget::Device {
        device: "example".to_owned(),
    };
    let request = PluginReadRequest {
        targets: vec![PluginReadTarget {
            target: target.clone(),
            facets: Vec::new(),
        }],
    };
    let encoded = crate::encode_cbor(&request).expect("serialize request");
    let mut output = vec![0_u8; 512];

    // SAFETY: the encoded request and output buffer remain valid for the call.
    let length = unsafe {
        read_state(
            &instance,
            context(),
            encoded.as_ptr(),
            encoded.len(),
            output.as_mut_ptr(),
            output.len(),
        )
    };

    let snapshot: PluginStateSnapshot =
        ciborium::from_reader(&output[..length]).expect("decode fallback snapshot");
    assert!(snapshot.observations.is_empty());
    assert_eq!(snapshot.errors.len(), 1);
    assert_eq!(snapshot.errors[0].target.to_string(), target.to_string());
    assert!(snapshot.errors[0].diagnostic.contains("offline"));
}
