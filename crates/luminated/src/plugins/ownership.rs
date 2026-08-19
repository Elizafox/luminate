// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Plugin device ownership and topology comparison.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use luminate_core::device::{Device, DeviceId};
use luminate_plugin_api::{ClaimExclusivity, DeviceDescriptor, HardwareBus};

use super::LoadedPluginMetadata;
use super::id::LoadedPluginId;

type ClaimKey = (HardwareBus, String, String);

/// Selects whole logical devices in descending provider-priority order.
pub(super) fn arbitrate_ownership<'a>(
    metadata: impl IntoIterator<Item = &'a LoadedPluginMetadata>,
    descriptors_by_plugin: &[Vec<DeviceDescriptor>],
) -> HashMap<String, usize> {
    let metadata = metadata.into_iter().collect::<Vec<_>>();
    let mut candidates = descriptors_by_plugin
        .iter()
        .enumerate()
        .flat_map(|(plugin_index, descriptors)| {
            descriptors
                .iter()
                .enumerate()
                .map(move |(descriptor_index, descriptor)| {
                    (plugin_index, descriptor_index, descriptor)
                })
        })
        .collect::<Vec<_>>();

    candidates.sort_by_key(|(plugin_index, descriptor_index, _)| {
        (
            Reverse(
                metadata
                    .get(*plugin_index)
                    .map_or(i32::MIN, |item| item.priority),
            ),
            *plugin_index,
            *descriptor_index,
        )
    });

    let mut owners = HashMap::new();
    let mut accepted_claims: HashMap<ClaimKey, Vec<ClaimExclusivity>> = HashMap::new();
    for (plugin_index, _, descriptor) in candidates {
        if owners.contains_key(&descriptor.id) {
            continue;
        }
        let conflict = descriptor.claims.iter().find(|claim| {
            let key = (
                claim.bus,
                claim.physical_identity.clone(),
                claim.control_domain.clone(),
            );
            accepted_claims.get(&key).is_some_and(|existing| {
                claim.exclusivity == ClaimExclusivity::Exclusive
                    || existing.contains(&ClaimExclusivity::Exclusive)
            })
        });
        if let Some(claim) = conflict {
            tracing::info!(
                plugin = metadata.get(plugin_index).map_or("unknown", |item| item.name.as_str()),
                device = %descriptor.id,
                bus = ?claim.bus,
                physical_identity = %claim.physical_identity,
                control_domain = %claim.control_domain,
                "plugin device lost a hardware resource claim"
            );
            continue;
        }

        owners.insert(descriptor.id.clone(), plugin_index);

        for claim in &descriptor.claims {
            accepted_claims
                .entry((
                    claim.bus,
                    claim.physical_identity.clone(),
                    claim.control_domain.clone(),
                ))
                .or_default()
                .push(claim.exclusivity);
        }
    }
    owners
}

/// Runs [`arbitrate_ownership`] over plugins identified by a stable
/// [`LoadedPluginId`] rather than position, translating its positional
/// result back to the corresponding id immediately, so nothing outside this
/// function ever observes a bare position that could go stale if a plugin
/// is unloaded before it's used.
pub(super) fn arbitrate_ownership_by_id(
    plugins: &[(LoadedPluginId, LoadedPluginMetadata, Vec<DeviceDescriptor>)],
) -> HashMap<String, LoadedPluginId> {
    let metadata = plugins
        .iter()
        .map(|(_, metadata, _)| metadata)
        .collect::<Vec<_>>();
    let descriptors = plugins
        .iter()
        .map(|(_, _, descriptors)| descriptors.clone())
        .collect::<Vec<_>>();
    let positional = arbitrate_ownership(metadata, &descriptors);
    positional
        .into_iter()
        .filter_map(|(device, index)| plugins.get(index).map(|(id, _, _)| (device, *id)))
        .collect()
}

/// Logs a per-plugin declared/owned/lost device breakdown.
///
/// Generic over the ownership key so it works both against the transient
/// positional indices used while validating a startup candidate and the
/// stable `LoadedPluginId` keys `PluginManager` keeps in its topology state
/// long-term.
pub(super) fn log_plugin_ownership_summary<'a, K>(
    plugins: impl Iterator<Item = (&'a str, K, &'a [DeviceDescriptor])>,
    owner_by_device: &HashMap<String, K>,
) where
    K: Copy + Eq + Hash,
{
    for (name, key, descriptors) in plugins {
        let (declared_devices, owned_devices, lost_devices) =
            ownership_summary(key, descriptors, owner_by_device);

        tracing::info!(
            plugin = %name,
            declared_device_count = declared_devices.len(),
            owned_device_count = owned_devices.len(),
            lost_device_count = lost_devices.len(),
            declared_devices = ?declared_devices,
            owned_devices = ?owned_devices,
            lost_devices = ?lost_devices,
            "plugin ownership summary"
        );
    }
}

fn ownership_summary<'a, K>(
    key: K,
    descriptors: &'a [DeviceDescriptor],
    owner_by_device: &HashMap<String, K>,
) -> (Vec<&'a str>, Vec<&'a str>, Vec<&'a str>)
where
    K: Copy + Eq + Hash,
{
    let declared_devices = descriptors
        .iter()
        .map(|descriptor| descriptor.id.as_str())
        .collect::<Vec<_>>();

    let owned_devices = descriptors
        .iter()
        .filter(|descriptor| owner_by_device.get(&descriptor.id) == Some(&key))
        .map(|descriptor| descriptor.id.as_str())
        .collect::<Vec<_>>();

    let lost_devices = descriptors
        .iter()
        .filter(|descriptor| owner_by_device.get(&descriptor.id) != Some(&key))
        .map(|descriptor| descriptor.id.as_str())
        .collect::<Vec<_>>();

    (declared_devices, owned_devices, lost_devices)
}

pub(super) fn owned_descriptors<'a, K>(
    owner_by_device: &HashMap<String, K>,
    plugins: impl Iterator<Item = (K, &'a [DeviceDescriptor])>,
) -> Vec<DeviceDescriptor>
where
    K: Copy + Eq + Hash,
{
    plugins
        .flat_map(|(key, descriptors)| {
            descriptors
                .iter()
                .filter(move |descriptor| owner_by_device.get(&descriptor.id) == Some(&key))
                .cloned()
        })
        .collect()
}

pub(super) fn changed_device_ids(previous: &[Device], current: &[Device]) -> Vec<DeviceId> {
    let previous = previous
        .iter()
        .map(|device| (device.id.clone(), device))
        .collect::<HashMap<_, _>>();

    let current = current
        .iter()
        .map(|device| (device.id.clone(), device))
        .collect::<HashMap<_, _>>();

    let mut ids = previous
        .keys()
        .chain(current.keys())
        .filter(|id| previous.get(*id) != current.get(*id))
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    ids
}

#[cfg(test)]
#[path = "ownership_tests.rs"]
mod tests;
