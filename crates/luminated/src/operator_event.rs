// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Stable vocabulary for operational events emitted by the daemon.
//!
//! Windows Event Log consumers identify events by source and numeric ID. Keep
//! the IDs here rather than scattering bare numbers through tracing calls.

use luminate_platform::terminal::escape;

/// An operational event with a stable Windows Event Log ID.
///
/// Assigned IDs are compatibility-sensitive and must not be reused. See
/// `docs/development/architecture/windows-event-log.md` for the allocation
/// policy and event catalogue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
#[cfg_attr(
    all(not(test), not(windows)),
    expect(
        dead_code,
        reason = "service lifecycle events are emitted only by the Windows SCM adapter"
    )
)]
pub(crate) enum OperatorEvent {
    /// The Windows service has begun initialization.
    ServiceStarting = 100,

    /// The daemon is ready to accept client connections.
    ServiceReady = 101,

    /// SCM requested an ordinary service stop.
    ServiceStopRequested = 102,

    /// SCM requested a stop during system preshutdown.
    ServicePreshutdownRequested = 103,

    /// The Windows service stopped.
    ServiceStopped = 104,

    /// Detailed service file logging could not be initialized.
    ServiceLoggingDegraded = 105,

    /// The daemon could not load its configuration.
    ConfigurationLoadFailed = 200,

    /// The daemon could not restore persisted state.
    PersistedStateLoadFailed = 201,

    /// The daemon could not save persisted state.
    PersistedStateSaveFailed = 202,

    /// A client transport listener could not be bound.
    ListenerBindFailed = 300,

    /// A supervised plugin host terminated unexpectedly.
    PluginHostCrashed = 400,
}

impl OperatorEvent {
    /// Dedicated tracing target consumed by the Windows Event Log layer.
    pub(crate) const TARGET: &str = "luminated::operator";

    /// All currently assigned events, in numeric order.
    #[cfg(test)]
    pub(crate) const ALL: [Self; 11] = [
        Self::ServiceStarting,
        Self::ServiceReady,
        Self::ServiceStopRequested,
        Self::ServicePreshutdownRequested,
        Self::ServiceStopped,
        Self::ServiceLoggingDegraded,
        Self::ConfigurationLoadFailed,
        Self::PersistedStateLoadFailed,
        Self::PersistedStateSaveFailed,
        Self::ListenerBindFailed,
        Self::PluginHostCrashed,
    ];

    /// Returns the stable numeric Event ID.
    #[inline]
    pub(crate) const fn id(self) -> u16 {
        self as u16
    }

    /// Emits this event through the dedicated operator-event target.
    ///
    /// Keeping target selection and stable IDs behind this typed boundary
    /// prevents ordinary tracing events from leaking into the curated Windows
    /// Event Log stream.
    pub(crate) fn emit(self, message: &str) {
        let message = escape(message);
        match self {
            Self::ServiceStarting
            | Self::ServiceReady
            | Self::ServiceStopRequested
            | Self::ServicePreshutdownRequested
            | Self::ServiceStopped => {
                tracing::info!(target: OperatorEvent::TARGET, id = self.id(), "{message}");
            }
            Self::ServiceLoggingDegraded
            | Self::ConfigurationLoadFailed
            | Self::PersistedStateLoadFailed
            | Self::PersistedStateSaveFailed
            | Self::ListenerBindFailed
            | Self::PluginHostCrashed => {
                tracing::error!(target: OperatorEvent::TARGET, id = self.id(), "{message}");
            }
        }
    }
}

#[cfg(test)]
#[path = "operator_event_tests.rs"]
mod tests;
