// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    unsafe_code,
    reason = "The C API boundary necessarily dereferences raw pointers and exports C ABI functions."
)]
#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "The exported C boundary documents pointer requirements on each function; its small unsafe expressions perform those documented checks and conversions."
)]

//! C API for `libluminate`.
//!
//! Variable-sized consumer models cross this boundary as owned opaque roots
//! with borrowed typed accessors. See `ffi_typed` and
//! `docs/development/c-api.md` for ownership, lifetime, and target rules. The
//! native plugin ABI's payload encoding is separate from this consumer C API.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::ffi::{CStr, CString, c_char, c_void};
use std::iter::once;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::ptr;
use std::slice;
use std::sync::{OnceLock, mpsc};
use std::thread;
use std::time::UNIX_EPOCH;

use tokio::runtime::{Builder, Runtime};

use crate::ffi_typed::policy::{
    LuminateResourceConstraintsInput, null_view, read_resource_constraints,
};
use crate::ffi_typed::{LuminateStringView, LuminateTarget, sv};
use crate::{
    Authentication, Client, ClientBuilder, Credential, Error, EventSubscription, ServerInfo,
};
use luminate_core::device::DeviceId;
use luminate_core::policy::{Operation, ResourceConstraints, ScopeGrant, SessionScope};
use luminate_core::target::TargetId;

thread_local! {
    static LAST_ERROR: RefCell<Option<DiagnosticSnapshot>> = const { RefCell::new(None) };
}

/// Owned storage backing synchronous and asynchronous C diagnostic views.
pub(crate) struct DiagnosticSnapshot {
    pub(crate) message: CString,
    pub(crate) retry_after_ms: Option<u64>,
    pub(crate) applied_targets: Vec<AppliedTarget>,
    pub(crate) permission_denied_reason: Option<CString>,
    pub(crate) incompatible_daemon_version: Option<CString>,
    pub(crate) incompatibility_reason: Option<CString>,
    pub(crate) supported_protocol_abi_version: Option<u32>,
    pub(crate) supported_event_protocol_version: Option<u32>,
}

impl DiagnosticSnapshot {
    fn message_only(message: impl Into<String>) -> Self {
        Self {
            message: sanitize_cstring(message.into()),
            retry_after_ms: None,
            applied_targets: Vec::new(),
            permission_denied_reason: None,
            incompatible_daemon_version: None,
            incompatibility_reason: None,
            supported_protocol_abi_version: None,
            supported_event_protocol_version: None,
        }
    }

    pub(crate) fn from_error(error: &Error) -> Self {
        let mut snapshot = Self::message_only(error.message());
        snapshot.retry_after_ms = error.retry_after_ms();
        snapshot.applied_targets = error
            .applied_targets()
            .iter()
            .map(AppliedTarget::from)
            .collect();

        match error {
            Error::PermissionDenied { reason } => {
                snapshot.permission_denied_reason = reason.clone().map(sanitize_cstring);
            }
            Error::IncompatibleDaemon {
                daemon_version,
                supported_protocol_abi_version,
                reason,
            } => {
                snapshot.incompatible_daemon_version =
                    Some(sanitize_cstring(daemon_version.clone()));
                snapshot.incompatibility_reason = reason.clone().map(sanitize_cstring);
                snapshot.supported_protocol_abi_version = Some(*supported_protocol_abi_version);
            }
            Error::IncompatibleEventSocket {
                daemon_version,
                supported_event_protocol_version,
                reason,
            } => {
                snapshot.incompatible_daemon_version =
                    Some(sanitize_cstring(daemon_version.clone()));
                snapshot.incompatibility_reason = reason.clone().map(sanitize_cstring);
                snapshot.supported_event_protocol_version = Some(*supported_event_protocol_version);
            }
            Error::DaemonUnavailable
            | Error::AuthenticationFailed(_)
            | Error::NotFound(_)
            | Error::Unsupported(_)
            | Error::UnknownState(_)
            | Error::InvalidArgument(_)
            | Error::Internal(_)
            | Error::Io(_)
            | Error::Unavailable(_)
            | Error::RateLimited { .. }
            | Error::PartialMutation { .. }
            | Error::Protocol(_)
            | Error::ConnectionPoisoned
            | Error::Timeout(_)
            | Error::Conflict(_)
            | Error::TransitionImpossible(_) => {}
        }

        snapshot
    }
}

/// Owns the strings backing one thread-local borrowed C diagnostic view.
pub(crate) struct AppliedTarget {
    device: CString,
    surface: Option<CString>,
    element: Option<CString>,
    group: Option<CString>,
}

impl From<&TargetId> for AppliedTarget {
    fn from(target: &TargetId) -> Self {
        let (device, surface, element, group) = match target {
            TargetId::Device(device) => (device.as_str(), None, None, None),
            TargetId::Surface { device, surface } => {
                (device.as_str(), Some(surface.as_str()), None, None)
            }
            TargetId::Element {
                device,
                surface,
                element,
            } => (
                device.as_str(),
                Some(surface.as_str()),
                Some(element.as_str()),
                None,
            ),
            TargetId::Group { device, group } => {
                (device.as_str(), None, None, Some(group.as_str()))
            }
        };
        Self {
            device: sanitize_cstring(device.to_owned()),
            surface: surface.map(|value| sanitize_cstring(value.to_owned())),
            element: element.map(|value| sanitize_cstring(value.to_owned())),
            group: group.map(|value| sanitize_cstring(value.to_owned())),
        }
    }
}

mod worker;
pub(crate) use worker::*;

/// Runs a status-returning FFI body behind a panic barrier.
///
/// This enforces the FFI contract that panics never unwind across an
/// `extern "C"` boundary. If the body panics, the panic is caught, its
/// message is recorded, and `Internal` is returned instead.
pub(crate) fn ffi_guard<F>(f: F) -> LuminateStatus
where
    F: FnOnce() -> LuminateStatus,
{
    panic::catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        set_last_error("libluminate caught an internal panic");
        LuminateStatus::Internal
    })
}

/// Opaque libluminate client handle.
///
/// Operations on one `LuminateClient` may be called concurrently. The caller
/// must still ensure `luminate_client_free` does not overlap another use of
/// that exact handle.
pub struct LuminateClient; // NOTE: Deliberately not #[repr(C)] as it is opaque

/// Opaque, deep-copying configuration for one client connection.
pub struct LuminateClientBuilder {
    path: Option<PathBuf>,
    authentication: Authentication,
    scope: Option<SessionScope>,
}

/// Opaque, deep-copying builder for an allow-only session scope.
pub struct LuminateSessionScopeBuilder(Vec<ScopeGrant>);

/// Owned snapshot of sanitized authenticated-session metadata.
pub struct LuminateSessionMetadata {
    authority: CString,
    subject: CString,
    verified_groups: Vec<CString>,
    source: LuminateAuthenticationSource,
    source_name: Option<CString>,
    credential_id: Option<CString>,
    expires_at_unix_ms: Option<u64>,
}

/// Sanitized authentication source for a connected client.
#[repr(u32)]
#[derive(Clone, Copy)]
#[allow(
    clippy::enum_variant_names,
    reason = "Fully prefixed Rust variants generate collision-free public C constants."
)]
pub enum LuminateAuthenticationSource {
    LuminateAuthenticationSourcePeer = 0,
    LuminateAuthenticationSourceBearer = 1,
    LuminateAuthenticationSourceAttestation = 2,
    LuminateAuthenticationSourceExternal = 3,
    LuminateAuthenticationSourceUnknown = u32::MAX,
}

#[allow(
    non_upper_case_globals,
    reason = "Internal aliases keep exhaustive source mapping readable while C receives prefixed names."
)]
impl LuminateAuthenticationSource {
    const Peer: Self = Self::LuminateAuthenticationSourcePeer;
    const Bearer: Self = Self::LuminateAuthenticationSourceBearer;
    const Attestation: Self = Self::LuminateAuthenticationSourceAttestation;
    const External: Self = Self::LuminateAuthenticationSourceExternal;
    const Unknown: Self = Self::LuminateAuthenticationSourceUnknown;
}

/// Opaque event subscription handle. Calls on one handle must not overlap.
pub struct LuminateEventSubscription;

/// Status returned by C API operations.
///
/// On `Ok`, all documented output pointers have been initialized. On any
/// other status, retrieve the calling thread's diagnostic before issuing
/// another libluminate call on that thread, as it may overwrite the
/// previous one.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    clippy::enum_variant_names,
    reason = "Fully prefixed Rust variants generate unambiguous C constants."
)]
pub enum LuminateStatus {
    LuminateStatusOk = 0,
    LuminateStatusNullPointer = 1,
    LuminateStatusInvalidUtf8 = 2,
    LuminateStatusDaemonUnavailable = 3,
    LuminateStatusIncompatibleDaemon = 4,
    LuminateStatusPermissionDenied = 5,
    LuminateStatusNotFound = 6,
    LuminateStatusUnsupported = 7,
    LuminateStatusInvalidArgument = 8,
    LuminateStatusInternal = 9,
    LuminateStatusIo = 10,
    LuminateStatusProtocol = 11,
    LuminateStatusConnectionPoisoned = 12,
    LuminateStatusUnknownState = 13,
    LuminateStatusTimeout = 14,
    LuminateStatusIncompatibleEventSocket = 15,
    LuminateStatusUnavailable = 16,
    LuminateStatusRateLimited = 17,
    LuminateStatusPartialMutation = 18,
    LuminateStatusConflict = 19,
    LuminateStatusTransitionImpossible = 20,
    LuminateStatusAuthenticationFailed = 21,
    LuminateStatusCancelled = 22,
}

#[allow(
    non_upper_case_globals,
    reason = "Internal aliases keep the status handling readable while C receives prefixed names."
)]
/// cbindgen:ignore
impl LuminateStatus {
    pub(crate) const Ok: Self = Self::LuminateStatusOk;
    pub(crate) const NullPointer: Self = Self::LuminateStatusNullPointer;
    pub(crate) const InvalidUtf8: Self = Self::LuminateStatusInvalidUtf8;
    pub(crate) const DaemonUnavailable: Self = Self::LuminateStatusDaemonUnavailable;
    pub(crate) const IncompatibleDaemon: Self = Self::LuminateStatusIncompatibleDaemon;
    pub(crate) const PermissionDenied: Self = Self::LuminateStatusPermissionDenied;
    pub(crate) const NotFound: Self = Self::LuminateStatusNotFound;
    pub(crate) const Unsupported: Self = Self::LuminateStatusUnsupported;
    pub(crate) const InvalidArgument: Self = Self::LuminateStatusInvalidArgument;
    pub(crate) const Internal: Self = Self::LuminateStatusInternal;
    pub(crate) const Io: Self = Self::LuminateStatusIo;
    pub(crate) const Protocol: Self = Self::LuminateStatusProtocol;
    pub(crate) const ConnectionPoisoned: Self = Self::LuminateStatusConnectionPoisoned;
    pub(crate) const UnknownState: Self = Self::LuminateStatusUnknownState;
    pub(crate) const Timeout: Self = Self::LuminateStatusTimeout;
    pub(crate) const IncompatibleEventSocket: Self = Self::LuminateStatusIncompatibleEventSocket;
    pub(crate) const Unavailable: Self = Self::LuminateStatusUnavailable;
    pub(crate) const RateLimited: Self = Self::LuminateStatusRateLimited;
    pub(crate) const PartialMutation: Self = Self::LuminateStatusPartialMutation;
    pub(crate) const Conflict: Self = Self::LuminateStatusConflict;
    pub(crate) const TransitionImpossible: Self = Self::LuminateStatusTransitionImpossible;
    pub(crate) const AuthenticationFailed: Self = Self::LuminateStatusAuthenticationFailed;
    pub(crate) const Cancelled: Self = Self::LuminateStatusCancelled;
}

/// Owned server metadata returned by `luminate_client_server_info`.
pub struct LuminateServerInfo(ServerInfo);

pub(crate) fn set_last_error(message: impl Into<String>) {
    LAST_ERROR.with(|slot| {
        *slot.borrow_mut() = Some(DiagnosticSnapshot::message_only(message));
    });
}

pub(crate) fn clear_last_error() {
    LAST_ERROR.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

pub(crate) fn sanitize_cstring(message: String) -> CString {
    let bytes: Vec<u8> = message
        .into_bytes()
        .into_iter()
        .filter(|byte| *byte != 0)
        .collect();
    let mut bytes = bytes;
    bytes.push(0);
    // SAFETY: interior NULs were filtered above and we explicitly append one
    // trailing terminator.
    unsafe { CString::from_vec_with_nul_unchecked(bytes) }
}

/// Maps an [`Error`] to its corresponding C status code.
///
/// Only the status code is produced here. The associated human-readable
/// diagnostic is stored separately by [`store_error`] in the thread-local
/// last-error slot, retrievable via
/// `luminate_last_error_message` or
/// `luminate_copy_last_error_message`.
///
/// Error paths should normally call [`store_error`] so callers receive
/// both the status code and its accompanying diagnostic.
fn status_from_error(error: &Error) -> LuminateStatus {
    match error {
        Error::DaemonUnavailable => LuminateStatus::DaemonUnavailable,
        Error::AuthenticationFailed(_) => LuminateStatus::AuthenticationFailed,
        Error::IncompatibleDaemon { .. } => LuminateStatus::IncompatibleDaemon,
        Error::IncompatibleEventSocket { .. } => LuminateStatus::IncompatibleEventSocket,
        Error::PermissionDenied { .. } => LuminateStatus::PermissionDenied,
        Error::NotFound(_) => LuminateStatus::NotFound,
        Error::Unsupported(_) => LuminateStatus::Unsupported,
        Error::UnknownState(_) => LuminateStatus::UnknownState,
        Error::InvalidArgument(_) => LuminateStatus::InvalidArgument,
        Error::Internal(_) => LuminateStatus::Internal,
        Error::Io(_) => LuminateStatus::Io,
        Error::Unavailable(_) => LuminateStatus::Unavailable,
        Error::RateLimited { .. } => LuminateStatus::RateLimited,
        Error::PartialMutation { .. } => LuminateStatus::PartialMutation,
        Error::Protocol(_) => LuminateStatus::Protocol,
        Error::ConnectionPoisoned => LuminateStatus::ConnectionPoisoned,
        Error::Timeout(_) => LuminateStatus::Timeout,
        Error::Conflict(_) => LuminateStatus::Conflict,
        Error::TransitionImpossible(_) => LuminateStatus::TransitionImpossible,
    }
}

pub(crate) fn create_runtime() -> Result<Runtime, Error> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::Internal(error.to_string()))
}

pub(crate) fn store_error(error: &Error) -> LuminateStatus {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(DiagnosticSnapshot::from_error(error)));
    status_from_error(error)
}

fn version_cstr() -> &'static CString {
    static VERSION_CSTR: OnceLock<CString> = OnceLock::new();
    VERSION_CSTR.get_or_init(|| sanitize_cstring(crate::version().to_owned()))
}

unsafe fn write_client(out_client: *mut *mut LuminateClient, client: FfiClient) {
    // SAFETY: caller guarantees `out_client` is a valid non-null out-pointer.
    unsafe {
        *out_client = Box::into_raw(Box::new(client)).cast();
    }
}

pub(crate) unsafe fn write_subscription(
    out_subscription: *mut *mut LuminateEventSubscription,
    subscription: FfiSubscription,
) {
    // SAFETY: caller guarantees a valid non-null out-pointer.
    unsafe {
        *out_subscription = Box::into_raw(Box::new(subscription)).cast();
    }
}

pub(crate) unsafe fn read_path<'a>(path: *const c_char) -> Result<&'a str, LuminateStatus> {
    // SAFETY: caller passes a valid, NUL-terminated string pointer or null.
    let cstr = unsafe { CStr::from_ptr(path) };
    cstr.to_str().map_err(|_| {
        set_last_error("path is not valid UTF-8");
        LuminateStatus::InvalidUtf8
    })
}

pub(crate) unsafe fn client_ref<'a>(
    client: *mut LuminateClient,
) -> Result<&'a FfiClient, LuminateStatus> {
    if client.is_null() {
        set_last_error("client handle is null");
        return Err(LuminateStatus::NullPointer);
    }

    // SAFETY: null checked above; caller owns a valid client handle and keeps
    // it alive for this call. The handle's shared core is thread-safe.
    Ok(unsafe { &*client.cast::<FfiClient>() })
}

pub(crate) unsafe fn subscription_ref<'a>(
    subscription: *mut LuminateEventSubscription,
) -> Result<&'a FfiSubscription, LuminateStatus> {
    if subscription.is_null() {
        set_last_error("event subscription handle is null");
        return Err(LuminateStatus::NullPointer);
    }
    // SAFETY: null checked above; caller owns a valid subscription handle.
    Ok(unsafe { &*subscription.cast::<FfiSubscription>() })
}

pub(crate) unsafe fn out_ptr_mut<'a, T>(
    out: *mut T,
    what: &'static str,
) -> Result<&'a mut T, LuminateStatus> {
    if out.is_null() {
        set_last_error(format!("{what} output pointer is null"));
        return Err(LuminateStatus::NullPointer);
    }

    // SAFETY: null checked above; caller provided a writable out-pointer.
    Ok(unsafe { &mut *out })
}

pub(crate) fn server_info_to_ffi(info: ServerInfo) -> LuminateServerInfo {
    LuminateServerInfo(info)
}

pub(crate) unsafe fn read_required_str<'a>(
    value: *const c_char,
    name: &'static str,
) -> Result<&'a str, LuminateStatus> {
    if value.is_null() {
        set_last_error(format!("{name} pointer is null"));
        return Err(LuminateStatus::NullPointer);
    }

    // SAFETY: checked for null above and documented as NUL-terminated.
    let cstr = unsafe { CStr::from_ptr(value) };
    cstr.to_str().map_err(|_| {
        set_last_error(format!("{name} is not valid UTF-8"));
        LuminateStatus::InvalidUtf8
    })
}

pub(crate) mod diagnostics;
#[cfg(test)]
use diagnostics::*;

mod async_calls;
mod async_common;
mod async_connect;
mod async_events;
mod async_operation;

/// Connects to the default daemon socket.
///
/// # Safety
///
/// `out_client` must be a valid non-null out-pointer. The returned handle must
/// later be released with `luminate_client_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_connect(
    out_client: *mut *mut LuminateClient,
) -> LuminateStatus {
    ffi_guard(|| {
        if out_client.is_null() {
            set_last_error("client output pointer is null");
            return LuminateStatus::NullPointer;
        }

        match spawn_ffi_client(|runtime| runtime.block_on(Client::connect())) {
            Ok(client) => {
                clear_last_error();
                // SAFETY: `out_client` was checked for null above.
                unsafe { write_client(out_client, client) };
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}

mod builder;
#[cfg(test)]
use builder::*;

/// Connects to a daemon socket at a specific path.
///
/// # Safety
///
/// `path` must point to a valid NUL-terminated UTF-8 string. `out_client` must
/// be a valid non-null out-pointer. The returned handle must later be released
/// with `luminate_client_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_connect_path(
    path: *const c_char,
    out_client: *mut *mut LuminateClient,
) -> LuminateStatus {
    ffi_guard(|| {
        if out_client.is_null() {
            set_last_error("client output pointer is null");
            return LuminateStatus::NullPointer;
        }

        if path.is_null() {
            set_last_error("path pointer is null");
            return LuminateStatus::NullPointer;
        }

        // SAFETY: `path` was checked for null above and is documented as NUL-terminated.
        let path = match unsafe { read_path(path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        // Own the path before handing it to the worker thread, which outlives
        // this call's borrow of the caller's buffer.
        let path = path.to_owned();

        match spawn_ffi_client(move |runtime| runtime.block_on(Client::connect_path(path))) {
            Ok(client) => {
                clear_last_error();
                // SAFETY: `out_client` was checked for null above.
                unsafe { write_client(out_client, client) };
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}

/// Releases a client handle returned by `luminate_client_connect*`.
///
/// # Safety
///
/// `client` must either be null or a valid handle previously returned by
/// `luminate_client_connect` or `luminate_client_connect_path`. It must not be
/// freed more than once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_free(client: *mut LuminateClient) {
    if client.is_null() {
        return;
    }

    // SAFETY: caller guarantees this is a valid, uniquely owned client pointer.
    let boxed = unsafe { Box::from_raw(client.cast::<FfiClient>()) };
    // Dropping the last retained core joins its runtime thread; contain any
    // panic so it cannot unwind across the `extern "C"` boundary.
    let _ = panic::catch_unwind(AssertUnwindSafe(move || drop(boxed)));
}

mod events;
#[cfg(test)]
use events::*;

mod operations;
#[cfg(test)]
use operations::*;

/// Releases string memory allocated by libluminate.
///
/// # Safety
///
/// `value` must either be null or a pointer returned by libluminate that was
/// documented as requiring `luminate_string_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_string_free(value: *mut c_char) {
    if value.is_null() {
        return;
    }

    // SAFETY: caller guarantees this pointer was allocated by CString::into_raw.
    let owned = unsafe { CString::from_raw(value) };
    // Contain any panic so it cannot unwind across the `extern "C"` boundary.
    let _ = panic::catch_unwind(AssertUnwindSafe(move || drop(owned)));
}

/// Releases an owned `LuminateServerInfo`. Null is a no-op.
///
/// # Safety
///
/// `info` must be null or a pointer returned by `luminate_client_server_info`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_server_info_free(info: *mut LuminateServerInfo) {
    if info.is_null() {
        return;
    }
    // SAFETY: ownership contract documented on this function.
    drop(unsafe { Box::from_raw(info) });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_server_info_daemon_name(
    info: *const LuminateServerInfo,
) -> LuminateStringView {
    unsafe { info.as_ref() }.map_or_else(null_view, |info| sv(&info.0.daemon_name))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_server_info_daemon_version(
    info: *const LuminateServerInfo,
) -> LuminateStringView {
    unsafe { info.as_ref() }.map_or_else(null_view, |info| sv(&info.0.daemon_version))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_server_info_protocol_abi_version(
    info: *const LuminateServerInfo,
) -> u32 {
    unsafe { info.as_ref() }.map_or(0, |info| info.0.protocol_abi_version)
}

#[cfg(test)]
#[path = "../ffi_tests.rs"]
mod tests;
