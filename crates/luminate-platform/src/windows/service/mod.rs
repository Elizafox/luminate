// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Windows Service Control Manager dispatch and lifecycle reporting.
//!
//! The control-handler context deliberately lives until process exit. SCM has
//! no handler-unregistration API, so reclaiming it when the first terminal
//! control arrives would race a concurrent or repeated control.

use std::time::Duration;

mod control;
mod dispatch;
mod error;
mod handler;
mod install;

pub use control::{ServiceState, query_state, start, stop};
pub use dispatch::{DispatchOutcome, dispatch};
pub use error::ServiceError;
pub use handler::{ServiceContext, ServiceShutdown, StatusReporter};
pub use install::{install, uninstall};

const SERVICE_NAME: &str = "luminated";
const SERVICE_DISPLAY_NAME: &str = "Luminate Lighting";
const SERVICE_DESCRIPTION: &str = "Manages lighting devices, restores saved lighting state at start-up, and serves client applications.";
const ERROR_FAILED_SERVICE_CONTROLLER_CONNECT: i32 = 1063;
const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;
const ERROR_SERVICE_EXISTS: i32 = 1073;
const ERROR_SERVICE_NOT_ACTIVE: i32 = 1062;
const ERROR_SERVICE_SPECIFIC_ERROR: u32 = 1066;
const REQUIRED_PRIVILEGES: [&str; 1] = ["SeChangeNotifyPrivilege"];
const FAILURE_COUNT_RESET_PERIOD: Duration = Duration::from_secs(24 * 60 * 60);
const FAILURE_RESTART_DELAYS: [Duration; 3] = [
    Duration::from_secs(5),
    Duration::from_secs(30),
    Duration::from_secs(5 * 60),
];
const PRESHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const START_WAIT_HINT: Duration = Duration::from_secs(30);
const STOP_WAIT_HINT: Duration = Duration::from_secs(21);
/// Stable name of the local group granted named-pipe client access.
pub const CLIENT_GROUP_NAME: &str = "Luminate Clients";
const CLIENT_GROUP_DESCRIPTION: &str = "Users allowed to connect to Luminate Lighting";
