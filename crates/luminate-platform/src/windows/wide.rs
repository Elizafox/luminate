// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Narrow/wide string marshalling shared by the Windows FFI wrappers.
//!
//! Every wide (`...W`) Win32 call needs a NUL-terminated UTF-16 buffer, and
//! several return one to read back. Keeping the conversions here stops each
//! call site from growing its own subtly different copy.

use std::io;
use std::slice;

/// Encodes `value` as a NUL-terminated UTF-16 buffer for a wide Win32 API.
///
/// Prefer [`to_wide_checked`] for externally supplied names, where an embedded
/// NUL would silently truncate a security-relevant string.
pub(crate) fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

/// Like [`to_wide`], but refuses an embedded NUL rather than letting it
/// silently truncate the string the Win32 API sees. Intended for values that
/// originate outside this crate, such as account and group names, where a
/// truncated string would name a different principal than the caller meant.
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidInput`] if `value` contains a NUL.
pub(crate) fn to_wide_checked(value: &str) -> io::Result<Vec<u16>> {
    if value.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a value passed to a wide Win32 API cannot contain an embedded NUL",
        ));
    }

    Ok(to_wide(value))
}

/// Reads a NUL-terminated wide string starting at `ptr` into an owned
/// `String`, lossily replacing any unpaired surrogates.
///
/// # Safety
///
/// `ptr` must be non-null and point to a NUL-terminated UTF-16 string valid
/// for the duration of this call.
#[allow(
    unsafe_code,
    reason = "Reading a NUL-terminated wide string out of a raw pointer returned by a Win32 API has \
              no safe standard-library equivalent; the caller contract above is the safety \
              invariant, and every read stays within the bounds that the NUL-terminator walk just \
              established."
)]
pub(crate) unsafe fn wide_string_from_nul_terminated(ptr: *const u16) -> String {
    let mut len = 0_usize;
    loop {
        // SAFETY: caller guarantees `ptr` is non-null and NUL-terminated, and
        // `len` stays within the bounds that walk establishes.
        let current = unsafe { ptr.add(len) };
        // SAFETY: `current` was just computed as a position within the
        // NUL-terminated string per the caller's contract.
        if unsafe { *current } == 0 {
            break;
        }
        len += 1;
    }

    // SAFETY: the loop above just walked `ptr..ptr+len` as valid, initialized
    // memory.
    let slice = unsafe { slice::from_raw_parts(ptr, len) };
    String::from_utf16_lossy(slice)
}

#[cfg(test)]
#[path = "wide_tests.rs"]
mod tests;
