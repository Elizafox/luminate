// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Shared asynchronous C submission and completion machinery.

use super::async_operation::LuminateAsyncOperation;
use super::{
    Client, Error, FfiClient, LuminateStatus, c_void, clear_last_error, mpsc, set_last_error,
    spawn_client_task, status_from_error,
};
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

/// Releases a consumer-owned asynchronous completion context.
pub type LuminateCompletionContextFreeFn = Option<unsafe extern "C" fn(context: *mut c_void)>;

pub(crate) fn finish_context(
    context: usize,
    completion_context_free: LuminateCompletionContextFreeFn,
) {
    if let Some(free) = completion_context_free {
        // SAFETY: an accepted submission transfers this context and destructor
        // to the completion contract.
        unsafe { free(context as *mut c_void) };
    }
}

pub(crate) fn validate_submission<C>(
    on_complete: Option<C>,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> Result<C, LuminateStatus> {
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

/// Submits one client future and serializes its terminal delivery.
///
/// `deliver` receives a payload only when the operation completed successfully.
pub(crate) unsafe fn submit_client_async<R, F, Fut, D>(
    client: &FfiClient,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    out_operation: *mut *mut LuminateAsyncOperation,
    future: F,
    deliver: D,
) -> LuminateStatus
where
    R: Send + 'static,
    F: FnOnce(Arc<Client>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<R, Error>> + Send + 'static,
    D: FnOnce(*const LuminateAsyncOperation, LuminateStatus, Option<R>) + Send + 'static,
{
    let context = completion_context as usize;
    let payload = Arc::new(Mutex::new(None::<R>));
    let delivery_payload = Arc::clone(&payload);
    let delivery_client = client.clone();
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let operation = LuminateAsyncOperation::new(move |operation| {
        let _ = delivery_client.dispatch(Box::new(move || {
            let _ = ready_rx.recv();
            let status = operation.status().unwrap_or(LuminateStatus::Internal);
            let payload = delivery_payload
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            let payload = if status == LuminateStatus::Ok {
                payload
            } else {
                drop(payload);
                None
            };
            deliver(Arc::as_ptr(&operation), status, payload);
            finish_context(context, completion_context_free);
        }));
    });
    if let Err(status) = spawn_client_task(
        client,
        Arc::clone(&operation),
        move |rust_client, operation| async move {
            match future(rust_client).await {
                Ok(value) => {
                    *payload.lock().unwrap_or_else(PoisonError::into_inner) = Some(value);
                    let _ = operation.finish(LuminateStatus::Ok, None);
                }
                Err(error) => {
                    let _ = operation.finish(status_from_error(&error), Some(&error));
                }
            }
        },
    ) {
        return status;
    }
    // SAFETY: the caller supplied a validated, writable out-pointer.
    unsafe { *out_operation = LuminateAsyncOperation::into_owned_handle(&operation) };
    clear_last_error();
    let _ = ready_tx.send(());
    LuminateStatus::Ok
}
