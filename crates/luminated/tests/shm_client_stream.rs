// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Cross-process integration test for the client → daemon shared-memory
//! frame-streaming fast path.
//!
//! The test runs a real `luminated` process with the real
//! `luminate-plugin-demo-display` plugin and drives it using a
//! hand-written protocol client in this process. Like
//! `plugin_conformance.rs`, it keeps local copies of the wire types rather
//! than reaching into another crate's private internals across a process
//! boundary.
//!
//! In the test environment the client and daemon naturally run as the same
//! OS user, so the real authorization gate is exercised and the complete
//! shared-memory path runs end-to-end rather than through a mock daemon.
//!
//! ## Verification
//!
//! Success is verified through the demo plugin's `shm_stream_end` log.
//! That log is emitted only after the plugin's private `frames_applied`
//! counter has been incremented through the daemon → plugin-host shared-memory
//! callbacks. Observing the log therefore proves that every frame traversed
//! the complete relay (client → daemon shared memory followed by
//! daemon → plugin-host shared memory), not merely the client → daemon hop.
//!
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::tests_outside_test_module,
    unsafe_code,
    reason = "Integration tests fail loudly on setup errors; the narrow libc::kill call lets the daemon flush coverage and other shutdown state."
)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, thread};

use iceoryx2::port::notifier::Notifier;
use iceoryx2::port::publisher::Publisher;
use iceoryx2::prelude::*;
use tokio::time;

use luminate_core::colour::Colour;
use luminate_core::device::DeviceId;
use luminate_core::rgb::Rgb;
use luminate_core::shm_frame::{
    SHM_CLIENT_FRAME_HEADER_LEN, SHM_CLIENT_FRAME_HEADER_VERSION, ShmClientFrameHeader,
    ShmPixelFormat,
};
use luminate_core::target::TargetId;
use luminate_platform::dynamic_library_candidates;
use luminate_platform::test_support::TestDir;
use luminate_platform::transport::{Address, Connection, connect};
use luminate_protocol::PROTOCOL_ABI_VERSION;
use luminate_protocol::framing;
use luminate_protocol::{
    Authentication, AuthenticationRequest, AuthenticationResponse, ClientHello, Compatibility,
    DaemonHello, Request, RequestMessage, ResponseMessage, ResponseStatus,
};

const WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Matches `luminate-plugin-demo-display`'s `DEVICE_ID`/`WIDTH`/`HEIGHT`.
/// Hardcoded rather than depending on that crate: this test only needs the
/// wire-visible device id and matrix size, both part of that plugin's
/// documented, stable demo-device contract.
const DEVICE_ID: &str = "demo-led-display";
const PIXEL_COUNT: usize = 16 * 16;
const FRAME_COUNT: u64 = 5;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "spawns workspace binaries and negotiates a real shared-memory frame stream"]
async fn client_published_shm_stream_delivers_frames_to_a_real_daemon() {
    let workspace = workspace_root();
    build_demo_stack(&workspace);

    let temp = temp_dir();
    let socket_path = temp.join("luminated.sock");
    let state_path = temp.join("state.json");
    let log_path = temp.join("luminated.log");
    let config_path = temp.join("luminated.toml");
    let plugin_path = workspace
        .join(format!(
            "target/debug/{}",
            dynamic_library_candidates("luminate_plugin_demo_display")[0]
        ))
        .display()
        .to_string();

    write_daemon_config(&config_path, &socket_path, &state_path, &plugin_path);

    let daemon = DaemonHarness::spawn(&config_path, &socket_path, &log_path);

    let target = TargetId::Device(DeviceId::new(DEVICE_ID));
    let mut stream = connect_and_handshake(&socket_path).await;
    let mut next_id = 1_u64;

    let status = call(
        &mut stream,
        &mut next_id,
        Request::BeginShmFrameStream {
            target: target.clone(),
        },
    )
    .await;
    let (generation, service_name, event_service_name, stream_nonce, segment_bytes) =
        expect_shm_ready(status);
    assert_eq!(
        usize::try_from(segment_bytes).expect("segment_bytes fits in usize"),
        SHM_CLIENT_FRAME_HEADER_LEN + PIXEL_COUNT * ShmPixelFormat::Rgb8.bytes_per_pixel()
    );

    let (publisher, notifier) = open_publisher(&service_name, &event_service_name, segment_bytes);
    for sequence in 0..FRAME_COUNT {
        publish_frame(&publisher, &notifier, generation, stream_nonce, sequence);
        // Avoid coalescing every sample before the daemon wakes.
        thread::sleep(Duration::from_millis(20));
    }

    // The fast path is deliberately fire-and-forget (no per-frame ack), so
    // give the daemon's stream thread a bounded window to drain and apply
    // every published sample before asking it to end the stream.
    thread::sleep(Duration::from_millis(500));

    let end_status = call(
        &mut stream,
        &mut next_id,
        Request::EndShmFrameStream {
            target: target.clone(),
            generation,
        },
    )
    .await;
    assert!(
        matches!(end_status, ResponseStatus::Ack),
        "unexpected EndShmFrameStream response: {end_status:?}"
    );

    let log = wait_for_log_containing(&log_path, "demo display shared-memory stream ended");
    assert!(
        log.contains(&format!("frames_applied={FRAME_COUNT}")),
        "expected frames_applied={FRAME_COUNT} in daemon log, got:\n{log}"
    );

    // Confirm cleanup: the daemon actually released the target, so a second
    // negotiation on it succeeds rather than reporting an active-stream
    // conflict.
    let second = call(
        &mut stream,
        &mut next_id,
        Request::BeginShmFrameStream {
            target: target.clone(),
        },
    )
    .await;
    let (second_generation, ..) = expect_shm_ready(second);
    let cleanup_status = call(
        &mut stream,
        &mut next_id,
        Request::EndShmFrameStream {
            target,
            generation: second_generation,
        },
    )
    .await;
    assert!(matches!(cleanup_status, ResponseStatus::Ack));

    drop(stream);
    drop(daemon);
}

async fn connect_and_handshake(socket_path: &Path) -> Connection {
    let address = Address::from_configured_path(socket_path);
    let deadline = Instant::now() + WAIT_TIMEOUT;
    let mut stream = loop {
        match connect(&address).await {
            Ok(stream) => break stream,
            Err(error) if Instant::now() < deadline => {
                time::sleep(POLL_INTERVAL).await;
                let _ = error;
            }
            Err(error) => panic!("connect to daemon socket: {error}"),
        }
    };
    framing::send(
        &mut stream,
        &ClientHello::new("shm-client-stream-test", "0.0.0"),
    )
    .await
    .expect("send client hello");
    let hello: DaemonHello = framing::receive(&mut stream)
        .await
        .expect("receive daemon hello");
    assert_eq!(
        hello.compatibility,
        Compatibility::Compatible,
        "daemon reported an incompatible protocol ABI version"
    );
    assert_eq!(
        hello.protocol_abi_version, PROTOCOL_ABI_VERSION,
        "daemon's protocol ABI version didn't match this test's compiled-in constant"
    );

    framing::send(
        &mut stream,
        &AuthenticationRequest {
            authentication: Authentication::Peer,
            scope: None,
        },
    )
    .await
    .expect("send peer authentication request");
    let authentication: AuthenticationResponse = framing::receive(&mut stream)
        .await
        .expect("receive authentication response");
    assert!(
        matches!(authentication, AuthenticationResponse::Authenticated { .. }),
        "daemon rejected same-user peer authentication: {authentication:?}"
    );

    stream
}

async fn call(stream: &mut Connection, next_id: &mut u64, request: Request) -> ResponseStatus {
    let id = *next_id;
    *next_id += 1;
    framing::send(stream, &RequestMessage { id, request })
        .await
        .expect("send request");
    let message: ResponseMessage = framing::receive(stream).await.expect("receive response");
    assert_eq!(message.id, id, "response id did not match request id");
    message.response.status
}

/// Extracts the negotiated parameters from a
/// `ResponseStatus::ShmFrameStreamReady` response.
///
/// Panics on any other response variant. This helper centralizes the
/// exhaustive `ResponseStatus` match required by the workspace's
/// `wildcard_enum_match_arm` lint and verifies the demo plugin's expected
/// `Rgb8` pixel format.
fn expect_shm_ready(status: ResponseStatus) -> (u32, String, String, u64, u32) {
    match status {
        ResponseStatus::ShmFrameStreamReady {
            generation,
            service_name,
            event_service_name,
            pixel_format,
            stream_nonce,
            segment_bytes,
        } => {
            assert_eq!(
                pixel_format,
                ShmPixelFormat::Rgb8,
                "demo-led-display only advertises Rgb8"
            );
            (
                generation,
                service_name,
                event_service_name,
                stream_nonce,
                segment_bytes,
            )
        }
        other @ (ResponseStatus::ServerInfo(_)
        | ResponseStatus::Devices(_)
        | ResponseStatus::WithdrawnDevices(_)
        | ResponseStatus::Device(_)
        | ResponseStatus::State(_)
        | ResponseStatus::Ack
        | ResponseStatus::FrameStreamStarted { .. }
        | ResponseStatus::FrameAck { .. }
        | ResponseStatus::CollectionCreated { .. }
        | ResponseStatus::Collections(_)
        | ResponseStatus::CollectionInfo(_)
        | ResponseStatus::CollectionState(_)
        | ResponseStatus::EventTicket(_)
        | ResponseStatus::CollectionApplied { .. }
        | ResponseStatus::PluginSetupWorkflows(_)
        | ResponseStatus::PluginSetupSession(_)
        | ResponseStatus::ManagementSnapshot(_)
        | ResponseStatus::ManagementPatched(_)
        | ResponseStatus::AccessPolicy(_)
        | ResponseStatus::TokenCreated { .. }
        | ResponseStatus::Tokens(_)
        | ResponseStatus::AttestationCreated { .. }
        | ResponseStatus::Attestations(_)
        | ResponseStatus::Scene(_)
        | ResponseStatus::Scenes(_)
        | ResponseStatus::SceneInfo(_)
        | ResponseStatus::SceneApplied { .. }
        | ResponseStatus::Transition(_)
        | ResponseStatus::Error(_)) => panic!("expected ShmFrameStreamReady, got {other:?}"),
    }
}

/// Attaches to the two iceoryx2 services the daemon already created,
/// mirroring `libluminate`'s `ffi_typed::shm::open_publisher`.
fn open_publisher(
    service_name: &str,
    event_service_name: &str,
    segment_bytes: u32,
) -> (
    Publisher<ipc_threadsafe::Service, [u8], ()>,
    Notifier<ipc_threadsafe::Service>,
) {
    let node = luminate_host_supervisor::create_node().expect("create iceoryx2 node");

    let pubsub_name = ServiceName::new(service_name).expect("valid publish-subscribe service name");
    let pubsub = node
        .service_builder(&pubsub_name)
        .publish_subscribe::<[u8]>()
        .open()
        .expect("open shared-memory segment");
    let publisher = pubsub
        .publisher_builder()
        .initial_max_slice_len(usize::try_from(segment_bytes).unwrap_or(usize::MAX))
        .create()
        .expect("create shared-memory publisher");

    let event_name = ServiceName::new(event_service_name).expect("valid event service name");
    let event = node
        .service_builder(&event_name)
        .event()
        .open()
        .expect("open shared-memory event channel");
    let notifier = event
        .notifier_builder()
        .create()
        .expect("create shared-memory notifier");

    (publisher, notifier)
}

fn publish_frame(
    publisher: &Publisher<ipc_threadsafe::Service, [u8], ()>,
    notifier: &Notifier<ipc_threadsafe::Service>,
    generation: u32,
    stream_nonce: u64,
    sequence: u64,
) {
    let per_pixel = ShmPixelFormat::Rgb8.bytes_per_pixel();
    let total_len = SHM_CLIENT_FRAME_HEADER_LEN + PIXEL_COUNT * per_pixel;
    let header = ShmClientFrameHeader {
        sequence,
        generation,
        pixel_count: u32::try_from(PIXEL_COUNT).expect("pixel count fits in u32"),
        pixel_format: ShmPixelFormat::Rgb8.to_abi(),
        header_version: SHM_CLIENT_FRAME_HEADER_VERSION,
        flags: 0,
        reserved: 0,
        stream_nonce,
    }
    .with_commit(true);

    let mut sample = publisher
        .loan_slice(total_len)
        .expect("loan a shared-memory sample");
    let payload = sample.payload_mut();
    let (header_bytes, pixel_bytes) = payload.split_at_mut(SHM_CLIENT_FRAME_HEADER_LEN);
    header_bytes.copy_from_slice(&header.to_bytes());

    let colour = Colour::rgb(Rgb::new(u8::try_from(sequence % 256).unwrap_or(0), 0, 0));
    for chunk in pixel_bytes.chunks_mut(per_pixel) {
        ShmPixelFormat::Rgb8
            .pack(&colour, chunk)
            .expect("pack a pixel");
    }

    sample.send().expect("send a shared-memory sample");
    notifier
        .notify()
        .expect("notify the shared-memory subscriber");
}

fn wait_for_log_containing(log_path: &Path, needle: &str) -> String {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        let contents = fs::read_to_string(log_path).unwrap_or_default();
        if contents.contains(needle) {
            return contents;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for daemon log to contain {needle:?}; log so far:\n{contents}"
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize workspace root")
}

fn build_demo_stack(workspace: &Path) {
    let status = Command::new("cargo")
        .args([
            "build",
            "-p",
            "luminated",
            "-p",
            "luminate-plugin-demo-display",
        ])
        .current_dir(workspace)
        .status()
        .expect("run cargo build for demo stack");
    assert!(status.success(), "demo stack build failed");
}

fn temp_dir() -> TestDir {
    TestDir::new("shm-client-it")
}

fn write_daemon_config(
    config_path: &Path,
    socket_path: &Path,
    state_path: &Path,
    plugin_path: &str,
) {
    let socket_path = toml_string(socket_path.display().to_string());
    let state_path = toml_string(state_path.display().to_string());
    let plugin_path = toml_string(plugin_path);
    let mut config = format!(
        "socket_path = {socket_path}\nstate_path = {state_path}\nplugin_dirs = []\n\n[plugin_management]\nactivation = \"explicit\"\n"
    );
    let _ = write!(
        config,
        "\n[[plugins]]\npath = {plugin_path}\nrequired = true\n"
    );
    fs::write(config_path, config).expect("write daemon config");
}

fn toml_string(value: impl AsRef<str>) -> String {
    format!("{:?}", value.as_ref())
}

struct DaemonHarness {
    child: Child,
    socket_path: PathBuf,
    log_path: PathBuf,
}

impl DaemonHarness {
    fn spawn(config_path: &Path, socket_path: &Path, log_path: &Path) -> Self {
        let stdout = fs::File::options()
            .create(true)
            .append(true)
            .open(log_path)
            .expect("open daemon log for stdout");
        let stderr = fs::File::options()
            .create(true)
            .append(true)
            .open(log_path)
            .expect("open daemon log for stderr");

        let daemon = PathBuf::from(env!("CARGO_BIN_EXE_luminated"));
        let child = Command::new(daemon)
            .env("LUMINATED_CONFIG", config_path)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .expect("spawn daemon");

        Self {
            child,
            socket_path: socket_path.to_path_buf(),
            log_path: log_path.to_path_buf(),
        }
    }
}

impl Drop for DaemonHarness {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Ok(pid) = libc::pid_t::try_from(self.child.id()) {
            // SAFETY: `pid` names the child owned by this harness, and SIGTERM
            // requests the same orderly shutdown used by the daemon service.
            let _ = unsafe { libc::kill(pid, libc::SIGTERM) };
        }
        #[cfg(windows)]
        let _ = self.child.kill();

        let deadline = Instant::now() + WAIT_TIMEOUT;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => break,
                Ok(None) => thread::sleep(POLL_INTERVAL),
            }
        }
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if thread::panicking() {
            let log = fs::read_to_string(&self.log_path).unwrap_or_default();
            println!("daemon log at test failure:\n{log}");
        }
        if self.socket_path.exists() {
            let _ = fs::remove_file(&self.socket_path);
        }
    }
}
