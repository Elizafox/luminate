// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Windows implementation of `secure_storage`: directories and files
//! restricted to the current user, since Windows has no POSIX mode-bit
//! equivalent.
//!
//! DACL construction and verification live in
//! [`crate::windows::security_descriptor`], shared with
//! [`crate::windows::transport`]'s named pipes, the same owner-only
//! access-control posture applies to every kind of private object this
//! crate creates on Windows.

use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _};
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateDirectoryW, CreateFileW, FILE_APPEND_DATA,
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_MODE, FILE_SHARE_READ,
    GetFileInformationByHandle, OPEN_ALWAYS, OPEN_EXISTING, READ_CONTROL,
};

use crate::windows::security_descriptor::{
    owner_only_security_attributes, verify_private, verify_private_handle,
};
use crate::windows::wide::to_wide;

/// See [`crate::secure_storage::ensure_private_directory`].
#[allow(
    unsafe_code,
    reason = "CreateDirectoryW is a Win32 API with no safe standard-library wrapper; the wide path \
              and security attributes both outlive the call."
)]
pub fn ensure_private_directory(path: &Path) -> io::Result<()> {
    if !path.exists() {
        let (attributes, _guard) = owner_only_security_attributes()?;
        let wide_path = to_wide(&path.display().to_string());
        // SAFETY: `wide_path` and `attributes` are both valid for the
        // duration of this call; `attributes.lpSecurityDescriptor` is kept
        // alive by `_guard`, which outlives the call.
        let ok = unsafe { CreateDirectoryW(wide_path.as_ptr(), &raw const attributes) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
    }

    if !path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} is not a directory", path.display()),
        ));
    }

    verify_private(path)
}

/// See [`crate::secure_storage::ensure_service_directory`].
pub fn ensure_service_directory(path: &Path) -> io::Result<()> {
    ensure_private_directory(path)
}

/// See [`crate::secure_storage::create_private_file`].
#[allow(
    unsafe_code,
    reason = "CreateFileW is a Win32 API with no safe standard-library wrapper; the wide path and \
              security attributes both outlive the call, and the returned handle is immediately \
              wrapped in an owning `std::fs::File`."
)]
pub fn create_private_file(path: &Path) -> io::Result<File> {
    let (attributes, _guard) = owner_only_security_attributes()?;
    let wide_path = to_wide(&path.display().to_string());

    // SAFETY: `wide_path` and `attributes` are both valid for the duration
    // of this call; `attributes.lpSecurityDescriptor` is kept alive by
    // `_guard`, which outlives the call. `CREATE_NEW` refuses to open an
    // existing file, symlink included, so no separate anti-symlink flag is
    // needed the way `O_NOFOLLOW` is on Unix.
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0 as FILE_SHARE_MODE,
            &raw const attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: `handle` was just returned by a successful `CreateFileW`
    // call and is not used again after being handed to `File`, which now
    // owns it exclusively.
    Ok(unsafe { File::from_raw_handle(handle.cast()) })
}

/// See [`crate::secure_storage::open_or_create_private_file_for_append`].
#[allow(
    unsafe_code,
    reason = "CreateFileW and GetFileInformationByHandle are Win32 APIs with no safe \
              standard-library wrapper; the wide path and security attributes outlive the call, \
              and the handle is closed on every path that does not transfer it to `File`."
)]
pub fn open_or_create_private_file_for_append(path: &Path) -> io::Result<File> {
    let (attributes, _guard) = owner_only_security_attributes()?;
    let wide_path = to_wide(&path.display().to_string());

    // A share mode that omits delete keeps the opened path bound to this
    // handle while its access controls are verified by name below.
    // SAFETY: `wide_path` and `attributes` remain valid for the duration of
    // this call. `OPEN_ALWAYS` applies the owner-only descriptor only when it
    // creates the file; an existing file is verified before the handle is
    // returned.
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            FILE_APPEND_DATA | READ_CONTROL,
            FILE_SHARE_READ,
            &raw const attributes,
            OPEN_ALWAYS,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `handle` was returned by the successful call above and `info`
    // is a local out-parameter this call alone writes to.
    let ok = unsafe { GetFileInformationByHandle(handle, &raw mut info) };
    if ok == 0 {
        let error = io::Error::last_os_error();
        // SAFETY: `handle` is valid and is not used after this close.
        unsafe { CloseHandle(handle) };
        return Err(error);
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        // SAFETY: `handle` is valid and is not used after this close.
        unsafe { CloseHandle(handle) };
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to follow a symlink at {}", path.display()),
        ));
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        // SAFETY: `handle` is valid and is not used after this close.
        unsafe { CloseHandle(handle) };
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    if let Err(error) = verify_private_handle(handle, path) {
        // SAFETY: `handle` is valid and is not used after this close.
        unsafe { CloseHandle(handle) };
        return Err(error);
    }

    // SAFETY: `handle` is valid and is not used again after ownership is
    // transferred to `File`.
    Ok(unsafe { File::from_raw_handle(handle.cast()) })
}

/// See [`crate::secure_storage::open_private_file_for_read`].
pub fn open_private_file_for_read(path: &Path) -> io::Result<File> {
    let file = open_existing_file_without_following_symlinks(path)?;
    if file.metadata()?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    verify_private_handle(file.as_raw_handle().cast(), path)?;
    Ok(file)
}

/// See
/// [`crate::secure_storage::open_existing_file_without_following_symlinks`].
#[allow(
    unsafe_code,
    reason = "CreateFileW and GetFileInformationByHandle are Win32 APIs with no safe \
              standard-library wrapper; the wide path outlives the call, and the handle is closed \
              on every path that doesn't hand it to an owning `std::fs::File`."
)]
pub fn open_existing_file_without_following_symlinks(path: &Path) -> io::Result<File> {
    let wide_path = to_wide(&path.display().to_string());

    // `FILE_FLAG_OPEN_REPARSE_POINT` opens a reparse point (which a symlink
    // is) as itself rather than transparently following it into its target,
    // mirroring `O_NOFOLLOW`'s refusal to follow a symlink on Unix.
    // `FILE_FLAG_BACKUP_SEMANTICS` permits opening a directory long enough to
    // inspect and reject it explicitly instead of surfacing a misleading
    // access-denied error from `CreateFileW`.
    // SAFETY: `wide_path` is valid for the duration of this call; no
    // security attributes or template handle are supplied.
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ,
            ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `handle` was just returned by the successful `CreateFileW`
    // call above; `info` is a local out-parameter this call alone writes
    // to.
    let ok = unsafe { GetFileInformationByHandle(handle, &raw mut info) };
    if ok == 0 {
        let error = io::Error::last_os_error();
        // SAFETY: `handle` is valid and not used again after this close.
        unsafe { CloseHandle(handle) };
        return Err(error);
    }

    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        // SAFETY: `handle` is valid and not used again after this close.
        unsafe { CloseHandle(handle) };
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to follow a symlink at {}", path.display()),
        ));
    }

    // SAFETY: `handle` was just returned by a successful `CreateFileW`
    // call, confirmed above not to be a reparse point, and is not used
    // again after being handed to `File`, which now owns it exclusively.
    Ok(unsafe { File::from_raw_handle(handle.cast()) })
}

#[cfg(test)]
#[path = "secure_storage_tests.rs"]
mod tests;
