// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Reads a built shared library's Mach-O `LC_ID_DYLIB` install name.

#![allow(
    clippy::expect_used,
    reason = "Test-support code used only from integration test binaries, which should fail loudly and immediately on an unmet assumption rather than propagate a Result."
)]

use std::path::Path;
use std::process::Command;

/// Reads the `LC_ID_DYLIB` install name out of a built shared library via
/// `otool -D`, reduced to its bare filename.
///
/// # Panics
///
/// Panics if `otool` can't run, exits unsuccessfully, or its output doesn't
/// contain a parseable install name.
pub(crate) fn read_runtime_name(library: &Path) -> String {
    let otool = Command::new("otool")
        .arg("-D")
        .arg(library)
        .output()
        .expect("run otool -D on built shared library");
    assert!(
        otool.status.success(),
        "otool -D failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&otool.stdout),
        String::from_utf8_lossy(&otool.stderr)
    );
    let stdout = String::from_utf8_lossy(&otool.stdout);
    let install_name = parse_macho_install_name(&stdout)
        .expect("built shared library has an LC_ID_DYLIB install name");
    Path::new(install_name)
        .file_name()
        .expect("install name has a file name component")
        .to_string_lossy()
        .into_owned()
}

/// Parses the install name out of `otool -D` output. The first line echoes
/// the queried path followed by a colon; the second line is the install
/// name itself, which may be a bare filename, an `@rpath`-relative path, or
/// an absolute path depending on how the library was linked.
fn parse_macho_install_name(otool_output: &str) -> Option<&str> {
    otool_output.lines().nth(1).map(str::trim)
}

#[cfg(test)]
#[path = "dylib_tests.rs"]
mod tests;
