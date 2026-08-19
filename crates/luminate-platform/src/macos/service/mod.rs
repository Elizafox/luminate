// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Installing and controlling `luminated` as a `launchd` `LaunchDaemon`.
//!
//! Much smaller than `windows::service`: launchd execs the configured program
//! as an ordinary foreground process and stops it with `SIGTERM`, so there is
//! no SCM-style control-handler bridge, no dispatcher, and no
//! `daemon::RunContext` variant here. The existing `RunContext::console()`
//! path is already the right shape for a launchd-run process.

mod account;
mod directories;
mod error;
mod install;
mod launchctl;
mod plist;

pub use account::{EnsureAccountOutcome, SERVICE_GROUP, SERVICE_USER};
pub use error::ServiceError;
pub use install::{install, plist_path, uninstall};
pub use launchctl::{ServiceState, query_state, start, stop};
pub use plist::LABEL;
