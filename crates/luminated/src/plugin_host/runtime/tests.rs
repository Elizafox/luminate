// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Unit tests for the isolated native plugin host.

use std::{ptr, result};

use luminate_core::capability;
use luminate_core::control::ReconciliationPolicy;
use luminate_core::element::ElementKind;
use luminate_core::surface::SurfaceKind;
use serde::ser;
use std::ffi::CString;
use std::io;
use std::sync::atomic::AtomicU8;
use std::thread;
use std::time::Instant;

use crate::plugin_host::lock_fixture;
use crate::plugins::TopologyNotification;
#[cfg(unix)]
use libloading::os::unix::Library as PlatformLibrary;
#[cfg(windows)]
use libloading::os::windows::Library as PlatformLibrary;
use luminate_core::frame::{FrameEnvelope, FramePayload};
use luminate_core::shm_frame::{ShmFrameHeader, ShmPixelFormat};
use luminate_plugin_api::{
    ElementDescriptor, PluginTarget, PluginUpdateOperation, PluginVendorId, SurfaceDescriptor,
    write_apply_result, write_state_snapshot,
};
use tokio::sync::mpsc::unbounded_channel;

use super::*;

static TOPOLOGY_CBOR: OnceLock<Mutex<Vec<u8>>> = OnceLock::new();

/// Serializes every test that touches the process-global ABI fixture
/// statics above. Always taken through
/// [`lock_fixture`](crate::plugin_host::lock_fixture), never `.unwrap()`.
static ABI_FIXTURE_LOCK: Mutex<()> = Mutex::new(());

/// Reason bytes seen by `recording_rescan_callback`, in call order.
static RESCAN_REASONS: OnceLock<Mutex<Vec<u8>>> = OnceLock::new();

unsafe extern "C" fn recording_rescan_callback(reason: u8) {
    RESCAN_REASONS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .push(reason);
}
static APPLY_MODE: AtomicU8 = AtomicU8::new(0);
static BATCH_MODE: AtomicU8 = AtomicU8::new(0);
static READ_MODE: AtomicU8 = AtomicU8::new(0);
static FRAME_MODE: AtomicU8 = AtomicU8::new(0);

fn test_context() -> PluginRequestContext {
    PluginRequestContext::new(Duration::from_secs(5))
}

fn set_topology(descriptors: &[DeviceDescriptor]) {
    let encoded = luminate_plugin_api::topology_cbor(descriptors);
    *TOPOLOGY_CBOR
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap() = encoded;
}

unsafe extern "C" fn topology_callback(length: *mut usize) -> *const u8 {
    let encoded = TOPOLOGY_CBOR
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap();
    if !length.is_null() {
        // SAFETY: the test caller supplies a writable length slot.
        unsafe { length.write(encoded.len()) };
    }
    encoded.as_ptr()
}

unsafe extern "C" fn null_topology_callback(_length: *mut usize) -> *const u8 {
    ptr::null()
}

unsafe extern "C" fn invalid_utf8_topology_callback(length: *mut usize) -> *const u8 {
    static INVALID_CBOR: [u8; 1] = [u8::MAX];
    if !length.is_null() {
        // SAFETY: the test caller supplies a writable length slot.
        unsafe { length.write(INVALID_CBOR.len()) };
    }
    INVALID_CBOR.as_ptr()
}

unsafe extern "C" fn apply_callback(
    _context: PluginRequestContext,
    update_cbor: *const u8,
    update_len: usize,
    result: *mut PluginApplyResult,
) -> u8 {
    if update_cbor.is_null() || result.is_null() || APPLY_MODE.load(Ordering::Acquire) == 6 {
        return 0;
    }
    // SAFETY: the test host passes a live length-delimited CBOR envelope.
    let encoded = unsafe { slice::from_raw_parts(update_cbor, update_len) };
    if ciborium::from_reader::<PluginUpdate, _>(encoded).is_err() {
        return 0;
    }
    let value = match APPLY_MODE.load(Ordering::Acquire) {
        0 => PluginApplyResult::applied(),
        1 => PluginApplyResult::unsupported("not supported"),
        2 => PluginApplyResult::invalid_argument("bad update"),
        3 => PluginApplyResult::io("device unavailable"),
        4 => PluginApplyResult::internal("plugin failed"),
        5 => {
            let mut malformed = PluginApplyResult::applied();
            malformed.code = u8::MAX;
            malformed
        }
        mode => panic!("unexpected apply mode {mode}"),
    };
    // SAFETY: the host supplied one writable result slot.
    unsafe { write_apply_result(result, value) }
}

unsafe extern "C" fn batch_callback(
    _context: PluginRequestContext,
    batch_cbor: *const u8,
    batch_len: usize,
    results: *mut PluginApplyResult,
    results_len: usize,
) -> u8 {
    if batch_cbor.is_null() || results.is_null() || BATCH_MODE.load(Ordering::Acquire) == 1 {
        return 0;
    }
    // SAFETY: the test host passes a live length-delimited CBOR envelope.
    let encoded = unsafe { slice::from_raw_parts(batch_cbor, batch_len) };
    let Ok(batch) = ciborium::from_reader::<PluginUpdateBatch, _>(encoded) else {
        return 0;
    };
    if batch.updates.len() != results_len {
        return 0;
    }
    // SAFETY: the host supplies exactly `results_len` writable slots.
    let output = unsafe { slice::from_raw_parts_mut(results, results_len) };
    for (index, result) in output.iter_mut().enumerate() {
        *result = if BATCH_MODE.load(Ordering::Acquire) == 2 && index == 0 {
            let mut malformed = PluginApplyResult::applied();
            malformed.flags = u8::MAX;
            malformed
        } else if index % 2 == 0 {
            PluginApplyResult::applied()
        } else {
            PluginApplyResult::unsupported("batch item unsupported")
        };
    }
    1
}

unsafe extern "C" fn read_state_callback(
    _context: PluginRequestContext,
    _request_cbor: *const u8,
    _request_len: usize,
    output: *mut u8,
    output_capacity: usize,
) -> usize {
    match READ_MODE.load(Ordering::Acquire) {
        0 => {
            // SAFETY: the host supplies a writable buffer of this capacity.
            unsafe {
                write_state_snapshot(output, output_capacity, &PluginStateSnapshot::default())
            }
        }
        1 => usize::MAX,
        2 => output_capacity + 1,
        3 => {
            if !output.is_null() && output_capacity > 0 {
                // SAFETY: the checked buffer contains at least one byte.
                unsafe { output.write(b'{') };
            }
            1
        }
        mode => panic!("unexpected read mode {mode}"),
    }
}

unsafe extern "C" fn frame_callback(
    _context: PluginRequestContext,
    frame_cbor: *const u8,
    frame_len: usize,
    result: *mut PluginApplyResult,
) -> u8 {
    if frame_cbor.is_null() || result.is_null() || FRAME_MODE.load(Ordering::Acquire) == 2 {
        return 0;
    }
    // SAFETY: the test host passes a live length-delimited CBOR envelope.
    let encoded = unsafe { slice::from_raw_parts(frame_cbor, frame_len) };
    if ciborium::from_reader::<PluginFrameUpload, _>(encoded).is_err() {
        return 0;
    }
    let value = match FRAME_MODE.load(Ordering::Acquire) {
        0 => PluginApplyResult::applied(),
        1 => PluginApplyResult::io("frame rejected"),
        mode => panic!("unexpected frame mode {mode}"),
    };
    // SAFETY: the host supplied one writable result slot.
    unsafe { write_apply_result(result, value) }
}

static SHM_ENDED: Mutex<Vec<(u64, u32)>> = Mutex::new(Vec::new());

unsafe extern "C" fn shm_begin_callback(
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
    // SAFETY: the test host supplies a writable `u64` slot per the ABI contract.
    unsafe { handle_out.write(1) };
    // SAFETY: the host supplied one writable result slot.
    unsafe { write_apply_result(result, PluginApplyResult::applied()) }
}

unsafe extern "C" fn shm_apply_callback(
    _context: PluginRequestContext,
    _handle: u64,
    _header: ShmFrameHeader,
    _pixels: *const u8,
    _pixels_len: usize,
    result: *mut PluginApplyResult,
) -> u8 {
    if result.is_null() {
        return 0;
    }
    // SAFETY: the host supplied one writable result slot.
    unsafe { write_apply_result(result, PluginApplyResult::applied()) }
}

unsafe extern "C" fn shm_end_callback(
    _context: PluginRequestContext,
    handle: u64,
    generation: u32,
) {
    SHM_ENDED.lock().unwrap().push((handle, generation));
}

unsafe extern "C" fn noop_init(
    _log: luminate_plugin_api::PluginLogFn,
    _max_level: u8,
    _notify: luminate_plugin_api::PluginNotifyFn,
    _configuration_cbor: *const u8,
    _configuration_len: usize,
) {
}

fn native_plugin(
    topology_cbor: Option<PluginTopologyCborFn>,
    apply_update_cbor: Option<PluginApplyUpdateFn>,
    apply_batch_cbor: Option<PluginApplyBatchFn>,
    read_state_cbor: Option<PluginReadStateFn>,
) -> NativePlugin {
    #[cfg(unix)]
    let library: Library = PlatformLibrary::this().into();
    #[cfg(windows)]
    let library: Library = PlatformLibrary::this()
        .expect("loading a handle to this same process should never fail")
        .into();
    NativePlugin {
        metadata: HostMetadata {
            name: "test-plugin".to_owned(),
            version: "1".to_owned(),
            priority: 0,
            recommended_reconciliation: None,
            probe_outcome: ProbeOutcome::Ready,
            buses: Vec::new(),
            vendors: Vec::new(),
            probe_hints: Vec::new(),
        },
        topology_cbor,
        rescan: None,
        apply_update_cbor,
        apply_batch_cbor,
        read_state_cbor,
        frame_upload_cbor: None,
        shm_stream_begin: None,
        shm_frame_apply: None,
        shm_stream_end: None,
        shm: ShmRuntime::default(),
        _library: ManuallyDrop::new(library),
    }
}

fn sample_update() -> PluginUpdate {
    PluginUpdate {
        target: PluginTarget::Device {
            device: "demo".to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    }
}

fn native_plugin_with_frame(frame_upload_cbor: Option<PluginFrameUploadFn>) -> NativePlugin {
    let mut plugin = native_plugin(None, None, None, None);
    plugin.frame_upload_cbor = frame_upload_cbor;
    plugin
}

fn native_plugin_with_shm() -> NativePlugin {
    let mut plugin = native_plugin(None, None, None, None);
    plugin.shm_stream_begin = Some(shm_begin_callback);
    plugin.shm_frame_apply = Some(shm_apply_callback);
    plugin.shm_stream_end = Some(shm_end_callback);
    plugin
}

fn sample_frame_upload() -> PluginFrameUpload {
    PluginFrameUpload {
        target: PluginTarget::Device {
            device: "demo".to_owned(),
        },
        envelope: FrameEnvelope {
            generation: 1,
            sequence: 0,
            payload: FramePayload::Full(Vec::new()),
            commit: false,
        },
    }
}

fn sample_descriptor(id: &str) -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: id.to_owned(),
        name: id.to_owned(),
        vendor: None,
        model: None,
        surfaces: Vec::new(),
        groups: Vec::new(),
        capabilities: CapabilitySet::default(),
        category: None,
        physical_tags: Vec::new(),
        host_attached: false,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

fn tagged_descriptor(id: &str) -> DeviceDescriptor {
    let mut descriptor = sample_descriptor(id);
    descriptor.physical_tags = vec!["shape:fixture".to_owned()];
    descriptor.surfaces.push(SurfaceDescriptor {
        id: "main".to_owned(),
        name: "Main".to_owned(),
        kind: SurfaceKind::Opaque,
        physical_tags: vec!["layout:linear".to_owned()],
        elements: vec![ElementDescriptor {
            id: "left".to_owned(),
            name: Some("Left".to_owned()),
            kind: ElementKind::Led,
            geometry: None,
            physical_tags: vec!["shape:round".to_owned(), "position:left".to_owned()],
            capabilities: CapabilitySet::default(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        capabilities: CapabilitySet::default(),
        notes: Vec::new(),
        warnings: Vec::new(),
    });
    descriptor
}

/// Set on a re-exec of this same test binary to select
/// `stdin_sink_fixture`'s active behaviour (see its doc comment); left unset,
/// the ordinary `cargo test` run of that function is a harmless no-op rather
/// than a read that would block forever on the suite's own stdin.
const STDIN_SINK_ENV: &str = "LUMINATE_TEST_STDIN_SINK_FIXTURE";

/// Not a real test: a disposable "reads stdin until EOF and discards it"
/// process, played by re-executing this test binary
/// (`env::current_exe()`) with a filter naming only this test. `test_connection`
/// spawns it as a portable stand-in for a shell's `cat >/dev/null`, which
/// doesn't exist on Windows.
#[test]
fn stdin_sink_fixture() {
    if env::var_os(STDIN_SINK_ENV).is_none() {
        return;
    }
    let _ = io::copy(&mut io::stdin(), &mut io::sink());
}

/// Not a real test: a disposable process that exits immediately, closing its
/// stdin as a side effect. `broken_input_connection` spawns it (via
/// `env::current_exe()`) and waits for it to exit before writing, so the
/// write observes a definitely-closed read end without needing a shell to
/// synchronize with (there is no portable `exec 0<&-` equivalent).
#[test]
fn immediate_exit_fixture() {}

fn spawn_test_fixture(test_name: &str) -> Child {
    let executable = env::current_exe().expect("locate current test binary");
    Command::new(executable)
        .args([test_name, "--nocapture"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env(STDIN_SINK_ENV, "1")
        .spawn()
        .expect("spawn disposable test-fixture child")
}

fn test_connection() -> HostConnection {
    let mut child = spawn_test_fixture("stdin_sink_fixture");
    let input = child.stdin.take().expect("child stdin should be piped");
    HostConnection {
        child: Arc::new(Mutex::new(child)),
        input: Mutex::new(input),
        pending: Arc::new(Mutex::new(HashMap::new())),
        alive: Arc::new(AtomicBool::new(true)),
        exit_reporting: Arc::new(ExitReporting {
            expected: AtomicBool::new(false),
            operator_events: false,
        }),
        next_id: 1,
    }
}

fn broken_input_connection() -> HostConnection {
    let mut child = spawn_test_fixture("immediate_exit_fixture");
    let input = child.stdin.take().expect("child stdin should be piped");
    child
        .wait()
        .expect("fixture child should exit immediately, closing its stdin");
    HostConnection {
        child: Arc::new(Mutex::new(child)),
        input: Mutex::new(input),
        pending: Arc::new(Mutex::new(HashMap::new())),
        alive: Arc::new(AtomicBool::new(true)),
        exit_reporting: Arc::new(ExitReporting {
            expected: AtomicBool::new(false),
            operator_events: false,
        }),
        next_id: 1,
    }
}

#[test]
fn plugin_host_exit_reporting_is_service_only_and_suppresses_expected_teardown() {
    assert!(!ExitReporting::new(false).should_emit());

    let service = ExitReporting::new(true);
    assert!(service.should_emit());
    service.expected.store(true, Ordering::Release);
    assert!(!service.should_emit());
}

fn reply_to_request(
    pending: PendingRequests,
    id: u64,
    response: Option<Result<HostResponse, String>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if let Some(sender) = pending.lock().unwrap().remove(&id) {
                if let Some(response) = response {
                    sender.send(response).expect("request receiver should wait");
                }
                return;
            }
            assert!(Instant::now() < deadline, "request was never registered");
            thread::yield_now();
        }
    })
}

#[test]
fn private_frames_round_trip() {
    let mut configuration_cbor = Vec::new();
    ciborium::into_writer(
        &toml::toml! { endpoint = "192.0.2.10" },
        &mut configuration_cbor,
    )
    .expect("encode plugin configuration");
    let bootstrap = HostBootstrap { configuration_cbor };
    let mut encoded_bootstrap = Vec::new();
    write_frame(&mut encoded_bootstrap, &bootstrap).expect("encode host bootstrap");
    let payload = encoded_bootstrap
        .get(4..)
        .expect("encoded frame has a length prefix");
    let cbor_bootstrap: HostBootstrap = ciborium::from_reader(payload).expect("payload is CBOR");
    let configuration: toml::Table =
        ciborium::from_reader(cbor_bootstrap.configuration_cbor.as_slice())
            .expect("decode plugin configuration");
    assert_eq!(configuration["endpoint"].as_str(), Some("192.0.2.10"));
    assert!(serde_json::from_slice::<HostBootstrap>(payload).is_err());

    let decoded_bootstrap: HostBootstrap =
        read_frame(&mut encoded_bootstrap.as_slice()).expect("decode host bootstrap");
    assert_eq!(
        decoded_bootstrap.configuration_cbor,
        bootstrap.configuration_cbor
    );

    let request = HostRequest {
        id: 42,
        timeout_millis: 5_000,
        command: HostCommand::Shutdown,
    };
    let mut encoded = Vec::new();
    write_frame(&mut encoded, &request).expect("encode host request");
    let decoded: HostRequest = read_frame(&mut encoded.as_slice()).expect("decode host request");
    assert_eq!(decoded.id, 42);
    assert!(matches!(decoded.command, HostCommand::Shutdown));
}

#[test]
fn private_frames_reject_oversized_lengths_before_allocation() {
    let oversized = u32::try_from(MAX_HOST_FRAME_SIZE + 1)
        .expect("host frame limit should fit in u32")
        .to_be_bytes();
    let error = read_frame::<_, HostRequest>(&mut oversized.as_slice())
        .expect_err("oversized frame must be rejected");
    assert!(error.to_string().contains("exceeds the"));
}

#[test]
fn private_frames_reject_truncated_and_invalid_cbor_payloads() {
    let truncated = [0, 0, 0, 4, b'{'];
    let error = read_frame::<_, HostRequest>(&mut truncated.as_slice())
        .expect_err("truncated payload must fail");
    assert!(error.to_string().contains("failed to fill whole buffer"));

    let invalid_cbor = [0, 0, 0, 1, 0xff];
    let error = read_frame::<_, HostRequest>(&mut invalid_cbor.as_slice())
        .expect_err("invalid CBOR must fail");
    assert!(error.to_string().contains("CBOR decoding error"));
}

#[test]
fn private_frame_writer_reports_serialization_size_and_transport_failures() {
    struct RefusesToSerialize;
    impl Serialize for RefusesToSerialize {
        fn serialize<S>(&self, _serializer: S) -> result::Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(ser::Error::custom("deliberate serialization failure"))
        }
    }
    struct RefusesWrites;
    impl Write for RefusesWrites {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "writer closed"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let serialization = write_frame(&mut Vec::new(), &RefusesToSerialize)
        .expect_err("serialization failure must propagate");
    assert!(serialization.to_string().contains("CBOR encoding error"));

    let oversized = "x".repeat(MAX_HOST_FRAME_SIZE + 1);
    let size = write_frame(&mut Vec::new(), &oversized)
        .expect_err("oversized encoded frame must be rejected");
    assert!(size.to_string().contains("exceeds"));

    let transport = write_frame(&mut RefusesWrites, &HostMessage::TopologyChanged)
        .expect_err("writer failure must propagate");
    assert!(transport.to_string().contains("writer closed"));
}

#[test]
fn wire_metadata_rejects_unknown_bus_codes() {
    let wire = WireMetadata {
        name: "test".to_owned(),
        version: "1".to_owned(),
        priority: 0,
        recommended_reconciliation: None,
        probe_outcome: ProbeOutcome::Ready.to_abi(),
        buses: vec![999],
        vendors: Vec::new(),
        probe_hints: Vec::new(),
    };
    assert!(HostMetadata::try_from(wire).is_err());
}

#[test]
fn wire_metadata_round_trips_all_supported_fields() {
    let metadata = HostMetadata {
        name: "test-plugin".to_owned(),
        version: "1.2.3".to_owned(),
        priority: 17,
        recommended_reconciliation: Some(ReconciliationPolicy::Adopt),
        probe_outcome: ProbeOutcome::Dormant,
        buses: vec![PluginBus::Usb, PluginBus::Hid],
        vendors: vec![PluginVendorId {
            vendor: 0x1234,
            product: 0x5678,
        }],
        probe_hints: vec!["usb:1234:5678".to_owned()],
    };

    let decoded = HostMetadata::try_from(WireMetadata::from(&metadata))
        .expect("supported metadata should round trip");
    assert_eq!(decoded.name, metadata.name);
    assert_eq!(decoded.version, metadata.version);
    assert_eq!(decoded.priority, metadata.priority);
    assert_eq!(
        decoded.recommended_reconciliation,
        metadata.recommended_reconciliation
    );
    assert_eq!(decoded.buses, metadata.buses);
    assert_eq!(decoded.vendors[0].vendor, 0x1234);
    assert_eq!(decoded.vendors[0].product, 0x5678);
    assert_eq!(decoded.probe_hints, metadata.probe_hints);
}

#[test]
fn host_connection_routes_success_and_host_errors() {
    let mut connection = test_connection();
    let responder = reply_to_request(
        Arc::clone(&connection.pending),
        1,
        Some(Ok(HostResponse::Shutdown)),
    );
    let response = connection
        .call(HostCommand::Shutdown, Duration::from_secs(1))
        .expect("synthetic host should reply");
    assert!(matches!(response, HostResponse::Shutdown));
    responder.join().expect("join success responder");

    let responder = reply_to_request(
        Arc::clone(&connection.pending),
        2,
        Some(Err("plugin rejected request".to_owned())),
    );
    let error = connection
        .call(HostCommand::Topology, Duration::from_secs(1))
        .expect_err("host error should propagate");
    assert!(error.to_string().contains("plugin rejected request"));
    responder.join().expect("join error responder");

    connection.alive.store(false, Ordering::Release);
    terminate_child(&connection.child);
}

#[test]
fn host_connection_detects_reply_disconnect() {
    let mut connection = test_connection();
    let responder = reply_to_request(Arc::clone(&connection.pending), 1, None);
    let error = connection
        .call(HostCommand::Topology, Duration::from_secs(1))
        .expect_err("dropped reply channel should fail");
    assert!(
        error
            .to_string()
            .contains("exited while processing request")
    );
    assert!(!connection.is_alive());
    responder.join().expect("join disconnect responder");
    terminate_child(&connection.child);
}

#[test]
fn host_connection_timeout_terminates_child_and_clears_request() {
    let mut connection = test_connection();
    let error = connection
        .call(HostCommand::Topology, Duration::from_millis(10))
        .expect_err("missing response should time out");
    assert!(error.to_string().contains("request exceeded"));
    assert!(!connection.is_alive());
    assert!(connection.pending.lock().unwrap().is_empty());
    assert!(
        connection
            .child
            .lock()
            .unwrap()
            .try_wait()
            .expect("query child status")
            .is_some()
    );
}

#[test]
fn host_connection_write_failure_marks_transport_dead() {
    let mut connection = broken_input_connection();

    let error = connection
        .call(HostCommand::Topology, Duration::from_secs(1))
        .expect_err("writing to a reaped child must fail");
    assert!(error.to_string().contains("sending request to plugin host"));
    assert!(!connection.is_alive());
    assert!(connection.pending.lock().unwrap().is_empty());
}

#[test]
fn host_connection_shutdown_requests_exit_and_reaps_child() {
    let mut connection = test_connection();
    let child = Arc::clone(&connection.child);
    let responder = reply_to_request(
        Arc::clone(&connection.pending),
        1,
        Some(Ok(HostResponse::Shutdown)),
    );

    connection.shutdown();

    responder.join().expect("join shutdown responder");
    assert!(
        child
            .lock()
            .unwrap()
            .try_wait()
            .expect("query child status")
            .is_some()
    );
}

#[test]
fn hosted_plugin_delegates_calls_and_reaps_on_drop() {
    let connection = test_connection();
    let pending = Arc::clone(&connection.pending);
    let alive = Arc::clone(&connection.alive);
    let child = Arc::clone(&connection.child);
    let (notifications, _receiver) = unbounded_channel();
    let plugin = HostedPlugin {
        path: PathBuf::from("unused-test-plugin.so"),
        expected_metadata: HostMetadata {
            name: "test-plugin".to_owned(),
            version: "1".to_owned(),
            priority: 0,
            recommended_reconciliation: None,
            probe_outcome: ProbeOutcome::Ready,
            buses: Vec::new(),
            vendors: Vec::new(),
            probe_hints: Vec::new(),
        },
        connection: Mutex::new(connection),
        notifications,
        max_log_level: PluginLogLevel::Info,
        configuration_cbor: vec![0xa0],
        connection_epoch: AtomicU64::new(0),
        operator_events: false,
    };
    let responder = reply_to_request(pending, 1, Some(Ok(HostResponse::Shutdown)));

    assert!(matches!(
        plugin.call(HostCommand::Shutdown).expect("delegated call"),
        HostResponse::Shutdown
    ));
    responder.join().expect("join delegated responder");
    alive.store(false, Ordering::Release);
    drop(plugin);
    assert!(
        child
            .lock()
            .unwrap()
            .try_wait()
            .expect("query child status")
            .is_some()
    );
}

#[test]
fn hosted_plugin_reports_early_child_exit_and_failed_restart() {
    let (notifications, _receiver) = unbounded_channel();
    let Err(spawn_error) = HostedPlugin::spawn_with_configuration(
        Path::new("/missing/test-plugin.so"),
        notifications.clone(),
        PluginLogLevel::Error,
        vec![0xa0],
        false,
    ) else {
        panic!("test harness child cannot enter plugin-host mode");
    };
    assert!(spawn_error.to_string().contains("plugin host"));

    let connection = test_connection();
    connection.alive.store(false, Ordering::Release);
    terminate_child(&connection.child);
    let plugin = HostedPlugin {
        path: PathBuf::from("/missing/test-plugin.so"),
        expected_metadata: HostMetadata {
            name: "test-plugin".to_owned(),
            version: "1".to_owned(),
            priority: 0,
            recommended_reconciliation: None,
            probe_outcome: ProbeOutcome::Ready,
            buses: Vec::new(),
            vendors: Vec::new(),
            probe_hints: Vec::new(),
        },
        connection: Mutex::new(connection),
        notifications,
        max_log_level: PluginLogLevel::Error,
        configuration_cbor: vec![0xa0],
        connection_epoch: AtomicU64::new(0),
        operator_events: false,
    };
    let restart_error = plugin
        .call(HostCommand::Topology)
        .expect_err("dead host restart should report child initialization failure");
    assert!(restart_error.to_string().contains("failed to restart"));
}

/// `force_reload` must tear the current host down even when it's still
/// healthy, unlike the lazy respawn-after-crash path in `call`, which only
/// replaces a connection that's already dead.
#[test]
fn force_reload_shuts_down_a_healthy_connection_before_attempting_the_replacement() {
    let connection = test_connection();
    let child = Arc::clone(&connection.child);
    assert!(
        connection.is_alive(),
        "test fixture connection should start alive"
    );
    let responder = reply_to_request(
        Arc::clone(&connection.pending),
        1,
        Some(Ok(HostResponse::Shutdown)),
    );
    let plugin = hosted_plugin_with_connection(connection);

    // The configured path doesn't exist, so the replacement can never
    // actually start; this test only cares that the previous, healthy
    // connection was torn down regardless of that failure.
    let error = plugin
        .force_reload()
        .expect_err("a bogus plugin path cannot produce a working replacement host");
    assert!(error.to_string().contains("failed to restart"));

    responder.join().expect("join shutdown responder");
    assert!(
        child
            .lock()
            .unwrap()
            .try_wait()
            .expect("query child status")
            .is_some(),
        "the previous host process must have been terminated even though it was healthy"
    );
}

fn hosted_plugin_with_connection(connection: HostConnection) -> HostedPlugin {
    let (notifications, _receiver) = unbounded_channel();
    HostedPlugin {
        path: PathBuf::from("unused-test-plugin.so"),
        expected_metadata: HostMetadata {
            name: "test-plugin".to_owned(),
            version: "1".to_owned(),
            priority: 0,
            recommended_reconciliation: None,
            probe_outcome: ProbeOutcome::Ready,
            buses: Vec::new(),
            vendors: Vec::new(),
            probe_hints: Vec::new(),
        },
        connection: Mutex::new(connection),
        notifications,
        max_log_level: PluginLogLevel::Info,
        configuration_cbor: vec![0xa0],
        connection_epoch: AtomicU64::new(0),
        operator_events: false,
    }
}

/// Every typed accessor (`topology`, `apply`, ...) sends its own
/// [`HostCommand`] and then rejects any [`HostResponse`] shape but its own.
/// A conforming plugin host never mismatches request and response kinds, but
/// a misbehaving or compromised one might, so each accessor's "wrong
/// response" arm needs its own coverage rather than trusting the others.
#[test]
fn hosted_plugin_accessors_reject_a_response_of_the_wrong_shape() {
    let connection = test_connection();
    let pending = Arc::clone(&connection.pending);
    let plugin = hosted_plugin_with_connection(connection);

    let responder = reply_to_request(Arc::clone(&pending), 1, Some(Ok(HostResponse::Shutdown)));
    let error = plugin
        .topology()
        .expect_err("a Shutdown response must not satisfy a Topology request");
    assert!(error.to_string().contains("wrong topology response"));
    responder.join().expect("join topology responder");

    let responder = reply_to_request(Arc::clone(&pending), 2, Some(Ok(HostResponse::Shutdown)));
    let error = plugin
        .apply(sample_update())
        .expect_err("a Shutdown response must not satisfy an Apply request");
    assert!(error.to_string().contains("wrong update response"));
    responder.join().expect("join apply responder");

    let responder = reply_to_request(Arc::clone(&pending), 3, Some(Ok(HostResponse::Shutdown)));
    let error = plugin
        .apply_batch(vec![sample_update()])
        .expect_err("a Shutdown response must not satisfy an ApplyBatch request");
    assert!(error.to_string().contains("wrong batch response"));
    responder.join().expect("join apply_batch responder");

    let responder = reply_to_request(Arc::clone(&pending), 4, Some(Ok(HostResponse::Shutdown)));
    let error = plugin
        .read_state(PluginReadRequest {
            targets: Vec::new(),
        })
        .expect_err("a Shutdown response must not satisfy a ReadState request");
    assert!(error.to_string().contains("wrong state response"));
    responder.join().expect("join read_state responder");

    let responder = reply_to_request(Arc::clone(&pending), 5, Some(Ok(HostResponse::Shutdown)));
    let error = plugin
        .upload_frame(sample_frame_upload())
        .expect_err("a Shutdown response must not satisfy an UploadFrame request");
    assert!(error.to_string().contains("wrong frame response"));
    responder.join().expect("join upload_frame responder");
}

#[test]
fn fail_pending_notifies_every_waiter() {
    let pending = Mutex::new(HashMap::new());
    let (first_tx, first_rx) = mpsc::sync_channel(1);
    let (second_tx, second_rx) = mpsc::sync_channel(1);
    pending.lock().expect("pending lock").insert(1, first_tx);
    pending.lock().expect("pending lock").insert(2, second_tx);

    fail_pending(&pending, "transport stopped");

    for receiver in [first_rx, second_rx] {
        let error = receiver
            .recv()
            .expect("waiter should be notified")
            .expect_err("transport failure should be an error");
        assert_eq!(error, "transport stopped");
    }
    assert!(pending.lock().expect("pending lock").is_empty());
}

#[test]
fn apply_results_preserve_structured_outcomes() {
    let cases = [
        (PluginApplyResult::applied(), "applied"),
        (PluginApplyResult::unsupported("nope"), "unsupported"),
        (PluginApplyResult::invalid_argument("bad input"), "invalid"),
        (PluginApplyResult::io("device gone"), "io"),
        (PluginApplyResult::unavailable("offline"), "unavailable"),
        (PluginApplyResult::rate_limited("slow down"), "rate-limited"),
        (PluginApplyResult::internal("broken"), "internal"),
    ];

    for (result, expected) in cases {
        let outcome = decode_apply_result(&result).expect("valid result should decode");
        match (expected, outcome) {
            ("applied", ApplyOutcome::Applied)
            | ("unsupported", ApplyOutcome::Unsupported(_))
            | ("invalid", ApplyOutcome::InvalidArgument(_))
            | ("io", ApplyOutcome::Io(_))
            | ("unavailable", ApplyOutcome::Unavailable(_))
            | ("rate-limited", ApplyOutcome::RateLimited { .. })
            | ("internal", ApplyOutcome::Internal(_)) => {}
            _ => panic!("result mapped to the wrong structured outcome"),
        }
    }

    let rate_limited =
        PluginApplyResult::rate_limited_after("slow down", Duration::from_millis(375));
    assert!(matches!(
        decode_apply_result(&rate_limited).expect("rate limit should decode"),
        ApplyOutcome::RateLimited {
            retry_after_ms: Some(375),
            ..
        }
    ));

    let mut malformed = PluginApplyResult::applied();
    malformed.code = u8::MAX;
    let error = decode_apply_result(&malformed).expect_err("unknown status must fail");
    assert!(error.to_string().contains("malformed apply result"));
}

#[test]
fn native_string_and_slice_readers_validate_boundary_inputs() {
    let valid = CString::new("plugin").expect("literal has no NUL");
    assert_eq!(
        read_c_string(valid.as_ptr(), "name").expect("valid C string"),
        "plugin"
    );
    assert!(read_c_string(ptr::null(), "name").is_err());
    assert_eq!(read_c_str_lossy(ptr::null()), "<null>");

    let invalid = [u8::MAX, 0];
    assert!(read_c_string(invalid.as_ptr().cast(), "name").is_err());
    assert_eq!(read_c_str_lossy(invalid.as_ptr().cast()), "�");

    let vendors = [
        PluginVendorId {
            vendor: 1,
            product: 2,
        },
        PluginVendorId {
            vendor: 3,
            product: 4,
        },
    ];
    let copied = read_slice_copy(vendors.as_ptr(), vendors.len());
    assert_eq!(copied.len(), 2);
    assert_eq!(copied[1].product, 4);
    assert!(read_slice_copy::<PluginVendorId>(ptr::null(), 2).is_empty());
    assert!(read_slice_copy(vendors.as_ptr(), 0).is_empty());
}

#[test]
fn descriptor_metadata_and_probe_hints_validate_foreign_fields() {
    let buses = [PluginBus::Usb, PluginBus::Hid];
    let vendors = [PluginVendorId {
        vendor: 0x1234,
        product: 0x5678,
    }];
    let hint_value = c"usb:1234:5678";
    let hints = [PluginProbeHint {
        kind: ProbeHintKind::UsbVidPid,
        value: hint_value.as_ptr(),
    }];
    let mut descriptor = PluginDescriptor {
        name: c"metadata-plugin".as_ptr(),
        version: c"2.0".as_ptr(),
        priority: 9,
        recommended_reconciliation: luminate_plugin_api::reconciliation_policy_to_abi(Some(
            ReconciliationPolicy::Restore,
        )),
        buses: buses.as_ptr(),
        bus_count: buses.len(),
        vendors: vendors.as_ptr(),
        vendor_count: vendors.len(),
        probe_hints: hints.as_ptr(),
        probe_hint_count: hints.len(),
        settings: ptr::null(),
        setting_count: 0,
        setup_workflows: ptr::null(),
        setup_workflow_count: 0,
        init: noop_init,
        probe: None,
        start: None,
        rescan: None,
        topology_cbor: None,
        apply_update_cbor: None,
        apply_batch_cbor: None,
        read_state_cbor: None,
        frame_upload_cbor: None,
        shm_stream_begin: None,
        shm_frame_apply: None,
        shm_stream_end: None,
        setup_cbor: None,
    };

    let metadata = descriptor_to_metadata(&descriptor).expect("valid descriptor metadata");
    assert_eq!(metadata.name, "metadata-plugin");
    assert_eq!(metadata.version, "2.0");
    assert_eq!(metadata.priority, 9);
    assert_eq!(metadata.buses, buses);
    assert_eq!(metadata.vendors, vendors);
    assert_eq!(metadata.probe_hints, vec!["usb:1234:5678"]);

    descriptor.recommended_reconciliation = u8::MAX;
    assert!(descriptor_to_metadata(&descriptor).is_err());

    let null_hint = [RawProbeHint {
        kind: ProbeHintKind::None.to_abi(),
        value: ptr::null(),
    }];
    assert_eq!(
        read_probe_hints(null_hint.as_ptr().cast(), null_hint.len())
            .expect("null hint value is represented as empty"),
        vec![String::new()]
    );
    let invalid_utf8 = [u8::MAX, 0];
    let invalid_hint = [RawProbeHint {
        kind: ProbeHintKind::DmiMatch.to_abi(),
        value: invalid_utf8.as_ptr().cast(),
    }];
    assert!(read_probe_hints(invalid_hint.as_ptr().cast(), invalid_hint.len()).is_err());
    assert!(read_probe_hints(ptr::null(), 3).unwrap().is_empty());

    let _level = current_max_plugin_log_level();
}

#[test]
fn native_plugin_load_reports_missing_shared_library() {
    let Err(error) = NativePlugin::load_with_configuration(
        Path::new("/definitely/missing/luminate-plugin.so"),
        PluginLogLevel::Error,
        &[0xa0],
    ) else {
        panic!("missing shared library must fail");
    };
    assert!(error.to_string().contains("failed to open"));
}

#[test]
fn host_message_reader_routes_ready_responses_notifications_and_logs() {
    let connection = test_connection();
    let child = Arc::clone(&connection.child);
    let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
    let (response_tx, response_rx) = mpsc::sync_channel(1);
    pending.lock().expect("pending lock").insert(7, response_tx);
    let alive = Arc::new(AtomicBool::new(true));
    let exit_reporting = ExitReporting {
        expected: AtomicBool::new(false),
        operator_events: false,
    };
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (notifications_tx, mut notifications_rx) = unbounded_channel();
    let metadata = WireMetadata {
        name: "reader-plugin".to_owned(),
        version: "1".to_owned(),
        priority: 0,
        recommended_reconciliation: None,
        probe_outcome: ProbeOutcome::Ready.to_abi(),
        buses: Vec::new(),
        vendors: Vec::new(),
        probe_hints: Vec::new(),
    };
    let mut encoded = Vec::new();
    write_frame(
        &mut encoded,
        &HostMessage::Ready(Ok(HostReady {
            metadata,
            descriptors: Vec::new(),
        })),
    )
    .expect("encode ready");
    write_frame(
        &mut encoded,
        &HostMessage::Response {
            id: 7,
            result: Ok(HostResponse::Shutdown),
        },
    )
    .expect("encode response");
    write_frame(
        &mut encoded,
        &HostMessage::Response {
            id: 999,
            result: Err("late response".to_owned()),
        },
    )
    .expect("encode unmatched response");
    write_frame(&mut encoded, &HostMessage::TopologyChanged).expect("encode notification");
    for level in 0..=5 {
        write_frame(
            &mut encoded,
            &HostMessage::Log {
                plugin: "reader-plugin".to_owned(),
                level,
                message: format!("message at {level}"),
            },
        )
        .expect("encode log");
    }

    read_host_messages(
        encoded.as_slice(),
        ready_tx,
        &pending,
        &alive,
        &exit_reporting,
        &child,
        &notifications_tx,
    );

    let ready = ready_rx
        .recv()
        .expect("ready sender should report")
        .expect("ready should succeed");
    assert_eq!(ready.metadata.name, "reader-plugin");
    assert!(matches!(
        response_rx.recv().expect("response should arrive"),
        Ok(HostResponse::Shutdown)
    ));
    assert_eq!(
        notifications_rx.try_recv().expect("topology notification"),
        TopologyNotification::PluginObserved("reader-plugin".to_owned())
    );
    assert_eq!(
        notifications_rx.try_recv().expect("exit notification"),
        TopologyNotification::PluginObserved("reader-plugin".to_owned())
    );
    assert!(!alive.load(Ordering::Acquire));
    terminate_child(&child);
}

#[test]
fn host_message_reader_fails_ready_and_pending_on_transport_end() {
    let connection = test_connection();
    let child = Arc::clone(&connection.child);
    let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
    let (response_tx, response_rx) = mpsc::sync_channel(1);
    pending.lock().expect("pending lock").insert(1, response_tx);
    let alive = Arc::new(AtomicBool::new(true));
    let exit_reporting = ExitReporting {
        expected: AtomicBool::new(false),
        operator_events: false,
    };
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (notifications_tx, mut notifications_rx) = unbounded_channel();

    read_host_messages(
        io::empty(),
        ready_tx,
        &pending,
        &alive,
        &exit_reporting,
        &child,
        &notifications_tx,
    );

    let ready_error = ready_rx
        .recv()
        .expect("ready failure should arrive")
        .expect_err("EOF before ready must fail");
    assert!(ready_error.contains("transport ended"));
    let response_error = response_rx
        .recv()
        .expect("pending failure should arrive")
        .expect_err("pending request must fail");
    assert!(response_error.contains("transport ended"));
    assert!(!alive.load(Ordering::Acquire));
    assert!(notifications_rx.try_recv().is_err());
    terminate_child(&child);
}

#[test]
fn rescan_dispatch_forwards_the_reason_and_tolerates_an_absent_callback() {
    let _fixture = lock_fixture(&ABI_FIXTURE_LOCK);

    // A plugin without the callback must be a silent no-op, not an error:
    // its `topology_cbor` already enumerates afresh, so the daemon's re-pull
    // is the whole rescan.
    let mut absent = native_plugin(None, None, None, None);
    absent.rescan = None;
    absent.rescan(RescanReason::Resume);

    RESCAN_REASONS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .clear();

    let mut present = native_plugin(None, None, None, None);
    present.rescan = Some(recording_rescan_callback);
    for reason in [
        RescanReason::Resume,
        RescanReason::DeviceChange,
        RescanReason::Operator,
    ] {
        present.rescan(reason);
    }

    // Each reason must reach the plugin as its own ABI byte: a plugin is
    // entitled to be proportionate about a resume versus an operator poke.
    assert_eq!(
        *RESCAN_REASONS.get().expect("recorder").lock().unwrap(),
        vec![
            RescanReason::Resume.to_abi(),
            RescanReason::DeviceChange.to_abi(),
            RescanReason::Operator.to_abi(),
        ]
    );
}

#[test]
fn native_topology_adapter_handles_absent_null_invalid_and_valid_snapshots() {
    let _fixture = lock_fixture(&ABI_FIXTURE_LOCK);
    let absent = native_plugin(None, None, None, None);
    assert!(absent.pull_topology().expect("absent callback").is_empty());

    let null = native_plugin(Some(null_topology_callback), None, None, None);
    assert!(null.pull_topology().expect("null topology").is_empty());

    let invalid_utf8 = native_plugin(Some(invalid_utf8_topology_callback), None, None, None);
    assert!(invalid_utf8.pull_topology().is_err());

    let callback = native_plugin(Some(topology_callback), None, None, None);
    *TOPOLOGY_CBOR
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap() = vec![0xff];
    assert!(callback.pull_topology().is_err());

    set_topology(&[
        sample_descriptor("duplicate"),
        sample_descriptor("duplicate"),
    ]);
    let error = callback
        .pull_topology()
        .expect_err("duplicate IDs must fail normalization");
    assert!(error.to_string().contains("invalid topology"));

    set_topology(&[tagged_descriptor("valid")]);
    let descriptors = callback.pull_topology().expect("valid topology");
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].id, "valid");
    assert_eq!(
        descriptors[0].surfaces[0].elements[0].physical_tags,
        ["shape:round", "position:left"]
    );
    assert!(matches!(
        callback
            .handle(HostCommand::Topology, test_context())
            .expect("handle topology"),
        HostResponse::Topology(topology) if topology.len() == 1
    ));

    let valid = tagged_descriptor("valid");
    let mut invalid = tagged_descriptor("invalid");
    invalid.surfaces[0].elements[0].physical_tags = vec![" shape:round".to_owned()];
    set_topology(&[valid, invalid]);
    assert!(
        callback.pull_topology().is_err(),
        "one invalid element must reject the complete candidate topology"
    );
}

#[test]
fn topology_contract_rejects_capabilities_without_required_callbacks() {
    let mut readable = sample_descriptor("readable");
    readable.capabilities.state_readback = StateReadbackCapability::Readable {
        facets: Vec::new(),
        read_disturbs_output: false,
        notifies_external_changes: false,
    };
    let error = validate_topology_contract(
        &[readable],
        PluginCallbacks {
            apply: true,
            read_state: false,
            frame_upload: false,
            shm_frame: false,
        },
    )
    .expect_err("readback without a callback must fail");
    assert!(error.to_string().contains("no state-readback callback"));

    let mut mutable = sample_descriptor("mutable");
    mutable.capabilities.colour = vec![capability::ColourCapability::rgb8()];
    let error = validate_topology_contract(
        &[mutable],
        PluginCallbacks {
            apply: false,
            read_state: false,
            frame_upload: false,
            shm_frame: false,
        },
    )
    .expect_err("mutable topology without a callback must fail");
    assert!(error.to_string().contains("no update callback"));
}

fn sample_frame_upload_capability() -> capability::FrameUploadCapability {
    capability::FrameUploadCapability {
        scope: capability::CapabilityScope::Device,
        update_mode: capability::FrameUpdateMode::FullFrameOnly,
        max_rate_hz: None,
        atomic: false,
        buffering: capability::BufferingMode::Immediate,
        shm: None,
    }
}

#[test]
fn topology_contract_rejects_frame_upload_without_a_callback() {
    let mut framed = sample_descriptor("framed");
    framed.capabilities.frame_upload = Some(sample_frame_upload_capability());

    let error = validate_topology_contract(
        &[framed],
        PluginCallbacks {
            apply: true,
            read_state: false,
            frame_upload: false,
            shm_frame: false,
        },
    )
    .expect_err("frame-upload capability without a callback must fail");
    assert!(error.to_string().contains("no frame-upload callback"));
}

#[test]
fn topology_contract_accepts_frame_upload_with_a_callback() {
    let mut framed = sample_descriptor("framed");
    framed.capabilities.frame_upload = Some(sample_frame_upload_capability());

    validate_topology_contract(
        &[framed],
        PluginCallbacks {
            apply: true,
            read_state: false,
            frame_upload: true,
            shm_frame: false,
        },
    )
    .expect("frame-upload capability with a callback must be accepted");
}

fn shm_begin_request(target: &str, generation: u32) -> HostCommand {
    HostCommand::BeginShmStream(BeginShmStreamRequest {
        target: PluginTarget::Device {
            device: target.to_owned(),
        },
        generation,
        pixel_format: ShmPixelFormat::Rgb8.to_abi(),
        shape: capability::ShmFrameShape::Linear { pixel_count: 4 },
    })
}

#[test]
fn handle_rejects_begin_shm_stream_when_the_plugin_has_no_shm_callbacks() {
    let plugin = native_plugin(None, None, None, None);
    let response = plugin
        .handle(shm_begin_request("no-shm-device", 1), test_context())
        .expect("handle should not error");
    assert!(matches!(
        response,
        HostResponse::ShmStream(ShmStreamOutcome::Unsupported(_))
    ));
}

#[test]
fn handle_dispatches_begin_and_end_shm_stream_to_the_shm_runtime() {
    let _fixture = lock_fixture(&ABI_FIXTURE_LOCK);
    SHM_ENDED.lock().unwrap().clear();
    let plugin = native_plugin_with_shm();
    let target = "shm-dispatch-device";

    let begin_response = plugin
        .handle(shm_begin_request(target, 3), test_context())
        .expect("handle should not error");
    assert!(matches!(
        begin_response,
        HostResponse::ShmStream(ShmStreamOutcome::Ready { .. })
    ));

    let end_response = plugin
        .handle(
            HostCommand::EndShmStream {
                target: PluginTarget::Device {
                    device: target.to_owned(),
                },
                generation: 3,
            },
            test_context(),
        )
        .expect("handle should not error");
    assert!(matches!(
        end_response,
        HostResponse::ShmStream(ShmStreamOutcome::Acknowledged)
    ));
    assert_eq!(SHM_ENDED.lock().unwrap().as_slice(), &[(1, 3)]);
}

#[test]
fn handle_shutdown_ends_every_active_shm_stream() {
    let _fixture = lock_fixture(&ABI_FIXTURE_LOCK);
    SHM_ENDED.lock().unwrap().clear();
    let plugin = native_plugin_with_shm();

    plugin
        .handle(shm_begin_request("shutdown-device", 9), test_context())
        .expect("begin should not error");
    assert!(matches!(
        plugin
            .handle(HostCommand::Shutdown, test_context())
            .expect("shutdown should not error"),
        HostResponse::Shutdown
    ));
    assert_eq!(SHM_ENDED.lock().unwrap().as_slice(), &[(1, 9)]);
}

#[test]
fn topology_contract_rejects_malformed_effects() {
    let mut malformed = sample_descriptor("malformed");
    malformed.capabilities.hardware_effects = Some(HardwareEffectsCapability {
        effects: vec![capability::HardwareEffectDescriptor {
            id: capability::HardwareEffectId::new("broken"),
            name: "Broken".to_owned(),
            parameters: vec![EffectParameter::Colour {
                minimum_colours: 2,
                maximum_colours: 1,
            }],
        }],
        scope: capability::CapabilityScope::Device,
        concurrent_with_streaming: false,
    });
    let error = validate_topology_contract(
        &[malformed],
        PluginCallbacks {
            apply: true,
            read_state: false,
            frame_upload: false,
            shm_frame: false,
        },
    )
    .expect_err("malformed effect schema must fail");
    assert!(error.to_string().contains("inverted colour-count range"));
}

#[test]
fn native_apply_adapter_covers_all_outcomes_and_envelope_failures() {
    let _fixture = lock_fixture(&ABI_FIXTURE_LOCK);
    let plugin = native_plugin(None, Some(apply_callback), None, None);
    let update = sample_update();
    for mode in 0..=4 {
        APPLY_MODE.store(mode, Ordering::Release);
        let outcome = plugin
            .apply(test_context(), &update)
            .expect("structured outcome");
        assert!(matches!(
            (mode, outcome),
            (0, ApplyOutcome::Applied)
                | (1, ApplyOutcome::Unsupported(_))
                | (2, ApplyOutcome::InvalidArgument(_))
                | (3, ApplyOutcome::Io(_))
                | (4, ApplyOutcome::Internal(_))
        ));
    }

    APPLY_MODE.store(5, Ordering::Release);
    assert!(plugin.apply(test_context(), &update).is_err());
    APPLY_MODE.store(6, Ordering::Release);
    assert!(matches!(
        plugin
            .apply(test_context(), &update)
            .expect("false callback is structured"),
        ApplyOutcome::Internal(message) if message.contains("apply envelope")
    ));

    let unsupported = native_plugin(None, None, None, None);
    assert!(matches!(
        unsupported
            .apply(test_context(), &update)
            .expect("missing callback is structured"),
        ApplyOutcome::Unsupported(message) if message.contains("no mutation callback")
    ));

    APPLY_MODE.store(0, Ordering::Release);
    assert!(matches!(
        plugin
            .handle(HostCommand::Apply(update), test_context())
            .expect("handle apply"),
        HostResponse::Apply(ApplyOutcome::Applied)
    ));
    assert!(matches!(
        plugin
            .handle(HostCommand::Shutdown, test_context())
            .expect("handle shutdown"),
        HostResponse::Shutdown
    ));
}

#[test]
fn native_batch_adapter_covers_callback_fallback_and_malformed_results() {
    let _fixture = lock_fixture(&ABI_FIXTURE_LOCK);
    let updates = vec![sample_update(), sample_update()];
    let plugin = native_plugin(None, Some(apply_callback), Some(batch_callback), None);
    BATCH_MODE.store(0, Ordering::Release);
    let outcomes = plugin
        .apply_batch(test_context(), &updates)
        .expect("valid batch outcomes");
    assert!(matches!(outcomes[0], ApplyOutcome::Applied));
    assert!(matches!(outcomes[1], ApplyOutcome::Unsupported(_)));

    BATCH_MODE.store(1, Ordering::Release);
    let outcomes = plugin
        .apply_batch(test_context(), &updates)
        .expect("false batch callback is structured");
    assert!(
        outcomes
            .iter()
            .all(|outcome| matches!(outcome, ApplyOutcome::Internal(_)))
    );

    BATCH_MODE.store(2, Ordering::Release);
    assert!(plugin.apply_batch(test_context(), &updates).is_err());

    let fallback = native_plugin(None, Some(apply_callback), None, None);
    APPLY_MODE.store(1, Ordering::Release);
    let outcomes = fallback
        .apply_batch(test_context(), &updates)
        .expect("missing batch callback falls back to singles");
    assert!(
        outcomes
            .iter()
            .all(|outcome| matches!(outcome, ApplyOutcome::Unsupported(_)))
    );

    BATCH_MODE.store(0, Ordering::Release);
    assert!(matches!(
        plugin
            .handle(HostCommand::ApplyBatch(updates), test_context())
            .expect("handle batch"),
        HostResponse::Batch(outcomes) if outcomes.len() == 2
    ));
}

#[test]
fn native_state_adapter_bounds_and_decodes_plugin_output() {
    let _fixture = lock_fixture(&ABI_FIXTURE_LOCK);
    let request = PluginReadRequest {
        targets: Vec::new(),
    };
    let missing = native_plugin(None, None, None, None);
    assert!(missing.read_state(test_context(), &request).is_err());

    let plugin = native_plugin(None, None, None, Some(read_state_callback));
    READ_MODE.store(0, Ordering::Release);
    let snapshot = plugin
        .read_state(test_context(), &request)
        .expect("valid empty snapshot");
    assert!(snapshot.observations.is_empty());
    assert!(snapshot.errors.is_empty());
    assert!(matches!(
        plugin
            .handle(HostCommand::ReadState(request.clone()), test_context())
            .expect("handle read state"),
        HostResponse::State(snapshot) if snapshot.observations.is_empty()
    ));

    READ_MODE.store(1, Ordering::Release);
    assert!(
        plugin
            .read_state(test_context(), &request)
            .expect_err("encoding failure")
            .to_string()
            .contains("could not encode")
    );
    READ_MODE.store(2, Ordering::Release);
    assert!(
        plugin
            .read_state(test_context(), &request)
            .expect_err("oversized snapshot")
            .to_string()
            .contains("requires")
    );
    READ_MODE.store(3, Ordering::Release);
    assert!(
        plugin
            .read_state(test_context(), &request)
            .expect_err("malformed JSON")
            .to_string()
            .contains("decoding plugin state snapshot")
    );
}

#[test]
fn native_host_rejects_oversized_batch_and_read_work() {
    let plugin = native_plugin(
        None,
        Some(apply_callback),
        Some(batch_callback),
        Some(read_state_callback),
    );
    let batch = vec![sample_update(); MAX_PLUGIN_BATCH_UPDATES + 1];
    assert!(
        plugin
            .handle(HostCommand::ApplyBatch(batch), test_context())
            .expect_err("oversized batch must be rejected")
            .to_string()
            .contains("batch contains")
    );

    let target = luminate_plugin_api::PluginReadTarget {
        target: PluginTarget::Device {
            device: "demo".to_owned(),
        },
        facets: Vec::new(),
    };
    let request = PluginReadRequest {
        targets: vec![target; MAX_PLUGIN_READ_TARGETS + 1],
    };
    assert!(
        plugin
            .handle(HostCommand::ReadState(request), test_context())
            .expect_err("oversized read must be rejected")
            .to_string()
            .contains("read contains")
    );
}

#[test]
fn native_frame_adapter_covers_missing_callback_outcomes_and_envelope_failures() {
    let _fixture = lock_fixture(&ABI_FIXTURE_LOCK);
    let frame_upload = sample_frame_upload();

    let missing = native_plugin_with_frame(None);
    assert!(matches!(
        missing
            .upload_frame(test_context(), &frame_upload)
            .expect("missing callback is structured"),
        ApplyOutcome::Unsupported(message) if message.contains("no frame-upload callback")
    ));

    let plugin = native_plugin_with_frame(Some(frame_callback));
    FRAME_MODE.store(0, Ordering::Release);
    assert!(matches!(
        plugin
            .upload_frame(test_context(), &frame_upload)
            .expect("applied outcome"),
        ApplyOutcome::Applied
    ));

    FRAME_MODE.store(1, Ordering::Release);
    assert!(matches!(
        plugin
            .upload_frame(test_context(), &frame_upload)
            .expect("io outcome"),
        ApplyOutcome::Io(_)
    ));

    FRAME_MODE.store(2, Ordering::Release);
    assert!(matches!(
        plugin
            .upload_frame(test_context(), &frame_upload)
            .expect("false callback is structured"),
        ApplyOutcome::Internal(message) if message.contains("frame envelope")
    ));

    FRAME_MODE.store(0, Ordering::Release);
    assert!(matches!(
        plugin
            .handle(HostCommand::UploadFrame(frame_upload), test_context())
            .expect("handle upload frame"),
        HostResponse::Frame(ApplyOutcome::Applied)
    ));
}
