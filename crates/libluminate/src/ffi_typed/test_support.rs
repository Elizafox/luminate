// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Shared fake-daemon support for portable typed C API tests.

use std::ffi::CString;
use std::path::PathBuf;

use luminate_core::policy::PrincipalId;
use luminate_platform::test_support::TestDir;
use luminate_platform::transport::{Address, Connection, Listener};
use luminate_protocol::PROTOCOL_ABI_VERSION;
use luminate_protocol::framing::{receive, send};
use luminate_protocol::{
    Authentication, AuthenticationRequest, AuthenticationResponse, AuthenticationSource,
    ClientHello, Compatibility, DaemonHello, Request, RequestMessage, Response, ResponseMessage,
    ResponseStatus, SessionMetadata,
};

pub(super) struct FakeDaemon {
    _directory: TestDir,
    path: PathBuf,
    listener: Listener,
}

impl FakeDaemon {
    pub(super) fn bind(label: &str) -> Self {
        let directory = TestDir::new(label);
        #[cfg(not(windows))]
        let path = directory.join("daemon");
        // Windows derives the named-pipe name from the configured path's file
        // stem. Keep that stem unique so parallel fake daemons do not all try
        // to claim `\\.\pipe\daemon`.
        #[cfg(windows)]
        let path = directory.join(
            directory
                .file_name()
                .expect("typed FFI test directory has a file name"),
        );
        let listener = Listener::bind(&Address::from_configured_path(&path))
            .expect("bind typed FFI fake daemon");
        Self {
            _directory: directory,
            path,
            listener,
        }
    }

    pub(super) fn c_path(&self) -> CString {
        CString::new(self.path.to_string_lossy().as_bytes()).expect("valid fake-daemon path")
    }

    pub(super) async fn accept(&mut self, daemon_name: &str) -> Connection {
        let (mut stream, _) = self
            .listener
            .accept()
            .await
            .expect("accept typed FFI client");
        let _: ClientHello = receive(&mut stream).await.expect("receive client hello");
        send(
            &mut stream,
            &DaemonHello {
                compatibility: Compatibility::Compatible,
                protocol_abi_version: PROTOCOL_ABI_VERSION,
                daemon_version: daemon_name.to_owned(),
            },
        )
        .await
        .expect("send daemon hello");
        let authentication: AuthenticationRequest =
            receive(&mut stream).await.expect("receive authentication");
        assert!(matches!(
            authentication.authentication,
            Authentication::Peer
        ));
        send(
            &mut stream,
            &AuthenticationResponse::Authenticated {
                session: SessionMetadata {
                    subject: PrincipalId::new("local", "typed-ffi-test")
                        .expect("valid test principal"),
                    verified_groups: Vec::new(),
                    source: AuthenticationSource::Peer,
                    credential_id: None,
                    expires_at: None,
                },
            },
        )
        .await
        .expect("send authentication response");
        stream
    }
}

pub(super) async fn respond(stream: &mut Connection, status: ResponseStatus) -> Request {
    let message: RequestMessage = receive(stream).await.expect("receive request");
    send(
        stream,
        &ResponseMessage {
            id: message.id,
            response: Response { status },
        },
    )
    .await
    .expect("send response");
    message.request
}

pub(super) async fn acknowledge(stream: &mut Connection) -> Request {
    respond(stream, ResponseStatus::Ack).await
}
