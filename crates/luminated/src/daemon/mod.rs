// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Daemon lifecycle, socket serving, reconciliation, and request dispatch.
//!
//! This is the composition root: it owns startup sequencing (`run`) and the
//! shared imports/constants used across the concern-specific submodules below.

mod authz;
mod connection;
mod dispatch;
mod executor;
mod listener;
mod reconciliation;
mod suspend;
#[cfg(test)]
#[path = "tests/support.rs"]
mod tests_support;
mod topology;
mod transition;

use luminate_core::device;
use luminate_core::state;
use luminate_core::state::AppearanceState;
use luminate_core::state::EmissionState;
use luminate_core::state::FacetValue;
use luminate_core::state::PhysicalPowerState;
use std::future::Future;
use std::mem;
use std::panic;
use std::slice;
use tokio::task;
use tokio::time;

use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{self, Arc, RwLock};
use std::time::Duration;
use std::{env, io};

use anyhow::Context as _;
#[cfg(all(test, unix))]
use tokio::net::UnixStream;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore, broadcast, mpsc};
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::authentication::AuthenticationService;
#[cfg(test)]
use luminate_core::capability::CctEmulation;
use luminate_core::capability::{PersistenceCapability, PersistenceRequirement};
use luminate_core::collection::{CollectionId, OwnerIdentity};
use luminate_core::control::ReconciliationPolicy;
use luminate_core::effect::Effect;
use luminate_core::target::TargetId;
use luminate_protocol::framing::{self, FramingError};
use luminate_protocol::{
    AuthenticationRequest, AuthenticationResponse, ClientHello, Compatibility, DaemonHello,
    ErrorCode, Event, EventCompatibility, OperationError, Request, RequestMessage, Response,
    ResponseMessage, ResponseStatus, Selector, ServerInfo, SubscribeAck, SubscribeHello,
    UnsupportedPolicy,
};
use luminate_protocol::{EVENT_PROTOCOL_VERSION, PROTOCOL_ABI_VERSION};

#[derive(Clone, Debug)]
struct PublishedEvent {
    event: Event,
    topology_generation: u64,
}

#[cfg(test)]
impl From<Event> for PublishedEvent {
    fn from(event: Event) -> Self {
        Self {
            event,
            topology_generation: 0,
        }
    }
}

#[cfg(test)]
impl PartialEq<Event> for PublishedEvent {
    fn eq(&self, other: &Event) -> bool {
        &self.event == other
    }
}

#[derive(Clone)]
struct EventPublisher {
    sender: broadcast::Sender<PublishedEvent>,
    topology_generation: Arc<sync::atomic::AtomicU64>,
}

impl EventPublisher {
    fn new(
        sender: broadcast::Sender<PublishedEvent>,
        topology_generation: Arc<sync::atomic::AtomicU64>,
    ) -> Self {
        Self {
            sender,
            topology_generation,
        }
    }

    fn send(&self, event: Event) -> Result<usize, broadcast::error::SendError<Event>> {
        let topology_generation = self
            .topology_generation
            .load(sync::atomic::Ordering::Acquire);
        self.sender
            .send(PublishedEvent {
                event,
                topology_generation,
            })
            .map_err(|error| broadcast::error::SendError(error.0.event))
    }

    fn subscribe(&self) -> broadcast::Receiver<PublishedEvent> {
        self.sender.subscribe()
    }

    #[cfg(test)]
    fn test_channel(
        state: &Arc<Mutex<DaemonState>>,
        capacity: usize,
    ) -> (Self, broadcast::Receiver<PublishedEvent>) {
        let topology_generation = state
            .try_lock()
            .expect("test state lock should be available")
            .topology_generation_counter();
        let (sender, receiver) = broadcast::channel(capacity);
        (Self::new(sender, topology_generation), receiver)
    }
}

#[cfg(test)]
use crate::audit::NullSink;
use crate::audit::{AuditedPolicy, JsonLinesSink, Sink, UnavailableSink};
use crate::authorization::{
    AuthorizationPolicy, Decision, IntrinsicRecoveryPolicy, Operation, Principal, RateLimitKey,
    Resource, SocketAccessPolicy, operation_for,
};
#[cfg(windows)]
use crate::device_config::PipeAccessConfig;
use crate::device_config::{DaemonConfig, PolicyKind};
use crate::error::DaemonError;
use crate::managed_config;
use crate::operator_event::OperatorEvent;
use crate::persistence;
use crate::plugins::{
    PluginManager, RescanRequester, TopologyNotification, install_topology_notification_sender,
};
use crate::policy_persistence::PolicyFileStore;
use crate::state::DaemonState;
use crate::state::target_state::TargetState;
#[cfg(test)]
use crate::state::target_state::TargetStateEntry;
use luminate_platform::default_path::{default_config_path, event_socket_path};
use luminate_platform::power::{PowerEventReceiver, SystemPowerEvent, start_sources};
use luminate_platform::process_signals::ShutdownSignals;
#[cfg(windows)]
use luminate_platform::transport::PipeAccess;
use luminate_plugin_api::RescanReason;

use listener::{accept_loop, bind_listener, load_config};
use reconciliation::run_startup_reconciliation;

const CONFIG_PATH_ENV: &str = "LUMINATED_CONFIG";
const SOCKET_PATH_ENV: &str = "LUMINATED_SOCKET_PATH";
const EVENT_SOCKET_PATH_ENV: &str = "LUMINATED_EVENT_SOCKET_PATH";
const STATE_PATH_ENV: &str = "LUMINATED_STATE_PATH";
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
/// Maximum time an established connection may remain idle.
///
/// This is intentionally much longer than a normal polling interval. It
/// reclaims connection slots from clients that complete the handshake and
/// then disappear. The timer resets after every request, so it does not limit
/// total connection lifetime.
const IDLE_CONNECTION_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_CONNECTIONS: usize = 128;
/// Per-peer share of the primary connection pool. This prevents one local UID
/// from monopolizing every global slot while leaving generous room for clients
/// that intentionally maintain a small connection pool.
const MAX_CONNECTIONS_PER_UID: usize = 16;
const MAX_EVENT_SUBSCRIBERS: usize = 128;
const EVENT_CHANNEL_DEPTH: usize = 128;
const TOPOLOGY_DEBOUNCE: Duration = Duration::from_millis(300);

/// Resolves the daemon's configured state path for offline administrative
/// commands without starting listeners or hardware plugins.
pub(crate) fn configured_state_path() -> anyhow::Result<PathBuf> {
    Ok(load_config()?.state_path)
}

/// Global policy and serialized mutable state for the management surface.
#[derive(Debug)]
struct ManagementReadState {
    global: DaemonConfig,
    managed_path: PathBuf,
    managed: Mutex<managed_config::ManagedConfig>,
    effective: RwLock<DaemonConfig>,
}

impl ManagementReadState {
    fn effective(&self) -> DaemonConfig {
        self.effective.read().expect("lock poisoned").clone()
    }

    fn replace_effective(&self, config: DaemonConfig) {
        *self.effective.write().expect("lock poisoned") = config;
    }
}

/// Sources a request to stop the daemon gracefully.
pub(crate) struct RunContext {
    shutdown: ShutdownSource,
    lifecycle: LifecycleReporter,
    listener_access: ListenerAccess,
    operator_events: bool,
    power_events: PowerEventSource,
}

#[derive(Clone)]
enum ListenerAccess {
    OwnerOnly,
    #[cfg(windows)]
    ServiceConfig,
    #[cfg(windows)]
    Service(PipeAccess),
}

/// Where `run()` gets its [`SystemPowerEvent`] stream from.
///
/// Most contexts ask the platform for whatever suspend/resume/device-change
/// source it can reach on its own (`start_sources`, self-driving). A Windows
/// service has no such source to reach: `SERVICE_CONTROL_POWEREVENT` only
/// ever arrives at the service control handler, so its events must be handed
/// in from outside rather than discovered.
enum PowerEventSource {
    Platform,
    #[cfg_attr(
        not(windows),
        allow(dead_code, reason = "only ever constructed under cfg(windows)")
    )]
    External(PowerEventReceiver),
}

impl RunContext {
    /// Builds the ordinary foreground context from the platform's process
    /// signals.
    pub(crate) fn console() -> anyhow::Result<Self> {
        Ok(Self {
            shutdown: ShutdownSource::ProcessSignals(ShutdownSignals::install()?),
            lifecycle: LifecycleReporter::discarding(),
            listener_access: ListenerAccess::OwnerOnly,
            operator_events: false,
            power_events: PowerEventSource::Platform,
        })
    }

    /// Builds a context driven by an external lifecycle controller.
    #[allow(dead_code, reason = "A focused constructor used by lifecycle tests.")]
    pub(crate) fn requested(shutdown: mpsc::Receiver<()>) -> Self {
        Self {
            shutdown: ShutdownSource::Requested(shutdown),
            lifecycle: LifecycleReporter::discarding(),
            listener_access: ListenerAccess::OwnerOnly,
            operator_events: false,
            power_events: PowerEventSource::Platform,
        }
    }

    #[allow(
        dead_code,
        reason = "A focused constructor used by lifecycle tests; production SCM contexts use `service`."
    )]
    pub(crate) fn reporting(
        shutdown: mpsc::Receiver<()>,
        lifecycle: mpsc::UnboundedSender<LifecycleEvent>,
    ) -> Self {
        Self {
            shutdown: ShutdownSource::Requested(shutdown),
            lifecycle: LifecycleReporter::new(lifecycle),
            listener_access: ListenerAccess::OwnerOnly,
            operator_events: false,
            power_events: PowerEventSource::Platform,
        }
    }

    #[cfg(windows)]
    pub(crate) fn service(
        shutdown: mpsc::Receiver<()>,
        lifecycle: mpsc::UnboundedSender<LifecycleEvent>,
        power_events: PowerEventReceiver,
    ) -> Self {
        Self {
            shutdown: ShutdownSource::Requested(shutdown),
            lifecycle: LifecycleReporter::new(lifecycle),
            listener_access: ListenerAccess::ServiceConfig,
            operator_events: true,
            power_events: PowerEventSource::External(power_events),
        }
    }
}

#[cfg(windows)]
fn service_pipe_access(config: &PipeAccessConfig) -> anyhow::Result<ListenerAccess> {
    use luminate_platform::windows::identity::{
        LUMINATED_SERVICE_ACCOUNT, account_sid, interactive_sid,
    };
    use luminate_platform::windows::security_descriptor::grant_process_query_access;

    let service_sid = account_sid(LUMINATED_SERVICE_ACCOUNT)
        .with_context(|| format!("resolve Windows service account {LUMINATED_SERVICE_ACCOUNT}"))?;
    let client_sid = match config {
        PipeAccessConfig::LocalGroup { group } => account_sid(group)
            .with_context(|| format!("resolve Windows pipe client group {group}"))?,
        PipeAccessConfig::Interactive {} => {
            interactive_sid().context("construct the Windows interactive-user SID")?
        }
    };

    // The pipe DACL alone lets `client_sid` connect; without this, its
    // subsequent attempt to open this process and verify its token (see
    // `windows::transport::authenticate_server`) is refused by Windows'
    // default process DACL, which grants that query right to SYSTEM and
    // Administrators only.
    grant_process_query_access(&client_sid)
        .context("grant the configured pipe client query access to this process")?;

    Ok(ListenerAccess::Service(PipeAccess::Service {
        service_sid,
        client_sid,
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StartupMilestone {
    ConfigurationLoaded,
    PluginsLoaded,
    ListenersBound,
    PersistedStateRestored,
    StartupReconciled,
    AuthorizationReady,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LifecycleEvent {
    StartupProgress(StartupMilestone),
    Ready,
}

#[cfg(test)]
use self::startup_cancellation_tests::TestMilestonePause;

#[derive(Clone)]
enum TestPause {
    None,
    #[cfg(test)]
    Paused(Arc<TestMilestonePause>),
}

#[derive(Clone)]
struct LifecycleReporter {
    events: Option<mpsc::UnboundedSender<LifecycleEvent>>,
    test_pause: TestPause,
}

impl LifecycleReporter {
    fn discarding() -> Self {
        Self {
            events: None,
            test_pause: TestPause::None,
        }
    }

    fn new(events: mpsc::UnboundedSender<LifecycleEvent>) -> Self {
        Self {
            events: Some(events),
            test_pause: TestPause::None,
        }
    }

    fn report(&self, event: LifecycleEvent) {
        if let Some(events) = &self.events {
            // A lifecycle controller disappearing must not prevent the daemon
            // from completing startup or shutting down cleanly.
            let _ = events.send(event);
        }
    }

    #[allow(
        unused_variables,
        reason = "milestone is only consulted by the #[cfg(test)] TestPause::Paused arm below; \
                  outside test builds TestPause::None is the only variant"
    )]
    #[allow(
        clippy::unused_async,
        reason = "this await only happens in the #[cfg(test)] arm; kept async unconditionally so \
                  run() has one call site rather than a cfg-split one"
    )]
    async fn pause_for_test(&self, milestone: StartupMilestone) {
        match &self.test_pause {
            TestPause::None => {}
            #[cfg(test)]
            TestPause::Paused(pause) => pause.pause_if_matching(milestone).await,
        }
    }
}

enum ShutdownSource {
    ProcessSignals(ShutdownSignals),
    #[cfg_attr(
        not(windows),
        allow(
            dead_code,
            reason = "The production lifecycle controller is Windows-only."
        )
    )]
    Requested(mpsc::Receiver<()>),
}

impl ShutdownSource {
    async fn recv(&mut self) -> anyhow::Result<&'static str> {
        match self {
            Self::ProcessSignals(signals) => Ok(signals.recv().await),
            Self::Requested(requests) => requests
                .recv()
                .await
                .map(|()| "requested")
                .context("daemon shutdown request source closed unexpectedly"),
        }
    }

    fn startup_requested(&mut self) -> anyhow::Result<bool> {
        match self {
            Self::ProcessSignals(_) => Ok(false),
            Self::Requested(requests) => match requests.try_recv() {
                Ok(()) => Ok(true),
                Err(mpsc::error::TryRecvError::Empty) => Ok(false),
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    anyhow::bail!("daemon shutdown request source closed unexpectedly")
                }
            },
        }
    }
}

async fn run_startup_phase<T>(
    shutdown: &mut ShutdownSource,
    phase: impl Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<Option<T>> {
    tokio::select! {
        biased;

        signal = shutdown.recv() => {
            let signal = signal?;
            tracing::info!(signal, "received shutdown request during startup");
            Ok(None)
        }
        result = phase => result.map(Some),
    }
}

fn stop_after_startup_checkpoint(shutdown: &mut ShutdownSource) -> anyhow::Result<bool> {
    if shutdown.startup_requested()? {
        tracing::info!(
            signal = "requested",
            "received shutdown request during startup"
        );
        return Ok(true);
    }

    Ok(false)
}

fn report_operator_failure<T>(
    enabled: bool,
    event: OperatorEvent,
    context: &str,
    result: anyhow::Result<T>,
) -> anyhow::Result<T> {
    result.map_err(|error| {
        if enabled {
            event.emit(&format!("{context}: {error:#}"));
        }
        error
    })
}

#[cfg(test)]
#[path = "operator_failure_tests.rs"]
mod operator_failure_tests;

#[allow(
    clippy::too_many_lines,
    reason = "Startup sequencing (config, sockets, plugins, persisted-state restore, reconciliation) belongs in one ordered function."
)]
pub async fn run(context: RunContext) -> anyhow::Result<()> {
    let RunContext {
        mut shutdown,
        lifecycle,
        listener_access,
        operator_events,
        power_events,
    } = context;
    let global_config = report_operator_failure(
        operator_events,
        OperatorEvent::ConfigurationLoadFailed,
        "failed to load daemon configuration",
        load_config(),
    )?;
    #[cfg(windows)]
    let listener_access = match listener_access {
        ListenerAccess::ServiceConfig => {
            service_pipe_access(&global_config.authorization.pipe_access)?
        }
        ListenerAccess::OwnerOnly => ListenerAccess::OwnerOnly,
        ListenerAccess::Service(access) => ListenerAccess::Service(access),
    };
    lifecycle.report(LifecycleEvent::StartupProgress(
        StartupMilestone::ConfigurationLoaded,
    ));
    lifecycle
        .pause_for_test(StartupMilestone::ConfigurationLoaded)
        .await;
    if stop_after_startup_checkpoint(&mut shutdown)? {
        return Ok(());
    }
    let managed_config_path = global_config.managed_config_path()?;
    let managed_config = managed_config::load(&managed_config_path)?;
    let config = managed_config::merge_daemon_preferences(&global_config, &managed_config.daemon);
    let management = Arc::new(ManagementReadState {
        global: global_config.clone(),
        managed_path: managed_config_path,
        managed: Mutex::new(managed_config.clone()),
        effective: RwLock::new(config.clone()),
    });
    let socket_path = config.socket_path.clone();
    let event_socket_path = config
        .event_socket_path
        .clone()
        .unwrap_or_else(|| event_socket_path(&socket_path));
    let state_path: Arc<Path> = Arc::from(config.state_path.clone().into_boxed_path());
    anyhow::ensure!(
        socket_path != event_socket_path,
        "event socket path must differ from primary socket path"
    );
    let (topology_notifications_tx, topology_notifications_rx) = mpsc::unbounded_channel();
    let rescans = RescanRequester::new(topology_notifications_tx.clone());
    install_topology_notification_sender(topology_notifications_tx);
    let plugin_manager = Arc::new(PluginManager::load(
        &config,
        &managed_config,
        operator_events,
    )?);
    let loaded_plugins = plugin_manager.loaded_metadata();
    let plugin_descriptors = plugin_manager.device_descriptors();
    lifecycle.report(LifecycleEvent::StartupProgress(
        StartupMilestone::PluginsLoaded,
    ));
    lifecycle
        .pause_for_test(StartupMilestone::PluginsLoaded)
        .await;
    if stop_after_startup_checkpoint(&mut shutdown)? {
        return Ok(());
    }

    let listener = report_operator_failure(
        operator_events,
        OperatorEvent::ListenerBindFailed,
        "failed to bind the primary client listener",
        bind_listener(&socket_path, &listener_access),
    )?;
    let event_listener = report_operator_failure(
        operator_events,
        OperatorEvent::ListenerBindFailed,
        "failed to bind the event client listener",
        bind_listener(&event_socket_path, &listener_access),
    )?;
    lifecycle.report(LifecycleEvent::StartupProgress(
        StartupMilestone::ListenersBound,
    ));
    lifecycle
        .pause_for_test(StartupMilestone::ListenersBound)
        .await;
    if stop_after_startup_checkpoint(&mut shutdown)? {
        return Ok(());
    }
    let mut state = DaemonState::from_descriptors(&plugin_descriptors)?;
    state.set_cct_emulation_override(config.cct_emulation);

    let persisted = report_operator_failure(
        operator_events,
        OperatorEvent::PersistedStateLoadFailed,
        "failed to load persisted daemon state",
        persistence::load(&state_path),
    )?;
    if !persisted.entries.is_empty() {
        let (restored, retained_withdrawn, dropped) = state
            .restore_persisted_retaining_withdrawn(persisted.entries, persisted.preserve_order);
        tracing::info!(
            restored,
            retained_withdrawn,
            dropped,
            "restored persisted target state"
        );
    }
    if !persisted.collections.is_empty() {
        let count = persisted.collections.len();
        state.restore_collections(persisted.collections);
        state.restore_scenes(persisted.scenes);
        tracing::info!(count, "restored persisted collections");
    }
    if !persisted.adopted_baseline.is_empty() {
        let count = persisted.adopted_baseline.len();
        state.restore_adopted_baseline(persisted.adopted_baseline);
        tracing::info!(count, "restored adopted physical baseline");
    }
    lifecycle.report(LifecycleEvent::StartupProgress(
        StartupMilestone::PersistedStateRestored,
    ));
    lifecycle
        .pause_for_test(StartupMilestone::PersistedStateRestored)
        .await;
    if stop_after_startup_checkpoint(&mut shutdown)? {
        return Ok(());
    }

    let state = Arc::new(Mutex::new(state));
    let Some(()) = run_startup_phase(
        &mut shutdown,
        run_startup_reconciliation(&state, &plugin_manager, &state_path, &config),
    )
    .await?
    else {
        return Ok(());
    };
    lifecycle.report(LifecycleEvent::StartupProgress(
        StartupMilestone::StartupReconciled,
    ));
    lifecycle
        .pause_for_test(StartupMilestone::StartupReconciled)
        .await;
    if stop_after_startup_checkpoint(&mut shutdown)? {
        return Ok(());
    }

    for plugin in &loaded_plugins {
        tracing::info!(
            plugin = %plugin.name,
            version = %plugin.version,
            priority = plugin.priority,
            probe_outcome = ?plugin.probe_outcome,
            buses = plugin.buses.len(),
            vendors = plugin.vendors.len(),
            probe_hints = plugin.probe_hints.len(),
            path = %plugin.path.display(),
            "plugin ready"
        );
    }

    tracing::info!(
        socket = %socket_path.display(),
        event_socket = %event_socket_path.display(),
        state_path = %state_path.display(),
        plugins = plugin_manager.len(),
        "luminated listening"
    );

    let policy: Arc<dyn AuthorizationPolicy> = match config.authorization.policy {
        PolicyKind::SocketAccess => Arc::new(SocketAccessPolicy),
    };
    tracing::info!(
        policy = policy.name(),
        "authorization admission policy active"
    );
    let audit_path = state_path.parent().map_or_else(
        || PathBuf::from("audit.jsonl"),
        |parent| parent.join("audit.jsonl"),
    );
    let policy_path = state_path.parent().map_or_else(
        || PathBuf::from("policy.json"),
        |parent| parent.join("policy.json"),
    );
    let policy_store = PolicyFileStore::new(policy_path);
    if let Some(document) = policy_store.load_document()? {
        tracing::info!(
            path = %policy_store.path().display(),
            revision = document.revision().0,
            "validated persisted authorization policy"
        );
    }
    let audit: Arc<dyn Sink> = match JsonLinesSink::open(&audit_path) {
        Ok(sink) => Arc::new(sink),
        Err(error) => {
            tracing::error!(error = %error, path = %audit_path.display(), "daemon authorization audit unavailable; continuing without persistence");
            Arc::new(UnavailableSink::new(&error))
        }
    };
    let policy: Arc<dyn AuthorizationPolicy> = Arc::new(AuditedPolicy::new(
        Arc::new(IntrinsicRecoveryPolicy::new(policy)),
        Arc::clone(&audit),
    ));
    lifecycle.report(LifecycleEvent::StartupProgress(
        StartupMilestone::AuthorizationReady,
    ));
    lifecycle
        .pause_for_test(StartupMilestone::AuthorizationReady)
        .await;
    if stop_after_startup_checkpoint(&mut shutdown)? {
        return Ok(());
    }

    // Suspend/resume and device-change sources are best-effort: a machine
    // with no logind, no udev, and no Windows service context gets no automatic
    // events, and the `Rescan` request and `SIGUSR1` still work.
    let power_events = match power_events {
        PowerEventSource::Platform => start_sources(),
        PowerEventSource::External(receiver) => receiver,
    };

    accept_loop(
        listener,
        event_listener,
        state,
        plugin_manager,
        management,
        state_path,
        topology_notifications_rx,
        rescans,
        policy,
        audit,
        shutdown,
        lifecycle,
        operator_events,
        power_events,
    )
    .await?;

    tracing::info!("luminated shutdown complete");
    Ok(())
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;

/// Drives `run()` end to end and proves that a shutdown request injected at
/// each concrete [`StartupMilestone`] stops it there, cleanly, without
/// reaching any later phase. `load_config` reads its configuration and path
/// overrides from process-wide environment variables (see
/// `listener::load_config_honours_explicit_files_and_path_overrides` for the
/// same constraint), so each milestone's run happens in its own child process
/// rather than racing environment mutation against other tests in this
/// binary.
#[cfg(test)]
#[path = "startup_cancellation_tests.rs"]
mod startup_cancellation_tests;
