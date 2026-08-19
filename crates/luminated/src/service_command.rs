// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Windows service-management and macOS launchd command dispatch.
//!
//! Windows and macOS accept different flags for `service install` and
//! `service uninstall`, so each platform has its own command type and parser.
//! Windows manages named-pipe client-group membership. On macOS, Unix-socket
//! group permissions serve that purpose; see
//! `docs/development/architecture/macos-service.md`.
//!
//! The command types and parsers remain available on every development
//! platform so their parsing logic can always be unit-tested. Only calls into
//! the platform service APIs are conditionally compiled.

use std::ffi::{OsStr, OsString};

use anyhow::{Result, bail};
#[cfg(target_os = "macos")]
use luminate_platform::macos::service::{
    EnsureAccountOutcome, ServiceState as MacosServiceState, install as macos_install,
    query_state as macos_query_state, start as macos_start, stop as macos_stop,
    uninstall as macos_uninstall,
};
#[cfg(windows)]
use luminate_platform::windows::service::{
    ServiceState, install, query_state, start, stop, uninstall,
};
#[cfg(any(windows, target_os = "macos"))]
use std::env;

const SERVICE_ARGUMENT: &str = "service";
const INSTALL_ARGUMENT: &str = "install";
const ALLOW_DEVELOPMENT_PATH_ARGUMENT: &str = "--allow-development-path";
#[cfg(any(windows, test))]
const ADD_USER_ARGUMENT: &str = "--add-user";
#[cfg(any(windows, test))]
const ADD_CURRENT_USER_ARGUMENT: &str = "--add-current-user";
const START_ON_INSTALL_ARGUMENT: &str = "--start";
const UNINSTALL_ARGUMENT: &str = "uninstall";
#[cfg(any(windows, test))]
const PURGE_CLIENT_GROUP_ARGUMENT: &str = "--purge-client-group";
#[cfg(any(target_os = "macos", test))]
const PURGE_ACCOUNT_ARGUMENT: &str = "--purge-account";
const START_ARGUMENT: &str = "start";
const STOP_ARGUMENT: &str = "stop";
const STATUS_ARGUMENT: &str = "status";

/// Windows service command parsed on Windows and in cross-platform tests.
#[cfg(any(windows, test))]
#[derive(Clone, Debug, PartialEq, Eq)]
enum Command {
    Install {
        allow_development_path: bool,
        add_users: Vec<String>,
        add_current_user: bool,
        start_on_install: bool,
    },
    Uninstall {
        purge_client_group: bool,
    },
    Start,
    Stop,
    Status,
}

/// Runs a service-management command when one was requested.
///
/// # Errors
///
/// Returns an error for an unknown or incomplete service command, or when SCM
/// rejects the requested operation.
#[cfg(windows)]
pub fn run_from_args() -> Result<bool> {
    let Some(command) = parse(env::args_os().skip(1))? else {
        return Ok(false);
    };

    match command {
        Command::Install {
            allow_development_path,
            add_users,
            add_current_user,
            start_on_install,
        } => {
            let membership_changed = install(allow_development_path, &add_users, add_current_user)?;
            if membership_changed {
                print_membership_notice();
            }
            if start_on_install {
                start()?;
            }
        }
        Command::Uninstall { purge_client_group } => uninstall(purge_client_group)?,
        Command::Start => start()?,
        Command::Stop => stop()?,
        Command::Status => print_state(query_state()?),
    }
    Ok(true)
}

#[cfg(any(windows, test))]
fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Option<Command>> {
    let mut arguments = arguments.into_iter();
    if arguments.next().as_deref() != Some(OsStr::new(SERVICE_ARGUMENT)) {
        return Ok(None);
    }

    let command = match arguments.next().as_deref() {
        Some(command) if command == OsStr::new(INSTALL_ARGUMENT) => {
            let mut allow_development_path = false;
            let mut add_users = Vec::new();
            let mut add_current_user = false;
            let mut start_on_install = false;
            while let Some(argument) = arguments.next() {
                if argument == OsStr::new(ALLOW_DEVELOPMENT_PATH_ARGUMENT) {
                    allow_development_path = true;
                } else if argument == OsStr::new(ADD_CURRENT_USER_ARGUMENT) {
                    add_current_user = true;
                } else if argument == OsStr::new(START_ON_INSTALL_ARGUMENT) {
                    start_on_install = true;
                } else if argument == OsStr::new(ADD_USER_ARGUMENT) {
                    let account = arguments
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--add-user requires an account name"))?;
                    let account = account.into_string().map_err(|account| {
                        anyhow::anyhow!(
                            "account name is not valid Unicode: {}",
                            account.to_string_lossy()
                        )
                    })?;
                    add_users.push(account);
                } else {
                    bail!(
                        "unexpected argument to service install: {}",
                        argument.to_string_lossy()
                    );
                }
            }
            Command::Install {
                allow_development_path,
                add_users,
                add_current_user,
                start_on_install,
            }
        }
        Some(command) if command == OsStr::new(UNINSTALL_ARGUMENT) => {
            let mut purge_client_group = false;
            for argument in arguments.by_ref() {
                if argument == OsStr::new(PURGE_CLIENT_GROUP_ARGUMENT) {
                    purge_client_group = true;
                } else {
                    bail!(
                        "unexpected argument to service uninstall: {}",
                        argument.to_string_lossy()
                    );
                }
            }
            Command::Uninstall { purge_client_group }
        }
        Some(command) if command == OsStr::new(START_ARGUMENT) => Command::Start,
        Some(command) if command == OsStr::new(STOP_ARGUMENT) => Command::Stop,
        Some(command) if command == OsStr::new(STATUS_ARGUMENT) => Command::Status,
        Some(command) => bail!("unknown service command: {}", command.to_string_lossy()),
        None => bail!("service command required"),
    };
    if let Some(argument) = arguments.next() {
        bail!(
            "unexpected argument to service {}: {}",
            command.name(),
            argument.to_string_lossy()
        );
    }

    Ok(Some(command))
}

#[cfg(any(windows, test))]
impl Command {
    const fn name(&self) -> &'static str {
        match self {
            Self::Install { .. } => INSTALL_ARGUMENT,
            Self::Uninstall { .. } => UNINSTALL_ARGUMENT,
            Self::Start => START_ARGUMENT,
            Self::Stop => STOP_ARGUMENT,
            Self::Status => STATUS_ARGUMENT,
        }
    }
}

#[allow(
    clippy::print_stdout,
    reason = "service installation guidance is intentional command-line output"
)]
#[cfg(windows)]
fn print_membership_notice() {
    println!(
        "Client-group membership changed. Sign out and back in, or reboot, before it takes effect."
    );
}

#[allow(
    clippy::print_stdout,
    reason = "service status is intentional command-line output"
)]
#[cfg(windows)]
fn print_state(state: ServiceState) {
    let state = match state {
        ServiceState::Stopped => "stopped",
        ServiceState::StartPending => "start-pending",
        ServiceState::StopPending => "stop-pending",
        ServiceState::Running => "running",
        ServiceState::ContinuePending => "continue-pending",
        ServiceState::PausePending => "pause-pending",
        ServiceState::Paused => "paused",
    };
    println!("{state}");
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
enum MacosCommand {
    Install {
        allow_development_path: bool,
        start_on_install: bool,
    },
    Uninstall {
        purge_account: bool,
    },
    Start,
    Stop,
    Status,
}

/// Runs a `launchd` service-management command when one was requested.
///
/// # Errors
///
/// Returns an error for an unknown or incomplete service command, or when
/// `launchctl`/`dscl` reject the requested operation.
#[cfg(target_os = "macos")]
pub fn run_from_args() -> Result<bool> {
    let Some(command) = parse_macos(env::args_os().skip(1))? else {
        return Ok(false);
    };

    match command {
        MacosCommand::Install {
            allow_development_path,
            start_on_install,
        } => {
            let outcome = macos_install(allow_development_path, start_on_install)?;
            print_account_outcome(outcome);
        }
        MacosCommand::Uninstall { purge_account } => macos_uninstall(purge_account)?,
        MacosCommand::Start => macos_start()?,
        MacosCommand::Stop => macos_stop()?,
        MacosCommand::Status => print_macos_state(macos_query_state()?),
    }
    Ok(true)
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos(arguments: impl IntoIterator<Item = OsString>) -> Result<Option<MacosCommand>> {
    let mut arguments = arguments.into_iter();
    if arguments.next().as_deref() != Some(OsStr::new(SERVICE_ARGUMENT)) {
        return Ok(None);
    }

    let command = match arguments.next().as_deref() {
        Some(command) if command == OsStr::new(INSTALL_ARGUMENT) => {
            let mut allow_development_path = false;
            let mut start_on_install = false;
            for argument in arguments.by_ref() {
                if argument == OsStr::new(ALLOW_DEVELOPMENT_PATH_ARGUMENT) {
                    allow_development_path = true;
                } else if argument == OsStr::new(START_ON_INSTALL_ARGUMENT) {
                    start_on_install = true;
                } else {
                    bail!(
                        "unexpected argument to service install: {}",
                        argument.to_string_lossy()
                    );
                }
            }
            MacosCommand::Install {
                allow_development_path,
                start_on_install,
            }
        }
        Some(command) if command == OsStr::new(UNINSTALL_ARGUMENT) => {
            let mut purge_account = false;
            for argument in arguments.by_ref() {
                if argument == OsStr::new(PURGE_ACCOUNT_ARGUMENT) {
                    purge_account = true;
                } else {
                    bail!(
                        "unexpected argument to service uninstall: {}",
                        argument.to_string_lossy()
                    );
                }
            }
            MacosCommand::Uninstall { purge_account }
        }
        Some(command) if command == OsStr::new(START_ARGUMENT) => MacosCommand::Start,
        Some(command) if command == OsStr::new(STOP_ARGUMENT) => MacosCommand::Stop,
        Some(command) if command == OsStr::new(STATUS_ARGUMENT) => MacosCommand::Status,
        Some(command) => bail!("unknown service command: {}", command.to_string_lossy()),
        None => bail!("service command required"),
    };
    if let Some(argument) = arguments.next() {
        bail!(
            "unexpected argument to service {}: {}",
            command.name(),
            argument.to_string_lossy()
        );
    }

    Ok(Some(command))
}

#[cfg(any(target_os = "macos", test))]
impl MacosCommand {
    const fn name(&self) -> &'static str {
        match self {
            Self::Install { .. } => INSTALL_ARGUMENT,
            Self::Uninstall { .. } => UNINSTALL_ARGUMENT,
            Self::Start => START_ARGUMENT,
            Self::Stop => STOP_ARGUMENT,
            Self::Status => STATUS_ARGUMENT,
        }
    }
}

#[allow(
    clippy::print_stdout,
    reason = "service installation guidance is intentional command-line output"
)]
#[cfg(target_os = "macos")]
fn print_account_outcome(outcome: EnsureAccountOutcome) {
    if outcome == EnsureAccountOutcome::Created {
        println!("Created the _luminated/_luminate service account.");
    }
}

#[allow(
    clippy::print_stdout,
    reason = "service status is intentional command-line output"
)]
#[cfg(target_os = "macos")]
fn print_macos_state(state: MacosServiceState) {
    let state = match state {
        MacosServiceState::NotLoaded => "not-loaded",
        MacosServiceState::Loaded { running: true } => "running",
        MacosServiceState::Loaded { running: false } => "loaded (not running)",
    };
    println!("{state}");
}

#[cfg(test)]
#[path = "service_command_tests.rs"]
mod tests;
