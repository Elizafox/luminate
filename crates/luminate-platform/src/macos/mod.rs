// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! macOS-specific implementations of the facts exposed at the crate root.

pub(crate) mod default_path;
pub(crate) mod power;
pub mod service;

// Mirrors `test_support`'s gate in lib.rs: reachable from this crate's own
// unit tests as well as from feature-enabled builds other crates consume.
#[cfg(any(feature = "test-support", test))]
pub(crate) mod dylib;
