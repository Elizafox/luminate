// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Cross-process security tests for the HTTP companion and daemon boundary.

#![cfg(unix)]
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::tests_outside_test_module,
    reason = "integration-test setup failures should stop the test with complete context"
)]

use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::fs;
use std::io::{Read as _, Write as _};
use std::net::{Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use luminate_platform::identity::daemon_own_uid;
use luminate_platform::test_support::TestDir;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

const WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const TOKEN_HASH_DOMAIN: &[u8] = b"luminate daemon token v1\0";

#[test]
#[ignore = "spawns real luminated and luminate-http processes"]
#[allow(
    clippy::too_many_lines,
    reason = "the process lifecycle and its cross-boundary assertions form one integration scenario"
)]
fn trusted_proxy_preserves_dual_identity_and_ipv6_loopback_is_reachable() {
    let directory = TestDir::new("http-process-security");
    let socket_path = directory.join("luminated.sock");
    let state_path = directory.join("state.json");
    let config_path = directory.join("luminated.toml");
    let daemon_log = directory.join("luminated.log");
    let http_log = directory.join("luminate-http.log");
    let audit_path = directory.join("audit.jsonl");
    let proxy_credential_path = directory.join("proxy.credential");
    let frontend_secret = b"phase-five-frontend-secret";
    let delegated_secret = b"phase-five-delegated-secret";

    write_daemon_config(&config_path, &socket_path, &state_path);
    write_tokens(directory.path(), frontend_secret, delegated_secret);
    write_policy(directory.path());
    write_private(
        &proxy_credential_path,
        URL_SAFE_NO_PAD.encode(frontend_secret).as_bytes(),
    );

    let mut daemon = ChildGuard::spawn(
        daemon_binary(),
        &[],
        &[("LUMINATED_CONFIG", config_path.as_os_str())],
        &daemon_log,
    );
    wait_for_path(&socket_path, &mut daemon, &daemon_log);

    let ipv4 = reserve_address("127.0.0.1:0");
    let mut http = ChildGuard::spawn(
        PathBuf::from(env!("CARGO_BIN_EXE_luminate-http")),
        &[
            "--listen".into(),
            ipv4.to_string().into(),
            "--socket-path".into(),
            socket_path.as_os_str().into(),
            "--authentication-mode".into(),
            "trusted-proxy".into(),
            "--trusted-proxy".into(),
            "127.0.0.1".into(),
            "--trusted-proxy-credential-file".into(),
            proxy_credential_path.as_os_str().into(),
        ],
        &[],
        &http_log,
    );
    wait_for_tcp(ipv4, &mut http, &http_log);

    let expires_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current Unix time")
        .as_secs()
        + 60;
    let delegation = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&json!({
            "authority": "remote",
            "subject": "alice",
            "verified_groups": [],
            "expires_at": expires_at,
        }))
        .expect("serialize delegation"),
    );
    let response = request(
        ipv4,
        "/api/v0/policy",
        &[
            (
                "Authorization",
                format!("Bearer {}", URL_SAFE_NO_PAD.encode(frontend_secret)),
            ),
            ("Luminate-Delegation", delegation),
        ],
    );
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "response: {response}\ndaemon log:\n{}\naudit:\n{}",
        read_log(&daemon_log),
        read_log(&audit_path),
    );

    let audit = wait_for_audit(&audit_path);
    assert_eq!(audit["subject"]["authority"], "remote");
    assert_eq!(audit["subject"]["subject"], "alice");
    assert!(
        audit["subject"]["credential_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("attestation:http-")),
        "unexpected delegated credential ID: {}",
        audit["subject"]["credential_id"],
    );
    assert_eq!(audit["frontend_actor"]["authority"], "remote");
    assert_eq!(audit["frontend_actor"]["subject"], "frontend");
    assert_eq!(audit["frontend_actor"]["credential_id"], "frontend-token");

    let log = fs::read_to_string(&http_log).expect("read HTTP log");
    for secret in [
        String::from_utf8_lossy(frontend_secret).into_owned(),
        URL_SAFE_NO_PAD.encode(frontend_secret),
        String::from_utf8_lossy(delegated_secret).into_owned(),
        URL_SAFE_NO_PAD.encode(delegated_secret),
    ] {
        assert!(
            !log.contains(&secret),
            "HTTP log exposed a bearer credential"
        );
    }

    if let Some(ipv6) = reserve_ipv6_address() {
        drop(http);
        let mut ipv6_http = ChildGuard::spawn(
            PathBuf::from(env!("CARGO_BIN_EXE_luminate-http")),
            &[
                "--listen".into(),
                ipv6.to_string().into(),
                "--socket-path".into(),
                socket_path.as_os_str().into(),
            ],
            &[],
            &http_log,
        );
        wait_for_tcp(ipv6, &mut ipv6_http, &http_log);
        let response = request(ipv6, "/health/live", &[]);
        assert!(response.starts_with("HTTP/1.1 200"), "response: {response}");
    }
}

fn daemon_binary() -> PathBuf {
    env::var_os("LUMINATE_TEST_DAEMON").map_or_else(
        || workspace_root().join("target/debug/luminated"),
        PathBuf::from,
    )
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn write_daemon_config(config: &Path, socket: &Path, state: &Path) {
    let uid = daemon_own_uid();
    let socket = serde_json::to_string(&socket.display().to_string()).expect("serialize path");
    let state = serde_json::to_string(&state.display().to_string()).expect("serialize path");
    fs::write(
        config,
        format!(
            "socket_path = {socket}\nstate_path = {state}\nplugin_dirs = []\n\n[plugin_management]\nactivation = \"explicit\"\n\n[[authorization.frontends]]\nplatform = \"unix\"\nuid = {uid}\n"
        ),
    )
    .expect("write daemon configuration");
}

fn write_tokens(directory: &Path, frontend: &[u8], delegated: &[u8]) {
    let payload = json!({
        "records": {
            "frontend-token": token("frontend-token", "frontend", frontend),
            "delegated-token": token("delegated-token", "alice", delegated),
        }
    });
    write_private(
        &directory.join("tokens.json"),
        serde_json::to_vec_pretty(&payload)
            .expect("serialize tokens")
            .as_slice(),
    );
}

fn token(id: &str, subject: &str, secret: &[u8]) -> Value {
    let mut digest = Sha256::new();
    digest.update(TOKEN_HASH_DOMAIN);
    digest.update(secret);
    json!({
        "id": id,
        "subject": {"authority": "remote", "subject": subject},
        "hash": digest.finalize().to_vec(),
        "expires_at": null,
        "revoked": false,
    })
}

fn write_policy(directory: &Path) {
    let policy = json!({
        "revision": 1,
        "roles": {
            "observe": {"rules": [{
                "id": "observe",
                "effect": "allow",
                "operations": ["manage-policy"],
                "reason": null,
                "cache_hint": null,
            }]},
            "frontend": {"rules": [{
                "id": "frontend",
                "effect": "allow",
                "operations": ["manage-policy", "administer-frontend"],
                "reason": null,
                "cache_hint": null,
            }]}
        },
        "bindings": [
            {"authority": "remote", "subjects": ["frontend"], "groups": [], "roles": ["frontend"]},
            {"authority": "remote", "subjects": ["alice"], "groups": [], "roles": ["observe"]},
        ]
    });
    write_private(
        &directory.join("policy.json"),
        serde_json::to_vec_pretty(&policy)
            .expect("serialize policy")
            .as_slice(),
    );
}

fn write_private(path: &Path, contents: &[u8]) {
    fs::write(path, contents).expect("write private fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("protect fixture");
    }
}

fn reserve_address(address: &str) -> SocketAddr {
    let listener = TcpListener::bind(address).expect("reserve loopback address");
    listener.local_addr().expect("reserved address")
}

fn reserve_ipv6_address() -> Option<SocketAddr> {
    let listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).ok()?;
    listener.local_addr().ok()
}

fn wait_for_path(path: &Path, child: &mut ChildGuard, log: &Path) {
    wait_until(child, log, || path.exists());
}

fn wait_for_tcp(address: SocketAddr, child: &mut ChildGuard, log: &Path) {
    wait_until(child, log, || TcpStream::connect(address).is_ok());
}

fn wait_until(child: &mut ChildGuard, log: &Path, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while Instant::now() < deadline {
        child.assert_running(log);
        if ready() {
            return;
        }
        thread::sleep(POLL_INTERVAL);
    }
    panic!("process readiness timed out:\n{}", read_log(log));
}

fn request(address: SocketAddr, path: &str, headers: &[(&str, String)]) -> String {
    let mut stream = TcpStream::connect(address).expect("connect HTTP companion");
    let mut request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    for (name, value) in headers {
        let _ = writeln!(request, "{name}: {value}\r");
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).expect("write request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}

fn wait_for_audit(path: &Path) -> Value {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(payload) = fs::read_to_string(path) {
            for line in payload.lines().rev() {
                let record: Value = serde_json::from_str(line).expect("parse audit record");
                let operation = record.get("operation").and_then(Value::as_str);
                let subject = record
                    .get("subject")
                    .and_then(|subject| subject.get("subject"))
                    .and_then(Value::as_str);
                if operation == Some("manage-policy") && subject == Some("alice") {
                    return record;
                }
            }
        }
        thread::sleep(POLL_INTERVAL);
    }
    panic!("timed out waiting for audit record:\n{}", read_log(path));
}

fn read_log(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| format!("failed to read log: {error}"))
}

struct ChildGuard {
    child: Child,
}

impl ChildGuard {
    fn spawn(
        binary: PathBuf,
        arguments: &[OsString],
        environment: &[(&str, &OsStr)],
        log: &Path,
    ) -> Self {
        let stdout = fs::File::create(log).expect("create process log");
        let stderr = stdout.try_clone().expect("clone process log");
        let mut command = Command::new(binary);
        command.args(arguments).stdout(stdout).stderr(stderr);
        for (name, value) in environment {
            command.env(name, value);
        }
        Self {
            child: command.spawn().expect("spawn process"),
        }
    }

    fn assert_running(&mut self, log: &Path) {
        if let Some(status) = self.child.try_wait().expect("inspect child") {
            panic!("process exited with {status}:\n{}", read_log(log));
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
