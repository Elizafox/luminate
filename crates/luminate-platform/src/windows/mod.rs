// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Windows-specific implementations of the facts exposed at the crate root.

pub(crate) mod default_path;
pub(crate) mod durability;
pub mod event_log;
pub mod identity;
pub(crate) mod installer_metadata;
pub mod local_group;
#[cfg(test)]
#[path = "local_test_account_tests.rs"]
mod local_test_account;
pub(crate) mod power;
pub(crate) mod process_signals;
pub(crate) mod registry;
pub(crate) mod secure_storage;
pub mod security_descriptor;
pub mod service;
pub(crate) mod transport;
pub(crate) mod wide;

use std::io;
use std::path::PathBuf;

/// Returns the current user's local, non-roaming application-data directory.
///
/// This is the location conventionally exposed as `%LOCALAPPDATA%`, resolved
/// through the Windows known-folder API rather than a mutable environment
/// variable.
///
/// # Errors
///
/// Returns an error when Windows cannot resolve the current user's local
/// application-data known folder.
pub fn current_user_local_data_directory() -> io::Result<PathBuf> {
    default_path::local_app_data()
}
