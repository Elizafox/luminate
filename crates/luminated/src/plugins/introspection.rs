// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Read-only views over the loaded plugin set and its committed topology.
//!
//! These are the cheap lookups the rest of the daemon leans on constantly, so
//! they all resolve against a snapshot rather than holding a lock across the
//! caller's work.

use luminate_core::control;
use luminate_core::device::DeviceId;
use luminate_plugin_api::DeviceDescriptor;

use super::ownership::owned_descriptors;
use super::{LoadedPluginMetadata, PluginManager};

impl PluginManager {
    #[must_use]
    pub fn loaded_metadata(&self) -> Vec<LoadedPluginMetadata> {
        self.loaded_snapshot()
            .into_iter()
            .map(|plugin| plugin.metadata)
            .collect()
    }

    #[must_use]
    #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
    pub fn len(&self) -> usize {
        self.loaded
            .read()
            .expect("plugin loaded lock poisoned")
            .len()
    }

    /// Every loaded plugin's name, in load order.
    ///
    /// A daemon-initiated rescan re-pulls all of them, since it has no way to
    /// know which hardware moved. Load order keeps the resulting batch
    /// deterministic before `collect_topology_batch` sorts it.
    #[must_use]
    pub fn plugin_names(&self) -> Vec<String> {
        self.loaded_snapshot()
            .into_iter()
            .map(|plugin| plugin.metadata.name)
            .collect()
    }

    #[must_use]
    pub fn recommended_reconciliation(
        &self,
        device: &DeviceId,
    ) -> Option<control::ReconciliationPolicy> {
        #[allow(
            clippy::expect_used,
            clippy::unwrap_in_result,
            reason = "Acceptable to panic on poisoned locks"
        )]
        let owner = self
            .topology
            .lock()
            .expect("plugin topology lock poisoned")
            .owner_by_device
            .get(device.as_str())
            .copied()?;
        self.loaded_snapshot()
            .into_iter()
            .find(|plugin| plugin.id == owner)?
            .metadata
            .recommended_reconciliation
    }

    #[must_use]
    pub fn owner_name(&self, device: &DeviceId) -> Option<String> {
        #[allow(
            clippy::expect_used,
            clippy::unwrap_in_result,
            reason = "Acceptable to panic on poisoned locks"
        )]
        let owner = self
            .topology
            .lock()
            .expect("plugin topology lock poisoned")
            .owner_by_device
            .get(device.as_str())
            .copied()?;
        self.loaded_snapshot()
            .into_iter()
            .find(|plugin| plugin.id == owner)
            .map(|plugin| plugin.metadata.name)
    }

    #[must_use]
    #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
    pub fn configured_reconciliation(
        &self,
        device: &DeviceId,
    ) -> Option<control::ReconciliationPolicy> {
        let owner = self.owner_name(device)?;
        self.catalogue
            .read()
            .expect("plugin catalogue lock poisoned")
            .iter()
            .find(|plugin| plugin.name == owner)?
            .effective_reconciliation
    }

    /// Every device descriptor from every loaded plugin whose device ID
    /// actually won ownership. A plugin that lost a device-ownership
    /// conflict (see `assign_device_owner`) still loads successfully and can
    /// still own its other devices, but its descriptor for the contested
    /// ID is filtered out here rather than reaching normalization, which
    /// would otherwise hard-error on the resulting duplicate ID.
    #[must_use]
    pub fn device_descriptors(&self) -> Vec<DeviceDescriptor> {
        #[allow(
            clippy::expect_used,
            clippy::unwrap_in_result,
            reason = "Acceptable to panic on poisoned locks"
        )]
        let topology = self.topology.lock().expect("plugin topology lock poisoned");
        owned_descriptors(
            &topology.owner_by_device,
            topology
                .descriptors_by_plugin
                .iter()
                .map(|(id, descriptors)| (*id, descriptors.as_slice())),
        )
    }
}
