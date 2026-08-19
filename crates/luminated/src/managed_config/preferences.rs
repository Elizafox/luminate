// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Merging daemon preferences across the global and managed layers.
//!
//! Global configuration supplies the base; the managed layer overrides it
//! unless the operator locked the setting. The change-recording helpers here
//! exist so a committed patch can report exactly which keys moved, which is
//! what the `ConfigurationChanged` event carries.

use std::collections::HashMap;
use std::hash::Hash;

use luminate_core::control::ReconciliationPolicy;
use luminate_core::device::DeviceId;
use luminate_protocol::{DaemonPreferences, DeviceReconciliationPreference};

use super::schema::PluginActivationInputs;
use super::settings::dotted_key_contains;
use super::{ManagedDaemonConfig, ManagedPluginConfig};
use crate::device_config::{DaemonConfig, ManagedPluginActivation, PluginActivationMode};

pub(super) fn daemon_preferences_from_managed(managed: &ManagedDaemonConfig) -> DaemonPreferences {
    DaemonPreferences {
        default_unsupported_policy: managed.default_unsupported_policy,
        reconciliation_policy: managed.reconciliation_policy,
        device_reconciliation: sorted_device_preferences(&managed.device_reconciliation),
        cct_emulation: managed.cct_emulation,
        prefer_shm: managed.prefer_shm,
        prefer_client_shm: managed.prefer_client_shm,
    }
}

pub(super) fn daemon_preferences_from_config(config: &DaemonConfig) -> DaemonPreferences {
    DaemonPreferences {
        default_unsupported_policy: Some(config.default_unsupported_policy),
        reconciliation_policy: config.reconciliation_policy,
        device_reconciliation: sorted_device_preferences(&config.device_reconciliation),
        cct_emulation: config.cct_emulation,
        prefer_shm: Some(config.prefer_shm),
        prefer_client_shm: Some(config.prefer_client_shm),
    }
}

fn sorted_device_preferences(
    preferences: &HashMap<DeviceId, ReconciliationPolicy>,
) -> Vec<DeviceReconciliationPreference> {
    let mut preferences = preferences
        .iter()
        .map(|(device, policy)| DeviceReconciliationPreference {
            device: device.clone(),
            policy: *policy,
        })
        .collect::<Vec<_>>();
    preferences.sort_by(|left, right| left.device.as_str().cmp(right.device.as_str()));
    preferences
}

pub(super) fn managed_daemon_config(preferences: DaemonPreferences) -> ManagedDaemonConfig {
    ManagedDaemonConfig {
        default_unsupported_policy: preferences.default_unsupported_policy,
        reconciliation_policy: preferences.reconciliation_policy,
        device_reconciliation: preferences
            .device_reconciliation
            .into_iter()
            .map(|preference| (preference.device, preference.policy))
            .collect(),
        cct_emulation: preferences.cct_emulation,
        prefer_shm: preferences.prefer_shm,
        prefer_client_shm: preferences.prefer_client_shm,
    }
}

pub(super) fn changed_daemon_preferences(
    previous: &ManagedDaemonConfig,
    next: &ManagedDaemonConfig,
) -> Vec<String> {
    let mut changed = Vec::new();
    record_change(
        &mut changed,
        "default_unsupported_policy",
        &previous.default_unsupported_policy,
        &next.default_unsupported_policy,
    );
    record_change(
        &mut changed,
        "reconciliation_policy",
        &previous.reconciliation_policy,
        &next.reconciliation_policy,
    );
    record_scoped_changes(
        &mut changed,
        "device_reconciliation",
        &previous.device_reconciliation,
        &next.device_reconciliation,
        DeviceId::as_str,
    );
    record_change(
        &mut changed,
        "cct_emulation",
        &previous.cct_emulation,
        &next.cct_emulation,
    );
    record_change(
        &mut changed,
        "prefer_shm",
        &previous.prefer_shm,
        &next.prefer_shm,
    );
    record_change(
        &mut changed,
        "prefer_client_shm",
        &previous.prefer_client_shm,
        &next.prefer_client_shm,
    );
    changed
}

fn record_change<T: PartialEq>(changed: &mut Vec<String>, key: &str, previous: &T, next: &T) {
    if previous != next {
        changed.push(key.to_owned());
    }
}

fn record_scoped_changes<K, V, F>(
    changed: &mut Vec<String>,
    root: &str,
    previous: &HashMap<K, V>,
    next: &HashMap<K, V>,
    key_name: F,
) where
    K: Eq + Hash,
    V: PartialEq,
    F: Fn(&K) -> &str,
{
    let mut keys = previous
        .keys()
        .chain(next.keys())
        .map(&key_name)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        let previous = previous
            .iter()
            .find(|(candidate, _)| key_name(candidate) == key)
            .map(|(_, value)| value);
        let next = next
            .iter()
            .find(|(candidate, _)| key_name(candidate) == key)
            .map(|(_, value)| value);
        if previous != next {
            changed.push(format!("{root}.{key}"));
        }
    }
}

/// Resolves manageable daemon preferences without changing either stored
/// layer.
///
/// Managed values blocked by administrator locks remain present in
/// [`ManagedConfig`], but do not affect the returned effective configuration.
#[must_use]
pub fn merge_daemon_preferences(
    global: &DaemonConfig,
    managed: &ManagedDaemonConfig,
) -> DaemonConfig {
    let mut effective = global.clone();
    let locks = &global.management.locked_daemon_settings;

    if !daemon_setting_locked(locks, "default_unsupported_policy")
        && let Some(value) = managed.default_unsupported_policy
    {
        effective.default_unsupported_policy = value;
    }
    if !daemon_setting_locked(locks, "reconciliation_policy")
        && let Some(value) = managed.reconciliation_policy
    {
        effective.reconciliation_policy = Some(value);
    }
    merge_scoped_preferences(
        &mut effective.device_reconciliation,
        &managed.device_reconciliation,
        locks,
        "device_reconciliation",
        |key| key.as_str(),
    );
    if !daemon_setting_locked(locks, "cct_emulation")
        && let Some(value) = managed.cct_emulation
    {
        effective.cct_emulation = Some(value);
    }
    if !daemon_setting_locked(locks, "prefer_shm")
        && let Some(value) = managed.prefer_shm
    {
        effective.prefer_shm = value;
    }
    if !daemon_setting_locked(locks, "prefer_client_shm")
        && let Some(value) = managed.prefer_client_shm
    {
        effective.prefer_client_shm = value;
    }

    effective
}

/// Resolves administrator policy, managed desired state, and global defaults
/// for one installed plugin.
#[must_use]
pub fn plugin_is_enabled(
    activation_mode: PluginActivationMode,
    global_activation: ManagedPluginActivation,
    managed: Option<&ManagedPluginConfig>,
    inputs: PluginActivationInputs,
) -> bool {
    if inputs.required {
        return true;
    }

    match global_activation {
        ManagedPluginActivation::Enabled => return true,
        ManagedPluginActivation::Disabled => return false,
        ManagedPluginActivation::Managed => {}
    }

    if let Some(enabled) = managed.and_then(|plugin| plugin.enabled) {
        return enabled;
    }

    inputs.explicit
        || (activation_mode != PluginActivationMode::Explicit && inputs.automatically_selected)
}

pub(super) fn daemon_setting_locked(locks: &[String], key: &str) -> bool {
    locks
        .iter()
        .any(|locked| locked == key || dotted_key_contains(locked, key))
}

fn merge_scoped_preferences<K, V, F>(
    effective: &mut HashMap<K, V>,
    managed: &HashMap<K, V>,
    locks: &[String],
    root: &str,
    key_name: F,
) where
    K: Clone + Eq + Hash,
    V: Copy,
    F: Fn(&K) -> &str,
{
    let mut entries: Vec<_> = managed.iter().collect();
    entries.sort_by(|(left, _), (right, _)| key_name(left).cmp(key_name(right)));
    for (key, value) in entries {
        let setting = format!("{root}.{}", key_name(key));
        if !daemon_setting_locked(locks, &setting) {
            effective.insert(key.clone(), *value);
        }
    }
}
