// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::{
    DeviceId, LuminateClient, LuminateServerInfo, LuminateStatus, c_char, call_client,
    clear_last_error, client_ref, ffi_guard, out_ptr_mut, read_required_str, sanitize_cstring,
    server_info_to_ffi, set_last_error, store_error,
};

/// Retrieves the connected daemon version string as a newly allocated C string.
///
/// # Safety
///
/// `client` must be a valid client handle. `out_version` must be a valid
/// non-null out-pointer. The returned string must be released with
/// `luminate_string_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_daemon_version(
    client: *mut LuminateClient,
    out_version: *mut *mut c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: validated by helper.
        let client = match unsafe { client_ref(client) } {
            Ok(client) => client,
            Err(status) => return status,
        };

        // SAFETY: validated by helper.
        let out_version = match unsafe { out_ptr_mut(out_version, "version") } {
            Ok(out_version) => out_version,
            Err(status) => return status,
        };

        let version = match client.daemon_version() {
            Ok(version) => version.to_owned(),
            Err(status) => return status,
        };

        clear_last_error();
        *out_version = sanitize_cstring(version).into_raw();
        LuminateStatus::Ok
    })
}

/// Retrieves the primary daemon socket path as a newly allocated C string.
///
/// # Safety
///
/// `client` must be a valid client handle. `out_path` must be a valid non-null
/// out-pointer. The returned string must be released with
/// `luminate_string_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_socket_path(
    client: *mut LuminateClient,
    out_path: *mut *mut c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: validated by helper.
        let client = match unsafe { client_ref(client) } {
            Ok(client) => client,
            Err(status) => return status,
        };

        // SAFETY: validated by helper.
        let out_path = match unsafe { out_ptr_mut(out_path, "socket path") } {
            Ok(out_path) => out_path,
            Err(status) => return status,
        };

        let path = match client.socket_path() {
            Ok(path) => path,
            Err(status) => return status,
        };
        let Some(path) = path.to_str() else {
            set_last_error("connected socket path is not valid UTF-8");
            return LuminateStatus::InvalidUtf8;
        };

        clear_last_error();
        *out_path = sanitize_cstring(path.to_owned()).into_raw();
        LuminateStatus::Ok
    })
}

/// Retrieves the conventional event socket path as a newly allocated C string.
///
/// # Safety
///
/// `client` must be a valid client handle. `out_path` must be a valid non-null
/// out-pointer. The returned string must be released with
/// `luminate_string_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_event_socket_path(
    client: *mut LuminateClient,
    out_path: *mut *mut c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: validated by helper.
        let client = match unsafe { client_ref(client) } {
            Ok(client) => client,
            Err(status) => return status,
        };

        // SAFETY: validated by helper.
        let out_path = match unsafe { out_ptr_mut(out_path, "event socket path") } {
            Ok(out_path) => out_path,
            Err(status) => return status,
        };

        let path = match client.event_socket_path() {
            Ok(path) => path,
            Err(status) => return status,
        };
        let Some(path) = path.to_str() else {
            set_last_error("derived event socket path is not valid UTF-8");
            return LuminateStatus::InvalidUtf8;
        };

        clear_last_error();
        *out_path = sanitize_cstring(path.to_owned()).into_raw();
        LuminateStatus::Ok
    })
}

/// Fetches server information as a newly allocated owned object.
///
/// # Safety
///
/// `client` must be a valid client handle. `out_info` must be a valid non-null
/// out-pointer. The returned object must be released with
/// `luminate_server_info_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_server_info(
    client: *mut LuminateClient,
    out_info: *mut *mut LuminateServerInfo,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: validated by helper.
        let client = match unsafe { client_ref(client) } {
            Ok(client) => client,
            Err(status) => return status,
        };

        // SAFETY: validated by helper.
        let out_info = match unsafe { out_ptr_mut(out_info, "server info") } {
            Ok(out_info) => out_info,
            Err(status) => return status,
        };

        let outcome = match call_client(client, |client| async move { client.server_info().await })
        {
            Ok(outcome) => outcome,
            Err(status) => return status,
        };

        match outcome {
            Ok(info) => {
                clear_last_error();
                *out_info = Box::into_raw(Box::new(server_info_to_ffi(info)));
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}

/// Checks that the client's existing daemon connection is responsive.
///
/// The daemon returns no metadata and performs no authorization. Calls are
/// subject to a small per-connection rate limit.
///
/// # Safety
///
/// `client` must be a valid client handle.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_ping(client: *mut LuminateClient) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: validated by the public function contract.
        let client = match unsafe { client_ref(client) } {
            Ok(client) => client,
            Err(status) => return status,
        };
        let outcome = match call_client(client, |client| async move { client.ping().await }) {
            Ok(outcome) => outcome,
            Err(status) => return status,
        };
        match outcome {
            Ok(()) => {
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}

/// Permanently removes retained state for a withdrawn device.
///
/// The daemon rejects active devices. This operation changes daemon persistence
/// only and never sends a hardware mutation. The presence check and purge are
/// atomic with respect to topology changes, so an identifier that reappears
/// after enumeration is not purged.
///
/// # Safety
///
/// `client` must be a valid client handle and `device_id` must point to a valid
/// NUL-terminated UTF-8 string.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_purge_withdrawn_device(
    client: *mut LuminateClient,
    device_id: *const c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: validated by the public function contract.
        let client = match unsafe { client_ref(client) } {
            Ok(client) => client,
            Err(status) => return status,
        };
        // SAFETY: validated by the public function contract.
        let device_id = match unsafe { read_required_str(device_id, "device_id") } {
            Ok(device_id) => device_id.to_owned(),
            Err(status) => return status,
        };
        let outcome = match call_client(client, move |client| async move {
            client
                .purge_withdrawn_device(DeviceId::new(device_id))
                .await
        }) {
            Ok(outcome) => outcome,
            Err(status) => return status,
        };
        match outcome {
            Ok(()) => {
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}

/// Asks the daemon to re-enumerate every plugin's hardware and reconcile
/// whatever changed, as it does on resume from suspend.
///
/// Returns once the rescan is scheduled, not once it completes. Watch for
/// topology and state events to learn what actually changed.
///
/// # Safety
///
/// `client` must be a valid client handle.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_rescan(client: *mut LuminateClient) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: validated by the public function contract.
        let client = match unsafe { client_ref(client) } {
            Ok(client) => client,
            Err(status) => return status,
        };
        let outcome = match call_client(client, |client| async move { client.rescan().await }) {
            Ok(outcome) => outcome,
            Err(status) => return status,
        };
        match outcome {
            Ok(()) => {
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}
