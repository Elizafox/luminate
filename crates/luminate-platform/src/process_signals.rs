// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! A host's process-level shutdown and rescan signals, abstracted over Unix
//! signals and Windows console-control events so callers need no `cfg` of
//! their own.
//!
//! [`ShutdownSignals`], [`RescanSignal`], and [`ReloadSignal`] have the same
//! shape on every platform (an `install`/`recv` pair); only what they wait
//! on differs, so the platform-specific bodies live under
//! [`crate::unix`]/[`crate::windows`] and this module just re-exports
//! whichever one matches the build target.

#[cfg(unix)]
pub use crate::unix::process_signals::{ReloadSignal, RescanSignal, ShutdownSignals};
#[cfg(windows)]
pub use crate::windows::process_signals::{ReloadSignal, RescanSignal, ShutdownSignals};
