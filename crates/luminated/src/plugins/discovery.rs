// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Plugin candidate discovery and configured-path resolution.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use luminate_core::control::ReconciliationPolicy;

use crate::device_config::{DaemonConfig, ManagedPluginActivation, PluginConfig};

#[derive(Debug, Clone)]
pub(super) struct PluginCandidate {
    pub(super) path: PathBuf,
    pub(super) explicit: bool,
    pub(super) required: bool,
    pub(super) activation: ManagedPluginActivation,
    pub(super) reconciliation: Option<ReconciliationPolicy>,
    pub(super) locked_settings: Vec<String>,
    pub(super) configuration: toml::Table,
}

pub(super) fn discover_candidates(config: &DaemonConfig) -> Result<Vec<PluginCandidate>> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    for plugin in &config.plugins {
        if let Some(path) = resolve_configured_plugin(plugin, &config.plugin_dirs)? {
            let canonical = canonicalize_candidate(&path);
            if seen.insert(canonical.clone()) {
                candidates.push(PluginCandidate {
                    path: canonical,
                    explicit: true,
                    required: plugin.required,
                    activation: plugin.activation,
                    reconciliation: plugin.reconciliation,
                    locked_settings: plugin.locked_settings.clone(),
                    configuration: plugin.config.clone(),
                });
            }
        }
    }

    for dir in &config.plugin_dirs {
        if !dir.exists() {
            continue;
        }

        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(
                    dir = %dir.display(),
                    error = %error,
                    "skipping unreadable plugin directory"
                );
                continue;
            }
        };

        let mut paths = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    tracing::warn!(
                        dir = %dir.display(),
                        error = %error,
                        "failed to read plugin directory entry; skipping rest of directory"
                    );
                    break;
                }
            };
            let path = entry.path();
            if is_shared_object(&path) {
                paths.push(path);
            }
        }

        paths.sort();

        for path in paths {
            let canonical = canonicalize_candidate(&path);
            if seen.insert(canonical.clone()) {
                candidates.push(PluginCandidate {
                    path: canonical,
                    explicit: false,
                    required: false,
                    activation: ManagedPluginActivation::Managed,
                    reconciliation: None,
                    locked_settings: Vec::new(),
                    configuration: toml::Table::new(),
                });
            }
        }
    }

    Ok(candidates)
}

pub(super) fn resolve_configured_plugin(
    plugin: &PluginConfig,
    plugin_dirs: &[PathBuf],
) -> Result<Option<PathBuf>> {
    if let Some(path) = &plugin.path {
        return Ok(Some(path.clone()));
    }

    let Some(name) = &plugin.name else {
        anyhow::bail!("configured plugin entry must specify either name or path");
    };

    let artifact_name = name.replace('-', "_");

    for dir in plugin_dirs {
        for lookup_name in [name.as_str(), artifact_name.as_str()] {
            for candidate_name in luminate_platform::dynamic_library_candidates(lookup_name) {
                let candidate = dir.join(candidate_name);
                if candidate.exists() {
                    return Ok(Some(candidate));
                }
            }
        }
    }

    if plugin.required {
        anyhow::bail!("configured plugin not found by name: {name}");
    }

    Ok(None)
}

fn canonicalize_candidate(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub(super) fn is_shared_object(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == luminate_platform::dynamic_library_extension())
}
