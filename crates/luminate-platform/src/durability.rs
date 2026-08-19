// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Making a filesystem change durable, without a caller needing to know
//! what the local platform requires for that.
//!
//! Writing a file atomically is the same dance everywhere (write a
//! temporary file alongside the target, flush it, rename it over the top),
//! right up to the last step. Whether the *rename itself* survives a crash
//! is where the platforms genuinely diverge, and [`sync_directory`] is that
//! step.

use std::io;
use std::path::Path;

#[cfg(unix)]
use crate::unix::durability as platform;
#[cfg(windows)]
use crate::windows::durability as platform;

/// Flushes directory-entry changes in `path` where the platform exposes a
/// suitable operation.
///
/// Call this after the rename that installs an atomically-written file, not
/// before: flushing the file's own contents (`File::sync_all`) only
/// guarantees the data is on disk, not that the directory entry pointing at
/// it is.
///
/// Unix flushes the directory itself so the rename survives a crash or power
/// loss. Windows has no equivalent operation available through the standard
/// library, so this is a documented no-op there and cannot strengthen the
/// rename's durability guarantee. See the platform implementation for
/// details.
///
/// # Errors
///
/// Returns an error if the directory cannot be opened or flushed.
pub fn sync_directory(path: &Path) -> io::Result<()> {
    platform::sync_directory(path)
}
