// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Creating and hardening the directories `service install` needs: the
//! machine data root, the runtime socket directory, and the log directory.
//! Mirrors `windows::security_descriptor::protect_machine_data_root`'s
//! "deny-by-default rather than inherit the parent's ACL" posture, using
//! Unix mode bits and ownership instead of an SDDL string.

use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use super::error::ServiceError;

/// Creates `path` if absent, then sets its mode and owning user/group
/// unconditionally; an already-existing directory must not silently keep
/// whatever looser permissions or ownership it already had.
pub(super) fn ensure_directory(
    path: &Path,
    mode: u32,
    uid: u32,
    gid: u32,
) -> Result<(), ServiceError> {
    fs::create_dir_all(path).map_err(ServiceError::MachineDataDirectory)?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(ServiceError::MachineDataDirectory)?;
    chown(path, uid, gid).map_err(ServiceError::MachineDataDirectory)
}

fn chown(path: &Path, uid: u32, gid: u32) -> io::Result<()> {
    let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "path contains an embedded NUL")
    })?;

    #[allow(
        unsafe_code,
        reason = "libc::chown has no safe standard-library equivalent; c_path is a valid \
                  NUL-terminated buffer for the duration of the call."
    )]
    // SAFETY: `c_path` is NUL-terminated and outlives the call; `uid`/`gid`
    // are plain integers with no aliasing concerns.
    let result = unsafe { libc::chown(c_path.as_ptr(), uid, gid) };

    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
#[path = "directories_tests.rs"]
mod tests;
