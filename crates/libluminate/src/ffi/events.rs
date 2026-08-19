// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::{
    AssertUnwindSafe, FfiSubscription, LuminateClient, LuminateEventSubscription, LuminateStatus,
    PathBuf, c_char, call_client, clear_last_error, client_ref, ffi_guard, panic, read_path,
    set_last_error, spawn_ffi_subscription, store_error, write_subscription,
};

/// Subscribes to daemon events using the event path derived from this client's
/// primary socket.
///
/// # Safety
///
/// `client` must be valid and `out_subscription` must be a valid non-null
/// out-pointer. Release the returned handle with
/// `luminate_event_subscription_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_subscribe(
    client: *mut LuminateClient,
    out_subscription: *mut *mut LuminateEventSubscription,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: validated by helper.
        let client = match unsafe { client_ref(client) } {
            Ok(client) => client,
            Err(status) => return status,
        };
        if out_subscription.is_null() {
            set_last_error("event subscription output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let subscription =
            match call_client(client, |client| async move { client.subscribe().await }) {
                Ok(Ok(subscription)) => subscription,
                Ok(Err(error)) => return store_error(&error),
                Err(status) => return status,
            };
        let subscription = spawn_ffi_subscription(client, subscription);
        clear_last_error();
        // SAFETY: output pointer was checked above.
        unsafe { write_subscription(out_subscription, subscription) };
        LuminateStatus::Ok
    })
}

/// Subscribes to an explicit daemon event socket using this client's
/// authenticated, single-use event ticket.
///
/// # Safety
///
/// `path` must be a valid NUL-terminated UTF-8 string and
/// `out_subscription` a valid non-null out-pointer. Release the returned
/// handle with `luminate_event_subscription_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_subscribe_path(
    client: *mut LuminateClient,
    path: *const c_char,
    out_subscription: *mut *mut LuminateEventSubscription,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: validated by helper.
        let client = match unsafe { client_ref(client) } {
            Ok(client) => client,
            Err(status) => return status,
        };
        if out_subscription.is_null() {
            set_last_error("event subscription output pointer is null");
            return LuminateStatus::NullPointer;
        }
        // SAFETY: pointer was checked and is documented as NUL-terminated.
        let path = match unsafe { read_path(path) } {
            Ok(path) => PathBuf::from(path),
            Err(status) => return status,
        };
        let subscription = match call_client(client, move |client| async move {
            client.subscribe_path(path).await
        }) {
            Ok(Ok(subscription)) => subscription,
            Ok(Err(error)) => return store_error(&error),
            Err(status) => return status,
        };
        let subscription = spawn_ffi_subscription(client, subscription);
        clear_last_error();
        // SAFETY: output pointer was checked above.
        unsafe { write_subscription(out_subscription, subscription) };
        LuminateStatus::Ok
    })
}

/// Releases an event subscription handle.
///
/// # Safety
///
/// `subscription` must be null or a uniquely owned valid handle and must not
/// be freed more than once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_event_subscription_free(
    subscription: *mut LuminateEventSubscription,
) {
    if subscription.is_null() {
        return;
    }
    // SAFETY: caller guarantees unique ownership of a valid handle.
    let boxed = unsafe { Box::from_raw(subscription.cast::<FfiSubscription>()) };
    let _ = panic::catch_unwind(AssertUnwindSafe(move || drop(boxed)));
}
