// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! End-to-end D-Bus tests against the demo plugin.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::tests_outside_test_module,
    clippy::unwrap_in_result,
    clippy::iter_over_hash_type,
    reason = "Integration tests are crate roots and intentionally fail loudly on setup errors; \
    hash-map iteration order does not matter for these membership/introspection checks."
)]

use std::collections::{BTreeSet, HashMap};
use std::env;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use luminate::policy::{Binding, PolicyRevision, Preset, materialize_presets};
use luminate_platform::secure_storage::create_private_file;
use luminate_platform::test_support::TestDir;
#[cfg(windows)]
use luminate_platform::windows::identity::current_process_sid;
use luminate_platform::{dynamic_library_candidates, executable_name};
use tokio::runtime::Runtime;
use tokio::time::sleep;
use zbus::fdo::{IntrospectableProxy, ManagedObjects, ObjectManagerProxy};
use zbus::zvariant::{OwnedValue, Value};

const DESTINATION: &str = "org.luminate.Luminate1";
const ROOT: &str = "/org/luminate/Luminate1";
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

fn static_rgb_effect(red: u32, green: u32, blue: u32) -> HashMap<&'static str, Value<'static>> {
    let channels = HashMap::from([
        ("red".to_owned(), red),
        ("green".to_owned(), green),
        ("blue".to_owned(), blue),
    ]);
    let colour = HashMap::from([
        ("Model", Value::from("additive")),
        ("Channels", Value::from(channels)),
    ]);

    HashMap::from([
        ("Kind", Value::from("static")),
        ("StaticColour", Value::from(colour)),
    ])
}

fn static_rgb_colour(red: u32, green: u32, blue: u32) -> Dictionary {
    HashMap::from([
        (
            "Model".to_owned(),
            OwnedValue::try_from(Value::from("additive")).expect("own colour model"),
        ),
        (
            "Channels".to_owned(),
            OwnedValue::try_from(Value::from(HashMap::from([
                ("red".to_owned(), red),
                ("green".to_owned(), green),
                ("blue".to_owned(), blue),
            ])))
            .expect("own colour channels"),
        ),
    ])
}

fn target_selector(target: &str) -> Dictionary {
    HashMap::from([
        (
            "Kind".to_owned(),
            OwnedValue::try_from(Value::from("target")).expect("own selector kind"),
        ),
        (
            "Target".to_owned(),
            OwnedValue::try_from(Value::from(target)).expect("own selector target"),
        ),
    ])
}
const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[test]
#[ignore = "spawns a private D-Bus session and workspace binaries with the demo plugin artifact"]
fn object_manager_enumeration_and_introspection_match_demo_plugin() {
    run_demo_test(
        "object_manager_enumeration_and_introspection_match_demo_plugin",
        DemoTest::Enumeration,
    );
}

#[test]
#[ignore = "spawns a private D-Bus session and workspace binaries with the demo plugin artifact"]
fn every_v2_mutation_reaches_demo_plugin() {
    run_demo_test("every_v2_mutation_reaches_demo_plugin", DemoTest::Mutations);
}

#[test]
#[ignore = "spawns a private D-Bus session and workspace binaries with the demo plugin artifact"]
fn topology_add_change_remove_signals_follow_demo_plugin_lifecycle() {
    run_demo_test(
        "topology_add_change_remove_signals_follow_demo_plugin_lifecycle",
        DemoTest::Lifecycle,
    );
}

#[test]
#[ignore = "spawns a private D-Bus session and workspace binaries with the demo plugin artifact"]
fn daemon_loss_reconnection_preserves_subscribe_before_baseline_topology() {
    run_demo_test(
        "daemon_loss_reconnection_preserves_subscribe_before_baseline_topology",
        DemoTest::Lifecycle,
    );
}

#[test]
#[ignore = "spawns a private D-Bus session and workspace binaries with the demo plugin artifact"]
fn authorization_denies_non_group_allows_group_and_limits_polkit_fallback() {
    run_demo_test(
        "authorization_denies_non_group_allows_group_and_limits_polkit_fallback",
        DemoTest::Authorization,
    );
}

#[derive(Clone, Copy)]
enum DemoTest {
    Enumeration,
    Mutations,
    Lifecycle,
    Authorization,
}

fn run_demo_test(test_name: &str, test: DemoTest) {
    let _guard = test_lock().lock().expect("lock poisoned");
    if env::var_os("LUMINATE_DBUS_TEST_BUS").is_none() {
        run_on_private_bus(test_name);
        return;
    }

    let workspace = workspace_root();
    build_demo_stack(&workspace);
    if matches!(test, DemoTest::Lifecycle) {
        run_lifecycle_test(&workspace);
        return;
    }
    if matches!(test, DemoTest::Authorization) {
        run_authorization_test(&workspace);
        return;
    }
    let temp = temp_dir();
    let socket_path = temp.join("luminated.sock");
    let config_path = temp.join("luminated.toml");
    let group_path = temp.join("group");
    let daemon_log = temp.join("luminated.log");
    let adapter_log = temp.join("luminate-dbus.log");

    let plugin_path = debug_artifact_dir(&workspace)
        .join(dynamic_library_candidates("luminate_plugin_demo_system")[0].as_str());
    write_daemon_config(
        &config_path,
        &socket_path,
        &temp.join("state.json"),
        &plugin_path,
    );
    fs::write(&group_path, format!("luminate:x:{}:\n", caller_group()))
        .expect("write fake group database");

    let daemon = spawn_logged(
        daemon_executable(&workspace),
        &[],
        &[("LUMINATED_CONFIG", config_path.as_os_str())],
        &daemon_log,
    );
    let _daemon = ChildGuard(daemon);
    wait_for_path(&socket_path, "luminated socket");

    let adapter = PathBuf::from(env!("CARGO_BIN_EXE_luminate-dbus"));
    let bus_address = env::var_os("DBUS_SESSION_BUS_ADDRESS").expect("private bus address");
    let adapter = spawn_logged(
        adapter,
        &[
            "--socket-path",
            socket_path.to_str().expect("UTF-8 socket path"),
            "--group-file",
            group_path.to_str().expect("UTF-8 group path"),
        ],
        &[("DBUS_SYSTEM_BUS_ADDRESS", bus_address.as_os_str())],
        &adapter_log,
    );
    let _adapter = ChildGuard(adapter);

    let runtime = Runtime::new().expect("create Tokio runtime");
    runtime.block_on(async {
        match test {
            DemoTest::Enumeration => assert_dbus_topology(&adapter_log).await,
            DemoTest::Mutations => assert_v2_mutations(&adapter_log, &daemon_log).await,
            DemoTest::Lifecycle => unreachable!("lifecycle uses its own process ordering"),
            DemoTest::Authorization => unreachable!("authorization uses multiple adapters"),
        }
    });
}

fn test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn run_authorization_test(workspace: &Path) {
    let temp = temp_dir();
    let socket_path = temp.join("luminated.sock");
    let config_path = temp.join("luminated.toml");
    let daemon_log = temp.join("luminated.log");
    write_daemon_config(
        &config_path,
        &socket_path,
        &temp.join("state.json"),
        &debug_artifact_dir(workspace)
            .join(dynamic_library_candidates("luminate_plugin_demo_system")[0].as_str()),
    );
    let daemon = spawn_daemon(workspace, &config_path, &daemon_log);
    let _daemon = ChildGuard(daemon);
    wait_for_path(&socket_path, "luminated socket");

    let allowed = run_authorization_case(&temp, &socket_path, caller_group(), true);
    assert!(
        allowed.is_ok(),
        "group member should bypass polkit: {allowed:?}"
    );

    let denied = run_authorization_case(&temp, &socket_path, "4294967294", false)
        .expect_err("non-group caller should be denied without polkit");
    assert!(
        denied.contains("requires membership in the luminate group"),
        "direct denial should identify the failed group check: {denied}"
    );

    let fallback = run_authorization_case(&temp, &socket_path, "4294967294", true)
        .expect_err("private test bus has no polkit authority");
    assert!(
        !fallback.contains("requires membership in the luminate group"),
        "polkit-enabled non-group caller should reach polkit instead of direct denial: {fallback}"
    );
}

fn run_authorization_case(
    temp: &Path,
    socket_path: &Path,
    gid: &str,
    polkit: bool,
) -> Result<(), String> {
    let group_path = temp.join(format!("group-{gid}-{polkit}"));
    let log_path = temp.join(format!("adapter-{gid}-{polkit}.log"));
    fs::write(&group_path, format!("luminate:x:{gid}:\n")).expect("write group fixture");
    let bus_address = env::var_os("DBUS_SESSION_BUS_ADDRESS").expect("private bus address");
    let mut args = vec![
        "--socket-path",
        socket_path.to_str().expect("UTF-8 socket path"),
        "--group-file",
        group_path.to_str().expect("UTF-8 group path"),
    ];
    if polkit {
        args.push("--polkit");
    }
    let adapter = spawn_logged(
        PathBuf::from(env!("CARGO_BIN_EXE_luminate-dbus")),
        &args,
        &[("DBUS_SYSTEM_BUS_ADDRESS", bus_address.as_os_str())],
        &log_path,
    );
    let mut adapter = ChildGuard(adapter);
    wait_for_adapter();
    let result = Runtime::new()
        .expect("create authorization runtime")
        .block_on(async {
            let connection = wait_for_service(&log_path).await;
            let path = target_path("demo-keyboard", Some("zones"), Some("g1"), None);
            target_proxy(&connection, &path)
                .await
                .call::<_, _, ()>("SetEffect", &(static_rgb_effect(1, 2, 3),))
                .await
                .map_err(|error| error.to_string())
        });
    adapter.0.kill().expect("stop authorization adapter");
    adapter.0.wait().expect("reap authorization adapter");
    wait_for_name_release();
    result
}

fn wait_for_name_release() {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        let owned = Runtime::new()
            .expect("create name-check runtime")
            .block_on(async {
                let connection = zbus::Connection::session().await.expect("connect to bus");
                zbus::fdo::DBusProxy::new(&connection)
                    .await
                    .expect("build bus proxy")
                    .name_has_owner(DESTINATION.try_into().expect("valid bus name"))
                    .await
                    .expect("query bus name")
            });
        if !owned {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "adapter bus name was not released"
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn run_lifecycle_test(workspace: &Path) {
    let temp = temp_dir();
    let socket_path = temp.join("luminated.sock");
    let config_path = temp.join("luminated.toml");
    let group_path = temp.join("group");
    let daemon_log = temp.join("luminated.log");
    let adapter_log = temp.join("luminate-dbus.log");
    let monitor_log = temp.join("dbus-monitor.log");
    let plugin_path = debug_artifact_dir(workspace)
        .join(dynamic_library_candidates("luminate_plugin_demo_system")[0].as_str());
    write_daemon_config(
        &config_path,
        &socket_path,
        &temp.join("state.json"),
        &plugin_path,
    );
    fs::write(&group_path, format!("luminate:x:{}:\n", caller_group()))
        .expect("write fake group database");

    let bus_address = env::var_os("DBUS_SESSION_BUS_ADDRESS").expect("private bus address");
    let adapter = spawn_logged(
        PathBuf::from(env!("CARGO_BIN_EXE_luminate-dbus")),
        &[
            "--socket-path",
            socket_path.to_str().expect("UTF-8 socket path"),
            "--group-file",
            group_path.to_str().expect("UTF-8 group path"),
        ],
        &[("DBUS_SYSTEM_BUS_ADDRESS", bus_address.as_os_str())],
        &adapter_log,
    );
    let _adapter = ChildGuard(adapter);
    wait_for_adapter();

    let monitor = spawn_logged(
        PathBuf::from("dbus-monitor"),
        &[
            "--session",
            "type='signal',path_namespace='/org/luminate/Luminate1'",
        ],
        &[],
        &monitor_log,
    );
    let _monitor = ChildGuard(monitor);

    let mut daemon = spawn_daemon(workspace, &config_path, &daemon_log);
    wait_for_path(&socket_path, "first luminated socket");
    wait_for_log_occurrences(&monitor_log, "member=InterfacesAdded", 1);
    let first = wait_for_stable_object_count();
    assert!(first > 10, "demo baseline should contain its full topology");

    daemon.kill().expect("stop first daemon");
    daemon.wait().expect("reap first daemon");
    let _ = fs::remove_file(&socket_path);
    wait_for_log_occurrences(&monitor_log, "member=InterfacesRemoved", 1);
    wait_for_log_occurrences(&monitor_log, "member=PropertiesChanged", 1);

    let mut daemon = spawn_daemon(workspace, &config_path, &daemon_log);
    wait_for_path(&socket_path, "restarted luminated socket");
    wait_for_log_occurrences(&monitor_log, "member=InterfacesAdded", 2);
    let second = wait_for_object_count(first);
    assert_eq!(
        second, first,
        "reconnection must restore the complete baseline"
    );
    assert!(
        fs::read_to_string(&monitor_log)
            .unwrap_or_default()
            .contains("Available"),
        "daemon lifecycle should emit the Manager Available property change"
    );
    daemon.kill().expect("stop restarted daemon");
    let _ = daemon.wait();
}

fn object_count() -> usize {
    let runtime = Runtime::new().expect("create topology-check runtime");
    runtime.block_on(async {
        let connection = zbus::Connection::session().await.expect("connect to bus");
        ObjectManagerProxy::builder(&connection)
            .destination(DESTINATION)
            .expect("valid destination")
            .path(ROOT)
            .expect("valid root")
            .build()
            .await
            .expect("build ObjectManager proxy")
            .get_managed_objects()
            .await
            .expect("enumerate topology")
            .len()
    })
}

fn wait_for_object_count(expected: usize) -> usize {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        let count = object_count();
        if count == expected {
            return count;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {expected} reconnected objects; found {count}"
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_for_stable_object_count() -> usize {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    let mut previous = None;
    let mut stable_polls = 0;
    loop {
        let count = object_count();
        if previous == Some(count) {
            stable_polls += 1;
            if stable_polls == 3 {
                return count;
            }
        } else {
            previous = Some(count);
            stable_polls = 0;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the initial object topology to stabilize"
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_for_adapter() {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        let runtime = Runtime::new().expect("create adapter-check runtime");
        let ready = runtime.block_on(async {
            let connection = zbus::Connection::session().await.ok()?;
            ObjectManagerProxy::builder(&connection)
                .destination(DESTINATION)
                .ok()?
                .path(ROOT)
                .ok()?
                .build()
                .await
                .ok()
        });
        if ready.is_some() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for D-Bus adapter"
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn spawn_daemon(workspace: &Path, config: &Path, log: &Path) -> Child {
    spawn_logged(
        daemon_executable(workspace),
        &[],
        &[("LUMINATED_CONFIG", config.as_os_str())],
        log,
    )
}

fn daemon_executable(workspace: &Path) -> PathBuf {
    env::var_os("LUMINATE_TEST_DAEMON").map_or_else(
        || debug_artifact_dir(workspace).join(executable_name("luminated")),
        PathBuf::from,
    )
}

fn wait_for_log_occurrences(log_path: &Path, needle: &str, count: usize) {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        let log = fs::read_to_string(log_path).unwrap_or_default();
        if log.matches(needle).count() >= count {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {count} occurrences of {needle:?} in {}\n{log}",
            log_path.display()
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn run_on_private_bus(test_name: &str) {
    let output = Command::new("dbus-run-session")
        .arg("--")
        .arg(env::current_exe().expect("locate integration test executable"))
        .args(["--ignored", "--exact", test_name, "--nocapture"])
        .env("LUMINATE_DBUS_TEST_BUS", "1")
        .output()
        .expect("run integration test on a private D-Bus session");
    assert!(
        output.status.success(),
        "private-bus test failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn assert_v2_mutations(adapter_log: &Path, daemon_log: &Path) {
    let connection = wait_for_service(adapter_log).await;
    let keyboard_key = target_path("demo-keyboard", Some("zones"), Some("g1"), None);
    let case_group = target_path("demo-case-lights", None, None, Some("chassis"));
    let mouse_wheel = target_path("demo-mouse", Some("lighting"), Some("wheel"), None);

    let keyboard = target_proxy(&connection, &keyboard_key).await;
    keyboard
        .call::<_, _, ()>("SetEffect", &(static_rgb_effect(12, 34, 56),))
        .await
        .expect("static SetEffect should succeed");
    let state = keyboard
        .call::<_, _, Vec<(String, String, String, bool, u64)>>("State", &())
        .await
        .expect("State should be readable after a mutation");
    assert!(!state.is_empty(), "the mutated key should report state");

    let case_lights = target_proxy(&connection, &case_group).await;
    case_lights
        .call::<_, _, ()>("SetBrightness", &(85_u32,))
        .await
        .expect("SetBrightness should succeed");

    let mouse = target_proxy(&connection, &mouse_wheel).await;
    let effect = static_rgb_effect(90, 10, 200);
    mouse
        .call::<_, _, ()>("SetEffect", &(effect,))
        .await
        .expect("SetEffect should succeed");
    mouse
        .call::<_, _, ()>("Off", &())
        .await
        .expect("Off should succeed");
    mouse
        .call::<_, _, ()>("ClearDesiredState", &())
        .await
        .expect("ClearDesiredState should succeed");

    let refresh = keyboard.call::<_, _, ()>("RefreshState", &()).await;
    let error = refresh.expect_err("demo-system advertises no live readback");
    assert!(
        error.to_string().contains("unsupported operation"),
        "RefreshState should reach the daemon's capability check: {error}"
    );

    assert_manager_operations(&connection).await;
    assert_phase3_controls(&connection, &keyboard_key).await;
    assert_phase4_operations(&connection).await;
    assert_phase5_operations(&connection).await;

    wait_for_log_contains(daemon_log, "operation=set-effect");
    wait_for_log_contains(daemon_log, "operation=set-brightness");
    wait_for_log_contains(daemon_log, "operation=set-effect");
    wait_for_log_contains(daemon_log, "operation=clear");
}

#[allow(
    clippy::too_many_lines,
    reason = "the integration sequence exercises every Phase 3 method through one attested private-bus session"
)]
async fn assert_phase3_controls(connection: &zbus::Connection, path: &str) {
    let target_id = "device:demo-keyboard/surface:zones/element:g1";
    let target = zbus::Proxy::new(connection, DESTINATION, path, "org.luminate.Target3")
        .await
        .expect("build Target3 proxy");
    target
        .call::<_, _, ()>("SetColour", &(static_rgb_colour(1, 2, 3),))
        .await
        .expect("SetColour should succeed");
    target
        .call::<_, _, ()>("SetRgb", &(4_u8, 5_u8, 6_u8))
        .await
        .expect("SetRgb should succeed");
    let save = target.call::<_, _, ()>("SaveCurrent", &()).await;
    assert!(
        save.is_err(),
        "the demo key should reject unsupported persistence"
    );
    target
        .call::<_, _, ()>("SetEmission", &("dark",))
        .await
        .expect("SetEmission dark should succeed");
    target
        .call::<_, _, ()>("RestoreAppearance", &())
        .await
        .expect("RestoreAppearance should succeed");
    target
        .call::<_, _, ()>("SetCct", &(4_000_u32,))
        .await
        .expect("SetCct should use the demo key's CCT emulation");

    let manager = zbus::Proxy::new(
        connection,
        DESTINATION,
        ROOT,
        "org.luminate.Luminate1.Manager2",
    )
    .await
    .expect("build Manager2 proxy");
    manager
        .call::<_, _, Dictionary>(
            "SetEffectSelector",
            &(
                target_selector(target_id),
                static_rgb_effect(7, 8, 9),
                false,
                "",
            ),
        )
        .await
        .expect("SetEffectSelector should succeed");
    manager
        .call::<_, _, Dictionary>(
            "SetColourSelector",
            &(
                target_selector(target_id),
                static_rgb_colour(10, 11, 12),
                false,
                "",
            ),
        )
        .await
        .expect("SetColourSelector should succeed");
    manager
        .call::<_, _, Dictionary>(
            "SetRgbSelector",
            &(target_selector(target_id), 13_u8, 14_u8, 15_u8, false, ""),
        )
        .await
        .expect("SetRgbSelector should succeed");
    manager
        .call::<_, _, Dictionary>(
            "SetBrightnessSelector",
            &(target_selector(target_id), 50_u32, false, ""),
        )
        .await
        .expect("SetBrightnessSelector should succeed");
    let save = manager
        .call::<_, _, Dictionary>("SaveCurrentSelector", &(target_selector(target_id),))
        .await;
    assert!(
        save.is_err(),
        "the demo key selector should reject unsupported persistence"
    );
    manager
        .call::<_, _, Dictionary>("SetEmissionSelector", &(target_selector(target_id), "dark"))
        .await
        .expect("SetEmissionSelector should succeed");
    manager
        .call::<_, _, Dictionary>("RestoreAppearanceSelector", &(target_selector(target_id),))
        .await
        .expect("RestoreAppearanceSelector should succeed");
    manager
        .call::<_, _, Dictionary>(
            "SetCctSelector",
            &(target_selector(target_id), 4_000_u32, false, ""),
        )
        .await
        .expect("SetCctSelector should use the demo key's CCT emulation");
}

fn transition_options(duration_ms: u64) -> Dictionary {
    HashMap::from([
        ("DurationMs".to_owned(), OwnedValue::from(duration_ms)),
        (
            "Function".to_owned(),
            OwnedValue::try_from(Value::from("linear")).expect("own transition function"),
        ),
        (
            "ColourInterpolation".to_owned(),
            OwnedValue::try_from(Value::from("encoded")).expect("own interpolation"),
        ),
    ])
}

fn transition_target_state(target: &str, brightness: u32) -> Dictionary {
    HashMap::from([
        (
            "Target".to_owned(),
            OwnedValue::try_from(Value::from(target)).expect("own transition target"),
        ),
        ("Brightness".to_owned(), OwnedValue::from(brightness)),
    ])
}

fn full_frame(generation: u32) -> Dictionary {
    HashMap::from([
        ("Generation".to_owned(), OwnedValue::from(generation)),
        ("Sequence".to_owned(), OwnedValue::from(1_u64)),
        (
            "Kind".to_owned(),
            OwnedValue::try_from(Value::from("full")).expect("own frame kind"),
        ),
        (
            "Colours".to_owned(),
            OwnedValue::try_from(Value::new(vec![static_rgb_colour(1, 2, 3)]))
                .expect("own frame colours"),
        ),
        ("Commit".to_owned(), OwnedValue::from(true)),
    ])
}

async fn assert_phase4_operations(connection: &zbus::Connection) {
    let target = "device:demo-keyboard/surface:zones/element:g1";
    let manager = zbus::Proxy::new(
        connection,
        DESTINATION,
        ROOT,
        "org.luminate.Luminate1.Manager2",
    )
    .await
    .expect("build Manager2 proxy");
    assert_transition_error_paths(&manager, target).await;
    let started = manager
        .call::<_, _, Dictionary>(
            "CreateCurrentToStatesTransition",
            &(
                vec![transition_target_state(target, 40)],
                transition_options(50),
            ),
        )
        .await
        .expect("create current-to-states transition");
    let id = String::try_from(
        started
            .get("Id")
            .expect("transition status identifier")
            .try_clone()
            .expect("clone transition identifier"),
    )
    .expect("transition identifier should be a string");
    manager
        .call::<_, _, Dictionary>("GetTransition", &(id.clone(),))
        .await
        .expect("get transition status");
    let terminal = manager
        .call::<_, _, Dictionary>("WaitTransition", &(id,))
        .await
        .expect("wait for transition");
    assert!(
        bool::try_from(
            terminal
                .get("HasOutcome")
                .expect("terminal outcome flag")
                .try_clone()
                .expect("clone outcome flag")
        )
        .expect("outcome flag should be boolean"),
        "WaitTransition should return a terminal outcome"
    );

    let begin = manager
        .call::<_, _, u32>("BeginFrameStream", &(target,))
        .await;
    assert!(
        begin.is_err(),
        "the demo key should reject unsupported ordinary frame streaming"
    );
    let upload = manager
        .call::<_, _, (u64, bool)>("UploadFrame", &(target, full_frame(1)))
        .await;
    assert!(
        upload.is_err(),
        "upload without a negotiated stream should fail"
    );
    manager
        .call::<_, _, ()>("EndFrameStream", &(target, 1_u32))
        .await
        .expect("ending an absent frame stream should be idempotent");
    manager
        .call::<_, _, Dictionary>("ClearSelector", &(target_selector(target),))
        .await
        .expect("ClearSelector should succeed");
}

async fn assert_transition_error_paths(manager: &zbus::Proxy<'_>, target: &str) {
    assert!(
        manager
            .call::<_, _, Dictionary>(
                "CreateSceneToSceneTransition",
                &(
                    "missing-source",
                    "missing-destination",
                    transition_options(50),
                ),
            )
            .await
            .is_err(),
        "scene-to-scene should report missing scenes"
    );
    assert!(
        manager
            .call::<_, _, Dictionary>(
                "CreateCurrentToSceneTransition",
                &("missing-destination", transition_options(50)),
            )
            .await
            .is_err(),
        "current-to-scene should report a missing destination"
    );
    assert!(
        manager
            .call::<_, _, Dictionary>(
                "CreateSceneToStatesTransition",
                &(
                    "missing-source",
                    vec![transition_target_state(target, 40)],
                    transition_options(50),
                ),
            )
            .await
            .is_err(),
        "scene-to-states should report a missing source"
    );
    assert!(
        manager
            .call::<_, _, Dictionary>("AbortTransition", &("missing-transition",))
            .await
            .is_err(),
        "abort should report an unknown transition"
    );
}

type AttestationRecord = (String, String, String, Vec<String>, String, bool, u64);

async fn assert_phase5_operations(connection: &zbus::Connection) {
    let manager = zbus::Proxy::new(
        connection,
        DESTINATION,
        ROOT,
        "org.luminate.Luminate1.Manager2",
    )
    .await
    .expect("build Manager2 proxy");
    let session = manager
        .call::<_, _, Dictionary>("SessionInformation", &())
        .await
        .expect("read sanitized session information");
    assert!(
        session.contains_key("Authority"),
        "session information should identify the authenticated authority"
    );
    assert!(
        !session.contains_key("Credential"),
        "session information must not expose a credential"
    );
    assert!(
        !session.contains_key("Secret"),
        "session information must not expose secret material"
    );
    assert_independent_caller_session(&session).await;

    let workflows = manager
        .call::<_, _, Vec<Dictionary>>("PluginSetupWorkflows", &("luminate-plugin-demo-system",))
        .await
        .expect("inspect demo setup workflows");
    assert!(
        workflows.is_empty(),
        "the demo plugin should advertise no setup workflow"
    );
    assert!(
        manager
            .call::<_, _, Dictionary>(
                "StartPluginSetup",
                &("luminate-plugin-demo-system", "missing"),
            )
            .await
            .is_err(),
        "starting an unadvertised workflow should fail"
    );
    assert!(
        manager
            .call::<_, _, Dictionary>("PluginSetupSession", &("malformed",))
            .await
            .is_err(),
        "malformed setup session identifiers should fail at the boundary"
    );
    let missing_session = "0123456789abcdef0123456789abcdef";
    let confirmed = HashMap::from([(
        "Kind".to_owned(),
        OwnedValue::try_from(Value::from("confirmed")).expect("own response kind"),
    )]);
    assert!(
        manager
            .call::<_, _, Dictionary>("RespondPluginSetup", &(missing_session, 1_u64, confirmed),)
            .await
            .is_err(),
        "responding to an unknown setup session should fail"
    );
    assert!(
        manager
            .call::<_, _, Dictionary>("CancelPluginSetup", &(missing_session,))
            .await
            .is_err(),
        "cancelling an unknown setup session should fail"
    );

    assert_attestation_denials(&manager).await;
}

async fn assert_independent_caller_session(session: &Dictionary) {
    let first_credential_id = String::try_from(
        session
            .get("CredentialId")
            .expect("attested session should identify its non-secret credential")
            .try_clone()
            .expect("clone first session credential identifier"),
    )
    .expect("credential identifier should be a string");

    let second_connection = zbus::Connection::session()
        .await
        .expect("open an independent connection to the private bus");
    let second_manager = zbus::Proxy::new(
        &second_connection,
        DESTINATION,
        ROOT,
        "org.luminate.Luminate1.Manager2",
    )
    .await
    .expect("build Manager2 proxy for the independent caller");
    let second_session = second_manager
        .call::<_, _, Dictionary>("SessionInformation", &())
        .await
        .expect("read the independent caller's session information");
    let second_credential_id = String::try_from(
        second_session
            .get("CredentialId")
            .expect("second attested session should identify its credential")
            .try_clone()
            .expect("clone second session credential identifier"),
    )
    .expect("second credential identifier should be a string");
    assert_ne!(
        first_credential_id, second_credential_id,
        "callers with the same UID must not share daemon session resources"
    );
}

async fn assert_attestation_denials(manager: &zbus::Proxy<'_>) {
    let created = manager
        .call::<_, _, (AttestationRecord, Vec<u8>)>(
            "CreateAttestation",
            &("dbus-test-attestation", "local", "frontend", false, 0_u64),
        )
        .await;
    assert!(
        created.is_err(),
        "the demo policy should deny attestation administration"
    );
    let principal = manager
        .call::<_, _, (AttestationRecord, Vec<u8>)>(
            "CreatePrincipalAttestation",
            &(
                "dbus-test-principal-attestation",
                "local",
                "operator",
                vec!["lighting"],
                false,
                0_u64,
            ),
        )
        .await;
    assert!(
        principal.is_err(),
        "the demo policy should deny principal attestation administration"
    );
    let records = manager
        .call::<_, _, Vec<AttestationRecord>>("ListAttestations", &())
        .await;
    assert!(
        records.is_err(),
        "the demo policy should deny attestation listing"
    );
    let revoke = manager
        .call::<_, _, ()>("RevokeAttestation", &("dbus-test-attestation",))
        .await
        .expect_err("the demo policy should deny attestation revocation");
    assert!(
        revoke.to_string().contains("permission denied"),
        "revocation should preserve the daemon's permission denial"
    );
}

type Dictionary = HashMap<String, OwnedValue>;
type SceneBindingRecord = (String, String, Dictionary);
type SceneRecord = (
    String,
    u64,
    String,
    bool,
    String,
    String,
    String,
    Vec<SceneBindingRecord>,
);

fn scene_binding() -> Dictionary {
    HashMap::from([
        (
            "Target".to_owned(),
            OwnedValue::try_from(Value::from("device:demo-keyboard/surface:zones/element:g1"))
                .expect("own scene target"),
        ),
        ("Brightness".to_owned(), OwnedValue::from(42_u32)),
    ])
}

#[allow(
    clippy::too_many_lines,
    reason = "the test keeps one ordered Manager session so scene revisions and management state flow through the real D-Bus boundary"
)]
async fn assert_manager_operations(connection: &zbus::Connection) {
    let manager = zbus::Proxy::new(
        connection,
        DESTINATION,
        ROOT,
        "org.luminate.Luminate1.Manager1",
    )
    .await
    .expect("build manager proxy");

    assert!(
        manager
            .get_property::<bool>("Available")
            .await
            .expect("read manager availability"),
        "the connected daemon should make the Manager available"
    );
    manager
        .call::<_, _, ()>("RefreshTopology", &())
        .await
        .expect("refresh topology");
    let initial = manager
        .call::<_, _, Vec<SceneRecord>>("ListScenes", &())
        .await
        .expect("list initial scenes");
    assert!(initial.is_empty(), "a fresh daemon should have no scenes");

    let created = manager
        .call::<_, _, SceneRecord>(
            "CreateScene",
            &(
                "D-Bus integration scene",
                true,
                "Exercises the complete scene manager surface",
                vec![scene_binding()],
            ),
        )
        .await
        .expect("create scene");
    assert_eq!(
        created.2, "D-Bus integration scene",
        "CreateScene should preserve the name"
    );
    assert!(created.3, "CreateScene should preserve the description");

    let fetched = manager
        .call::<_, _, SceneRecord>("GetScene", &(created.0.clone(),))
        .await
        .expect("get created scene");
    assert_eq!(
        fetched.0, created.0,
        "GetScene should return the created identifier"
    );
    assert_eq!(
        fetched.1, created.1,
        "GetScene should return the current revision"
    );

    let replaced = manager
        .call::<_, _, SceneRecord>(
            "ReplaceScene",
            &(
                created.0.clone(),
                created.1,
                "Renamed D-Bus integration scene",
                false,
                String::new(),
                vec![scene_binding()],
            ),
        )
        .await
        .expect("replace scene");
    assert_eq!(
        replaced.2, "Renamed D-Bus integration scene",
        "ReplaceScene should update the name"
    );
    assert!(
        !replaced.3,
        "ReplaceScene should clear the optional description"
    );

    let outcome = manager
        .call::<_, _, (Vec<String>, Vec<String>)>("ApplyScene", &(created.0.clone(),))
        .await
        .expect("apply scene");
    assert_eq!(
        outcome.0,
        ["device:demo-keyboard/surface:zones/element:g1"],
        "ApplyScene should report the applied binding"
    );
    assert!(
        outcome.1.is_empty(),
        "the authorized scene should have no denied bindings"
    );

    let management = manager
        .call::<_, _, Dictionary>("GetManagement", &())
        .await
        .expect("get management snapshot");
    assert!(
        management.contains_key("Revision"),
        "management snapshots should expose their revision"
    );
    assert!(
        management.contains_key("Plugins"),
        "management snapshots should expose plugin state"
    );
    let revision = u64::try_from(
        management
            .get("Revision")
            .expect("management revision")
            .try_clone()
            .expect("clone management revision"),
    )
    .expect("management revision should be a u64");
    let patched = manager
        .call::<_, _, (u64, Vec<(String, String, Vec<String>, bool)>)>(
            "PatchManagement",
            &(revision, Vec::<Dictionary>::new()),
        )
        .await
        .expect("apply an empty management patch");
    assert!(
        patched.0 >= revision,
        "an empty patch must not move the revision backwards"
    );
    assert!(
        patched.1.is_empty(),
        "an empty patch should report no changes"
    );

    manager
        .call::<_, _, ()>("DeleteScene", &(created.0.clone(), replaced.1))
        .await
        .expect("delete scene");
    assert!(
        manager
            .call::<_, _, Vec<SceneRecord>>("ListScenes", &())
            .await
            .expect("list scenes after deletion")
            .is_empty(),
        "DeleteScene should remove the scene from ListScenes"
    );

    let captured = manager
        .call::<_, _, SceneRecord>(
            "CaptureScene",
            &(
                "Captured D-Bus integration scene",
                false,
                String::new(),
                false,
                String::new(),
                vec!["device:demo-keyboard/surface:zones/element:g1"],
            ),
        )
        .await
        .expect("capture scene");
    assert_eq!(
        captured.7.len(),
        1,
        "CaptureScene should snapshot the requested target"
    );

    let recaptured = manager
        .call::<_, _, SceneRecord>(
            "RecaptureScene",
            &(
                captured.0.clone(),
                captured.1,
                false,
                String::new(),
                vec!["device:demo-keyboard/surface:zones/element:g1"],
            ),
        )
        .await
        .expect("recapture scene");
    assert!(
        recaptured.1 > captured.1,
        "RecaptureScene should advance the scene revision"
    );
    manager
        .call::<_, _, ()>("DeleteScene", &(recaptured.0.clone(), recaptured.1))
        .await
        .expect("delete captured scene");

    assert_manager2_collection_operations(connection).await;
}

fn collection_member(kind: &str, field: &str, value: &str) -> Dictionary {
    HashMap::from([
        (
            "Kind".to_owned(),
            OwnedValue::try_from(Value::from(kind)).expect("own member kind"),
        ),
        (
            field.to_owned(),
            OwnedValue::try_from(Value::from(value)).expect("own member identifier"),
        ),
    ])
}

fn collection_request(name: &str, members: Vec<Dictionary>) -> Dictionary {
    HashMap::from([
        (
            "Name".to_owned(),
            OwnedValue::try_from(Value::from(name)).expect("own collection name"),
        ),
        (
            "Members".to_owned(),
            OwnedValue::try_from(Value::new(members)).expect("own collection members"),
        ),
    ])
}

async fn assert_manager2_collection_operations(connection: &zbus::Connection) {
    let manager = zbus::Proxy::new(
        connection,
        DESTINATION,
        ROOT,
        "org.luminate.Luminate1.Manager2",
    )
    .await
    .expect("build Manager2 proxy");
    assert!(
        manager
            .call::<_, _, Vec<Dictionary>>("ListCollections", &())
            .await
            .expect("list initial collections")
            .is_empty(),
        "a fresh daemon should have no collections"
    );

    let nested = manager
        .call::<_, _, String>(
            "CreateCollection",
            &(collection_request("Nested", Vec::new()),),
        )
        .await
        .expect("create nested collection");
    let parent = manager
        .call::<_, _, String>(
            "CreateCollection",
            &(collection_request(
                "Desk",
                vec![collection_member(
                    "target",
                    "Target",
                    "device:demo-keyboard",
                )],
            ),),
        )
        .await
        .expect("create collection with direct target");

    manager
        .call::<_, _, ()>(
            "AddCollectionMember",
            &(
                parent.clone(),
                collection_member("collection", "Collection", &nested),
            ),
        )
        .await
        .expect("add nested collection member");
    let fetched = manager
        .call::<_, _, Dictionary>("GetCollection", &(parent.clone(),))
        .await
        .expect("get created collection");
    assert!(
        bool::try_from(
            fetched
                .get("Found")
                .expect("lookup should contain Found")
                .try_clone()
                .expect("clone found flag")
        )
        .expect("Found should be boolean"),
        "the created collection should be found"
    );

    manager
        .call::<_, _, ()>(
            "RemoveCollectionMember",
            &(
                parent.clone(),
                collection_member("collection", "Collection", &nested),
            ),
        )
        .await
        .expect("remove nested collection member");
    manager
        .call::<_, _, ()>("DestroyCollection", &(parent,))
        .await
        .expect("destroy parent collection");
    manager
        .call::<_, _, ()>("DestroyCollection", &(nested,))
        .await
        .expect("destroy nested collection");
}

async fn wait_for_service(adapter_log: &Path) -> zbus::Connection {
    let connection = zbus::Connection::session()
        .await
        .expect("connect to private D-Bus session");
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        let proxy = ObjectManagerProxy::builder(&connection)
            .destination(DESTINATION)
            .expect("valid destination")
            .path(ROOT)
            .expect("valid ObjectManager path")
            .build()
            .await;
        if let Ok(proxy) = proxy
            && proxy
                .get_managed_objects()
                .await
                .is_ok_and(|objects| !objects.is_empty())
        {
            let manager = zbus::Proxy::new(
                &connection,
                DESTINATION,
                ROOT,
                "org.luminate.Luminate1.Manager2",
            )
            .await;
            if let Ok(manager) = manager
                && manager
                    .get_property::<bool>("Available")
                    .await
                    .unwrap_or(false)
            {
                return connection;
            }
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for demo topology; adapter log:\n{}",
            fs::read_to_string(adapter_log).unwrap_or_default()
        );
        sleep(POLL_INTERVAL).await;
    }
}

async fn target_proxy<'a>(connection: &'a zbus::Connection, path: &'a str) -> zbus::Proxy<'a> {
    zbus::Proxy::new(connection, DESTINATION, path, "org.luminate.Target2")
        .await
        .expect("build target proxy")
}

fn target_path(
    device: &str,
    surface: Option<&str>,
    element: Option<&str>,
    group: Option<&str>,
) -> String {
    use std::fmt::Write as _;

    let encode = |value: &str| {
        let mut encoded = String::from('x');
        for byte in value.bytes() {
            let _ = write!(encoded, "{byte:02x}");
        }
        encoded
    };

    let mut path = format!("{ROOT}/devices/{}", encode(device));
    if let Some(surface) = surface {
        let _ = write!(path, "/surfaces/{}", encode(surface));
    }
    if let Some(element) = element {
        let _ = write!(path, "/elements/{}", encode(element));
    }
    if let Some(group) = group {
        let _ = write!(path, "/groups/{}", encode(group));
    }
    path
}

fn caller_group() -> &'static str {
    static GROUP: OnceLock<String> = OnceLock::new();
    GROUP.get_or_init(|| {
        fs::read_to_string("/proc/self/status")
            .expect("read caller process status")
            .lines()
            .find_map(|line| line.strip_prefix("Groups:"))
            .and_then(|groups| groups.split_whitespace().next())
            .expect("caller should belong to a group")
            .to_owned()
    })
}

fn wait_for_log_contains(log_path: &Path, needle: &str) {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        let log = fs::read_to_string(log_path).unwrap_or_default();
        if log.contains(needle) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {needle:?} in {}\n{log}",
            log_path.display()
        );
        thread::sleep(POLL_INTERVAL);
    }
}

async fn assert_dbus_topology(adapter_log: &Path) {
    let connection = zbus::Connection::session()
        .await
        .expect("connect to private D-Bus session");
    let deadline = Instant::now() + WAIT_TIMEOUT;
    let managed = loop {
        let proxy = ObjectManagerProxy::builder(&connection)
            .destination(DESTINATION)
            .expect("valid destination")
            .path(ROOT)
            .expect("valid ObjectManager path")
            .build()
            .await;
        if let Ok(proxy) = proxy
            && let Ok(objects) = proxy.get_managed_objects().await
            && !objects.is_empty()
        {
            break objects;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for demo topology; adapter log:\n{}",
            fs::read_to_string(adapter_log).unwrap_or_default()
        );
        sleep(POLL_INTERVAL).await;
    };

    assert_demo_objects(&managed);
    let manager_proxy = zbus::Proxy::new(
        &connection,
        DESTINATION,
        ROOT,
        "org.luminate.Luminate1.Manager2",
    )
    .await
    .expect("build Manager2 proxy");
    let devices = manager_proxy
        .call::<_, _, Vec<Dictionary>>("ListDevices", &())
        .await
        .expect("list complete device snapshots");
    assert!(
        !devices.is_empty(),
        "the demo should expose device snapshots"
    );
    let keyboard = manager_proxy
        .call::<_, _, Dictionary>("GetDevice", &("demo-keyboard",))
        .await
        .expect("get demo keyboard snapshot");
    assert!(
        bool::try_from(
            keyboard
                .get("Found")
                .expect("device lookup should contain Found")
                .try_clone()
                .expect("clone device Found flag")
        )
        .expect("Found should be boolean"),
        "the demo keyboard should have a complete snapshot"
    );
    let state = manager_proxy
        .call::<_, _, Dictionary>("GetDeviceState", &("demo-keyboard",))
        .await
        .expect("get structured demo keyboard state");
    assert!(
        state.contains_key("HasState"),
        "device state lookup should report whether a snapshot exists"
    );
    for (path, interfaces) in &managed {
        let proxy = IntrospectableProxy::builder(&connection)
            .destination(DESTINATION)
            .expect("valid destination")
            .path(path)
            .expect("managed object path")
            .build()
            .await
            .expect("build Introspectable proxy");
        let xml = proxy.introspect().await.expect("introspect managed object");
        for interface in interfaces.keys() {
            let name = interface.as_str();
            assert!(
                xml.contains(&format!("interface name=\"{name}\"")),
                "{path} enumeration includes {name}, but its introspection XML does not"
            );
        }
    }
}

fn assert_demo_objects(managed: &ManagedObjects) {
    let keyboard = "/org/luminate/Luminate1/devices/x64656d6f2d6b6579626f617264";
    let keyboard_interfaces = managed
        .iter()
        .find_map(|(path, interfaces)| (path.as_str() == keyboard).then_some(interfaces))
        .expect("ObjectManager should enumerate the demo keyboard");
    assert!(
        keyboard_interfaces
            .keys()
            .any(|name| name.as_str() == "org.luminate.Target2"),
        "demo keyboard should expose the common target interface"
    );
    assert!(
        keyboard_interfaces
            .keys()
            .any(|name| name.as_str() == "org.luminate.Target3"),
        "demo keyboard should expose the complete target interface"
    );
    assert!(
        keyboard_interfaces
            .keys()
            .any(|name| name.as_str() == "org.luminate.Luminate1.Device1"),
        "demo keyboard should expose the device interface"
    );
    assert!(
        keyboard_interfaces
            .keys()
            .any(|name| name.as_str() == "org.luminate.Luminate1.Device2"),
        "demo keyboard should expose the complete device interface"
    );
    assert!(
        managed
            .keys()
            .any(|path| path.as_str().contains("/surfaces/")),
        "demo topology should enumerate surfaces"
    );
    assert!(
        managed.values().any(|interfaces| interfaces
            .keys()
            .any(|name| name.as_str() == "org.luminate.Luminate1.Surface2")),
        "demo topology should expose complete surface interfaces"
    );
    assert!(
        managed
            .keys()
            .any(|path| path.as_str().contains("/elements/")),
        "demo topology should enumerate elements"
    );
    assert!(
        managed.values().any(|interfaces| interfaces
            .keys()
            .any(|name| name.as_str() == "org.luminate.Luminate1.Element2")),
        "demo topology should expose complete element interfaces"
    );
    assert!(
        managed.values().any(|interfaces| interfaces
            .iter()
            .find(|(name, _)| name.as_str() == "org.luminate.Luminate1.Element2")
            .is_some_and(|(_, properties)| properties.contains_key("PhysicalTags"))),
        "Element2 should expose element-scoped physical tags"
    );
    assert!(
        managed
            .keys()
            .any(|path| path.as_str().contains("/groups/")),
        "demo topology should enumerate groups"
    );
    assert!(
        managed.values().any(|interfaces| interfaces
            .keys()
            .any(|name| name.as_str() == "org.luminate.Luminate1.Group2")),
        "demo topology should expose complete group interfaces"
    );
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize workspace root")
}

fn build_demo_stack(workspace: &Path) {
    let mut command = Command::new("cargo");
    command.args([
        "build",
        "-p",
        "luminated",
        "-p",
        "luminate-dbus",
        "-p",
        "luminate-plugin-demo-system",
    ]);
    if env::var_os("CARGO_LLVM_COV").is_some() {
        command.arg("--target-dir").arg(
            debug_artifact_dir(workspace)
                .parent()
                .expect("debug directory has a parent"),
        );
    }

    let status = command
        .current_dir(workspace)
        .status()
        .expect("build D-Bus demo stack");
    assert!(status.success(), "D-Bus demo stack build failed");
}

fn debug_artifact_dir(workspace: &Path) -> PathBuf {
    if env::var_os("CARGO_LLVM_COV").is_some() {
        return Path::new(env!("CARGO_BIN_EXE_luminate-dbus"))
            .parent()
            .expect("coverage adapter binary has a parent directory")
            .to_owned();
    }

    workspace.join("target/debug")
}

fn write_daemon_config(config: &Path, socket: &Path, state: &Path, plugin: &Path) {
    let quoted =
        |path: &Path| serde_json::to_string(&path.display().to_string()).expect("quote path");
    let (authority, subject, registration) = caller_identity();
    fs::write(
        config,
        format!(
            "socket_path = {}\nstate_path = {}\n\n[plugin_management]\nactivation = \"explicit\"\n\n{registration}\n\n[[plugins]]\npath = {}\nrequired = true\n",
            quoted(socket),
            quoted(state),
            quoted(plugin)
        ),
    )
    .expect("write daemon config");

    let mut policy = materialize_presets(PolicyRevision(1), [Preset::Operator]);
    policy.bindings.push(Binding {
        authority,
        subjects: BTreeSet::from([subject]),
        groups: BTreeSet::new(),
        roles: BTreeSet::from([Preset::Operator.name().to_owned()]),
    });
    let policy_path = state.parent().expect("state parent").join("policy.json");
    let mut policy_file = create_private_file(&policy_path).expect("create test access policy");
    policy_file
        .write_all(&serde_json::to_vec_pretty(&policy).expect("serialize test access policy"))
        .expect("write test access policy");
}

#[cfg(unix)]
fn caller_identity() -> (String, String, String) {
    use std::os::unix::fs::MetadataExt as _;

    let uid = fs::metadata("/proc/self")
        .expect("read caller process metadata")
        .uid()
        .to_string();
    (
        "unix".to_owned(),
        uid.clone(),
        format!("[[authorization.frontends]]\nplatform = \"unix\"\nuid = {uid}"),
    )
}

#[cfg(windows)]
fn caller_identity() -> (String, String, String) {
    let sid = current_process_sid().expect("read caller process SID");
    let quoted_sid = serde_json::to_string(&sid).expect("quote caller SID");
    (
        "windows".to_owned(),
        sid,
        format!("[[authorization.frontends]]\nplatform = \"windows\"\nsid = {quoted_sid}"),
    )
}

fn spawn_logged(
    executable: PathBuf,
    args: &[&str],
    environment: &[(&str, &OsStr)],
    log_path: &Path,
) -> Child {
    let stdout = File::options()
        .create(true)
        .append(true)
        .open(log_path)
        .expect("open process log");
    let stderr = stdout.try_clone().expect("clone process log");
    let mut command = Command::new(executable);
    command.args(args).envs(environment.iter().copied());
    command
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("spawn test process")
}

fn wait_for_path(path: &Path, description: &str) {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        thread::sleep(POLL_INTERVAL);
    }
    let daemon_log = path
        .parent()
        .map(|parent| fs::read_to_string(parent.join("luminated.log")).unwrap_or_default())
        .unwrap_or_default();
    panic!(
        "timed out waiting for {description} at {}\nluminated log:\n{daemon_log}",
        path.display()
    );
}

fn temp_dir() -> TestDir {
    TestDir::new("dbus-it")
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let pid = self.0.id().to_string();
        let _ = Command::new("kill").args(["-INT", &pid]).status();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.0.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(POLL_INTERVAL);
        }

        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
