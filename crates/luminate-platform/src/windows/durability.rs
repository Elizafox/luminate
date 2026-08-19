// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Windows implementation of `durability`.

use std::io;
use std::path::Path;

/// See [`crate::durability::sync_directory`].
///
/// The standard library does not expose a Windows equivalent of opening and
/// flushing a directory. In particular, `File::open` does not pass
/// `FILE_FLAG_BACKUP_SEMANTICS`, which Windows requires when opening a
/// directory handle. This implementation therefore cannot add a portable
/// durability guarantee after the rename and remains a no-op.
///
/// `path` is accepted and ignored so the cross-platform signature stays
/// consistent. Validating it here would invent a failure mode for an
/// operation Windows does not perform.
#[allow(
    clippy::unnecessary_wraps,
    reason = "signature is fixed by the cross-platform `durability::sync_directory` surface this \
              implements; only the Unix branch can actually fail."
)]
pub fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}
