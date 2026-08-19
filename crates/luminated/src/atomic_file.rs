// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Private, crash-safe replacement of daemon-owned files.

use std::fs;
use std::io::{self, Write as _};
use std::path::Path;

use luminate_platform::durability::sync_directory;
use luminate_platform::secure_storage::create_private_file;

/// Writes `contents` to `temporary`, then atomically installs it at `target`.
///
/// Both paths must have the same parent directory. The caller owns directory
/// validation and the temporary-name policy because those differ between the
/// daemon's persisted formats. The temporary file is created with private
/// permissions and exclusive-create semantics.
///
/// # Errors
///
/// Returns an error when the paths are not siblings, or when creating,
/// writing, syncing, renaming, or syncing the parent directory fails.
pub(crate) fn replace(target: &Path, temporary: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "target has no parent"))?;
    if temporary.parent() != Some(parent) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "temporary file must be beside its target",
        ));
    }

    let result = (|| {
        let mut file = create_private_file(temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);

        fs::rename(temporary, target)?;
        sync_directory(parent)
    })();

    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use luminate_platform::test_support::TestDir;

    #[test]
    fn installs_private_contents_and_rejects_a_different_temporary_directory() {
        let directory = TestDir::new("atomic-file");
        let target = directory.path().join("state.json");
        let temporary = directory.path().join(".state.tmp");
        replace(&target, &temporary, b"new state").expect("install private file");
        assert_eq!(
            fs::read(&target).expect("read installed file"),
            b"new state"
        );
        assert!(!temporary.exists());

        let nested = directory.path().join("nested");
        fs::create_dir_all(&nested).expect("create second directory");
        let misplaced = nested.join(".state.tmp");
        let error = replace(&target, &misplaced, b"other state")
            .expect_err("cross-directory temporary must be rejected");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(fs::read(&target).expect("read original file"), b"new state");
    }

    #[test]
    fn removes_the_temporary_file_when_installation_fails() {
        let directory = TestDir::new("atomic-file-failure");
        let target = directory.path().join("occupied");
        fs::create_dir_all(&target).expect("create target directory");
        let temporary = directory.path().join(".occupied.tmp");

        replace(&target, &temporary, b"state").expect_err("directory target cannot be replaced");
        assert!(!temporary.exists());
        assert!(target.is_dir());
    }
}
