// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Generic Unix implementations of the facts exposed at the crate root:
//! code that works identically on Linux, macOS, and BSD because it relies
//! only on POSIX, not on a Linux- or macOS-specific mechanism.

// macOS overrides these with its own system-daemon layout
// (`crate::macos::default_path`); this generic Unix version would otherwise
// go unused there.
#[cfg(not(target_os = "macos"))]
pub(crate) mod default_path;
pub(crate) mod durability;

// Mirrors `test_support`'s gate in lib.rs: reachable from this crate's own
// unit tests as well as from feature-enabled builds other crates consume.
#[cfg(any(feature = "test-support", test))]
pub(crate) mod dylib;

pub(crate) mod identity;
pub(crate) mod process_signals;

#[cfg(any(
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
))]
pub(crate) mod process_title;

pub(crate) mod secure_storage;
pub(crate) mod transport;
