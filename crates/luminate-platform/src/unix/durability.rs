// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Unix implementation of `durability`: `fsync` on a directory file
//! descriptor, the portable POSIX way to persist a directory entry.

use std::fs::File;
use std::io;
use std::path::Path;

/// See [`crate::durability::sync_directory`].
pub fn sync_directory(path: &Path) -> io::Result<()> {
    // Opening a directory read-only and `fsync`ing the descriptor is
    // well-defined on POSIX and is the standard idiom for persisting a
    // rename.
    File::open(path)?.sync_all()
}
