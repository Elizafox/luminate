// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Test-only helpers for integration tests that build a native shared
//! library and link a separate consumer against it out-of-process.
//!
//! Gated behind the `test-support` Cargo feature rather than
//! `#[cfg(test)]`: that attribute only reaches this crate's own unit tests,
//! not another crate's integration test binaries, which are separate
//! compilation units that depend on `luminate-platform` like any other
//! crate. Enable the feature from a `[dev-dependencies]` entry to reach it.

#![allow(
    clippy::expect_used,
    reason = "Test-support code used only from test binaries, which should fail loudly and immediately on an unmet assumption rather than propagate a Result."
)]

#[cfg(windows)]
use std::env;
use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::secure_storage::ensure_private_directory;
use crate::transport::max_address_len;

/// How much of the socket-path budget [`unique_runtime_dir`] keeps free for
/// a filename joined onto the directory it returns.
/// `luminated.sock.events` plus a separator needs 22 bytes; the remaining
/// ten bytes keep a longer future name from silently consuming the margin.
const SOCKET_FILE_NAME_HEADROOM: usize = 32;

/// Labels exist so a stray temporary directory can be traced back to the
/// test that left it. Uniqueness comes from the PID and timestamp, so a
/// long label buys nothing and spends a budget that is genuinely tight.
const MAX_LABEL_LEN: usize = 24;

/// The last timestamp used in a temporary path. Some clocks can return the
/// same nanosecond to concurrent callers, so the timestamp alone is not a
/// uniqueness guarantee within one process.
static LAST_PATH_TIMESTAMP: AtomicU64 = AtomicU64::new(0);

/// A short root for temporary sockets. `/tmp` rather than
/// [`std::env::temp_dir`] deliberately: see [`unique_runtime_dir`].
#[cfg(unix)]
fn temp_root() -> PathBuf {
    PathBuf::from("/tmp")
}

/// Windows named pipes live in a kernel namespace rather than the
/// filesystem, so there is no comparable length pressure and the standard
/// temporary directory is fine.
#[cfg(windows)]
fn temp_root() -> PathBuf {
    env::temp_dir()
}

/// Returns a unique, deliberately short path for a test that needs to bind
/// a Unix domain socket, or to place one inside the directory it names.
///
/// Socket paths get far less room than ordinary filesystem paths (see
/// [`crate::transport::max_address_len`]), and [`std::env::temp_dir`] is
/// not a safe base for one: it honours `TMPDIR`, which defaults to a long
/// per-user `/var/folders/<...>/T` on macOS and can be arbitrary anywhere.
/// A path built on it fits on Linux largely by luck and overruns on macOS
/// outright, which is why this builds on a short fixed root instead.
///
/// The returned directory is not created; callers create and clean up
/// whatever they need under it.
///
/// # Panics
///
/// Panics if the resulting path leaves too little room for a socket
/// filename, or if the system clock predates the Unix epoch. Failing here
/// is the point: the alternative is an opaque `EINVAL` from `bind` much
/// further downstream, with nothing naming the real cause.
#[must_use]
pub fn unique_runtime_dir(label: &str) -> PathBuf {
    let label: String = label.chars().take(MAX_LABEL_LEN).collect();
    let observed_nanos: u64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is before the Unix epoch")
        .as_nanos()
        .try_into()
        .expect("system time exceeds the temporary-path timestamp range");
    let nanos = loop {
        let previous = LAST_PATH_TIMESTAMP.load(Ordering::Relaxed);
        let candidate = observed_nanos.max(
            previous
                .checked_add(1)
                .expect("temporary-path timestamp counter overflowed"),
        );
        if LAST_PATH_TIMESTAMP
            .compare_exchange_weak(previous, candidate, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            break candidate;
        }
    };
    let directory = temp_root().join(format!("lum-{label}-{}-{nanos}", process::id()));

    let budget = max_address_len();
    let length = directory.as_os_str().len();
    assert!(
        length + SOCKET_FILE_NAME_HEADROOM <= budget,
        "temporary socket directory {} is {length} bytes, leaving fewer than \
         {SOCKET_FILE_NAME_HEADROOM} bytes for a socket filename within this platform's \
         {budget}-byte limit",
        directory.display()
    );

    directory
}

/// A temporary test directory that removes itself when it goes out of scope.
///
/// Cleanup is best-effort so an earlier test failure is not hidden by a
/// secondary panic while unwinding.
#[derive(Debug)]
pub struct TestDir {
    path: PathBuf,
}

impl TestDir {
    /// Creates a private temporary directory with a short, unique path.
    ///
    /// # Panics
    ///
    /// Panics if the path cannot be created or secured for the current user.
    #[must_use]
    pub fn new(label: &str) -> Self {
        let path = unique_runtime_dir(label);
        ensure_private_directory(&path).expect("create private test directory");
        Self { path }
    }

    /// Reserves a guarded temporary directory path without creating it.
    #[must_use]
    pub fn uncreated(label: &str) -> Self {
        Self {
            path: unique_runtime_dir(label),
        }
    }

    /// Returns the temporary directory's path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Deref for TestDir {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        self.path()
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        self.path()
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Ensures a same-directory link exists under `library`'s platform
/// runtime-load name, alongside the literal file Cargo produced.
///
/// Dynamic loaders resolve a shared library's dependents by a name embedded
/// in the library itself, which does not necessarily match the filename it
/// sits under on disk:
///
/// - ELF (Linux): `DT_SONAME`, read via `readelf -d` (see
///   [`crate::linux::dylib`]).
/// - Mach-O (macOS): the `LC_ID_DYLIB` install name, read via `otool -D`
///   (see [`crate::macos::dylib`]).
/// - PE (Windows): resolved by the literal on-disk filename baked into the
///   importing binary at link time. There is no separate embedded runtime
///   name to reconcile, so this is a no-op there.
///
/// Cargo only ever produces the plain, unversioned build output
/// (`libfoo.so`, `libfoo.dylib`); a build that intentionally embeds a
/// versioned runtime name (for a loader-level ABI mismatch guarantee, say)
/// needs a same-directory link under that versioned name for the loader to
/// find when a consumer links with a plain `-lfoo` plus `-rpath`. Publishing
/// that link is itself platform-generic (see [`crate::unix::dylib`]) once
/// the runtime name is known.
///
/// # Panics
///
/// Panics if the platform's introspection tool can't run, exits
/// unsuccessfully, or its output doesn't contain a parseable runtime name.
#[cfg(target_os = "linux")]
pub fn ensure_dylib_runtime_link(library: &Path) {
    use crate::linux::dylib::read_runtime_name;
    use crate::unix::dylib::link_under_runtime_name;

    let runtime_name = read_runtime_name(library);
    link_under_runtime_name(library, &runtime_name);
}

/// See the `target_os = "linux"` overload's documentation.
///
/// # Panics
///
/// Panics if the platform's introspection tool can't run, exits
/// unsuccessfully, or its output doesn't contain a parseable runtime name.
#[cfg(target_os = "macos")]
pub fn ensure_dylib_runtime_link(library: &Path) {
    use crate::macos::dylib::read_runtime_name;
    use crate::unix::dylib::link_under_runtime_name;

    let runtime_name = read_runtime_name(library);
    link_under_runtime_name(library, &runtime_name);
}

/// See the `target_os = "linux"` overload's documentation. A no-op on every
/// other platform: only Linux and macOS embed a runtime-load name distinct
/// from the on-disk filename.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn ensure_dylib_runtime_link(_library: &Path) {}

#[cfg(test)]
#[path = "test_support_tests.rs"]
mod tests;
