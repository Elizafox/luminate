// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::time::{SystemTime, UNIX_EPOCH};
use std::{io, process};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;

use crate::windows::identity::{account_sid, current_process_sid};
use crate::windows::local_test_account::{TestAccount, TestGroup};

use super::*;

fn unique_pipe_name(case: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    format!("luminate-transport-test-{case}-{nanos}")
}

#[tokio::test]
async fn accept_captures_the_connecting_process_identity() {
    let address = Address::NamedPipe(unique_pipe_name("accept-identity"));
    let mut listener = Listener::bind(&address).expect("bind named pipe listener");

    let connect_address = address.clone();
    let client_task =
        tokio::spawn(async move { connect(&connect_address).await.expect("connect client") });

    let (_connection, credential) = listener.accept().await.expect("accept connection");
    let _client = client_task.await.expect("client task");

    let daemon_sid = current_process_sid().expect("read this process's own SID");
    match credential {
        PeerCredential::Windows { sid, pid } => {
            // Client and server are the same process here, so their SID
            // and PID must match. The same-process assertion
            // `examples/windows_named_pipe_identity.rs` already proved
            // against the raw identity helpers; this exercises the same
            // capture through the production `Listener`/`connect` path.
            assert_eq!(sid, daemon_sid);
            assert_eq!(pid, Some(process::id()));
        }
        PeerCredential::Unix { .. } => {
            unreachable!("Windows accept must yield PeerCredential::Windows")
        }
    }
}

#[tokio::test]
async fn connection_round_trips_data_and_the_next_instance_accepts_a_second_client() {
    let address = Address::NamedPipe(unique_pipe_name("round-trip"));
    let mut listener = Listener::bind(&address).expect("bind named pipe listener");

    for message in [b"first client\0", b"second client"] {
        let connect_address = address.clone();
        let client_task = tokio::spawn(async move {
            let mut client = connect(&connect_address).await.expect("connect client");
            client.write_all(message).await.expect("write to server");
            client
        });

        let (mut server_side, _credential) = listener.accept().await.expect("accept connection");
        let mut buffer = [0_u8; 13];
        server_side
            .read_exact(&mut buffer)
            .await
            .expect("read from client");
        assert_eq!(&buffer, message);

        let _client = client_task.await.expect("client task");
    }
}

/// Run this manually on the Windows VM while a pipe with this name is
/// served by `LocalSystem` without the `NT SERVICE\luminated` SID. The
/// distinct token is essential: a same-user server is the legitimate
/// console-mode case and must be accepted.
#[tokio::test]
#[ignore = "requires a named-pipe server running as LocalSystem outside the luminated service"]
async fn client_refuses_local_system_without_the_luminated_service_sid() {
    let address = Address::NamedPipe("luminate-transport-impostor-validation".to_owned());

    let Err(error) = connect(&address).await else {
        panic!("client accepted LocalSystem without the luminated service SID");
    };
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(
        error
            .to_string()
            .contains("refusing named-pipe server process"),
        "the pipe must be rejected by server authentication, not its DACL: {error}"
    );
}

/// Proves the service-pipe DACL against distinct security principals rather
/// than trusting the access-mask constant alone.
#[tokio::test]
#[ignore = "creates and deletes throwaway local accounts and a local group; needs \
                administrator privileges"]
async fn service_pipe_dacl_enforces_client_group_membership() {
    let group = TestGroup::create(
        &unique_name("grp"),
        "luminate-platform transport DACL test group",
    )
    .expect("create throwaway client group");

    let outsider = TestAccount::create(&unique_name("out"), &[]).expect("create outsider account");
    let member =
        TestAccount::create(&unique_name("mem"), &[&group]).expect("create member account");

    let access = PipeAccess::Service {
        service_sid: current_process_sid().expect("read this process's own SID"),
        client_sid: account_sid(&group).expect("resolve throwaway group SID"),
    };
    let name = unique_pipe_name("dacl");
    // Held for the DACL it carries, not read again: the assertions below
    // exercise that DACL indirectly, through `path` and `create_instance`.
    let _listener = Listener::bind_with_access(&Address::NamedPipe(name.clone()), access.clone())
        .expect("bind service-posture named pipe listener");
    let path = pipe_path(&name);

    assert_access_denied(
        outsider.run_impersonated(|| ClientOptions::new().open(&path).map(|_client| ())),
        "an account outside the client group must be refused the connect outright",
    );

    member
        .run_impersonated(|| ClientOptions::new().open(&path))
        .expect("a client-group member must be able to connect");

    assert_access_denied(
        member.run_impersonated(|| create_instance(&name, false, &access).map(|_server| ())),
        "a client-group member must not be able to create a second pipe instance",
    );
}

fn assert_access_denied(result: io::Result<()>, message: &str) {
    let Err(error) = result else {
        panic!("{message}");
    };
    assert_eq!(
        error.raw_os_error(),
        Some(ERROR_ACCESS_DENIED.cast_signed()),
        "expected ERROR_ACCESS_DENIED, got: {error}"
    );
}

/// An adversarial process that claims the well-known pipe name first must not
/// make the real daemon serve traffic on that name. This exercises
/// `FILE_FLAG_FIRST_PIPE_INSTANCE`, so it needs neither elevation nor a
/// distinct principal.
#[tokio::test]
async fn bind_fails_when_the_pipe_name_is_already_claimed() {
    let address = Address::NamedPipe(unique_pipe_name("pre-created"));
    let _squatter =
        Listener::bind(&address).expect("bind the well-known pipe name as the adversary");

    let Err(error) = Listener::bind(&address) else {
        panic!(
            "the daemon must refuse to bind a pipe name an adversarial process already \
                 claimed"
        );
    };
    assert_eq!(
        error.raw_os_error(),
        Some(ERROR_ACCESS_DENIED.cast_signed()),
        "expected ERROR_ACCESS_DENIED from FILE_FLAG_FIRST_PIPE_INSTANCE, got: {error}"
    );
}

/// A short, SAM-account-length-safe unique name (Windows caps local
/// account names at 20 characters) for a throwaway test principal.
fn unique_name(role: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    format!("lum-{role}-{:x}", nanos & 0xffff)
}

#[test]
fn address_from_path_derives_the_file_stem_as_the_pipe_name() {
    let address = address_from_path(Path::new(r"C:\ProgramData\luminated\luminated.sock"));
    assert_eq!(address, Address::NamedPipe("luminated".to_owned()));

    let fallback = address_from_path(Path::new(""));
    assert_eq!(fallback, Address::NamedPipe("luminated".to_owned()));
}
