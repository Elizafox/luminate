// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Unix process-level signals: `SIGTERM`/`SIGINT` for shutdown, `SIGUSR1`
//! for an operator-driven rescan, and `SIGHUP` for a plugin reload.

use std::io;

use tokio::signal::unix::{Signal, SignalKind, signal};

/// A host's process-level shutdown signals.
pub struct ShutdownSignals {
    sigterm: Signal,
    sigint: Signal,
}

impl ShutdownSignals {
    /// Installs the signal handlers.
    ///
    /// # Errors
    ///
    /// Returns the underlying `io::Error` if registering either handler
    /// fails.
    pub fn install() -> io::Result<Self> {
        Ok(Self {
            sigterm: signal(SignalKind::terminate())?,
            sigint: signal(SignalKind::interrupt())?,
        })
    }

    /// Waits for the next shutdown-worthy signal and returns a name for it,
    /// suitable for a log message.
    pub async fn recv(&mut self) -> &'static str {
        tokio::select! {
            _ = self.sigterm.recv() => "SIGTERM",
            _ = self.sigint.recv() => "SIGINT",
        }
    }
}

/// `SIGUSR1`, an operator-driven rescan trigger for callers that cannot
/// speak the daemon's control protocol.
pub struct RescanSignal {
    sigusr1: Signal,
}

impl RescanSignal {
    /// Installs the signal handler.
    ///
    /// # Errors
    ///
    /// Returns the underlying `io::Error` if registering the handler fails.
    pub fn install() -> io::Result<Self> {
        Ok(Self {
            sigusr1: signal(SignalKind::user_defined1())?,
        })
    }

    /// Waits for the next `SIGUSR1`.
    pub async fn recv(&mut self) {
        // A `Signal` stream never ends; `recv` only returns `None` if the
        // underlying OS signal handling is torn down, which a running host
        // never does.
        self.sigusr1.recv().await;
    }
}

/// `SIGHUP`, reloading every currently loaded plugin at its existing path
/// and configuration, for callers that cannot speak the daemon's control
/// protocol.
///
/// This does not re-read the daemon's configuration file for added or
/// removed plugin entries, only restart what's already loaded.
pub struct ReloadSignal {
    sighup: Signal,
}

impl ReloadSignal {
    /// Installs the signal handler.
    ///
    /// # Errors
    ///
    /// Returns the underlying `io::Error` if registering the handler fails.
    pub fn install() -> io::Result<Self> {
        Ok(Self {
            sighup: signal(SignalKind::hangup())?,
        })
    }

    /// Waits for the next `SIGHUP`.
    pub async fn recv(&mut self) {
        self.sighup.recv().await;
    }
}

#[cfg(test)]
#[path = "process_signals_tests.rs"]
mod tests;
