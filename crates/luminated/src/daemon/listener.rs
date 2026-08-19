// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Config loading, Unix socket filesystem setup, and the connection accept loop.

use super::connection::{handle_connection, handle_event_subscription};
use super::dispatch::ConnectionDependencies;
use super::executor::MutationExecutor;
use super::topology::run_topology_coordinator;
use super::*;

use std::collections::BTreeMap;

use luminate_core::policy::{
    ManagedAccessPolicy, PolicyDocument, PolicyDocumentSource, PolicyRevision, PolicyStore,
    RuntimeAccessPolicy,
};

use luminate_platform::process_signals::{ReloadSignal, RescanSignal};
use luminate_platform::transport::{Address, Connection, Listener};

use crate::device_config::FrontendRegistration;

pub(super) fn bind_listener(path: &Path, access: &ListenerAccess) -> anyhow::Result<Listener> {
    let address = Address::from_configured_path(path);
    match access {
        ListenerAccess::OwnerOnly => Listener::bind(&address).map_err(Into::into),
        #[cfg(windows)]
        ListenerAccess::Service(access) => {
            Listener::bind_with_pipe_access(&address, access.clone()).map_err(Into::into)
        }
        #[cfg(windows)]
        ListenerAccess::ServiceConfig => {
            anyhow::bail!("Windows service pipe access must be resolved before binding listeners")
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ListenerKind {
    Primary,
    Event,
}

struct AcceptedConnection {
    kind: ListenerKind,
    stream: Connection,
    principal: Principal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionLimitReached {
    Global,
    PerUid,
}

struct ConnectionPermit {
    _global: OwnedSemaphorePermit,
    _per_uid: OwnedSemaphorePermit,
}

struct ConnectionLimiter {
    global: Arc<Semaphore>,
    per_principal: HashMap<RateLimitKey, Arc<Semaphore>>,
    maximum_per_uid: usize,
}

impl ConnectionLimiter {
    fn new(maximum_global: usize, maximum_per_uid: usize) -> Self {
        Self {
            global: Arc::new(Semaphore::new(maximum_global)),
            per_principal: HashMap::new(),
            maximum_per_uid,
        }
    }

    fn try_acquire(
        &mut self,
        key: RateLimitKey,
    ) -> Result<ConnectionPermit, ConnectionLimitReached> {
        let global = Arc::clone(&self.global)
            .try_acquire_owned()
            .map_err(|_| ConnectionLimitReached::Global)?;
        let per_uid = Arc::clone(
            self.per_principal
                .entry(key)
                .or_insert_with(|| Arc::new(Semaphore::new(self.maximum_per_uid))),
        )
        .try_acquire_owned()
        .map_err(|_| ConnectionLimitReached::PerUid)?;

        Ok(ConnectionPermit {
            _global: global,
            _per_uid: per_uid,
        })
    }

    fn prune_idle_uids(&mut self) {
        self.per_principal
            .retain(|_, slots| slots.available_permits() != self.maximum_per_uid);
    }
}

async fn run_accept_worker(
    mut listener: Listener,
    kind: ListenerKind,
    accepted: mpsc::Sender<anyhow::Result<AcceptedConnection>>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, credential)) => {
                let principal = Principal::from(credential);
                if accepted
                    .send(Ok(AcceptedConnection {
                        kind,
                        stream,
                        principal,
                    }))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Err(error) => {
                let _ = accepted
                    .send(Err(anyhow::Error::from(error)
                        .context(format!("{kind:?} socket accept failed"))))
                    .await;
                return;
            }
        }
    }
}

/// Main process accept loop
#[allow(
    clippy::too_many_lines,
    reason = "The daemon loop keeps protocol dispatch and state persistence sequencing together."
)]
#[allow(
    clippy::too_many_arguments,
    reason = "Threads the daemon's fixed startup-assembled dependencies (state, plugins, config, notifications, authorization policy) into one loop; grouping them would just move the same count into a struct."
)]
pub(super) async fn accept_loop(
    listener: Listener,
    event_listener: Listener,
    state: Arc<Mutex<DaemonState>>,
    plugin_manager: Arc<PluginManager>,
    management: Arc<ManagementReadState>,
    state_path: Arc<Path>,
    topology_notifications: mpsc::UnboundedReceiver<TopologyNotification>,
    rescans: RescanRequester,
    policy: Arc<dyn AuthorizationPolicy>,
    administration_audit: Arc<dyn Sink>,
    mut shutdown: ShutdownSource,
    lifecycle: LifecycleReporter,
    operator_events: bool,
    mut power_events: PowerEventReceiver,
) -> anyhow::Result<()> {
    // `SIGUSR1` has an unambiguous meaning on the platforms that have it:
    // re-enumerate hardware. It exists so an init system, a sleep hook, or an
    // operator can drive a rescan on a platform whose native suspend/resume
    // hook Luminate doesn't implement.
    //
    // The control protocol's `Rescan` request is the interface with real
    // feedback; this is the one that works from a shell script.
    let mut rescan_signal = RescanSignal::install()?;

    // `SIGHUP` is the config/plugin-reload story: it restarts every
    // currently loaded plugin at its existing path and configuration,
    // without re-reading `luminated`'s own configuration file for added or
    // removed plugin entries. The control protocol's `ReloadPlugin` request
    // reloads a single named plugin and gives the caller real feedback;
    // this is the one that works from a shell script or init system.
    let mut reload_signal = ReloadSignal::install()?;
    let mut connections = JoinSet::new();
    let mut subscriptions = JoinSet::new();
    let connection_limit = management.effective().authorization.limits.connections;
    let subscription_limit = management.effective().authorization.limits.subscriptions;
    let maximum_connections = connection_limit
        .and_then(|limit| usize::try_from(limit).ok())
        .unwrap_or(MAX_CONNECTIONS);
    let maximum_subscribers = subscription_limit
        .and_then(|limit| usize::try_from(limit).ok())
        .unwrap_or(MAX_EVENT_SUBSCRIBERS);
    let mut connection_limiter =
        ConnectionLimiter::new(maximum_connections, MAX_CONNECTIONS_PER_UID);
    let subscription_slots = Arc::new(Semaphore::new(maximum_subscribers));
    let (event_sender, _) = broadcast::channel(EVENT_CHANNEL_DEPTH);
    let events = EventPublisher::new(
        event_sender,
        state.lock().await.topology_generation_counter(),
    );
    let token_path = state_path.parent().map_or_else(
        || PathBuf::from("tokens.json"),
        |parent| parent.join("tokens.json"),
    );
    let authentication = AuthenticationService::load(token_path)?;
    authentication
        .start_providers(
            &management
                .effective()
                .authorization
                .authentication_providers,
        )
        .await?;
    let policy_path = state_path.parent().map_or_else(
        || PathBuf::from("policy.json"),
        |parent| parent.join("policy.json"),
    );
    let empty_source = PolicyDocumentSource {
        revision: PolicyRevision(0),
        roles: BTreeMap::new(),
        bindings: Vec::new(),
    };
    let fallback = PolicyDocument::new(empty_source.clone())?;
    let recovery = PolicyDocument::new(empty_source)?;
    let store: Arc<dyn PolicyStore> =
        Arc::new(PolicyFileStore::with_fallback(policy_path, fallback));
    let access_policy: Arc<dyn ManagedAccessPolicy> =
        Arc::new(RuntimeAccessPolicy::load(store, recovery).await?);
    let frontend_actors = management
        .effective()
        .authorization
        .frontends
        .iter()
        .map(FrontendRegistration::resolve)
        .collect::<anyhow::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    let access_administration = Arc::new(dispatch::AccessAdministration {
        policy: access_policy,
        authentication: authentication.clone(),
        audit: administration_audit,
        frontend_actors,
    });
    let (accepted_tx, mut accepted_rx) = mpsc::channel(2);
    let mut accept_workers = JoinSet::new();
    accept_workers.spawn(run_accept_worker(
        listener,
        ListenerKind::Primary,
        accepted_tx.clone(),
    ));
    accept_workers.spawn(run_accept_worker(
        event_listener,
        ListenerKind::Event,
        accepted_tx,
    ));
    lifecycle.report(LifecycleEvent::Ready);

    // Different plugin hosts may make progress concurrently. Each hosted
    // plugin serializes its own calls, while state is locked only for short
    // validation/commit windows and persistence snapshots.
    let mutations = MutationExecutor {
        state: Arc::clone(&state),
        state_path: Arc::clone(&state_path),
        commit: Arc::new(sync::Mutex::new(())),
        device_sequencers: Arc::new(sync::Mutex::new(HashMap::new())),
        events: events.clone(),
        operator_events,
        transitions: Arc::new(transition::TransitionRegistry::default()),
    };

    let topology_coordinator = tokio::spawn(run_topology_coordinator(
        topology_notifications,
        mutations.clone(),
        Arc::clone(&plugin_manager),
        Arc::clone(&management),
        events.clone(),
    ));
    let mut authentication_maintenance = time::interval(Duration::from_secs(30));
    authentication_maintenance.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = authentication_maintenance.tick() => authentication.prune_expired(),
            accepted = accepted_rx.recv() => {
                match accepted {
                    Some(Ok(AcceptedConnection { kind: ListenerKind::Primary, stream, principal })) => {
                        let permit = match connection_limiter.try_acquire(principal.rate_limit_key()) {
                            Ok(permit) => permit,
                            Err(ConnectionLimitReached::Global) => {
                                    tracing::warn!(
                                        max_connections = maximum_connections,
                                        "global connection limit reached; rejecting client"
                                    );
                                    drop(stream);
                                    continue;
                            }
                            Err(ConnectionLimitReached::PerUid) => {
                                    tracing::warn!(
                                        rate_limit_key = ?principal.rate_limit_key(),
                                        max_connections_per_uid = MAX_CONNECTIONS_PER_UID,
                                        "per-UID connection limit reached; rejecting client"
                                    );
                                    drop(stream);
                                    continue;
                            }
                        };

                        let dependencies = ConnectionDependencies {
                            state: Arc::clone(&state),
                            plugin_manager: Arc::clone(&plugin_manager),
                            management: Arc::clone(&management),
                            mutations: mutations.clone(),
                            rescans: rescans.clone(),
                            principal,
                            policy: Arc::clone(&policy),
                            authentication: authentication.clone(),
                            access_administration: Some(Arc::clone(&access_administration)),
                        };
                        connections.spawn(async move {
                            let _permit = permit;

                            if let Err(error) = handle_connection(stream, dependencies).await {
                                tracing::warn!(error = %error, "connection handling failed");
                            }
                        });
                    }
                    Some(Ok(AcceptedConnection { kind: ListenerKind::Event, stream, principal })) => {
                        let Ok(permit) = Arc::clone(&subscription_slots).try_acquire_owned() else {
                            tracing::warn!(
                                max_subscribers = maximum_subscribers,
                                "event subscriber limit reached; rejecting client"
                            );
                            drop(stream);
                            continue;
                        };
                        let events = events.clone();
                        let subscription_state = Arc::clone(&state);
                        let subscription_plugin_manager = Arc::clone(&plugin_manager);
                        let subscription_policy = Arc::clone(&policy);
                        let subscription_authentication = authentication.clone();
                        let subscription_access_administration = Arc::clone(&access_administration);
                        subscriptions.spawn(async move {
                            let _permit = permit;
                            if let Err(error) = handle_event_subscription(
                                stream,
                                events,
                                subscription_state,
                                subscription_plugin_manager,
                                subscription_policy,
                                principal,
                                subscription_authentication,
                                Some(subscription_access_administration),
                            )
                            .await
                            {
                                tracing::debug!(error = %error, "event subscription ended");
                            }
                        });
                    }
                    Some(Err(error)) => return Err(error),
                    None => anyhow::bail!("socket accept workers stopped unexpectedly"),
                }
            }
            result = connections.join_next(), if !connections.is_empty() => {
                connection_limiter.prune_idle_uids();
                match result {
                    Some(Ok(())) => {}
                    Some(Err(error)) if error.is_panic() => {
                        tracing::error!(
                            error = %error,
                            "connection task panicked"
                        );
                    }
                    Some(Err(error)) => {
                        tracing::debug!(
                            error = %error,
                            "connection task was cancelled"
                        );
                    }
                    None => unreachable!("JoinSet was non-empty"),
                }
            }
            result = subscriptions.join_next(), if !subscriptions.is_empty() => {
                if let Some(Err(error)) = result {
                    tracing::debug!(error = %error, "event subscription task ended");
                }
            }
            signal = shutdown.recv() => {
                let signal = signal?;
                tracing::info!(signal, "received shutdown signal, shutting down gracefully");
                break;
            }
            () = rescan_signal.recv() => {
                tracing::info!("received SIGUSR1, rescanning hardware");
                if !rescans.request(RescanReason::Operator) {
                    tracing::warn!("rescan request dropped: topology coordinator is gone");
                }
            }
            () = reload_signal.recv() => {
                tracing::info!("received SIGHUP, reloading every loaded plugin");
                // Reloading a plugin host takes real wall-clock time (spawning
                // and probing a fresh process), so this is handed off to a
                // background task rather than awaited inline here, the same
                // way the topology coordinator's own rescans never block this
                // loop from servicing new connections or signals meanwhile.
                let manager = Arc::clone(&plugin_manager);
                let reload_state = Arc::clone(&state);
                let reload_events = events.clone();
                let reload_rescans = rescans.clone();
                tokio::spawn(async move {
                    let results = match task::spawn_blocking(move || manager.reload_all()).await {
                        Ok(results) => results,
                        Err(error) => {
                            tracing::error!(error = %error, "SIGHUP plugin reload task panicked");
                            return;
                        }
                    };
                    let mut any_reloaded = false;
                    for (_name, result) in results {
                        // Failures are already logged inside `reload_all`.
                        if let Ok(reconciled) = result {
                            any_reloaded = true;
                            if !reconciled.changed_devices.is_empty() {
                                reload_state
                                    .lock()
                                    .await
                                    .replace_devices_preserving_withdrawn_state(reconciled.devices);
                                let _ = reload_events.send(Event::TopologyChanged {
                                    devices: reconciled.changed_devices,
                                });
                            }
                        }
                    }
                    if any_reloaded && !reload_rescans.request(RescanReason::Operator) {
                        tracing::warn!(
                            "plugins reloaded but the follow-up rescan could not be scheduled: the daemon is shutting down"
                        );
                    }
                });
            }
            // Handle suspend inline rather than spawning another task. Quiescing must
            // complete before `SuspendLease` is released, because releasing the lease
            // drops logind's delay inhibitor and allows suspend to proceed. Awaiting
            // here makes that ordering structural rather than a race between tasks.
            Some(event) = power_events.recv() => {
                match event {
                    SystemPowerEvent::Suspending(lease) => {
                        tracing::info!("system is suspending; quiescing");
                        suspend::quiesce_for_suspend(&mutations, &plugin_manager).await;
                        // Release the suspend inhibitor explicitly: this is the point
                        // where the daemon tells the system that quiescing is complete
                        // and suspend may continue.
                        //
                        // On platforms without an inhibitor to release, `SuspendLease`
                        // has no `Drop` implementation, thus this becomes a no-op.
                        // Keep the call unconditional so the release point remains
                        // explicit and identical across platforms.
                        #[allow(
                            clippy::drop_non_drop,
                            reason = "SuspendLease has no Drop impl on platforms with nothing to \
                                      release; the call is still meaningful and documents the \
                                      release point on platforms where it does."
                        )]
                        drop(lease);
                    }
                    SystemPowerEvent::Resumed => {
                        suspend::reconcile_after_resume(&rescans);
                    }
                    SystemPowerEvent::DevicesChanged => {
                        if !rescans.request(RescanReason::DeviceChange) {
                            tracing::warn!("device-change rescan dropped: coordinator is gone");
                        }
                    }
                }
            }
        }
    }

    // Stop accepting new connections, then give in-flight ones a bounded
    // window to finish. Each mutation already persists on success
    // (`mutate`), so there is no separate "final save" step needed here.
    accept_workers.abort_all();
    while accept_workers.join_next().await.is_some() {}
    topology_coordinator.abort();
    let _ = topology_coordinator.await;
    subscriptions.abort_all();
    while subscriptions.join_next().await.is_some() {}

    // Reap outstanding tasks that may be pending. Isolated host calls have
    // their own deadlines, so blocking mutation tasks have a finite lifetime.
    let cleanup_result = cleanup_connection_tasks(connections).await;
    authentication.shutdown_providers().await;
    cleanup_result?;

    Ok(())
}

/// Clean up all outstanding connection tasks in the given `JoinSet`.
///
/// Gives in-flight connections a bounded window to finish on their own. If that
/// window elapses, abort the stragglers and reap them under a second bound.
/// The abort reap is itself bounded because a task wedged in uninterruptible
/// work may never observe cancellation, so shutdown must not wait on it forever.
pub(super) async fn cleanup_connection_tasks(mut connections: JoinSet<()>) -> anyhow::Result<()> {
    cleanup_connection_tasks_with_timeout(&mut connections, SHUTDOWN_DRAIN_TIMEOUT).await
}

async fn cleanup_connection_tasks_with_timeout(
    connections: &mut JoinSet<()>,
    drain_timeout: Duration,
) -> anyhow::Result<()> {
    tracing::debug!(
        pending = connections.len(),
        "draining in-flight connections"
    );

    let drained_cleanly = timeout(drain_timeout, async {
        while let Some(result) = connections.join_next().await {
            if let Err(error) = result {
                tracing::warn!(
                    error = %error,
                    "connection task failed during shutdown"
                );
            }
        }
    })
    .await
    .is_ok();

    if drained_cleanly {
        return Ok(());
    }

    tracing::warn!(
        pending = connections.len(),
        "shutdown drain timed out; aborting remaining connection tasks"
    );
    connections.abort_all();

    if timeout(drain_timeout, async {
        while connections.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        tracing::warn!(
            pending = connections.len(),
            "aborted connection tasks did not stop within shutdown bound; exiting anyway"
        );
    }

    Ok(())
}

pub(super) fn load_config() -> anyhow::Result<DaemonConfig> {
    let explicit_config_path = env::var_os(CONFIG_PATH_ENV).map(PathBuf::from);
    let global_config_path = if let Some(path) = &explicit_config_path {
        path.clone()
    } else {
        default_config_path().context("failed to resolve the default configuration path")?
    };
    let mut config = if let Some(path) = explicit_config_path {
        // An operator who explicitly points `LUMINATED_CONFIG` at a file did so
        // to control daemon behaviour. Failing open to built-in defaults when
        // that file is missing or unreadable could silently change plugin
        // activation policy, so a missing/broken explicit config is a hard
        // error rather than a fall-back.
        DaemonConfig::load(&path).with_context(|| {
            format!(
                "{CONFIG_PATH_ENV} was set to {}, but that config could not be loaded",
                path.display()
            )
        })?
    } else {
        // Only the implicit compiled-in default path may be absent: a fresh
        // install or dev run legitimately has no file there, so we fall back to
        // defaults. A file that exists but fails to parse/validate still errors.
        if global_config_path.exists() {
            DaemonConfig::load(&global_config_path)?
        } else {
            DaemonConfig::try_default()?
        }
    };

    if let Some(socket_path) = env::var_os(SOCKET_PATH_ENV) {
        config.socket_path = socket_path.into();
    }

    if let Some(event_socket_path) = env::var_os(EVENT_SOCKET_PATH_ENV) {
        config.event_socket_path = Some(event_socket_path.into());
    }

    if let Some(state_path) = env::var_os(STATE_PATH_ENV) {
        config.state_path = state_path.into();
    }

    // Re-validate: the env overrides above can introduce empty paths that never
    // went through `DaemonConfig::load`'s validation.
    config.validate()?;
    config.validate_managed_config_path(&global_config_path)?;

    Ok(config)
}

#[cfg(test)]
#[path = "listener_tests.rs"]
mod tests;
