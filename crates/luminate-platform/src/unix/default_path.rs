// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Conventional Unix installation paths.

use std::path::PathBuf;

pub fn config() -> PathBuf {
    PathBuf::from("/etc/luminate/luminated.toml")
}

pub fn socket() -> PathBuf {
    PathBuf::from("/run/luminated.sock")
}

pub fn state() -> PathBuf {
    PathBuf::from("/var/lib/luminated/state.json")
}

pub fn http_state_dir() -> PathBuf {
    PathBuf::from("/var/lib/luminate-http")
}

pub fn plugin_local() -> PathBuf {
    PathBuf::from("/usr/local/lib/luminate/plugins")
}

pub fn plugin_system() -> PathBuf {
    PathBuf::from("/usr/lib/luminate/plugins")
}
