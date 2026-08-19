// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Owner-private files and directories, plus owner-controlled service
//! directories which other identities may traverse to reach an IPC endpoint.
//!
//! [`ensure_private_directory`], [`create_private_file`],
//! [`open_private_file_for_read`], and
//! [`open_or_create_private_file_for_append`] have the same security posture
//! on every platform. None silently tightens an existing, unexpectedly
//! permissive location; that is treated as a possible misconfiguration or
//! compromise and errors loudly instead.

use std::fs::File;
use std::io;
use std::path::Path;

#[cfg(unix)]
use crate::unix::secure_storage as platform;
#[cfg(windows)]
use crate::windows::secure_storage as platform;

/// Creates `path` as a directory only the current user can access, or
/// verifies an existing one already meets that bar.
///
/// # Errors
///
/// Returns an error if the directory cannot be created, if something other
/// than a directory already exists at `path`, or if an existing directory
/// is accessible to another local identity.
pub fn ensure_private_directory(path: &Path) -> io::Result<()> {
    platform::ensure_private_directory(path)
}

/// Creates `path` as an owner-controlled service directory, or verifies an
/// existing one already meets that bar.
///
/// On Unix, group and other identities may read or traverse the directory but
/// cannot write it. This permits a service group to reach a protected socket.
/// Windows IPC does not use filesystem paths, so its service directories retain
/// the owner-private DACL used by [`ensure_private_directory`].
///
/// This verifies the final directory. Its ancestors remain under the platform
/// administrator's trust boundary and must not be writable by untrusted users.
///
/// # Errors
///
/// Returns an error if the directory cannot be created, is not owned by the
/// current identity, is writable by another identity, or is not a directory.
pub fn ensure_service_directory(path: &Path) -> io::Result<()> {
    platform::ensure_service_directory(path)
}

/// Creates a new file at `path`, readable and writable only by the current
/// user. Fails if anything already exists at `path`, including a symlink.
///
/// # Errors
///
/// Returns an error if the file cannot be created, including because
/// something already exists at `path`.
pub fn create_private_file(path: &Path) -> io::Result<File> {
    platform::create_private_file(path)
}

/// Opens or creates a regular file at `path` for append-only writes, provided
/// it is accessible only to the current user. Refuses to follow a symlink.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or created, if an existing
/// file is accessible to another local identity, or if `path` names a symlink
/// or something other than a regular file.
pub fn open_or_create_private_file_for_append(path: &Path) -> io::Result<File> {
    platform::open_or_create_private_file_for_append(path)
}

/// Opens an existing owner-private regular file for reading.
///
/// The final path is not followed through a symlink or reparse point. The
/// opened object is verified through its handle before it is returned, and a
/// Unix FIFO cannot block the open. Ancestor directories remain under the
/// platform administrator's trust boundary and must not be writable by
/// untrusted users.
///
/// # Errors
///
/// Returns an error if `path` cannot be opened, is not an owner-private regular
/// file, or names a symlink or reparse point.
pub fn open_private_file_for_read(path: &Path) -> io::Result<File> {
    platform::open_private_file_for_read(path)
}

/// Opens an existing administrator-provided file at `path` for reading,
/// refusing to follow a symlink planted at that path.
///
/// This helper deliberately does not require current-user ownership or private
/// permissions. Use [`open_private_file_for_read`] for daemon-owned authority
/// files.
///
/// # Errors
///
/// Returns an error if `path` cannot be opened, including because it is a
/// symlink.
pub fn open_existing_file_without_following_symlinks(path: &Path) -> io::Result<File> {
    platform::open_existing_file_without_following_symlinks(path)
}
