// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Small shared helpers for the Windows registry FFI: the RAII key guard,
//! status-code translation, and value-size math used by both the installer
//! metadata store and the Event Log source registration.

use std::io;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{HKEY, RegCloseKey};

/// Owns an open registry key handle and closes it exactly once on drop, so an
/// early return can never leak it.
pub(crate) struct RegistryKey(pub(crate) HKEY);

impl RegistryKey {
    /// Borrows the raw key handle for a Win32 call without surrendering
    /// ownership of it.
    pub(crate) fn raw(&self) -> HKEY {
        self.0
    }
}

impl Drop for RegistryKey {
    #[allow(
        unsafe_code,
        reason = "RegCloseKey has no safe standard-library wrapper"
    )]
    fn drop(&mut self) {
        // SAFETY: this guard owns a key returned by a Reg*KeyExW call and
        // closes it exactly once.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

/// Translates a Win32 registry status code into an `io::Result`, naming the
/// operation that produced a failure.
///
/// # Errors
///
/// Returns an error carrying `status` for any value other than
/// `ERROR_SUCCESS`.
pub(crate) fn check_status(status: u32, operation: &str) -> io::Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::new(
            io::Error::from_raw_os_error(status.cast_signed()).kind(),
            format!("failed to {operation}: Windows error {status}"),
        ))
    }
}

/// Returns the byte length of a wide buffer for the `cbData` argument of a
/// registry write.
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidInput`] if the length does not fit in a
/// `u32`.
pub(crate) fn byte_len(value: &[u16]) -> io::Result<u32> {
    value
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|length| u32::try_from(length).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "registry value is too large"))
}
