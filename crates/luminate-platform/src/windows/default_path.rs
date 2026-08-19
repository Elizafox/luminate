// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Windows machine-wide installation paths.

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt as _;
use std::path::PathBuf;
use std::ptr;
use std::slice;

use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::Shell::{
    FOLDERID_LocalAppData, FOLDERID_ProgramData, FOLDERID_ProgramFiles, KF_FLAG_DEFAULT,
    SHGetKnownFolderPath,
};
use windows_sys::core::GUID;

#[allow(
    unsafe_code,
    reason = "SHGetKnownFolderPath and CoTaskMemFree have no safe standard-library wrappers; the \
              known-folder identifier and output pointer remain valid for each call, and the \
              returned allocation is copied before it is freed"
)]
fn known_folder(id: &GUID) -> io::Result<PathBuf> {
    let mut raw = ptr::null_mut();

    // SAFETY: `id` points to a static known-folder identifier, the token is
    // null to select the current process token, and `raw` is a valid output
    // pointer. The returned allocation is released with `CoTaskMemFree`.
    let result = unsafe {
        SHGetKnownFolderPath(
            id,
            KF_FLAG_DEFAULT.cast_unsigned(),
            ptr::null_mut(),
            &raw mut raw,
        )
    };
    if result < 0 {
        return Err(io::Error::other(format!(
            "SHGetKnownFolderPath failed with HRESULT 0x{:08x}",
            result.cast_unsigned()
        )));
    }
    if raw.is_null() {
        return Err(io::Error::other(
            "SHGetKnownFolderPath succeeded without returning a path",
        ));
    }

    let mut length = 0;
    loop {
        // SAFETY: a successful `SHGetKnownFolderPath` call returns a valid,
        // NUL-terminated UTF-16 string; each prior iteration observed a
        // non-NUL code unit, so advancing to the next one remains within it.
        let current = unsafe { raw.add(length) };
        // SAFETY: `current` is a position within that NUL-terminated string.
        if unsafe { *current } == 0 {
            break;
        }
        length += 1;
    }

    // SAFETY: the scan above established that the allocation contains at
    // least `length` initialized UTF-16 code units.
    let path = OsString::from_wide(unsafe { slice::from_raw_parts(raw, length) });

    // SAFETY: `raw` was allocated by `SHGetKnownFolderPath` and has not been
    // freed or transferred.
    unsafe { CoTaskMemFree(raw.cast()) };

    Ok(PathBuf::from(path))
}

pub(crate) fn program_files() -> io::Result<PathBuf> {
    known_folder(&FOLDERID_ProgramFiles)
}

pub(crate) fn local_app_data() -> io::Result<PathBuf> {
    known_folder(&FOLDERID_LocalAppData)
}

/// The machine-wide data root, `%ProgramData%\Luminate`. The installer
/// creates and hardens this directory's own ACL; everything below it is
/// resolved relative to this one known-folder lookup.
pub(crate) fn program_data_root() -> io::Result<PathBuf> {
    Ok(known_folder(&FOLDERID_ProgramData)?.join("Luminate"))
}

pub fn config() -> io::Result<PathBuf> {
    Ok(program_data_root()?.join("luminated.toml"))
}

pub fn socket() -> PathBuf {
    PathBuf::from("luminated.sock")
}

pub fn state() -> io::Result<PathBuf> {
    Ok(program_data_root()?.join(r"state\state.json"))
}

pub fn http_state_dir() -> io::Result<PathBuf> {
    Ok(program_data_root()?.join("http"))
}

pub fn logs() -> io::Result<PathBuf> {
    Ok(program_data_root()?.join("logs"))
}

pub fn plugin_local() -> io::Result<PathBuf> {
    Ok(program_files()?.join(r"Luminate\plugins"))
}

pub fn plugin_system() -> io::Result<PathBuf> {
    plugin_local()
}
