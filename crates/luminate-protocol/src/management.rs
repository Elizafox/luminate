// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Shared models for reading and atomically changing managed configuration.

use std::collections::BTreeMap;
use std::fmt;

use luminate_core::capability::CctEmulation;
use luminate_core::control::{ReconciliationPolicy, UnsupportedPolicy};
use luminate_core::device::DeviceId;
use serde::{Deserialize, Serialize};

/// A value supported by plugin setting schemas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SettingValue {
    /// A Boolean value.
    Boolean(bool),
    /// A signed integer value.
    Integer(i64),
    /// A finite floating-point value.
    Number(f64),
    /// A UTF-8 string value.
    String(String),
    /// An ordered sequence of values.
    Array(Vec<Self>),
    /// A string-keyed table.
    Table(BTreeMap<String, Self>),
}

/// A request value whose debug representation never reveals its contents.
///
/// Serialization deliberately includes the inner value because management
/// requests must deliver it to the daemon. Responses and change records use
/// [`ReportedSettingValue`] instead.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WriteOnly<T>(T);

impl<T> WriteOnly<T> {
    /// Wraps a value for transport in a request with redacted diagnostics.
    #[must_use]
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Consumes the wrapper and returns the value at the receiving boundary.
    #[must_use]
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for WriteOnly<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WriteOnly([REDACTED])")
    }
}

/// A setting value safe to return from the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ReportedSettingValue {
    /// The setting has no value in this configuration layer.
    Unset,
    /// A non-sensitive value.
    Visible(SettingValue),
    /// A sensitive value exists but is deliberately withheld.
    Redacted,
}

/// The value category accepted by a plugin setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginSettingKind {
    /// A Boolean value.
    Boolean,
    /// A signed integer value.
    Integer,
    /// A floating-point value.
    Number,
    /// A UTF-8 string value.
    String,
    /// One string selected from schema constraints.
    Enumeration,
    /// An array whose element constraints are described by the schema.
    Array,
}

/// How a changed plugin setting becomes active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginSettingApplyMode {
    /// The plugin host must be restarted.
    RestartRequired,
}

/// Public metadata for one manageable plugin setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginSettingSchema {
    /// Canonical dotted setting key.
    pub key: String,
    /// Short user-facing label.
    pub label: String,
    /// User-facing explanation of the setting.
    pub description: String,
    /// Accepted value category.
    pub kind: PluginSettingKind,
    /// Schema default, redacted when the setting is sensitive.
    pub default: ReportedSettingValue,
    /// Whether the effective configuration must contain a value.
    pub required: bool,
    /// Whether values for this setting must remain secret.
    pub sensitive: bool,
    /// How a changed value is applied.
    pub apply_mode: PluginSettingApplyMode,
    /// Inclusive numeric lower bound.
    pub minimum: Option<f64>,
    /// Inclusive numeric upper bound.
    pub maximum: Option<f64>,
    /// Kind-specific constraints, redacted when they contain sensitive data.
    pub constraints: ReportedSettingValue,
}

/// The daemon's current lifecycle state for an installed plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginRuntimeState {
    /// The plugin is not selected to run.
    Inactive,
    /// A plugin host is being started.
    Loading,
    /// The plugin host is running.
    Loaded,
    /// Startup or a later restart failed.
    Failed {
        /// Operator-safe failure diagnostic.
        diagnostic: String,
    },
}

/// Readable state for one installed plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManagedPlugin {
    /// Canonical plugin name.
    pub name: String,
    /// Inspected plugin version.
    pub version: String,
    /// Whether administrator policy requires the plugin.
    pub required: bool,
    /// Stored managed activation override.
    pub desired_enabled: Option<bool>,
    /// Stored managed reconciliation override.
    pub desired_reconciliation: Option<ReconciliationPolicy>,
    /// Reconciliation after global policy and managed state are merged.
    pub effective_reconciliation: Option<ReconciliationPolicy>,
    /// Activation after global policy and managed state are merged.
    pub effective_enabled: bool,
    /// Current runtime state.
    pub runtime: PluginRuntimeState,
    /// Whether managed activation is locked by administrator policy.
    pub activation_locked: bool,
    /// Settings declared by the plugin.
    pub schema: Vec<PluginSettingSchema>,
    /// Stored managed setting overrides, keyed by canonical dotted key.
    pub desired_settings: BTreeMap<String, ReportedSettingValue>,
    /// Effective settings after defaults, global policy, and managed state.
    pub effective_settings: BTreeMap<String, ReportedSettingValue>,
    /// Setting keys locked by administrator policy.
    pub locked_settings: Vec<String>,
}

/// One per-device reconciliation override.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceReconciliationPreference {
    /// Device whose policy is overridden.
    pub device: DeviceId,
    /// Reconciliation policy for the device.
    pub policy: ReconciliationPolicy,
}

/// Manageable daemon preference values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonPreferences {
    /// Behaviour when a selected target does not support an operation.
    pub default_unsupported_policy: Option<UnsupportedPolicy>,
    /// Default reconciliation policy.
    pub reconciliation_policy: Option<ReconciliationPolicy>,
    /// Per-device reconciliation overrides.
    pub device_reconciliation: Vec<DeviceReconciliationPreference>,
    /// Correlated-colour-temperature emulation preference.
    pub cct_emulation: Option<CctEmulation>,
    /// Whether daemon-to-plugin shared memory is preferred.
    pub prefer_shm: Option<bool>,
    /// Whether client-to-daemon shared memory is preferred.
    pub prefer_client_shm: Option<bool>,
}

/// Authoritative managed configuration and its resolved runtime view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManagementSnapshot {
    /// Monotonically increasing optimistic-concurrency revision.
    pub revision: u64,
    /// Stored daemon preference overrides.
    pub desired_daemon: DaemonPreferences,
    /// Effective daemon preferences after administrator locks are applied.
    pub effective_daemon: DaemonPreferences,
    /// Serialized daemon preference names locked by administrator policy.
    pub locked_daemon_settings: Vec<String>,
    /// Installed plugins in deterministic discovery order.
    pub plugins: Vec<ManagedPlugin>,
}

/// One mutation in an atomic managed-configuration patch.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum ManagementMutation {
    /// Replaces all stored daemon preference overrides.
    SetDaemonPreferences(DaemonPreferences),
    /// Sets or clears a plugin activation override.
    SetPluginEnabled {
        /// Canonical plugin name.
        plugin: String,
        /// Desired activation, or `None` to return to global policy.
        enabled: Option<bool>,
    },
    /// Sets or clears a plugin reconciliation override.
    SetPluginReconciliation {
        /// Canonical plugin name.
        plugin: String,
        /// Desired policy, or `None` to return to global policy.
        reconciliation: Option<ReconciliationPolicy>,
    },
    /// Sets one plugin setting.
    SetPluginSetting {
        /// Canonical plugin name.
        plugin: String,
        /// Canonical dotted setting key.
        key: String,
        /// New value. Debug output is always redacted.
        value: WriteOnly<SettingValue>,
    },
    /// Removes one stored plugin setting override.
    ClearPluginSetting {
        /// Canonical plugin name.
        plugin: String,
        /// Canonical dotted setting key.
        key: String,
    },
}

impl fmt::Debug for ManagementMutation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SetDaemonPreferences(preferences) => formatter
                .debug_tuple("SetDaemonPreferences")
                .field(preferences)
                .finish(),
            Self::SetPluginEnabled { plugin, enabled } => formatter
                .debug_struct("SetPluginEnabled")
                .field("plugin", plugin)
                .field("enabled", enabled)
                .finish(),
            Self::SetPluginReconciliation {
                plugin,
                reconciliation,
            } => formatter
                .debug_struct("SetPluginReconciliation")
                .field("plugin", plugin)
                .field("reconciliation", reconciliation)
                .finish(),
            Self::SetPluginSetting { plugin, key, .. } => formatter
                .debug_struct("SetPluginSetting")
                .field("plugin", plugin)
                .field("key", key)
                .field("value", &"[REDACTED]")
                .finish(),
            Self::ClearPluginSetting { plugin, key } => formatter
                .debug_struct("ClearPluginSetting")
                .field("plugin", plugin)
                .field("key", key)
                .finish(),
        }
    }
}

/// An optimistic-concurrency patch applied as one durable transaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManagementPatch {
    /// Revision the caller read before constructing this patch.
    pub expected_revision: u64,
    /// Ordered mutations to validate and apply atomically.
    pub mutations: Vec<ManagementMutation>,
}

/// A redacted description of a committed management change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManagementChange {
    /// One or more manageable daemon preferences changed.
    DaemonPreferencesChanged {
        /// Serialized preference names which changed.
        keys: Vec<String>,
    },
    /// A plugin activation override changed.
    PluginActivationChanged {
        /// Canonical plugin name.
        plugin: String,
    },
    /// A plugin reconciliation override changed.
    PluginReconciliationChanged {
        /// Canonical plugin name.
        plugin: String,
    },
    /// A plugin setting changed; its value is never included.
    PluginSettingChanged {
        /// Canonical plugin name.
        plugin: String,
        /// Canonical dotted setting key.
        key: String,
        /// Whether the setting is sensitive.
        sensitive: bool,
    },
}

/// Redacted changes committed at one managed revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagementChangeSet {
    /// Revision created by the transaction.
    pub revision: u64,
    /// Redacted changes in request order.
    pub changes: Vec<ManagementChange>,
}

#[cfg(test)]
#[path = "management_tests.rs"]
mod tests;
