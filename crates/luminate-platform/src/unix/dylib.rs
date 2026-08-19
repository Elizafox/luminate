// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Publishes a same-directory symlink under a shared library's runtime-load
//! name, alongside the literal file Cargo produced.

#![allow(
    clippy::expect_used,
    reason = "Test-support code used only from integration test binaries, which should fail loudly and immediately on an unmet assumption rather than propagate a Result."
)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::{process, thread};

/// Ensures `library`'s directory has a symlink named `runtime_name` pointing
/// at `library`'s filename, creating or repointing it if necessary.
///
/// # Panics
///
/// Panics if `library` has no parent directory or file name, or if creating
/// the symlink fails.
pub(crate) fn link_under_runtime_name(library: &Path, runtime_name: &str) {
    let dir = library
        .parent()
        .expect("built library path has a parent directory");
    let file_name = library
        .file_name()
        .expect("built library path has a file name");
    let link = dir.join(runtime_name);
    if link == library {
        return;
    }
    if fs::read_link(&link).is_ok_and(|target| target.as_os_str() == file_name) {
        return;
    }

    // Parallel tests share this directory and may already have consumer
    // binaries running against `link` right now, so it must never observe a
    // missing or half-written state: build the replacement under a
    // process-and-thread-unique name and `rename` it into place, which POSIX
    // guarantees is atomic even when the destination already exists.
    let tmp_link = dir.join(format!(
        ".{runtime_name}.tmp-{}-{:?}",
        process::id(),
        thread::current().id()
    ));
    symlink(file_name, &tmp_link).expect("create temporary runtime-name symlink");
    fs::rename(&tmp_link, &link).expect("publish runtime-name symlink");
}
