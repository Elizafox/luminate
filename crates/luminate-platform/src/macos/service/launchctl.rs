// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Narrow `launchctl` wrappers for loading, unloading, and controlling the
//! registered `LaunchDaemon`, mirroring the shape of
//! `windows::service::control`'s SCM wrappers.

use std::path::Path;
use std::process::{Command, Output};

use super::error::ServiceError;
use super::plist::LABEL;

/// The `launchctl` service target for the system domain, e.g.
/// `system/com.wilcoxti.luminate.luminated`.
fn service_target() -> String {
    format!("system/{LABEL}")
}

/// Current `launchd` state of the registered daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceState {
    /// No job with this label is loaded in the system domain.
    NotLoaded,
    /// The job is loaded; `running` reports whether it currently has a live
    /// process.
    Loaded {
        /// Whether `launchctl print` reported a running process.
        running: bool,
    },
}

/// Loads the plist at `plist_path` into the system domain, starting it
/// immediately if its `RunAtLoad` key is set (it is; see [`super::plist`]).
///
/// # Errors
///
/// Returns an error if `launchctl` fails for a reason other than the job
/// already being loaded.
pub fn bootstrap(plist_path: &Path) -> Result<(), ServiceError> {
    let output = run(&["bootstrap", "system", &plist_path.display().to_string()])?;
    if output.status.success() || already_bootstrapped(&stderr(&output)) {
        Ok(())
    } else {
        Err(command_failed(
            &["bootstrap", "system", &plist_path.display().to_string()],
            &output,
        ))
    }
}

/// Unloads the daemon's job from the system domain.
///
/// # Errors
///
/// Returns an error if `launchctl` fails for a reason other than the job
/// already being absent.
pub fn bootout() -> Result<(), ServiceError> {
    let target = service_target();
    let output = run(&["bootout", &target])?;
    if output.status.success() || not_loaded(&stderr(&output)) {
        Ok(())
    } else {
        Err(command_failed(&["bootout", &target], &output))
    }
}

/// Starts the daemon if it is not already running.
///
/// # Errors
///
/// Returns an error if `launchctl` fails.
pub fn start() -> Result<(), ServiceError> {
    let target = service_target();
    let output = run(&["kickstart", &target])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failed(&["kickstart", &target], &output))
    }
}

/// Sends `SIGTERM` to the daemon's running process, if any.
///
/// # Errors
///
/// Returns an error if `launchctl` fails for a reason other than the job not
/// currently having a running process.
pub fn stop() -> Result<(), ServiceError> {
    let target = service_target();
    let output = run(&["kill", "SIGTERM", &target])?;
    if output.status.success() || not_running(&stderr(&output)) {
        Ok(())
    } else {
        Err(command_failed(&["kill", "SIGTERM", &target], &output))
    }
}

/// Queries the daemon's current `launchd` state.
///
/// # Errors
///
/// Returns an error if `launchctl` fails for a reason other than the job not
/// being loaded.
pub fn query_state() -> Result<ServiceState, ServiceError> {
    let target = service_target();
    let output = run(&["print", &target])?;
    if !output.status.success() {
        return if not_loaded(&stderr(&output)) {
            Ok(ServiceState::NotLoaded)
        } else {
            Err(command_failed(&["print", &target], &output))
        };
    }
    Ok(parse_print_state(&stdout(&output)))
}

fn run(args: &[&str]) -> Result<Output, ServiceError> {
    Command::new("launchctl")
        .args(args)
        .output()
        .map_err(ServiceError::Command)
}

fn command_failed(args: &[&str], output: &Output) -> ServiceError {
    ServiceError::CommandFailed {
        program: "launchctl",
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        stderr: stderr(output).trim().to_owned(),
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// `launchctl bootstrap` reports an already-loaded job this way rather than
/// with a distinct exit code.
fn already_bootstrapped(stderr: &str) -> bool {
    stderr.contains("already bootstrapped")
}

/// `launchctl bootout`/`print` report an absent job this way.
fn not_loaded(stderr: &str) -> bool {
    stderr.contains("Could not find service") || stderr.contains("No such process")
}

/// `launchctl kill` reports a loaded-but-not-running job this way.
fn not_running(stderr: &str) -> bool {
    stderr.contains("No such process") || stderr.contains("not running")
}

/// Parses `launchctl print`'s `state = <value>` line.
fn parse_print_state(stdout: &str) -> ServiceState {
    let running = stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("state = "))
        .is_some_and(|state| state.trim() == "running");
    ServiceState::Loaded { running }
}

#[cfg(test)]
#[path = "launchctl_tests.rs"]
mod tests;
