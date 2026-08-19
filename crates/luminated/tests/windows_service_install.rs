// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Elevated, VM-only tests for `luminated.exe service install|uninstall|start`.
//!
//! The suite covers fresh and idempotent installation, partial-failure
//! rollback, preservation of pre-existing machine objects, guarded purge,
//! paths containing spaces, rejection of a user-writable production binary,
//! and uninstall while running. It drives the real `luminated.exe` binary as
//! a subprocess because the self-install command resolves its own executable,
//! just as it does when run by an administrator.
//!
//! "Quoted path containing spaces" has no dedicated test function. It is
//! covered by construction: these tests build and run from whatever
//! directory `cargo test` was invoked in, and the suite is intended to be
//! built from a path containing a space (as the checked-out state on the
//! development VM does) so `CARGO_BIN_EXE_luminated` itself contains one.
//! `service_install_registers_service_group_and_event_source` asserts the
//! registered binary path round-trips through SCM despite that space.
//!
//! Every test mutates real machine-wide state: the actual `luminated`
//! service registration, the actual `Luminate Clients` local group, and the
//! actual `luminated` Event Log source. Each test resets that state via
//! `sc`/`net`/`reg` (bypassing our own ownership guards, which is the point)
//! before it starts, so tests may be run individually or as a sequential
//! batch, but never in parallel with each other or with anything else that
//! touches the same objects. Run with `--test-threads=1`.

#![cfg(windows)]
#![allow(
    clippy::expect_used,
    clippy::tests_outside_test_module,
    clippy::absolute_paths,
    reason = "Integration tests are crate roots and intentionally fail loudly when setup \
              assumptions break."
)]

use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

use luminate_platform::windows::event_log::{SOURCE_NAME, event_source_is_registered};
use luminate_platform::windows::local_group::local_group_is_empty;
use luminate_platform::windows::service::{CLIENT_GROUP_NAME, ServiceState, query_state};

/// Mirrors the private `SOURCE_KEY` in `luminate_platform::windows::event_log`.
/// Kept in sync by hand; a mismatch here would only weaken this test's own
/// teardown, not the code under test.
const EVENT_SOURCE_REGISTRY_KEY: &str =
    r"HKLM\SYSTEM\CurrentControlSet\Services\EventLog\Application\luminated";
/// Mirrors the private `INSTALLER_KEY` in
/// `luminate_platform::windows::installer_metadata`.
const INSTALLER_REGISTRY_KEY: &str = r"HKLM\Software\Luminate";
/// Mirrors the private `OWNED_CLIENT_GROUP_VALUE` constant, asserted stable by
/// that module's own `ownership_schema_is_stable` unit test.
const OWNED_CLIENT_GROUP_VALUE: &str = "InstallerOwnedClientGroup";
/// Mirrors the private `OWNED_EVENT_LOG_SOURCE_VALUE` constant, asserted
/// stable by that module's own `ownership_schema_is_stable` unit test.
const OWNED_EVENT_LOG_SOURCE_VALUE: &str = "InstallerOwnedEventLogSource";

/// Reads installer ownership metadata directly from the registry.
/// `installer_metadata` is a `pub(crate)` module, deliberately not part of
/// this crate's public surface, so an external integration test reads back
/// the same values a real administrator inspecting `HKLM` would see instead
/// of reaching around the encapsulation.
fn installer_owns(value_name: &str, expected: &str) -> bool {
    let output = Command::new("reg")
        .args(["query", INSTALLER_REGISTRY_KEY, "/v", value_name])
        .output()
        .expect("run reg query");
    output.status.success() && stdout_text(&output).contains(expected)
}

const START_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(250);

fn luminated(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_luminated"))
        .args(args)
        .output()
        .expect("spawn luminated.exe")
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Best-effort teardown of every object these tests touch, ignoring failures
/// from objects that are already absent. Run before each test so a prior
/// test's failure can never leak into the next one's preconditions.
fn reset_environment() {
    let _ = Command::new("sc").args(["stop", "luminated"]).output();
    let _ = Command::new("sc").args(["delete", "luminated"]).output();
    let _ = Command::new("net")
        .args(["localgroup", CLIENT_GROUP_NAME, "/delete"])
        .output();
    let _ = Command::new("reg")
        .args(["delete", EVENT_SOURCE_REGISTRY_KEY, "/f"])
        .output();
    let _ = Command::new("reg")
        .args(["delete", INSTALLER_REGISTRY_KEY, "/f"])
        .output();
    // SCM can retain a "marked for deletion" service briefly after `sc
    // delete` returns; `install` treats that transiently as ExistingService.
    wait_until(
        || query_state().is_err(),
        Duration::from_secs(10),
        "waiting for the previous test's service registration to clear",
    );
}

fn wait_until(mut predicate: impl FnMut() -> bool, timeout: Duration, message: &str) {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out {message}");
        thread::sleep(POLL_INTERVAL);
    }
}

fn sc_query_config(service: &str) -> String {
    let output = Command::new("sc")
        .args(["qc", service])
        .output()
        .expect("run sc qc");
    stdout_text(&output)
}

#[test]
#[ignore = "modifies SCM, HKLM, and local group state; needs administrator privileges"]
fn service_install_registers_service_group_and_event_source() {
    reset_environment();

    let install = luminated(&["service", "install", "--allow-development-path"]);
    assert!(
        install.status.success(),
        "fresh install should succeed: {}",
        stderr_text(&install)
    );
    assert!(
        !stdout_text(&install).contains("Client-group membership changed"),
        "a fresh install with no --add-user/--add-current-user should not report a membership \
         change"
    );

    assert_eq!(
        query_state().expect("service should be registered"),
        ServiceState::Stopped
    );
    assert!(
        installer_owns(OWNED_CLIENT_GROUP_VALUE, CLIENT_GROUP_NAME),
        "install should record ownership of the client group it created"
    );
    assert!(
        local_group_is_empty(CLIENT_GROUP_NAME).expect("query client-group membership"),
        "a fresh install adds no members without --add-user/--add-current-user"
    );
    assert!(
        installer_owns(OWNED_EVENT_LOG_SOURCE_VALUE, SOURCE_NAME),
        "install should record ownership of the Event Log source it created"
    );
    assert!(event_source_is_registered().expect("query Event Log source registration"),);

    // `CARGO_BIN_EXE_luminated` is expected to contain a space (see the
    // module doc comment); if it doesn't, this assertion is vacuous rather
    // than failing, so the build layout matters for real coverage here.
    let binary_path = env!("CARGO_BIN_EXE_luminated");
    if binary_path.contains(' ') {
        let config = sc_query_config("luminated");
        let binary_line = config
            .lines()
            .find(|line| line.contains("BINARY_PATH_NAME"))
            .expect("sc qc output should report BINARY_PATH_NAME");
        assert!(
            binary_line.contains('"'),
            "a service binary path containing a space must be quoted in SCM's registration: {binary_line}"
        );
    }

    let uninstall = luminated(&["service", "uninstall", "--purge-client-group"]);
    assert!(
        uninstall.status.success(),
        "cleanup uninstall should succeed: {}",
        stderr_text(&uninstall)
    );
    assert!(
        query_state().is_err(),
        "service should no longer be registered"
    );
    assert!(
        !installer_owns(OWNED_CLIENT_GROUP_VALUE, CLIENT_GROUP_NAME),
        "purge should have removed the ownership marker along with the group"
    );
    assert!(
        !event_source_is_registered().expect("query Event Log source registration"),
        "uninstall always removes an installer-owned Event Log source"
    );
}

#[test]
#[ignore = "modifies SCM, HKLM, and local group state; needs administrator privileges"]
fn service_install_hardens_the_machine_data_root_against_ordinary_users() {
    reset_environment();

    let install = luminated(&["service", "install", "--allow-development-path"]);
    assert!(install.status.success(), "{}", stderr_text(&install));

    let data_root = luminate_platform::default_path::config()
        .expect("resolve the machine configuration path")
        .parent()
        .expect("the configuration path has a parent directory")
        .to_owned();
    let acl = icacls(&data_root);
    assert!(
        acl.contains("NT AUTHORITY\\SYSTEM:(OI)(CI)(F)")
            || acl.contains("NT AUTHORITY\\SYSTEM:(F)"),
        "the machine data root should grant SYSTEM full access: {acl}"
    );
    assert!(
        acl.contains("BUILTIN\\Administrators:(OI)(CI)(F)")
            || acl.contains("BUILTIN\\Administrators:(F)"),
        "the machine data root should grant Administrators full access: {acl}"
    );
    assert!(
        !acl.to_uppercase().contains("BUILTIN\\USERS"),
        "the machine data root must not be reachable by ordinary users: {acl}"
    );

    // `service install` does not create `logs` itself -- the daemon does, the
    // first time it runs as `LocalSystem` -- so this simulates that with a
    // plain, unprivileged directory creation to prove the *inherited* ACL is
    // what actually protects it, not something `install` set on it directly.
    let logs_dir = data_root.join("logs");
    std::fs::create_dir_all(&logs_dir).expect("create the log directory");
    let logs_acl = icacls(&logs_dir);
    assert!(
        !logs_acl.to_uppercase().contains("BUILTIN\\USERS"),
        "logs must inherit the hardened root posture, not the ordinary-user-writable \
         %ProgramData% default: {logs_acl}"
    );

    let uninstall = luminated(&["service", "uninstall", "--purge-client-group"]);
    assert!(uninstall.status.success(), "{}", stderr_text(&uninstall));
}

/// Renders `path`'s access control list via `icacls`, the same tool an
/// administrator would run by hand to audit it.
fn icacls(path: &std::path::Path) -> String {
    let output = Command::new("icacls")
        .arg(path)
        .output()
        .expect("run icacls");
    stdout_text(&output)
}

#[test]
#[ignore = "modifies SCM, HKLM, and local group state; needs administrator privileges"]
fn service_install_is_idempotent_and_membership_changes_are_reported_once() {
    reset_environment();

    let first = luminated(&["service", "install", "--allow-development-path"]);
    assert!(first.status.success(), "{}", stderr_text(&first));

    let second = luminated(&["service", "install", "--allow-development-path"]);
    assert!(
        second.status.success(),
        "reinstalling over an identical registration should succeed: {}",
        stderr_text(&second)
    );
    assert_eq!(
        query_state().expect("service should still be registered"),
        ServiceState::Stopped
    );

    let with_membership = luminated(&[
        "service",
        "install",
        "--allow-development-path",
        "--add-current-user",
    ]);
    assert!(
        with_membership.status.success(),
        "{}",
        stderr_text(&with_membership)
    );
    assert!(
        stdout_text(&with_membership).contains("Client-group membership changed"),
        "adding a new member should report the sign-out/reboot notice"
    );
    assert!(!local_group_is_empty(CLIENT_GROUP_NAME).expect("query client-group membership"));

    let repeat_membership = luminated(&[
        "service",
        "install",
        "--allow-development-path",
        "--add-current-user",
    ]);
    assert!(
        repeat_membership.status.success(),
        "{}",
        stderr_text(&repeat_membership)
    );
    assert!(
        !stdout_text(&repeat_membership).contains("Client-group membership changed"),
        "an account that is already a member should not be reported as newly added"
    );

    let current_user =
        String::from_utf8(Command::new("whoami").output().expect("run whoami").stdout)
            .expect("whoami output should be UTF-8");
    let current_user = current_user.trim();
    let remove_member = Command::new("net")
        .args(["localgroup", CLIENT_GROUP_NAME, current_user, "/delete"])
        .output()
        .expect("run net localgroup /delete");
    assert!(
        remove_member.status.success(),
        "removing the added member should succeed: {}",
        String::from_utf8_lossy(&remove_member.stderr)
    );

    let uninstall = luminated(&["service", "uninstall", "--purge-client-group"]);
    assert!(uninstall.status.success(), "{}", stderr_text(&uninstall));
}

#[test]
fn service_install_rejects_a_user_writable_production_path() {
    // No `reset_environment()`: this path must be rejected before anything
    // is created, so nothing needs elevation or teardown. `CARGO_BIN_EXE_luminated`
    // is under the cargo target directory, never under Program Files.
    let install = luminated(&["service", "install"]);

    assert!(
        !install.status.success(),
        "installing from outside Program Files without the development override must fail"
    );
    assert!(
        stderr_text(&install).contains("outside Program Files"),
        "the failure should explain why: {}",
        stderr_text(&install)
    );
    assert!(
        query_state().is_err(),
        "a rejected install must not register the service"
    );
}

#[test]
#[ignore = "starts the real daemon as LocalSystem and modifies SCM state; needs administrator \
            privileges"]
fn service_uninstall_while_running_stops_and_removes_the_service() {
    reset_environment();

    let install = luminated(&["service", "install", "--allow-development-path"]);
    assert!(install.status.success(), "{}", stderr_text(&install));

    let start = luminated(&["service", "start"]);
    assert!(start.status.success(), "{}", stderr_text(&start));

    wait_until(
        || matches!(query_state(), Ok(ServiceState::Running)),
        START_TIMEOUT,
        "waiting for the service to report SERVICE_RUNNING",
    );

    let uninstall = luminated(&["service", "uninstall", "--purge-client-group"]);
    assert!(
        uninstall.status.success(),
        "uninstalling a running service should stop it first, then remove it: {}",
        stderr_text(&uninstall)
    );
    assert!(
        query_state().is_err(),
        "service should no longer be registered"
    );
}

#[test]
#[ignore = "modifies SCM, HKLM, and local group state; needs administrator privileges"]
fn service_uninstall_preserves_the_client_group_by_default() {
    reset_environment();

    let install = luminated(&["service", "install", "--allow-development-path"]);
    assert!(install.status.success(), "{}", stderr_text(&install));

    let uninstall = luminated(&["service", "uninstall"]);
    assert!(uninstall.status.success(), "{}", stderr_text(&uninstall));

    assert!(
        query_state().is_err(),
        "service should no longer be registered"
    );
    assert!(local_group_is_empty(CLIENT_GROUP_NAME).expect(
        "the client group should still exist after an uninstall without \
             --purge-client-group"
    ),);
    assert!(
        installer_owns(OWNED_CLIENT_GROUP_VALUE, CLIENT_GROUP_NAME),
        "ownership metadata should also survive an uninstall that did not purge"
    );
    assert!(
        !event_source_is_registered().expect("query Event Log source registration"),
        "the Event Log source is always removed on uninstall, independent of \
         --purge-client-group"
    );

    // The group is still installer-owned and empty, so a follow-up purge
    // should succeed even with the service already gone.
    let purge = luminated(&["service", "uninstall", "--purge-client-group"]);
    assert!(purge.status.success(), "{}", stderr_text(&purge));
    assert!(!installer_owns(OWNED_CLIENT_GROUP_VALUE, CLIENT_GROUP_NAME));
}

#[test]
#[ignore = "modifies SCM, HKLM, and local group state; needs administrator privileges"]
fn service_uninstall_refuses_to_purge_a_client_group_it_does_not_own() {
    reset_environment();

    let precreate = Command::new("net")
        .args([
            "localgroup",
            CLIENT_GROUP_NAME,
            "/add",
            "/comment:pre-existing, not created by Luminate",
        ])
        .output()
        .expect("run net localgroup /add");
    assert!(precreate.status.success());

    let install = luminated(&["service", "install", "--allow-development-path"]);
    assert!(
        install.status.success(),
        "install should adopt the pre-existing group without claiming ownership: {}",
        stderr_text(&install)
    );
    assert!(
        !installer_owns(OWNED_CLIENT_GROUP_VALUE, CLIENT_GROUP_NAME),
        "install must not record ownership of a group it did not create"
    );

    let purge = luminated(&["service", "uninstall", "--purge-client-group"]);
    assert!(
        !purge.status.success(),
        "purging a group Luminate does not own must be refused"
    );
    assert!(
        stderr_text(&purge).contains("ownership metadata is absent"),
        "the refusal should explain why: {}",
        stderr_text(&purge)
    );
    // The purge guard is checked before the service is touched at all.
    assert_eq!(
        query_state().expect("the refused purge must not have removed the service"),
        ServiceState::Stopped
    );
    assert!(local_group_is_empty(CLIENT_GROUP_NAME).expect("the group must survive the refusal"),);

    let uninstall = luminated(&["service", "uninstall"]);
    assert!(uninstall.status.success(), "{}", stderr_text(&uninstall));
    let remove_group = Command::new("net")
        .args(["localgroup", CLIENT_GROUP_NAME, "/delete"])
        .output()
        .expect("run net localgroup /delete");
    assert!(remove_group.status.success());
}

#[test]
#[ignore = "modifies SCM, HKLM, and local group state; needs administrator privileges"]
fn service_install_preserves_a_pre_existing_event_log_source_it_does_not_own() {
    reset_environment();

    // Registers a source with exactly the metadata `ensure_event_source`
    // would write, simulating one that predates this Luminate installation
    // (an administrator's own registration, or a prior install this
    // installer's ownership metadata was separately cleared for) rather than
    // one this invocation creates.
    let message_file = r"%SystemRoot%\Microsoft.NET\Framework64\v4.0.30319\EventLogMessages.dll";
    let create_message_file = Command::new("reg")
        .args([
            "add",
            EVENT_SOURCE_REGISTRY_KEY,
            "/v",
            "EventMessageFile",
            "/t",
            "REG_EXPAND_SZ",
            "/d",
            message_file,
            "/f",
        ])
        .output()
        .expect("run reg add EventMessageFile");
    assert!(create_message_file.status.success());
    let create_types_supported = Command::new("reg")
        .args([
            "add",
            EVENT_SOURCE_REGISTRY_KEY,
            "/v",
            "TypesSupported",
            "/t",
            "REG_DWORD",
            "/d",
            "7",
            "/f",
        ])
        .output()
        .expect("run reg add TypesSupported");
    assert!(create_types_supported.status.success());

    let install = luminated(&["service", "install", "--allow-development-path"]);
    assert!(
        install.status.success(),
        "install should adopt the pre-existing source without claiming ownership: {}",
        stderr_text(&install)
    );
    assert!(
        !installer_owns(OWNED_EVENT_LOG_SOURCE_VALUE, SOURCE_NAME),
        "install must not record ownership of a source it did not create"
    );

    let uninstall = luminated(&["service", "uninstall", "--purge-client-group"]);
    assert!(uninstall.status.success(), "{}", stderr_text(&uninstall));
    assert!(
        event_source_is_registered().expect("query Event Log source registration"),
        "an Event Log source this installer does not own must survive uninstall"
    );

    let remove_source = Command::new("reg")
        .args(["delete", EVENT_SOURCE_REGISTRY_KEY, "/f"])
        .output()
        .expect("run reg delete");
    assert!(remove_source.status.success());
}

#[test]
#[ignore = "modifies SCM, HKLM, and local group state; needs administrator privileges"]
fn service_install_rolls_back_group_and_event_source_when_scm_registration_conflicts() {
    reset_environment();

    // A conflicting registration under the same service name, pointing at an
    // unrelated executable, so `install_service`'s existing-service check
    // refuses to touch it -- the same failure an administrator would hit if
    // some other `luminated` service already existed.
    let precreate = Command::new("sc")
        .args([
            "create",
            "luminated",
            "binPath=",
            r"C:\Windows\System32\cmd.exe",
            "start=",
            "demand",
        ])
        .output()
        .expect("run sc create");
    assert!(precreate.status.success());

    let install = luminated(&["service", "install", "--allow-development-path"]);
    assert!(
        !install.status.success(),
        "install must refuse to replace an unrelated existing service"
    );
    assert!(
        stderr_text(&install).contains("refusing to replace an existing"),
        "the failure should explain why: {}",
        stderr_text(&install)
    );

    assert!(
        !installer_owns(OWNED_CLIENT_GROUP_VALUE, CLIENT_GROUP_NAME),
        "the freshly created group should have been rolled back"
    );
    assert!(
        local_group_is_empty(CLIENT_GROUP_NAME).is_err(),
        "the freshly created group should have been deleted entirely by rollback, not just \
         emptied"
    );
    assert!(
        !event_source_is_registered().expect("query Event Log source registration"),
        "the freshly created Event Log source should have been rolled back"
    );

    let config = sc_query_config("luminated");
    assert!(
        config.to_ascii_lowercase().contains("cmd.exe"),
        "the pre-existing, unrelated service registration must be untouched by the failed \
         install: {config}"
    );

    let remove_conflicting_service = Command::new("sc")
        .args(["delete", "luminated"])
        .output()
        .expect("run sc delete");
    assert!(remove_conflicting_service.status.success());
}
