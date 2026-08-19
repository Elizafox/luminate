// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Plugin discovery, loading, ownership resolution, and update routing.
//!
//! `PluginManager` is the crate-facing type and composition root; its concerns
//! are split across sibling modules under `plugins/`:
//!
//! - `loading`: startup load and the required/optional isolation policy
//! - `managed`: reconciling hosts with committed managed configuration
//! - `introspection`: read-only views over the loaded set and topology
//! - `topology`: rescans, re-pulls, unloads, and reloads
//! - `apply`: routing validated updates to each device's owning plugin
//! - `streams`: the shared-memory frame transports in both directions
//!
//! This file keeps the types themselves, the topology-notification plumbing,
//! and the few private lookups every one of those modules needs.

use luminate_core::control;

mod apply;
mod catalogue;
mod discovery;
mod id;
mod introspection;
mod loading;
mod managed;
mod ownership;
pub(crate) mod setup;
mod shm;
mod shm_client;
mod streams;
mod topology;

use catalogue::{PluginCatalogueEntry, PluginRuntimeState};
#[cfg(test)]
use discovery::{discover_candidates, is_shared_object, resolve_configured_plugin};
use id::LoadedPluginId;
use shm::ShmPublisherRegistry;
use shm_client::ShmClientSubscriberRegistry;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use tokio::sync::mpsc;

use luminate_core::device::{Device, DeviceId};
use luminate_core::target::TargetId;
#[cfg(test)]
use luminate_plugin_api::ClaimExclusivity;
use luminate_plugin_api::{
    DeviceDescriptor, PluginBus, PluginTarget, PluginVendorId, RescanReason,
};

#[cfg(test)]
use crate::device_config::PluginConfig;
use crate::error::DaemonError;
use crate::plugin_host::HostedPlugin;

/// Why the topology coordinator was woken up.
///
/// The variants differ in who detected the change, and therefore in how
/// much work is required. A plugin that observed its own hardware change
/// knows exactly which topology may have gone stale, so only that plugin
/// is re-pulled. A daemon-initiated rescan has no such knowledge; a resume
/// or operator request means "anything may have changed". Therefore, every
/// loaded plugin is re-enumerated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopologyNotification {
    /// A plugin observed its own hardware change and asked for a re-pull.
    PluginObserved(String),

    /// The daemon is asking every plugin to re-enumerate.
    Rescan(RescanReason),
}

/// A handle for requesting a full topology rescan.
///
/// Wraps the notification sender so every rescan source (resume from
/// suspend, device-change notifications, `SIGUSR1`, and the control
/// protocol) uses the same entry point instead of constructing
/// `TopologyNotification::Rescan` directly.
#[derive(Clone, Debug)]
pub struct RescanRequester {
    notifications: mpsc::UnboundedSender<TopologyNotification>,
}

impl RescanRequester {
    #[must_use]
    pub fn new(notifications: mpsc::UnboundedSender<TopologyNotification>) -> Self {
        Self { notifications }
    }

    /// Requests a full topology rescan.
    ///
    /// Returns whether the request was successfully queued. `false` means the
    /// topology coordinator has already gone away, which occurs only during
    /// daemon shutdown. Callers log this condition rather than treating it as
    /// an error, because a rescan cannot complete once shutdown has begun.
    pub fn request(&self, reason: RescanReason) -> bool {
        self.notifications
            .send(TopologyNotification::Rescan(reason))
            .is_ok()
    }
}

static TOPOLOGY_NOTIFICATIONS: OnceLock<
    Mutex<Option<mpsc::UnboundedSender<TopologyNotification>>>,
> = OnceLock::new();

#[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
pub fn install_topology_notification_sender(sender: mpsc::UnboundedSender<TopologyNotification>) {
    let slot = TOPOLOGY_NOTIFICATIONS.get_or_init(|| Mutex::new(None));
    *slot
        .lock()
        .expect("topology notification sender lock poisoned") = Some(sender);
}

#[derive(Debug, Clone)]
pub struct LoadedPluginMetadata {
    pub name: String,
    pub version: String,
    pub priority: i32,
    pub recommended_reconciliation: Option<control::ReconciliationPolicy>,
    pub probe_outcome: luminate_plugin_api::ProbeOutcome,
    pub buses: Vec<PluginBus>,
    pub vendors: Vec<PluginVendorId>,
    pub probe_hints: Vec<String>,

    pub path: PathBuf,
}

/// A loaded plugin instance.
///
/// Cheaply `Clone`, so callers can pull a consistent snapshot out from
/// under `PluginManager.loaded`'s lock and use it (including the
/// potentially slow plugin-host round trip in `host`'s methods) without
/// holding that lock for the duration.
#[derive(Clone)]
pub struct LoadedPlugin {
    id: LoadedPluginId,
    metadata: LoadedPluginMetadata,
    host: Arc<HostedPlugin>,
}

/// A plugin manager's mutable state.
///
/// `loaded` and `topology` are two separate locks rather than one, so a
/// slow plugin-host round trip (topology re-pulls, applies) taken under
/// `topology` doesn't block unrelated lookups of `loaded`. Because plugins
/// can be removed at runtime (see `unload_plugin`/`reload_plugin`), a
/// `LoadedPluginId` obtained from `topology` may no longer exist in `loaded`
/// by the time it's looked up there; callers treat that as an ordinary
/// "device currently unowned" outcome rather than a bug. Methods that must
/// mutate both always take `loaded` before `topology` to avoid a lock-order
/// inversion.
pub struct PluginManager {
    catalogue: RwLock<Vec<PluginCatalogueEntry>>,
    loaded: RwLock<Vec<LoadedPlugin>>,
    topology: Mutex<PluginTopologyState>,
    shm: ShmPublisherRegistry,
    shm_client: ShmClientSubscriberRegistry,
    setup_sessions: Mutex<setup::SetupSessions>,
    operator_events: bool,
}

struct PluginTopologyState {
    descriptors_by_plugin: Vec<(LoadedPluginId, Vec<DeviceDescriptor>)>,
    owner_by_device: HashMap<String, LoadedPluginId>,
}

#[derive(Debug)]
pub struct TopologyReconcile {
    pub devices: Vec<Device>,
    pub changed_devices: Vec<DeviceId>,
}

#[derive(Debug)]
pub struct ManagedActivationReconcile {
    pub topology: Option<TopologyReconcile>,
    pub activated: bool,
}

impl PluginManager {
    #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
    fn loaded_snapshot(&self) -> Vec<LoadedPlugin> {
        self.loaded
            .read()
            .expect("plugin loaded lock poisoned")
            .clone()
    }

    #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
    fn set_runtime_state(&self, name: &str, runtime: PluginRuntimeState) {
        if let Some(entry) = self
            .catalogue
            .write()
            .expect("plugin catalogue lock poisoned")
            .iter_mut()
            .find(|entry| entry.name == name)
        {
            entry.set_runtime(runtime);
        }
    }

    /// Looks up the plugin currently owning `device_id`.
    ///
    /// Returns an owned [`LoadedPlugin`] (a cheap clone) rather than a
    /// reference, so the caller doesn't hold `self.loaded`'s lock for the
    /// duration of whatever plugin-host call it makes next. A lookup miss
    /// here is reported the same way as "no owner" (`DeviceUnowned`), since
    /// the plugin may have been legitimately unloaded between resolving the
    /// device's owner id and looking that id up here.
    fn plugin_for_device(&self, device_id: &str) -> Result<LoadedPlugin, DaemonError> {
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
            .get(device_id)
            .copied()
            .ok_or_else(|| DaemonError::DeviceUnowned(device_id.to_owned()))?;

        self.loaded_snapshot()
            .into_iter()
            .find(|plugin| plugin.id == owner)
            .ok_or_else(|| DaemonError::DeviceUnowned(device_id.to_owned()))
    }
}

fn plugin_target_from_target_id(target: &TargetId) -> PluginTarget {
    match target {
        TargetId::Device(device) => PluginTarget::Device {
            device: device.as_str().to_owned(),
        },
        TargetId::Surface { device, surface } => PluginTarget::Surface {
            device: device.as_str().to_owned(),
            surface: surface.as_str().to_owned(),
        },
        TargetId::Element {
            device,
            surface,
            element,
        } => PluginTarget::Element {
            device: device.as_str().to_owned(),
            surface: surface.as_str().to_owned(),
            element: element.as_str().to_owned(),
        },
        TargetId::Group { device, group } => PluginTarget::Group {
            device: device.as_str().to_owned(),
            group: group.as_str().to_owned(),
        },
    }
}

#[cfg(test)]
mod tests;
