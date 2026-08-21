// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for executable authentication-provider supervision.

#[cfg(unix)]
use std::env;
#[cfg(unix)]
use std::fs;
use std::io::{Error, ErrorKind};
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::time::{Duration, SystemTime};

#[cfg(unix)]
use luminate_platform::test_support::TestDir;
#[cfg(unix)]
use luminate_protocol::AUTHENTICATION_PROVIDER_PROTOCOL_VERSION;
#[cfg(unix)]
use luminate_protocol::Continuation;
use luminate_protocol::Credential;
#[cfg(unix)]
use luminate_protocol::framing;
#[cfg(unix)]
use luminate_protocol::{
    ProviderHello, ProviderHelloResponse, ProviderIdentity, ProviderRequest, ProviderResponse,
};
#[cfg(unix)]
use tokio::fs::{File, OpenOptions};
#[cfg(unix)]
use tokio::sync::Mutex;

use super::AuthenticationProvider;

#[cfg(unix)]
const PROVIDER_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/authentication-provider.sh"
);

#[cfg(unix)]
static PROVIDER_FIXTURE_LOCK: Mutex<()> = Mutex::const_new(());

fn credential(value: &str) -> Credential {
    Credential::new(value).expect("valid bounded credential")
}

#[cfg(unix)]
fn continuation(value: &str) -> Continuation {
    Continuation::new(value).expect("valid bounded continuation")
}

#[cfg(unix)]
fn provider_fixture(case: &str) -> (TestDir, PathBuf) {
    let directory = TestDir::new("auth-provider");
    let script = directory.join("provider");
    let test_binary = env::current_exe().expect("locate daemon test executable");
    fs::write(directory.join("case"), case).expect("write provider fixture case");
    symlink(test_binary, directory.join("test-binary")).expect("link daemon test executable");
    symlink(PROVIDER_FIXTURE, &script).expect("link provider fixture launcher");
    (directory, script)
}

#[cfg(unix)]
fn identity(subject: &str) -> ProviderIdentity {
    ProviderIdentity {
        subject: subject.to_owned(),
        verified_groups: vec!["operators".to_owned()],
        credential_id: "provider-key-1".to_owned(),
        expires_at: SystemTime::now() + Duration::from_secs(60),
        continuation: continuation("continuation"),
    }
}

#[tokio::test]
async fn startup_reports_a_missing_provider_executable() {
    let result = AuthenticationProvider::start(
        Path::new("/luminate/tests/provider-does-not-exist"),
        credential("initialization"),
    )
    .await;
    let Err(error) = result else {
        panic!("a missing provider must fail closed");
    };

    assert!(
        error
            .downcast_ref::<Error>()
            .is_some_and(|error| error.kind() == ErrorKind::NotFound),
        "unexpected startup error: {error:#}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn startup_rejects_processes_that_exit_without_a_handshake() {
    let result = AuthenticationProvider::start(Path::new("true"), credential("initialize")).await;
    let Err(error) = result else {
        panic!("a process without the provider protocol must fail closed");
    };

    assert!(
        error
            .to_string()
            .contains("authentication provider handshake failed"),
        "unexpected handshake error: {error:#}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn startup_rejects_a_process_that_echoes_the_wrong_message_type() {
    let result =
        AuthenticationProvider::start(Path::new("/bin/cat"), credential("initialize")).await;
    let Err(error) = result else {
        panic!("an echoed provider hello is not a provider response");
    };

    assert!(
        format!("{error:#}").contains("CBOR decoding error"),
        "unexpected handshake error: {error:#}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn provider_authentication_rejection_and_revalidation_round_trip() {
    let _fixture_guard = PROVIDER_FIXTURE_LOCK.lock().await;
    let (directory, executable) = provider_fixture("round-trip");
    let provider = AuthenticationProvider::start(executable, credential("initialize"))
        .await
        .unwrap_or_else(|error| {
            let log = fs::read_to_string(directory.join("harness.log")).unwrap_or_default();
            panic!("start provider fixture: {error:#}\n{log}");
        });

    let authenticated = provider
        .authenticate(credential("accept"))
        .await
        .expect("authenticate accepted credential")
        .expect("accepted identity");
    assert_eq!(authenticated.subject, "alice");
    assert_eq!(authenticated.verified_groups, ["operators"]);

    assert!(
        provider
            .authenticate(credential("reject"))
            .await
            .expect("reject credential without transport failure")
            .is_none()
    );
    let revalidated = provider
        .revalidate(continuation("continuation"))
        .await
        .expect("revalidate continuation")
        .expect("revalidated identity");
    assert_eq!(revalidated.subject, "renewed-alice");
}

#[cfg(unix)]
#[tokio::test]
async fn provider_rejects_protocol_identity_and_exchange_contract_violations() {
    let _fixture_guard = PROVIDER_FIXTURE_LOCK.lock().await;
    for (case, expected) in [
        ("version-mismatch", "protocol version mismatch"),
        ("initialization-rejected", "initialization rejected"),
    ] {
        let (_directory, executable) = provider_fixture(case);
        let error = AuthenticationProvider::start(executable, credential("initialize"))
            .await
            .err()
            .expect("invalid handshake must fail");
        assert!(
            error.to_string().contains(expected),
            "unexpected error: {error:#}"
        );
    }

    for (case, expected) in [
        ("mismatched-id", "mismatched exchange ID"),
        ("invalid-identity", "empty identity field"),
    ] {
        let (_directory, executable) = provider_fixture(case);
        let provider = AuthenticationProvider::start(executable, credential("initialize"))
            .await
            .expect("start provider fixture");
        let Err(error) = provider.authenticate(credential("credential")).await else {
            panic!("invalid provider response must fail closed");
        };
        assert!(
            error.to_string().contains(expected),
            "unexpected error: {error:#}"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn transport_failure_restarts_the_provider_for_the_next_exchange() {
    let _fixture_guard = PROVIDER_FIXTURE_LOCK.lock().await;
    let (_directory, executable) = provider_fixture("restart");
    let provider = AuthenticationProvider::start(executable, credential("initialize"))
        .await
        .expect("start provider fixture");

    assert!(provider.authenticate(credential("first")).await.is_err());
    let recovered = provider
        .authenticate(credential("second"))
        .await
        .expect("use restarted provider")
        .expect("restarted provider accepts exchange");
    assert_eq!(recovered.subject, "recovered");
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_requests_a_cooperative_provider_exit() {
    let _fixture_guard = PROVIDER_FIXTURE_LOCK.lock().await;
    let (directory, executable) = provider_fixture("shutdown");
    let provider = AuthenticationProvider::start(executable, credential("initialize"))
        .await
        .expect("start provider fixture");

    provider.shutdown().await;

    assert!(
        directory.join("shutdown").exists(),
        "provider must observe the cooperative shutdown request"
    );
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "run as an isolated authentication-provider child"]
#[allow(
    clippy::too_many_lines,
    reason = "This process fixture implements one small, cohesive provider protocol state machine."
)]
async fn provider_child_probe() {
    let case = env::var("LUMINATE_PROVIDER_TEST_CASE").expect("provider fixture case");
    let mut input =
        File::open(env::var_os("LUMINATE_PROVIDER_TEST_INPUT").expect("provider input path"))
            .await
            .expect("open provider input");
    let mut output = OpenOptions::new()
        .write(true)
        .open(env::var_os("LUMINATE_PROVIDER_TEST_OUTPUT").expect("provider output path"))
        .await
        .expect("open provider output");
    let hello: ProviderHello = framing::receive(&mut input).await.expect("receive hello");
    assert_eq!(
        hello.protocol_version,
        AUTHENTICATION_PROVIDER_PROTOCOL_VERSION
    );
    assert_eq!(hello.initialization.expose(), b"initialize");

    let hello_response = match case.as_str() {
        "version-mismatch" => ProviderHelloResponse::Ready {
            protocol_version: AUTHENTICATION_PROVIDER_PROTOCOL_VERSION + 1,
        },
        "initialization-rejected" => ProviderHelloResponse::Rejected {
            reason: "fixture refused initialization".to_owned(),
        },
        _ => ProviderHelloResponse::Ready {
            protocol_version: AUTHENTICATION_PROVIDER_PROTOCOL_VERSION,
        },
    };
    framing::send(&mut output, &hello_response)
        .await
        .expect("send hello response");
    if matches!(
        case.as_str(),
        "version-mismatch" | "initialization-rejected"
    ) {
        return;
    }

    let request: ProviderRequest = framing::receive(&mut input).await.expect("receive request");
    if case == "restart" {
        let marker = Path::new(
            &env::var_os("LUMINATE_PROVIDER_TEST_DIR").expect("provider fixture directory"),
        )
        .join("failed-once");
        if !marker.exists() {
            fs::write(marker, []).expect("record first provider failure");
            return;
        }
    }

    let response = match request {
        ProviderRequest::Authenticate { id, credential } => match case.as_str() {
            "round-trip" if credential.expose() == b"reject" => ProviderResponse::Rejected {
                id,
                reason: "fixture rejection".to_owned(),
            },
            "mismatched-id" => ProviderResponse::Rejected {
                id: id + 1,
                reason: String::new(),
            },
            "invalid-identity" => ProviderResponse::Authenticated {
                id,
                identity: identity(""),
            },
            "restart" => ProviderResponse::Authenticated {
                id,
                identity: identity("recovered"),
            },
            _ => ProviderResponse::Authenticated {
                id,
                identity: identity("alice"),
            },
        },
        ProviderRequest::Revalidate { id, .. } => ProviderResponse::Authenticated {
            id,
            identity: identity("renewed-alice"),
        },
        ProviderRequest::Shutdown => {
            fs::write(
                Path::new(
                    &env::var_os("LUMINATE_PROVIDER_TEST_DIR").expect("provider fixture directory"),
                )
                .join("shutdown"),
                [],
            )
            .expect("record provider shutdown");
            return;
        }
    };
    framing::send(&mut output, &response)
        .await
        .expect("send provider response");

    if case == "round-trip" {
        for _ in 0..2 {
            let request: ProviderRequest = framing::receive(&mut input)
                .await
                .expect("receive subsequent request");
            let response = match request {
                ProviderRequest::Authenticate { id, .. } => ProviderResponse::Rejected {
                    id,
                    reason: "fixture rejection".to_owned(),
                },
                ProviderRequest::Revalidate { id, .. } => ProviderResponse::Authenticated {
                    id,
                    identity: identity("renewed-alice"),
                },
                ProviderRequest::Shutdown => return,
            };
            framing::send(&mut output, &response)
                .await
                .expect("send subsequent response");
        }
    }
}
