// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Deterministic one-request daemon fixture for HTTP boundary tests.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
#[cfg(windows)]
use std::sync::atomic::Ordering;
use std::time::Duration;

use luminate::Credential;
use luminate::policy::PrincipalId;
use luminate_platform::test_support::TestDir;
use luminate_platform::transport::{Address, Listener};
use luminate_protocol::framing::{receive, send};
use luminate_protocol::{
    AuthenticationResponse, AuthenticationSource, ClientHello, Compatibility, DaemonHello,
    RequestMessage, Response, ResponseMessage, ResponseStatus, SessionMetadata,
};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::auth::AuthContext;
use crate::resource::credential_digest;
use crate::{
    AppState, AuthenticationBoundary, AuthenticationMode, DEFAULT_MAXIMUM_REQUEST_BYTES, rest,
    websocket,
};

#[cfg(windows)]
static NEXT_MOCK_DAEMON_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) struct MockDaemon {
    pub(crate) state: AppState,
    pub(crate) auth: AuthContext,
    _directory: TestDir,
    task: JoinHandle<()>,
}

impl MockDaemon {
    pub(crate) async fn finish(self) {
        self.task.await.expect("mock daemon task");
    }
}

pub(crate) fn auth() -> AuthContext {
    let credential = Credential::new("http-test-token").expect("credential");
    AuthContext {
        credential_digest: credential_digest(credential.expose()),
        credential,
        delegation: None,
        peer: "127.0.0.1".parse().expect("test peer"),
    }
}

pub(crate) fn one_response(status: ResponseStatus) -> MockDaemon {
    scripted(vec![vec![status]])
}

pub(crate) fn scripted(connections: Vec<Vec<ResponseStatus>>) -> MockDaemon {
    let directory = TestDir::new("http-daemon");

    #[cfg(windows)]
    let instance = NEXT_MOCK_DAEMON_ID.fetch_add(1, Ordering::Relaxed);
    #[cfg(windows)]
    let path = directory.join(format!("luminated-{}-{instance}.sock", std::process::id()));
    #[cfg(not(windows))]
    let path = directory.join("luminated.sock");

    let mut listener =
        Listener::bind(&Address::from_configured_path(&path)).expect("bind mock daemon");
    let task = tokio::spawn(async move {
        for responses in connections {
            let (mut stream, _credential) = listener.accept().await.expect("accept HTTP client");
            let _: ClientHello = receive(&mut stream).await.expect("receive hello");
            send(
                &mut stream,
                &DaemonHello {
                    compatibility: Compatibility::Compatible,
                    protocol_abi_version: luminate_protocol::PROTOCOL_ABI_VERSION,
                    daemon_version: "http-test-daemon".to_owned(),
                },
            )
            .await
            .expect("send hello");
            let _authentication: luminate_protocol::AuthenticationRequest =
                receive(&mut stream).await.expect("receive authentication");
            send(
                &mut stream,
                &AuthenticationResponse::Authenticated {
                    session: SessionMetadata {
                        subject: PrincipalId::new("local", "http-test").expect("principal"),
                        verified_groups: vec!["operators".to_owned()],
                        source: AuthenticationSource::Bearer,
                        credential_id: Some("http-test-token".to_owned()),
                        expires_at: None,
                    },
                },
            )
            .await
            .expect("send authentication");
            for status in responses {
                let request: RequestMessage = receive(&mut stream).await.expect("receive request");
                send(
                    &mut stream,
                    &ResponseMessage {
                        id: request.id,
                        response: Response { status },
                    },
                )
                .await
                .expect("send response");
            }
            if matches!(
                timeout(
                    Duration::from_millis(100),
                    receive::<RequestMessage>(&mut stream),
                )
                .await,
                Ok(Ok(_))
            ) {
                panic!("mock daemon received an unexpected request");
            }
        }
    });
    MockDaemon {
        state: AppState {
            request_id: Arc::new(AtomicU64::new(1)),
            socket_path: Some(path.clone()),
            transitions: rest::TransitionStore::default(),
            tickets: Arc::new(websocket::TicketStore::default()),
            authentication: Arc::new(AuthenticationBoundary {
                mode: AuthenticationMode::Direct,
                trusted_proxies: Vec::new(),
                proxy_credential: None,
            }),
            origins: Arc::new(crate::OriginPolicy::default()),
            resources: Arc::new(crate::test_resource_limits()),
            maximum_request_bytes: DEFAULT_MAXIMUM_REQUEST_BYTES,
        },
        auth: auth(),
        _directory: directory,
        task,
    }
}
