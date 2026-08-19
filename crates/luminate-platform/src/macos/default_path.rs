// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Conventional macOS installation paths for `luminated` as a system-wide
//! `LaunchDaemon`. This matches the multi-user service model used on Linux and
//! Windows; launchd packaging remains responsible for creating these paths.

use std::path::PathBuf;

/// The machine-wide data root, `/Library/Application Support/Luminate`. The
/// macOS analogue of `windows::default_path::program_data_root()`: a system
/// daemon's config, state, and installer-shipped plugins all live under it,
/// separate from `/Library/Preferences`, which is for `.plist`-shaped
/// preferences rather than an arbitrary config file.
pub(crate) fn data_root() -> PathBuf {
    PathBuf::from("/Library/Application Support/Luminate")
}

pub fn config() -> PathBuf {
    data_root().join("luminated.toml")
}

/// A dedicated runtime subdirectory, not `/var/run/luminated.sock` directly:
/// `luminated` runs as the unprivileged `_luminated` account under launchd
/// (`docs/development/architecture/macos-service.md`, "Service account"),
/// and `/var/run` itself is `root:wheel` mode `0755`, so `_luminated` cannot
/// create a socket file there. `service install` creates this directory
/// `0750`, owned by
/// `_luminated:_luminate`, exactly mirroring the Linux packaged
/// `RUNTIME_DIR=/run/luminated` layout.
pub(crate) fn runtime_dir() -> PathBuf {
    PathBuf::from("/var/run/luminated")
}

pub fn socket() -> PathBuf {
    runtime_dir().join("luminated.sock")
}

pub(crate) fn state_dir() -> PathBuf {
    data_root().join("state")
}

pub fn state() -> PathBuf {
    state_dir().join("state.json")
}

pub fn http_state_dir() -> PathBuf {
    data_root().join("http")
}

/// Where `service install` points `StandardOutPath`/`StandardErrorPath`, so
/// the daemon's existing `tracing_subscriber::fmt()` console output lands
/// somewhere durable under launchd instead of `/dev/null`.
pub(crate) fn log_dir() -> PathBuf {
    PathBuf::from("/Library/Logs/luminated")
}

pub fn log_path() -> PathBuf {
    log_dir().join("luminated.log")
}

pub fn plugin_local() -> PathBuf {
    PathBuf::from("/usr/local/lib/luminate/plugins")
}

pub fn plugin_system() -> PathBuf {
    data_root().join("plugins")
}

/// Where `service install` expects `luminated` to already be placed,
/// consistent with [`plugin_local`]'s `/usr/local` prefix rather than
/// inventing a separate one. The macOS analogue of
/// `windows::default_path::program_files()`, but a fixed path: macOS has no
/// known-folder API for it, since there is no per-machine-configurable
/// equivalent of `%ProgramFiles%` to resolve.
pub(crate) fn program_root() -> PathBuf {
    PathBuf::from("/usr/local/bin")
}
