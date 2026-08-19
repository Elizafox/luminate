// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Asynchronous client connection operations.

use super::async_common::LuminateCompletionContextFreeFn;
use super::async_operation::LuminateAsyncOperation;
use super::{
    Authentication, Client, ClientBuilder, FfiClient, FfiClientAttempt, LuminateClient,
    LuminateClientBuilder, LuminateStatus, c_char, c_void, clear_last_error, ffi_guard, mpsc,
    read_path, set_last_error, start_ffi_client_async, status_from_error, store_error, thread,
};
use std::future::Future;
use std::ptr;
use std::sync::{Arc, Mutex, PoisonError};

/// Completes an asynchronous connection attempt with an owned client.
type AsyncClientCallback = unsafe extern "C" fn(
    context: *mut c_void,
    operation: *const LuminateAsyncOperation,
    status: LuminateStatus,
    client: *mut LuminateClient,
);
pub type LuminateAsyncClientCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        client: *mut LuminateClient,
    ),
>;

fn validate_submission(
    on_complete: LuminateAsyncClientCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> Result<AsyncClientCallback, LuminateStatus> {
    let Some(on_complete) = on_complete else {
        set_last_error("asynchronous completion callback is null");
        return Err(LuminateStatus::NullPointer);
    };
    if out_operation.is_null() {
        set_last_error("asynchronous operation output pointer is null");
        return Err(LuminateStatus::NullPointer);
    }
    Ok(on_complete)
}

#[allow(
    clippy::too_many_lines,
    reason = "Connection acceptance and every rare thread/queue teardown path stay together so context ownership changes at one auditable boundary."
)]
fn connect_async<F, Fut>(
    connect: F,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncClientCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<Client, super::Error>> + Send + 'static,
{
    ffi_guard(|| {
        let on_complete = match validate_submission(on_complete, out_operation) {
            Ok(callback) => callback,
            Err(status) => return status,
        };
        let mut attempt = match start_ffi_client_async(connect) {
            Ok(attempt) => attempt,
            Err(error) => return store_error(&error),
        };
        let completion_sender = attempt.completion_sender();
        let coordinator_join_sender = completion_sender.clone();
        let context = completion_context as usize;
        let payload = Arc::new(Mutex::new(None::<FfiClient>));
        let delivery_payload = Arc::clone(&payload);
        let (ready_tx, ready_rx) = mpsc::channel::<()>();
        let operation = LuminateAsyncOperation::new(move |operation| {
            let _ = completion_sender.send(Box::new(move || {
                let _ = ready_rx.recv();
                let status = operation.status().unwrap_or(LuminateStatus::Internal);
                let payload = delivery_payload
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                let client = if status == LuminateStatus::Ok {
                    payload.map_or(ptr::null_mut(), |client| {
                        Box::into_raw(Box::new(client)).cast()
                    })
                } else {
                    drop(payload);
                    ptr::null_mut()
                };
                unsafe {
                    on_complete(
                        context as *mut c_void,
                        Arc::as_ptr(&operation),
                        status,
                        client,
                    );
                };
                if let Some(free) = completion_context_free {
                    unsafe { free(context as *mut c_void) };
                }
            }));
        });
        let Some(abort_handle) = attempt.take_connect_abort_handle() else {
            drop(operation);
            drop(
                attempt
                    .finish()
                    .map_err(super::FfiClientFailure::into_error),
            );
            set_last_error("connection worker stopped before installing cancellation");
            return LuminateStatus::Internal;
        };
        operation.set_abort_handle(abort_handle);
        let coordinator_operation = Arc::clone(&operation);
        let (attempt_tx, attempt_rx) = mpsc::channel::<FfiClientAttempt>();
        let coordinator = thread::Builder::new()
            .name("luminate-ffi-connect-coordinator".to_owned())
            .spawn(move || {
                let Ok(attempt) = attempt_rx.recv() else {
                    return;
                };
                match attempt.finish() {
                    Ok(client) => {
                        *payload.lock().unwrap_or_else(PoisonError::into_inner) = Some(client);
                        let _ = coordinator_operation.finish(LuminateStatus::Ok, None);
                    }
                    Err(failure) => {
                        let status = status_from_error(failure.error());
                        let _ = coordinator_operation.finish(status, Some(failure.error()));
                        drop(failure.into_error_without_join());
                    }
                }
            });
        let coordinator = match coordinator {
            Ok(coordinator) => coordinator,
            Err(error) => {
                drop(operation);
                drop(
                    attempt
                        .finish()
                        .map_err(super::FfiClientFailure::into_error),
                );
                set_last_error(format!("failed to spawn connection coordinator: {error}"));
                return LuminateStatus::Internal;
            }
        };
        if let Err(error) = attempt_tx.send(attempt) {
            let _ = coordinator.join();
            drop(operation);
            drop(
                error
                    .0
                    .finish()
                    .map_err(super::FfiClientFailure::into_error),
            );
            set_last_error("connection coordinator stopped before accepting its attempt");
            return LuminateStatus::Internal;
        }
        if let Err(error) = coordinator_join_sender.send(Box::new(move || {
            let _ = coordinator.join();
        })) {
            error.0();
            drop(operation);
            set_last_error("libluminate completion dispatcher is not running");
            return LuminateStatus::Internal;
        }
        unsafe { *out_operation = LuminateAsyncOperation::into_owned_handle(&operation) };
        clear_last_error();
        let _ = ready_tx.send(());
        LuminateStatus::Ok
    })
}

/// Asynchronously connects to the default daemon socket.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_connect_async(
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncClientCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    connect_async(
        Client::connect,
        completion_context,
        completion_context_free,
        on_complete,
        out_operation,
    )
}

/// Asynchronously connects to an explicit daemon socket path.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_connect_path_async(
    path: *const c_char,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncClientCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    if path.is_null() {
        set_last_error("path pointer is null");
        return LuminateStatus::NullPointer;
    }
    let path = match unsafe { read_path(path) } {
        Ok(path) => path.to_owned(),
        Err(status) => return status,
    };
    connect_async(
        move || Client::connect_path(path),
        completion_context,
        completion_context_free,
        on_complete,
        out_operation,
    )
}

/// Asynchronously connects using a deep copy of a client builder.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_builder_connect_async(
    builder: *const LuminateClientBuilder,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncClientCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    let Some(builder) = (unsafe { builder.as_ref() }) else {
        set_last_error("client builder pointer is null");
        return LuminateStatus::NullPointer;
    };
    let path = builder.path.clone();
    let authentication: Authentication = builder.authentication.clone();
    let scope = builder.scope.clone();
    connect_async(
        move || async move {
            let mut configured = ClientBuilder::new().authentication(authentication);
            if let Some(path) = path {
                configured = configured.path(path);
            }
            if let Some(scope) = scope {
                configured = configured.scope(scope);
            }
            configured.connect().await
        },
        completion_context,
        completion_context_free,
        on_complete,
        out_operation,
    )
}
