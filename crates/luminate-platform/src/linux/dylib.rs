// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Reads a built shared library's ELF `DT_SONAME` entry.

#![allow(
    clippy::expect_used,
    reason = "Test-support code used only from integration test binaries, which should fail loudly and immediately on an unmet assumption rather than propagate a Result."
)]

use std::path::Path;
use std::process::Command;

/// Reads the `DT_SONAME` entry out of a built shared library via `readelf -d`.
///
/// # Panics
///
/// Panics if `readelf` can't run, exits unsuccessfully, or its output
/// doesn't contain a parseable `DT_SONAME` entry.
pub(crate) fn read_runtime_name(library: &Path) -> String {
    let readelf = Command::new("readelf")
        .arg("-d")
        .arg(library)
        .output()
        .expect("run readelf on built shared library");
    assert!(
        readelf.status.success(),
        "readelf -d failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&readelf.stdout),
        String::from_utf8_lossy(&readelf.stderr)
    );
    let stdout = String::from_utf8_lossy(&readelf.stdout);
    parse_elf_soname(&stdout)
        .expect("built shared library has a DT_SONAME entry")
        .to_owned()
}

/// Parses the `DT_SONAME` entry out of `readelf -d` output, e.g.
/// `Library soname: [libfoo.so.4]` → `libfoo.so.4`.
fn parse_elf_soname(readelf_output: &str) -> Option<&str> {
    let (_, rest) = readelf_output
        .lines()
        .find_map(|line| line.split_once("Library soname: ["))?;
    rest.strip_suffix(']')
}

#[cfg(test)]
#[path = "dylib_tests.rs"]
mod tests;
