// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Unix-only: this fake-daemon test fixture binds a raw `tokio::net::UnixListener`
//! rather than the portable `luminate_platform::transport::Listener` the real
//! daemon uses. The production `Client`
//! this exercises is already cross-platform, but generalizing this fixture to
//! the `Connection` abstraction across every test in this file has not been
//! done yet.

#![cfg(unix)]
#![allow(
    clippy::borrow_as_ptr,
    clippy::multiple_unsafe_ops_per_block,
    clippy::too_many_lines,
    reason = "these tests deliberately exercise sequences of public C ABI calls with borrowed fixtures"
)]

use super::*;
use crate::ffi::LuminateServerInfo;
use crate::ffi_typed::access_administration::{
    self, LuminateAttestationList, LuminateCreatedAttestation, LuminateCreatedToken,
    LuminateTokenList,
};
use crate::ffi_typed::collections::LuminateCollectionMemberInput;
use crate::ffi_typed::effects::luminate_effect_create_off;
use crate::ffi_typed::frame::LuminateFrameAck;
use crate::ffi_typed::luminate_effect_free;
use crate::ffi_typed::management::{
    self, LuminateManagementChangeSet, LuminateManagementPatchBuilder, LuminateManagementSnapshot,
    luminate_management_patch_builder_free, luminate_management_patch_builder_new,
};
use crate::ffi_typed::policy::{self, LuminatePolicyDocument};
use crate::ffi_typed::scenes::{self, LuminateSceneBindingInput, LuminateSceneTargetStateInput};
use crate::ffi_typed::selector::{self, LuminateCollectionOutcome, LuminateSelectorInput};
use crate::ffi_typed::setup::{self, LuminatePluginSetupSession, LuminatePluginSetupWorkflowList};
use crate::ffi_typed::shm::LuminateShmFrameStream;
use crate::ffi_typed::transitions::{
    LuminateTransitionOptions, LuminateTransitionTargetStateInput,
};
use crate::ffi_typed::{
    self, LuminateCollectionList, LuminateCollectionSnapshot, LuminateCollectionStateSnapshot,
    LuminateDeviceSnapshot, LuminateEvent, LuminateRgb, LuminateSceneList, LuminateSceneSnapshot,
    LuminateStateSnapshot, LuminateTopologySnapshot, LuminateTransitionSnapshot,
    LuminateWithdrawnDeviceList, luminate_event_subscription_next,
};
use luminate_core::collection::OwnerIdentity;
use luminate_core::scene::{Scene, SceneBinding, SceneId, SceneTargetState};

fn view_bytes(view: LuminateStringView) -> &'static [u8] {
    if view.data.is_null() {
        return &[];
    }
    // SAFETY: callers keep the owning FFI object alive while reading the view.
    unsafe { slice::from_raw_parts(view.data.cast(), view.len) }
}

use luminate_core::policy::PrincipalId;
use luminate_platform::default_path::event_socket_path;
use luminate_protocol::framing::{FramingError, receive, send};
use luminate_protocol::{
    AttestationMetadata, ClientHello, Compatibility, Credential, DaemonHello,
    EVENT_PROTOCOL_VERSION, EventCompatibility, EventTicket, Request, RequestMessage, Response,
    ResponseMessage, ResponseStatus, ServerInfo as ProtocolServerInfo, SubscribeAck,
    SubscribeHello, TokenMetadata,
};
use std::env::temp_dir;
use std::fs;
use std::future;
use std::process;
use std::str;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc as std_mpsc};
use std::time::Duration;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex as TokioMutex;
use tokio::task::spawn_blocking;

fn ffi_socket_path(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    temp_dir().join(format!(
        "luminate-ffi-{label}-{}-{}",
        process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

async fn accept_ffi_client(listener: &UnixListener, version: &str) -> UnixStream {
    let (mut stream, _) = listener.accept().await.expect("accept FFI client");
    let _: ClientHello = receive(&mut stream).await.expect("receive FFI hello");
    send(
        &mut stream,
        &DaemonHello {
            compatibility: Compatibility::Compatible,
            protocol_abi_version: luminate_protocol::PROTOCOL_ABI_VERSION,
            daemon_version: version.to_owned(),
        },
    )
    .await
    .expect("send FFI daemon hello");
    #[allow(
        clippy::absolute_paths,
        reason = "shared crate-local test handshake helper"
    )]
    crate::client::authenticate_test_client(&mut stream).await;
    stream
}

fn last_error() -> Option<String> {
    let message = luminate_last_error_message();
    if message.is_null() {
        None
    } else {
        // SAFETY: the returned pointer borrows the current thread's error slot.
        Some(
            unsafe { CStr::from_ptr(message) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}

fn string_view(value: LuminateStringView) -> Option<&'static str> {
    if value.data.is_null() {
        return None;
    }
    let bytes = unsafe { slice::from_raw_parts(value.data.cast::<u8>(), value.len) };
    str::from_utf8(bytes).ok()
}

enum AsyncClientSignal {
    Completion {
        status: LuminateStatus,
        operation: usize,
        client: usize,
        thread: thread::ThreadId,
    },
    Destroyed,
}

struct AsyncClientContext(std_mpsc::Sender<AsyncClientSignal>);

unsafe extern "C" fn async_client_complete(
    context: *mut c_void,
    operation: *const async_operation::LuminateAsyncOperation,
    status: LuminateStatus,
    client: *mut LuminateClient,
) {
    let context = unsafe { &*(context.cast::<AsyncClientContext>()) };
    let _ = context.0.send(AsyncClientSignal::Completion {
        status,
        operation: operation.addr(),
        client: client.addr(),
        thread: thread::current().id(),
    });
}

unsafe extern "C" fn async_client_context_free(context: *mut c_void) {
    let context = unsafe { Box::from_raw(context.cast::<AsyncClientContext>()) };
    let _ = context.0.send(AsyncClientSignal::Destroyed);
}

unsafe extern "C" fn reentrant_async_client_complete(
    context: *mut c_void,
    _operation: *const async_operation::LuminateAsyncOperation,
    status: LuminateStatus,
    client: *mut LuminateClient,
) {
    let context = unsafe { &*(context.cast::<AsyncClientContext>()) };
    let ping_status = if status == LuminateStatus::Ok && !client.is_null() {
        let ping_status = unsafe { luminate_client_ping(client) };
        unsafe { luminate_client_free(client) };
        ping_status
    } else {
        status
    };
    let _ = context.0.send(AsyncClientSignal::Completion {
        status: ping_status,
        operation: 0,
        client: 0,
        thread: thread::current().id(),
    });
}

#[test]
fn asynchronous_connection_failure_has_durable_status_and_ordered_context_teardown() {
    let path = ffi_socket_path("async-missing");
    let path = CString::new(path.to_string_lossy().as_bytes()).expect("C path");
    let (tx, rx) = std_mpsc::channel();
    let context = Box::into_raw(Box::new(AsyncClientContext(tx))).cast();
    let mut operation = ptr::null_mut();
    let submitting_thread = thread::current().id();

    let submission = unsafe {
        async_connect::luminate_client_connect_path_async(
            path.as_ptr(),
            context,
            Some(async_client_context_free),
            Some(async_client_complete),
            &raw mut operation,
        )
    };
    assert_eq!(submission, LuminateStatus::Ok);
    assert!(!operation.is_null());

    let AsyncClientSignal::Completion {
        status,
        operation: callback_operation,
        client,
        thread: callback_thread,
    } = rx.recv().expect("completion callback")
    else {
        panic!("context must be destroyed after completion");
    };
    assert_ne!(status, LuminateStatus::Ok);
    assert_eq!(callback_operation, operation.addr());
    assert_eq!(client, 0);
    assert_ne!(callback_thread, submitting_thread);
    assert!(matches!(
        rx.recv().expect("context destructor"),
        AsyncClientSignal::Destroyed
    ));

    let mut polled = LuminateStatus::Ok;
    assert!(unsafe {
        async_operation::luminate_async_operation_status(operation, &raw mut polled)
    });
    assert_eq!(polled, status);
    let diagnostic = unsafe { async_operation::luminate_async_operation_error_message(operation) };
    assert!(!diagnostic.data.is_null());
    unsafe { async_operation::luminate_async_operation_release(operation) };
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn asynchronous_connection_callback_can_block_on_and_release_its_client() {
    let path = ffi_socket_path("async-reentrant-connect");
    let listener = UnixListener::bind(&path).expect("bind daemon socket");
    let server = tokio::spawn(async move {
        let mut stream = accept_ffi_client(&listener, "ffi-async-reentrant").await;
        let ping: RequestMessage = receive(&mut stream).await.expect("receive callback ping");
        assert!(matches!(ping.request, Request::Ping));
        send(
            &mut stream,
            &ResponseMessage {
                id: ping.id,
                response: Response {
                    status: ResponseStatus::Ack,
                },
            },
        )
        .await
        .expect("send callback ping response");
    });
    let path_c = CString::new(path.to_string_lossy().as_bytes()).expect("C path");
    let (tx, rx) = std_mpsc::channel();
    let context = Box::into_raw(Box::new(AsyncClientContext(tx))).cast();
    let mut operation = ptr::null_mut();
    assert_eq!(
        unsafe {
            async_connect::luminate_client_connect_path_async(
                path_c.as_ptr(),
                context,
                Some(async_client_context_free),
                Some(reentrant_async_client_complete),
                &raw mut operation,
            )
        },
        LuminateStatus::Ok
    );

    let AsyncClientSignal::Completion { status, .. } = rx.recv().expect("callback") else {
        panic!("context destructor ran before callback");
    };
    assert_eq!(status, LuminateStatus::Ok);
    assert!(matches!(
        rx.recv().expect("context destructor"),
        AsyncClientSignal::Destroyed
    ));
    unsafe { async_operation::luminate_async_operation_release(operation) };
    server.await.expect("daemon server");
    let _ = fs::remove_file(path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_asynchronous_connection_aborts_its_handshake() {
    let path = ffi_socket_path("async-cancel-connect");
    let listener = UnixListener::bind(&path).expect("bind daemon socket");
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.expect("accept connection");
        future::pending::<()>().await;
    });
    let path_c = CString::new(path.to_string_lossy().as_bytes()).expect("C path");
    let (tx, rx) = std_mpsc::channel();
    let context = Box::into_raw(Box::new(AsyncClientContext(tx))).cast();
    let mut operation = ptr::null_mut();
    assert_eq!(
        unsafe {
            async_connect::luminate_client_connect_path_async(
                path_c.as_ptr(),
                context,
                Some(async_client_context_free),
                Some(async_client_complete),
                &raw mut operation,
            )
        },
        LuminateStatus::Ok
    );
    assert_eq!(
        unsafe { async_operation::luminate_async_operation_cancel(operation) },
        async_operation::LuminateAsyncCancelResult::LuminateAsyncCancelAccepted
    );
    let AsyncClientSignal::Completion { status, client, .. } = rx.recv().expect("callback") else {
        panic!("context destructor ran before callback");
    };
    assert_eq!(status, LuminateStatus::Cancelled);
    assert_eq!(client, 0);
    assert!(matches!(
        rx.recv().expect("context destructor"),
        AsyncClientSignal::Destroyed
    ));
    unsafe { async_operation::luminate_async_operation_release(operation) };

    server.abort();
    let _ = fs::remove_file(path);
}

enum AsyncEventSignal {
    Completion {
        status: LuminateStatus,
        event: usize,
    },
    Destroyed,
}

struct AsyncEventContext(std_mpsc::Sender<AsyncEventSignal>);

unsafe extern "C" fn async_event_complete(
    context: *mut c_void,
    _operation: *const async_operation::LuminateAsyncOperation,
    status: LuminateStatus,
    event: *mut LuminateEvent,
) {
    let context = unsafe { &*(context.cast::<AsyncEventContext>()) };
    let _ = context.0.send(AsyncEventSignal::Completion {
        status,
        event: event.addr(),
    });
}

unsafe extern "C" fn async_event_context_free(context: *mut c_void) {
    let context = unsafe { Box::from_raw(context.cast::<AsyncEventContext>()) };
    let _ = context.0.send(AsyncEventSignal::Destroyed);
}

enum AsyncCoverageSignal {
    Completed(LuminateStatus),
}

struct AsyncCoverageContext(std_mpsc::Sender<AsyncCoverageSignal>);

unsafe extern "C" {
    #[link_name = "luminate_colour_value"]
    fn async_coverage_colour_value(value: *const c_void, channel: u32, out_value: *mut u32)
    -> bool;

    #[link_name = "luminate_effect_view_rgb"]
    fn async_coverage_effect_view_rgb(value: *const c_void, out_rgb: *mut LuminateRgb) -> bool;

    #[link_name = "luminate_device_group_count"]
    fn async_coverage_device_group_count(value: *const c_void) -> usize;
}

unsafe extern "C" fn async_coverage_status_complete(
    context: *mut c_void,
    _operation: *const async_operation::LuminateAsyncOperation,
    status: LuminateStatus,
) {
    let context = unsafe { &*(context.cast::<AsyncCoverageContext>()) };
    let _ = context.0.send(AsyncCoverageSignal::Completed(status));
}

macro_rules! async_coverage_passthrough_callback {
    ($name:ident, $($arg:ty),*) => {
        unsafe extern "C" fn $name(
            context: *mut c_void,
            operation: *const async_operation::LuminateAsyncOperation,
            status: LuminateStatus,
            $(_: *mut $arg),*
        ) {
            unsafe { async_coverage_status_complete(context, operation, status) };
        }
    };
}

async_coverage_passthrough_callback!(
    async_coverage_collection_list_complete,
    LuminateCollectionList
);
async_coverage_passthrough_callback!(
    async_coverage_collection_snapshot_complete,
    LuminateCollectionSnapshot
);
async_coverage_passthrough_callback!(
    async_coverage_collection_state_snapshot_complete,
    LuminateCollectionStateSnapshot
);
async_coverage_passthrough_callback!(
    async_coverage_collection_outcome_complete,
    LuminateCollectionOutcome
);
async_coverage_passthrough_callback!(async_coverage_created_token_complete, LuminateCreatedToken);
async_coverage_passthrough_callback!(async_coverage_token_list_complete, LuminateTokenList);
async_coverage_passthrough_callback!(
    async_coverage_policy_document_complete,
    LuminatePolicyDocument
);
async_coverage_passthrough_callback!(
    async_coverage_attestation_created_complete,
    LuminateCreatedAttestation
);
async_coverage_passthrough_callback!(
    async_coverage_attestation_list_complete,
    LuminateAttestationList
);
async_coverage_passthrough_callback!(async_coverage_string_complete, c_char);
async_coverage_passthrough_callback!(
    async_coverage_device_snapshot_complete,
    LuminateDeviceSnapshot
);
async_coverage_passthrough_callback!(
    async_coverage_state_snapshot_complete,
    LuminateStateSnapshot
);
async_coverage_passthrough_callback!(
    async_coverage_scene_snapshot_complete,
    LuminateSceneSnapshot
);
async_coverage_passthrough_callback!(async_coverage_scene_list_complete, LuminateSceneList);
async_coverage_passthrough_callback!(
    async_coverage_withdrawn_list_complete,
    LuminateWithdrawnDeviceList
);
async_coverage_passthrough_callback!(async_coverage_topology_complete, LuminateTopologySnapshot);
async_coverage_passthrough_callback!(
    async_coverage_management_snapshot_complete,
    LuminateManagementSnapshot
);
async_coverage_passthrough_callback!(
    async_coverage_management_changeset_complete,
    LuminateManagementChangeSet
);
async_coverage_passthrough_callback!(
    async_coverage_plugin_setup_session_complete,
    LuminatePluginSetupSession
);
async_coverage_passthrough_callback!(
    async_coverage_plugin_setup_workflows_complete,
    LuminatePluginSetupWorkflowList
);
async_coverage_passthrough_callback!(
    async_coverage_transition_snapshot_complete,
    LuminateTransitionSnapshot
);
async_coverage_passthrough_callback!(
    async_coverage_event_subscription_complete,
    LuminateEventSubscription
);
async_coverage_passthrough_callback!(async_coverage_shm_stream_complete, LuminateShmFrameStream);
async_coverage_passthrough_callback!(async_coverage_server_info_complete, LuminateServerInfo);

unsafe extern "C" fn async_coverage_client_complete(
    context: *mut c_void,
    operation: *const async_operation::LuminateAsyncOperation,
    status: LuminateStatus,
    client: *mut LuminateClient,
) {
    unsafe { async_coverage_status_complete(context, operation, status) };
    unsafe { luminate_client_free(client) };
}

unsafe extern "C" fn async_coverage_subscription_baseline_complete(
    context: *mut c_void,
    operation: *const async_operation::LuminateAsyncOperation,
    status: LuminateStatus,
    _subscription: *mut LuminateEventSubscription,
    _topology: *mut LuminateTopologySnapshot,
) {
    unsafe { async_coverage_status_complete(context, operation, status) };
}

unsafe extern "C" fn async_coverage_context_free(context: *mut c_void) {
    drop(unsafe { Box::from_raw(context.cast::<AsyncCoverageContext>()) });
}

fn async_coverage_wait(rx: &std_mpsc::Receiver<AsyncCoverageSignal>) -> LuminateStatus {
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(AsyncCoverageSignal::Completed(status)) => status,
        Err(error) => panic!("coverage async callback timed out: {error}"),
    }
}

async fn unsupported_response_daemon(stream: &mut UnixStream) -> Result<(), FramingError> {
    while let Ok(message) = receive::<RequestMessage>(stream).await {
        let response = Response {
            status: ResponseStatus::Error(luminate_protocol::OperationError {
                code: luminate_protocol::ErrorCode::Unsupported,
                message: "coverage fixture unsupported".to_owned(),
                retry_after_ms: None,
                applied_targets: Vec::new(),
            }),
        };
        send(
            stream,
            &ResponseMessage {
                id: message.id,
                response,
            },
        )
        .await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn synchronous_administration_success_payloads_are_owned() {
    let socket_path = ffi_socket_path("administration-success");
    let listener = UnixListener::bind(&socket_path).expect("bind daemon socket");
    let principal = PrincipalId::new("local", "alice").expect("principal");
    let attestation = AttestationMetadata {
        name: "browser".to_owned(),
        subject: principal.clone(),
        verified_groups: vec!["operators".to_owned()],
        credential_id: "attestation:browser".to_owned(),
        expires_at: None,
    };
    let token = TokenMetadata {
        id: "desk-token".to_owned(),
        subject: principal,
        expires_at: None,
        revoked: false,
    };
    let server = tokio::spawn({
        let attestation = attestation.clone();
        let token = token.clone();
        async move {
            let mut stream = accept_ffi_client(&listener, "administration-success").await;
            let responses = [
                ResponseStatus::AttestationCreated {
                    metadata: attestation.clone(),
                    secret: Credential::new("attestation-secret").expect("credential"),
                },
                ResponseStatus::Attestations(vec![attestation]),
                ResponseStatus::Ack,
                ResponseStatus::TokenCreated {
                    metadata: token.clone(),
                    secret: Credential::new("token-secret").expect("credential"),
                },
                ResponseStatus::Tokens(vec![token.clone()]),
                ResponseStatus::TokenCreated {
                    metadata: token,
                    secret: Credential::new("rotated-secret").expect("credential"),
                },
                ResponseStatus::Ack,
            ];
            for status in responses {
                let message: RequestMessage = receive(&mut stream).await.expect("request");
                send(
                    &mut stream,
                    &ResponseMessage {
                        id: message.id,
                        response: Response { status },
                    },
                )
                .await
                .expect("response");
            }
        }
    });

    let path = CString::new(socket_path.to_string_lossy().as_bytes()).expect("C path");
    let mut client = ptr::null_mut();
    assert_eq!(
        unsafe { luminate_client_connect_path(path.as_ptr(), &raw mut client) },
        LuminateStatus::Ok
    );
    let name = CString::new("browser").expect("C string");
    let authority = CString::new("local").expect("C string");
    let subject = CString::new("alice").expect("C string");
    let token_id = CString::new("desk-token").expect("C string");

    let mut created_attestation = ptr::null_mut();
    assert_eq!(
        unsafe {
            access_administration::luminate_client_create_attestation(
                client,
                name.as_ptr(),
                authority.as_ptr(),
                subject.as_ptr(),
                false,
                0,
                &raw mut created_attestation,
            )
        },
        LuminateStatus::Ok
    );
    assert_eq!(
        view_bytes(unsafe {
            access_administration::luminate_created_attestation_name(created_attestation)
        }),
        b"browser"
    );
    assert_eq!(
        view_bytes(unsafe {
            access_administration::luminate_created_attestation_authority(created_attestation)
        }),
        b"local"
    );
    assert_eq!(
        view_bytes(unsafe {
            access_administration::luminate_created_attestation_subject(created_attestation)
        }),
        b"alice"
    );
    let mut secret = [0_u8; 32];
    assert_eq!(
        unsafe {
            access_administration::luminate_created_attestation_secret(
                created_attestation,
                secret.as_mut_ptr(),
                secret.len(),
            )
        },
        18
    );
    unsafe { access_administration::luminate_created_attestation_free(created_attestation) };

    let mut attestations = ptr::null_mut();
    assert_eq!(
        unsafe {
            access_administration::luminate_client_list_attestations(client, &raw mut attestations)
        },
        LuminateStatus::Ok
    );
    let attestation =
        unsafe { access_administration::luminate_attestation_list_at(attestations, 0) };
    assert_eq!(
        view_bytes(unsafe { access_administration::luminate_attestation_authority(attestation) }),
        b"local"
    );
    assert_eq!(
        view_bytes(unsafe { access_administration::luminate_attestation_subject(attestation) }),
        b"alice"
    );
    assert_eq!(
        view_bytes(unsafe {
            access_administration::luminate_attestation_credential_id(attestation)
        }),
        b"attestation:browser"
    );
    unsafe { access_administration::luminate_attestation_list_free(attestations) };
    assert_eq!(
        unsafe { access_administration::luminate_client_revoke_attestation(client, name.as_ptr()) },
        LuminateStatus::Ok
    );

    let mut created_token = ptr::null_mut();
    assert_eq!(
        unsafe {
            access_administration::luminate_client_create_token(
                client,
                token_id.as_ptr(),
                authority.as_ptr(),
                subject.as_ptr(),
                false,
                0,
                &raw mut created_token,
            )
        },
        LuminateStatus::Ok
    );
    unsafe { access_administration::luminate_created_token_free(created_token) };

    let mut tokens = ptr::null_mut();
    assert_eq!(
        unsafe { access_administration::luminate_client_list_tokens(client, &raw mut tokens) },
        LuminateStatus::Ok
    );
    let token = unsafe { access_administration::luminate_token_list_at(tokens, 0) };
    assert_eq!(
        view_bytes(unsafe { access_administration::luminate_token_authority(token) }),
        b"local"
    );
    assert_eq!(
        view_bytes(unsafe { access_administration::luminate_token_subject(token) }),
        b"alice"
    );
    unsafe { access_administration::luminate_token_list_free(tokens) };

    assert_eq!(
        unsafe {
            access_administration::luminate_client_rotate_token(
                client,
                token_id.as_ptr(),
                false,
                0,
                &raw mut created_token,
            )
        },
        LuminateStatus::Ok
    );
    unsafe { access_administration::luminate_created_token_free(created_token) };
    assert_eq!(
        unsafe { access_administration::luminate_client_revoke_token(client, token_id.as_ptr()) },
        LuminateStatus::Ok
    );

    unsafe { luminate_client_free(client) };
    server.await.expect("server");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remaining_async_coverage_test_targets_unmatched_symbols() {
    let socket_path = ffi_socket_path("async-unmatched");
    let path = CString::new(socket_path.to_string_lossy().as_bytes()).expect("C path");
    let listener = UnixListener::bind(&socket_path).expect("bind daemon socket");
    let server = tokio::spawn({
        let path = path.clone();
        async move {
            for _ in 0..4 {
                let mut stream = accept_ffi_client(&listener, "coverage-daemon").await;
                tokio::spawn(async move {
                    if let Err(error) = unsupported_response_daemon(&mut stream).await {
                        panic!("unsupported daemon failure: {error}");
                    }
                });
            }
            let _ = path;
        }
    });

    let connect_path = CString::new(path.to_string_lossy().as_bytes()).expect("connect C path");
    let mut client = ptr::null_mut();
    assert_eq!(
        unsafe { luminate_client_connect_path(connect_path.as_ptr(), &raw mut client) },
        LuminateStatus::Ok
    );

    let mut effect = ptr::null_mut();
    assert_eq!(
        unsafe { luminate_effect_create_off(&raw mut effect) },
        LuminateStatus::Ok
    );

    let mut management_patch: *mut LuminateManagementPatchBuilder = ptr::null_mut();
    assert_eq!(
        unsafe { luminate_management_patch_builder_new(0, &raw mut management_patch) },
        LuminateStatus::Ok
    );

    let mut builder = ptr::null_mut();
    assert_eq!(
        unsafe { luminate_client_builder_new(&raw mut builder) },
        LuminateStatus::Ok
    );
    assert_eq!(
        unsafe { luminate_client_builder_set_path(builder, path.as_ptr()) },
        LuminateStatus::Ok
    );

    let mut builder_client = ptr::null_mut();
    assert_eq!(
        unsafe { luminate_client_builder_connect(builder, &raw mut builder_client) },
        LuminateStatus::Ok
    );
    unsafe { luminate_client_free(builder_client) };

    let mut session_metadata = ptr::null_mut();
    assert_eq!(
        unsafe { luminate_client_get_session_metadata(client, &raw mut session_metadata) },
        LuminateStatus::Ok
    );
    assert!(!session_metadata.is_null());
    let _ = unsafe { luminate_session_metadata_authority(session_metadata) };
    let _ = unsafe { luminate_session_metadata_subject(session_metadata) };
    let _ = unsafe { luminate_session_metadata_group_count(session_metadata) };
    let _ = unsafe { luminate_session_metadata_group_at(session_metadata, 0) };
    let _ = unsafe { luminate_session_metadata_source(session_metadata) };
    let _ = unsafe { luminate_session_metadata_source_name(session_metadata) };
    let _ = unsafe { luminate_session_metadata_credential_id(session_metadata) };
    let mut session_expiry = 0;
    assert!(!unsafe {
        luminate_session_metadata_expires_at_unix_ms(session_metadata, &raw mut session_expiry)
    });
    unsafe { luminate_session_metadata_free(session_metadata) };

    let device = CString::new("coverage-device").expect("C string");
    let target = LuminateTarget {
        device_id: device.as_ptr(),
        surface_id: ptr::null(),
        element_id: ptr::null(),
        group_id: ptr::null(),
    };

    let scene_input_state = LuminateSceneTargetStateInput {
        appearance: ptr::null(),
        has_brightness: true,
        brightness: 0,
        has_emission: false,
        emission: 0,
        appearance_slots: ptr::null(),
        appearance_slot_count: 0,
    };
    let scene_binding = LuminateSceneBindingInput {
        dynamic_collection_id: ptr::null(),
        target,
        state: scene_input_state,
    };
    let scene_bindings = [scene_binding];
    let scene = Scene {
        id: SceneId::new("coverage-scene"),
        revision: 1,
        name: "coverage".to_owned(),
        description: None,
        owner: OwnerIdentity::Uid(1),
        bindings: vec![SceneBinding::Frozen {
            target: TargetId::device("coverage-device"),
            state: SceneTargetState {
                appearance: None,
                brightness: Some(1),
                emission: None,
                appearance_slots: None,
            },
        }],
    };
    let mut scene_builder = ptr::null_mut();
    assert_eq!(
        unsafe {
            scenes::luminate_scene_builder_from_scene(
                ptr::from_ref(&scene).cast(),
                &raw mut scene_builder,
            )
        },
        LuminateStatus::Ok
    );
    let empty_selector = LuminateSelectorInput {
        kind: 0,
        target,
        collection_id: ptr::null(),
        targets: ptr::null(),
        target_count: 0,
    };
    let transition_state = LuminateTransitionTargetStateInput {
        target,
        state: LuminateSceneTargetStateInput {
            appearance: ptr::null(),
            has_brightness: true,
            brightness: 0,
            has_emission: false,
            emission: 0,
            appearance_slots: ptr::null(),
            appearance_slot_count: 0,
        },
    };
    let transition_states = [transition_state];
    let transition_options = LuminateTransitionOptions {
        duration_ms: 1,
        step_interval_ms: 0,
        function: 0,
        colour_interpolation: 0,
        hue_direction: 0,
    };

    let name = CString::new("fixture").expect("C string");
    let authority = CString::new("authority").expect("C string");
    let subject = CString::new("subject").expect("C string");
    let session_id = CString::new("0123456789abcdef0123456789abcdef").expect("C string");
    let plugin = CString::new("plugin").expect("C string");
    let workflow = CString::new("workflow").expect("C string");
    let choice = CString::new("choice").expect("C string");
    let id = CString::new("id").expect("C string");
    let dynamic_id = CString::new("dynamic-id").expect("C string");
    let description = CString::new("description").expect("C string");
    let collection_name = CString::new("collection").expect("C string");
    let collection = LuminateCollectionMemberInput {
        is_collection: false,
        target,
        collection_id: ptr::null(),
    };
    let collection_members = [collection];

    let mut topology = ptr::null_mut();
    assert_eq!(
        unsafe { ffi_typed::luminate_client_list_devices(client, &raw mut topology) },
        LuminateStatus::Unsupported
    );
    assert!(topology.is_null());

    let mut withdrawn = ptr::null_mut();
    assert_eq!(
        unsafe { ffi_typed::luminate_client_list_withdrawn_devices(client, &raw mut withdrawn) },
        LuminateStatus::Unsupported
    );
    assert!(withdrawn.is_null());

    let mut device_snapshot = ptr::null_mut();
    assert_eq!(
        unsafe {
            ffi_typed::luminate_client_get_device(client, id.as_ptr(), &raw mut device_snapshot)
        },
        LuminateStatus::Unsupported
    );
    assert!(device_snapshot.is_null());

    let mut state_snapshot = ptr::null_mut();
    assert_eq!(
        unsafe {
            ffi_typed::luminate_client_get_state(client, id.as_ptr(), &raw mut state_snapshot)
        },
        LuminateStatus::Unsupported
    );
    assert!(state_snapshot.is_null());

    let mut collection_state = ptr::null_mut();
    assert_eq!(
        unsafe {
            ffi_typed::luminate_client_get_collection_state(
                client,
                id.as_ptr(),
                &raw mut collection_state,
            )
        },
        LuminateStatus::Unsupported
    );
    assert!(collection_state.is_null());

    let mut subscription = ptr::null_mut();
    assert_eq!(
        unsafe { luminate_client_subscribe(client, &raw mut subscription) },
        LuminateStatus::Unsupported
    );
    assert!(subscription.is_null());

    unsafe { ffi_typed::luminate_withdrawn_device_list_free(ptr::null_mut()) };
    unsafe { access_administration::luminate_attestation_list_free(ptr::null_mut()) };
    unsafe { access_administration::luminate_created_attestation_free(ptr::null_mut()) };
    unsafe { access_administration::luminate_created_token_free(ptr::null_mut()) };
    unsafe { management::luminate_management_change_set_free(ptr::null_mut()) };
    unsafe { selector::luminate_collection_outcome_free(ptr::null_mut()) };

    assert!(
        view_bytes(unsafe { policy::luminate_authorization_evaluation_reason(ptr::null()) })
            .is_empty()
    );
    let mut colour_value = 0;
    assert!(!unsafe { async_coverage_colour_value(ptr::null(), 0, &raw mut colour_value) });
    let mut rgb = LuminateRgb { r: 0, g: 0, b: 0 };
    assert!(!unsafe { async_coverage_effect_view_rgb(ptr::null(), &raw mut rgb) });
    assert!(
        unsafe { management::luminate_management_change_set_view_at(ptr::null(), 0) }.is_null()
    );
    assert!(
        unsafe { management::luminate_management_snapshot_effective_daemon(ptr::null()) }.is_null()
    );
    assert_eq!(unsafe { async_coverage_device_group_count(ptr::null()) }, 0);

    let mut created_attestation = ptr::null_mut();
    assert_eq!(
        unsafe {
            access_administration::luminate_client_create_attestation(
                client,
                name.as_ptr(),
                authority.as_ptr(),
                subject.as_ptr(),
                false,
                0,
                &raw mut created_attestation,
            )
        },
        LuminateStatus::Unsupported
    );
    assert!(created_attestation.is_null());

    assert_eq!(
        unsafe {
            access_administration::luminate_client_create_principal_attestation(
                client,
                name.as_ptr(),
                authority.as_ptr(),
                subject.as_ptr(),
                ptr::null(),
                0,
                false,
                0,
                &raw mut created_attestation,
            )
        },
        LuminateStatus::Unsupported
    );
    assert!(created_attestation.is_null());

    let mut attestations = ptr::null_mut();
    assert_eq!(
        unsafe {
            access_administration::luminate_client_list_attestations(client, &raw mut attestations)
        },
        LuminateStatus::Unsupported
    );
    assert!(attestations.is_null());
    assert_eq!(
        unsafe { access_administration::luminate_client_revoke_attestation(client, name.as_ptr()) },
        LuminateStatus::Unsupported
    );

    let mut policy = ptr::null_mut();
    assert_eq!(
        unsafe {
            access_administration::luminate_client_get_access_policy(client, &raw mut policy)
        },
        LuminateStatus::Unsupported
    );
    assert!(policy.is_null());

    let mut created_token = ptr::null_mut();
    assert_eq!(
        unsafe {
            access_administration::luminate_client_create_token(
                client,
                id.as_ptr(),
                authority.as_ptr(),
                subject.as_ptr(),
                false,
                0,
                &raw mut created_token,
            )
        },
        LuminateStatus::Unsupported
    );
    assert!(created_token.is_null());

    let mut tokens = ptr::null_mut();
    assert_eq!(
        unsafe { access_administration::luminate_client_list_tokens(client, &raw mut tokens) },
        LuminateStatus::Unsupported
    );
    assert!(tokens.is_null());
    assert_eq!(
        unsafe { access_administration::luminate_client_revoke_token(client, id.as_ptr()) },
        LuminateStatus::Unsupported
    );
    assert_eq!(
        unsafe {
            access_administration::luminate_client_rotate_token(
                client,
                id.as_ptr(),
                false,
                0,
                &raw mut created_token,
            )
        },
        LuminateStatus::Unsupported
    );
    assert!(created_token.is_null());

    let mut scene = ptr::null_mut();
    assert_eq!(
        unsafe {
            scenes::luminate_client_create_scene(
                client,
                name.as_ptr(),
                description.as_ptr(),
                scene_bindings.as_ptr(),
                scene_bindings.len(),
                &raw mut scene,
            )
        },
        LuminateStatus::Unsupported
    );
    assert_eq!(
        unsafe {
            scenes::luminate_client_capture_scene(
                client,
                name.as_ptr(),
                description.as_ptr(),
                ptr::null(),
                &target,
                1,
                &raw mut scene,
            )
        },
        LuminateStatus::Unsupported
    );
    assert_eq!(
        unsafe {
            scenes::luminate_client_replace_scene(
                client,
                id.as_ptr(),
                0,
                name.as_ptr(),
                description.as_ptr(),
                scene_bindings.as_ptr(),
                scene_bindings.len(),
                &raw mut scene,
            )
        },
        LuminateStatus::Unsupported
    );
    assert_eq!(
        unsafe {
            scenes::luminate_client_recapture_scene(
                client,
                id.as_ptr(),
                0,
                ptr::null(),
                &target,
                1,
                &raw mut scene,
            )
        },
        LuminateStatus::Unsupported
    );
    assert_eq!(
        unsafe { scenes::luminate_client_delete_scene(client, id.as_ptr(), 0) },
        LuminateStatus::Unsupported
    );

    let mut scenes_list = ptr::null_mut();
    assert_eq!(
        unsafe { scenes::luminate_client_list_scenes(client, &raw mut scenes_list) },
        LuminateStatus::Unsupported
    );
    assert_eq!(
        unsafe { scenes::luminate_client_get_scene(client, id.as_ptr(), &raw mut scene) },
        LuminateStatus::Unsupported
    );
    assert_eq!(
        unsafe { scenes::luminate_client_apply_scene(client, id.as_ptr()) },
        LuminateStatus::Unsupported
    );
    assert!(scene.is_null());
    assert!(scenes_list.is_null());

    let mut setup_session = ptr::null_mut();
    assert_eq!(
        unsafe {
            setup::luminate_client_plugin_setup_start(
                client,
                plugin.as_ptr(),
                workflow.as_ptr(),
                &raw mut setup_session,
            )
        },
        LuminateStatus::Unsupported
    );
    assert_eq!(
        unsafe {
            setup::luminate_client_plugin_setup_choose(
                client,
                session_id.as_ptr(),
                0,
                choice.as_ptr(),
                &raw mut setup_session,
            )
        },
        LuminateStatus::Unsupported
    );
    assert_eq!(
        unsafe {
            setup::luminate_client_plugin_setup_confirm(
                client,
                session_id.as_ptr(),
                0,
                &raw mut setup_session,
            )
        },
        LuminateStatus::Unsupported
    );
    assert_eq!(
        unsafe {
            setup::luminate_client_plugin_setup_get(
                client,
                session_id.as_ptr(),
                &raw mut setup_session,
            )
        },
        LuminateStatus::Unsupported
    );
    assert_eq!(
        unsafe {
            setup::luminate_client_plugin_setup_cancel(
                client,
                session_id.as_ptr(),
                &raw mut setup_session,
            )
        },
        LuminateStatus::Unsupported
    );
    assert!(setup_session.is_null());

    let mut workflows = ptr::null_mut();
    assert_eq!(
        unsafe {
            setup::luminate_client_plugin_setup_workflows(
                client,
                plugin.as_ptr(),
                &raw mut workflows,
            )
        },
        LuminateStatus::Unsupported
    );
    assert!(workflows.is_null());

    let mut management_snapshot = ptr::null_mut();
    assert_eq!(
        unsafe { management::luminate_client_get_management(client, &raw mut management_snapshot) },
        LuminateStatus::Unsupported
    );
    assert!(management_snapshot.is_null());

    let mut management_changes = ptr::null_mut();
    assert_eq!(
        unsafe {
            management::luminate_client_patch_management(
                client,
                management_patch,
                &raw mut management_changes,
            )
        },
        LuminateStatus::Unsupported
    );
    assert!(management_changes.is_null());

    let (null_tx, coverage_rx) = std_mpsc::channel::<AsyncCoverageSignal>();

    macro_rules! coverage_call {
        ($label:literal, $callback:expr, $call:expr) => {{ coverage_call!($label, $callback, LuminateStatus::Unsupported, $call) }};
        ($label:literal, $callback:expr, $expected:expr, $call:expr) => {{
            let operation = ptr::null_mut();
            assert_eq!($call, LuminateStatus::Ok, concat!($label, " submit"));
            assert_eq!(
                async_coverage_wait(&coverage_rx),
                $expected,
                concat!($label, " completion")
            );
            unsafe { async_operation::luminate_async_operation_release(operation) };
            let _ = $callback;
        }};
    }

    let mut operation = ptr::null_mut();
    assert_eq!(
        unsafe {
            async_connect::luminate_client_connect_async(
                ptr::null_mut(),
                Some(async_coverage_context_free),
                None,
                &raw mut operation,
            )
        },
        LuminateStatus::NullPointer
    );
    assert!(operation.is_null());

    coverage_call!(
        "connect_path_async",
        async_coverage_status_complete,
        LuminateStatus::Ok,
        unsafe {
            async_connect::luminate_client_connect_path_async(
                path.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_client_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "builder_connect_async",
        async_coverage_status_complete,
        LuminateStatus::Ok,
        unsafe {
            async_connect::luminate_client_builder_connect_async(
                builder,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_client_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "server_info_async",
        async_coverage_server_info_complete,
        unsafe {
            async_calls::luminate_client_server_info_async(
                client,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_server_info_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "purge_withdrawn_device_async",
        async_coverage_status_complete,
        unsafe {
            async_calls::luminate_client_purge_withdrawn_device_async(
                client,
                id.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_status_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "list_devices_async",
        async_coverage_topology_complete,
        unsafe {
            async_calls::luminate_client_list_devices_async(
                client,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_topology_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "list_withdrawn_devices_async",
        async_coverage_withdrawn_list_complete,
        unsafe {
            async_calls::luminate_client_list_withdrawn_devices_async(
                client,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_withdrawn_list_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "refresh_state_async",
        async_coverage_status_complete,
        unsafe {
            async_calls::luminate_client_refresh_state_async(
                client,
                id.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_status_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "create_attestation_async",
        async_coverage_attestation_created_complete,
        unsafe {
            async_calls::luminate_client_create_attestation_async(
                client,
                name.as_ptr(),
                authority.as_ptr(),
                subject.as_ptr(),
                false,
                0,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_attestation_created_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "create_principal_attestation_async",
        async_coverage_attestation_created_complete,
        unsafe {
            async_calls::luminate_client_create_principal_attestation_async(
                client,
                name.as_ptr(),
                authority.as_ptr(),
                subject.as_ptr(),
                ptr::null(),
                0,
                false,
                0,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_attestation_created_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "list_attestations_async",
        async_coverage_attestation_list_complete,
        unsafe {
            async_calls::luminate_client_list_attestations_async(
                client,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_attestation_list_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "revoke_attestation_async",
        async_coverage_status_complete,
        unsafe {
            async_calls::luminate_client_revoke_attestation_async(
                client,
                name.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_status_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "get_access_policy_async",
        async_coverage_policy_document_complete,
        unsafe {
            async_calls::luminate_client_get_access_policy_async(
                client,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_policy_document_complete),
                &raw mut operation,
            )
        }
    );

    assert_eq!(
        unsafe {
            async_calls::luminate_client_replace_access_policy_async(
                client,
                0,
                ptr::null(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_policy_document_complete),
                &raw mut operation,
            )
        },
        LuminateStatus::NullPointer
    );

    coverage_call!(
        "create_token_async",
        async_coverage_created_token_complete,
        unsafe {
            async_calls::luminate_client_create_token_async(
                client,
                id.as_ptr(),
                authority.as_ptr(),
                subject.as_ptr(),
                false,
                0,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_created_token_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "list_tokens_async",
        async_coverage_token_list_complete,
        unsafe {
            async_calls::luminate_client_list_tokens_async(
                client,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_token_list_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "revoke_token_async",
        async_coverage_status_complete,
        unsafe {
            async_calls::luminate_client_revoke_token_async(
                client,
                id.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_status_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "rotate_token_async",
        async_coverage_created_token_complete,
        unsafe {
            async_calls::luminate_client_rotate_token_async(
                client,
                id.as_ptr(),
                false,
                0,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_created_token_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "create_collection_async",
        async_coverage_string_complete,
        unsafe {
            async_calls::luminate_client_create_collection_async(
                client,
                collection_name.as_ptr(),
                description.as_ptr(),
                ptr::null(),
                collection_members.as_ptr(),
                collection_members.len(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_string_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "destroy_collection_async",
        async_coverage_status_complete,
        unsafe {
            async_calls::luminate_client_destroy_collection_async(
                client,
                id.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_status_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "add_collection_member_async",
        async_coverage_status_complete,
        unsafe {
            async_calls::luminate_client_add_collection_member_async(
                client,
                id.as_ptr(),
                collection_members.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_status_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "remove_collection_member_async",
        async_coverage_status_complete,
        unsafe {
            async_calls::luminate_client_remove_collection_member_async(
                client,
                id.as_ptr(),
                collection_members.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_status_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "list_collections_async",
        async_coverage_collection_list_complete,
        unsafe {
            async_calls::luminate_client_list_collections_async(
                client,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_collection_list_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "get_collection_async",
        async_coverage_collection_snapshot_complete,
        unsafe {
            async_calls::luminate_client_get_collection_async(
                client,
                id.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_collection_snapshot_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "set_appearance_slots_async",
        async_coverage_status_complete,
        unsafe {
            async_calls::luminate_client_set_appearance_slots_async(
                client,
                &target,
                ptr::null(),
                0,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_status_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "create_scene_async",
        async_coverage_scene_snapshot_complete,
        unsafe {
            async_calls::luminate_client_create_scene_async(
                client,
                name.as_ptr(),
                description.as_ptr(),
                scene_bindings.as_ptr(),
                scene_bindings.len(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_scene_snapshot_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "capture_scene_async",
        async_coverage_scene_snapshot_complete,
        unsafe {
            async_calls::luminate_client_capture_scene_async(
                client,
                name.as_ptr(),
                description.as_ptr(),
                dynamic_id.as_ptr(),
                &target,
                0,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_scene_snapshot_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "replace_scene_async",
        async_coverage_scene_snapshot_complete,
        unsafe {
            async_calls::luminate_client_replace_scene_async(
                client,
                id.as_ptr(),
                0,
                name.as_ptr(),
                description.as_ptr(),
                scene_bindings.as_ptr(),
                scene_bindings.len(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_scene_snapshot_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "replace_scene_from_builder_async",
        async_coverage_scene_snapshot_complete,
        unsafe {
            async_calls::luminate_client_replace_scene_from_builder_async(
                client,
                scene_builder,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_scene_snapshot_complete),
                &raw mut operation,
            )
        }
    );
    unsafe { scenes::luminate_scene_builder_free(scene_builder) };

    coverage_call!(
        "recapture_scene_async",
        async_coverage_scene_snapshot_complete,
        unsafe {
            async_calls::luminate_client_recapture_scene_async(
                client,
                id.as_ptr(),
                0,
                dynamic_id.as_ptr(),
                &target,
                0,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_scene_snapshot_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "delete_scene_async",
        async_coverage_status_complete,
        unsafe {
            async_calls::luminate_client_delete_scene_async(
                client,
                id.as_ptr(),
                0,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_status_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "list_scenes_async",
        async_coverage_scene_list_complete,
        unsafe {
            async_calls::luminate_client_list_scenes_async(
                client,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_scene_list_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "get_scene_async",
        async_coverage_scene_snapshot_complete,
        unsafe {
            async_calls::luminate_client_get_scene_async(
                client,
                id.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_scene_snapshot_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "apply_scene_async",
        async_coverage_status_complete,
        unsafe {
            async_calls::luminate_client_apply_scene_async(
                client,
                id.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_status_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!("set_effect_async", async_coverage_status_complete, unsafe {
        async_calls::luminate_client_set_effect_async(
            client,
            &target,
            effect,
            Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
            Some(async_coverage_context_free),
            Some(async_coverage_status_complete),
            &raw mut operation,
        )
    });

    coverage_call!(
        "set_emission_async",
        async_coverage_status_complete,
        unsafe {
            async_calls::luminate_client_set_emission_async(
                client,
                &target,
                0,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_status_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "set_brightness_async",
        async_coverage_status_complete,
        unsafe {
            async_calls::luminate_client_set_brightness_async(
                client,
                &target,
                50,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_status_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "set_brightness_selector_async",
        async_coverage_collection_outcome_complete,
        unsafe {
            async_calls::luminate_client_set_brightness_selector_async(
                client,
                &empty_selector,
                0,
                false,
                0,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_collection_outcome_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "set_effect_selector_async",
        async_coverage_collection_outcome_complete,
        unsafe {
            async_calls::luminate_client_set_effect_selector_async(
                client,
                &empty_selector,
                effect,
                false,
                0,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_collection_outcome_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "set_emission_selector_async",
        async_coverage_collection_outcome_complete,
        unsafe {
            async_calls::luminate_client_set_emission_selector_async(
                client,
                &empty_selector,
                0,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_collection_outcome_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "begin_frame_stream_async",
        async_coverage_u32_complete,
        unsafe {
            async_calls::luminate_client_begin_frame_stream_async(
                client,
                &target,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_u32_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "upload_frame_full_async",
        async_coverage_frame_ack_complete,
        unsafe {
            async_calls::luminate_client_upload_frame_full_async(
                client,
                &target,
                0,
                0,
                ptr::null(),
                0,
                0,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_frame_ack_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "upload_frame_partial_async",
        async_coverage_frame_ack_complete,
        unsafe {
            async_calls::luminate_client_upload_frame_partial_async(
                client,
                &target,
                0,
                0,
                ptr::null(),
                ptr::null(),
                0,
                0,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_frame_ack_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "end_frame_stream_async",
        async_coverage_status_complete,
        unsafe {
            async_calls::luminate_client_end_frame_stream_async(
                client,
                &target,
                0,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_status_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "begin_shm_frame_stream_async",
        async_coverage_shm_stream_complete,
        unsafe {
            async_calls::luminate_client_begin_shm_frame_stream_async(
                client,
                &target,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_shm_stream_complete),
                &raw mut operation,
            )
        }
    );

    assert_eq!(
        unsafe {
            async_calls::luminate_client_end_shm_frame_stream_async(
                ptr::null_mut(),
                ptr::null_mut(),
                None,
                None,
                &raw mut operation,
            )
        },
        LuminateStatus::NullPointer
    );

    coverage_call!(
        "get_management_async",
        async_coverage_management_snapshot_complete,
        unsafe {
            async_calls::luminate_client_get_management_async(
                client,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_management_snapshot_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "patch_management_async",
        async_coverage_management_changeset_complete,
        unsafe {
            async_calls::luminate_client_patch_management_async(
                client,
                management_patch,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_management_changeset_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "plugin_setup_start_async",
        async_coverage_plugin_setup_session_complete,
        unsafe {
            async_calls::luminate_client_plugin_setup_start_async(
                client,
                plugin.as_ptr(),
                workflow.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_plugin_setup_session_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "plugin_setup_choose_async",
        async_coverage_plugin_setup_session_complete,
        unsafe {
            async_calls::luminate_client_plugin_setup_choose_async(
                client,
                session_id.as_ptr(),
                0,
                choice.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_plugin_setup_session_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "plugin_setup_get_async",
        async_coverage_plugin_setup_session_complete,
        unsafe {
            async_calls::luminate_client_plugin_setup_get_async(
                client,
                session_id.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_plugin_setup_session_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "plugin_setup_cancel_async",
        async_coverage_plugin_setup_session_complete,
        unsafe {
            async_calls::luminate_client_plugin_setup_cancel_async(
                client,
                session_id.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_plugin_setup_session_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "plugin_setup_workflows_async",
        async_coverage_plugin_setup_workflows_complete,
        unsafe {
            async_calls::luminate_client_plugin_setup_workflows_async(
                client,
                plugin.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_plugin_setup_workflows_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "plugin_setup_confirm_async",
        async_coverage_plugin_setup_session_complete,
        unsafe {
            async_calls::luminate_client_plugin_setup_confirm_async(
                client,
                session_id.as_ptr(),
                0,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_plugin_setup_session_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "transition_scene_to_scene_async",
        async_coverage_transition_snapshot_complete,
        unsafe {
            async_calls::luminate_client_transition_scene_to_scene_async(
                client,
                id.as_ptr(),
                id.as_ptr(),
                transition_options,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_transition_snapshot_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "transition_current_to_scene_async",
        async_coverage_transition_snapshot_complete,
        unsafe {
            async_calls::luminate_client_transition_current_to_scene_async(
                client,
                id.as_ptr(),
                transition_options,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_transition_snapshot_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "transition_scene_to_states_async",
        async_coverage_transition_snapshot_complete,
        unsafe {
            async_calls::luminate_client_transition_scene_to_states_async(
                client,
                id.as_ptr(),
                transition_states.as_ptr(),
                transition_states.len(),
                transition_options,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_transition_snapshot_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "transition_get_async",
        async_coverage_transition_snapshot_complete,
        unsafe {
            async_calls::luminate_client_transition_get_async(
                client,
                id.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_transition_snapshot_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "transition_abort_async",
        async_coverage_transition_snapshot_complete,
        unsafe {
            async_calls::luminate_client_transition_abort_async(
                client,
                id.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_transition_snapshot_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "transition_wait_async",
        async_coverage_transition_snapshot_complete,
        unsafe {
            async_calls::luminate_client_transition_wait_async(
                client,
                id.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_transition_snapshot_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "subscribe_async",
        async_coverage_event_subscription_complete,
        LuminateStatus::Unsupported,
        unsafe {
            async_events::luminate_client_subscribe_async(
                client,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_event_subscription_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "subscribe_path_async",
        async_coverage_event_subscription_complete,
        LuminateStatus::Unsupported,
        unsafe {
            async_events::luminate_client_subscribe_path_async(
                client,
                path.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_event_subscription_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "subscribe_with_baseline_async",
        async_coverage_subscription_baseline_complete,
        LuminateStatus::Unsupported,
        unsafe {
            async_events::luminate_client_subscribe_with_baseline_async(
                client,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_subscription_baseline_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "subscribe_with_baseline_path_async",
        async_coverage_subscription_baseline_complete,
        LuminateStatus::Unsupported,
        unsafe {
            async_events::luminate_client_subscribe_with_baseline_path_async(
                client,
                path.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_subscription_baseline_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "clear_target_async",
        async_coverage_status_complete,
        unsafe {
            async_calls::luminate_client_clear_target_async(
                client,
                &target,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_status_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!("set_off_async", async_coverage_status_complete, unsafe {
        async_calls::luminate_client_set_off_async(
            client,
            &target,
            Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
            Some(async_coverage_context_free),
            Some(async_coverage_status_complete),
            &raw mut operation,
        )
    });

    coverage_call!(
        "restore_appearance_async",
        async_coverage_status_complete,
        unsafe {
            async_calls::luminate_client_restore_appearance_async(
                client,
                &target,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_status_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "save_current_async",
        async_coverage_status_complete,
        unsafe {
            async_calls::luminate_client_save_current_async(
                client,
                &target,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_status_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "clear_target_selector_async",
        async_coverage_collection_outcome_complete,
        unsafe {
            async_calls::luminate_client_clear_target_selector_async(
                client,
                &empty_selector,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_collection_outcome_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "save_current_selector_async",
        async_coverage_collection_outcome_complete,
        unsafe {
            async_calls::luminate_client_save_current_selector_async(
                client,
                &empty_selector,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_collection_outcome_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "restore_appearance_selector_async",
        async_coverage_collection_outcome_complete,
        unsafe {
            async_calls::luminate_client_restore_appearance_selector_async(
                client,
                &empty_selector,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_collection_outcome_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "get_device_async",
        async_coverage_device_snapshot_complete,
        unsafe {
            async_calls::luminate_client_get_device_async(
                client,
                id.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_device_snapshot_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "get_state_async",
        async_coverage_state_snapshot_complete,
        unsafe {
            async_calls::luminate_client_get_state_async(
                client,
                id.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_state_snapshot_complete),
                &raw mut operation,
            )
        }
    );

    coverage_call!(
        "get_collection_state_async",
        async_coverage_collection_state_snapshot_complete,
        unsafe {
            async_calls::luminate_client_get_collection_state_async(
                client,
                id.as_ptr(),
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_collection_state_snapshot_complete),
                &raw mut operation,
            )
        }
    );

    assert_eq!(
        unsafe {
            async_calls::luminate_client_transition_current_to_states_async(
                client,
                transition_states.as_ptr(),
                0,
                transition_options,
                Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
                Some(async_coverage_context_free),
                Some(async_coverage_transition_snapshot_complete),
                &raw mut operation,
            )
        },
        LuminateStatus::InvalidArgument
    );

    {
        let event_path = event_socket_path(&socket_path);
        let event_listener =
            UnixListener::bind(&event_path).expect("bind async coverage event socket");
        let event_server = tokio::spawn(async move {
            let (mut stream, _) = event_listener
                .accept()
                .await
                .expect("accept async coverage event subscriber");
            let _: SubscribeHello = receive(&mut stream).await.expect("subscribe hello");
            send(
                &mut stream,
                &SubscribeAck {
                    compatibility: EventCompatibility::Compatible,
                    event_protocol_version: EVENT_PROTOCOL_VERSION,
                    daemon_version: "coverage-event-subscription".to_owned(),
                },
            )
            .await
            .expect("send async coverage event ack");
            future::pending::<()>().await;
        });
        let event_client = unsafe { client_ref(client) }
            .expect("coverage client")
            .clone();
        let event_subscription = EventSubscription::connect_path_with_ticket(
            &event_path,
            EventTicket::new([7_u8; 32]).expect("coverage event ticket"),
        )
        .await
        .expect("connect async coverage event subscription");
        let subscription = FfiSubscription {
            client: event_client,
            subscription: Arc::new(TokioMutex::new(event_subscription)),
            started_tx: None,
        };
        let subscription =
            Box::into_raw(Box::new(subscription)).cast::<LuminateEventSubscription>();
        let (tx, rx) = std_mpsc::channel::<AsyncEventSignal>();
        let context = Box::into_raw(Box::new(AsyncEventContext(tx))).cast();
        let mut operation = ptr::null_mut();
        assert_eq!(
            unsafe {
                async_events::luminate_event_subscription_next_async(
                    subscription,
                    context,
                    Some(async_event_context_free),
                    Some(async_event_complete),
                    &raw mut operation,
                )
            },
            LuminateStatus::Ok
        );
        assert_eq!(
            unsafe { async_operation::luminate_async_operation_cancel(operation) },
            async_operation::LuminateAsyncCancelResult::LuminateAsyncCancelAccepted
        );
        let AsyncEventSignal::Completion { status, .. } = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("coverage event callback")
        else {
            panic!("coverage event context destroyed before callback");
        };
        assert_eq!(status, LuminateStatus::Cancelled);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(2))
                .expect("coverage event context destroy"),
            AsyncEventSignal::Destroyed
        ));
        unsafe { async_operation::luminate_async_operation_release(operation) };
        unsafe { luminate_event_subscription_free(subscription) };
        event_server.abort();
        let _ = fs::remove_file(event_path);
    }

    assert_eq!(
        unsafe { luminate_client_ping(client) },
        LuminateStatus::Unsupported
    );
    coverage_call!("ping_async", async_coverage_status_complete, unsafe {
        async_calls::luminate_client_ping_async(
            client,
            Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
            Some(async_coverage_context_free),
            Some(async_coverage_status_complete),
            &raw mut operation,
        )
    });

    coverage_call!("rescan_async", async_coverage_status_complete, unsafe {
        async_calls::luminate_client_rescan_async(
            client,
            Box::into_raw(Box::new(AsyncCoverageContext(null_tx.clone()))).cast(),
            Some(async_coverage_context_free),
            Some(async_coverage_status_complete),
            &raw mut operation,
        )
    });

    unsafe { luminate_client_free(client) };
    unsafe { luminate_client_builder_free(builder) };
    unsafe { luminate_effect_free(effect) };
    unsafe { luminate_management_patch_builder_free(management_patch) };
    server.abort();
}

unsafe extern "C" fn async_coverage_frame_ack_complete(
    context: *mut c_void,
    operation: *const async_operation::LuminateAsyncOperation,
    status: LuminateStatus,
    _value: LuminateFrameAck,
) {
    unsafe { async_coverage_status_complete(context, operation, status) };
}

unsafe extern "C" fn async_coverage_u32_complete(
    context: *mut c_void,
    operation: *const async_operation::LuminateAsyncOperation,
    status: LuminateStatus,
    _value: u32,
) {
    unsafe { async_coverage_status_complete(context, operation, status) };
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_async_event_wait_reports_cancelled_and_poisons_started_read() {
    let primary_path = ffi_socket_path("async-event-primary");
    let event_path = ffi_socket_path("async-event-stream");
    let primary_listener = UnixListener::bind(&primary_path).expect("bind primary socket");
    let event_listener = UnixListener::bind(&event_path).expect("bind event socket");
    let primary_server = tokio::spawn(async move {
        let mut stream = accept_ffi_client(&primary_listener, "ffi-async-event").await;
        let _: Result<RequestMessage, FramingError> = receive(&mut stream).await;
    });
    let event_server = tokio::spawn(async move {
        let (mut stream, _) = event_listener.accept().await.expect("accept event client");
        let _: SubscribeHello = receive(&mut stream).await.expect("subscribe hello");
        send(
            &mut stream,
            &SubscribeAck {
                compatibility: EventCompatibility::Compatible,
                event_protocol_version: EVENT_PROTOCOL_VERSION,
                daemon_version: "ffi-async-event".to_owned(),
            },
        )
        .await
        .expect("subscribe ack");
        future::pending::<()>().await;
    });

    let connect_path = primary_path.clone();
    let client = spawn_blocking(move || {
        spawn_ffi_client(move |runtime| runtime.block_on(Client::connect_path(connect_path)))
            .expect("connect client")
    })
    .await
    .expect("connection task");
    let event_subscription = EventSubscription::connect_path_with_ticket(
        &event_path,
        EventTicket::new([9_u8; 32]).expect("event ticket"),
    )
    .await
    .expect("connect event subscription");
    let (started_tx, started_rx) = std_mpsc::channel();
    let subscription = FfiSubscription {
        client,
        subscription: Arc::new(TokioMutex::new(event_subscription)),
        started_tx: Some(started_tx),
    };
    let subscription = Box::into_raw(Box::new(subscription)).cast::<LuminateEventSubscription>();
    let subscription_address = subscription.addr();

    let (tx, rx) = std_mpsc::channel();
    let context = Box::into_raw(Box::new(AsyncEventContext(tx))).cast();
    let mut operation = ptr::null_mut();
    assert_eq!(
        unsafe {
            async_events::luminate_event_subscription_next_async(
                subscription,
                context,
                Some(async_event_context_free),
                Some(async_event_complete),
                &raw mut operation,
            )
        },
        LuminateStatus::Ok
    );

    started_rx.recv().expect("event read should begin");
    assert_eq!(
        unsafe { async_operation::luminate_async_operation_cancel(operation) },
        async_operation::LuminateAsyncCancelResult::LuminateAsyncCancelAccepted
    );
    let AsyncEventSignal::Completion { status, event } = rx.recv().expect("cancel completion")
    else {
        panic!("context destructor ran before callback");
    };
    assert_eq!(status, LuminateStatus::Cancelled);
    assert_eq!(event, 0);
    assert!(matches!(
        rx.recv().expect("context destructor"),
        AsyncEventSignal::Destroyed
    ));

    let poisoned = spawn_blocking(move || {
        let subscription = ptr::with_exposed_provenance_mut(subscription_address);
        let mut event = ptr::null_mut();
        let status = unsafe { luminate_event_subscription_next(subscription, &raw mut event) };
        unsafe { luminate_event_subscription_free(subscription) };
        status
    })
    .await
    .expect("poison check");
    assert_eq!(poisoned, LuminateStatus::ConnectionPoisoned);
    unsafe { async_operation::luminate_async_operation_release(operation) };

    event_server.abort();
    primary_server.abort();
    let _ = fs::remove_file(primary_path);
    let _ = fs::remove_file(event_path);
}

#[test]
fn c_client_builder_supports_typical_authentication_and_scope_configuration() {
    let mut builder = ptr::null_mut();
    let mut scope = ptr::null_mut();
    let path = CString::new("/tmp/luminate-builder-missing/socket").expect("C string");
    let name = CString::new("browser").expect("C string");
    let credential = b"private credential";

    unsafe {
        assert_eq!(
            luminate_client_builder_new(&raw mut builder),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_builder_set_path(builder, path.as_ptr()),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_builder_authenticate_bearer(
                builder,
                credential.as_ptr(),
                credential.len(),
            ),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_builder_authenticate_attestation(
                builder,
                name.as_ptr(),
                credential.as_ptr(),
                credential.len(),
            ),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_builder_authenticate_external(
                builder,
                name.as_ptr(),
                credential.as_ptr(),
                credential.len(),
            ),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_builder_authenticate_peer(builder),
            LuminateStatus::Ok
        );

        for operation in 0..=16 {
            assert_eq!(
                luminate_client_builder_add_scope_operation(builder, operation),
                LuminateStatus::Ok
            );
        }
        assert_eq!(
            luminate_client_builder_add_scope_operation(builder, 17),
            LuminateStatus::InvalidArgument
        );

        assert_eq!(
            luminate_session_scope_builder_new(&raw mut scope),
            LuminateStatus::Ok
        );
        let operations = [0, 2, 11];
        assert_eq!(
            luminate_session_scope_builder_add_grant(scope, operations.as_ptr(), operations.len(),),
            LuminateStatus::Ok
        );
        let device = CString::new("keyboard").expect("C string");
        let provider = CString::new("usb:1").expect("C string");
        let collection = CString::new("desk").expect("C string");
        let devices = [device.as_ptr()];
        let providers = [provider.as_ptr()];
        let collections = [collection.as_ptr()];
        let resources = LuminateResourceConstraintsInput {
            device_ids: devices.as_ptr(),
            device_id_count: devices.len(),
            provider_instances: providers.as_ptr(),
            provider_instance_count: providers.len(),
            has_host_attached: true,
            host_attached: true,
            collections: collections.as_ptr(),
            collection_count: collections.len(),
        };
        assert_eq!(
            luminate_session_scope_builder_add_constrained_grant(
                scope,
                operations.as_ptr(),
                operations.len(),
                &raw const resources,
            ),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_builder_set_scope(builder, scope),
            LuminateStatus::Ok
        );

        let mut client = ptr::null_mut();
        assert_eq!(
            luminate_client_builder_connect(builder, &raw mut client),
            LuminateStatus::DaemonUnavailable
        );
        luminate_session_scope_builder_free(scope);
        luminate_client_builder_free(builder);
    }
}

#[test]
fn c_client_builders_reject_null_and_malformed_configuration() {
    let mut builder = ptr::null_mut();
    let mut scope = ptr::null_mut();
    let invalid_utf8 = [0xff_u8, 0];

    unsafe {
        assert_eq!(
            luminate_client_builder_new(ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_builder_new(&raw mut builder),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_builder_set_path(builder, invalid_utf8.as_ptr().cast()),
            LuminateStatus::InvalidUtf8
        );
        assert_eq!(
            luminate_client_builder_authenticate_bearer(builder, ptr::null(), 1),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_builder_authenticate_attestation(
                builder,
                ptr::null(),
                b"x".as_ptr(),
                1,
            ),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_builder_authenticate_external(
                builder,
                invalid_utf8.as_ptr().cast(),
                b"x".as_ptr(),
                1,
            ),
            LuminateStatus::InvalidUtf8
        );
        assert_eq!(
            luminate_session_scope_builder_new(&raw mut scope),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_session_scope_builder_add_grant(scope, ptr::null(), 0),
            LuminateStatus::InvalidArgument
        );
        assert_eq!(
            luminate_session_scope_builder_add_grant(scope, ptr::null(), 1),
            LuminateStatus::NullPointer
        );
        let invalid_operation = [99];
        assert_eq!(
            luminate_session_scope_builder_add_grant(
                scope,
                invalid_operation.as_ptr(),
                invalid_operation.len(),
            ),
            LuminateStatus::InvalidArgument
        );
        assert_eq!(
            luminate_session_scope_builder_add_constrained_grant(
                scope,
                ptr::null(),
                0,
                ptr::null(),
            ),
            LuminateStatus::InvalidArgument
        );
        let valid_operation = [0];
        assert_eq!(
            luminate_session_scope_builder_add_constrained_grant(
                scope,
                valid_operation.as_ptr(),
                valid_operation.len(),
                ptr::null(),
            ),
            LuminateStatus::NullPointer
        );
        let empty_resources = LuminateResourceConstraintsInput {
            device_ids: ptr::null(),
            device_id_count: 0,
            provider_instances: ptr::null(),
            provider_instance_count: 0,
            has_host_attached: false,
            host_attached: false,
            collections: ptr::null(),
            collection_count: 0,
        };
        assert_eq!(
            luminate_session_scope_builder_add_constrained_grant(
                scope,
                invalid_operation.as_ptr(),
                invalid_operation.len(),
                &raw const empty_resources,
            ),
            LuminateStatus::InvalidArgument
        );
        assert_eq!(
            luminate_client_builder_set_scope(builder, ptr::null()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_builder_connect(builder, ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        luminate_session_scope_builder_free(scope);
        luminate_client_builder_free(builder);
    }
}

#[test]
fn c_session_metadata_accessors_cover_present_absent_and_boundary_values() {
    let metadata = LuminateSessionMetadata {
        authority: CString::new("local").expect("C string"),
        subject: CString::new("alice").expect("C string"),
        verified_groups: vec![CString::new("operators").expect("C string")],
        source: LuminateAuthenticationSource::External,
        source_name: Some(CString::new("sso").expect("C string")),
        credential_id: Some(CString::new("credential-7").expect("C string")),
        expires_at_unix_ms: Some(1_234),
    };
    let mut expiry = 0;

    unsafe {
        assert_eq!(
            string_view(luminate_session_metadata_authority(&metadata)),
            Some("local")
        );
        assert_eq!(
            string_view(luminate_session_metadata_subject(&metadata)),
            Some("alice")
        );
        assert_eq!(luminate_session_metadata_group_count(&metadata), 1);
        assert_eq!(
            string_view(luminate_session_metadata_group_at(&metadata, 0)),
            Some("operators")
        );
        assert!(string_view(luminate_session_metadata_group_at(&metadata, 1)).is_none());
        assert_eq!(
            luminate_session_metadata_source(&metadata) as u32,
            LuminateAuthenticationSource::External as u32
        );
        assert_eq!(
            string_view(luminate_session_metadata_source_name(&metadata)),
            Some("sso")
        );
        assert_eq!(
            string_view(luminate_session_metadata_credential_id(&metadata)),
            Some("credential-7")
        );
        assert!(luminate_session_metadata_expires_at_unix_ms(
            &metadata,
            &raw mut expiry
        ));
        assert_eq!(expiry, 1_234);

        assert!(string_view(luminate_session_metadata_authority(ptr::null())).is_none());
        assert_eq!(luminate_session_metadata_group_count(ptr::null()), 0);
        assert_eq!(
            luminate_session_metadata_source(ptr::null()) as u32,
            LuminateAuthenticationSource::Unknown as u32
        );
        assert!(!luminate_session_metadata_expires_at_unix_ms(
            &metadata,
            ptr::null_mut()
        ));
    }
}

#[test]
fn c_last_error_exposes_structured_metadata() {
    let error = Error::RateLimited {
        message: "slow down".to_owned(),
        retry_after_ms: Some(250),
    };
    assert_eq!(store_error(&error), LuminateStatus::RateLimited);
    let mut retry_after_ms = 0;
    // SAFETY: the output pointer refers to writable local storage.
    assert!(unsafe { luminate_last_error_retry_after_ms(&raw mut retry_after_ms) });
    assert_eq!(retry_after_ms, 250);

    let error = Error::PartialMutation {
        message: "later target failed".to_owned(),
        applied_targets: vec![TargetId::element("keyboard0", "keys", "escape")],
    };
    assert_eq!(store_error(&error), LuminateStatus::PartialMutation);
    assert_eq!(luminate_last_error_applied_target_count(), 1);
    let mut target = LuminateTarget {
        device_id: ptr::null(),
        surface_id: ptr::null(),
        element_id: ptr::null(),
        group_id: ptr::null(),
    };
    // SAFETY: the output pointer refers to writable local storage.
    assert!(unsafe { luminate_last_error_applied_target(0, &raw mut target) });
    // SAFETY: successful retrieval guarantees borrowed NUL-terminated strings.
    let device_id = unsafe { CStr::from_ptr(target.device_id) };
    assert_eq!(device_id.to_bytes(), b"keyboard0");
    // SAFETY: this element target has a non-null borrowed element identifier.
    let element_id = unsafe { CStr::from_ptr(target.element_id) };
    assert_eq!(element_id.to_bytes(), b"escape");
}

#[test]
fn status_mapping_covers_public_error_categories() {
    let cases = [
        (Error::DaemonUnavailable, LuminateStatus::DaemonUnavailable),
        (
            Error::AuthenticationFailed("rejected".to_owned()),
            LuminateStatus::AuthenticationFailed,
        ),
        (
            Error::IncompatibleDaemon {
                daemon_version: "1".to_owned(),
                supported_protocol_abi_version: 2,
                reason: None,
            },
            LuminateStatus::IncompatibleDaemon,
        ),
        (
            Error::IncompatibleEventSocket {
                daemon_version: "1".to_owned(),
                supported_event_protocol_version: 2,
                reason: None,
            },
            LuminateStatus::IncompatibleEventSocket,
        ),
        (
            Error::PermissionDenied { reason: None },
            LuminateStatus::PermissionDenied,
        ),
        (Error::NotFound("x".to_owned()), LuminateStatus::NotFound),
        (
            Error::Unsupported("x".to_owned()),
            LuminateStatus::Unsupported,
        ),
        (
            Error::UnknownState("x".to_owned()),
            LuminateStatus::UnknownState,
        ),
        (
            Error::InvalidArgument("x".to_owned()),
            LuminateStatus::InvalidArgument,
        ),
        (Error::Internal("x".to_owned()), LuminateStatus::Internal),
        (Error::Io("x".to_owned()), LuminateStatus::Io),
        (
            Error::Unavailable("x".to_owned()),
            LuminateStatus::Unavailable,
        ),
        (
            Error::RateLimited {
                message: "x".to_owned(),
                retry_after_ms: None,
            },
            LuminateStatus::RateLimited,
        ),
        (
            Error::PartialMutation {
                message: "x".to_owned(),
                applied_targets: Vec::new(),
            },
            LuminateStatus::PartialMutation,
        ),
        (Error::Protocol("x".to_owned()), LuminateStatus::Protocol),
        (
            Error::ConnectionPoisoned,
            LuminateStatus::ConnectionPoisoned,
        ),
        (Error::Conflict("x".to_owned()), LuminateStatus::Conflict),
    ];
    for (error, expected) in cases {
        assert_eq!(status_from_error(&error), expected);
    }
}

#[test]
fn synchronous_variant_metadata_is_sanitized_scoped_and_invalidated() {
    let denied = Error::PermissionDenied {
        reason: Some("safe\0reason".to_owned()),
    };
    assert_eq!(store_error(&denied), LuminateStatus::PermissionDenied);
    assert_eq!(
        string_view(luminate_last_error_permission_denied_reason()),
        Some("safereason")
    );
    assert_eq!(
        string_view(luminate_last_error_incompatible_daemon_version()),
        None
    );
    let mut unchanged = 41;
    // SAFETY: the output points to writable local storage.
    assert!(!unsafe { luminate_last_error_supported_protocol_abi_version(&raw mut unchanged) });
    assert_eq!(unchanged, 41);

    let primary = Error::IncompatibleDaemon {
        daemon_version: "daemon\0version".to_owned(),
        supported_protocol_abi_version: 30,
        reason: Some("primary\0reason".to_owned()),
    };
    assert_eq!(store_error(&primary), LuminateStatus::IncompatibleDaemon);
    assert_eq!(
        string_view(luminate_last_error_incompatible_daemon_version()),
        Some("daemonversion")
    );
    assert_eq!(
        string_view(luminate_last_error_incompatibility_reason()),
        Some("primaryreason")
    );
    let mut protocol = 0;
    let mut event = 42;
    // SAFETY: both outputs point to writable local storage.
    unsafe {
        assert!(luminate_last_error_supported_protocol_abi_version(
            &raw mut protocol
        ));
        assert!(!luminate_last_error_supported_event_protocol_version(
            &raw mut event
        ));
        assert!(!luminate_last_error_supported_protocol_abi_version(
            ptr::null_mut()
        ));
    };
    assert_eq!(protocol, 30);
    assert_eq!(event, 42);

    let event_error = Error::IncompatibleEventSocket {
        daemon_version: "event-daemon".to_owned(),
        supported_event_protocol_version: 8,
        reason: None,
    };
    assert_eq!(
        store_error(&event_error),
        LuminateStatus::IncompatibleEventSocket
    );
    assert_eq!(
        string_view(luminate_last_error_incompatibility_reason()),
        None
    );
    // SAFETY: `event` points to writable local storage.
    assert!(unsafe { luminate_last_error_supported_event_protocol_version(&raw mut event) });
    assert_eq!(event, 8);

    set_last_error("replacement");
    assert_eq!(
        string_view(luminate_last_error_incompatible_daemon_version()),
        None
    );
    assert_eq!(
        string_view(luminate_last_error_permission_denied_reason()),
        None
    );
}

#[test]
fn last_error_copy_supports_sizing_truncation_and_clearing() {
    set_last_error("bad\0message");
    assert_eq!(last_error().as_deref(), Some("badmessage"));

    // SAFETY: null with a zero length is the documented sizing call.
    let needed = unsafe { luminate_copy_last_error_message(ptr::null_mut(), 0) };
    assert_eq!(needed, 11);
    let mut short = [b'x'.cast_signed(); 5];
    // SAFETY: `short` provides five writable bytes.
    let copied = unsafe { luminate_copy_last_error_message(short.as_mut_ptr(), short.len()) };
    assert_eq!(copied, needed);
    assert_eq!(short[4], 0);

    clear_last_error();
    assert_eq!(last_error(), None);
    let mut empty = [b'x'.cast_signed(); 1];
    // SAFETY: `empty` provides one writable byte.
    let copied = unsafe { luminate_copy_last_error_message(empty.as_mut_ptr(), empty.len()) };
    assert_eq!(copied, 1);
    assert_eq!(empty[0], 0);
    // SAFETY: null is explicitly accepted and returns false.
    let has_retry = unsafe { luminate_last_error_retry_after_ms(ptr::null_mut()) };
    assert!(!has_retry);
}

#[test]
fn applied_target_metadata_covers_every_target_shape_and_boundaries() {
    let targets = vec![
        TargetId::device("device"),
        TargetId::surface("device", "surface"),
        TargetId::element("device", "surface", "element"),
        TargetId::group("device", "group"),
    ];
    store_error(&Error::PartialMutation {
        message: "partial".to_owned(),
        applied_targets: targets,
    });
    assert_eq!(luminate_last_error_applied_target_count(), 4);
    // SAFETY: null is explicitly accepted and returns false.
    let retrieved = unsafe { luminate_last_error_applied_target(0, ptr::null_mut()) };
    assert!(!retrieved);

    let mut target = LuminateTarget {
        device_id: ptr::null(),
        surface_id: ptr::null(),
        element_id: ptr::null(),
        group_id: ptr::null(),
    };
    for index in 0..4 {
        // SAFETY: `target` is writable and the requested index is valid.
        assert!(unsafe { luminate_last_error_applied_target(index, &raw mut target) });
    }
    assert!(target.surface_id.is_null());
    assert!(target.element_id.is_null());
    assert!(!target.group_id.is_null());
    // SAFETY: `target` is writable; the requested index is out of range.
    assert!(!unsafe { luminate_last_error_applied_target(4, &raw mut target) });
}

#[test]
#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "The test checks a matrix of independent C pointer validation paths."
)]
fn pointer_and_string_helpers_return_documented_errors() {
    let invalid = [0xff_u8, 0];
    // SAFETY: each non-null input points to a live NUL-terminated byte string.
    unsafe {
        assert_eq!(
            read_required_str(ptr::null(), "value"),
            Err(LuminateStatus::NullPointer)
        );
        assert_eq!(
            read_required_str(invalid.as_ptr().cast(), "value"),
            Err(LuminateStatus::InvalidUtf8)
        );
        assert_eq!(
            read_path(invalid.as_ptr().cast()),
            Err(LuminateStatus::InvalidUtf8)
        );
        assert!(matches!(
            client_ref(ptr::null_mut()),
            Err(LuminateStatus::NullPointer)
        ));
        assert!(matches!(
            subscription_ref(ptr::null_mut()),
            Err(LuminateStatus::NullPointer)
        ));
        assert_eq!(
            out_ptr_mut::<u8>(ptr::null_mut(), "byte"),
            Err(LuminateStatus::NullPointer)
        );
    }
}

#[test]
#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "The test checks the common null-safe contract of C free functions."
)]
fn stopped_workers_fail_cleanly_and_handles_are_null_safe() {
    let client = FfiClient::stopped();
    assert_eq!(
        call_client(&client, |_| async { 1 }),
        Err(LuminateStatus::Internal)
    );
    assert_eq!(ffi_guard(|| panic!("test panic")), LuminateStatus::Internal);

    // SAFETY: null is explicitly accepted by every free operation.
    unsafe {
        luminate_client_free(ptr::null_mut());
        luminate_event_subscription_free(ptr::null_mut());
        luminate_string_free(ptr::null_mut());
        luminate_server_info_free(ptr::null_mut());
    }
}

#[test]
#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "The test checks a matrix of independent public C validation paths."
)]
fn public_operations_validate_inputs_before_dispatch() {
    let mut client_out = ptr::null_mut();
    let mut subscription_out = ptr::null_mut();
    let missing = CString::new("/tmp/luminate-tests-missing/socket").expect("valid path");
    let invalid = [0xff_u8, 0];

    // SAFETY: all pointers are null or point to live local storage as documented.
    unsafe {
        assert_eq!(
            luminate_client_connect(ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_connect_path(ptr::null(), &raw mut client_out),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_connect_path(invalid.as_ptr().cast(), &raw mut client_out),
            LuminateStatus::InvalidUtf8
        );
        assert_eq!(
            luminate_client_connect_path(missing.as_ptr(), &raw mut client_out),
            LuminateStatus::DaemonUnavailable
        );
        assert_eq!(
            luminate_client_subscribe_path(ptr::null_mut(), ptr::null(), &raw mut subscription_out),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_daemon_version(ptr::null_mut(), ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_socket_path(ptr::null_mut(), ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_event_socket_path(ptr::null_mut(), ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_server_info(ptr::null_mut(), ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_ping(ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_purge_withdrawn_device(ptr::null_mut(), ptr::null()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_rescan(ptr::null_mut()),
            LuminateStatus::NullPointer
        );
    }
}

#[test]
fn allocated_server_info_is_freed_and_cleared() {
    let info = Box::into_raw(Box::new(server_info_to_ffi(ServerInfo {
        daemon_name: "daemon\0name".to_owned(),
        daemon_version: "1.2.3".to_owned(),
        protocol_abi_version: 7,
    })));
    // SAFETY: `info` is a live owned server-information object.
    let () = unsafe {
        assert_eq!(
            view_bytes(luminate_server_info_daemon_name(info)),
            b"daemon\0name"
        );
        assert_eq!(
            view_bytes(luminate_server_info_daemon_version(info)),
            b"1.2.3"
        );
        assert_eq!(luminate_server_info_protocol_abi_version(info), 7);
        luminate_server_info_free(info);
    };
    assert!(!luminate_version().is_null());
}

#[test]
fn client_worker_reports_connection_failure_and_joins() {
    let Err(error) =
        spawn_ffi_client(|_| Err(Error::Unavailable("fixture unavailable".to_owned())))
    else {
        panic!("connection failure should be returned");
    };
    assert!(matches!(error, Error::Unavailable(_)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_execution_domain_routes_concurrent_requests_and_shuts_down() {
    let path = ffi_socket_path("concurrent-domain");
    let listener = UnixListener::bind(&path).expect("bind FFI test socket");
    let server = tokio::spawn(async move {
        let mut stream = accept_ffi_client(&listener, "ffi-test-daemon").await;

        // Do not answer either request until both have arrived. A serialized
        // worker cannot make progress through this exchange.
        let first: RequestMessage = receive(&mut stream).await.expect("receive first request");
        let second: RequestMessage = receive(&mut stream).await.expect("receive second request");
        assert!(matches!(first.request, Request::Ping));
        assert!(matches!(second.request, Request::Ping));

        for id in [second.id, first.id] {
            send(
                &mut stream,
                &ResponseMessage {
                    id,
                    response: Response {
                        status: ResponseStatus::Ack,
                    },
                },
            )
            .await
            .expect("send ping response");
        }

        let closed: Result<RequestMessage, FramingError> = receive(&mut stream).await;
        assert!(
            closed.is_err(),
            "dropping the domain should close the connection"
        );
    });

    let connect_path = path.clone();
    let client = spawn_blocking(move || {
        spawn_ffi_client(move |runtime| runtime.block_on(Client::connect_path(connect_path)))
            .expect("connect FFI client")
    })
    .await
    .expect("connection task");
    let client = Box::into_raw(Box::new(client)).cast::<LuminateClient>();
    let client_address = client.expose_provenance();

    thread::scope(|scope| {
        let first = scope.spawn(move || {
            // SAFETY: the external handle remains live until both scoped calls return.
            unsafe { luminate_client_ping(ptr::with_exposed_provenance_mut(client_address)) }
        });
        let second = scope.spawn(move || {
            // SAFETY: concurrent operations are supported; destruction is serialized below.
            unsafe { luminate_client_ping(ptr::with_exposed_provenance_mut(client_address)) }
        });

        assert_eq!(first.join().expect("first caller"), LuminateStatus::Ok);
        assert_eq!(second.join().expect("second caller"), LuminateStatus::Ok);
    });

    // SAFETY: both concurrent calls have returned and this handle is released once.
    unsafe { luminate_client_free(client) };
    server.await.expect("FFI server task");
    let _ = fs::remove_file(path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "The test exercises one end-to-end ownership lifecycle across the C boundary."
)]
async fn exported_ffi_worker_survives_a_malformed_reply_inside_tokio() {
    let path = ffi_socket_path("worker-survival");
    let listener = UnixListener::bind(&path).expect("bind FFI test socket");
    let server = tokio::spawn(async move {
        let mut stream = accept_ffi_client(&listener, "ffi-test-daemon").await;

        let rejected: RequestMessage = receive(&mut stream).await.expect("receive purge request");
        send(
            &mut stream,
            &ResponseMessage {
                id: rejected.id,
                response: Response {
                    status: ResponseStatus::Devices(Vec::new()),
                },
            },
        )
        .await
        .expect("send mismatched purge response");

        let ping: RequestMessage = receive(&mut stream).await.expect("receive ping request");
        assert_eq!(ping.id, 1);
        assert!(matches!(ping.request, Request::Ping));
        send(
            &mut stream,
            &ResponseMessage {
                id: ping.id,
                response: Response {
                    status: ResponseStatus::Ack,
                },
            },
        )
        .await
        .expect("send ping response");

        let follow_up: RequestMessage = receive(&mut stream)
            .await
            .expect("receive server-info request after error");
        send(
            &mut stream,
            &ResponseMessage {
                id: follow_up.id,
                response: Response {
                    status: ResponseStatus::ServerInfo(ProtocolServerInfo {
                        daemon_name: "fixture".to_owned(),
                        daemon_version: "ffi-test-daemon".to_owned(),
                        protocol_abi_version: luminate_protocol::PROTOCOL_ABI_VERSION,
                    }),
                },
            },
        )
        .await
        .expect("send server-info response");
    });

    let path_c = CString::new(path.as_os_str().as_encoded_bytes()).expect("valid socket path");
    let device = CString::new("retired").expect("valid device id");
    let mut client = ptr::null_mut();
    let mut info = ptr::null_mut();
    // SAFETY: all C strings and output storage remain live for their calls; returned ownership
    // is released exactly once below.
    let () = unsafe {
        assert_eq!(
            luminate_client_connect_path(path_c.as_ptr(), &raw mut client),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_purge_withdrawn_device(client, device.as_ptr()),
            LuminateStatus::Protocol
        );
        assert_eq!(luminate_client_ping(client), LuminateStatus::Ok);

        assert_eq!(
            luminate_client_server_info(client, &raw mut info),
            LuminateStatus::Ok
        );
        assert_eq!(
            view_bytes(luminate_server_info_daemon_name(info)),
            b"fixture"
        );
        luminate_server_info_free(info);
        luminate_client_free(client);
    };
    server.await.expect("FFI server task");
    let _ = fs::remove_file(path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "The test compares two complete independent C handle lifecycles."
)]
async fn exported_ffi_clients_keep_independent_workers_and_versions() {
    let path = ffi_socket_path("independent");
    let listener = UnixListener::bind(&path).expect("bind FFI test socket");
    let server = tokio::spawn(async move {
        let first = accept_ffi_client(&listener, "daemon-one").await;
        let second = accept_ffi_client(&listener, "daemon-two").await;
        (first, second)
    });
    let path_c = CString::new(path.as_os_str().as_encoded_bytes()).expect("valid socket path");
    let mut first = ptr::null_mut();
    let mut second = ptr::null_mut();

    // SAFETY: the path and writable output slots remain live, and both independent handles are
    // freed exactly once after their borrowed version strings are copied.
    let () = unsafe {
        assert_eq!(
            luminate_client_connect_path(path_c.as_ptr(), &raw mut first),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_connect_path(path_c.as_ptr(), &raw mut second),
            LuminateStatus::Ok
        );
        let mut first_version = ptr::null_mut();
        let mut second_version = ptr::null_mut();
        assert_eq!(
            luminate_client_daemon_version(first, &raw mut first_version),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_daemon_version(second, &raw mut second_version),
            LuminateStatus::Ok
        );
        assert_eq!(CStr::from_ptr(first_version).to_bytes(), b"daemon-one");
        assert_eq!(CStr::from_ptr(second_version).to_bytes(), b"daemon-two");
        luminate_string_free(first_version);
        luminate_string_free(second_version);

        let mut primary_path = ptr::null_mut();
        let mut event_path = ptr::null_mut();
        assert_eq!(
            luminate_client_socket_path(first, &raw mut primary_path),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_event_socket_path(first, &raw mut event_path),
            LuminateStatus::Ok
        );
        assert_eq!(
            CStr::from_ptr(primary_path).to_bytes(),
            path.as_os_str().as_encoded_bytes()
        );
        assert_eq!(
            CStr::from_ptr(event_path).to_bytes(),
            event_socket_path(&path).as_os_str().as_encoded_bytes()
        );
        luminate_string_free(primary_path);
        luminate_string_free(event_path);

        luminate_client_free(first);
        luminate_client_free(second);
    };
    let streams = server.await.expect("FFI server task");
    drop(streams);
    let _ = fs::remove_file(path);
}
