// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! End-to-end CLI tests against an in-process protocol peer.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::missing_assert_message,
    clippy::panic,
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::tests_outside_test_module,
    clippy::too_many_lines,
    reason = "This process-level integration suite uses compact JSON fixture assertions and \
              intentionally fails loudly when setup assumptions break."
)]

use std::env;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{self, Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use luminate_platform::test_support::TestDir;
use luminate_platform::{dynamic_library_candidates, executable_name};
use serde_json::{Value, json};

const WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[test]
fn missing_command_help_uses_structural_line_breaks() {
    let cli = PathBuf::from(env!("CARGO_BIN_EXE_luminatectl"));
    let output = Command::new(cli)
        .output()
        .expect("run luminatectl without a command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("CLI stderr is UTF-8");
    assert!(
        stderr.contains("\u{1b}["),
        "missing-command help has no ANSI styling: {stderr:?}"
    );
    assert!(stderr.contains("Control lighting devices through luminated\n\n"));
    assert!(stderr.contains("Usage:"));
    assert!(
        !stderr.contains("luminated\\n\\nUsage:"),
        "missing-command help contains escaped line feeds: {stderr:?}"
    );
}

#[test]
fn nested_subcommand_help_retains_colour_and_line_breaks() {
    let cli = PathBuf::from(env!("CARGO_BIN_EXE_luminatectl"));
    let output = Command::new(cli)
        .arg("collection")
        .output()
        .expect("run a command without its required subcommand");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("CLI stderr is UTF-8");
    assert!(
        stderr.contains("\u{1b}["),
        "subcommand help has no ANSI styling: {stderr:?}"
    );
    assert!(stderr.contains("Manage user-created target collections\n\n"));
    assert!(stderr.contains("Usage:"));
    assert!(!stderr.contains("collections\\n\\nUsage:"));
}

#[test]
fn ordinary_parse_errors_retain_colour_and_line_breaks() {
    let cli = PathBuf::from(env!("CARGO_BIN_EXE_luminatectl"));
    let output = Command::new(cli)
        .args(["inspect", "alienware-aw-elc"])
        .output()
        .expect("run luminatectl with an unexpected argument");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("CLI stderr is UTF-8");
    assert!(
        stderr.contains("\u{1b}["),
        "parse error has no ANSI styling: {stderr:?}"
    );
    assert!(
        stderr.contains("found\n\n"),
        "parse error lost line breaks: {stderr:?}"
    );
    assert!(!stderr.contains("found\\n\\nUsage:"));
}

#[test]
fn command_line_errors_escape_terminal_controls() {
    let cli = PathBuf::from(env!("CARGO_BIN_EXE_luminatectl"));
    let output = Command::new(cli)
        .arg("unknown\n\u{1b}]0;title\u{7}\u{9b}31m\u{7f}")
        .output()
        .expect("run luminatectl with a hostile argument");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("CLI stderr is UTF-8");
    assert!(
        stderr.contains("\u{1b}["),
        "parse error has no ANSI styling: {stderr:?}"
    );
    assert!(
        !stderr.contains("\u{1b}]0;title")
            && !stderr.contains('\u{9b}')
            && !stderr.contains('\u{7f}'),
        "stderr contains a user-supplied terminal control: {stderr:?}"
    );
    assert!(
        stderr.contains("\\n"),
        "missing escaped line feed: {stderr:?}"
    );
    assert!(
        stderr.contains("\\u{9b}"),
        "missing escaped CSI: {stderr:?}"
    );
}

#[test]
#[ignore = "spawns workspace binaries and requires the LIFX plugin artifact"]
fn plugin_management_commands_round_trip_and_reject_stale_revisions() {
    let _guard = test_lock().lock().expect("lock poisoned");
    let workspace = workspace_root();
    build_management_stack(&workspace);

    let temp = temp_dir();
    let socket_path = temp_socket_path(&temp);
    let log_path = temp.join("luminated.log");
    let config_path = temp.join("luminated.toml");
    let state_path = temp_state_path(&temp);
    let plugin_path = workspace
        .join(format!(
            "target/debug/{}",
            dynamic_library_candidates("luminate_plugin_lifx")[0]
        ))
        .display()
        .to_string();

    write_daemon_config(&config_path, &socket_path, &state_path, &[plugin_path]);

    let _daemon = DaemonHarness::spawn(&workspace, &config_path, &socket_path, &log_path);

    let listed = run_cli(&workspace, &socket_path, &["plugin", "list", "--json"]);
    let listed: Value = serde_json::from_slice(&listed.stdout).expect("parse plugin list");
    assert_eq!(listed["revision"], 0);
    assert_eq!(listed["plugins"][0]["name"], "luminate-plugin-lifx");

    let listed_human = run_cli(&workspace, &socket_path, &["plugin", "list"]);
    assert!(String::from_utf8_lossy(&listed_human.stdout).contains("luminate-plugin-lifx"));
    let shown_human = run_cli(
        &workspace,
        &socket_path,
        &["plugin", "show", "luminate-plugin-lifx"],
    );
    assert!(String::from_utf8_lossy(&shown_human.stdout).contains("discovery_address"));

    let disabled = run_cli(
        &workspace,
        &socket_path,
        &[
            "plugin",
            "disable",
            "luminate-plugin-lifx",
            "--revision",
            "0",
            "--json",
        ],
    );
    let disabled: Value =
        serde_json::from_slice(&disabled.stdout).expect("parse activation change");
    assert_eq!(disabled["revision"], 1);
    assert_eq!(
        disabled["changes"][0]["PluginActivationChanged"]["plugin"],
        "luminate-plugin-lifx"
    );

    let stale = run_cli_output(
        &workspace,
        &socket_path,
        &["plugin", "reset", "luminate-plugin-lifx", "--revision", "0"],
    );
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("conflict"));

    let reset = run_cli(
        &workspace,
        &socket_path,
        &["plugin", "reset", "luminate-plugin-lifx", "--revision", "1"],
    );
    assert!(String::from_utf8_lossy(&reset.stdout).contains("plugin activation changed"));
    let enabled = run_cli(
        &workspace,
        &socket_path,
        &[
            "plugin",
            "enable",
            "luminate-plugin-lifx",
            "--revision",
            "2",
        ],
    );
    assert!(String::from_utf8_lossy(&enabled.stdout).contains("plugin activation changed"));

    let setting_value = "\"127.0.0.1:56701\"";
    let configured = run_cli_with_stdin(
        &workspace,
        &socket_path,
        &[
            "config",
            "set",
            "luminate-plugin-lifx",
            "discovery_address",
            "--revision",
            "3",
            "--json",
        ],
        setting_value,
    );
    assert!(
        !String::from_utf8_lossy(&configured.stdout).contains("127.0.0.1"),
        "committed change output must not echo write-only setting values"
    );
    let configured: Value =
        serde_json::from_slice(&configured.stdout).expect("parse setting change");
    assert_eq!(configured["revision"], 4);

    let shown = run_cli(
        &workspace,
        &socket_path,
        &["plugin", "show", "luminate-plugin-lifx", "--json"],
    );
    let shown: Value = serde_json::from_slice(&shown.stdout).expect("parse plugin detail");
    assert_eq!(shown["revision"], 4);
    assert_eq!(
        shown["plugin"]["desired_settings"]["discovery_address"]["Visible"]["String"],
        "127.0.0.1:56701"
    );

    let cleared = run_cli(
        &workspace,
        &socket_path,
        &[
            "config",
            "clear",
            "luminate-plugin-lifx",
            "discovery_address",
            "--revision",
            "4",
        ],
    );
    assert!(String::from_utf8_lossy(&cleared.stdout).contains("plugin setting changed"));
    let reconciliation = run_cli(
        &workspace,
        &socket_path,
        &[
            "config",
            "set-reconciliation",
            "luminate-plugin-lifx",
            "restore",
            "--revision",
            "5",
        ],
    );
    assert!(
        String::from_utf8_lossy(&reconciliation.stdout).contains("plugin reconciliation changed")
    );
    let _ = run_cli(
        &workspace,
        &socket_path,
        &[
            "config",
            "clear-reconciliation",
            "luminate-plugin-lifx",
            "--revision",
            "6",
        ],
    );
    let daemon_set = run_cli(
        &workspace,
        &socket_path,
        &[
            "config",
            "daemon",
            "set",
            "prefer-shm",
            "true",
            "--revision",
            "7",
        ],
    );
    assert!(String::from_utf8_lossy(&daemon_set.stdout).contains("daemon preferences changed"));
    let daemon_clear = run_cli(
        &workspace,
        &socket_path,
        &[
            "config",
            "daemon",
            "clear",
            "prefer-shm",
            "--revision",
            "8",
            "--json",
        ],
    );
    let daemon_clear: Value =
        serde_json::from_slice(&daemon_clear.stdout).expect("parse daemon preference change");
    assert_eq!(daemon_clear["revision"], 9);

    let managed =
        fs::read_to_string(temp.join("managed.toml")).expect("read managed configuration");
    assert!(managed.contains("revision = 9"));
    assert!(!managed.contains("127.0.0.1:56701"));
}

#[test]
#[ignore = "spawns workspace binaries and requires the demo-bulb plugin artifact"]
fn managed_plugin_disable_withdraws_topology_and_enable_restores_it() {
    let _guard = test_lock().lock().expect("lock poisoned");
    let workspace = workspace_root();
    build_management_stack(&workspace);

    let temp = temp_dir();
    let socket_path = temp_socket_path(&temp);
    let log_path = temp.join("luminated.log");
    let config_path = temp.join("luminated.toml");
    let state_path = temp_state_path(&temp);
    let plugin_path = workspace
        .join(format!(
            "target/debug/{}",
            dynamic_library_candidates("luminate_plugin_demo_bulb")[0]
        ))
        .display()
        .to_string();
    write_daemon_config(&config_path, &socket_path, &state_path, &[plugin_path]);
    let config = fs::read_to_string(&config_path).expect("read daemon configuration");
    fs::write(
        &config_path,
        config.replace("required = true", "required = false"),
    )
    .expect("make demo plugin administratively disableable");

    let _daemon = DaemonHarness::spawn(&workspace, &config_path, &socket_path, &log_path);
    assert_device_presence(&workspace, &socket_path, "demo-smart-bulb", true);

    let _ = run_cli(
        &workspace,
        &socket_path,
        &[
            "plugin",
            "disable",
            "luminate-plugin-demo-bulb",
            "--revision",
            "0",
        ],
    );
    assert_device_presence(&workspace, &socket_path, "demo-smart-bulb", false);

    let _ = run_cli(
        &workspace,
        &socket_path,
        &[
            "plugin",
            "enable",
            "luminate-plugin-demo-bulb",
            "--revision",
            "1",
        ],
    );
    assert_device_presence(&workspace, &socket_path, "demo-smart-bulb", true);
}

#[test]
#[ignore = "spawns workspace binaries and requires the demo-bulb plugin artifact"]
fn transitions_complete_replace_abort_and_retain_terminal_status() {
    use luminate::Client;
    use luminate_core::colour::Colour;
    use luminate_core::effect::Effect;
    use luminate_core::rgb::Rgb;
    use luminate_core::scene::SceneTargetState;
    use luminate_core::state::EmissionState;
    use luminate_core::target::TargetId;
    use luminate_core::transition::{
        TransitionCancellation, TransitionOptions, TransitionOutcome, TransitionTargetState,
    };
    use tokio::runtime::Builder;

    let _guard = test_lock().lock().expect("lock poisoned");
    let workspace = workspace_root();
    build_management_stack(&workspace);

    let temp = temp_dir();
    let socket_path = temp_socket_path(&temp);
    let log_path = temp.join("luminated.log");
    let config_path = temp.join("luminated.toml");
    let state_path = temp_state_path(&temp);
    let plugin_path = workspace
        .join(format!(
            "target/debug/{}",
            dynamic_library_candidates("luminate_plugin_demo_bulb")[0]
        ))
        .display()
        .to_string();
    write_daemon_config(&config_path, &socket_path, &state_path, &[plugin_path]);
    let _daemon = DaemonHarness::spawn(&workspace, &config_path, &socket_path, &log_path);

    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build transition test runtime");
    runtime.block_on(async {
        let client = Client::connect_path(&socket_path)
            .await
            .expect("connect transition client");
        let target = TargetId::device("demo-smart-bulb");
        client
            .set_rgb(target.clone(), 10, 20, 30)
            .await
            .expect("prime a fresh current appearance");
        client
            .set_brightness(target.clone(), 20)
            .await
            .expect("prime a fresh current brightness");

        let destination = |colour: Rgb, brightness| {
            vec![TransitionTargetState {
                target: target.clone(),
                state: SceneTargetState {
                    appearance: Some(Effect::Static {
                        colour: Colour::rgb(colour),
                    }),
                    brightness: Some(brightness),
                    emission: Some(EmissionState::Emitting),
                    appearance_slots: None,
                },
            }]
        };
        let long =
            TransitionOptions::new(Duration::from_millis(500), Some(Duration::from_millis(50)))
                .expect("valid long transition");
        let short =
            TransitionOptions::new(Duration::from_millis(60), Some(Duration::from_millis(10)))
                .expect("valid short transition");

        let first = client
            .transitions()
            .current_to_states(destination(Rgb::new(100, 20, 30), 60), long)
            .await
            .expect("start first transition");
        assert!(
            client
                .transitions()
                .get(first.id.clone())
                .await
                .expect("fetch active transition")
                .outcome
                .is_none(),
            "the first transition should initially be active"
        );

        let replacement = client
            .transitions()
            .current_to_states(destination(Rgb::new(40, 80, 120), 80), short)
            .await
            .expect("start replacement transition");
        let replaced = client
            .transitions()
            .wait(first.id)
            .await
            .expect("wait for replaced transition");
        assert_eq!(
            replaced.outcome,
            Some(TransitionOutcome::Cancelled(
                TransitionCancellation::Replaced
            )),
            "overlapping transition should retain its replacement reason"
        );

        let completed = client
            .transitions()
            .wait(replacement.id.clone())
            .await
            .expect("wait for replacement completion");
        assert_eq!(
            completed.outcome,
            Some(TransitionOutcome::Completed),
            "replacement should reach its exact destination"
        );
        assert_eq!(
            client
                .transitions()
                .get(replacement.id)
                .await
                .expect("fetch retained terminal transition")
                .outcome,
            Some(TransitionOutcome::Completed),
            "completed status should remain available"
        );

        let aborting = client
            .transitions()
            .current_to_states(destination(Rgb::new(200, 10, 10), 30), long)
            .await
            .expect("start transition to abort");
        let aborted = client
            .transitions()
            .abort(aborting.id)
            .await
            .expect("abort active transition");
        assert_eq!(
            aborted.outcome,
            Some(TransitionOutcome::Cancelled(
                TransitionCancellation::Aborted
            )),
            "abort should wait for a retained terminal status"
        );
    });
}

#[test]
#[ignore = "spawns workspace binaries and requires the demo plugin artifact"]
fn list_json_matches_demo_snapshot() {
    let _guard = test_lock().lock().expect("lock poisoned");
    let workspace = workspace_root();
    build_demo_stack(&workspace);

    let temp = temp_dir();
    let socket_path = temp_socket_path(&temp);
    let log_path = temp.join("luminated.log");
    let config_path = temp.join("luminated.toml");
    let plugin_path = workspace
        .join(format!(
            "target/debug/{}",
            dynamic_library_candidates("luminate_plugin_demo_system")[0]
        ))
        .display()
        .to_string();

    write_daemon_config(
        &config_path,
        &socket_path,
        &temp_state_path(&temp),
        &[plugin_path],
    );

    let _daemon = DaemonHarness::spawn(&workspace, &config_path, &socket_path, &log_path);

    let output = run_cli(&workspace, &socket_path, &["list", "--json"]);
    let parsed: Value = serde_json::from_slice(&output.stdout).expect("parse list json");
    let summary = topology_summary(&parsed);
    let expected: Value =
        serde_json::from_str(&expected_summary_snapshot()).expect("parse expected summary");

    assert_eq!(summary, expected);
    assert_typed_list_views(&workspace, &socket_path, &temp);
}

#[test]
#[ignore = "spawns workspace binaries and requires the demo plugin artifact"]
fn mutations_reach_the_demo_plugin() {
    let _guard = test_lock().lock().expect("lock poisoned");
    let workspace = workspace_root();
    build_demo_stack(&workspace);

    let temp = temp_dir();
    let socket_path = temp_socket_path(&temp);
    let log_path = temp.join("luminated.log");
    let config_path = temp.join("luminated.toml");
    let plugin_path = workspace
        .join(format!(
            "target/debug/{}",
            dynamic_library_candidates("luminate_plugin_demo_system")[0]
        ))
        .display()
        .to_string();

    write_daemon_config(
        &config_path,
        &socket_path,
        &temp_state_path(&temp),
        &[plugin_path],
    );

    let _daemon = DaemonHarness::spawn(&workspace, &config_path, &socket_path, &log_path);

    let colour = run_cli(
        &workspace,
        &socket_path,
        &[
            "set-effect",
            "--effect",
            "static",
            "--device",
            "demo-keyboard",
            "--surface",
            "zones",
            "--element",
            "g1",
            "--rgb",
            "rgb(12, 34, 56)",
        ],
    );
    assert_eq!(String::from_utf8_lossy(&colour.stdout).trim(), "ok");

    let brightness = run_cli(
        &workspace,
        &socket_path,
        &[
            "set-brightness",
            "--device",
            "demo-case-lights",
            "--group",
            "chassis",
            "85",
        ],
    );
    assert_eq!(String::from_utf8_lossy(&brightness.stdout).trim(), "ok");

    let effect = run_cli(
        &workspace,
        &socket_path,
        &[
            "set-effect",
            "--device",
            "demo-mouse",
            "--surface",
            "lighting",
            "--element",
            "wheel",
            "--effect",
            "static",
            "--rgb",
            "rgb(90, 10, 200)",
        ],
    );
    assert_eq!(String::from_utf8_lossy(&effect.stdout).trim(), "ok");

    assert_status_led_capability_rejections(&workspace, &socket_path);

    wait_for_log_contains(&log_path, "plugin update applied");
    wait_for_log_contains(&log_path, "plugin=luminate-plugin-demo-system");
    wait_for_log_contains(&log_path, "target=demo-keyboard/zones/g1");
    wait_for_log_contains(&log_path, "operation=set-effect");
    wait_for_log_contains(&log_path, "target=demo-case-lights/group:chassis");
    wait_for_log_contains(&log_path, "operation=set-brightness");
    wait_for_log_contains(&log_path, "target=demo-mouse/lighting/wheel");
    wait_for_log_contains(&log_path, "operation=set-effect");
}

#[test]
#[ignore = "spawns workspace binaries and requires demo plugin artifacts"]
fn rescan_re_enumerates_every_loaded_plugin() {
    // Exercise the same rescan chain used after resume, without suspending.
    let _guard = test_lock().lock().expect("lock poisoned");
    let workspace = workspace_root();
    build_demo_stack(&workspace);

    let temp = temp_dir();
    let socket_path = temp_socket_path(&temp);
    let log_path = temp.join("luminated.log");
    let config_path = temp.join("luminated.toml");
    let plugin_path = workspace
        .join(format!(
            "target/debug/{}",
            dynamic_library_candidates("luminate_plugin_demo_system")[0]
        ))
        .display()
        .to_string();

    write_daemon_config(
        &config_path,
        &socket_path,
        &temp_state_path(&temp),
        &[plugin_path],
    );

    let _daemon = DaemonHarness::spawn(&workspace, &config_path, &socket_path, &log_path);

    let rescan = run_cli(&workspace, &socket_path, &["rescan"]);
    assert_eq!(
        String::from_utf8_lossy(&rescan.stdout).trim(),
        "rescan scheduled"
    );

    wait_for_log_contains(&log_path, "client requested a hardware rescan");
    wait_for_log_contains(&log_path, "rescanning every loaded plugin's topology");

    // Ownership is re-arbitrated after each real topology pull.
    let ownership_summaries = wait_for_log_occurrences(&log_path, "plugin ownership summary", 2);
    assert!(
        ownership_summaries >= 2,
        "rescan should re-pull the plugin's topology"
    );

    // Stable topology must not suppress post-resume hardware reconciliation.
    wait_for_log_contains(&log_path, "plugin topology reconciled");
    let log = fs::read_to_string(&log_path).expect("read daemon log");
    assert!(
        log.contains("changed_devices=[]"),
        "the demo topology should be stable across a rescan"
    );
    assert!(
        log.contains("reconciled_devices=[") && !log.contains("reconciled_devices=[]"),
        "a rescan must reconcile owned devices despite an unchanged topology"
    );
    for device in ["demo-keyboard", "demo-case-lights", "demo-mouse"] {
        assert!(
            log.contains(device),
            "every owned device should be reconciled, missing {device}"
        );
    }

    let devices = run_cli(&workspace, &socket_path, &["list"]);
    assert!(
        String::from_utf8_lossy(&devices.stdout).contains("demo-keyboard"),
        "daemon should still serve topology after a rescan"
    );
}

#[test]
#[ignore = "spawns workspace binaries and requires demo plugin artifacts"]
fn mixed_demo_plugins_route_topology_and_mutations_independently() {
    let _guard = test_lock().lock().expect("lock poisoned");
    let workspace = workspace_root();
    build_demo_stack(&workspace);

    let temp = temp_dir();
    let socket_path = temp_socket_path(&temp);
    let log_path = temp.join("luminated.log");
    let config_path = temp.join("luminated.toml");
    let demo_system_path = workspace
        .join(format!(
            "target/debug/{}",
            dynamic_library_candidates("luminate_plugin_demo_system")[0]
        ))
        .display()
        .to_string();
    let demo_keyboard_path = workspace
        .join(format!(
            "target/debug/{}",
            dynamic_library_candidates("luminate_plugin_demo_keyboard")[0]
        ))
        .display()
        .to_string();

    write_daemon_config(
        &config_path,
        &socket_path,
        &temp_state_path(&temp),
        &[demo_system_path, demo_keyboard_path],
    );

    let _daemon = DaemonHarness::spawn(&workspace, &config_path, &socket_path, &log_path);

    assert_mixed_topology(&workspace, &socket_path);
    assert_mixed_mutations_route_independently(&workspace, &socket_path);
    assert_mixed_mutation_logs(&log_path);
}

#[test]
#[ignore = "spawns workspace binaries and requires demo plugin artifacts"]
fn hung_plugin_host_does_not_block_topology_or_another_plugin() {
    let _guard = test_lock().lock().expect("lock poisoned");
    let workspace = workspace_root();
    build_demo_stack(&workspace);
    let temp = temp_dir();
    let socket_path = temp_socket_path(&temp);
    let log_path = temp.join("luminated.log");
    let config_path = temp.join("luminated.toml");
    let demo_system = workspace
        .join(format!(
            "target/debug/{}",
            dynamic_library_candidates("luminate_plugin_demo_system")[0]
        ))
        .display()
        .to_string();
    let demo_keyboard = workspace
        .join(format!(
            "target/debug/{}",
            dynamic_library_candidates("luminate_plugin_demo_keyboard")[0]
        ))
        .display()
        .to_string();
    write_daemon_config(
        &config_path,
        &socket_path,
        &temp_state_path(&temp),
        &[demo_system, demo_keyboard],
    );
    let _daemon = DaemonHarness::spawn_with_env(
        &workspace,
        &config_path,
        &socket_path,
        &log_path,
        &[("LUMINATE_DEMO_PLUGIN_TEST_FAULT", "hang")],
    );

    let cli = PathBuf::from(env!("CARGO_BIN_EXE_luminatectl"));
    let hanging = Command::new(&cli)
        .arg("--socket-path")
        .arg(&socket_path)
        .args([
            "set-effect",
            "--effect",
            "static",
            "--device",
            "demo-keyboard",
            "--surface",
            "zones",
            "--element",
            "g1",
            "--rgb",
            "rgb(1, 2, 3)",
        ])
        .current_dir(&workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start mutation against hanging plugin");
    thread::sleep(Duration::from_millis(250));

    let started = Instant::now();
    assert_mixed_topology(&workspace, &socket_path);
    let other = run_cli(
        &workspace,
        &socket_path,
        &[
            "set-effect",
            "--effect",
            "static",
            "--device",
            "demo-keyboard-only",
            "--surface",
            "keys",
            "--key",
            "escape",
            "--rgb",
            "rgb(9, 8, 7)",
        ],
    );
    assert_eq!(String::from_utf8_lossy(&other.stdout).trim(), "ok");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "unrelated topology and mutation were delayed by the hanging host"
    );

    let hanging = hanging
        .wait_with_output()
        .expect("wait for bounded hanging mutation");
    assert!(!hanging.status.success(), "hung plugin mutation must fail");
    wait_for_log_contains(&log_path, "plugin host exited");
    let _ = run_cli(&workspace, &socket_path, &["list", "--json"]);
}

#[test]
#[ignore = "spawns workspace binaries and requires demo plugin artifacts"]
fn crashed_plugin_host_does_not_take_down_daemon_or_another_plugin() {
    let _guard = test_lock().lock().expect("lock poisoned");
    let workspace = workspace_root();
    build_demo_stack(&workspace);
    let temp = temp_dir();
    let socket_path = temp_socket_path(&temp);
    let log_path = temp.join("luminated.log");
    let config_path = temp.join("luminated.toml");
    let plugins = [
        workspace
            .join(format!(
                "target/debug/{}",
                dynamic_library_candidates("luminate_plugin_demo_system")[0]
            ))
            .display()
            .to_string(),
        workspace
            .join(format!(
                "target/debug/{}",
                dynamic_library_candidates("luminate_plugin_demo_keyboard")[0]
            ))
            .display()
            .to_string(),
    ];
    write_daemon_config(
        &config_path,
        &socket_path,
        &temp_state_path(&temp),
        &plugins,
    );
    let _daemon = DaemonHarness::spawn_with_env(
        &workspace,
        &config_path,
        &socket_path,
        &log_path,
        &[("LUMINATE_DEMO_PLUGIN_TEST_FAULT", "abort")],
    );

    let crashed = run_cli_output(
        &workspace,
        &socket_path,
        &[
            "set-effect",
            "--effect",
            "static",
            "--device",
            "demo-keyboard",
            "--surface",
            "zones",
            "--element",
            "g1",
            "--rgb",
            "rgb(1, 2, 3)",
        ],
    );
    assert!(
        !crashed.status.success(),
        "crashed plugin mutation must fail"
    );
    wait_for_log_contains(&log_path, "plugin host exited");

    let other = run_cli(
        &workspace,
        &socket_path,
        &[
            "set-effect",
            "--effect",
            "static",
            "--device",
            "demo-keyboard-only",
            "--surface",
            "keys",
            "--key",
            "escape",
            "--rgb",
            "rgb(3, 2, 1)",
        ],
    );
    assert_eq!(String::from_utf8_lossy(&other.stdout).trim(), "ok");
    let _ = run_cli(&workspace, &socket_path, &["list", "--json"]);
}

#[test]
#[ignore = "spawns workspace binaries and requires the demo plugin artifact"]
fn crashed_plugin_host_restarts_and_reconciles_topology_on_next_call() {
    let _guard = test_lock().lock().expect("lock poisoned");
    let workspace = workspace_root();
    build_demo_stack(&workspace);
    let temp = temp_dir();
    let socket_path = temp_socket_path(&temp);
    let log_path = temp.join("luminated.log");
    let config_path = temp.join("luminated.toml");
    let marker_path = temp.join("abort-once.marker");
    let plugin = workspace
        .join(format!(
            "target/debug/{}",
            dynamic_library_candidates("luminate_plugin_demo_system")[0]
        ))
        .display()
        .to_string();
    write_daemon_config(
        &config_path,
        &socket_path,
        &temp_state_path(&temp),
        &[plugin],
    );
    let marker = marker_path.display().to_string();
    let _daemon = DaemonHarness::spawn_with_env(
        &workspace,
        &config_path,
        &socket_path,
        &log_path,
        &[
            ("RUST_LOG", "debug"),
            ("LUMINATE_DEMO_PLUGIN_TEST_FAULT", "abort-once"),
            ("LUMINATE_DEMO_PLUGIN_TEST_FAULT_MARKER", &marker),
        ],
    );
    let mutation = [
        "set-effect",
        "--effect",
        "static",
        "--device",
        "demo-keyboard",
        "--surface",
        "zones",
        "--element",
        "g1",
        "--rgb",
        "rgb(1, 2, 3)",
    ];

    let crashed = run_cli_output(&workspace, &socket_path, &mutation);
    assert!(
        !crashed.status.success(),
        "the injected first call must crash"
    );
    wait_for_log_contains(&log_path, "plugin host exited");

    let recovered = run_cli(&workspace, &socket_path, &mutation);
    assert_eq!(String::from_utf8_lossy(&recovered.stdout).trim(), "ok");
    wait_for_log_contains(&log_path, "plugin host restarted after failure");
    wait_for_log_contains(
        &log_path,
        "topology notification produced no owned-device change",
    );

    let topology = run_cli(
        &workspace,
        &socket_path,
        &["list", "--json", "--device", "demo-keyboard"],
    );
    let topology: Value = serde_json::from_slice(&topology.stdout).expect("parse topology");
    assert_eq!(topology.as_array().map(Vec::len), Some(1));
}

fn assert_mixed_topology(workspace: &Path, socket_path: &Path) {
    let output = run_cli(workspace, socket_path, &["list", "--json"]);
    let devices: Value = serde_json::from_slice(&output.stdout).expect("parse list json");
    let devices = devices
        .as_array()
        .expect("topology root should be an array");
    let device_ids = devices
        .iter()
        .map(|device| device["id"].clone())
        .collect::<Vec<_>>();
    assert!(
        devices
            .iter()
            .any(|device| device["id"] == "demo-keyboard-only"),
        "standalone demo keyboard should be present in mixed topology; got {device_ids:?}"
    );
    assert!(
        devices.iter().any(|device| device["id"] == "demo-keyboard"),
        "demo-system keyboard should remain present in mixed topology"
    );

    let standalone_only = run_cli(
        workspace,
        socket_path,
        &["list", "--json", "--device", "demo-keyboard-only"],
    );
    let standalone_only: Value =
        serde_json::from_slice(&standalone_only.stdout).expect("parse filtered list json");
    let standalone_only = standalone_only
        .as_array()
        .expect("filtered topology root should be an array");
    assert_eq!(
        standalone_only.len(),
        1,
        "filtered standalone keyboard topology should contain exactly one device"
    );
    assert_eq!(
        standalone_only.first().and_then(|device| device.get("id")),
        Some(&json!("demo-keyboard-only")),
        "filtered topology should return the standalone keyboard"
    );

    let keyboards = run_cli(
        workspace,
        socket_path,
        &["devices", "--json", "--category", "keyboard"],
    );
    let keyboards: Value =
        serde_json::from_slice(&keyboards.stdout).expect("parse category list json");
    let keyboards = keyboards
        .as_array()
        .expect("category topology root should be an array");
    assert!(
        keyboards.len() >= 2,
        "both demo keyboards should match keyboard category"
    );
}

fn assert_typed_list_views(workspace: &Path, socket_path: &Path, temp: &Path) {
    let collection_id = create_demo_collection(workspace, socket_path);

    assert_typed_list_schemas(workspace, socket_path);
    assert_filtered_element_view(workspace, socket_path);
    assert_collection_views(workspace, socket_path, &collection_id);
    assert_collection_and_scene_commands(workspace, socket_path, temp, &collection_id);
}

fn create_demo_collection(workspace: &Path, socket_path: &Path) -> String {
    let created = run_cli(
        workspace,
        socket_path,
        &[
            "collection",
            "create",
            "--name",
            "Demo collection",
            "--kind",
            "logical-grouping",
            "--member-device",
            "demo-keyboard",
        ],
    );
    String::from_utf8(created.stdout)
        .expect("collection id is UTF-8")
        .trim()
        .to_owned()
}

fn assert_typed_list_schemas(workspace: &Path, socket_path: &Path) {
    for (view, expected_fields) in [
        ("device", &["id", "name", "category", "host_attached"][..]),
        ("surface", &["device", "id", "name", "kind"][..]),
        ("element", &["device", "surface", "id", "name", "kind"][..]),
        ("group", &["device", "id", "name", "kind"][..]),
        ("collection", &["id", "name", "kind"][..]),
    ] {
        let output = run_cli(workspace, socket_path, &["list", view, "--json"]);
        let summaries: Value =
            serde_json::from_slice(&output.stdout).expect("parse typed list JSON");
        let records = summaries.as_array().expect("typed list root is an array");
        assert!(!records.is_empty(), "{view} list should not be empty");
        for record in records {
            let object = record.as_object().expect("typed list record is an object");
            assert_eq!(
                object.len(),
                expected_fields.len(),
                "{view} discovery records should contain only compact fields"
            );
            assert!(
                expected_fields
                    .iter()
                    .all(|field| object.contains_key(*field)),
                "unexpected {view} discovery schema: {object:?}"
            );
        }

        let human = run_cli(workspace, socket_path, &["list", view]);
        let human = String::from_utf8(human.stdout).expect("typed human output is UTF-8");
        assert!(
            human.lines().all(|line| !line.is_empty()),
            "{view} summaries should be one non-empty record per line"
        );
    }
}

fn assert_filtered_element_view(workspace: &Path, socket_path: &Path) {
    let elements = run_cli(
        workspace,
        socket_path,
        &[
            "list",
            "--device",
            "demo-keyboard",
            "--category",
            "keyboard",
            "element",
            "--json",
        ],
    );
    let elements: Value =
        serde_json::from_slice(&elements.stdout).expect("parse filtered element summaries");
    assert!(
        elements
            .as_array()
            .expect("element summaries are an array")
            .iter()
            .all(|element| element["device"] == "demo-keyboard"),
        "device and category filters should apply to nested element owners"
    );
}

fn assert_collection_views(workspace: &Path, socket_path: &Path, collection_id: &str) {
    let collections = run_cli(workspace, socket_path, &["list", "collection", "--json"]);
    let collections: Value =
        serde_json::from_slice(&collections.stdout).expect("parse collection summaries");
    assert!(
        collections
            .as_array()
            .expect("collections are an array")
            .iter()
            .any(|collection| collection["id"] == collection_id
                && collection["kind"] == "logical-grouping"),
        "compact collection discovery should include the created collection"
    );

    let full_collection = run_cli(workspace, socket_path, &["collection", "list"]);
    let full_collection: Value =
        serde_json::from_slice(&full_collection.stdout).expect("parse full collection list");
    let collection = full_collection
        .as_array()
        .expect("full collection list is an array")
        .iter()
        .find(|collection| collection["id"] == collection_id)
        .expect("created collection is present");
    assert!(
        collection.get("owner").is_some(),
        "full-fidelity collection list should retain owner"
    );
    assert!(
        collection.get("members").is_some(),
        "full-fidelity collection list should retain members"
    );
}

fn assert_collection_and_scene_commands(
    workspace: &Path,
    socket_path: &Path,
    temp: &Path,
    collection_id: &str,
) {
    let shown = run_cli(
        workspace,
        socket_path,
        &["collection", "show", collection_id],
    );
    let shown: Value = serde_json::from_slice(&shown.stdout).expect("parse shown collection");
    assert_eq!(shown["id"], collection_id);

    let _ = run_cli(
        workspace,
        socket_path,
        &[
            "collection",
            "add-member",
            collection_id,
            "--member-device",
            "demo-case-lights",
        ],
    );
    let _ = run_cli(
        workspace,
        socket_path,
        &[
            "collection",
            "remove-member",
            collection_id,
            "--member-device",
            "demo-keyboard",
        ],
    );

    let nested = run_cli(
        workspace,
        socket_path,
        &[
            "collection",
            "create",
            "--name",
            "Nested collection",
            "--member-collection",
            collection_id,
        ],
    );
    let nested_id = String::from_utf8(nested.stdout)
        .expect("nested collection id is UTF-8")
        .trim()
        .to_owned();
    let _ = run_cli(
        workspace,
        socket_path,
        &["collection", "destroy", &nested_id],
    );

    let _ = run_cli(
        workspace,
        socket_path,
        &["set-brightness", "--device", "demo-case-lights", "50"],
    );
    let captured = run_cli(
        workspace,
        socket_path,
        &[
            "scene",
            "capture",
            "Captured scene",
            "--description",
            "Captured by the CLI integration test",
            "--device",
            "demo-case-lights",
        ],
    );
    let captured: Value = serde_json::from_slice(&captured.stdout).expect("parse captured scene");
    let captured_id = captured["id"].as_str().expect("captured scene id");
    let captured_revision = captured["revision"]
        .as_u64()
        .expect("captured scene revision");

    let listed = run_cli(workspace, socket_path, &["scene", "list"]);
    let listed: Value = serde_json::from_slice(&listed.stdout).expect("parse scene list");
    assert!(
        listed
            .as_array()
            .expect("scene list is an array")
            .iter()
            .any(|scene| scene["id"] == captured_id)
    );
    let shown = run_cli(workspace, socket_path, &["scene", "show", captured_id]);
    let shown: Value = serde_json::from_slice(&shown.stdout).expect("parse shown scene");
    assert_eq!(shown["id"], captured_id);

    let revision = captured_revision.to_string();
    let recaptured = run_cli(
        workspace,
        socket_path,
        &[
            "scene",
            "recapture",
            captured_id,
            "--revision",
            &revision,
            "--collection",
            collection_id,
        ],
    );
    let recaptured: Value =
        serde_json::from_slice(&recaptured.stdout).expect("parse recaptured scene");
    let recaptured_revision = recaptured["revision"]
        .as_u64()
        .expect("recaptured scene revision");
    let _ = run_cli(workspace, socket_path, &["scene", "apply", captured_id]);

    let definition_path = temp.join("scene-definition.json");
    fs::write(
        &definition_path,
        serde_json::to_vec_pretty(&json!({
            "name": "Defined scene",
            "description": "Created from a typed definition",
            "bindings": captured["bindings"].clone(),
        }))
        .expect("serialize scene definition"),
    )
    .expect("write scene definition");
    let definition = definition_path.to_str().expect("definition path is UTF-8");
    let created = run_cli(workspace, socket_path, &["scene", "create", definition]);
    let created: Value = serde_json::from_slice(&created.stdout).expect("parse created scene");
    let created_id = created["id"].as_str().expect("created scene id");
    let created_revision = created["revision"]
        .as_u64()
        .expect("created scene revision")
        .to_string();

    let replaced = run_cli(
        workspace,
        socket_path,
        &[
            "scene",
            "replace",
            created_id,
            "--revision",
            &created_revision,
            definition,
        ],
    );
    let replaced: Value = serde_json::from_slice(&replaced.stdout).expect("parse replaced scene");
    let replaced_revision = replaced["revision"]
        .as_u64()
        .expect("replaced scene revision")
        .to_string();
    let _ = run_cli(
        workspace,
        socket_path,
        &[
            "scene",
            "delete",
            created_id,
            "--revision",
            &replaced_revision,
        ],
    );

    let recaptured_revision = recaptured_revision.to_string();
    let _ = run_cli(
        workspace,
        socket_path,
        &[
            "scene",
            "delete",
            captured_id,
            "--revision",
            &recaptured_revision,
        ],
    );

    let _ = run_cli(workspace, socket_path, &["ping"]);
    let version = run_cli(workspace, socket_path, &["version"]);
    assert!(String::from_utf8_lossy(&version.stdout).contains("protocol abi"));
    let state = run_cli(
        workspace,
        socket_path,
        &["state", "--device", "demo-case-lights"],
    );
    let _: Value = serde_json::from_slice(&state.stdout).expect("parse device state");
    let _ = run_cli(
        workspace,
        socket_path,
        &[
            "inspect",
            "--device",
            "demo-case-lights",
            "--group",
            "chassis",
        ],
    );
    let _ = run_cli(
        workspace,
        socket_path,
        &["off", "--device", "demo-case-lights", "--group", "chassis"],
    );
    let _ = run_cli(
        workspace,
        socket_path,
        &[
            "clear",
            "--device",
            "demo-case-lights",
            "--group",
            "chassis",
        ],
    );
    let collection_state = run_cli(
        workspace,
        socket_path,
        &["state", "--collection", collection_id],
    );
    let _: Value =
        serde_json::from_slice(&collection_state.stdout).expect("parse collection state");

    let _ = run_cli(
        workspace,
        socket_path,
        &["collection", "destroy", collection_id],
    );

    let all_off = run_cli_output(workspace, socket_path, &["all-off"]);
    assert!(String::from_utf8_lossy(&all_off.stdout).contains("devices processed without errors"));

    let unavailable_socket = temp.join("unavailable.sock");
    let unavailable_version = run_cli(workspace, &unavailable_socket, &["version"]);
    assert!(String::from_utf8_lossy(&unavailable_version.stdout).contains("luminated unavailable"));
}

fn assert_mixed_mutations_route_independently(workspace: &Path, socket_path: &Path) {
    let demo_system_mutation = run_cli(
        workspace,
        socket_path,
        &[
            "set-effect",
            "--effect",
            "static",
            "--device",
            "demo-keyboard",
            "--surface",
            "zones",
            "--element",
            "g1",
            "--rgb",
            "rgb(12, 34, 56)",
        ],
    );
    assert_eq!(
        String::from_utf8_lossy(&demo_system_mutation.stdout).trim(),
        "ok",
        "demo-system mutation should succeed"
    );

    let demo_keyboard_mutation = run_cli(
        workspace,
        socket_path,
        &[
            "set-effect",
            "--effect",
            "static",
            "--device",
            "demo-keyboard-only",
            "--surface",
            "keys",
            "--key",
            "escape",
            "--rgb",
            "rgb(90, 10, 200)",
        ],
    );
    assert_eq!(
        String::from_utf8_lossy(&demo_keyboard_mutation.stdout).trim(),
        "ok",
        "standalone demo keyboard mutation should succeed"
    );

    let inspect = run_cli(
        workspace,
        socket_path,
        &[
            "inspect",
            "--device",
            "demo-keyboard-only",
            "--surface",
            "keys",
            "--key",
            "escape",
        ],
    );
    let inspect_stdout = String::from_utf8_lossy(&inspect.stdout);
    assert!(
        inspect_stdout.contains("escape"),
        "inspect output should include the selected key id"
    );
    assert!(
        inspect_stdout.contains("[key]"),
        "inspect output should include the selected element kind"
    );
}

fn assert_mixed_mutation_logs(log_path: &Path) {
    wait_for_log_contains(log_path, "plugin=luminate-plugin-demo-system");
    wait_for_log_contains(log_path, "target=demo-keyboard/zones/g1");
    wait_for_log_contains(log_path, "plugin=luminate-plugin-demo-keyboard");
    wait_for_log_contains(log_path, "plugin ownership summary");
    wait_for_log_contains(log_path, "plugin load summary");
    wait_for_log_contains(log_path, "target=demo-keyboard-only/keys/escape");
}

fn assert_status_led_capability_rejections(workspace: &Path, socket_path: &Path) {
    for args in [
        vec![
            "set-effect",
            "--effect",
            "static",
            "--device",
            "demo-status-led",
            "--rgb",
            "rgb(1, 2, 3)",
        ],
        vec![
            "set-brightness",
            "--device",
            "demo-status-led",
            "4294967295",
        ],
    ] {
        let rejected = run_cli_output(workspace, socket_path, &args);
        assert!(
            !rejected.status.success(),
            "unsupported status LED command should fail"
        );
        assert!(
            String::from_utf8_lossy(&rejected.stderr).contains("unsupported operation"),
            "unsupported status LED command should explain the capability rejection"
        );
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
            "luminate-plugin-demo-system",
            "-p",
            "luminate-plugin-demo-keyboard",
        ])
        .current_dir(workspace)
        .status()
        .expect("run cargo build for demo stack");
    assert!(status.success(), "demo stack build failed");
}

fn build_management_stack(workspace: &Path) {
    let status = Command::new("cargo")
        .args([
            "build",
            "-p",
            "luminated",
            "-p",
            "luminate-plugin-lifx",
            "-p",
            "luminate-plugin-demo-bulb",
        ])
        .current_dir(workspace)
        .status()
        .expect("run cargo build for management stack");
    assert!(status.success(), "management stack build failed");
}

fn assert_device_presence(workspace: &Path, socket_path: &Path, device_id: &str, expected: bool) {
    let listed = run_cli(workspace, socket_path, &["devices", "--json"]);
    let listed: Value = serde_json::from_slice(&listed.stdout).expect("parse device topology");
    let present = listed
        .as_array()
        .expect("device topology is an array")
        .iter()
        .any(|device| device["id"] == device_id);
    assert_eq!(
        present, expected,
        "device {device_id} presence should be {expected}"
    );
}

fn temp_dir() -> TestDir {
    TestDir::new("cli-it")
}

fn temp_socket_path(temp: &Path) -> PathBuf {
    temp.join("luminated.sock")
}

fn temp_state_path(temp: &Path) -> PathBuf {
    temp.join("state.json")
}

fn write_daemon_config(
    config_path: &Path,
    socket_path: &Path,
    state_path: &Path,
    plugin_paths: &[String],
) {
    let socket_path = toml_string(socket_path.display().to_string());
    let state_path = toml_string(state_path.display().to_string());
    let mut config = format!(
        "socket_path = {socket_path}\nstate_path = {state_path}\n\n[plugin_management]\nactivation = \"explicit\"\n"
    );
    for plugin_path in plugin_paths {
        let plugin_path = toml_string(plugin_path);
        let _ = write!(
            config,
            "\n[[plugins]]\npath = {plugin_path}\nrequired = true\n"
        );
    }
    fs::write(config_path, config).expect("write daemon config");
}

fn toml_string(value: impl AsRef<str>) -> String {
    serde_json::to_string(value.as_ref()).expect("serialize TOML-compatible string")
}

fn run_cli(workspace: &Path, socket_path: &Path, args: &[&str]) -> process::Output {
    let output = run_cli_output(workspace, socket_path, args);

    assert!(
        output.status.success(),
        "cli failed: status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    output
}

fn run_cli_output(workspace: &Path, socket_path: &Path, args: &[&str]) -> process::Output {
    let cli = PathBuf::from(env!("CARGO_BIN_EXE_luminatectl"));
    Command::new(cli)
        .arg("--socket-path")
        .arg(socket_path)
        .args(args)
        .current_dir(workspace)
        .output()
        .expect("run cli")
}

fn run_cli_with_stdin(
    workspace: &Path,
    socket_path: &Path,
    args: &[&str],
    stdin: &str,
) -> process::Output {
    use std::io::Write as _;

    let cli = PathBuf::from(env!("CARGO_BIN_EXE_luminatectl"));
    let mut child = Command::new(cli)
        .arg("--socket-path")
        .arg(socket_path)
        .args(args)
        .current_dir(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cli");
    child
        .stdin
        .take()
        .expect("take cli stdin")
        .write_all(stdin.as_bytes())
        .expect("write cli stdin");
    let output = child.wait_with_output().expect("wait for cli");

    assert!(
        output.status.success(),
        "cli failed: status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    output
}

fn wait_for_log_contains(log_path: &Path, needle: &str) {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        let mut log = String::new();
        if let Ok(file_log) = fs::read_to_string(log_path) {
            log = file_log;
            if log.contains(needle) {
                return;
            }
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for log entry {needle:?} in {}\ncurrent log:\n{log}",
            log_path.display()
        );

        thread::sleep(POLL_INTERVAL);
    }
}

/// Waits for at least `expected` occurrences of `needle` and returns the
/// observed count. Use this for log lines expected more than once.
fn wait_for_log_occurrences(log_path: &Path, needle: &str, expected: usize) -> usize {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        let mut log = String::new();
        if let Ok(file_log) = fs::read_to_string(log_path) {
            log = file_log;
            let count = log.matches(needle).count();
            if count >= expected {
                return count;
            }
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for {expected} occurrences of {needle:?} in {}\ncurrent log:\n{log}",
            log_path.display()
        );

        thread::sleep(POLL_INTERVAL);
    }
}

fn topology_summary(value: &Value) -> Value {
    let devices = value.as_array().expect("topology root should be an array");
    let summary = devices
        .iter()
        .map(|device| {
            let surfaces = device["surfaces"]
                .as_array()
                .expect("device surfaces should be an array")
                .iter()
                .map(|surface| {
                    json!({
                        "id": surface["id"],
                        "elements": surface["elements"]
                            .as_array()
                            .expect("surface elements should be an array")
                            .iter()
                            .map(|element| element["id"].clone())
                            .collect::<Vec<_>>(),
                    })
                })
                .collect::<Vec<_>>();

            let groups = device["groups"]
                .as_array()
                .expect("device groups should be an array")
                .iter()
                .map(|group| group["id"].clone())
                .collect::<Vec<_>>();

            json!({
                "id": device["id"],
                "name": device["name"],
                "vendor": device["vendor"],
                "model": device["model"],
                "surface_count": device["surfaces"].as_array().expect("surface array").len(),
                "group_count": device["groups"].as_array().expect("group array").len(),
                "surfaces": surfaces,
                "groups": groups,
            })
        })
        .collect::<Vec<_>>();

    Value::Array(summary)
}

#[allow(
    clippy::too_many_lines,
    reason = "The daemon harness keeps integration-test process setup in one local helper."
)]
fn expected_summary_snapshot() -> String {
    r#"[
  {
    "groups": [],
    "group_count": 0,
    "id": "demo-addressable-strip",
    "model": "Pixelvine 16",
    "name": "Addressable Strip",
    "surface_count": 1,
    "surfaces": [
      {
        "elements": [
          "led-0",
          "led-1",
          "led-2",
          "led-3",
          "led-4",
          "led-5",
          "led-6",
          "led-7",
          "led-8",
          "led-9",
          "led-10",
          "led-11",
          "led-12",
          "led-13",
          "led-14",
          "led-15"
        ],
        "id": "strip"
      }
    ],
    "vendor": "Moonbeam Systems"
  },
  {
    "groups": [
      "chassis"
    ],
    "group_count": 1,
    "id": "demo-case-lights",
    "model": "ChromaBurrow 24",
    "name": "Case Lights",
    "surface_count": 1,
    "surfaces": [
      {
        "elements": [],
        "id": "strip"
      }
    ],
    "vendor": "Moonbeam Systems"
  },
  {
    "groups": [],
    "group_count": 0,
    "id": "demo-cct-fan",
    "model": "Driftvane 140",
    "name": "CCT Fan",
    "surface_count": 0,
    "surfaces": [],
    "vendor": "Moonbeam Systems"
  },
  {
    "groups": [],
    "group_count": 0,
    "id": "demo-controller",
    "model": "GoblinGlow Fabric",
    "name": "Moonbeam GoblinGlow Fabric",
    "surface_count": 0,
    "surfaces": [],
    "vendor": "Moonbeam Systems"
  },
  {
    "groups": [
      "cooling"
    ],
    "group_count": 1,
    "id": "demo-cpu-cooler",
    "model": "Cyclone 120",
    "name": "Cyclone 120",
    "surface_count": 1,
    "surfaces": [
      {
        "elements": [],
        "id": "ring"
      }
    ],
    "vendor": "Moonbeam Systems"
  },
  {
    "groups": [
      "all",
      "gamer-keys",
      "reactive-zones"
    ],
    "group_count": 3,
    "id": "demo-keyboard",
    "model": "TypeWyrm 100",
    "name": "TypeWyrm 100",
    "surface_count": 1,
    "surfaces": [
      {
        "elements": [
          "function-row",
          "alpha-block",
          "numpad",
          "underglow",
          "g1",
          "g2",
          "g3",
          "turbo",
          "key-a",
          "key-b"
        ],
        "id": "zones"
      }
    ],
    "vendor": "Moonbeam Systems"
  },
  {
    "groups": [],
    "group_count": 0,
    "id": "demo-monitor",
    "model": "Bifröst 27",
    "name": "Monitor",
    "surface_count": 3,
    "surfaces": [
      {
        "elements": [],
        "id": "backlight"
      },
      {
        "elements": [
          "badge"
        ],
        "id": "logo"
      },
      {
        "elements": [],
        "id": "power-indicator"
      }
    ],
    "vendor": "Moonbeam Systems"
  },
  {
    "groups": [],
    "group_count": 0,
    "id": "demo-mouse",
    "model": "Clickwyrm Pro",
    "name": "Clickwyrm Pro",
    "surface_count": 1,
    "surfaces": [
      {
        "elements": [
          "logo",
          "wheel",
          "side-strip"
        ],
        "id": "lighting"
      }
    ],
    "vendor": "Moonbeam Systems"
  },
  {
    "groups": [],
    "group_count": 0,
    "id": "demo-mousepad",
    "model": "Balrog Battlemat",
    "name": "Gaming Mousepad",
    "surface_count": 1,
    "surfaces": [
      {
        "elements": [
          "led-0",
          "led-1",
          "led-2",
          "led-3",
          "led-4",
          "led-5",
          "led-6",
          "led-7",
          "led-8",
          "led-9"
        ],
        "id": "perimeter"
      }
    ],
    "vendor": "Moonbeam Systems"
  },
  {
    "groups": [],
    "group_count": 0,
    "id": "demo-power-button",
    "model": "WakeSigil",
    "name": "Power Button",
    "surface_count": 1,
    "surfaces": [
      {
        "elements": [
          "ring"
        ],
        "id": "button"
      }
    ],
    "vendor": "Moonbeam Systems"
  },
  {
    "groups": [],
    "group_count": 0,
    "id": "demo-power-supply",
    "model": "Sparkheap 850",
    "name": "Power Supply",
    "surface_count": 1,
    "surfaces": [
      {
        "elements": [
          "badge"
        ],
        "id": "logo"
      }
    ],
    "vendor": "Moonbeam Systems"
  },
  {
    "groups": [
      "memory"
    ],
    "group_count": 1,
    "id": "demo-ram-a",
    "model": "MemoryMirth",
    "name": "RAM Stick A",
    "surface_count": 1,
    "surfaces": [
      {
        "elements": [],
        "id": "bar"
      }
    ],
    "vendor": "Moonbeam Systems"
  },
  {
    "groups": [
      "memory"
    ],
    "group_count": 1,
    "id": "demo-ram-b",
    "model": "MemoryMirth",
    "name": "RAM Stick B",
    "surface_count": 1,
    "surfaces": [
      {
        "elements": [],
        "id": "bar"
      }
    ],
    "vendor": "Moonbeam Systems"
  },
  {
    "groups": [],
    "group_count": 0,
    "id": "demo-rgbw-strip",
    "model": "Fourglow 16",
    "name": "RGBW Strip",
    "surface_count": 1,
    "surfaces": [
      {
        "elements": [],
        "id": "strip"
      }
    ],
    "vendor": "Moonbeam Systems"
  },
  {
    "groups": [],
    "group_count": 0,
    "id": "demo-status-led",
    "model": "Pinlight",
    "name": "Status LED",
    "surface_count": 0,
    "surfaces": [],
    "vendor": "Moonbeam Systems"
  },
  {
    "groups": [],
    "group_count": 0,
    "id": "demo-streaming-keypad",
    "model": "HighElf Streamkey",
    "name": "Streaming Keypad",
    "surface_count": 1,
    "surfaces": [
      {
        "elements": [
          "key-0-0",
          "key-0-1",
          "key-0-2",
          "key-0-3",
          "key-0-4",
          "key-1-0",
          "key-1-1",
          "key-1-2",
          "key-1-3",
          "key-1-4",
          "key-2-0",
          "key-2-1",
          "key-2-2",
          "key-2-3",
          "key-2-4"
        ],
        "id": "grid"
      }
    ],
    "vendor": "Moonbeam Systems"
  }
]"#
    .to_owned()
}

fn test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct DaemonHarness {
    child: Child,
    socket_path: PathBuf,
}

impl DaemonHarness {
    fn spawn(workspace: &Path, config_path: &Path, socket_path: &Path, log_path: &Path) -> Self {
        Self::spawn_with_env(workspace, config_path, socket_path, log_path, &[])
    }

    fn spawn_with_env(
        workspace: &Path,
        config_path: &Path,
        socket_path: &Path,
        log_path: &Path,
        environment: &[(&str, &str)],
    ) -> Self {
        let stdout = File::options()
            .create(true)
            .append(true)
            .open(log_path)
            .expect("open daemon log for stdout");
        let stderr = File::options()
            .create(true)
            .append(true)
            .open(log_path)
            .expect("open daemon log for stderr");

        let daemon = env::var_os("LUMINATE_TEST_DAEMON").map_or_else(
            || workspace.join(format!("target/debug/{}", executable_name("luminated"))),
            PathBuf::from,
        );
        let mut command = Command::new(daemon);
        command.env("LUMINATED_CONFIG", config_path);
        command.envs(environment.iter().copied());
        let child = command
            .current_dir(workspace)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .expect("spawn daemon");
        let mut harness = Self {
            child,
            socket_path: socket_path.to_path_buf(),
        };
        wait_for_daemon(workspace, socket_path, &mut harness.child, log_path);
        harness
    }
}

impl Drop for DaemonHarness {
    fn drop(&mut self) {
        #[cfg(unix)]
        let _ = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status();
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
        if self.socket_path.exists() {
            let _ = fs::remove_file(&self.socket_path);
        }
    }
}

fn wait_for_daemon(workspace: &Path, socket_path: &Path, child: &mut Child, log_path: &Path) {
    let cli = PathBuf::from(env!("CARGO_BIN_EXE_luminatectl"));
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while Instant::now() < deadline {
        let ready = Command::new(&cli)
            .arg("--socket-path")
            .arg(socket_path)
            .arg("ping")
            .current_dir(workspace)
            .output()
            .is_ok_and(|output| output.status.success());
        if ready {
            return;
        }
        if let Some(status) = child.try_wait().expect("inspect daemon process") {
            let log = fs::read_to_string(log_path).expect("read daemon log after early exit");
            panic!(
                "daemon exited with {status} before becoming ready at {}:\n{log}",
                socket_path.display()
            );
        }

        thread::sleep(POLL_INTERVAL);
    }

    let log = fs::read_to_string(log_path).expect("read daemon log after startup timeout");
    panic!(
        "timed out waiting for daemon at {}:\n{log}",
        socket_path.display()
    );
}
