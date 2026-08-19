// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Startup plugin loading and the isolation policy applied to each candidate.
//!
//! This is the one-shot path that turns a discovered catalogue into a
//! populated [`PluginManager`]. Its defining concern is deciding what a
//! failure means: a required plugin that fails aborts daemon startup, while
//! an optional one is logged, marked failed in the catalogue, and skipped so
//! the rest of the hardware still comes up. Runtime (re)loading of an
//! individual plugin lives in `managed` and `topology` instead.

use std::collections::HashMap;
use std::iter;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{Context as _, Result};

use luminate_core::capability::{ReadbackFidelity, StateReadbackCapability};
use luminate_core::control;
use luminate_plugin_api::DeviceDescriptor;

use super::catalogue::{PluginRuntimeState, build_catalogue};
use super::id::LoadedPluginId;
use super::ownership::{
    arbitrate_ownership_by_id, log_plugin_ownership_summary, owned_descriptors,
};
use super::shm::ShmPublisherRegistry;
use super::shm_client::ShmClientSubscriberRegistry;
use super::{
    LoadedPlugin, LoadedPluginMetadata, PluginManager, PluginTopologyState, TOPOLOGY_NOTIFICATIONS,
};
use crate::device_config::DaemonConfig;
use crate::managed_config::ManagedConfig;
use crate::normalize;
use crate::plugin_host::{HostedPlugin, current_max_plugin_log_level};

impl PluginManager {
    #[allow(
        clippy::too_many_lines,
        reason = "One straight-line startup loop over discovered candidates; splitting it apart would scatter one sequential process across several small functions."
    )]
    pub fn load(
        config: &DaemonConfig,
        managed: &ManagedConfig,
        operator_events: bool,
    ) -> Result<Self> {
        let mut catalogue = build_catalogue(config, managed)?;
        let candidate_count = catalogue.len();

        let mut loaded: Vec<LoadedPlugin> = Vec::new();
        let mut loaded_ids: Vec<LoadedPluginId> = Vec::new();
        let mut descriptors_by_plugin: Vec<Vec<DeviceDescriptor>> = Vec::new();
        let mut loaded_metadata: Vec<LoadedPluginMetadata> = Vec::new();
        let mut owner_by_device: HashMap<String, LoadedPluginId> = HashMap::new();
        let mut skipped_optional = 0_usize;

        for entry in &mut catalogue {
            if !entry.effective_enabled {
                tracing::debug!(
                    path = %entry.path.display(),
                    plugin = %entry.name,
                    "leaving installed plugin inactive"
                );
                continue;
            }
            if matches!(entry.runtime, PluginRuntimeState::Failed(_)) {
                skipped_optional += 1;
                continue;
            }
            tracing::debug!(
                path = %entry.path.display(),
                required = entry.required,
                "loading plugin candidate"
            );
            let outcome = load_plugin(
                &entry.path,
                entry.configuration_cbor.clone(),
                operator_events,
            );
            let Some((plugin, descriptors)) =
                classify_load_outcome(outcome, entry.required, &entry.path)?
            else {
                skipped_optional += 1;
                entry.set_runtime(PluginRuntimeState::Failed(
                    "plugin failed during startup".to_owned(),
                ));
                continue;
            };

            if plugin.metadata.name != entry.name || plugin.metadata.version != entry.version {
                let error = anyhow::anyhow!(
                    "plugin identity changed after inspection: expected {} {}, loaded {} {}",
                    entry.name,
                    entry.version,
                    plugin.metadata.name,
                    plugin.metadata.version
                );
                if entry.required {
                    return Err(error).with_context(|| {
                        format!("required plugin failed to load: {}", entry.path.display())
                    });
                }
                tracing::warn!(
                    path = %entry.path.display(),
                    error = %error,
                    "skipping optional plugin whose identity changed after inspection"
                );
                skipped_optional += 1;
                entry.set_runtime(PluginRuntimeState::Failed(error.to_string()));
                continue;
            }

            if loaded_metadata
                .iter()
                .any(|metadata| metadata.name == plugin.metadata.name)
            {
                let error =
                    anyhow::anyhow!("duplicate loaded plugin name: {}", plugin.metadata.name);
                if entry.required {
                    return Err(error).with_context(|| {
                        format!("required plugin failed to load: {}", entry.path.display())
                    });
                }
                tracing::warn!(
                    path = %entry.path.display(),
                    plugin = %plugin.metadata.name,
                    "skipping optional plugin with duplicate name"
                );
                skipped_optional += 1;
                entry.set_runtime(PluginRuntimeState::Failed(error.to_string()));
                continue;
            }

            let candidate_owners = match validate_startup_candidate(
                &loaded_metadata,
                &loaded_ids,
                &descriptors_by_plugin,
                plugin.id,
                &plugin.metadata,
                &descriptors,
            ) {
                Ok(owners) => owners,
                Err(error) if entry.required => {
                    return Err(error).with_context(|| {
                        format!("required plugin failed to load: {}", entry.path.display())
                    });
                }
                Err(error) => {
                    tracing::warn!(path = %entry.path.display(), plugin = %plugin.metadata.name, error = %error, "skipping optional plugin with conflicting aggregate topology");
                    skipped_optional += 1;
                    entry.set_runtime(PluginRuntimeState::Failed(error.to_string()));
                    continue;
                }
            };

            warn_if_adopt_has_no_exact_readback(&plugin.metadata, &descriptors);
            owner_by_device = candidate_owners;
            tracing::info!(
                plugin = %plugin.metadata.name,
                version = %plugin.metadata.version,
                priority = plugin.metadata.priority,
                declared_devices = descriptors.len(),
                path = %plugin.metadata.path.display(),
                "loaded plugin"
            );
            loaded_metadata.push(plugin.metadata.clone());
            loaded_ids.push(plugin.id);
            loaded.push(plugin);
            descriptors_by_plugin.push(descriptors);
            entry.set_runtime(PluginRuntimeState::Loaded);
        }

        log_plugin_ownership_summary(
            loaded
                .iter()
                .zip(descriptors_by_plugin.iter())
                .map(|(plugin, descriptors)| {
                    (
                        plugin.metadata.name.as_str(),
                        plugin.id,
                        descriptors.as_slice(),
                    )
                }),
            &owner_by_device,
        );
        tracing::info!(
            candidates = candidate_count,
            inactive = catalogue
                .iter()
                .filter(|entry| entry.runtime == PluginRuntimeState::Inactive)
                .count(),
            desired_overrides = catalogue
                .iter()
                .filter(|entry| entry.desired_enabled.is_some())
                .count(),
            loaded = loaded.len(),
            skipped_optional,
            owned_devices = owner_by_device.len(),
            "plugin load summary"
        );

        Ok(Self {
            catalogue: RwLock::new(catalogue),
            loaded: RwLock::new(loaded),
            topology: Mutex::new(PluginTopologyState {
                descriptors_by_plugin: loaded_ids.into_iter().zip(descriptors_by_plugin).collect(),
                owner_by_device,
            }),
            shm: ShmPublisherRegistry::default(),
            shm_client: ShmClientSubscriberRegistry::default(),
            setup_sessions: Mutex::new(super::setup::SetupSessions::new()),
            operator_events,
        })
    }
}

/// Candidate-normalizes a plugin against all previously accepted startup
/// topology and returns the ownership map to commit with it.
pub(super) fn validate_startup_candidate(
    loaded_metadata: &[LoadedPluginMetadata],
    existing_ids: &[LoadedPluginId],
    descriptors_by_plugin: &[Vec<DeviceDescriptor>],
    candidate_id: LoadedPluginId,
    candidate_metadata: &LoadedPluginMetadata,
    candidate_descriptors: &[DeviceDescriptor],
) -> Result<HashMap<String, LoadedPluginId>> {
    let mut plugins = existing_ids
        .iter()
        .zip(loaded_metadata)
        .zip(descriptors_by_plugin)
        .map(|((&id, metadata), descriptors)| (id, metadata.clone(), descriptors.clone()))
        .collect::<Vec<_>>();
    plugins.push((
        candidate_id,
        candidate_metadata.clone(),
        candidate_descriptors.to_vec(),
    ));

    let candidate_owners = arbitrate_ownership_by_id(&plugins);
    let aggregate = owned_descriptors(
        &candidate_owners,
        plugins
            .iter()
            .map(|(id, _, descriptors)| (*id, descriptors.as_slice())),
    );
    normalize::normalize_devices(&aggregate).with_context(|| {
        format!(
            "plugin {} conflicts with the accepted startup topology",
            candidate_metadata.name
        )
    })?;
    Ok(candidate_owners)
}

/// Applies the daemon's plugin-isolation policy to one plugin load
/// result.
///
/// Successfully loaded plugins are kept (`Ok(Some)`). Failed optional
/// plugins are logged and skipped (`Ok(None)`). Failed required plugins
/// abort daemon startup (`Err`).
///
/// Separated from the startup loop so this policy can be tested in
/// isolation, including invalid-topology failures reported by
/// `load_plugin`, without loading a real shared object.
pub(super) fn classify_load_outcome<T>(
    outcome: Result<T>,
    required: bool,
    path: &Path,
) -> Result<Option<T>> {
    match outcome {
        Ok(plugin) => Ok(Some(plugin)),
        Err(error) if required => Err(error)
            .with_context(|| format!("required plugin failed to load: {}", path.display())),
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                error_chain = ?error,
                "skipping optional plugin"
            );
            Ok(None)
        }
    }
}

pub(super) fn warn_if_adopt_has_no_exact_readback(
    metadata: &LoadedPluginMetadata,
    descriptors: &[DeviceDescriptor],
) {
    if metadata.recommended_reconciliation != Some(control::ReconciliationPolicy::Adopt)
        || descriptors.iter().any(device_has_exact_readback)
    {
        return;
    }

    tracing::warn!(
        plugin = %metadata.name,
        "plugin recommends Adopt reconciliation but advertises no exact readable state facets"
    );
}

pub(super) fn device_has_exact_readback(device: &DeviceDescriptor) -> bool {
    iter::once(&device.capabilities)
        .chain(device.surfaces.iter().map(|surface| &surface.capabilities))
        .chain(
            device
                .surfaces
                .iter()
                .flat_map(|surface| surface.elements.iter().map(|element| &element.capabilities)),
        )
        .chain(device.groups.iter().map(|group| &group.capabilities))
        .any(|capabilities| {
            matches!(
                &capabilities.state_readback,
                StateReadbackCapability::Readable { facets, .. }
                    if facets.iter().any(|facet| facet.fidelity == ReadbackFidelity::Exact)
            )
        })
}

pub(super) fn load_plugin(
    path: &Path,
    configuration_cbor: Vec<u8>,
    operator_events: bool,
) -> Result<(LoadedPlugin, Vec<DeviceDescriptor>)> {
    #[allow(
        clippy::expect_used,
        clippy::unwrap_in_result,
        reason = "Acceptable to panic on poisoned locks"
    )]
    let sender = TOPOLOGY_NOTIFICATIONS
        .get()
        .and_then(|slot| {
            slot.lock()
                .expect("topology notification sender lock poisoned")
                .clone()
        })
        .context("topology notification sender was not installed before plugin loading")?;
    let (host, metadata, descriptors) = HostedPlugin::spawn_with_configuration(
        path,
        sender,
        current_max_plugin_log_level(),
        configuration_cbor,
        operator_events,
    )?;
    normalize::normalize_devices(&descriptors)
        .with_context(|| format!("plugin {} produced an invalid topology", metadata.name))?;
    Ok((
        LoadedPlugin {
            id: LoadedPluginId::new(),
            metadata: LoadedPluginMetadata {
                name: metadata.name,
                version: metadata.version,
                priority: metadata.priority,
                recommended_reconciliation: metadata.recommended_reconciliation,
                probe_outcome: metadata.probe_outcome,
                buses: metadata.buses,
                vendors: metadata.vendors,
                probe_hints: metadata.probe_hints,
                path: path.to_path_buf(),
            },
            host: Arc::new(host),
        },
        descriptors,
    ))
}
