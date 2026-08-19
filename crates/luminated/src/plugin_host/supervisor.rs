// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Daemon-side plugin-host process supervision and typed request routing.

use std::collections::HashMap;
use std::env;
use std::io::Read;
use std::mem;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result};
use tokio::sync::mpsc::UnboundedSender;
use tracing::level_filters;

use luminate_host_supervisor::sync_io::{read_frame, write_frame};
use luminate_host_supervisor::{Compatibility, HostHello, SupervisorHello};
use luminate_plugin_api::{
    DeviceDescriptor, PluginFrameUpload, PluginLogLevel, PluginReadRequest, PluginStateSnapshot,
    PluginTarget, PluginUpdate, RescanReason,
};

use crate::operator_event::OperatorEvent;
use crate::plugins::TopologyNotification;

use super::protocol::{
    ApplyOutcome, BeginShmStreamRequest, HostBootstrap, HostCommand, HostMessage, HostMetadata,
    HostReady, HostRequest, HostResponse, ShmStreamOutcome,
};
use super::{HOST_LOG_LEVEL_ENV, HOST_MODE_ARGUMENT};

const HOST_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const HOST_LOAD_TIMEOUT: Duration = Duration::from_secs(10);
const HOST_CALL_TIMEOUT: Duration = Duration::from_secs(5);
const PLUGIN_CALLBACK_GRACE: Duration = Duration::from_millis(100);
const HOST_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);

type HostReply = mpsc::SyncSender<Result<HostResponse, String>>;
pub(super) type PendingRequests = Arc<Mutex<HashMap<u64, HostReply>>>;

/// Information about a hosted plugin.
pub struct HostedPlugin {
    /// Path to the plugin.
    pub(super) path: PathBuf,

    /// Expected host metadata for the plugin.
    pub(super) expected_metadata: HostMetadata,

    /// Connection to the host.
    pub(super) connection: Mutex<HostConnection>,

    /// Notifications for topology updates.
    pub(super) notifications: UnboundedSender<TopologyNotification>,

    /// The maximum log level for the plugin.
    pub(super) max_log_level: PluginLogLevel,

    /// Configuration for the plugin.
    pub(super) configuration_cbor: Vec<u8>,

    /// Identifies the current plugin-host connection lifetime.
    ///
    /// Incremented whenever [`Self::call`] respawns the underlying plugin-host
    /// process. Callers may cache this value alongside connection-scoped state
    /// and compare it later to detect that the host has been replaced.
    ///
    /// Currently, the shared-memory publisher registry uses this to invalidate
    /// active streams whose publishers still refer to the previous host
    /// process, without requiring a plugin-host round trip.
    pub(super) connection_epoch: AtomicU64,

    pub(super) operator_events: bool,
}

impl HostedPlugin {
    pub fn spawn_with_configuration(
        path: &Path,
        notifications: UnboundedSender<TopologyNotification>,
        max_log_level: PluginLogLevel,
        configuration_cbor: Vec<u8>,
        operator_events: bool,
    ) -> Result<(Self, HostMetadata, Vec<DeviceDescriptor>)> {
        let (connection, ready) = HostConnection::spawn_with_configuration(
            path,
            notifications.clone(),
            max_log_level,
            &configuration_cbor,
            operator_events,
        )?;
        let metadata = HostMetadata::try_from(ready.metadata)?;
        let plugin = Self {
            path: path.to_path_buf(),
            expected_metadata: metadata.clone(),
            connection: Mutex::new(connection),
            notifications,
            max_log_level,
            configuration_cbor,
            connection_epoch: AtomicU64::new(0),
            operator_events,
        };
        Ok((plugin, metadata, ready.descriptors))
    }

    /// The current connection's epoch; see `connection_epoch`'s field docs.
    #[must_use]
    pub fn connection_epoch(&self) -> u64 {
        self.connection_epoch.load(Ordering::Acquire)
    }

    pub(super) fn call(&self, command: HostCommand) -> Result<HostResponse> {
        #[allow(
            clippy::expect_used,
            clippy::unwrap_in_result,
            reason = "Acceptable to panic on poisoned locks"
        )]
        let mut connection = self
            .connection
            .lock()
            .expect("plugin host connection lock poisoned");
        if !connection.is_alive() {
            self.respawn(&mut connection)?;
            tracing::warn!(
                plugin = %self.expected_metadata.name,
                "plugin host restarted after failure"
            );
        }
        connection.call(command, HOST_CALL_TIMEOUT)
    }

    /// Explicitly terminates the current plugin-host process and starts a
    /// fresh one at the same path and configuration, returning its freshly
    /// probed topology.
    ///
    /// Unlike the crash-triggered respawn in [`Self::call`], this always
    /// tears the current host down first with a graceful `Shutdown` rather
    /// than a kill, even if it's still healthy. This is the primitive
    /// behind an explicit plugin reload.
    ///
    /// # Errors
    ///
    /// Returns an error if the replacement host fails to start, or if it
    /// reports different identity metadata than the plugin being reloaded.
    pub fn force_reload(&self) -> Result<Vec<DeviceDescriptor>> {
        #[allow(
            clippy::expect_used,
            clippy::unwrap_in_result,
            reason = "Acceptable to panic on poisoned locks"
        )]
        let mut connection = self
            .connection
            .lock()
            .expect("plugin host connection lock poisoned");
        connection.shutdown();
        let descriptors = self.respawn(&mut connection)?;
        tracing::info!(plugin = %self.expected_metadata.name, "plugin host reloaded");
        Ok(descriptors)
    }

    /// Spawns a replacement plugin-host connection and swaps it in, verifying
    /// the restarted plugin reports the same identity metadata as before.
    ///
    /// Shared by the lazy respawn-after-crash path in [`Self::call`] and the
    /// explicit [`Self::force_reload`]. Callers are responsible for tearing
    /// down `connection`'s previous process first, if it might still be
    /// running.
    fn respawn(&self, connection: &mut HostConnection) -> Result<Vec<DeviceDescriptor>> {
        let (replacement, ready) = HostConnection::spawn_with_configuration(
            &self.path,
            self.notifications.clone(),
            self.max_log_level,
            &self.configuration_cbor,
            self.operator_events,
        )
        .with_context(|| {
            format!(
                "failed to restart plugin host for {}",
                self.expected_metadata.name
            )
        })?;
        let restarted = HostMetadata::try_from(ready.metadata)?;
        anyhow::ensure!(
            restarted.name == self.expected_metadata.name
                && restarted.version == self.expected_metadata.version
                && restarted.priority == self.expected_metadata.priority,
            "restarted plugin metadata changed for {}",
            self.expected_metadata.name
        );
        *connection = replacement;
        self.connection_epoch.fetch_add(1, Ordering::AcqRel);
        // The ready descriptors are a point-in-time probe, not a topology
        // update we can safely commit here. Ask the coordinator to pull
        // and atomically reconcile the replacement host instead.
        let _ = self
            .notifications
            .send(TopologyNotification::PluginObserved(
                self.expected_metadata.name.clone(),
            ));
        Ok(ready.descriptors)
    }

    pub fn topology(&self) -> Result<Vec<DeviceDescriptor>> {
        match self.call(HostCommand::Topology)? {
            HostResponse::Topology(descriptors) => Ok(descriptors),
            HostResponse::Apply(_)
            | HostResponse::Batch(_)
            | HostResponse::State(_)
            | HostResponse::Frame(_)
            | HostResponse::ShmStream(_)
            | HostResponse::Rescan
            | HostResponse::Shutdown => {
                anyhow::bail!("plugin host returned the wrong topology response")
            }
        }
    }

    /// Asks the plugin to discard any cached view of its hardware, before the
    /// caller re-pulls [`Self::topology`].
    pub fn rescan(&self, reason: RescanReason) -> Result<()> {
        match self.call(HostCommand::Rescan {
            reason: reason.to_abi(),
        })? {
            HostResponse::Rescan => Ok(()),
            HostResponse::Topology(_)
            | HostResponse::Apply(_)
            | HostResponse::Batch(_)
            | HostResponse::State(_)
            | HostResponse::Frame(_)
            | HostResponse::ShmStream(_)
            | HostResponse::Shutdown => {
                anyhow::bail!("plugin host returned the wrong rescan response")
            }
        }
    }

    pub fn apply(&self, update: PluginUpdate) -> Result<ApplyOutcome> {
        match self.call(HostCommand::Apply(update))? {
            HostResponse::Apply(outcome) => Ok(outcome),
            HostResponse::Topology(_)
            | HostResponse::Batch(_)
            | HostResponse::State(_)
            | HostResponse::Frame(_)
            | HostResponse::ShmStream(_)
            | HostResponse::Rescan
            | HostResponse::Shutdown => {
                anyhow::bail!("plugin host returned the wrong update response")
            }
        }
    }

    pub fn apply_batch(&self, updates: Vec<PluginUpdate>) -> Result<Vec<ApplyOutcome>> {
        match self.call(HostCommand::ApplyBatch(updates))? {
            HostResponse::Batch(outcomes) => Ok(outcomes),
            HostResponse::Topology(_)
            | HostResponse::Apply(_)
            | HostResponse::State(_)
            | HostResponse::Frame(_)
            | HostResponse::ShmStream(_)
            | HostResponse::Rescan
            | HostResponse::Shutdown => {
                anyhow::bail!("plugin host returned the wrong batch response")
            }
        }
    }

    pub fn read_state(&self, request: PluginReadRequest) -> Result<PluginStateSnapshot> {
        match self.call(HostCommand::ReadState(request))? {
            HostResponse::State(snapshot) => Ok(snapshot),
            HostResponse::Topology(_)
            | HostResponse::Apply(_)
            | HostResponse::Batch(_)
            | HostResponse::Frame(_)
            | HostResponse::ShmStream(_)
            | HostResponse::Rescan
            | HostResponse::Shutdown => {
                anyhow::bail!("plugin host returned the wrong state response")
            }
        }
    }

    pub fn upload_frame(&self, frame: PluginFrameUpload) -> Result<ApplyOutcome> {
        match self.call(HostCommand::UploadFrame(frame))? {
            HostResponse::Frame(outcome) => Ok(outcome),
            HostResponse::Topology(_)
            | HostResponse::Apply(_)
            | HostResponse::Batch(_)
            | HostResponse::State(_)
            | HostResponse::ShmStream(_)
            | HostResponse::Rescan
            | HostResponse::Shutdown => {
                anyhow::bail!("plugin host returned the wrong frame response")
            }
        }
    }

    /// Requests the shared-memory fast path for `target`. Every rejection
    /// (unsupported, transport failure, etc.) comes back as a typed
    /// [`ShmStreamOutcome`] rather than `Err`, mirroring `begin`'s own
    /// contract in `plugin_host::shm`: the caller falls back to the
    /// ordinary pipe path rather than failing the client-visible request.
    pub fn begin_shm_stream(&self, request: BeginShmStreamRequest) -> Result<ShmStreamOutcome> {
        match self.call(HostCommand::BeginShmStream(request))? {
            HostResponse::ShmStream(outcome) => Ok(outcome),
            HostResponse::Topology(_)
            | HostResponse::Apply(_)
            | HostResponse::Batch(_)
            | HostResponse::State(_)
            | HostResponse::Frame(_)
            | HostResponse::Rescan
            | HostResponse::Shutdown => {
                anyhow::bail!("plugin host returned the wrong shared-memory stream response")
            }
        }
    }

    pub fn end_shm_stream(
        &self,
        target: PluginTarget,
        generation: u32,
    ) -> Result<ShmStreamOutcome> {
        match self.call(HostCommand::EndShmStream { target, generation })? {
            HostResponse::ShmStream(outcome) => Ok(outcome),
            HostResponse::Topology(_)
            | HostResponse::Apply(_)
            | HostResponse::Batch(_)
            | HostResponse::State(_)
            | HostResponse::Frame(_)
            | HostResponse::Rescan
            | HostResponse::Shutdown => {
                anyhow::bail!("plugin host returned the wrong shared-memory stream response")
            }
        }
    }

    /// Bumps a shared-memory stream's generation without disconnecting it.
    /// Not used at the present except for tests, but may be useful in the
    /// future when stream resetting is needed.
    #[allow(
        dead_code,
        reason = "part of the daemon<->plugin-host shared-memory protocol \
                   contract (HostCommand::ResetShmStream); no daemon-side \
                   trigger exists yet"
    )]
    pub fn reset_shm_stream(
        &self,
        target: PluginTarget,
        generation: u32,
    ) -> Result<ShmStreamOutcome> {
        match self.call(HostCommand::ResetShmStream { target, generation })? {
            HostResponse::ShmStream(outcome) => Ok(outcome),
            HostResponse::Topology(_)
            | HostResponse::Apply(_)
            | HostResponse::Batch(_)
            | HostResponse::State(_)
            | HostResponse::Frame(_)
            | HostResponse::Rescan
            | HostResponse::Shutdown => {
                anyhow::bail!("plugin host returned the wrong shared-memory stream response")
            }
        }
    }
}

impl Drop for HostedPlugin {
    fn drop(&mut self) {
        #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
        let connection = self
            .connection
            .get_mut()
            .expect("plugin host connection lock poisoned");
        connection.shutdown();
    }
}

pub(super) struct HostConnection {
    pub(super) child: Arc<Mutex<Child>>,
    pub(super) input: Mutex<ChildStdin>,
    pub(super) pending: PendingRequests,
    pub(super) alive: Arc<AtomicBool>,
    pub(super) exit_reporting: Arc<ExitReporting>,
    pub(super) next_id: u64,
}

pub(super) struct ExitReporting {
    pub(super) expected: AtomicBool,
    pub(super) operator_events: bool,
}

impl ExitReporting {
    pub(super) fn new(operator_events: bool) -> Self {
        Self {
            expected: AtomicBool::new(false),
            operator_events,
        }
    }

    pub(super) fn should_emit(&self) -> bool {
        self.operator_events && !self.expected.load(Ordering::Acquire)
    }
}

impl HostConnection {
    #[allow(
        clippy::too_many_lines,
        reason = "Process setup, handshake, reader startup, and readiness form one ordered initialization sequence"
    )]
    fn spawn_with_configuration(
        path: &Path,
        notifications: UnboundedSender<TopologyNotification>,
        max_log_level: PluginLogLevel,
        configuration_cbor: &[u8],
        operator_events: bool,
    ) -> Result<(Self, HostReady)> {
        let executable = env::current_exe().context("locating luminated executable")?;
        let child_stderr = if cfg!(test) {
            Stdio::null()
        } else {
            Stdio::inherit()
        };
        let mut child = Command::new(executable)
            .arg(HOST_MODE_ARGUMENT)
            .arg(path)
            .env(HOST_LOG_LEVEL_ENV, max_log_level.to_abi().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(child_stderr)
            .spawn()
            .with_context(|| format!("spawning plugin host for {}", path.display()))?;
        let mut input = child
            .stdin
            .take()
            .context("plugin host stdin was not piped")?;
        let output = child
            .stdout
            .take()
            .context("plugin host stdout was not piped")?;
        let child = Arc::new(Mutex::new(child));

        // Belt-and-suspenders version handshake before the real bootstrap
        // frame: parent and child are always the same binary build (both
        // come from `env::current_exe()`), so a mismatch should be
        // impossible. This catches one anyway if that invariant is ever
        // violated (e.g. a packaging bug mixing binary versions), mirroring
        // the policy host's handshake.
        if let Err(error) = write_frame(&mut input, &SupervisorHello::new()) {
            terminate_child(&child);
            return Err(error).context("sending supervisor hello to plugin host");
        }
        let output = match read_handshake_reply(output, HOST_HANDSHAKE_TIMEOUT) {
            Ok((output, hello)) if matches!(hello.compatibility, Compatibility::Compatible) => {
                output
            }
            Ok((_output, hello)) => {
                terminate_child(&child);
                anyhow::bail!(
                    "plugin host protocol incompatible: {:?}",
                    hello.compatibility
                );
            }
            Err(error) => {
                terminate_child(&child);
                return Err(error).context("plugin host handshake failed");
            }
        };

        let pending = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let exit_reporting = Arc::new(ExitReporting::new(operator_events));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);

        if let Err(error) = write_frame(
            &mut input,
            &HostBootstrap {
                configuration_cbor: configuration_cbor.to_vec(),
            },
        ) {
            terminate_child(&child);
            return Err(error).context("sending bootstrap configuration to plugin host");
        }

        let reader_child = Arc::clone(&child);
        let reader_pending = Arc::clone(&pending);
        let reader_alive = Arc::clone(&alive);
        let reader_exit_reporting = Arc::clone(&exit_reporting);
        thread::Builder::new()
            .name("luminate-plugin-host-reader".to_owned())
            .spawn(move || {
                read_host_messages(
                    output,
                    ready_tx,
                    &reader_pending,
                    &reader_alive,
                    &reader_exit_reporting,
                    &reader_child,
                    &notifications,
                );
            })
            .context("spawning plugin-host reader")?;
        let ready = match ready_rx.recv_timeout(HOST_LOAD_TIMEOUT) {
            Ok(Ok(ready)) => ready,
            Ok(Err(error)) => {
                terminate_child(&child);
                anyhow::bail!(error);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                terminate_child(&child);
                anyhow::bail!("plugin host did not initialize within {HOST_LOAD_TIMEOUT:?}");
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                terminate_child(&child);
                anyhow::bail!("plugin host exited before reporting initialization");
            }
        };

        Ok((
            Self {
                child,
                input: Mutex::new(input),
                pending,
                alive,
                exit_reporting,
                next_id: 1,
            },
            ready,
        ))
    }

    pub(super) fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    pub(super) fn call(
        &mut self,
        command: HostCommand,
        call_timeout: Duration,
    ) -> Result<HostResponse> {
        anyhow::ensure!(self.is_alive(), "plugin host is not running");
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        #[allow(
            clippy::expect_used,
            clippy::unwrap_in_result,
            reason = "Acceptable to panic on poisoned locks"
        )]
        self.pending
            .lock()
            .expect("plugin host pending-requests lock poisoned")
            .insert(id, reply_tx);

        let callback_budget = call_timeout.saturating_sub(PLUGIN_CALLBACK_GRACE);
        let timeout_millis = u64::try_from(callback_budget.as_millis()).unwrap_or(u64::MAX);
        let request = HostRequest {
            id,
            timeout_millis,
            command,
        };
        #[allow(
            clippy::expect_used,
            clippy::unwrap_in_result,
            reason = "Acceptable to panic on poisoned locks"
        )]
        let write_result = write_frame(
            &mut *self.input.lock().expect("plugin host input lock poisoned"),
            &request,
        );
        if let Err(error) = write_result {
            #[allow(
                clippy::expect_used,
                clippy::unwrap_in_result,
                reason = "Acceptable to panic on poisoned locks"
            )]
            self.pending
                .lock()
                .expect("plugin host pending-requests lock poisoned")
                .remove(&id);
            self.alive.store(false, Ordering::Release);
            terminate_child(&self.child);
            return Err(error).context("sending request to plugin host");
        }

        match reply_rx.recv_timeout(call_timeout) {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => anyhow::bail!(error),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                #[allow(
                    clippy::expect_used,
                    clippy::unwrap_in_result,
                    reason = "Acceptable to panic on poisoned locks"
                )]
                self.pending
                    .lock()
                    .expect("plugin host pending-requests lock poisoned")
                    .remove(&id);
                self.alive.store(false, Ordering::Release);
                terminate_child(&self.child);
                anyhow::bail!("plugin host request exceeded {call_timeout:?}");
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.alive.store(false, Ordering::Release);
                anyhow::bail!("plugin host exited while processing request");
            }
        }
    }

    pub(super) fn shutdown(&mut self) {
        self.exit_reporting.expected.store(true, Ordering::Release);
        if self.is_alive() {
            let _ = self.call(HostCommand::Shutdown, HOST_SHUTDOWN_TIMEOUT);
        }
        terminate_child(&self.child);
    }
}

pub(super) fn read_host_messages(
    mut output: impl Read,
    ready_tx: mpsc::SyncSender<Result<HostReady, String>>,
    pending: &PendingRequests,
    alive: &Arc<AtomicBool>,
    exit_reporting: &ExitReporting,
    child: &Arc<Mutex<Child>>,
    notifications: &UnboundedSender<TopologyNotification>,
) {
    let mut ready_tx = Some(ready_tx);
    let mut plugin_name = None;
    loop {
        let message = match read_frame::<_, HostMessage>(&mut output) {
            Ok(message) => message,
            Err(error) => {
                let message = format!("plugin host transport ended: {error:#}");
                if let Some(sender) = ready_tx.take() {
                    let _ = sender.send(Err(message.clone()));
                }
                fail_pending(pending, &message);
                break;
            }
        };
        match message {
            HostMessage::Ready(result) => {
                if let Ok(ready) = &result {
                    plugin_name = Some(ready.metadata.name.clone());
                }
                if let Some(sender) = ready_tx.take() {
                    let _ = sender.send(result);
                }
            }
            HostMessage::Response { id, result } => {
                #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
                let removed = pending
                    .lock()
                    .expect("plugin host pending-requests lock poisoned")
                    .remove(&id);
                if let Some(sender) = removed {
                    let _ = sender.send(result);
                }
            }
            HostMessage::TopologyChanged => {
                if let Some(name) = &plugin_name {
                    let _ = notifications.send(TopologyNotification::PluginObserved(name.clone()));
                }
            }
            HostMessage::Log {
                plugin,
                level,
                message,
            } => log_plugin_message(&plugin, level, &message),
        }
    }

    alive.store(false, Ordering::Release);
    #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
    let status = child
        .lock()
        .expect("plugin host child lock poisoned")
        .try_wait();
    tracing::warn!(plugin = ?plugin_name, ?status, "plugin host exited");
    if exit_reporting.should_emit() {
        OperatorEvent::PluginHostCrashed.emit(&format!(
            "plugin host exited unexpectedly (plugin: {plugin_name:?}, status: {status:?})"
        ));
    }
    if let Some(name) = plugin_name {
        let _ = notifications.send(TopologyNotification::PluginObserved(name));
    }
}

pub(super) fn fail_pending(
    pending: &Mutex<HashMap<u64, mpsc::SyncSender<Result<HostResponse, String>>>>,
    message: &str,
) {
    #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
    let pending = mem::take(
        &mut *pending
            .lock()
            .expect("plugin host pending-requests lock poisoned"),
    );
    for sender in pending.into_values() {
        let _ = sender.send(Err(message.to_owned()));
    }
}

pub(super) fn terminate_child(child: &Mutex<Child>) {
    #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
    let mut child = child.lock().expect("plugin host child lock poisoned");
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
}

/// Reads the one-shot [`HostHello`] handshake reply with a bounded wait.
///
/// The blocking pipe read runs on a background thread so a wedged or
/// incompatible child cannot hang plugin-host startup beyond `timeout`.
/// Every caller kills the child if this function fails, which closes the
/// pipe and unblocks that thread after a timeout.
///
/// This runs before `output` is transferred to the long-lived reader
/// thread that demultiplexes [`HostMessage`]s for the remainder of the
/// connection.
fn read_handshake_reply(
    mut output: ChildStdout,
    timeout: Duration,
) -> Result<(ChildStdout, HostHello)> {
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("luminate-plugin-host-handshake".to_owned())
        .spawn(move || {
            let result = read_frame::<_, HostHello>(&mut output);
            let _ = reply_tx.send((output, result));
        })
        .context("spawning plugin-host handshake reader")?;
    match reply_rx.recv_timeout(timeout) {
        Ok((output, Ok(hello))) => Ok((output, hello)),
        Ok((_output, Err(error))) => Err(error).context("reading plugin host hello"),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            anyhow::bail!("plugin host did not complete its handshake within {timeout:?}")
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            anyhow::bail!("plugin host exited before completing its handshake")
        }
    }
}

fn log_plugin_message(plugin: &str, level: u8, message: &str) {
    match PluginLogLevel::from_abi(level).unwrap_or(PluginLogLevel::Warn) {
        PluginLogLevel::Error => tracing::error!(plugin, "{message}"),
        PluginLogLevel::Warn => tracing::warn!(plugin, "{message}"),
        PluginLogLevel::Info => tracing::info!(plugin, "{message}"),
        PluginLogLevel::Debug => tracing::debug!(plugin, "{message}"),
        PluginLogLevel::Trace => tracing::trace!(plugin, "{message}"),
    }
}

pub fn current_max_plugin_log_level() -> PluginLogLevel {
    match level_filters::LevelFilter::current().into_level() {
        Some(tracing::Level::ERROR) | None => PluginLogLevel::Error,
        Some(tracing::Level::WARN) => PluginLogLevel::Warn,
        Some(tracing::Level::INFO) => PluginLogLevel::Info,
        Some(tracing::Level::DEBUG) => PluginLogLevel::Debug,
        Some(tracing::Level::TRACE) => PluginLogLevel::Trace,
    }
}
