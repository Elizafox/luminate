// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Smoke tests for the generated C API surface and its ownership contracts.
//!
//! Every test here builds `libluminate` as a native shared library and links
//! a staged C/C++ consumer against it. The library is built with a versioned
//! runtime-load name baked in (see `build.rs`), so
//! `luminate_platform::test_support::ensure_dylib_runtime_link` is used to
//! create a matching same-directory link before linking a consumer against
//! it. See that function's docs for why. That mechanism (and this file's
//! use of a Unix-style `cc`/`-rpath` toolchain) has a real implementation on
//! Linux and macOS but no direct Windows equivalent, so the whole file
//! stays gated to those two for now.

#![cfg(any(target_os = "linux", target_os = "macos"))]
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::tests_outside_test_module,
    clippy::wildcard_enum_match_arm,
    reason = "Integration tests are crate roots and intentionally fail loudly when setup assumptions or mock protocol expectations break."
)]

use luminate_core::capability::{
    BrightnessCapability, BufferingMode, CapabilityScope, CapabilitySet, CctEmulation,
    ColourCapability, EffectChoice, EffectDirection, EffectParameter, FrameUpdateMode,
    FrameUploadCapability, HardwareEffectDescriptor, HardwareEffectId, HardwareEffectsCapability,
    PersistenceCapability, PersistenceRequirement, PhysicalPowerCapability, PowerDomainRef,
    ReadableFacet, ReadbackFidelity, ShmFrameCapability, ShmFrameShape, StateReadbackCapability,
};
use luminate_core::collection::{
    Collection, CollectionCategory, CollectionId, CollectionMember, OwnerIdentity,
};
use luminate_core::colour::Colour;
use luminate_core::control::{ReconciliationPolicy, UnsupportedPolicy};
use luminate_core::device::{Device, DeviceCategory, DeviceId};
use luminate_core::effect::Effect;
use luminate_core::element::{Element, ElementGeometry, ElementId, ElementKind};
use luminate_core::group::{Group, GroupId, GroupKind, GroupMember};
use luminate_core::rgb::Rgb;
use luminate_core::shm_frame::ShmPixelFormat;
use luminate_core::state::{
    AdoptionStatus, AppearanceState, DeviceStateStatus, EffectiveAppearanceState, EmissionState,
    FacetObservation, FacetValue, ObservationConfidence, ObservationSource, PhysicalPowerState,
    Reachability, ReconciliationStatus, StateFacetKind,
};
use luminate_core::surface::{Surface, SurfaceId, SurfaceKind};
use luminate_core::target::TargetId;
use luminate_core::util::DiscreteRange;
use luminate_platform::test_support::{TestDir, unique_runtime_dir};
use std::collections::BTreeMap;
use std::env;
use std::process;
use tokio::io::AsyncReadExt as _;
use tokio::net;
use tokio::runtime;

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::thread;

use luminate_core::policy::PrincipalId;
use luminate_platform::test_support::ensure_dylib_runtime_link;
use luminate_protocol::framing::{receive, send};
use luminate_protocol::{
    AuthenticationRequest, AuthenticationResponse, AuthenticationSource, ClientHello,
    Compatibility, DaemonHello, DaemonPreferences, ErrorCode, Event, EventCompatibility,
    EventTicket, ManagedPlugin, ManagementChange, ManagementChangeSet, ManagementSnapshot,
    OperationError, PluginRuntimeState, PluginSettingApplyMode, PluginSettingKind,
    PluginSettingSchema, PluginSetupSession, PluginSetupSessionId, PluginSetupSessionState,
    PluginSetupWorkflow, PluginSetupWorkflowKind, ReportedSettingValue, Request, RequestMessage,
    Response, ResponseMessage, ResponseStatus, ServerInfo, SettingValue, SubscribeAck,
    SubscribeHello,
};

macro_rules! declare_c_test {
    ($name:ident, $source:expr) => {
        #[test]
        fn $name() {
            let socket_path = unique_path(concat!(stringify!($name), "-sock"));
            let server = spawn_mock_daemon(&socket_path);
            run_c_consumer(stringify!($name), $source, &[socket_path.as_os_str()]);
            server.join().expect("mock daemon thread should not panic");
            let _ = fs::remove_file(socket_path);
        }
    };
    ($name:ident, $source:expr, with_events) => {
        #[test]
        fn $name() {
            let socket_path = unique_path(concat!(stringify!($name), "-sock"));
            let event_path = luminate_platform::default_path::event_socket_path(&socket_path);
            let server = spawn_mock_daemon(&socket_path);
            let event_server = spawn_mock_event_daemon(&event_path);
            run_c_consumer(stringify!($name), $source, &[socket_path.as_os_str()]);
            event_server
                .join()
                .expect("mock event daemon thread should not panic");
            server.join().expect("mock daemon thread should not panic");
            let _ = fs::remove_file(socket_path);
            let _ = fs::remove_file(event_path);
        }
    };
    ($name:ident, $source:expr, with_stalling_events) => {
        #[test]
        fn $name() {
            let socket_path = unique_path(concat!(stringify!($name), "-sock"));
            let event_path = luminate_platform::default_path::event_socket_path(&socket_path);
            let server = spawn_mock_daemon(&socket_path);
            let event_server = spawn_stalling_mock_event_daemon(&event_path);
            run_c_consumer(stringify!($name), $source, &[socket_path.as_os_str()]);
            event_server
                .join()
                .expect("stalling event daemon thread should not panic");
            server.join().expect("mock daemon thread should not panic");
            let _ = fs::remove_file(socket_path);
            let _ = fs::remove_file(event_path);
        }
    };
    ($name:ident, $source:expr, with_explicit_events) => {
        #[test]
        fn $name() {
            let socket_path = unique_path(concat!(stringify!($name), "-sock"));
            let event_path = unique_path(concat!(stringify!($name), "-events"));
            let server = spawn_mock_daemon(&socket_path);
            let event_server = spawn_mock_event_daemon(&event_path);
            run_c_consumer(
                stringify!($name),
                $source,
                &[socket_path.as_os_str(), event_path.as_os_str()],
            );
            event_server
                .join()
                .expect("mock event daemon thread should not panic");
            server.join().expect("mock daemon thread should not panic");
            let _ = fs::remove_file(socket_path);
            let _ = fs::remove_file(event_path);
        }
    };
    ($name:ident, $source:expr, without_daemon) => {
        #[test]
        fn $name() {
            let missing_path = unique_path(concat!(stringify!($name), "-missing"));
            run_c_consumer(stringify!($name), $source, &[missing_path.as_os_str()]);
        }
    };
    ($name:ident, $source:expr, with_incompatible_sockets) => {
        #[test]
        fn $name() {
            let incompatible_path = unique_path(concat!(stringify!($name), "-incompatible"));
            let socket_path = unique_path(concat!(stringify!($name), "-sock"));
            let event_path = unique_path(concat!(stringify!($name), "-events"));
            let incompatible_server = spawn_incompatible_mock_daemon(&incompatible_path);
            let server = spawn_mock_daemon(&socket_path);
            let event_server = spawn_incompatible_mock_event_daemon(&event_path);
            run_c_consumer(
                stringify!($name),
                $source,
                &[
                    incompatible_path.as_os_str(),
                    socket_path.as_os_str(),
                    event_path.as_os_str(),
                ],
            );
            event_server
                .join()
                .expect("incompatible event daemon thread should not panic");
            incompatible_server
                .join()
                .expect("incompatible mock daemon thread should not panic");
            server.join().expect("mock daemon thread should not panic");
            let _ = fs::remove_file(incompatible_path);
            let _ = fs::remove_file(socket_path);
            let _ = fs::remove_file(event_path);
        }
    };
    ($name:ident, $source:expr, with_disconnect) => {
        #[test]
        fn $name() {
            let socket_path = unique_path(concat!(stringify!($name), "-sock"));
            let server = spawn_disconnecting_mock_daemon(&socket_path);
            run_c_consumer(stringify!($name), $source, &[socket_path.as_os_str()]);
            server
                .join()
                .expect("disconnecting mock daemon thread should not panic");
            let _ = fs::remove_file(socket_path);
        }
    };
}

macro_rules! declare_staged_c_test {
    ($name:ident, $source:expr) => {
        #[test]
        fn $name() {
            run_staged_c_consumer(stringify!($name), $source);
        }
    };
}

#[path = "c_api_smoke/async_tests.rs"]
mod async_tests;
#[path = "c_api_smoke/compilation_tests.rs"]
mod compilation_tests;
#[path = "c_api_smoke/model_tests.rs"]
mod model_tests;
#[path = "c_api_smoke/protocol_tests.rs"]
mod protocol_tests;

/// Which compiler a smoke test needs, independent of what its `CC`/`CXX`
/// override happens to be named.
#[derive(Clone, Copy)]
enum Language {
    C,
    Cxx,
}

/// The compiler to invoke for `language`, honouring `CC`/`CXX` from the
/// environment (mirroring `build_libluminate`'s `CARGO` lookup) and falling
/// back to the conventional `cc`/`c++` names.
fn compiler_for(language: Language) -> OsString {
    let env_var = match language {
        Language::C => "CC",
        Language::Cxx => "CXX",
    };
    env::var_os(env_var).unwrap_or_else(|| match language {
        Language::C => "cc".into(),
        Language::Cxx => "c++".into(),
    })
}

/// The newest C standard the configured C compiler can build, preferring
/// `c23` and falling back to `c11` (the baseline the rest of this file
/// targets) on older toolchains that don't recognise `-std=c23` yet.
///
/// This only affects the strict-C23 header smoke test: everything else here
/// targets C11 unconditionally, since that's the supported floor for
/// consumers of the generated header.
fn best_available_c_standard() -> &'static str {
    static STANDARD: OnceLock<&'static str> = OnceLock::new();
    STANDARD.get_or_init(|| {
        let probe_dir = TestDir::new("c23-probe-dir");
        let source_path = probe_dir.join("probe.c");
        let binary_path = probe_dir.join("probe");
        fs::write(&source_path, "int main(void) { return 0; }\n").expect("write C23 probe source");
        let supports_c23 = Command::new(compiler_for(Language::C))
            .arg("-std=c23")
            .arg(&source_path)
            .arg("-o")
            .arg(&binary_path)
            .output()
            .is_ok_and(|output| output.status.success());
        if supports_c23 { "c23" } else { "c11" }
    })
}

fn run_header_consumer(name: &str, source: &str, language: Language, standard: &str) {
    let temp_dir = TestDir::new(&format!("{name}-dir"));
    let extension = match language {
        Language::C => "c",
        Language::Cxx => "cpp",
    };
    let source_path = temp_dir.join(format!("{name}.{extension}"));
    let binary_path = temp_dir.join(name);
    fs::write(&source_path, source).expect("write header smoke source");
    build_libluminate();
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_debug = target_debug_dir();
    let compile = Command::new(compiler_for(language))
        .arg(format!("-std={standard}"))
        .arg("-pedantic-errors")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("-pthread")
        .arg("-I")
        .arg(&manifest_dir)
        .arg(&source_path)
        .arg("-L")
        .arg(&target_debug)
        .arg(format!("-Wl,-rpath,{}", target_debug.display()))
        .arg("-lluminate")
        .arg("-o")
        .arg(&binary_path)
        .output()
        .expect("compile header smoke consumer");
    assert_command_succeeded("header smoke compile", &compile);
    let run = Command::new(&binary_path)
        .output()
        .expect("run header smoke consumer");
    assert_command_succeeded("header smoke consumer", &run);
}

fn run_c_consumer(name: &str, source: &str, args: &[&OsStr]) {
    let temp_dir = TestDir::new(&format!("{name}-dir"));

    let source_path = temp_dir.join(format!("c_api_{name}.c"));
    let binary_path = temp_dir.join(format!("c_api_{name}"));
    fs::write(&source_path, source).expect("write C smoke source");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_debug = target_debug_dir();
    build_libluminate();

    let compile = Command::new(compiler_for(Language::C))
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("-I")
        .arg(&manifest_dir)
        .arg(&source_path)
        .arg("-L")
        .arg(&target_debug)
        .arg(format!("-Wl,-rpath,{}", target_debug.display()))
        .arg("-lluminate")
        .arg("-o")
        .arg(&binary_path)
        .output()
        .expect("run C compiler");
    assert_command_succeeded("C smoke compile", &compile);

    let run = Command::new(&binary_path)
        .args(args)
        .output()
        .expect("run C smoke binary");
    assert_command_succeeded("C smoke binary", &run);
}

fn run_cpp_consumer(name: &str, source: &str, args: &[&OsStr]) {
    let temp_dir = TestDir::new(&format!("{name}-dir"));
    let source_path = temp_dir.join(format!("c_api_{name}.cpp"));
    let binary_path = temp_dir.join(format!("c_api_{name}"));
    fs::write(&source_path, source).expect("write C++ smoke source");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_debug = target_debug_dir();
    build_libluminate();
    let compile = Command::new(compiler_for(Language::Cxx))
        .arg("-std=c++17")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("-I")
        .arg(&manifest_dir)
        .arg(&source_path)
        .arg("-L")
        .arg(&target_debug)
        .arg(format!("-Wl,-rpath,{}", target_debug.display()))
        .arg("-lluminate")
        .arg("-o")
        .arg(&binary_path)
        .output()
        .expect("run C++ compiler");
    assert_command_succeeded("C++ smoke compile", &compile);
    let run = Command::new(&binary_path)
        .args(args)
        .output()
        .expect("run C++ smoke binary");
    assert_command_succeeded("C++ smoke binary", &run);
}

fn run_staged_c_consumer(name: &str, source: &str) {
    let temp_dir = TestDir::new(&format!("{name}-dir"));
    let include_dir = temp_dir.join("include/luminate");
    let library_dir = temp_dir.join("lib");
    fs::create_dir_all(&include_dir).expect("create staged include dir");
    fs::create_dir_all(&library_dir).expect("create staged library dir");

    build_libluminate();

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_debug = target_debug_dir();
    fs::copy(
        manifest_dir.join("luminate.h"),
        include_dir.join("luminate.h"),
    )
    .expect("stage public C header");
    let dylib_file_name = dylib_file_name();
    fs::copy(
        target_debug.join(&dylib_file_name),
        library_dir.join(&dylib_file_name),
    )
    .expect("stage libluminate shared library");
    ensure_dylib_runtime_link(&library_dir.join(&dylib_file_name));

    let source_path = temp_dir.join(format!("c_api_{name}.c"));
    let binary_path = temp_dir.join(format!("c_api_{name}"));
    fs::write(&source_path, source).expect("write staged C consumer source");

    let compile = Command::new(compiler_for(Language::C))
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("-I")
        .arg(&include_dir)
        .arg(&source_path)
        .arg("-L")
        .arg(&library_dir)
        .arg(format!("-Wl,-rpath,{}", library_dir.display()))
        .arg("-lluminate")
        .arg("-o")
        .arg(&binary_path)
        .output()
        .expect("compile staged C consumer");
    assert_command_succeeded("staged C consumer compile", &compile);

    let run = Command::new(&binary_path)
        .output()
        .expect("run staged C consumer");
    assert_command_succeeded("staged C consumer", &run);
}

/// Builds and links `libluminate`'s cdylib exactly once per test binary run.
///
/// Every test calls this before staging a C consumer, and `cargo test` runs
/// them concurrently on multiple threads. Without serialising, concurrent
/// `cargo build` invocations each race to replace `target/debug/libluminate`
/// via rename, so another thread's `ensure_dylib_runtime_link` can observe
/// the file mid-replacement and fail to find it. `OnceLock::get_or_init`
/// blocks every caller but the first until the build and link both finish.
fn build_libluminate() {
    static BUILT: OnceLock<()> = OnceLock::new();
    BUILT.get_or_init(|| {
        let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let target_dir = target_debug_dir()
            .parent()
            .expect("target debug directory has a target root")
            .to_path_buf();
        let build = Command::new(cargo)
            .arg("build")
            .arg("-p")
            .arg("libluminate")
            .arg("--target-dir")
            .arg(target_dir)
            .output()
            .expect("run cargo build for libluminate cdylib");
        assert_command_succeeded("libluminate cdylib build", &build);
        ensure_dylib_runtime_link(&target_debug_dir().join(dylib_file_name()));
    });
}

/// The filename Cargo gives `libluminate`'s cdylib output on this platform
/// (`libluminate.so`, `libluminate.dylib`, ...).
fn dylib_file_name() -> String {
    luminate_platform::dynamic_library_candidates("luminate")[0].clone()
}

fn assert_command_succeeded(context: &str, output: &process::Output) {
    assert!(
        output.status.success(),
        "{context} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn target_debug_dir() -> PathBuf {
    let exe = env::current_exe().expect("test executable path");
    let deps_dir = exe.parent().expect("test executable directory");
    deps_dir
        .parent()
        .expect("target debug directory")
        .to_path_buf()
}

fn unique_path(kind: &str) -> PathBuf {
    unique_runtime_dir(kind)
}

#[allow(
    clippy::too_many_lines,
    reason = "One exhaustive fixture keeps the C model-walk test internally consistent."
)]
fn typed_fixture() -> (Device, DeviceStateStatus) {
    let target = TargetId::device("fixture-device");
    let capabilities = CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 7,
            maximum: 100,
            scope: CapabilityScope::Device,
        },
        frame_upload: Some(FrameUploadCapability {
            scope: CapabilityScope::Surface,
            update_mode: FrameUpdateMode::Both,
            max_rate_hz: Some(60),
            atomic: true,
            buffering: BufferingMode::DoubleBuffered,
            shm: Some(ShmFrameCapability {
                pixel_formats: vec![ShmPixelFormat::Rgb8, ShmPixelFormat::Rgbx8],
                shape: ShmFrameShape::Matrix {
                    width: 3,
                    height: 2,
                },
                max_rate_hz: Some(120),
            }),
        }),
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![HardwareEffectDescriptor {
                id: HardwareEffectId::new("scene"),
                name: "Scene".to_owned(),
                parameters: vec![
                    EffectParameter::Colour {
                        minimum_colours: 1,
                        maximum_colours: 2,
                    },
                    EffectParameter::Speed {
                        range: DiscreteRange::new(1, 10, 1),
                    },
                    EffectParameter::Direction {
                        values: vec![EffectDirection::Forward, EffectDirection::Reverse],
                    },
                    EffectParameter::Duration {
                        milliseconds: DiscreteRange::new(100, 1_000, 100),
                    },
                    EffectParameter::Brightness { bits: 8 },
                    EffectParameter::Choice {
                        options: vec![EffectChoice {
                            id: "calm".to_owned(),
                            name: "Calm".to_owned(),
                        }],
                    },
                ],
            }],
            scope: CapabilityScope::Device,
            concurrent_with_streaming: false,
        }),
        appearance_slots: None,
        persistence: PersistenceCapability::Profiles {
            requirement: PersistenceRequirement::Optional,
            slots: 2,
            explicit_commit: true,
            readback: true,
        },
        state_readback: StateReadbackCapability::Readable {
            facets: vec![ReadableFacet {
                facet: StateFacetKind::Appearance,
                fidelity: ReadbackFidelity::Exact,
            }],
            read_disturbs_output: false,
            notifies_external_changes: true,
        },
        emission: true,
        off_is_wear_safe: false,
        physical_power: Some(PhysicalPowerCapability {
            scope: CapabilityScope::Device,
        }),
        power_domain: Some(PowerDomainRef::Device),
    };
    let device = Device {
        id: DeviceId::new("fixture-device"),
        name: "Fixture".to_owned(),
        vendor: Some("Luminate".to_owned()),
        model: None,
        provider_instance: None,
        surfaces: vec![Surface {
            id: SurfaceId::new("panel"),
            name: "Panel".to_owned(),
            kind: SurfaceKind::Matrix { rows: 2, cols: 3 },
            physical_tags: vec!["layout:grid".to_owned(), "position:front".to_owned()],
            elements: vec![Element {
                id: ElementId::new("pixel"),
                name: Some("Pixel".to_owned()),
                kind: ElementKind::Led,
                geometry: Some(ElementGeometry::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 0.5,
                    h: 0.5,
                }),
                physical_tags: vec![
                    "shape:rectangular".to_owned(),
                    "position:top-left".to_owned(),
                ],
                capabilities: CapabilitySet::default(),
                notes: vec!["element note".to_owned()],
                warnings: vec!["element warning".to_owned()],
            }],
            capabilities: CapabilitySet::default(),
            notes: vec!["surface note".to_owned()],
            warnings: vec!["surface warning".to_owned()],
        }],
        groups: vec![Group {
            id: GroupId::new("all"),
            name: "All".to_owned(),
            description: Some("fixture group".to_owned()),
            kind: GroupKind::Application,
            members: vec![GroupMember::Element {
                surface: SurfaceId::new("panel"),
                element: ElementId::new("pixel"),
            }],
            capabilities: CapabilitySet::default(),
            notes: vec!["group note".to_owned()],
            warnings: vec!["group warning".to_owned()],
        }],
        capabilities,
        category: Some(DeviceCategory::new("fixture-kind")),
        physical_tags: vec!["shape:modular-light-bar".to_owned()],
        host_attached: false,
        notes: vec!["note".to_owned()],
        warnings: vec!["warning".to_owned()],
    };
    let observation = |value, observed_at_ms| FacetObservation {
        target: target.clone(),
        value,
        confidence: ObservationConfidence::Confirmed,
        source: ObservationSource::Readback,
        observed_at_ms,
        stale: false,
    };
    let state = DeviceStateStatus {
        device: DeviceId::new("fixture-device"),
        observations: vec![
            observation(
                FacetValue::Appearance(AppearanceState::Static(Colour::rgb(Rgb::new(1, 2, 3)))),
                1,
            ),
            observation(FacetValue::Brightness(50), 2),
            observation(FacetValue::Emission(EmissionState::Emitting), 3),
            observation(FacetValue::PhysicalPower(PhysicalPowerState::On), 4),
            observation(
                FacetValue::EffectiveAppearance(EffectiveAppearanceState::Effect(
                    Effect::Breathe {
                        colour: Rgb::new(7, 8, 9),
                        period_ms: 250,
                    },
                )),
                5,
            ),
        ],
        reachability: Reachability::Reachable,
        reconciliation: ReconciliationStatus::Complete,
        adoption: vec![(target, StateFacetKind::Appearance, AdoptionStatus::Durable)],
        latest_error: Some("last failure".to_owned()),
        latest_attempt_ms: Some(99),
    };
    (device, state)
}

#[allow(
    clippy::too_many_lines,
    reason = "the exhaustive mock request router keeps fixture responses explicit"
)]
fn spawn_typed_model_daemon(socket_path: &Path) -> thread::JoinHandle<()> {
    let socket_path = socket_path.to_path_buf();
    let (ready_tx, ready_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let runtime = runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build typed model daemon runtime");
        runtime.block_on(async move {
            let listener = net::UnixListener::bind(&socket_path).expect("bind typed model socket");
            ready_tx.send(()).expect("send typed model ready signal");
            let (mut stream, _) = listener.accept().await.expect("accept typed model client");
            let _: ClientHello = receive(&mut stream).await.expect("receive client hello");
            send(
                &mut stream,
                &DaemonHello {
                    compatibility: Compatibility::Compatible,
                    protocol_abi_version: luminate_protocol::PROTOCOL_ABI_VERSION,
                    daemon_version: "typed-model-daemon".to_owned(),
                },
            )
            .await
            .expect("send typed model hello");
            authenticate_client(&mut stream).await;
            let (device, state) = typed_fixture();
            while let Ok(message) = receive::<RequestMessage>(&mut stream).await {
                let status = match message.request {
                    Request::IssueEventTicket => ResponseStatus::EventTicket(
                        EventTicket::new([4_u8; 32]).expect("event ticket"),
                    ),
                    Request::ListDevices => ResponseStatus::Devices(vec![device.clone()]),
                    Request::ListWithdrawnDevices => {
                        ResponseStatus::WithdrawnDevices(vec![DeviceId::new("retired-device")])
                    }
                    Request::GetDevice { .. } => {
                        ResponseStatus::Device(Some(Box::new(device.clone())))
                    }
                    Request::GetState { .. } => {
                        ResponseStatus::State(Some(Box::new(state.clone())))
                    }
                    Request::GetCollectionState { .. } => ResponseStatus::CollectionState(None),
                    Request::ServerInfo => ResponseStatus::ServerInfo(ServerInfo {
                        daemon_name: "typed-model-daemon".to_owned(),
                        daemon_version: "0.1".to_owned(),
                        protocol_abi_version: luminate_protocol::PROTOCOL_ABI_VERSION,
                    }),
                    Request::Ping
                    | Request::SetEffect(_)
                    | Request::SetAppearanceSlots(_)
                    | Request::SetBrightness(_)
                    | Request::RestoreAppearance { .. }
                    | Request::ClearTarget { .. }
                    | Request::SaveCurrent { .. }
                    | Request::RefreshState { .. }
                    | Request::PurgeWithdrawnDevice { .. }
                    | Request::UnloadPlugin { .. }
                    | Request::ReloadPlugin { .. }
                    | Request::Rescan
                    | Request::DestroyCollection { .. }
                    | Request::AddCollectionMember { .. }
                    | Request::RemoveCollectionMember { .. }
                    | Request::EndFrameStream { .. }
                    | Request::EndShmFrameStream { .. } => ResponseStatus::Ack,
                    Request::GetManagement => {
                        ResponseStatus::ManagementSnapshot(Box::new(mock_management_snapshot()))
                    }
                    Request::PatchManagement { .. } => {
                        ResponseStatus::ManagementPatched(mock_management_changes())
                    }
                    Request::ListPluginSetupWorkflows { plugin } => {
                        ResponseStatus::PluginSetupWorkflows(vec![PluginSetupWorkflow::new(
                            plugin,
                            "pair",
                            "Pair hardware",
                            "Connect nearby hardware.",
                            PluginSetupWorkflowKind::Provision,
                        )])
                    }
                    Request::StartPluginSetup { plugin, workflow } => {
                        ResponseStatus::PluginSetupSession(Box::new(mock_setup_session(
                            plugin,
                            workflow,
                            1,
                            PluginSetupSessionState::PhysicalAction {
                                instruction: "Press the button.".to_owned(),
                            },
                        )))
                    }
                    Request::RespondPluginSetup { .. } => {
                        ResponseStatus::PluginSetupSession(Box::new(mock_setup_session(
                            "example".to_owned(),
                            "pair".to_owned(),
                            2,
                            PluginSetupSessionState::Completed {
                                summary: "Connected.".to_owned(),
                                revision: 7,
                            },
                        )))
                    }
                    Request::GetPluginSetup { .. } => {
                        ResponseStatus::PluginSetupSession(Box::new(mock_setup_session(
                            "example".to_owned(),
                            "pair".to_owned(),
                            1,
                            PluginSetupSessionState::PhysicalAction {
                                instruction: "Press the button.".to_owned(),
                            },
                        )))
                    }
                    Request::CancelPluginSetup { .. } => {
                        ResponseStatus::PluginSetupSession(Box::new(mock_setup_session(
                            "example".to_owned(),
                            "pair".to_owned(),
                            2,
                            PluginSetupSessionState::Cancelled,
                        )))
                    }
                    Request::CreateCollection { .. } => ResponseStatus::CollectionCreated {
                        id: CollectionId::new("mock-collection"),
                    },
                    Request::ListCollections => ResponseStatus::Collections(Vec::new()),
                    Request::GetCollection { .. } => ResponseStatus::CollectionInfo(None),
                    Request::BeginFrameStream { .. } => {
                        ResponseStatus::FrameStreamStarted { generation: 1 }
                    }
                    Request::UploadFrame { envelope, .. } => ResponseStatus::FrameAck {
                        sequence: envelope.sequence,
                        dropped: false,
                    },
                    Request::BeginShmFrameStream { .. } => ResponseStatus::Error(OperationError {
                        code: ErrorCode::Unsupported,
                        message: "mock daemon does not support shared-memory streaming".to_owned(),
                        retry_after_ms: None,
                        applied_targets: Vec::new(),
                    }),
                    Request::GetAccessPolicy
                    | Request::ReplaceAccessPolicy { .. }
                    | Request::CreateToken { .. }
                    | Request::ListTokens
                    | Request::RotateToken { .. }
                    | Request::RevokeToken { .. }
                    | Request::CreateScene { .. }
                    | Request::CaptureScene { .. }
                    | Request::ReplaceScene { .. }
                    | Request::RecaptureScene { .. }
                    | Request::DeleteScene { .. }
                    | Request::ListScenes
                    | Request::GetScene { .. }
                    | Request::ApplyScene { .. }
                    | Request::StartTransition(_)
                    | Request::GetTransition { .. }
                    | Request::AbortTransition { .. }
                    | Request::RenewTransition { .. }
                    | Request::CreateAttestation { .. }
                    | Request::ListAttestations
                    | Request::RevokeAttestation { .. } => ResponseStatus::Error(OperationError {
                        code: ErrorCode::Unsupported,
                        message: "scene request is outside this fixture".to_owned(),
                        retry_after_ms: None,
                        applied_targets: Vec::new(),
                    }),
                };
                send(
                    &mut stream,
                    &ResponseMessage {
                        id: message.id,
                        response: Response { status },
                    },
                )
                .await
                .expect("send typed model response");
            }
        });
    });
    ready_rx
        .recv()
        .expect("typed model daemon should become ready");
    handle
}

fn collection_fixture() -> Collection {
    Collection {
        id: CollectionId::new("living-room"),
        name: "Living Room".to_owned(),
        description: Some("Downstairs lighting".to_owned()),
        owner: OwnerIdentity::Uid(1000),
        kind: Some(CollectionCategory::new("location")),
        members: vec![
            CollectionMember::Target(TargetId::surface("lamp", "shade")),
            CollectionMember::Collection(CollectionId::new("nook")),
        ],
    }
}

fn spawn_collection_model_daemon(socket_path: &Path) -> thread::JoinHandle<()> {
    let socket_path = socket_path.to_path_buf();
    let (ready_tx, ready_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let runtime = runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build collection model daemon runtime");
        runtime.block_on(async move {
            let listener =
                net::UnixListener::bind(&socket_path).expect("bind collection model socket");
            ready_tx
                .send(())
                .expect("send collection model ready signal");
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept collection model client");
            let _: ClientHello = receive(&mut stream)
                .await
                .expect("receive collection client hello");
            send(
                &mut stream,
                &DaemonHello {
                    compatibility: Compatibility::Compatible,
                    protocol_abi_version: luminate_protocol::PROTOCOL_ABI_VERSION,
                    daemon_version: "collection-model-daemon".to_owned(),
                },
            )
            .await
            .expect("send collection daemon hello");
            authenticate_client(&mut stream).await;

            while let Ok(message) = receive::<RequestMessage>(&mut stream).await {
                let status = match message.request {
                    Request::CreateCollection { .. } => ResponseStatus::CollectionCreated {
                        id: CollectionId::new("living-room"),
                    },
                    Request::AddCollectionMember { .. }
                    | Request::RemoveCollectionMember { .. }
                    | Request::DestroyCollection { .. } => ResponseStatus::Ack,
                    Request::ListCollections => {
                        ResponseStatus::Collections(vec![collection_fixture()])
                    }
                    Request::GetCollection { .. } => {
                        ResponseStatus::CollectionInfo(Some(Box::new(collection_fixture())))
                    }
                    request => panic!("unexpected collection model request: {request:?}"),
                };
                send(
                    &mut stream,
                    &ResponseMessage {
                        id: message.id,
                        response: Response { status },
                    },
                )
                .await
                .expect("send collection model response");
            }
        });
    });
    ready_rx
        .recv()
        .expect("collection model daemon should become ready");
    handle
}

fn mock_management_snapshot() -> ManagementSnapshot {
    let preferences = DaemonPreferences {
        default_unsupported_policy: Some(UnsupportedPolicy::Reject),
        reconciliation_policy: Some(ReconciliationPolicy::Restore),
        device_reconciliation: vec![luminate_protocol::DeviceReconciliationPreference {
            device: DeviceId::new("fixture-device"),
            policy: ReconciliationPolicy::Adopt,
        }],
        cct_emulation: Some(CctEmulation::Disabled),
        prefer_shm: Some(true),
        prefer_client_shm: Some(false),
    };
    ManagementSnapshot {
        revision: 7,
        desired_daemon: preferences.clone(),
        effective_daemon: preferences,
        locked_daemon_settings: vec!["reconciliation-policy".to_owned()],
        plugins: vec![ManagedPlugin {
            name: "fixture".to_owned(),
            version: "1.2.3".to_owned(),
            required: false,
            desired_enabled: Some(true),
            desired_reconciliation: None,
            effective_reconciliation: None,
            effective_enabled: true,
            runtime: PluginRuntimeState::Loaded,
            activation_locked: false,
            schema: vec![PluginSettingSchema {
                key: "mode".to_owned(),
                label: "Mode".to_owned(),
                description: "Fixture mode".to_owned(),
                kind: PluginSettingKind::String,
                default: ReportedSettingValue::Visible(SettingValue::String("calm".to_owned())),
                required: true,
                sensitive: false,
                apply_mode: PluginSettingApplyMode::RestartRequired,
                minimum: None,
                maximum: None,
                constraints: ReportedSettingValue::Visible(SettingValue::Array(vec![
                    SettingValue::String("calm".to_owned()),
                    SettingValue::String("party".to_owned()),
                ])),
            }],
            desired_settings: BTreeMap::from([(
                "token".to_owned(),
                ReportedSettingValue::Redacted,
            )]),
            effective_settings: BTreeMap::from([(
                "mode".to_owned(),
                ReportedSettingValue::Visible(SettingValue::String("calm".to_owned())),
            )]),
            locked_settings: vec!["mode".to_owned()],
        }],
    }
}

fn mock_management_changes() -> ManagementChangeSet {
    ManagementChangeSet {
        revision: 8,
        changes: vec![ManagementChange::PluginActivationChanged {
            plugin: "fixture".to_owned(),
        }],
    }
}

fn mock_setup_session(
    plugin: String,
    workflow: String,
    generation: u64,
    state: PluginSetupSessionState,
) -> PluginSetupSession {
    PluginSetupSession {
        id: PluginSetupSessionId::parse("0123456789abcdef0123456789abcdef")
            .expect("valid mock setup session ID"),
        plugin,
        workflow,
        generation,
        state,
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the exhaustive mock protocol responder keeps every response fixture visible"
)]
fn spawn_mock_daemon(socket_path: &Path) -> thread::JoinHandle<()> {
    let socket_path = socket_path.to_path_buf();
    let (ready_tx, ready_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let runtime = runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build mock daemon runtime");

        runtime.block_on(async move {
            let listener = net::UnixListener::bind(&socket_path).expect("bind mock daemon socket");
            ready_tx.send(()).expect("send mock daemon ready signal");
            let (mut stream, _) = listener.accept().await.expect("accept C client");

            let _: ClientHello = receive(&mut stream).await.expect("receive client hello");
            send(
                &mut stream,
                &DaemonHello {
                    compatibility: Compatibility::Compatible,
                    protocol_abi_version: luminate_protocol::PROTOCOL_ABI_VERSION,
                    daemon_version: "mock-daemon-0.1".to_owned(),
                },
            )
            .await
            .expect("send daemon hello");
            authenticate_client(&mut stream).await;

            while let Ok(message) = receive::<RequestMessage>(&mut stream).await {
                let status = match message.request {
                    Request::IssueEventTicket => ResponseStatus::EventTicket(
                        EventTicket::new([4_u8; 32]).expect("event ticket"),
                    ),
                    Request::ServerInfo => ResponseStatus::ServerInfo(ServerInfo {
                        daemon_name: "luminated-mock".to_owned(),
                        daemon_version: "mock-daemon-0.1".to_owned(),
                        protocol_abi_version: luminate_protocol::PROTOCOL_ABI_VERSION,
                    }),
                    Request::ListDevices => ResponseStatus::Devices(Vec::new()),
                    Request::ListWithdrawnDevices => {
                        ResponseStatus::WithdrawnDevices(vec![DeviceId::new("retired-device")])
                    }
                    Request::GetDevice { .. } => ResponseStatus::Device(None),
                    Request::GetState { .. } => ResponseStatus::State(None),
                    Request::GetCollectionState { .. } => ResponseStatus::CollectionState(None),
                    Request::Ping
                    | Request::SetEffect(_)
                    | Request::SetAppearanceSlots(_)
                    | Request::SetBrightness(_)
                    | Request::RestoreAppearance { .. }
                    | Request::ClearTarget { .. }
                    | Request::SaveCurrent { .. }
                    | Request::RefreshState { .. }
                    | Request::PurgeWithdrawnDevice { .. }
                    | Request::UnloadPlugin { .. }
                    | Request::ReloadPlugin { .. }
                    | Request::Rescan
                    | Request::DestroyCollection { .. }
                    | Request::AddCollectionMember { .. }
                    | Request::RemoveCollectionMember { .. }
                    | Request::EndFrameStream { .. }
                    | Request::EndShmFrameStream { .. } => ResponseStatus::Ack,
                    Request::GetManagement => {
                        ResponseStatus::ManagementSnapshot(Box::new(mock_management_snapshot()))
                    }
                    Request::PatchManagement { .. } => {
                        ResponseStatus::ManagementPatched(mock_management_changes())
                    }
                    Request::ListPluginSetupWorkflows { plugin } => {
                        ResponseStatus::PluginSetupWorkflows(vec![PluginSetupWorkflow::new(
                            plugin,
                            "pair",
                            "Pair hardware",
                            "Connect nearby hardware.",
                            PluginSetupWorkflowKind::Provision,
                        )])
                    }
                    Request::StartPluginSetup { plugin, workflow } => {
                        ResponseStatus::PluginSetupSession(Box::new(mock_setup_session(
                            plugin,
                            workflow,
                            1,
                            PluginSetupSessionState::PhysicalAction {
                                instruction: "Press the button.".to_owned(),
                            },
                        )))
                    }
                    Request::RespondPluginSetup { .. } => {
                        ResponseStatus::PluginSetupSession(Box::new(mock_setup_session(
                            "example".to_owned(),
                            "pair".to_owned(),
                            2,
                            PluginSetupSessionState::Completed {
                                summary: "Connected.".to_owned(),
                                revision: 7,
                            },
                        )))
                    }
                    Request::GetPluginSetup { .. } => {
                        ResponseStatus::PluginSetupSession(Box::new(mock_setup_session(
                            "example".to_owned(),
                            "pair".to_owned(),
                            1,
                            PluginSetupSessionState::PhysicalAction {
                                instruction: "Press the button.".to_owned(),
                            },
                        )))
                    }
                    Request::CancelPluginSetup { .. } => {
                        ResponseStatus::PluginSetupSession(Box::new(mock_setup_session(
                            "example".to_owned(),
                            "pair".to_owned(),
                            2,
                            PluginSetupSessionState::Cancelled,
                        )))
                    }
                    Request::CreateCollection { .. } => ResponseStatus::CollectionCreated {
                        id: CollectionId::new("mock-collection"),
                    },
                    Request::ListCollections => ResponseStatus::Collections(Vec::new()),
                    Request::GetCollection { .. } => ResponseStatus::CollectionInfo(None),
                    Request::BeginFrameStream { .. } => {
                        ResponseStatus::FrameStreamStarted { generation: 1 }
                    }
                    Request::UploadFrame { envelope, .. } => ResponseStatus::FrameAck {
                        sequence: envelope.sequence,
                        dropped: false,
                    },
                    Request::BeginShmFrameStream { .. } => ResponseStatus::Error(OperationError {
                        code: ErrorCode::Unsupported,
                        message: "mock daemon does not support shared-memory streaming".to_owned(),
                        retry_after_ms: None,
                        applied_targets: Vec::new(),
                    }),
                    Request::GetAccessPolicy
                    | Request::ReplaceAccessPolicy { .. }
                    | Request::CreateToken { .. }
                    | Request::ListTokens
                    | Request::RotateToken { .. }
                    | Request::RevokeToken { .. }
                    | Request::CreateScene { .. }
                    | Request::CaptureScene { .. }
                    | Request::ReplaceScene { .. }
                    | Request::RecaptureScene { .. }
                    | Request::DeleteScene { .. }
                    | Request::ListScenes
                    | Request::GetScene { .. }
                    | Request::ApplyScene { .. }
                    | Request::StartTransition(_)
                    | Request::GetTransition { .. }
                    | Request::AbortTransition { .. }
                    | Request::RenewTransition { .. }
                    | Request::CreateAttestation { .. }
                    | Request::ListAttestations
                    | Request::RevokeAttestation { .. } => ResponseStatus::Error(OperationError {
                        code: ErrorCode::Unsupported,
                        message: "scene request is outside this fixture".to_owned(),
                        retry_after_ms: None,
                        applied_targets: Vec::new(),
                    }),
                };

                send(
                    &mut stream,
                    &ResponseMessage {
                        id: message.id,
                        response: Response { status },
                    },
                )
                .await
                .expect("send mock response");
            }
        });
    });
    ready_rx.recv().expect("mock daemon should become ready");
    handle
}

fn spawn_mock_event_daemon(socket_path: &Path) -> thread::JoinHandle<()> {
    spawn_mock_event_daemon_with_device(socket_path, "changed-device")
}

fn spawn_stalling_mock_event_daemon(socket_path: &Path) -> thread::JoinHandle<()> {
    let socket_path = socket_path.to_path_buf();
    let (ready_tx, ready_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let runtime = runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build stalling event daemon runtime");
        runtime.block_on(async move {
            let listener = net::UnixListener::bind(&socket_path).expect("bind event socket");
            ready_tx.send(()).expect("send event daemon ready signal");
            let (mut stream, _) = listener.accept().await.expect("accept C event client");
            let _: SubscribeHello = receive(&mut stream)
                .await
                .expect("receive C subscription hello");
            send(
                &mut stream,
                &SubscribeAck {
                    compatibility: EventCompatibility::Compatible,
                    event_protocol_version: luminate_protocol::EVENT_PROTOCOL_VERSION,
                    daemon_version: "mock-daemon-0.1".to_owned(),
                },
            )
            .await
            .expect("send C subscription ack");
            let mut byte = [0_u8; 1];
            let _ = stream.read(&mut byte).await;
        });
    });
    ready_rx
        .recv()
        .expect("stalling event daemon should be ready");
    handle
}

fn spawn_mock_event_daemon_with_device(
    socket_path: &Path,
    device_id: &str,
) -> thread::JoinHandle<()> {
    let socket_path = socket_path.to_path_buf();
    let device_id = DeviceId::new(device_id);
    let (ready_tx, ready_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let runtime = runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build mock event daemon runtime");
        runtime.block_on(async move {
            let listener = net::UnixListener::bind(&socket_path).expect("bind mock event socket");
            ready_tx.send(()).expect("send event daemon ready signal");
            let (mut stream, _) = listener.accept().await.expect("accept C event client");
            let hello: SubscribeHello = receive(&mut stream)
                .await
                .expect("receive C subscription hello");
            assert_eq!(
                hello.ticket.expose(),
                [4_u8; 32],
                "C subscriber should send the supplied event ticket"
            );
            send(
                &mut stream,
                &SubscribeAck {
                    compatibility: EventCompatibility::Compatible,
                    event_protocol_version: luminate_protocol::EVENT_PROTOCOL_VERSION,
                    daemon_version: "mock-daemon-0.1".to_owned(),
                },
            )
            .await
            .expect("send C subscription ack");
            send(
                &mut stream,
                &Event::TopologyChanged {
                    devices: vec![device_id.clone()],
                },
            )
            .await
            .expect("send C topology event");
            send(
                &mut stream,
                &Event::StateChanged {
                    devices: vec![device_id.clone()],
                },
            )
            .await
            .expect("send C state event");
            send(
                &mut stream,
                &Event::ConfigurationChanged {
                    changes: mock_management_changes(),
                },
            )
            .await
            .expect("send C configuration event");
            send(
                &mut stream,
                &Event::ShmStreamEnded {
                    target: TargetId::surface(device_id.as_str(), "panel"),
                    generation: 9,
                },
            )
            .await
            .expect("send C shared-memory stream event");
        });
    });
    ready_rx.recv().expect("mock event daemon should be ready");
    handle
}

fn spawn_structured_error_daemon(socket_path: &Path) -> thread::JoinHandle<()> {
    let socket_path = socket_path.to_path_buf();
    let (ready_tx, ready_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let runtime = runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build structured error daemon runtime");
        runtime.block_on(async move {
            let listener =
                net::UnixListener::bind(&socket_path).expect("bind structured error daemon socket");
            ready_tx
                .send(())
                .expect("send structured error daemon ready signal");
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept structured error C client");

            let _: ClientHello = receive(&mut stream)
                .await
                .expect("receive structured error client hello");
            send(
                &mut stream,
                &DaemonHello {
                    compatibility: Compatibility::Compatible,
                    protocol_abi_version: luminate_protocol::PROTOCOL_ABI_VERSION,
                    daemon_version: "structured-error-daemon".to_owned(),
                },
            )
            .await
            .expect("send structured error daemon hello");
            authenticate_client(&mut stream).await;

            let responses = [
                OperationError {
                    code: ErrorCode::RateLimited,
                    message: "provider asks the caller to wait before retrying".to_owned(),
                    retry_after_ms: Some(375),
                    applied_targets: Vec::new(),
                },
                OperationError {
                    code: ErrorCode::PartialMutation,
                    message: "one target changed before the operation failed".to_owned(),
                    retry_after_ms: None,
                    applied_targets: vec![TargetId::element("keyboard", "keys", "escape")],
                },
                OperationError {
                    code: ErrorCode::PermissionDenied,
                    message: "safe policy reason".to_owned(),
                    retry_after_ms: None,
                    applied_targets: Vec::new(),
                },
            ];
            for error in responses {
                let message: RequestMessage = receive(&mut stream)
                    .await
                    .expect("receive structured error request");
                assert!(
                    matches!(message.request, Request::SetEffect(_)),
                    "structured error fixture expects a set-effect request"
                );
                send(
                    &mut stream,
                    &ResponseMessage {
                        id: message.id,
                        response: Response {
                            status: ResponseStatus::Error(error),
                        },
                    },
                )
                .await
                .expect("send structured error response");
            }
        });
    });
    ready_rx
        .recv()
        .expect("structured error daemon should become ready");
    handle
}

fn spawn_incompatible_mock_daemon(socket_path: &Path) -> thread::JoinHandle<()> {
    let socket_path = socket_path.to_path_buf();
    let (ready_tx, ready_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let runtime = runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build incompatible mock daemon runtime");
        runtime.block_on(async move {
            let listener = net::UnixListener::bind(&socket_path)
                .expect("bind incompatible mock daemon socket");
            ready_tx
                .send(())
                .expect("send incompatible mock daemon ready signal");
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept incompatible C client");
            let _: ClientHello = receive(&mut stream)
                .await
                .expect("receive incompatible client hello");
            send(
                &mut stream,
                &DaemonHello {
                    compatibility: Compatibility::Incompatible {
                        supported_protocol_abi_version: luminate_protocol::PROTOCOL_ABI_VERSION
                            .saturating_add(1),
                        reason: Some("mock primary protocol mismatch".to_owned()),
                    },
                    protocol_abi_version: luminate_protocol::PROTOCOL_ABI_VERSION.saturating_add(1),
                    daemon_version: "mock-incompatible-daemon".to_owned(),
                },
            )
            .await
            .expect("send incompatible daemon hello");
        });
    });
    ready_rx
        .recv()
        .expect("incompatible mock daemon should become ready");
    handle
}

fn spawn_incompatible_mock_event_daemon(socket_path: &Path) -> thread::JoinHandle<()> {
    let socket_path = socket_path.to_path_buf();
    let (ready_tx, ready_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let runtime = runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build incompatible event daemon runtime");
        runtime.block_on(async move {
            let listener =
                net::UnixListener::bind(&socket_path).expect("bind incompatible event socket");
            ready_tx
                .send(())
                .expect("send incompatible event daemon ready signal");
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept incompatible C event client");
            let _: SubscribeHello = receive(&mut stream)
                .await
                .expect("receive incompatible subscription hello");
            send(
                &mut stream,
                &SubscribeAck {
                    compatibility: EventCompatibility::Incompatible {
                        supported_event_protocol_version: luminate_protocol::EVENT_PROTOCOL_VERSION
                            .saturating_add(1),
                        reason: Some("mock event protocol mismatch".to_owned()),
                    },
                    event_protocol_version: luminate_protocol::EVENT_PROTOCOL_VERSION
                        .saturating_add(1),
                    daemon_version: "mock-incompatible-daemon".to_owned(),
                },
            )
            .await
            .expect("send incompatible subscription ack");
        });
    });
    ready_rx
        .recv()
        .expect("incompatible event daemon should become ready");
    handle
}

fn spawn_disconnecting_mock_daemon(socket_path: &Path) -> thread::JoinHandle<()> {
    let socket_path = socket_path.to_path_buf();
    let (ready_tx, ready_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let runtime = runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build disconnecting mock daemon runtime");
        runtime.block_on(async move {
            let listener = net::UnixListener::bind(&socket_path)
                .expect("bind disconnecting mock daemon socket");
            ready_tx
                .send(())
                .expect("send disconnecting mock daemon ready signal");
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept disconnect-test C client");
            let _: ClientHello = receive(&mut stream)
                .await
                .expect("receive disconnect-test client hello");
            send(
                &mut stream,
                &DaemonHello {
                    compatibility: Compatibility::Compatible,
                    protocol_abi_version: luminate_protocol::PROTOCOL_ABI_VERSION,
                    daemon_version: "mock-disconnecting-daemon".to_owned(),
                },
            )
            .await
            .expect("send disconnect-test daemon hello");
            authenticate_client(&mut stream).await;
            let _: RequestMessage = receive(&mut stream)
                .await
                .expect("receive request before disconnect");
            // Dropping the stream without a response fails the pending call.
        });
    });
    ready_rx
        .recv()
        .expect("disconnecting mock daemon should become ready");
    handle
}
async fn authenticate_client(stream: &mut net::UnixStream) {
    let request: AuthenticationRequest = receive(stream).await.expect("receive authentication");
    assert!(
        matches!(
            request.authentication,
            luminate_protocol::Authentication::Peer
        ),
        "C clients must use peer authentication during this phase"
    );
    send(
        stream,
        &AuthenticationResponse::Authenticated {
            session: luminate_protocol::SessionMetadata {
                subject: PrincipalId::new("unix", "1000").expect("valid principal"),
                verified_groups: Vec::new(),
                source: AuthenticationSource::Peer,
                credential_id: None,
                expires_at: None,
            },
        },
    )
    .await
    .expect("send authentication response");
}
