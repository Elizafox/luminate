// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Unix account identity lookup for daemon trust decisions.

use std::ffi::CString;
use std::io;
use std::mem::MaybeUninit;
use std::ptr;

const MAX_ACCOUNT_LOOKUP_BYTES: usize = 1024 * 1024;

/// Returns the calling process's own real user ID.
///
/// Cheap enough to call directly on every comparison rather than caching,
/// unlike [`crate::windows::identity::daemon_own_sid`]'s token query.
#[must_use]
#[allow(
    unsafe_code,
    reason = "libc::getuid() takes no arguments, touches no memory, and cannot fail; it is the \
              ordinary way to read the calling process's own real user ID, and the standard \
              library exposes no safe wrapper for it."
)]
pub fn daemon_own_uid() -> u32 {
    // SAFETY: getuid() takes no arguments, touches no memory, and cannot fail.
    unsafe { libc::getuid() }
}

/// Resolves a Unix account name to its numeric user ID.
///
/// # Errors
///
/// Returns an error when the name contains a NUL byte, the platform lookup
/// fails, or the required lookup buffer cannot be represented.
#[allow(
    unsafe_code,
    reason = "getpwnam_r is the thread-safe POSIX account lookup API; all pointers refer to live, writable storage for the duration of the call and the returned pointer is used only as a presence signal"
)]
pub fn uid_for_user(name: &str) -> io::Result<Option<u32>> {
    let name = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "account name contains NUL"))?;
    // SAFETY: `sysconf` reads one process-wide configuration value and does
    // not retain pointers or access Rust-owned memory.
    let suggested = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let initial = usize::try_from(suggested)
        .unwrap_or(16 * 1024)
        .clamp(1024, MAX_ACCOUNT_LOOKUP_BYTES);
    let mut buffer = vec![0_u8; initial];

    loop {
        let mut entry = MaybeUninit::<libc::passwd>::uninit();
        let mut result = ptr::null_mut();
        // SAFETY: `name` is NUL-terminated, `entry` and `result` are valid
        // output storage, and `buffer` is writable for its reported length.
        let status = unsafe {
            libc::getpwnam_r(
                name.as_ptr(),
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &raw mut result,
            )
        };
        if status == 0 {
            return if result.is_null() {
                Ok(None)
            } else {
                // SAFETY: a non-null result on success initializes `entry`.
                Ok(Some(unsafe { entry.assume_init() }.pw_uid))
            };
        }
        if status != libc::ERANGE {
            return Err(io::Error::from_raw_os_error(status));
        }
        let next = buffer
            .len()
            .checked_mul(2)
            .filter(|next| *next <= MAX_ACCOUNT_LOOKUP_BYTES)
            .ok_or_else(|| io::Error::other("account lookup buffer is too large"))?;
        buffer.resize(next, 0);
    }
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
