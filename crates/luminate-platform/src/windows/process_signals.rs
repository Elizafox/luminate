// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Windows console-control events, abstracted to the same shape as the Unix
//! process signals a host also needs.

use std::future::pending;
use std::io;

use tokio::signal::windows::{
    CtrlBreak, CtrlC, CtrlClose, CtrlShutdown, ctrl_break, ctrl_c, ctrl_close, ctrl_shutdown,
};

/// A host's process-level shutdown signals.
///
/// `CTRL_LOGOFF_EVENT` (the Windows analogue of `SIGHUP`) is deliberately
/// not handled here. Unlike on Unix, [`ReloadSignal`] has no Windows signal
/// to drive it either (see its docs). On Windows, plugin reloads are available
/// only through the control protocol's `ReloadPlugin` request.
pub struct ShutdownSignals {
    c: CtrlC,
    break_signal: CtrlBreak,
    close: CtrlClose,
    shutdown: CtrlShutdown,
}

impl ShutdownSignals {
    /// Installs the console-control handlers.
    ///
    /// # Errors
    ///
    /// Returns the underlying `io::Error` if registering any handler fails.
    pub fn install() -> io::Result<Self> {
        Ok(Self {
            c: ctrl_c()?,
            break_signal: ctrl_break()?,
            close: ctrl_close()?,
            shutdown: ctrl_shutdown()?,
        })
    }

    /// Waits for the next shutdown-worthy event and returns a name for it,
    /// suitable for a log message.
    pub async fn recv(&mut self) -> &'static str {
        tokio::select! {
            _ = self.c.recv() => "Ctrl+C",
            _ = self.break_signal.recv() => "Ctrl+Break",
            _ = self.close.recv() => "console close",
            _ = self.shutdown.recv() => "system shutdown",
        }
    }
}

/// The operator-driven rescan trigger on Unix has no Windows equivalent
/// signal, so [`RescanSignal::recv`] never resolves here. This leaves the
/// operator-driven rescan reachable only through the control protocol's
/// `Rescan` request on this platform.
pub struct RescanSignal;

impl RescanSignal {
    /// Installs the (empty) handler.
    ///
    /// # Errors
    ///
    /// Never fails; the `Result` return type matches the Unix side's
    /// fallible installation so callers don't need their own `cfg`.
    pub fn install() -> io::Result<Self> {
        Ok(Self)
    }

    /// Never resolves; see the struct documentation.
    pub async fn recv(&mut self) {
        pending::<()>().await;
    }
}

/// `SIGHUP`'s plugin-reload trigger on Unix has no Windows equivalent
/// signal, so [`ReloadSignal::recv`] never resolves here. This leaves a
/// plugin reload reachable only through the control protocol's
/// `ReloadPlugin` request on this platform.
pub struct ReloadSignal;

impl ReloadSignal {
    /// Installs the (empty) handler.
    ///
    /// # Errors
    ///
    /// Never fails; the `Result` return type matches the Unix side's
    /// fallible installation so callers don't need their own `cfg`.
    pub fn install() -> io::Result<Self> {
        Ok(Self)
    }

    /// Never resolves; see the struct documentation.
    pub async fn recv(&mut self) {
        pending::<()>().await;
    }
}
