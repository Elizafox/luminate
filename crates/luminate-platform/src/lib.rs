// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Platform-specific services behind portable workspace interfaces.
//!
//! Modules are organized by what varies, not by platform: [`power`],
//! [`process_signals`], [`process_title`], [`identity`], [`default_path`],
//! [`secure_storage`], [`secure_random`], [`durability`], [`terminal`], and [`transport`]
//! are the cross-platform surfaces. Platform modules contain their
//! implementations and narrowly platform-specific APIs.

#![deny(missing_docs)]

use std::env::consts::{DLL_EXTENSION, DLL_PREFIX, DLL_SUFFIX, EXE_SUFFIX};

pub mod default_path;
pub mod durability;
pub mod identity;
pub mod power;
pub mod process_signals;
pub mod process_title;
pub mod secure_random;
pub mod secure_storage;
pub mod terminal;
pub mod transport;

// `test` as well as the feature: this crate's own unit tests are a separate
// compilation unit from the feature-enabled builds other crates consume, and
// they want the same helpers rather than a second private copy.
#[cfg(any(feature = "test-support", test))]
pub mod test_support;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
pub mod windows;

/// Returns the current platform's dynamic library filename extension,
/// without a leading dot: `"so"` on Linux, `"dylib"` on macOS, `"dll"` on
/// Windows.
#[must_use]
#[inline]
pub fn dynamic_library_extension() -> &'static str {
    DLL_EXTENSION
}

/// Returns the candidate dynamic library filenames for a bare component
/// name on the current platform, most conventional first: a
/// prefixed-and-suffixed form (`libfoo.so`, `libfoo.dylib`) followed by a
/// suffixed-only form (`foo.so`, `foo.dll`).
///
/// Callers that discover a plugin or provider by name should check every
/// candidate: Unix-like platforms conventionally prefix shared libraries
/// with `lib`, but Windows does not, and `DLL_PREFIX` is already empty
/// there.
#[must_use]
pub fn dynamic_library_candidates(name: &str) -> [String; 2] {
    [
        format!("{DLL_PREFIX}{name}{DLL_SUFFIX}"),
        format!("{name}{DLL_SUFFIX}"),
    ]
}

/// Returns the conventional executable filename for a bare program name on
/// the current platform: `luminated` on Unix, `luminated.exe` on Windows.
///
/// Callers that build a path to a binary this workspace produces (test
/// harnesses locating `target/debug/luminated`, say) should route the
/// filename through this rather than writing the bare stem, which silently
/// names a file that does not exist on Windows.
#[must_use]
#[inline]
pub fn executable_name(name: &str) -> String {
    format!("{name}{EXE_SUFFIX}")
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
