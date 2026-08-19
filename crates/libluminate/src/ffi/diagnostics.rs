// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::{
    AssertUnwindSafe, CString, DiagnosticSnapshot, LAST_ERROR, LuminateStringView, LuminateTarget,
    c_char, panic, ptr, version_cstr,
};

fn string_view(value: Option<&CString>) -> LuminateStringView {
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

fn write_optional_u32(value: Option<u32>, output: *mut u32) -> bool {
    if output.is_null() {
        return false;
    }
    let Some(value) = value else {
        return false;
    };
    // SAFETY: the caller supplied a non-null writable output pointer.
    unsafe { *output = value };
    true
}

/// Returns the libluminate version string.
///
/// The returned pointer is a static NUL-terminated string valid for the
/// lifetime of the process; it must not be freed.
#[unsafe(no_mangle)]
pub extern "C" fn luminate_version() -> *const c_char {
    panic::catch_unwind(version_cstr).map_or(ptr::null(), |cstring| cstring.as_ptr())
}

/// Returns a borrowed pointer to the last error message for the current thread,
/// or null if none has been recorded.
///
/// # Lifetime and invalidation
///
/// The returned pointer **borrows** thread-local storage and stays valid only
/// until the next libluminate call *on this same thread*. Any such call may
/// overwrite or clear the message and free the buffer this pointer refers to.
/// Do not retain it, share it across threads, or use it after another
/// libluminate call; copy the string out first if you need to keep it. For a
/// caller that cannot uphold that, use [`luminate_copy_last_error_message`],
/// which copies into a buffer you own and has no lifetime hazard.
#[unsafe(no_mangle)]
pub extern "C" fn luminate_last_error_message() -> *const c_char {
    panic::catch_unwind(|| {
        LAST_ERROR.with(|slot| {
            slot.borrow()
                .as_ref()
                .map_or(ptr::null(), |diagnostic| diagnostic.message.as_ptr())
        })
    })
    .unwrap_or(ptr::null())
}

/// Retrieves retry guidance for the current thread's last error.
///
/// Returns `true` and writes milliseconds to `out_retry_after_ms` when the
/// provider supplied guidance. Returns `false` for other errors or a null
/// output pointer. Like the message, this metadata is replaced by the next
/// libluminate call on this thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_last_error_retry_after_ms(out_retry_after_ms: *mut u64) -> bool {
    if out_retry_after_ms.is_null() {
        return false;
    }
    panic::catch_unwind(AssertUnwindSafe(|| {
        LAST_ERROR.with(|slot| {
            let Some(value) = slot
                .borrow()
                .as_ref()
                .and_then(|value| value.retry_after_ms)
            else {
                return false;
            };
            // SAFETY: the caller supplied a non-null writable output pointer.
            unsafe { *out_retry_after_ms = value };
            true
        })
    }))
    .unwrap_or(false)
}

/// Returns the number of targets applied before the current thread's last
/// partial mutation error.
#[unsafe(no_mangle)]
pub extern "C" fn luminate_last_error_applied_target_count() -> usize {
    panic::catch_unwind(|| {
        LAST_ERROR.with(|slot| {
            slot.borrow()
                .as_ref()
                .map_or(0, |diagnostic| diagnostic.applied_targets.len())
        })
    })
    .unwrap_or_default()
}

/// Retrieves one borrowed target from the current thread's last partial
/// mutation error.
///
/// Returns `false` for an out-of-range index or null output pointer. Component
/// strings follow the same nullability rules as mutation input targets and
/// remain valid only until the next libluminate call on this thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_last_error_applied_target(
    index: usize,
    out_target: *mut LuminateTarget,
) -> bool {
    if out_target.is_null() {
        return false;
    }
    panic::catch_unwind(AssertUnwindSafe(|| {
        LAST_ERROR.with(|slot| {
            let borrow = slot.borrow();
            let Some(target) = borrow
                .as_ref()
                .and_then(|diagnostic| diagnostic.applied_targets.get(index))
            else {
                return false;
            };
            let optional_ptr = |value: &Option<CString>| {
                value.as_ref().map_or(ptr::null(), |value| value.as_ptr())
            };
            // SAFETY: the caller supplied a non-null writable output pointer.
            unsafe {
                *out_target = LuminateTarget {
                    device_id: target.device.as_ptr(),
                    surface_id: optional_ptr(&target.surface),
                    element_id: optional_ptr(&target.element),
                    group_id: optional_ptr(&target.group),
                }
            };
            true
        })
    }))
    .unwrap_or(false)
}

fn last_error_string(
    select: impl FnOnce(&DiagnosticSnapshot) -> Option<&CString>,
) -> LuminateStringView {
    LAST_ERROR.with(|slot| {
        let borrow = slot.borrow();
        string_view(borrow.as_ref().and_then(select))
    })
}

/// Returns the safe policy reason from the current permission-denied error.
#[unsafe(no_mangle)]
pub extern "C" fn luminate_last_error_permission_denied_reason() -> LuminateStringView {
    panic::catch_unwind(|| last_error_string(|value| value.permission_denied_reason.as_ref()))
        .unwrap_or_else(|_| string_view(None))
}

/// Returns the daemon version from the current incompatibility error.
#[unsafe(no_mangle)]
pub extern "C" fn luminate_last_error_incompatible_daemon_version() -> LuminateStringView {
    panic::catch_unwind(|| last_error_string(|value| value.incompatible_daemon_version.as_ref()))
        .unwrap_or_else(|_| string_view(None))
}

/// Returns the optional reason from the current incompatibility error.
#[unsafe(no_mangle)]
pub extern "C" fn luminate_last_error_incompatibility_reason() -> LuminateStringView {
    panic::catch_unwind(|| last_error_string(|value| value.incompatibility_reason.as_ref()))
        .unwrap_or_else(|_| string_view(None))
}

/// Retrieves the supported primary protocol ABI from an incompatibility error.
///
/// Returns `false` and leaves the output unchanged for other errors or a null
/// output pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_last_error_supported_protocol_abi_version(
    out_version: *mut u32,
) -> bool {
    panic::catch_unwind(AssertUnwindSafe(|| {
        LAST_ERROR.with(|slot| {
            write_optional_u32(
                slot.borrow()
                    .as_ref()
                    .and_then(|value| value.supported_protocol_abi_version),
                out_version,
            )
        })
    }))
    .unwrap_or(false)
}

/// Retrieves the supported event protocol version from an incompatibility error.
///
/// Returns `false` and leaves the output unchanged for other errors or a null
/// output pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_last_error_supported_event_protocol_version(
    out_version: *mut u32,
) -> bool {
    panic::catch_unwind(AssertUnwindSafe(|| {
        LAST_ERROR.with(|slot| {
            write_optional_u32(
                slot.borrow()
                    .as_ref()
                    .and_then(|value| value.supported_event_protocol_version),
                out_version,
            )
        })
    }))
    .unwrap_or(false)
}

/// Copies the current thread's last error message into a caller-owned
/// buffer, always NUL-terminating it, and returns the total number of
/// bytes required (including the terminator).
///
/// This is the lifetime-safe alternative to
/// [`luminate_last_error_message`]. Because the caller owns the
/// destination buffer, no pointer into libluminate-managed storage is
/// exposed. If no error is recorded, the message is empty (a single NUL),
/// so the return value is 1.
///
/// If `buf` is null or `buf_len` is 0, nothing is written and the
/// required length is still returned. This allows callers to query the
/// required buffer size before allocating.
///
/// If the message (including its terminator) does not fit, it is
/// truncated to `buf_len` bytes and still NUL-terminated. A return value
/// greater than `buf_len` indicates that truncation occurred.
///
/// # Safety
///
/// If `buf` is non-null, it must point to at least `buf_len` writable
/// bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_copy_last_error_message(
    buf: *mut c_char,
    buf_len: usize,
) -> usize {
    panic::catch_unwind(AssertUnwindSafe(|| {
        LAST_ERROR.with(|slot| {
            let borrow = slot.borrow();

            // Empty (no error) is represented as a lone terminator.
            let bytes_with_nul: &[u8] = borrow
                .as_ref()
                .map_or(&[0_u8], |diagnostic| diagnostic.message.as_bytes_with_nul());
            let needed = bytes_with_nul.len();

            if !buf.is_null() && buf_len > 0 {
                let copy_len = needed.min(buf_len);

                // SAFETY: `bytes_with_nul` has `needed >= copy_len` bytes and
                // the caller guarantees `buf` has at least `buf_len >= copy_len`
                // writable bytes; the ranges do not overlap (distinct
                // allocations).
                unsafe {
                    ptr::copy_nonoverlapping(
                        bytes_with_nul.as_ptr().cast::<c_char>(),
                        buf,
                        copy_len,
                    );
                };

                // Force a terminator even when the copy was truncated
                // mid-message.
                // SAFETY: `copy_len > 0` because this branch requires
                // `buf_len > 0`, so this points inside the writable buffer.
                let terminator = unsafe { buf.add(copy_len - 1) };

                // SAFETY: `terminator` points inside the writable caller
                // buffer as established above.
                unsafe {
                    *terminator = 0;
                }
            }

            needed
        })
    }))
    .unwrap_or(0)
}
