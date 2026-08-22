// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Shared conformance suite for bundled plugins.
//!
//! Unlike a plugin's unit tests, which exercise typed Rust APIs in-process,
//! this suite drives compiled `cdylib` plugins through the real
//! plugin-host protocol by spawning `luminated --plugin-host <path>`,
//! exactly as the daemon does. It therefore validates the complete
//! host ↔ plugin integration: process startup, framing, version negotiation,
//! protocol handling, and plugin behaviour.
//!
//! Framing, the frame-size limit, and the version handshake come directly
//! from the shared `luminate-host-supervisor` crate. The
//! plugin-host-specific protocol types (`HostBootstrap`, `HostRequest`,
//! `HostResponse`, `HostMessage`, and related types) remain private to the
//! `luminated` binary crate, so this test keeps matching client-side
//! definitions. Those definitions must remain serialization-compatible
//! with the server because `serde`/`ciborium` communicate by serialized
//! field and variant names rather than Rust type identity.
//!
//! These are process-level integration tests and are therefore `#[ignore]`
//! by default, like the crate's other integration suites. Run them with:
//!
//! ```sh
//! cargo test -p luminated --test plugin_conformance -- --ignored
//! ```
//!
//! ## Coverage
//!
//! Coverage is intentionally tiered according to how hermetically each
//! plugin can be exercised:
//!
//! - Demo plugins and Linux LEDs (via a fixture sysfs tree) receive full
//!   functional coverage: stable topology, representative capability-based
//!   operations, ordered batch results, and scoped read snapshots.
//! - WLED and LIFX receive full loopback coverage against mock HTTP and UDP
//!   devices, including discovery, topology, representative writes,
//!   ordered batches, and readback. Their empty-topology tests (and
//!   Govee's baseline tests) still exercise bootstrap and shutdown without
//!   external devices.
//! - Alienware depends on physical HID hardware. Without compatible
//!   hardware the plugin host correctly reports initialization failure, so
//!   this suite verifies that the failure is propagated cleanly over the
//!   protocol rather than hanging or crashing. Hardware-specific behaviour
//!   is covered separately.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::tests_outside_test_module,
    reason = "Integration tests are crate roots and intentionally fail loudly when setup assumptions break."
)]

use std::env;
#[cfg(target_os = "linux")]
use std::fs;
use std::io::{ErrorKind, Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use luminate_core::capability::{BrightnessCapability, CapabilitySet, StateReadbackCapability};
use luminate_core::colour::Colour;
use luminate_core::control::ReconciliationPolicy;
use luminate_core::effect::Effect;
use luminate_core::frame::{FrameEnvelope, FramePayload};
use luminate_core::rgb::Rgb;
use luminate_host_supervisor::sync_io::{read_frame, write_frame};
use luminate_host_supervisor::{Compatibility, HostHello, SupervisorHello};
#[cfg(target_os = "linux")]
use luminate_platform::test_support::TestDir;
use luminate_platform::{dynamic_library_candidates, executable_name};
use luminate_plugin_api::{
    DeviceDescriptor, PluginFrameUpload, PluginReadRequest, PluginReadTarget,
    PluginSettingApplyMode, PluginSettingKind, PluginStateSnapshot, PluginTarget, PluginUpdate,
    PluginUpdateOperation,
};

const HOST_MODE_ARGUMENT: &str = "--plugin-host";
const INSPECT_MODE_ARGUMENT: &str = "--plugin-inspect";
const HOST_LOG_LEVEL_ENV: &str = "LUMINATE_PLUGIN_HOST_LOG_LEVEL";
const HOST_LOG_LEVEL_WARN: &str = "1";
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Serialize)]
struct HostBootstrap {
    configuration_cbor: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct InspectionResult {
    name: String,
    version: String,
    buses: Vec<u32>,
    settings: Vec<InspectedSetting>,
}

#[derive(Debug, Deserialize)]
struct InspectedSetting {
    key: String,
    label: String,
    description: String,
    kind: u32,
    default_toml: Option<String>,
    required: bool,
    sensitive: bool,
    apply_mode: u32,
    minimum: Option<f64>,
    maximum: Option<f64>,
    constraints_toml: Option<String>,
}

#[derive(Debug, Serialize)]
struct HostRequest {
    id: u64,
    timeout_millis: u64,
    command: HostCommand,
}

#[derive(Debug, Serialize)]
enum HostCommand {
    Topology,
    Apply(PluginUpdate),
    ApplyBatch(Vec<PluginUpdate>),
    ReadState(PluginReadRequest),
    UploadFrame(PluginFrameUpload),
    Shutdown,
}

#[allow(
    dead_code,
    reason = "message payloads are kept for wire-shape parity with the server type and are \
              inspected only through the derived Debug impl in assertion failure output"
)]
#[derive(Debug, Clone, Deserialize)]
enum ApplyOutcome {
    Applied,
    Unsupported(String),
    InvalidArgument(String),
    Io(String),
    Unavailable(String),
    RateLimited {
        diagnostic: String,
        retry_after_ms: Option<u64>,
    },
    Internal(String),
}

#[derive(Debug, Deserialize)]
enum HostResponse {
    Topology(Vec<DeviceDescriptor>),
    Apply(ApplyOutcome),
    Batch(Vec<ApplyOutcome>),
    State(PluginStateSnapshot),
    Frame(ApplyOutcome),
    Shutdown,
}

#[derive(Debug, Deserialize)]
struct WireMetadata {
    #[allow(dead_code, reason = "kept for wire-shape parity with the server type")]
    name: String,

    #[allow(dead_code, reason = "kept for wire-shape parity with the server type")]
    version: String,

    #[allow(dead_code, reason = "kept for wire-shape parity with the server type")]
    priority: i32,

    #[allow(dead_code, reason = "kept for wire-shape parity with the server type")]
    recommended_reconciliation: Option<ReconciliationPolicy>,

    #[allow(dead_code, reason = "kept for wire-shape parity with the server type")]
    probe_outcome: u8,

    #[allow(dead_code, reason = "kept for wire-shape parity with the server type")]
    buses: Vec<u32>,

    #[allow(dead_code, reason = "kept for wire-shape parity with the server type")]
    vendors: Vec<(u32, u32)>,

    #[allow(dead_code, reason = "kept for wire-shape parity with the server type")]
    probe_hints: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct HostReady {
    #[allow(dead_code, reason = "kept for wire-shape parity with the server type")]
    metadata: WireMetadata,
    descriptors: Vec<DeviceDescriptor>,
}

#[allow(
    dead_code,
    reason = "Log fields are kept for wire-shape parity with the server type; this client \
              discards log content and only distinguishes the message kind"
)]
#[derive(Debug, Deserialize)]
enum HostMessage {
    Ready(Result<HostReady, String>),
    Response {
        id: u64,
        result: Result<HostResponse, String>,
    },
    TopologyChanged,
    Log {
        plugin: String,
        level: u8,
        message: String,
    },
}

/// One `--plugin-host` child process, driven over its real stdin/stdout wire
/// protocol. A background reader thread decouples reading from writing
/// (matching `HostConnection`'s own design), so `call_with_timeout` can
/// bound how long this client waits regardless of what the child does.
#[derive(Debug)]
struct HostSession {
    child: Child,
    stdin: ChildStdin,
    messages: mpsc::Receiver<HostMessage>,
    next_id: u64,
}

impl HostSession {
    fn spawn(
        luminated: &Path,
        plugin: &Path,
        configuration: &Value,
    ) -> Result<(Self, HostReady), String> {
        Self::spawn_with_envs(luminated, plugin, configuration, &[])
    }

    /// Like `spawn`, but also sets extra environment variables on the child
    /// only, rather than mutating this (test) process's own environment.
    fn spawn_with_envs(
        luminated: &Path,
        plugin: &Path,
        configuration: &Value,
        envs: &[(&str, &Path)],
    ) -> Result<(Self, HostReady), String> {
        let mut command = Command::new(luminated);
        command
            .arg(HOST_MODE_ARGUMENT)
            .arg(plugin)
            .env(HOST_LOG_LEVEL_ENV, HOST_LOG_LEVEL_WARN)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (key, value) in envs {
            command.env(key, value);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("spawn plugin host for {}: {error}", plugin.display()))?;
        let mut stdin = child.stdin.take().expect("plugin host stdin was piped");
        let mut stdout = child.stdout.take().expect("plugin host stdout was piped");

        // Version handshake before the real bootstrap frame, exercising the
        // same real `luminate-host-supervisor` types and framing
        // `plugin_host/mod.rs` uses on the production supervisor side.
        write_frame(&mut stdin, &SupervisorHello::new())
            .map_err(|error| format!("write supervisor hello: {error}"))?;
        let hello: HostHello =
            read_frame(&mut stdout).map_err(|error| format!("read host hello: {error}"))?;
        if !matches!(hello.compatibility, Compatibility::Compatible) {
            return Err(format!(
                "unexpected handshake compatibility: {:?}",
                hello.compatibility
            ));
        }

        let mut configuration_cbor = Vec::new();
        ciborium::into_writer(&configuration, &mut configuration_cbor)
            .map_err(|error| format!("encode plugin configuration: {error}"))?;
        write_frame(&mut stdin, &HostBootstrap { configuration_cbor })
            .map_err(|error| format!("write host bootstrap: {error}"))?;

        let (sender, receiver) = mpsc::channel();
        let _reader = thread::spawn(move || {
            let mut stdout = stdout;
            while let Ok(message) = read_frame::<_, HostMessage>(&mut stdout) {
                if sender.send(message).is_err() {
                    break;
                }
            }
        });

        let mut session = Self {
            child,
            stdin,
            messages: receiver,
            next_id: 1,
        };
        let ready = session.await_ready(READY_TIMEOUT)?;
        Ok((session, ready))
    }

    fn await_ready(&mut self, timeout: Duration) -> Result<HostReady, String> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("plugin host did not become ready in time".to_owned());
            }
            match self.messages.recv_timeout(remaining) {
                Ok(HostMessage::Ready(result)) => return result,
                Ok(
                    HostMessage::Log { .. }
                    | HostMessage::TopologyChanged
                    | HostMessage::Response { .. },
                ) => {}
                Err(_) => return Err("plugin host closed before becoming ready".to_owned()),
            }
        }
    }

    fn call(&mut self, command: HostCommand) -> Result<HostResponse, String> {
        self.call_with_timeout(command, CALL_TIMEOUT)
    }

    /// `budget` is both the deadline told to the plugin (`timeout_millis`,
    /// the same value the daemon derives from its own call timeout) and,
    /// with headroom added, this client's own wait bound, so a plugin that
    /// ignores its budget and hangs cannot hang the test suite either.
    fn call_with_timeout(
        &mut self,
        command: HostCommand,
        budget: Duration,
    ) -> Result<HostResponse, String> {
        let id = self.next_id;
        self.next_id += 1;
        let timeout_millis = u64::try_from(budget.as_millis()).unwrap_or(u64::MAX);
        write_frame(
            &mut self.stdin,
            &HostRequest {
                id,
                timeout_millis,
                command,
            },
        )
        .map_err(|error| format!("write host request: {error}"))?;

        let deadline = Instant::now() + budget + Duration::from_secs(2);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("plugin host request {id} exceeded {budget:?}"));
            }
            match self.messages.recv_timeout(remaining) {
                Ok(HostMessage::Response {
                    id: response_id,
                    result,
                }) if response_id == id => {
                    return result;
                }
                Ok(
                    HostMessage::Response { .. }
                    | HostMessage::TopologyChanged
                    | HostMessage::Ready(_)
                    | HostMessage::Log { .. },
                ) => {}
                Err(_) => return Err("plugin host exited while processing request".to_owned()),
            }
        }
    }

    fn shutdown(mut self) {
        let _ = self.call(HostCommand::Shutdown);
        let _ = self.child.wait();
    }
}

impl Drop for HostSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize workspace root")
}

fn build_plugin_stack(workspace: &Path, packages: &[&str]) {
    let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.arg("build").arg("-p").arg("luminated");
    for package in packages {
        command.arg("-p").arg(package);
    }
    let status = command
        .current_dir(workspace)
        .status()
        .expect("run cargo build for the plugin conformance stack");
    assert!(status.success(), "plugin conformance stack build failed");
}

fn luminated_binary(workspace: &Path) -> PathBuf {
    workspace.join(format!("target/debug/{}", executable_name("luminated")))
}

fn plugin_artifact(workspace: &Path, crate_name: &str) -> PathBuf {
    workspace.join(format!(
        "target/debug/{}",
        dynamic_library_candidates(&crate_name.replace('-', "_"))[0]
    ))
}

fn empty_configuration() -> Value {
    Value::Object(Map::new())
}

fn inspect_plugin(luminated: &Path, plugin: &Path) -> InspectionResult {
    let mut child = Command::new(luminated)
        .arg(INSPECT_MODE_ARGUMENT)
        .arg(plugin)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn disposable plugin inspector");
    let result: Result<InspectionResult, String> = read_frame(
        child
            .stdout
            .as_mut()
            .expect("plugin inspector stdout was piped"),
    )
    .expect("read plugin inspection result");
    let status = child.wait().expect("wait for plugin inspector");
    assert!(status.success(), "plugin inspector exited with {status}");
    result.expect("plugin inspection succeeded")
}

// ---------------------------------------------------------------------
// Shared conformance assertions
// ---------------------------------------------------------------------

fn encode_cbor(value: &impl Serialize) -> Vec<u8> {
    let mut buffer = Vec::new();
    ciborium::into_writer(value, &mut buffer).expect("encode CBOR for comparison");
    buffer
}

#[test]
#[ignore = "spawns workspace binaries and requires the lifx plugin artifact"]
fn lifx_static_inspection_reports_complete_configuration_metadata() {
    let workspace = workspace_root();
    build_plugin_stack(&workspace, &["luminate-plugin-lifx"]);
    let inspected = inspect_plugin(
        &luminated_binary(&workspace),
        &plugin_artifact(&workspace, "luminate-plugin-lifx"),
    );

    assert_eq!(inspected.name, "luminate-plugin-lifx");
    assert!(!inspected.version.is_empty());
    assert_eq!(
        inspected.buses,
        [luminate_plugin_api::PluginBus::Network as u32]
    );
    assert_eq!(inspected.settings.len(), 1);
    let setting = &inspected.settings[0];
    assert_eq!(setting.key, "discovery_address");
    assert_eq!(setting.label, "Discovery address");
    assert!(!setting.description.is_empty());
    assert_eq!(setting.kind, PluginSettingKind::String as u32);
    assert_eq!(
        setting.default_toml.as_deref(),
        Some("\"255.255.255.255:56700\"")
    );
    assert!(!setting.required);
    assert!(!setting.sensitive);
    assert_eq!(
        setting.apply_mode,
        PluginSettingApplyMode::RestartRequired as u32
    );
    assert_eq!(setting.minimum, None);
    assert_eq!(setting.maximum, None);
    assert_eq!(setting.constraints_toml, None);
}

fn topology(session: &mut HostSession) -> Vec<DeviceDescriptor> {
    match session.call(HostCommand::Topology).expect("topology call") {
        HostResponse::Topology(descriptors) => descriptors,
        other @ (HostResponse::Apply(_)
        | HostResponse::Batch(_)
        | HostResponse::State(_)
        | HostResponse::Frame(_)
        | HostResponse::Shutdown) => panic!("expected Topology response, got {other:?}"),
    }
}

/// Verifies that repeated `Topology` requests produce identical results
/// when nothing has changed.
///
/// Two consecutive `Topology` calls must have byte-identical CBOR
/// encodings, proving that topology enumeration is deterministic and
/// stably ordered in the absence of topology-changing events.
///
/// Callback/capability consistency is also exercised implicitly because
/// the host validates that contract (`validate_topology_contract`) before
/// returning any topology response.
fn assert_topology_is_stable(session: &mut HostSession) -> Vec<DeviceDescriptor> {
    let first = topology(session);
    let second = topology(session);
    assert_eq!(
        encode_cbor(&first),
        encode_cbor(&second),
        "topology must be deterministic and identically ordered across repeated calls"
    );
    first
}

fn await_device(
    session: &mut HostSession,
    device_id: &str,
    wait: Duration,
) -> Vec<DeviceDescriptor> {
    let deadline = Instant::now() + wait;
    loop {
        let descriptors = topology(session);
        if descriptors.iter().any(|device| device.id == device_id) {
            return descriptors;
        }
        assert!(
            Instant::now() < deadline,
            "plugin did not publish expected device {device_id} within {wait:?}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn apply(
    session: &mut HostSession,
    target: PluginTarget,
    operation: PluginUpdateOperation,
) -> ApplyOutcome {
    match session
        .call(HostCommand::Apply(PluginUpdate { target, operation }))
        .expect("apply call")
    {
        HostResponse::Apply(outcome) => outcome,
        other @ (HostResponse::Topology(_)
        | HostResponse::Batch(_)
        | HostResponse::State(_)
        | HostResponse::Frame(_)
        | HostResponse::Shutdown) => panic!("expected Apply response, got {other:?}"),
    }
}

/// Chooses a representative set of operations from a device's advertised
/// capabilities.
///
/// `Clear` is accepted by every bundled plugin and therefore serves as the
/// baseline mutating operation. Static `SetEffect` and `SetBrightness` are added
/// only when the device advertises those capabilities, providing one
/// representative operation for each relevant capability without encoding
/// plugin-specific operation matrices into the test suite.
fn representative_operations(capabilities: &CapabilitySet) -> Vec<PluginUpdateOperation> {
    let mut operations = vec![PluginUpdateOperation::Clear];
    if !capabilities.colour.is_empty() {
        operations.push(PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: Colour::rgb(Rgb::new(20, 40, 60)),
            },
        });
    }
    if matches!(
        capabilities.brightness,
        BrightnessCapability::Independent { .. }
    ) {
        operations.push(PluginUpdateOperation::SetBrightness { value: 50 });
    }
    operations
}

/// Verifies that each advertised capability accepts a representative,
/// well-typed request.
///
/// One representative operation is exercised against every device's
/// whole-device target. Success is not required: a plugin may legitimately
/// reject, for example, a whole-device `SetBrightness` in favor of
/// per-surface control. Such rejections must be reported with an
/// appropriate `ApplyOutcome`, never `Internal`, which indicates a
/// host/plugin bug rather than an intentional decision.
fn assert_representative_operations_are_well_typed(
    session: &mut HostSession,
    descriptors: &[DeviceDescriptor],
) {
    for device in descriptors {
        let target = PluginTarget::Device {
            device: device.id.clone(),
        };
        for operation in representative_operations(&device.capabilities) {
            let outcome = apply(session, target.clone(), operation.clone());
            assert!(
                !matches!(outcome, ApplyOutcome::Internal(_)),
                "device {} operation {} produced an internal error instead of a typed decision: {outcome:?}",
                device.id,
                operation.name()
            );
        }
    }
}

/// Verifies that batch processing is ordered and isolated.
///
/// A batch containing `[valid, unowned, valid]` updates must produce
/// exactly one ordered outcome per request. The unowned target must be
/// rejected with a typed error rather than aborting the batch or changing
/// the ordering of later results.
///
/// Dynamic plugins may report `Unavailable` because, after ownership
/// expires, an expired known device and an identity that was never owned
/// are intentionally indistinguishable.
fn assert_batch_results_are_ordered(session: &mut HostSession, device_id: &str) {
    let valid = PluginUpdate {
        target: PluginTarget::Device {
            device: device_id.to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };
    let invalid = PluginUpdate {
        target: PluginTarget::Device {
            device: "luminate-conformance-suite-unowned-device".to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };
    let batch = vec![valid.clone(), invalid, valid];
    let outcomes = match session
        .call(HostCommand::ApplyBatch(batch))
        .expect("apply-batch call")
    {
        HostResponse::Batch(outcomes) => outcomes,
        other @ (HostResponse::Topology(_)
        | HostResponse::Apply(_)
        | HostResponse::State(_)
        | HostResponse::Frame(_)
        | HostResponse::Shutdown) => panic!("expected Batch response, got {other:?}"),
    };
    let [first, second, third] = outcomes.try_into().unwrap_or_else(|outcomes: Vec<ApplyOutcome>| {
        panic!(
            "batch must return exactly one ordered outcome per submitted update, got {} outcomes: {outcomes:?}",
            outcomes.len()
        )
    });
    assert!(
        !matches!(first, ApplyOutcome::Internal(_)),
        "first (valid) batch entry produced an internal error: {first:?}"
    );
    assert!(
        matches!(
            second,
            ApplyOutcome::Unsupported(_)
                | ApplyOutcome::InvalidArgument(_)
                | ApplyOutcome::Unavailable(_)
        ),
        "the unowned-device entry must be rejected with a typed error, got {second:?}"
    );
    assert!(
        !matches!(third, ApplyOutcome::Internal(_)),
        "third (valid) batch entry produced an internal error: {third:?}"
    );
}

/// Verifies that readback responses are scoped to the requested target.
///
/// For every device advertising readback, requests every readable facet at
/// the whole-device target. Every returned observation or error must refer
/// only to that target; the response must never include observations for
/// targets the caller did not request.
fn assert_read_state_is_bounded_and_scoped(
    session: &mut HostSession,
    descriptors: &[DeviceDescriptor],
) {
    for device in descriptors {
        let StateReadbackCapability::Readable { facets, .. } = &device.capabilities.state_readback
        else {
            continue;
        };
        let target = PluginTarget::Device {
            device: device.id.clone(),
        };
        let request = PluginReadRequest {
            targets: vec![PluginReadTarget {
                target: target.clone(),
                facets: facets.iter().map(|facet| facet.facet).collect(),
            }],
        };
        let snapshot: PluginStateSnapshot = match session
            .call(HostCommand::ReadState(request))
            .expect("read-state call")
        {
            HostResponse::State(snapshot) => snapshot,
            other @ (HostResponse::Topology(_)
            | HostResponse::Apply(_)
            | HostResponse::Batch(_)
            | HostResponse::Frame(_)
            | HostResponse::Shutdown) => panic!("expected State response, got {other:?}"),
        };
        for observation in &snapshot.observations {
            assert_eq!(
                observation.target.to_string(),
                target.to_string(),
                "device {}: observation must be scoped to the requested target",
                device.id
            );
        }
        for error in &snapshot.errors {
            assert_eq!(
                error.target.to_string(),
                target.to_string(),
                "device {}: read error must be scoped to the requested target",
                device.id
            );
        }
    }
}

fn upload_frame(
    session: &mut HostSession,
    target: PluginTarget,
    envelope: FrameEnvelope,
) -> ApplyOutcome {
    match session
        .call(HostCommand::UploadFrame(PluginFrameUpload {
            target,
            envelope,
        }))
        .expect("upload-frame call")
    {
        HostResponse::Frame(outcome) => outcome,
        other @ (HostResponse::Topology(_)
        | HostResponse::Apply(_)
        | HostResponse::Batch(_)
        | HostResponse::State(_)
        | HostResponse::Shutdown) => panic!("expected Frame response, got {other:?}"),
    }
}

/// Verifies that `frame_upload` produces well-typed outcomes for both
/// valid and invalid requests.
///
/// A well-formed frame must not return `Internal`. Likewise, a malformed
/// frame (for example, the wrong pixel count for a `FullFrameOnly`
/// target) and a frame addressed to an unowned device must both be rejected
/// with typed errors rather than `Internal`.
///
/// As with `assert_batch_results_are_ordered`, the unowned-device case
/// exercises authorization failure rather than a host or plugin bug.
fn assert_frame_streaming_is_well_typed(
    session: &mut HostSession,
    device_id: &str,
    valid_frame: FrameEnvelope,
    malformed_frame: FrameEnvelope,
) {
    let target = PluginTarget::Device {
        device: device_id.to_owned(),
    };
    let outcome = upload_frame(session, target.clone(), valid_frame);
    assert!(
        !matches!(outcome, ApplyOutcome::Internal(_)),
        "device {device_id}: a well-formed frame produced an internal error instead of a typed decision: {outcome:?}"
    );

    let outcome = upload_frame(session, target, malformed_frame.clone());
    assert!(
        !matches!(outcome, ApplyOutcome::Internal(_)),
        "device {device_id}: a malformed frame produced an internal error instead of a typed decision: {outcome:?}"
    );

    let unowned = PluginTarget::Device {
        device: "luminate-conformance-suite-unowned-device".to_owned(),
    };
    let outcome = upload_frame(session, unowned, malformed_frame);
    assert!(
        !matches!(outcome, ApplyOutcome::Internal(_)),
        "an unowned-device frame produced an internal error instead of a typed decision: {outcome:?}"
    );
}

/// Runs every shared assertion against an already-spawned session and hands
/// the topology back, for tests that want to layer plugin-specific checks on
/// top.
fn run_shared_conformance_checks(session: &mut HostSession) -> Vec<DeviceDescriptor> {
    let descriptors = assert_topology_is_stable(session);
    assert_representative_operations_are_well_typed(session, &descriptors);
    assert_read_state_is_bounded_and_scoped(session, &descriptors);
    if let Some(device) = descriptors.first() {
        assert_batch_results_are_ordered(session, &device.id);
    }
    descriptors
}

#[test]
#[ignore = "spawns workspace binaries and requires the demo-bulb plugin artifact"]
fn demo_bulb_conforms_to_the_plugin_host_protocol() {
    let workspace = workspace_root();
    build_plugin_stack(&workspace, &["luminate-plugin-demo-bulb"]);
    let plugin = plugin_artifact(&workspace, "luminate-plugin-demo-bulb");

    let (mut session, ready) = HostSession::spawn(
        &luminated_binary(&workspace),
        &plugin,
        &empty_configuration(),
    )
    .expect("spawn demo-bulb plugin host");
    assert_eq!(
        ready.descriptors.len(),
        1,
        "demo bulb reports exactly one device"
    );

    let descriptors = run_shared_conformance_checks(&mut session);
    assert_eq!(descriptors.len(), 1);
    assert_frame_streaming_is_well_typed(
        &mut session,
        &descriptors[0].id,
        FrameEnvelope {
            generation: 1,
            sequence: 0,
            payload: FramePayload::Full(vec![Colour::rgb(Rgb::new(10, 20, 30))]),
            commit: false,
        },
        FrameEnvelope {
            generation: 1,
            sequence: 1,
            payload: FramePayload::Full(vec![
                Colour::rgb(Rgb::new(1, 2, 3)),
                Colour::rgb(Rgb::new(4, 5, 6)),
            ]),
            commit: false,
        },
    );

    session.shutdown();
}

#[test]
#[ignore = "spawns workspace binaries and requires the demo-ambient plugin artifact"]
fn demo_ambient_conforms_to_the_plugin_host_protocol() {
    let workspace = workspace_root();
    build_plugin_stack(&workspace, &["luminate-plugin-demo-ambient"]);
    let plugin = plugin_artifact(&workspace, "luminate-plugin-demo-ambient");

    let (mut session, ready) = HostSession::spawn(
        &luminated_binary(&workspace),
        &plugin,
        &empty_configuration(),
    )
    .expect("spawn demo-ambient plugin host");
    assert!(
        !ready.descriptors.is_empty(),
        "demo ambient reports at least one device"
    );

    let descriptors = run_shared_conformance_checks(&mut session);
    assert!(
        descriptors.iter().any(|device| matches!(
            device.capabilities.state_readback,
            StateReadbackCapability::Readable { .. }
        )),
        "demo ambient's whole point is exact readback; the shared readback check must have run"
    );

    session.shutdown();
}

#[test]
#[ignore = "spawns workspace binaries and requires the demo-keyboard plugin artifact"]
fn demo_keyboard_conforms_to_the_plugin_host_protocol() {
    let workspace = workspace_root();
    build_plugin_stack(&workspace, &["luminate-plugin-demo-keyboard"]);
    let plugin = plugin_artifact(&workspace, "luminate-plugin-demo-keyboard");

    let (mut session, ready) = HostSession::spawn(
        &luminated_binary(&workspace),
        &plugin,
        &empty_configuration(),
    )
    .expect("spawn demo-keyboard plugin host");
    let device_id = ready
        .descriptors
        .first()
        .expect("demo keyboard reports its device")
        .id
        .clone();

    run_shared_conformance_checks(&mut session);

    // Native batching's whole point is coalescing many per-key updates into
    // one hardware transaction; drive several element targets through one
    // ApplyBatch and confirm they still come back as ordered, well-typed
    // per-entry outcomes (not collapsed into one shared result).
    let surface = ready
        .descriptors
        .first()
        .and_then(|device| device.surfaces.first())
        .expect("demo keyboard reports a surface")
        .id
        .clone();
    let element = ready
        .descriptors
        .first()
        .and_then(|device| device.surfaces.first())
        .and_then(|surface| surface.elements.first())
        .expect("demo keyboard reports at least one key element")
        .id
        .clone();
    let per_key_updates = vec![
        PluginUpdate {
            target: PluginTarget::Element {
                device: device_id.clone(),
                surface: surface.clone(),
                element: element.clone(),
            },
            operation: PluginUpdateOperation::SetEffect {
                effect: Effect::Static {
                    colour: Colour::rgb(Rgb::new(1, 2, 3)),
                },
            },
        },
        PluginUpdate {
            target: PluginTarget::Element {
                device: device_id,
                surface,
                element,
            },
            operation: PluginUpdateOperation::SetEffect {
                effect: Effect::Static {
                    colour: Colour::rgb(Rgb::new(4, 5, 6)),
                },
            },
        },
    ];
    let outcomes = match session
        .call(HostCommand::ApplyBatch(per_key_updates))
        .expect("per-key apply-batch call")
    {
        HostResponse::Batch(outcomes) => outcomes,
        other @ (HostResponse::Topology(_)
        | HostResponse::Apply(_)
        | HostResponse::State(_)
        | HostResponse::Frame(_)
        | HostResponse::Shutdown) => panic!("expected Batch response, got {other:?}"),
    };
    assert_eq!(outcomes.len(), 2, "one outcome per per-key update");
    for outcome in &outcomes {
        assert!(
            !matches!(outcome, ApplyOutcome::Internal(_)),
            "per-key batched update produced an internal error: {outcome:?}"
        );
    }

    session.shutdown();
}

#[test]
#[ignore = "spawns workspace binaries and requires the demo-system plugin artifact"]
fn demo_system_conforms_to_the_plugin_host_protocol() {
    let workspace = workspace_root();
    build_plugin_stack(&workspace, &["luminate-plugin-demo-system"]);
    let plugin = plugin_artifact(&workspace, "luminate-plugin-demo-system");

    let (mut session, ready) = HostSession::spawn(
        &luminated_binary(&workspace),
        &plugin,
        &empty_configuration(),
    )
    .expect("spawn demo-system plugin host");
    assert!(
        ready.descriptors.len() > 1,
        "demo system is the broad multi-device capability reference"
    );

    run_shared_conformance_checks(&mut session);

    session.shutdown();
}

// ---------------------------------------------------------------------
// Linux LEDs: hermetic via a fixture sysfs root
// ---------------------------------------------------------------------

#[cfg(target_os = "linux")]
#[test]
#[ignore = "spawns workspace binaries and requires the linux-leds plugin artifact"]
fn linux_leds_conforms_to_the_plugin_host_protocol() {
    let workspace = workspace_root();
    build_plugin_stack(&workspace, &["luminate-plugin-linux-leds"]);
    let plugin = plugin_artifact(&workspace, "luminate-plugin-linux-leds");

    let fixture_root = TestDir::new("conformance-linux-leds");
    let led_path = fixture_root.join("platform::kbd_backlight");
    fs::create_dir_all(&led_path).expect("create fixture LED directory");
    fs::write(led_path.join("max_brightness"), "20").expect("write fixture max_brightness");
    fs::write(led_path.join("brightness"), "0").expect("write fixture brightness");

    let (mut session, ready) = HostSession::spawn_with_envs(
        &luminated_binary(&workspace),
        &plugin,
        &empty_configuration(),
        &[("LUMINATE_LED_SYSFS_ROOT", fixture_root.path())],
    )
    .expect("spawn linux-leds plugin host");

    assert_eq!(
        ready.descriptors.len(),
        1,
        "the fixture sysfs root advertises exactly one keyboard backlight"
    );

    run_shared_conformance_checks(&mut session);

    session.shutdown();
}

// ---------------------------------------------------------------------
// Network-discovery plugins: loopback fixtures plus empty baselines
// ---------------------------------------------------------------------

struct MockWled {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl MockWled {
    fn spawn() -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind mock WLED");
        listener
            .set_nonblocking(true)
            .expect("make mock WLED listener nonblocking");
        let address = listener.local_addr().expect("mock WLED address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_in_thread = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_in_thread = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !stop_in_thread.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _peer)) => {
                        serve_wled_request(stream, &requests_in_thread);
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("mock WLED accept failed: {error}"),
                }
            }
        });

        Self {
            address,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("mock WLED request log").clone()
    }
}

impl Drop for MockWled {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let result = thread.join();
            if !thread::panicking() {
                result.expect("mock WLED thread");
            }
        }
    }
}

fn serve_wled_request(mut stream: TcpStream, requests: &Mutex<Vec<String>>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set mock WLED read timeout");
    let mut request = Vec::new();
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let mut chunk = [0_u8; 512];
        let length = match stream.read(&mut chunk) {
            Ok(length) => length,
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset
                ) =>
            {
                return;
            }
            Err(error) => panic!("read mock WLED request: {error}"),
        };
        if length == 0 {
            return;
        }
        request.extend_from_slice(
            chunk
                .get(..length)
                .expect("read length fits mock WLED buffer"),
        );
    }
    let request = str::from_utf8(&request).expect("mock WLED request UTF-8");
    let request_line = request.lines().next().expect("mock WLED request line");
    requests
        .lock()
        .expect("mock WLED request log")
        .push(request_line.to_owned());
    let mut parts = request_line.split_whitespace();
    let method = parts.next().expect("mock WLED method");
    let path = parts.next().expect("mock WLED path");
    let response = match (method, path) {
        ("GET", "/json/info") => {
            r#"{"name":"Conformance WLED","ver":"0.15.0","mac":"AA:BB:CC:DD:EE:FF","leds":{"count":4}}"#
        }
        ("GET", "/json/state") => {
            r#"{"on":true,"bri":128,"seg":[{"id":0,"start":0,"stop":4,"on":true,"bri":128,"col":[[10,20,30]],"fx":0}]}"#
        }
        ("GET", "/json/eff") => r#"["Solid","Breathe"]"#,
        ("POST", "/json/state") => "{}",
        _ => panic!("unexpected mock WLED request: {request_line}"),
    };
    let result = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
        response.len()
    );
    if let Err(error) = result
        && !matches!(
            error.kind(),
            ErrorKind::BrokenPipe | ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset
        )
    {
        panic!("write mock WLED response: {error}");
    }
}

const LIFX_HEADER_LEN: usize = 36;
const LIFX_GET_SERVICE: u16 = 2;
const LIFX_STATE_SERVICE: u16 = 3;
const LIFX_SET_POWER: u16 = 21;
const LIFX_GET_LABEL: u16 = 23;
const LIFX_STATE_LABEL: u16 = 25;
const LIFX_GET_VERSION: u16 = 32;
const LIFX_STATE_VERSION: u16 = 33;
const LIFX_ACKNOWLEDGEMENT: u16 = 45;
const LIFX_GET_COLOR: u16 = 101;
const LIFX_SET_COLOR: u16 = 102;
const LIFX_LIGHT_STATE: u16 = 107;
const LIFX_TARGET: [u8; 8] = [0xd0, 0x73, 0xd5, 0, 0x13, 0x37, 0, 0];
const LIFX_DEVICE_ID: &str = "lifx-d073d5001337";

struct MockLifx {
    address: SocketAddr,
    messages: Arc<Mutex<Vec<u16>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl MockLifx {
    fn spawn() -> Self {
        let socket =
            UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind mock LIFX discovery socket");
        let address = socket.local_addr().expect("read mock LIFX address");
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .expect("set mock LIFX timeout");
        let messages = Arc::new(Mutex::new(Vec::new()));
        let messages_in_thread = Arc::clone(&messages);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_in_thread = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            let mut buffer = [0_u8; 2048];
            while !stop_in_thread.load(Ordering::Acquire) {
                let (length, peer) = match socket.recv_from(&mut buffer) {
                    Ok(received) => received,
                    Err(error)
                        if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                    {
                        continue;
                    }
                    Err(error) => panic!("mock LIFX receive failed: {error}"),
                };
                let Some((source, target, sequence, message_type)) = parse_lifx_header(
                    buffer
                        .get(..length)
                        .expect("received length fits mock LIFX buffer"),
                ) else {
                    continue;
                };
                messages_in_thread
                    .lock()
                    .expect("mock LIFX message log")
                    .push(message_type);
                let (response_type, response_target, payload) = match message_type {
                    LIFX_GET_SERVICE => {
                        let mut payload = vec![1];
                        payload.extend_from_slice(&u32::from(address.port()).to_le_bytes());
                        (LIFX_STATE_SERVICE, LIFX_TARGET, payload)
                    }
                    LIFX_GET_VERSION if target == LIFX_TARGET => {
                        let mut payload = Vec::new();
                        payload.extend_from_slice(&1_u32.to_le_bytes());
                        payload.extend_from_slice(&23_u32.to_le_bytes());
                        (LIFX_STATE_VERSION, LIFX_TARGET, payload)
                    }
                    LIFX_GET_LABEL if target == LIFX_TARGET => {
                        let mut payload = vec![0; 32];
                        payload
                            .get_mut(..16)
                            .expect("mock label field")
                            .copy_from_slice(b"Conformance LIFX");
                        (LIFX_STATE_LABEL, LIFX_TARGET, payload)
                    }
                    LIFX_GET_COLOR if target == LIFX_TARGET => {
                        let mut payload = vec![0; 12];
                        payload
                            .get_mut(4..6)
                            .expect("mock brightness field")
                            .copy_from_slice(&32_896_u16.to_le_bytes());
                        payload
                            .get_mut(6..8)
                            .expect("mock kelvin field")
                            .copy_from_slice(&3_500_u16.to_le_bytes());
                        payload
                            .get_mut(10..12)
                            .expect("mock power field")
                            .copy_from_slice(&u16::MAX.to_le_bytes());
                        (LIFX_LIGHT_STATE, LIFX_TARGET, payload)
                    }
                    LIFX_SET_POWER | LIFX_SET_COLOR if target == LIFX_TARGET => {
                        (LIFX_ACKNOWLEDGEMENT, LIFX_TARGET, Vec::new())
                    }
                    _ => continue,
                };
                let response =
                    lifx_packet(source, response_target, sequence, response_type, &payload);
                socket
                    .send_to(&response, peer)
                    .expect("send mock LIFX response");
            }
        });
        Self {
            address,
            messages,
            stop,
            thread: Some(thread),
        }
    }

    fn messages(&self) -> Vec<u16> {
        self.messages.lock().expect("mock LIFX message log").clone()
    }
}

impl Drop for MockLifx {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("mock LIFX thread");
        }
    }
}

fn parse_lifx_header(packet: &[u8]) -> Option<(u32, [u8; 8], u8, u16)> {
    if packet.len() < LIFX_HEADER_LEN {
        return None;
    }
    let source = u32::from_le_bytes(packet.get(4..8)?.try_into().ok()?);
    let target = packet.get(8..16)?.try_into().ok()?;
    let sequence = *packet.get(23)?;
    let message_type = u16::from_le_bytes(packet.get(32..34)?.try_into().ok()?);
    Some((source, target, sequence, message_type))
}

fn lifx_packet(
    source: u32,
    target: [u8; 8],
    sequence: u8,
    message_type: u16,
    payload: &[u8],
) -> Vec<u8> {
    let length = LIFX_HEADER_LEN + payload.len();
    let mut packet = vec![0_u8; LIFX_HEADER_LEN];
    packet
        .get_mut(0..2)
        .expect("mock size field")
        .copy_from_slice(
            &u16::try_from(length)
                .expect("mock LIFX packet length")
                .to_le_bytes(),
        );
    *packet.get_mut(3).expect("mock frame field") = 0x14;
    packet
        .get_mut(4..8)
        .expect("mock source field")
        .copy_from_slice(&source.to_le_bytes());
    packet
        .get_mut(8..16)
        .expect("mock target field")
        .copy_from_slice(&target);
    *packet.get_mut(23).expect("mock sequence field") = sequence;
    packet
        .get_mut(32..34)
        .expect("mock message-type field")
        .copy_from_slice(&message_type.to_le_bytes());
    packet.extend_from_slice(payload);
    packet
}

#[test]
#[ignore = "spawns workspace binaries and requires the wled plugin artifact"]
fn wled_boots_with_an_empty_stable_topology() {
    let workspace = workspace_root();
    build_plugin_stack(&workspace, &["luminate-plugin-wled"]);
    let plugin = plugin_artifact(&workspace, "luminate-plugin-wled");

    let (mut session, ready) = HostSession::spawn(
        &luminated_binary(&workspace),
        &plugin,
        &empty_configuration(),
    )
    .expect("spawn wled plugin host");
    assert!(
        ready.descriptors.is_empty(),
        "no endpoints are configured, so WLED must not fabricate a device"
    );

    let descriptors = assert_topology_is_stable(&mut session);
    assert!(descriptors.is_empty());

    session.shutdown();
}

#[test]
#[ignore = "spawns workspace binaries and loopback mock WLED"]
fn wled_conforms_through_a_mock_http_controller() {
    let workspace = workspace_root();
    build_plugin_stack(&workspace, &["luminate-plugin-wled"]);
    let plugin = plugin_artifact(&workspace, "luminate-plugin-wled");
    let mock = MockWled::spawn();
    let configuration = serde_json::json!({
        "endpoints": [mock.address.to_string()],
        "mdns": false
    });

    let (mut session, _ready) =
        HostSession::spawn(&luminated_binary(&workspace), &plugin, &configuration)
            .expect("spawn WLED plugin host");
    let descriptors = await_device(&mut session, "wled-aabbccddeeff", READY_TIMEOUT);
    assert_eq!(descriptors.len(), 1, "mock publishes one WLED controller");
    let descriptors = run_shared_conformance_checks(&mut session);
    assert_frame_streaming_is_well_typed(
        &mut session,
        &descriptors[0].id,
        FrameEnvelope {
            generation: 1,
            sequence: 0,
            payload: FramePayload::Full(vec![Colour::rgb(Rgb::new(10, 20, 30)); 4]),
            commit: false,
        },
        FrameEnvelope {
            generation: 1,
            sequence: 1,
            payload: FramePayload::Full(vec![Colour::rgb(Rgb::new(1, 2, 3))]),
            commit: false,
        },
    );
    session.shutdown();

    let requests = mock.requests();
    for expected in [
        "GET /json/info HTTP/1.1",
        "GET /json/state HTTP/1.1",
        "GET /json/eff HTTP/1.1",
        "POST /json/state HTTP/1.1",
    ] {
        assert!(
            requests.iter().any(|request| request == expected),
            "mock WLED did not receive {expected}; requests: {requests:?}"
        );
    }
}

#[test]
#[ignore = "spawns workspace binaries and requires the lifx plugin artifact"]
fn lifx_boots_with_an_empty_stable_topology() {
    let workspace = workspace_root();
    build_plugin_stack(&workspace, &["luminate-plugin-lifx"]);
    let plugin = plugin_artifact(&workspace, "luminate-plugin-lifx");

    let (mut session, ready) = HostSession::spawn(
        &luminated_binary(&workspace),
        &plugin,
        &empty_configuration(),
    )
    .expect("spawn lifx plugin host");
    assert!(
        ready.descriptors.is_empty(),
        "no bulbs answer broadcast discovery in a hermetic sandbox"
    );

    let descriptors = assert_topology_is_stable(&mut session);
    assert!(descriptors.is_empty());

    session.shutdown();
}

#[test]
#[ignore = "spawns workspace binaries and a mock LIFX UDP peer"]
fn lifx_conforms_through_a_mock_udp_device() {
    let workspace = workspace_root();
    build_plugin_stack(&workspace, &["luminate-plugin-lifx"]);
    let plugin = plugin_artifact(&workspace, "luminate-plugin-lifx");
    let mock = MockLifx::spawn();
    let configuration = serde_json::json!({
        "discovery_address": mock.address.to_string()
    });

    let (mut session, _ready) =
        HostSession::spawn(&luminated_binary(&workspace), &plugin, &configuration)
            .expect("spawn LIFX plugin host");
    let descriptors = await_device(&mut session, LIFX_DEVICE_ID, READY_TIMEOUT);
    assert_eq!(descriptors.len(), 1, "mock publishes one LIFX bulb");
    run_shared_conformance_checks(&mut session);
    session.shutdown();

    let messages = mock.messages();
    for expected in [
        LIFX_GET_SERVICE,
        LIFX_GET_VERSION,
        LIFX_GET_LABEL,
        LIFX_GET_COLOR,
        LIFX_SET_POWER,
        LIFX_SET_COLOR,
    ] {
        assert!(
            messages.contains(&expected),
            "mock LIFX did not receive message type {expected}; messages: {messages:?}"
        );
    }
}

#[test]
#[ignore = "spawns workspace binaries and requires the govee plugin artifact"]
fn govee_boots_with_an_empty_stable_topology() {
    let workspace = workspace_root();
    build_plugin_stack(&workspace, &["luminate-plugin-govee"]);
    let plugin = plugin_artifact(&workspace, "luminate-plugin-govee");

    let (mut session, ready) = HostSession::spawn(
        &luminated_binary(&workspace),
        &plugin,
        &empty_configuration(),
    )
    .expect("spawn Govee plugin host");
    assert!(
        ready.descriptors.is_empty(),
        "no Govee scan reply should produce an empty initial topology"
    );
    let descriptors = assert_topology_is_stable(&mut session);
    assert!(descriptors.is_empty());

    session.shutdown();
}

// ---------------------------------------------------------------------
// Alienware: requires physical HID hardware to probe as ready
// ---------------------------------------------------------------------

/// Opt-in guard for the one Alienware test that issues real writes.
/// Alienware is real, attached HID hardware, not a fixture: on a machine
/// that has one (as the machine this suite was first written on did), an
/// unguarded representative-operation check would actually change the
/// keyboard's lighting. `#[ignore]` alone isn't enough of a gate for that,
/// since `--ignored` is exactly the flag someone runs to exercise this
/// whole suite; require this environment variable too.
const RUN_ALIENWARE_HARDWARE_WRITES_ENV: &str = "LUMINATE_CONFORMANCE_TOUCH_ALIENWARE_HARDWARE";

#[test]
#[ignore = "spawns workspace binaries and requires the alienware plugin artifact"]
fn alienware_boots_cleanly_with_or_without_hardware() {
    let workspace = workspace_root();
    build_plugin_stack(&workspace, &["luminate-plugin-alienware"]);
    let plugin = plugin_artifact(&workspace, "luminate-plugin-alienware");

    // Without a real Alienware HID device, probe() reports Unsupported and
    // the plugin host must fail bootstrap cleanly (never hang or crash). If
    // this machine happens to have one attached, the host instead reports
    // Ready with a real device; this test only reads its topology, never
    // writes to it (see `alienware_representative_operations_are_well_typed`
    // for the opt-in write-touching check).
    match HostSession::spawn(
        &luminated_binary(&workspace),
        &plugin,
        &empty_configuration(),
    ) {
        Ok((mut session, ready)) => {
            assert!(
                !ready.descriptors.is_empty(),
                "a Ready alienware host must have found at least one real device"
            );
            assert_topology_is_stable(&mut session);
            session.shutdown();
        }
        Err(error) => assert!(
            !error.is_empty(),
            "the plugin host must explain why it could not start"
        ),
    }
}

#[test]
#[ignore = "spawns workspace binaries and writes to real Alienware hardware if present; set \
            LUMINATE_CONFORMANCE_TOUCH_ALIENWARE_HARDWARE=1 to run"]
fn alienware_representative_operations_are_well_typed() {
    if env::var_os(RUN_ALIENWARE_HARDWARE_WRITES_ENV).is_none() {
        println!(
            "skipping: set {RUN_ALIENWARE_HARDWARE_WRITES_ENV}=1 to run write checks against \
             real Alienware hardware"
        );
        return;
    }

    let workspace = workspace_root();
    build_plugin_stack(&workspace, &["luminate-plugin-alienware"]);
    let plugin = plugin_artifact(&workspace, "luminate-plugin-alienware");

    let (mut session, ready) = HostSession::spawn(
        &luminated_binary(&workspace),
        &plugin,
        &empty_configuration(),
    )
    .expect("this check requires real Alienware hardware to be attached");

    assert_representative_operations_are_well_typed(&mut session, &ready.descriptors);
    if let Some(device) = ready.descriptors.first() {
        assert_batch_results_are_ordered(&mut session, &device.id);
    }

    session.shutdown();
}

// ---------------------------------------------------------------------
// Razer: requires physical HID hardware to probe as ready
// ---------------------------------------------------------------------

const RUN_RAZER_HARDWARE_WRITES_ENV: &str = "LUMINATE_CONFORMANCE_TOUCH_RAZER_HARDWARE";

#[test]
#[ignore = "spawns workspace binaries and requires the Razer plugin artifact"]
fn razer_boots_cleanly_with_or_without_hardware() {
    let workspace = workspace_root();
    build_plugin_stack(&workspace, &["luminate-plugin-razer"]);
    let plugin = plugin_artifact(&workspace, "luminate-plugin-razer");

    match HostSession::spawn(
        &luminated_binary(&workspace),
        &plugin,
        &empty_configuration(),
    ) {
        Ok((mut session, ready)) => {
            assert!(
                !ready.descriptors.is_empty(),
                "a Ready Razer host must have found the exact supported HID interface"
            );
            assert_topology_is_stable(&mut session);
            session.shutdown();
        }
        Err(error) => assert!(
            !error.is_empty(),
            "the Razer plugin host must explain why it could not start"
        ),
    }
}

#[test]
#[ignore = "spawns workspace binaries and writes to real Razer hardware if present; set \
            LUMINATE_CONFORMANCE_TOUCH_RAZER_HARDWARE=1 to run"]
fn razer_representative_operations_are_well_typed() {
    if env::var_os(RUN_RAZER_HARDWARE_WRITES_ENV).is_none() {
        println!(
            "skipping: set {RUN_RAZER_HARDWARE_WRITES_ENV}=1 to run write checks against \
             dedicated Razer test hardware"
        );
        return;
    }

    let workspace = workspace_root();
    build_plugin_stack(&workspace, &["luminate-plugin-razer"]);
    let plugin = plugin_artifact(&workspace, "luminate-plugin-razer");
    let (mut session, ready) = HostSession::spawn(
        &luminated_binary(&workspace),
        &plugin,
        &empty_configuration(),
    )
    .expect("this check requires the 1532:028d test keyboard to be attached");

    assert_representative_operations_are_well_typed(&mut session, &ready.descriptors);
    if let Some(device) = ready.descriptors.first() {
        if let Some(surface) = device.surfaces.first()
            && let Some(element) = surface.elements.first()
        {
            let target = PluginTarget::Element {
                device: device.id.clone(),
                surface: surface.id.clone(),
                element: element.id.clone(),
            };
            for operation in representative_operations(&element.capabilities) {
                let outcome = apply(&mut session, target.clone(), operation.clone());
                assert!(
                    !matches!(outcome, ApplyOutcome::Internal(_)),
                    "Razer element {} operation {} produced an internal error: {outcome:?}",
                    element.id,
                    operation.name()
                );
            }
            let operations = representative_operations(&element.capabilities);
            let updates = operations
                .into_iter()
                .map(|operation| PluginUpdate {
                    target: target.clone(),
                    operation,
                })
                .collect();
            let HostResponse::Batch(outcomes) = session
                .call(HostCommand::ApplyBatch(updates))
                .expect("Razer element batch call")
            else {
                panic!("Razer element batch returned the wrong response shape");
            };
            assert_eq!(outcomes.len(), 2);
            assert!(
                outcomes
                    .iter()
                    .all(|outcome| !matches!(outcome, ApplyOutcome::Internal(_))),
                "Razer element batch produced an internal error: {outcomes:?}"
            );
        }
        assert_batch_results_are_ordered(&mut session, &device.id);
    }
    session.shutdown();
}
