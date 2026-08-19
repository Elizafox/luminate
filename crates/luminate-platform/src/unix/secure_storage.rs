// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Unix implementation of `secure_storage`: directories and files
//! restricted to the current user via POSIX mode bits.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;

use crate::identity::daemon_own_uid;

/// Mode applied to a private directory this process creates: owner-only.
const PRIVATE_DIR_MODE: u32 = 0o700;

/// Mode applied to a private file this process creates: owner read/write
/// only.
const PRIVATE_FILE_MODE: u32 = 0o600;

/// See [`crate::secure_storage::ensure_private_directory`].
pub fn ensure_private_directory(path: &Path) -> io::Result<()> {
    ensure_directory(path, true)
}

/// See [`crate::secure_storage::ensure_service_directory`].
pub fn ensure_service_directory(path: &Path) -> io::Result<()> {
    ensure_directory(path, false)
}

fn ensure_directory(path: &Path, owner_private: bool) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_DIR_MODE))?;
        }
        Err(error) => return Err(error),
    }

    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = directory.metadata()?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} is not a directory", path.display()),
        ));
    }
    if metadata.uid() != daemon_own_uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not owned by the current user", path.display()),
        ));
    }
    let forbidden_mode = if owner_private { 0o077 } else { 0o022 };
    if metadata.mode() & forbidden_mode != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} {} (mode {:04o})",
                path.display(),
                if owner_private {
                    "must not be accessible by group or others"
                } else {
                    "must not be writable by group or others"
                },
                metadata.mode() & 0o7777
            ),
        ));
    }

    Ok(())
}

/// See [`crate::secure_storage::open_private_file_for_read`].
pub fn open_private_file_for_read(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    verify_private_regular_file(path, &file)?;
    Ok(file)
}

fn verify_private_regular_file(path: &Path, file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    if metadata.uid() != daemon_own_uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not owned by the current user", path.display()),
        ));
    }
    if metadata.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} must not be accessible by group or others (mode {:04o})",
                path.display(),
                metadata.mode() & 0o7777
            ),
        ));
    }
    Ok(())
}

/// See [`crate::secure_storage::create_private_file`].
pub fn create_private_file(path: &Path) -> io::Result<File> {
    // `create_new` (`O_CREAT|O_EXCL`) proves this call created the file
    // rather than a racing local attacker; `O_NOFOLLOW` refuses to follow a
    // symlink planted at `path` ahead of time.
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(PRIVATE_FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

/// See [`crate::secure_storage::open_or_create_private_file_for_append`].
pub fn open_or_create_private_file_for_append(path: &Path) -> io::Result<File> {
    // `O_NONBLOCK` has no effect on regular files and prevents a planted FIFO
    // from stalling the daemon before the descriptor can be validated.
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(PRIVATE_FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;

    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    if metadata.uid() != daemon_own_uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not owned by the current user", path.display()),
        ));
    }
    if metadata.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} must not be accessible by group or others (mode {:04o})",
                path.display(),
                metadata.mode() & 0o7777
            ),
        ));
    }

    Ok(file)
}

/// See [`crate::secure_storage::open_existing_file_without_following_symlinks`].
pub fn open_existing_file_without_following_symlinks(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(test)]
#[path = "secure_storage_tests.rs"]
mod tests;
