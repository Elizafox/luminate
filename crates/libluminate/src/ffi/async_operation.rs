// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Lifetime, cancellation, and durable diagnostics for C asynchronous calls.

use super::{
    AssertUnwindSafe, DiagnosticSnapshot, Error, LuminateStatus, LuminateStringView,
    LuminateTarget, panic, ptr,
};
use std::ffi::CString;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use tokio::task::AbortHandle;

const PENDING: u32 = u32::MAX;
type Delivery = Box<dyn FnOnce(Arc<LuminateAsyncOperation>) + Send>;

/// Result of trying to cancel a local asynchronous operation.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    clippy::enum_variant_names,
    reason = "Fully prefixed Rust variants generate unambiguous C constants."
)]
pub enum LuminateAsyncCancelResult {
    LuminateAsyncCancelAccepted = 0,
    LuminateAsyncCancelAlreadyCancelled = 1,
    LuminateAsyncCancelAlreadyCompleted = 2,
    LuminateAsyncCancelInvalid = u32::MAX,
}

#[allow(
    non_upper_case_globals,
    reason = "Internal aliases keep cancellation readable."
)]
impl LuminateAsyncCancelResult {
    const Accepted: Self = Self::LuminateAsyncCancelAccepted;
    const AlreadyCancelled: Self = Self::LuminateAsyncCancelAlreadyCancelled;
    const AlreadyCompleted: Self = Self::LuminateAsyncCancelAlreadyCompleted;
    const Invalid: Self = Self::LuminateAsyncCancelInvalid;
}

/// Opaque, reference-counted handle for one asynchronous C operation.
pub struct LuminateAsyncOperation {
    terminal_status: AtomicU32,
    terminal: OnceLock<AsyncTerminal>,
    abort_handle: Mutex<Option<AbortHandle>>,
    delivery: Mutex<Option<Delivery>>,
}

struct AsyncTerminal {
    status: LuminateStatus,
    diagnostic: Option<DiagnosticSnapshot>,
}

impl LuminateAsyncOperation {
    pub(crate) fn new(delivery: impl FnOnce(Arc<Self>) + Send + 'static) -> Arc<Self> {
        Arc::new(Self {
            terminal_status: AtomicU32::new(PENDING),
            terminal: OnceLock::new(),
            abort_handle: Mutex::new(None),
            delivery: Mutex::new(Some(Box::new(delivery))),
        })
    }

    pub(crate) fn into_owned_handle(operation: &Arc<Self>) -> *mut Self {
        Arc::into_raw(Arc::clone(operation)).cast_mut()
    }

    #[cfg(test)]
    pub(crate) fn pending() -> *mut Self {
        Arc::into_raw(Self::new(|_| {})).cast_mut()
    }

    #[allow(
        dead_code,
        reason = "Phase 4 defines terminal publication before phase 5 submissions use it."
    )]
    fn publish(&self, status: LuminateStatus, diagnostic: Option<DiagnosticSnapshot>) -> bool {
        if self
            .terminal_status
            .compare_exchange(PENDING, status as u32, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }

        // This operation won the sole transition out of pending, so this set
        // cannot fail. Readers use the OnceLock as the publication boundary,
        // keeping status and its diagnostic snapshot coherent.
        let _ = self.terminal.set(AsyncTerminal { status, diagnostic });
        true
    }

    #[allow(
        dead_code,
        reason = "Phase 4 defines terminal publication before phase 5 submissions use it."
    )]
    pub(crate) fn complete(&self, status: LuminateStatus, error: Option<&Error>) -> bool {
        self.publish(status, error.map(DiagnosticSnapshot::from_error))
    }

    pub(crate) fn finish(self: &Arc<Self>, status: LuminateStatus, error: Option<&Error>) -> bool {
        if !self.publish(status, error.map(DiagnosticSnapshot::from_error)) {
            return false;
        }
        self.deliver();
        true
    }

    pub(crate) fn status(&self) -> Option<LuminateStatus> {
        self.terminal.get().map(|terminal| terminal.status)
    }

    fn deliver(self: &Arc<Self>) {
        let delivery = self
            .delivery
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(delivery) = delivery {
            delivery(Arc::clone(self));
        }
    }

    #[allow(
        dead_code,
        reason = "Phase 4 defines cancellation plumbing before phase 5 submissions attach tasks."
    )]
    pub(crate) fn set_abort_handle(&self, abort_handle: AbortHandle) {
        let mut slot = self
            .abort_handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.terminal_status.load(Ordering::Acquire) == PENDING {
            *slot = Some(abort_handle);
        } else {
            abort_handle.abort();
        }
    }

    fn cancel(&self) -> LuminateAsyncCancelResult {
        match self.terminal_status.compare_exchange(
            PENDING,
            LuminateStatus::Cancelled as u32,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                let _ = self.terminal.set(AsyncTerminal {
                    status: LuminateStatus::Cancelled,
                    diagnostic: None,
                });
                let mut slot = self
                    .abort_handle
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if let Some(handle) = slot.take() {
                    handle.abort();
                }
                drop(slot);
                // SAFETY: cancellation is called through a live Arc-backed
                // external handle, so reconstructing a temporary Arc retains
                // the operation while its delivery is enqueued.
                let operation = unsafe { Arc::from_raw(self) };
                operation.deliver();
                let _ = Arc::into_raw(operation);
                LuminateAsyncCancelResult::Accepted
            }
            Err(status) if status == LuminateStatus::Cancelled as u32 => {
                LuminateAsyncCancelResult::AlreadyCancelled
            }
            Err(_) => LuminateAsyncCancelResult::AlreadyCompleted,
        }
    }

    fn diagnostic(&self) -> Option<&DiagnosticSnapshot> {
        let terminal = self.terminal.get()?;
        if terminal.status == LuminateStatus::Ok || terminal.status == LuminateStatus::Cancelled {
            return None;
        }
        terminal.diagnostic.as_ref()
    }
}

/// Retains an asynchronous operation handle. Null returns null.
///
/// # Safety
///
/// `operation` must be null or a live operation handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_async_operation_retain(
    operation: *const LuminateAsyncOperation,
) -> *mut LuminateAsyncOperation {
    if operation.is_null() {
        return ptr::null_mut();
    }
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the caller guarantees a live Arc-backed operation pointer.
        unsafe { Arc::increment_strong_count(operation) };
    }));
    operation.cast_mut()
}

/// Releases an asynchronous operation reference. Null is a no-op.
///
/// # Safety
///
/// `operation` must be null or one owned reference to a live operation handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_async_operation_release(operation: *mut LuminateAsyncOperation) {
    if operation.is_null() {
        return;
    }
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the caller transfers one valid Arc-backed reference.
        unsafe { Arc::decrement_strong_count(operation) };
    }));
}

/// Requests deterministic local cancellation of an asynchronous operation.
///
/// # Safety
///
/// `operation` must be null or a live operation handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_async_operation_cancel(
    operation: *mut LuminateAsyncOperation,
) -> LuminateAsyncCancelResult {
    let Some(operation) = (unsafe { operation.as_ref() }) else {
        return LuminateAsyncCancelResult::Invalid;
    };
    panic::catch_unwind(AssertUnwindSafe(|| operation.cancel()))
        .unwrap_or(LuminateAsyncCancelResult::Invalid)
}

/// Retrieves the terminal status, leaving `out_status` unchanged while pending.
///
/// # Safety
///
/// Both pointers must be live for the duration of the call when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_async_operation_status(
    operation: *const LuminateAsyncOperation,
    out_status: *mut LuminateStatus,
) -> bool {
    if operation.is_null() || out_status.is_null() {
        return false;
    }
    panic::catch_unwind(AssertUnwindSafe(|| {
        let Some(terminal) = (unsafe { &*operation }).terminal.get() else {
            return false;
        };
        // SAFETY: checked above; the caller guarantees a writable pointer.
        unsafe { *out_status = terminal.status };
        true
    }))
    .unwrap_or(false)
}

/// Returns the durable error message borrowed from a failed operation.
///
/// # Safety
///
/// `operation` must be null or a live operation handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_async_operation_error_message(
    operation: *const LuminateAsyncOperation,
) -> LuminateStringView {
    panic::catch_unwind(AssertUnwindSafe(|| {
        (unsafe { operation.as_ref() })
            .and_then(LuminateAsyncOperation::diagnostic)
            .map_or(
                LuminateStringView {
                    data: ptr::null(),
                    len: 0,
                },
                |diagnostic| LuminateStringView {
                    data: diagnostic.message.as_ptr(),
                    len: diagnostic.message.as_bytes().len(),
                },
            )
    }))
    .unwrap_or(LuminateStringView {
        data: ptr::null(),
        len: 0,
    })
}

fn diagnostic_string(
    operation: *const LuminateAsyncOperation,
    select: impl FnOnce(&DiagnosticSnapshot) -> Option<&CString>,
) -> LuminateStringView {
    // SAFETY: callers ensure a non-null operation is a live operation handle.
    let value = unsafe { operation.as_ref() }
        .and_then(LuminateAsyncOperation::diagnostic)
        .and_then(select);
    value.map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |value| LuminateStringView {
            data: value.as_ptr(),
            len: value.as_bytes().len(),
        },
    )
}

/// Returns the durable safe policy reason from a permission-denied operation.
///
/// # Safety
///
/// `operation` must be null or a live operation handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_async_operation_error_permission_denied_reason(
    operation: *const LuminateAsyncOperation,
) -> LuminateStringView {
    panic::catch_unwind(AssertUnwindSafe(|| {
        diagnostic_string(operation, |value| value.permission_denied_reason.as_ref())
    }))
    .unwrap_or(LuminateStringView {
        data: ptr::null(),
        len: 0,
    })
}

/// Returns the durable daemon version from an incompatible operation.
///
/// # Safety
///
/// `operation` must be null or a live operation handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_async_operation_error_incompatible_daemon_version(
    operation: *const LuminateAsyncOperation,
) -> LuminateStringView {
    panic::catch_unwind(AssertUnwindSafe(|| {
        diagnostic_string(operation, |value| {
            value.incompatible_daemon_version.as_ref()
        })
    }))
    .unwrap_or(LuminateStringView {
        data: ptr::null(),
        len: 0,
    })
}

/// Returns the durable optional reason from an incompatible operation.
///
/// # Safety
///
/// `operation` must be null or a live operation handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_async_operation_error_incompatibility_reason(
    operation: *const LuminateAsyncOperation,
) -> LuminateStringView {
    panic::catch_unwind(AssertUnwindSafe(|| {
        diagnostic_string(operation, |value| value.incompatibility_reason.as_ref())
    }))
    .unwrap_or(LuminateStringView {
        data: ptr::null(),
        len: 0,
    })
}

unsafe fn async_diagnostic_u32(
    operation: *const LuminateAsyncOperation,
    output: *mut u32,
    select: impl FnOnce(&DiagnosticSnapshot) -> Option<u32>,
) -> bool {
    if operation.is_null() || output.is_null() {
        return false;
    }
    // SAFETY: both pointers were checked and the caller guarantees they are live.
    let Some(value) = (unsafe { &*operation }).diagnostic().and_then(select) else {
        return false;
    };
    // SAFETY: the caller guarantees a writable output pointer.
    unsafe { *output = value };
    true
}

/// Retrieves the durable supported primary protocol ABI version.
///
/// Returns `false` without changing the output for other errors or nulls.
///
/// # Safety
///
/// Both pointers must be live for the duration of the call when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_async_operation_error_supported_protocol_abi_version(
    operation: *const LuminateAsyncOperation,
    out_version: *mut u32,
) -> bool {
    panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        async_diagnostic_u32(operation, out_version, |value| {
            value.supported_protocol_abi_version
        })
    }))
    .unwrap_or(false)
}

/// Retrieves the durable supported event protocol version.
///
/// Returns `false` without changing the output for other errors or nulls.
///
/// # Safety
///
/// Both pointers must be live for the duration of the call when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_async_operation_error_supported_event_protocol_version(
    operation: *const LuminateAsyncOperation,
    out_version: *mut u32,
) -> bool {
    panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        async_diagnostic_u32(operation, out_version, |value| {
            value.supported_event_protocol_version
        })
    }))
    .unwrap_or(false)
}

/// Retrieves durable retry guidance from a failed operation.
///
/// # Safety
///
/// Both pointers must be live for the duration of the call when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_async_operation_error_retry_after_ms(
    operation: *const LuminateAsyncOperation,
    out_retry_after_ms: *mut u64,
) -> bool {
    if out_retry_after_ms.is_null() {
        return false;
    }
    panic::catch_unwind(AssertUnwindSafe(|| {
        let Some(value) = (unsafe { operation.as_ref() })
            .and_then(LuminateAsyncOperation::diagnostic)
            .and_then(|diagnostic| diagnostic.retry_after_ms)
        else {
            return false;
        };
        // SAFETY: checked above; the caller guarantees a writable pointer.
        unsafe { *out_retry_after_ms = value };
        true
    }))
    .unwrap_or(false)
}

/// Returns the number of durable applied targets on a failed operation.
///
/// # Safety
///
/// `operation` must be null or a live operation handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_async_operation_error_applied_target_count(
    operation: *const LuminateAsyncOperation,
) -> usize {
    panic::catch_unwind(AssertUnwindSafe(|| {
        (unsafe { operation.as_ref() })
            .and_then(LuminateAsyncOperation::diagnostic)
            .map_or(0, |diagnostic| diagnostic.applied_targets.len())
    }))
    .unwrap_or(0)
}

/// Retrieves one durable applied target borrowed from a failed operation.
///
/// # Safety
///
/// Both pointers must be live for the duration of the call when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_async_operation_error_applied_target_at(
    operation: *const LuminateAsyncOperation,
    index: usize,
    out_target: *mut LuminateTarget,
) -> bool {
    if out_target.is_null() {
        return false;
    }
    panic::catch_unwind(AssertUnwindSafe(|| {
        let Some(target) = (unsafe { operation.as_ref() })
            .and_then(LuminateAsyncOperation::diagnostic)
            .and_then(|diagnostic| diagnostic.applied_targets.get(index))
        else {
            return false;
        };
        let optional_ptr =
            |value: &Option<CString>| value.as_ref().map_or(ptr::null(), |value| value.as_ptr());
        // SAFETY: checked above; the caller guarantees a writable pointer.
        unsafe {
            *out_target = LuminateTarget {
                device_id: target.device.as_ptr(),
                surface_id: optional_ptr(&target.surface),
                element_id: optional_ptr(&target.element),
                group_id: optional_ptr(&target.group),
            }
        };
        true
    }))
    .unwrap_or(false)
}

#[cfg(test)]
#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "These tests exercise sequences of unsafe C boundary calls over handles they own."
)]
mod tests {
    use super::*;
    use crate::TargetId;
    use crate::ffi::{luminate_last_error_message, set_last_error};
    use std::ffi::CStr;
    use std::future::pending;
    use std::slice;
    use std::sync::Barrier;
    use std::thread;

    #[test]
    fn null_and_pending_accessors_are_absent_without_writing_outputs() {
        let operation = LuminateAsyncOperation::pending();
        let mut status = LuminateStatus::Internal;
        let mut retry = 99;
        set_last_error("thread-local sentinel");

        unsafe {
            assert!(luminate_async_operation_retain(ptr::null()).is_null());
            luminate_async_operation_release(ptr::null_mut());
            assert_eq!(
                luminate_async_operation_cancel(ptr::null_mut()),
                LuminateAsyncCancelResult::Invalid
            );
            assert!(!luminate_async_operation_status(operation, &raw mut status));
            assert_eq!(status, LuminateStatus::Internal);
            assert!(!luminate_async_operation_error_retry_after_ms(
                operation,
                &raw mut retry
            ));
            assert_eq!(retry, 99);
            assert_eq!(
                luminate_async_operation_error_permission_denied_reason(operation).len,
                0
            );
            assert_eq!(
                luminate_async_operation_error_incompatible_daemon_version(operation).len,
                0
            );
            let mut version = 77;
            assert!(
                !luminate_async_operation_error_supported_protocol_abi_version(
                    operation,
                    &raw mut version
                )
            );
            assert_eq!(version, 77);
            assert_eq!(
                luminate_async_operation_error_applied_target_count(operation),
                0
            );
            assert_eq!(
                CStr::from_ptr(luminate_last_error_message()).to_bytes(),
                b"thread-local sentinel"
            );
            luminate_async_operation_release(operation);
        };

        let operation = LuminateAsyncOperation::pending();
        let error = Error::PartialMutation {
            message: "later target failed".to_owned(),
            applied_targets: vec![TargetId::element("keyboard0", "keys", "escape")],
        };
        assert!(unsafe { &*operation }.complete(LuminateStatus::PartialMutation, Some(&error)));

        let mut target = LuminateTarget {
            device_id: ptr::null(),
            surface_id: ptr::null(),
            element_id: ptr::null(),
            group_id: ptr::null(),
        };
        unsafe {
            assert_eq!(
                luminate_async_operation_error_applied_target_count(operation),
                1
            );
            assert!(luminate_async_operation_error_applied_target_at(
                operation,
                0,
                &raw mut target
            ));
            assert_eq!(CStr::from_ptr(target.device_id).to_bytes(), b"keyboard0");
            assert_eq!(CStr::from_ptr(target.element_id).to_bytes(), b"escape");
            assert!(!luminate_async_operation_error_applied_target_at(
                operation,
                1,
                &raw mut target
            ));
            luminate_async_operation_release(operation);
        }
    }

    #[test]
    fn cancellation_and_completion_have_one_winner() {
        for _ in 0..100 {
            let operation = LuminateAsyncOperation::pending();
            let retained = unsafe { luminate_async_operation_retain(operation) } as usize;
            let barrier = Arc::new(Barrier::new(3));
            let cancel_barrier = Arc::clone(&barrier);
            let complete_barrier = Arc::clone(&barrier);

            let cancel = thread::spawn(move || {
                cancel_barrier.wait();
                unsafe { luminate_async_operation_cancel(retained as *mut LuminateAsyncOperation) }
            });
            let complete_operation = operation as usize;
            let complete = thread::spawn(move || {
                complete_barrier.wait();
                unsafe { &*(complete_operation as *const LuminateAsyncOperation) }
                    .complete(LuminateStatus::Ok, None)
            });
            barrier.wait();

            let cancel_result = cancel.join().expect("cancel thread should finish");
            let completed = complete.join().expect("completion thread should finish");
            assert_eq!(
                cancel_result == LuminateAsyncCancelResult::Accepted,
                !completed
            );

            let mut status = LuminateStatus::Internal;
            assert!(unsafe { luminate_async_operation_status(operation, &raw mut status) });
            assert_eq!(status == LuminateStatus::Cancelled, !completed);
            unsafe {
                luminate_async_operation_release(retained as *mut LuminateAsyncOperation);
                luminate_async_operation_release(operation);
            }
        }
    }

    #[test]
    fn failed_operation_owns_durable_diagnostic() {
        let operation = LuminateAsyncOperation::pending();
        let error = Error::RateLimited {
            message: "slow down\0please".to_owned(),
            retry_after_ms: Some(125),
        };
        assert!(unsafe { &*operation }.complete(LuminateStatus::RateLimited, Some(&error)));

        unsafe {
            let message = luminate_async_operation_error_message(operation);
            assert_eq!(
                CStr::from_ptr(message.data)
                    .to_str()
                    .expect("message is UTF-8"),
                "rate limited: slow downplease"
            );
            assert_eq!(message.len, "rate limited: slow downplease".len());
            let mut retry = 0;
            assert!(luminate_async_operation_error_retry_after_ms(
                operation,
                &raw mut retry
            ));
            assert_eq!(retry, 125);
            assert_eq!(
                luminate_async_operation_cancel(operation),
                LuminateAsyncCancelResult::AlreadyCompleted
            );
            luminate_async_operation_release(operation);
        }
    }

    #[test]
    fn variant_metadata_is_durable_sanitized_and_error_specific() {
        let denied_operation = LuminateAsyncOperation::pending();
        let denied = Error::PermissionDenied {
            reason: Some("safe\0policy reason".to_owned()),
        };
        assert!(
            unsafe { &*denied_operation }.complete(LuminateStatus::PermissionDenied, Some(&denied))
        );

        set_last_error("unrelated thread-local replacement");
        unsafe {
            let reason = luminate_async_operation_error_permission_denied_reason(denied_operation);
            assert_eq!(
                slice::from_raw_parts(reason.data.cast::<u8>(), reason.len),
                b"safepolicy reason"
            );
            assert_eq!(
                luminate_async_operation_error_incompatibility_reason(denied_operation).len,
                0
            );
            let mut unchanged = 51;
            assert!(
                !luminate_async_operation_error_supported_protocol_abi_version(
                    denied_operation,
                    &raw mut unchanged
                )
            );
            assert_eq!(unchanged, 51);
            luminate_async_operation_release(denied_operation);
        };

        let primary_operation = LuminateAsyncOperation::pending();
        let primary = Error::IncompatibleDaemon {
            daemon_version: "daemon\0version".to_owned(),
            supported_protocol_abi_version: 30,
            reason: Some("primary\0reason".to_owned()),
        };
        assert!(
            unsafe { &*primary_operation }
                .complete(LuminateStatus::IncompatibleDaemon, Some(&primary))
        );
        unsafe {
            let daemon =
                luminate_async_operation_error_incompatible_daemon_version(primary_operation);
            assert_eq!(
                slice::from_raw_parts(daemon.data.cast::<u8>(), daemon.len),
                b"daemonversion"
            );
            let reason = luminate_async_operation_error_incompatibility_reason(primary_operation);
            assert_eq!(
                slice::from_raw_parts(reason.data.cast::<u8>(), reason.len),
                b"primaryreason"
            );
            let mut protocol = 0;
            let mut event = 61;
            assert!(
                luminate_async_operation_error_supported_protocol_abi_version(
                    primary_operation,
                    &raw mut protocol
                )
            );
            assert_eq!(protocol, 30);
            assert!(
                !luminate_async_operation_error_supported_event_protocol_version(
                    primary_operation,
                    &raw mut event
                )
            );
            assert_eq!(event, 61);
            assert!(
                !luminate_async_operation_error_supported_protocol_abi_version(
                    primary_operation,
                    ptr::null_mut()
                )
            );
            luminate_async_operation_release(primary_operation);
        };

        let event_operation = LuminateAsyncOperation::pending();
        let event_error = Error::IncompatibleEventSocket {
            daemon_version: "event-daemon".to_owned(),
            supported_event_protocol_version: 8,
            reason: None,
        };
        assert!(
            unsafe { &*event_operation }
                .complete(LuminateStatus::IncompatibleEventSocket, Some(&event_error))
        );
        unsafe {
            assert_eq!(
                luminate_async_operation_error_incompatibility_reason(event_operation).len,
                0
            );
            let mut event = 0;
            assert!(
                luminate_async_operation_error_supported_event_protocol_version(
                    event_operation,
                    &raw mut event
                )
            );
            assert_eq!(event, 8);
            luminate_async_operation_release(event_operation);
        };
    }

    #[test]
    fn null_arguments_do_not_panic_and_return_falsy_results() {
        let operation = LuminateAsyncOperation::pending();
        let error = Error::PermissionDenied {
            reason: Some("permission denied".to_owned()),
        };
        assert!(unsafe { &*operation }.complete(LuminateStatus::PermissionDenied, Some(&error)));

        let mut status = LuminateStatus::Internal;
        let mut retry = 42_u64;
        let mut target = LuminateTarget {
            device_id: ptr::null(),
            surface_id: ptr::null(),
            element_id: ptr::null(),
            group_id: ptr::null(),
        };

        unsafe {
            assert_eq!(luminate_async_operation_error_message(ptr::null()).len, 0);
            assert_eq!(
                luminate_async_operation_error_permission_denied_reason(ptr::null()).len,
                0
            );
            assert_eq!(
                luminate_async_operation_error_incompatible_daemon_version(ptr::null()).len,
                0
            );
            assert_eq!(
                luminate_async_operation_error_incompatibility_reason(ptr::null()).len,
                0
            );
            let mut version = 91;
            assert!(
                !luminate_async_operation_error_supported_protocol_abi_version(
                    ptr::null(),
                    &raw mut version
                )
            );
            assert_eq!(version, 91);
            assert!(
                !luminate_async_operation_error_supported_event_protocol_version(
                    operation,
                    ptr::null_mut()
                )
            );
            assert!(!luminate_async_operation_error_retry_after_ms(
                ptr::null(),
                &raw mut retry
            ));
            assert_eq!(retry, 42);
            assert_eq!(
                luminate_async_operation_error_applied_target_count(ptr::null()),
                0
            );
            assert!(!luminate_async_operation_error_applied_target_at(
                ptr::null(),
                0,
                &raw mut target
            ));
            assert!(!luminate_async_operation_error_applied_target_at(
                operation,
                0,
                ptr::null_mut()
            ));
            assert!(!luminate_async_operation_status(
                ptr::null(),
                &raw mut status
            ));
            assert!(!luminate_async_operation_status(operation, ptr::null_mut()));
            assert_eq!(status, LuminateStatus::Internal);
            luminate_async_operation_release(operation);
        }
    }

    #[tokio::test]
    async fn accepted_cancellation_aborts_an_attached_task() {
        let operation = LuminateAsyncOperation::pending();
        let task = tokio::spawn(pending::<()>());
        unsafe { &*operation }.set_abort_handle(task.abort_handle());

        assert_eq!(
            unsafe { luminate_async_operation_cancel(operation) },
            LuminateAsyncCancelResult::Accepted
        );
        assert!(
            task.await
                .expect_err("task should be aborted")
                .is_cancelled()
        );
        assert_eq!(
            unsafe { luminate_async_operation_cancel(operation) },
            LuminateAsyncCancelResult::AlreadyCancelled
        );
        assert_eq!(
            unsafe { luminate_async_operation_error_permission_denied_reason(operation) }.len,
            0
        );
        let mut version = 73;
        assert!(!unsafe {
            luminate_async_operation_error_supported_protocol_abi_version(
                operation,
                &raw mut version,
            )
        });
        assert_eq!(version, 73);
        unsafe { luminate_async_operation_release(operation) };
    }
}
