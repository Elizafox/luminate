// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Reading and atomically rewriting `managed.toml`.
//!
//! The file is the daemon's own authority, so an existing file that will not
//! parse is a hard error rather than something to recover from: silently
//! discarding an administrator's configuration would be worse than refusing
//! to start. Writes go through a private temporary file and a directory sync
//! so a crash cannot leave a half-written revision behind.

use std::io::{ErrorKind, Read as _};
use std::path::Path;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use luminate_platform::secure_random::fill_bytes;
use luminate_platform::secure_storage::{ensure_service_directory, open_private_file_for_read};

use crate::atomic_file;

use super::ManagedConfig;

const MAX_MANAGED_CONFIG_BYTES: usize = 1024 * 1024;

/// Loads managed configuration. A missing file means revision zero.
///
/// Unlike cached device state, an invalid existing managed file is a hard
/// startup error because silently ignoring it could reactivate plugins.
pub fn load(path: &Path) -> Result<ManagedConfig> {
    let file = match open_private_file_for_read(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(ManagedConfig::default()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to open managed config {}", path.display()));
        }
    };
    let limit = u64::try_from(MAX_MANAGED_CONFIG_BYTES)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut contents = String::new();
    let read = (&file)
        .take(limit)
        .read_to_string(&mut contents)
        .with_context(|| format!("failed to read managed config {}", path.display()))?;
    anyhow::ensure!(
        read <= MAX_MANAGED_CONFIG_BYTES,
        "managed config {} exceeds {} bytes",
        path.display(),
        MAX_MANAGED_CONFIG_BYTES
    );
    let config: ManagedConfig = toml::from_str(&contents)
        .with_context(|| format!("failed to parse managed config {}", path.display()))?;
    config
        .validate()
        .with_context(|| format!("invalid managed config {}", path.display()))?;
    Ok(config)
}

/// Atomically writes managed configuration through an owner-only temporary
/// file in the same private directory.
pub fn save(path: &Path, config: &ManagedConfig) -> Result<()> {
    config.validate()?;
    let parent = path
        .parent()
        .context("managed config path must have a parent directory")?;
    ensure_service_directory(parent).with_context(|| {
        format!(
            "managed config parent {} is not an owner-controlled service directory",
            parent.display()
        )
    })?;
    let contents = toml::to_string_pretty(config).context("serializing managed configuration")?;
    let temporary = parent.join(format!(
        ".managed-{}-{:016x}.tmp",
        process::id(),
        random_token()
    ));
    atomic_file::replace(path, &temporary, contents.as_bytes()).with_context(|| {
        format!(
            "atomically replacing managed config {} with {}",
            path.display(),
            temporary.display()
        )
    })
}

fn random_token() -> u64 {
    let mut bytes = [0_u8; 8];
    if fill_bytes(&mut bytes).is_ok() {
        return u64::from_ne_bytes(bytes);
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
        })
}
