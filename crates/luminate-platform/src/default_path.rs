// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Platform-native fallback paths for an unconfigured Luminate installation.

use std::io;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use crate::macos::default_path as platform;
#[cfg(all(unix, not(target_os = "macos")))]
use crate::unix::default_path as platform;
#[cfg(windows)]
use crate::windows::default_path as platform;

/// Returns the default daemon configuration path.
///
/// # Errors
///
/// Returns an error when the platform cannot resolve its shared application
/// data directory.
pub fn config() -> io::Result<PathBuf> {
    #[cfg(unix)]
    return Ok(platform::config());
    #[cfg(windows)]
    platform::config()
}

/// Returns the default daemon IPC path.
///
/// # Errors
///
/// Returns an error when the platform cannot resolve the path.
pub fn socket() -> io::Result<PathBuf> {
    #[cfg(unix)]
    return Ok(platform::socket());
    #[cfg(windows)]
    Ok(platform::socket())
}

/// Returns the default persisted target-state path.
///
/// # Errors
///
/// Returns an error when the platform cannot resolve its shared application
/// data directory.
pub fn state() -> io::Result<PathBuf> {
    #[cfg(unix)]
    return Ok(platform::state());
    #[cfg(windows)]
    platform::state()
}

/// Returns the default directory for `luminate-http`'s persisted front-end
/// state.
///
/// # Errors
///
/// Returns an error when the platform cannot resolve its shared application
/// data directory.
pub fn http_state_dir() -> io::Result<PathBuf> {
    #[cfg(unix)]
    return Ok(platform::http_state_dir());
    #[cfg(windows)]
    platform::http_state_dir()
}

/// Returns the default daemon log directory.
///
/// # Errors
///
/// Returns an error when the platform cannot resolve its shared application
/// data directory. Logs are currently file-backed only in Windows service
/// mode, so Unix has no corresponding default.
#[cfg(windows)]
pub fn logs() -> io::Result<PathBuf> {
    platform::logs()
}

/// Returns the default locally installed plugin directory.
///
/// # Errors
///
/// Returns an error when the platform cannot resolve its installation
/// directory.
pub fn plugin_local() -> io::Result<PathBuf> {
    #[cfg(unix)]
    return Ok(platform::plugin_local());
    #[cfg(windows)]
    platform::plugin_local()
}

/// Returns the default distribution-installed plugin directory.
///
/// # Errors
///
/// Returns an error when the platform cannot resolve its installation
/// directory.
pub fn plugin_system() -> io::Result<PathBuf> {
    #[cfg(unix)]
    return Ok(platform::plugin_system());
    #[cfg(windows)]
    platform::plugin_system()
}

/// Returns the compiled default daemon configuration path.
///
/// # Errors
///
/// Returns an error when the platform fallback cannot be resolved.
pub fn default_config_path() -> io::Result<PathBuf> {
    option_env!("LUMINATE_DEFAULT_CONFIG_PATH").map_or_else(config, |value| Ok(value.into()))
}

/// Returns the compiled default daemon IPC path.
///
/// # Errors
///
/// Returns an error when the platform fallback cannot be resolved.
pub fn default_socket_path() -> io::Result<PathBuf> {
    option_env!("LUMINATE_DEFAULT_SOCKET_PATH").map_or_else(socket, |value| Ok(value.into()))
}

/// Derives the conventional event IPC path from the primary daemon path.
#[must_use]
pub fn event_socket_path(primary: impl AsRef<Path>) -> PathBuf {
    let mut path = primary.as_ref().as_os_str().to_os_string();
    path.push(".events");
    path.into()
}

/// Returns the compiled default persisted daemon-state path.
///
/// # Errors
///
/// Returns an error when the platform fallback cannot be resolved.
pub fn default_state_path() -> io::Result<PathBuf> {
    option_env!("LUMINATE_DEFAULT_STATE_PATH").map_or_else(state, |value| Ok(value.into()))
}

/// Returns the compiled default HTTP front-end state directory.
///
/// # Errors
///
/// Returns an error when the platform fallback cannot be resolved.
pub fn default_http_state_dir() -> io::Result<PathBuf> {
    option_env!("LUMINATE_DEFAULT_HTTP_STATE_DIR")
        .map_or_else(http_state_dir, |value| Ok(value.into()))
}

/// Returns the compiled default locally installed plugin directory.
///
/// # Errors
///
/// Returns an error when the platform fallback cannot be resolved.
pub fn default_plugin_dir_local() -> io::Result<PathBuf> {
    option_env!("LUMINATE_DEFAULT_PLUGIN_DIR_LOCAL")
        .map_or_else(plugin_local, |value| Ok(value.into()))
}

/// Returns the compiled default distribution plugin directory.
///
/// # Errors
///
/// Returns an error when the platform fallback cannot be resolved.
pub fn default_plugin_dir_system() -> io::Result<PathBuf> {
    option_env!("LUMINATE_DEFAULT_PLUGIN_DIR_SYSTEM")
        .map_or_else(plugin_system, |value| Ok(value.into()))
}

#[cfg(test)]
#[path = "default_path_tests.rs"]
mod tests;
