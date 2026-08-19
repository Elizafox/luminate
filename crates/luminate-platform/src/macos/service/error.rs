// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Error type returned by macOS launchd service install, uninstall, and
//! lifecycle operations.

use std::io;

use thiserror::Error;

/// Error returned while installing, uninstalling, or controlling the
/// `launchd` `LaunchDaemon` registration.
#[derive(Debug, Error)]
pub enum ServiceError {
    /// Resolving or validating the service executable path failed.
    #[error("resolve executable path: {0}")]
    ExecutablePath(#[source] io::Error),

    /// Creating or hardening the machine-wide data root, runtime directory,
    /// or log directory failed.
    #[error("prepare machine data directory: {0}")]
    MachineDataDirectory(#[source] io::Error),

    /// Running or parsing the output of an external tool (`dscl`,
    /// `launchctl`) failed.
    #[error("run external command: {0}")]
    Command(#[source] io::Error),

    /// An external tool ran but reported failure.
    #[error("{program} {args} failed: {stderr}", args = args.join(" "))]
    CommandFailed {
        /// The program that failed, e.g. `"dscl"`.
        program: &'static str,
        /// The arguments passed to it.
        args: Vec<String>,
        /// Its captured standard error, trimmed.
        stderr: String,
    },

    /// No unused id remained in the reserved system account range.
    #[error("no unused system {kind} available")]
    IdRangeExhausted {
        /// `"UID"` or `"GID"`, for the error message.
        kind: &'static str,
    },

    /// The `_luminated` user or `_luminate` group already exists with
    /// attributes this installer did not set, so it is not safe to treat as
    /// Luminate's own.
    #[error("account conflict: {0}")]
    AccountConflict(String),

    /// Explicit account removal was refused because its safety
    /// preconditions were not met.
    #[error("refusing to remove account: {0}")]
    UnsafeAccountPurge(String),

    /// Writing the launchd property list failed.
    #[error("write launchd property list: {0}")]
    Plist(#[source] io::Error),
}
