// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Installing, configuring, and uninstalling the `launchd` `LaunchDaemon`
//! registration, including service-account ownership lifecycle. Mirrors
//! `windows::service::install`'s shape at the reduced scope launchd needs;
//! see `docs/development/architecture/macos-service.md` for why there is no
//! SCM-equivalent lifecycle machinery here.

use std::env;
use std::fs;
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};

use crate::macos::default_path;

use super::account::{self, EnsureAccountOutcome};
use super::directories::ensure_directory;
use super::error::ServiceError;
use super::launchctl;
use super::plist::{self, LABEL};

/// Deny-by-default: the machine data root and the log directory it contains
/// are readable only by their owner and group, mirroring
/// `windows::security_descriptor::protect_machine_data_root`'s reasoning for
/// not inheriting a parent directory's looser default.
const DATA_ROOT_MODE: u32 = 0o750;
const LOG_DIR_MODE: u32 = 0o750;
const STATE_DIR_MODE: u32 = 0o700;
/// Matches the Linux packaged `RUNTIME_DIR` mode
/// (`packaging/systemd/luminated.service.in`'s `install -d -m 0750`).
const RUNTIME_DIR_MODE: u32 = 0o750;
const ROOT_UID: u32 = 0;

/// Where `service install` writes the plist.
#[must_use]
pub fn plist_path() -> PathBuf {
    PathBuf::from("/Library/LaunchDaemons").join(format!("{LABEL}.plist"))
}

/// Installs or refreshes the daemon's `LaunchDaemon` registration: the
/// service account, the data/runtime/log directories, and the plist.
///
/// Starts the daemon immediately only if `start` is set; otherwise the plist
/// is in place and `RunAtLoad` takes effect at the next boot on its own. See
/// `docs/development/architecture/macos-service.md`, "Installer mechanism",
/// for why launchd needs no separate "register" step the way SCM does.
///
/// # Errors
///
/// Returns an error if the executable path cannot be resolved, is outside
/// `/usr/local/bin` without the development override, the service account
/// cannot be created or conflicts with a pre-existing, unrelated account, a
/// directory cannot be created or hardened, or `launchctl` rejects loading
/// the plist.
pub fn install(
    allow_development_path: bool,
    start: bool,
) -> Result<EnsureAccountOutcome, ServiceError> {
    let executable_path = env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(ServiceError::ExecutablePath)?;
    validate_install_path(&executable_path, allow_development_path)?;

    let account_outcome = account::ensure_service_account()?;
    let gid = account::group_gid()?;
    let uid = account::user_uid()?;

    ensure_directory(&default_path::data_root(), DATA_ROOT_MODE, ROOT_UID, gid)?;
    ensure_directory(&default_path::log_dir(), LOG_DIR_MODE, uid, gid)?;
    ensure_directory(&default_path::state_dir(), STATE_DIR_MODE, uid, gid)?;
    ensure_directory(&default_path::runtime_dir(), RUNTIME_DIR_MODE, uid, gid)?;

    let log_path = default_path::log_path();
    let rendered = plist::render(&executable_path, &log_path);
    fs::write(plist_path(), rendered).map_err(ServiceError::Plist)?;

    if start {
        launchctl::bootstrap(&plist_path())?;
    }

    Ok(account_outcome)
}

/// Unloads the daemon (if loaded) and removes its plist. `purge_account`
/// additionally removes `_luminated`/`_luminate`, refusing unless their
/// attributes still exactly match what [`install`] set.
///
/// # Errors
///
/// Returns an error if `launchctl` fails to unload a loaded job, the plist
/// cannot be removed, or (when `purge_account` is set) the service account
/// cannot be safely removed.
pub fn uninstall(purge_account: bool) -> Result<(), ServiceError> {
    launchctl::bootout()?;

    let path = plist_path();
    if path.exists() {
        fs::remove_file(&path).map_err(ServiceError::Plist)?;
    }

    if purge_account {
        account::remove_service_account()?;
    }

    Ok(())
}

/// Rejects an executable path outside `/usr/local/bin` unless
/// `allow_development_path` is set. Like the Windows installer, production
/// installation accepts executables only from the platform's trusted program
/// directory.
fn validate_install_path(path: &Path, allow_development_path: bool) -> Result<(), ServiceError> {
    if allow_development_path || path.starts_with(default_path::program_root()) {
        Ok(())
    } else {
        Err(ServiceError::ExecutablePath(IoError::new(
            ErrorKind::InvalidInput,
            format!(
                "{} is not under {}; pass --allow-development-path for a development install",
                path.display(),
                default_path::program_root().display()
            ),
        )))
    }
}

#[cfg(test)]
#[path = "install_tests.rs"]
mod tests;
