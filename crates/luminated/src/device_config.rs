// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Daemon configuration loading and per-device policy overrides.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use luminate_core::capability::CctEmulation;
use luminate_core::control::ReconciliationPolicy;
use luminate_core::device::DeviceId;
use luminate_core::policy::{LimitCeilings, PrincipalId};
use luminate_platform::default_path::{
    default_plugin_dir_local, default_plugin_dir_system, default_socket_path, default_state_path,
};
use luminate_protocol::UnsupportedPolicy;
use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::authorization::RateLimitKey;
#[cfg(unix)]
use luminate_platform::identity::uid_for_user;
#[cfg(windows)]
use luminate_platform::windows::identity::{account_sid, canonical_sid};

/// Policy for automatically activating plugins discovered in global paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginActivationMode {
    /// Activate only plugins selected explicitly by global or managed policy.
    Explicit,

    /// Automatically activate plugins attached through known local buses.
    HostAttached,

    /// Automatically activate every discovered plugin.
    All,
}

/// Administrator-owned policy for the daemon-managed configuration layer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ManagementConfig {
    /// Managed TOML path. When absent, it is derived from [`DaemonConfig::state_path`].
    pub managed_config_path: Option<PathBuf>,

    /// Managed daemon preference keys which the administrator has locked.
    pub locked_daemon_settings: Vec<String>,
}

/// Administrator-owned plugin activation and lock policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PluginManagementConfig {
    /// Automatic activation policy.
    pub activation: PluginActivationMode,
}

impl Default for PluginManagementConfig {
    fn default() -> Self {
        Self {
            activation: PluginActivationMode::HostAttached,
        }
    }
}

/// Global activation disposition for one plugin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManagedPluginActivation {
    /// Permit managed desired state to select activation.
    #[default]
    Managed,

    /// Force the plugin active.
    Enabled,

    /// Force the plugin inactive.
    Disabled,
}

/// Daemon configuration.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DaemonConfig {
    /// Path to the control socket.
    pub socket_path: PathBuf,

    /// Path to the event socket.
    pub event_socket_path: Option<PathBuf>,

    /// Directory to store daemon state.
    pub state_path: PathBuf,

    /// Directories to search for plugins.
    pub plugin_dirs: Vec<PathBuf>,

    /// Managed-layer location and daemon-setting locks.
    pub management: ManagementConfig,

    /// Global plugin activation policy.
    pub plugin_management: PluginManagementConfig,

    /// Policy used for collection requests that omit `on_unsupported`.
    pub default_unsupported_policy: UnsupportedPolicy,

    /// Optional global reconciliation preference.
    ///
    /// Device-specific and plugin-specific policies take precedence. If no
    /// configured policy applies, reconciliation falls back to the plugin's
    /// recommendation.
    pub reconciliation_policy: Option<ReconciliationPolicy>,

    pub device_reconciliation: HashMap<DeviceId, ReconciliationPolicy>,

    /// Global override for `Cct` emulation on targets without native `Cct`
    /// support.
    ///
    /// `None` preserves each target's advertised
    /// `CapabilitySet::cct_emulation`. A plugin-provided
    /// `CctEmulation::Disabled` always takes precedence, because it indicates
    /// hardware for which colour-temperature emulation is meaningless.
    pub cct_emulation: Option<CctEmulation>,

    pub plugins: Vec<PluginConfig>,

    /// Whether to prefer shared memory over the ordinary pipe for the
    /// daemon → plugin-host leg of a frame stream.
    ///
    /// Enabled by default. Disable this to force every frame stream through
    /// the pipe, for example when troubleshooting or rolling back the
    /// shared-memory transport.
    ///
    /// This affects only the internal transport. The client-visible
    /// `BeginFrameStream` contract is unchanged.
    pub prefer_shm: bool,

    /// Whether to offer the client → daemon shared-memory frame-streaming path
    /// (`BeginShmFrameStream`) to eligible same-user clients.
    ///
    /// Enabled by default. Disable this to require clients to use the ordinary
    /// `BeginFrameStream` path, for example when troubleshooting or rolling
    /// back client-published shared-memory streaming.
    ///
    /// This affects only transport selection, not frame-streaming capability:
    /// clients that cannot use this path continue through `BeginFrameStream`.
    ///
    /// This is intentionally separate from [`Self::prefer_shm`], which controls
    /// the distinct daemon → plugin-host leg.
    pub prefer_client_shm: bool,

    /// Client authorization configuration.
    ///
    /// An absent section selects the built-in `socket-access` policy. Unknown
    /// policy names fail configuration parsing rather than falling back to a
    /// permissive policy.
    pub authorization: AuthorizationConfig,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartialDaemonConfig {
    socket_path: Option<PathBuf>,
    event_socket_path: Option<PathBuf>,
    state_path: Option<PathBuf>,
    plugin_dirs: Option<Vec<PathBuf>>,
    management: Option<ManagementConfig>,
    plugin_management: Option<PluginManagementConfig>,
    default_unsupported_policy: Option<UnsupportedPolicy>,
    reconciliation_policy: Option<ReconciliationPolicy>,
    device_reconciliation: Option<HashMap<DeviceId, ReconciliationPolicy>>,
    cct_emulation: Option<CctEmulation>,
    plugins: Option<Vec<PluginConfig>>,
    prefer_shm: Option<bool>,
    prefer_client_shm: Option<bool>,
    authorization: Option<AuthorizationConfig>,
}

impl<'de> Deserialize<'de> for DaemonConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let partial = PartialDaemonConfig::deserialize(deserializer)?;
        let mut config = Self::try_default().map_err(D::Error::custom)?;

        if let Some(value) = partial.socket_path {
            config.socket_path = value;
        }
        if let Some(value) = partial.event_socket_path {
            config.event_socket_path = Some(value);
        }
        if let Some(value) = partial.state_path {
            config.state_path = value;
        }
        if let Some(value) = partial.plugin_dirs {
            config.plugin_dirs = value;
        }
        if let Some(value) = partial.management {
            config.management = value;
        }
        if let Some(value) = partial.plugin_management {
            config.plugin_management = value;
        }
        if let Some(value) = partial.default_unsupported_policy {
            config.default_unsupported_policy = value;
        }
        if let Some(value) = partial.reconciliation_policy {
            config.reconciliation_policy = Some(value);
        }
        if let Some(value) = partial.device_reconciliation {
            config.device_reconciliation = value;
        }
        if let Some(value) = partial.cct_emulation {
            config.cct_emulation = Some(value);
        }
        if let Some(value) = partial.plugins {
            config.plugins = value;
        }
        if let Some(value) = partial.prefer_shm {
            config.prefer_shm = value;
        }
        if let Some(value) = partial.prefer_client_shm {
            config.prefer_client_shm = value;
        }
        if let Some(value) = partial.authorization {
            config.authorization = value;
        }

        Ok(config)
    }
}

#[cfg(test)]
impl Default for DaemonConfig {
    fn default() -> Self {
        Self::try_default().expect("test host must provide default installation paths")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthorizationConfig {
    pub policy: PolicyKind,

    /// Independent daemon admission ceilings. An absent value retains the
    /// daemon's conservative built-in ceiling for that dimension.
    pub limits: LimitCeilings,

    /// Exact principals which retain policy and authentication recovery when
    /// the active policy is unavailable. Group membership never grants this
    /// privilege.
    pub recovery_principals: BTreeSet<PrincipalId>,

    /// Registered front-end identities. Registration records trust in the
    /// front-end process, not permission to act as an end user.
    pub frontends: Vec<FrontendRegistration>,

    /// Supervised executable authentication providers.
    pub authentication_providers: Vec<AuthenticationProviderConfig>,

    /// Principal admitted to service-mode named pipes on Windows. Console
    /// mode remains owner-only regardless of this setting.
    pub pipe_access: PipeAccessConfig,
}

/// Configuration for one supervised executable authentication provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthenticationProviderConfig {
    /// Public name selected by clients.
    pub name: String,

    /// Canonical authority assigned to authenticated subjects.
    pub authority: String,

    /// Absolute path to the provider executable.
    pub executable: PathBuf,

    /// Private file containing the opaque provider initialization secret.
    pub initialization_file: PathBuf,

    /// Whether startup failure prevents daemon startup.
    pub required: bool,

    /// Maximum simultaneously leased sessions for this provider.
    pub max_sessions: usize,
}

impl Default for AuthenticationProviderConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            authority: String::new(),
            executable: PathBuf::new(),
            initialization_file: PathBuf::new(),
            required: true,
            max_sessions: 64,
        }
    }
}

/// A front-end process identity admitted to daemon delegation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "platform", rename_all = "kebab-case", deny_unknown_fields)]
pub enum FrontendRegistration {
    /// Unix account owning the front-end process.
    Unix {
        /// Fixed numeric user ID, mutually exclusive with `user`.
        uid: Option<u32>,

        /// Account name resolved to a user ID at daemon startup.
        user: Option<String>,
    },
    /// Windows account identity owning the front-end process.
    Windows {
        /// Canonical SID, mutually exclusive with `account`.
        sid: Option<String>,

        /// Account name resolved to a canonical SID at daemon startup.
        account: Option<String>,
    },
}

impl FrontendRegistration {
    /// Resolves this host platform's registration to its canonical actor key.
    /// Registrations for another platform are retained in portable
    /// configuration but do not require that platform's account database.
    pub fn resolve(&self) -> anyhow::Result<Option<RateLimitKey>> {
        match self {
            Self::Unix { uid, user } => {
                #[cfg(unix)]
                {
                    let uid = match (uid, user) {
                        (Some(uid), None) => *uid,
                        (None, Some(user)) => uid_for_user(user)?
                            .ok_or_else(|| anyhow::anyhow!("Unix account {user} does not exist"))?,
                        _ => anyhow::bail!("invalid Unix front-end registration"),
                    };
                    Ok(Some(RateLimitKey::Uid(uid)))
                }
                #[cfg(not(unix))]
                {
                    let _ = (uid, user);
                    Ok(None)
                }
            }
            Self::Windows { sid, account } => {
                #[cfg(windows)]
                {
                    let sid = match (sid, account) {
                        (Some(sid), None) => canonical_sid(sid)?,
                        (None, Some(account)) => account_sid(account)?,
                        _ => anyhow::bail!("invalid Windows front-end registration"),
                    };
                    Ok(Some(RateLimitKey::Sid(sid)))
                }
                #[cfg(not(windows))]
                {
                    let _ = (sid, account);
                    Ok(None)
                }
            }
        }
    }
}

fn validate_frontends(frontends: &[FrontendRegistration]) -> anyhow::Result<()> {
    for (index, frontend) in frontends.iter().enumerate() {
        match frontend {
            FrontendRegistration::Unix { uid, user } => {
                anyhow::ensure!(
                    uid.is_some() != user.is_some(),
                    "authorization.frontends[{index}] must set exactly one of uid or user"
                );
                if let Some(user) = user {
                    anyhow::ensure!(
                        !user.trim().is_empty(),
                        "authorization.frontends[{index}].user must not be empty"
                    );
                }
            }
            FrontendRegistration::Windows { sid, account } => {
                anyhow::ensure!(
                    sid.is_some() != account.is_some(),
                    "authorization.frontends[{index}] must set exactly one of sid or account"
                );
                let value = sid.as_ref().or(account.as_ref());
                anyhow::ensure!(
                    value.is_some_and(|value| !value.trim().is_empty()),
                    "authorization.frontends[{index}] identity must not be empty"
                );
            }
        }
    }
    Ok(())
}

/// Windows service-mode named-pipe access configuration.
///
/// This is parsed on every platform so configuration can be checked and
/// managed consistently, but it only affects listeners hosted by the Windows
/// service. The local-group posture is deliberately the default: a fresh,
/// empty group admits no ordinary users until an administrator adds them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "principal", rename_all = "kebab-case", deny_unknown_fields)]
pub enum PipeAccessConfig {
    LocalGroup {
        #[serde(default = "default_windows_client_group")]
        group: String,
    },
    #[allow(
        clippy::empty_enum_variants_with_brackets,
        reason = "Serde only applies deny_unknown_fields to this internally tagged variant when it is a struct variant"
    )]
    Interactive {},
}

impl Default for PipeAccessConfig {
    fn default() -> Self {
        Self::LocalGroup {
            group: default_windows_client_group(),
        }
    }
}

fn default_windows_client_group() -> String {
    "Luminate Clients".to_owned()
}

/// Selects the daemon's
/// [`AuthorizationPolicy`](crate::authorization::AuthorizationPolicy).
///
/// Unknown policy names are configuration errors. The daemon never falls
/// back to the permissive built-in policy after an explicit policy
/// selection: absence and explicit failure are deliberately given
/// different meanings.
///
/// `SocketAccess` is represented as the unit string `socket-access`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyKind {
    #[default]
    SocketAccess,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PluginConfig {
    pub name: Option<String>,

    pub path: Option<PathBuf>,

    pub required: bool,

    /// Whether activation remains managed or is forced globally.
    pub activation: ManagedPluginActivation,

    /// Plugin-specific reconciliation policy.
    pub reconciliation: Option<ReconciliationPolicy>,

    /// Dotted setting keys which managed configuration cannot override.
    pub locked_settings: Vec<String>,

    /// Plugin-defined immutable settings delivered during host initialization.
    pub config: toml::Table,
}

impl DaemonConfig {
    pub(crate) fn try_default() -> anyhow::Result<Self> {
        let plugin_local = default_plugin_dir_local()?;
        let plugin_system = default_plugin_dir_system()?;
        let mut plugin_dirs = vec![plugin_local];
        if plugin_dirs.first() != Some(&plugin_system) {
            plugin_dirs.push(plugin_system);
        }

        Ok(Self {
            socket_path: default_socket_path()?,
            event_socket_path: None,
            state_path: default_state_path()?,
            plugin_dirs,
            management: ManagementConfig::default(),
            plugin_management: PluginManagementConfig::default(),
            default_unsupported_policy: UnsupportedPolicy::Skip,
            reconciliation_policy: None,
            device_reconciliation: HashMap::new(),
            cct_emulation: None,
            plugins: Vec::new(),
            prefer_shm: true,
            prefer_client_shm: true,
            authorization: AuthorizationConfig::default(),
        })
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let config: Self = toml::from_str(&contents)
            .with_context(|| format!("failed to parse config file {}", path.display()))?;
        config
            .validate()
            .with_context(|| format!("invalid config file {}", path.display()))?;
        Ok(config)
    }

    /// Returns the configured plugin activation mode.
    #[must_use]
    pub fn plugin_activation_mode(&self) -> PluginActivationMode {
        self.plugin_management.activation
    }

    /// Resolves the managed TOML path from global configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the effective state path has no parent.
    pub fn managed_config_path(&self) -> anyhow::Result<PathBuf> {
        if let Some(path) = &self.management.managed_config_path {
            return Ok(path.clone());
        }

        let parent = self
            .state_path
            .parent()
            .context("state_path must have a parent to derive managed_config_path")?;
        Ok(parent.join("managed.toml"))
    }

    /// Validates that daemon-managed state cannot replace administrator-owned
    /// global configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when both paths identify the same configured path.
    pub fn validate_managed_config_path(&self, global_config_path: &Path) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.managed_config_path()? != global_config_path,
            "management.managed_config_path must differ from the global config path"
        );
        Ok(())
    }

    /// Validates configuration invariants that cannot be expressed through
    /// deserialization alone.
    ///
    /// Rejects empty security-relevant paths (which would otherwise propagate
    /// as `""`) and plugin entries that specify neither a `name` nor a
    /// `path`. The latter would otherwise not be detected until
    /// `resolve_configured_plugin`.
    ///
    /// # Errors
    ///
    /// Returns an error describing the first violated configuration
    /// invariant.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.socket_path.as_os_str().is_empty(),
            "socket_path must not be empty"
        );

        anyhow::ensure!(
            !self.state_path.as_os_str().is_empty(),
            "state_path must not be empty"
        );

        let managed_config_path = self.managed_config_path()?;
        anyhow::ensure!(
            !managed_config_path.as_os_str().is_empty(),
            "management.managed_config_path must not be empty"
        );
        anyhow::ensure!(
            managed_config_path != self.state_path,
            "management.managed_config_path must differ from state_path"
        );

        if let Some(event_socket_path) = &self.event_socket_path {
            anyhow::ensure!(
                !event_socket_path.as_os_str().is_empty(),
                "event_socket_path must not be empty"
            );

            anyhow::ensure!(
                event_socket_path != &self.socket_path,
                "event_socket_path must differ from socket_path"
            );
        }

        if let PipeAccessConfig::LocalGroup { group } = &self.authorization.pipe_access {
            anyhow::ensure!(
                !group.trim().is_empty(),
                "authorization.pipe_access.group must not be empty"
            );
        }

        validate_frontends(&self.authorization.frontends)?;
        validate_limit("connections", self.authorization.limits.connections)?;
        validate_limit("subscriptions", self.authorization.limits.subscriptions)?;

        validate_authentication_providers(&self.authorization.authentication_providers)?;

        let mut daemon_locks = HashSet::new();
        for (index, key) in self.management.locked_daemon_settings.iter().enumerate() {
            anyhow::ensure!(
                valid_managed_daemon_setting(key),
                "management.locked_daemon_settings[{index}] is not a manageable daemon setting"
            );
            anyhow::ensure!(
                daemon_locks.insert(key.as_str()),
                "duplicate managed daemon-setting lock for {key}"
            );
        }

        for (index, plugin) in self.plugins.iter().enumerate() {
            anyhow::ensure!(
                plugin.name.is_some() || plugin.path.is_some(),
                "plugins[{index}] must specify at least one of `name` or `path`"
            );

            anyhow::ensure!(
                !(plugin.required && plugin.activation == ManagedPluginActivation::Disabled),
                "required plugin in plugins[{index}] cannot be globally disabled"
            );

            let mut setting_locks = HashSet::new();
            for (setting_index, key) in plugin.locked_settings.iter().enumerate() {
                anyhow::ensure!(
                    valid_dotted_setting_key(key),
                    "plugins[{index}].locked_settings[{setting_index}] must be a dotted setting key"
                );
                anyhow::ensure!(
                    setting_locks.insert(key.as_str()),
                    "duplicate setting lock in plugins[{index}]: {key}"
                );
            }
        }

        Ok(())
    }
}

fn validate_authentication_providers(
    providers: &[AuthenticationProviderConfig],
) -> anyhow::Result<()> {
    let mut names = HashSet::new();
    let mut authorities = HashSet::new();
    for (index, provider) in providers.iter().enumerate() {
        anyhow::ensure!(
            valid_provider_name(&provider.name),
            "authorization.authentication_providers[{index}].name must be a non-empty ASCII identifier"
        );
        anyhow::ensure!(
            names.insert(provider.name.as_str()),
            "duplicate authentication provider name {}",
            provider.name
        );
        anyhow::ensure!(
            !provider.authority.trim().is_empty(),
            "authorization.authentication_providers[{index}].authority must not be empty"
        );
        anyhow::ensure!(
            authorities.insert(provider.authority.as_str()),
            "duplicate authentication provider authority {}",
            provider.authority
        );
        anyhow::ensure!(
            provider.executable.is_absolute(),
            "authorization.authentication_providers[{index}].executable must be absolute"
        );
        anyhow::ensure!(
            provider.initialization_file.is_absolute(),
            "authorization.authentication_providers[{index}].initialization_file must be absolute"
        );
        anyhow::ensure!(
            (1..=Semaphore::MAX_PERMITS).contains(&provider.max_sessions),
            "authorization.authentication_providers[{index}].max_sessions must be between 1 and {}",
            Semaphore::MAX_PERMITS
        );
    }
    Ok(())
}

fn valid_provider_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn validate_limit<T>(name: &str, value: Option<T>) -> anyhow::Result<()>
where
    T: PartialEq + From<u8>,
{
    if let Some(value) = value {
        anyhow::ensure!(
            value != T::from(0),
            "authorization.limits.{name} must be greater than zero"
        );
    }
    Ok(())
}

fn valid_managed_daemon_setting(key: &str) -> bool {
    matches!(
        key,
        "default_unsupported_policy"
            | "reconciliation_policy"
            | "device_reconciliation"
            | "cct_emulation"
            | "prefer_shm"
            | "prefer_client_shm"
    ) || ["device_reconciliation."].iter().any(|prefix| {
        key.strip_prefix(prefix)
            .is_some_and(|name| !name.is_empty())
    })
}

fn valid_dotted_setting_key(key: &str) -> bool {
    !key.is_empty()
        && key.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        })
}

#[cfg(test)]
#[path = "device_config_tests.rs"]
mod tests;
