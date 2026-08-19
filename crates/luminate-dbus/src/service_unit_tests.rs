// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Unit tests for executable lookup, Polkit invocation, and service state.

use super::*;

#[cfg(unix)]
#[test]
fn executable_lookup_uses_the_first_match_then_falls_back() {
    assert_eq!(
        find_executable("sh", &["/does/not/exist", "/bin"], Path::new("/fallback")),
        PathBuf::from("/bin/sh")
    );
    assert_eq!(
        find_executable(
            "not-a-real-luminate-helper",
            &["/does/not/exist"],
            Path::new("/fallback")
        ),
        PathBuf::from("/fallback")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn polkit_command_reports_success_failure_and_timeout() {
    assert!(
        run_polkit_command(Command::new("true"), Duration::from_secs(1))
            .await
            .is_ok()
    );
    assert!(matches!(
        run_polkit_command(Command::new("false"), Duration::from_secs(1)).await,
        Err(MethodError::PermissionDenied(message)) if message == "Polkit denied the mutation"
    ));
    let mut sleeping = Command::new("sleep");
    sleeping.arg("1");
    assert!(matches!(
        run_polkit_command(sleeping, Duration::from_millis(1)).await,
        Err(MethodError::PermissionDenied(message)) if message == "Polkit authorization timed out"
    ));
}

#[tokio::test]
async fn shared_client_and_manager_availability_reflect_connection_state() {
    let shared = Arc::new(Shared {
        client: RwLock::new(None),
        socket_path: None,
        attestation_sequence: AtomicU64::new(1),
        attested_clients: Mutex::new(HashMap::new()),
        objects: RwLock::new(BTreeMap::new()),
        required_gid: 0,
        polkit: false,
        proc_root: PathBuf::from("/proc"),
    });
    assert!(matches!(
        shared.client().await,
        Err(MethodError::DaemonUnavailable(message))
            if message == "the Luminate daemon is unavailable"
    ));
    assert!(!Manager::new(shared).available().await);
}
