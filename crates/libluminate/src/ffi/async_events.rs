// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Asynchronous connection-domain event operations.

use super::async_common::{LuminateCompletionContextFreeFn, finish_context, validate_submission};
use super::async_operation::LuminateAsyncOperation;
use super::{
    FfiSubscription, LuminateClient, LuminateEventSubscription, LuminateStatus, PathBuf, c_char,
    c_void, clear_last_error, client_ref, ffi_guard, mpsc, read_path, set_last_error,
    spawn_client_task, spawn_ffi_subscription, status_from_error, subscription_ref,
};
use crate::ffi_typed::{LuminateEvent, LuminateTopologySnapshot};
use std::ptr;
use std::sync::{Arc, Mutex, PoisonError};

/// Completes an asynchronous event-subscription operation.
pub type LuminateAsyncEventSubscriptionCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        subscription: *mut LuminateEventSubscription,
    ),
>;

/// Completes an asynchronous subscription-and-baseline operation.
pub type LuminateAsyncSubscriptionBaselineCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        subscription: *mut LuminateEventSubscription,
        topology: *mut LuminateTopologySnapshot,
    ),
>;

/// Completes an asynchronous event wait.
pub type LuminateAsyncEventCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        event: *mut LuminateEvent,
    ),
>;

unsafe fn subscribe_async(
    client: *mut LuminateClient,
    path: Option<PathBuf>,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncEventSubscriptionCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client) } {
            Ok(client) => client.clone(),
            Err(status) => return status,
        };
        let on_complete = match validate_submission(on_complete, out_operation) {
            Ok(callback) => callback,
            Err(status) => return status,
        };
        let context = completion_context as usize;
        let payload = Arc::new(Mutex::new(None::<FfiSubscription>));
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
                let subscription = if status == LuminateStatus::Ok {
                    payload.map_or(ptr::null_mut(), |subscription| {
                        Box::into_raw(Box::new(subscription)).cast()
                    })
                } else {
                    drop(payload);
                    ptr::null_mut()
                };
                // SAFETY: the callback and context were validated and retained
                // by the accepted submission.
                unsafe {
                    on_complete(
                        context as *mut c_void,
                        Arc::as_ptr(&operation),
                        status,
                        subscription,
                    );
                };
                finish_context(context, completion_context_free);
            }));
        });
        let task_client = client.clone();
        if let Err(status) = spawn_client_task(
            &client,
            Arc::clone(&operation),
            move |rust_client, operation| async move {
                let result = match path {
                    Some(path) => rust_client.subscribe_path(path).await,
                    None => rust_client.subscribe().await,
                };
                match result {
                    Ok(subscription) => {
                        *payload.lock().unwrap_or_else(PoisonError::into_inner) =
                            Some(spawn_ffi_subscription(&task_client, subscription));
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
        unsafe { *out_operation = LuminateAsyncOperation::into_owned_handle(&operation) };
        clear_last_error();
        let _ = ready_tx.send(());
        LuminateStatus::Ok
    })
}

/// Asynchronously subscribes using the client's derived event path.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_subscribe_async(
    client: *mut LuminateClient,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncEventSubscriptionCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    unsafe {
        subscribe_async(
            client,
            None,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
        )
    }
}

/// Asynchronously subscribes using an explicit event socket path.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_subscribe_path_async(
    client: *mut LuminateClient,
    path: *const c_char,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncEventSubscriptionCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    if path.is_null() {
        set_last_error("event path pointer is null");
        return LuminateStatus::NullPointer;
    }
    let path = match unsafe { read_path(path) } {
        Ok(path) => PathBuf::from(path),
        Err(status) => return status,
    };
    unsafe {
        subscribe_async(
            client,
            Some(path),
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
        )
    }
}

unsafe fn subscribe_with_baseline_async(
    client: *mut LuminateClient,
    path: Option<PathBuf>,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncSubscriptionBaselineCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client) } {
            Ok(client) => client.clone(),
            Err(status) => return status,
        };
        let on_complete = match validate_submission(on_complete, out_operation) {
            Ok(callback) => callback,
            Err(status) => return status,
        };
        let context = completion_context as usize;
        let payload = Arc::new(Mutex::new(
            None::<(FfiSubscription, LuminateTopologySnapshot)>,
        ));
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
                let (subscription, topology) = if status == LuminateStatus::Ok {
                    payload.map_or(
                        (ptr::null_mut(), ptr::null_mut()),
                        |(subscription, topology)| {
                            (
                                Box::into_raw(Box::new(subscription)).cast(),
                                Box::into_raw(Box::new(topology)),
                            )
                        },
                    )
                } else {
                    drop(payload);
                    (ptr::null_mut(), ptr::null_mut())
                };
                unsafe {
                    on_complete(
                        context as *mut c_void,
                        Arc::as_ptr(&operation),
                        status,
                        subscription,
                        topology,
                    );
                };
                finish_context(context, completion_context_free);
            }));
        });
        let task_client = client.clone();
        if let Err(status) = spawn_client_task(
            &client,
            Arc::clone(&operation),
            move |rust_client, operation| async move {
                let result = match path {
                    Some(path) => rust_client.subscribe_with_baseline_path(path).await,
                    None => rust_client.subscribe_with_baseline().await,
                };
                match result {
                    Ok((subscription, devices)) => {
                        *payload.lock().unwrap_or_else(PoisonError::into_inner) = Some((
                            spawn_ffi_subscription(&task_client, subscription),
                            LuminateTopologySnapshot(devices),
                        ));
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
        unsafe { *out_operation = LuminateAsyncOperation::into_owned_handle(&operation) };
        clear_last_error();
        let _ = ready_tx.send(());
        LuminateStatus::Ok
    })
}

/// Asynchronously subscribes and retrieves a topology baseline.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_subscribe_with_baseline_async(
    client: *mut LuminateClient,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncSubscriptionBaselineCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    unsafe {
        subscribe_with_baseline_async(
            client,
            None,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
        )
    }
}

/// Asynchronously subscribes on an explicit path and retrieves a baseline.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_subscribe_with_baseline_path_async(
    client: *mut LuminateClient,
    path: *const c_char,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncSubscriptionBaselineCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    if path.is_null() {
        set_last_error("event path pointer is null");
        return LuminateStatus::NullPointer;
    }
    let path = match unsafe { read_path(path) } {
        Ok(path) => PathBuf::from(path),
        Err(status) => return status,
    };
    unsafe {
        subscribe_with_baseline_async(
            client,
            Some(path),
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
        )
    }
}

/// Asynchronously waits for the next event on a subscription.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_event_subscription_next_async(
    subscription: *mut LuminateEventSubscription,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncEventCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let subscription = match unsafe { subscription_ref(subscription) } {
            Ok(subscription) => subscription.clone(),
            Err(status) => return status,
        };
        let on_complete = match validate_submission(on_complete, out_operation) {
            Ok(callback) => callback,
            Err(status) => return status,
        };
        let context = completion_context as usize;
        let payload = Arc::new(Mutex::new(None::<LuminateEvent>));
        let delivery_payload = Arc::clone(&payload);
        let delivery_client = subscription.client.clone();
        let (ready_tx, ready_rx) = mpsc::channel::<()>();
        let operation = LuminateAsyncOperation::new(move |operation| {
            let _ = delivery_client.dispatch(Box::new(move || {
                let _ = ready_rx.recv();
                let status = operation.status().unwrap_or(LuminateStatus::Internal);
                let payload = delivery_payload
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                let event = if status == LuminateStatus::Ok {
                    payload.map_or(ptr::null_mut(), |event| Box::into_raw(Box::new(event)))
                } else {
                    drop(payload);
                    ptr::null_mut()
                };
                unsafe {
                    on_complete(
                        context as *mut c_void,
                        Arc::as_ptr(&operation),
                        status,
                        event,
                    );
                };
                finish_context(context, completion_context_free);
            }));
        });
        let inner = Arc::clone(&subscription.subscription);
        #[cfg(test)]
        let started_tx = subscription.started_tx.clone();
        if let Err(status) = spawn_client_task(
            &subscription.client,
            Arc::clone(&operation),
            move |_, operation| async move {
                let mut subscription = inner.lock().await;
                #[cfg(test)]
                if let Some(started_tx) = started_tx {
                    let _ = started_tx.send(());
                }
                let result = subscription.next_event().await;
                match result {
                    Ok(event) => {
                        *payload.lock().unwrap_or_else(PoisonError::into_inner) =
                            Some(LuminateEvent(event));
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
        unsafe { *out_operation = LuminateAsyncOperation::into_owned_handle(&operation) };
        clear_last_error();
        let _ = ready_tx.send(());
        LuminateStatus::Ok
    })
}
