// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Idempotent management of the dedicated `_luminated`/`_luminate` service
//! account, via `dscl`.
//!
//! macOS has no `useradd`/`sysusers.d` equivalent and no atomic "allocate a
//! free system id" primitive, so this scans the existing `UniqueID`/
//! `PrimaryGroupID` values and picks the lowest free one in the conventional
//! system-account range (see `docs/development/architecture/macos-service.md`,
//! "UID/GID allocation"). Installation is a single administrator-driven
//! operation, the same non-concurrency assumption the Windows installer
//! makes for its own
//! registry/SCM writes, so the resulting scan-then-create race is accepted.

use std::iter;
use std::ops::RangeInclusive;
use std::process::{Command, Output};
use std::{fmt, str};

use super::error::ServiceError;

/// The dedicated, unprivileged account `luminated` runs as under launchd.
pub const SERVICE_USER: &str = "_luminated";
/// The dedicated group named on the runtime socket directory and the socket
/// itself, mirroring the Linux `luminate` group's role.
pub const SERVICE_GROUP: &str = "_luminate";

const REAL_NAME: &str = "Luminate Lighting Daemon";
const GROUP_REAL_NAME: &str = "Luminate Lighting Daemon Group";
const USER_SHELL: &str = "/sbin/nologin";
const HOME_DIRECTORY: &str = "/var/empty";
/// Apple's conventional (unenforced) system-account id range.
const SYSTEM_ID_RANGE: RangeInclusive<u32> = 200..=400;

/// Outcome of ensuring the service account exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnsureAccountOutcome {
    /// This invocation created the user and/or group.
    Created,
    /// The user and group already existed with matching attributes.
    AlreadyExisted,
}

/// Creates `_luminated`/`_luminate` if absent, or verifies an idempotent
/// match if present.
///
/// # Errors
///
/// Returns [`ServiceError::AccountConflict`] if either the user or the group
/// already exists with attributes this function did not set, so a
/// pre-existing, unrelated account is never silently repurposed. Returns
/// [`ServiceError::IdRangeExhausted`] if no id remains in the reserved range,
/// and [`ServiceError::Command`]/[`ServiceError::CommandFailed`] if `dscl`
/// itself fails.
pub fn ensure_service_account() -> Result<EnsureAccountOutcome, ServiceError> {
    let group_created = ensure_group()?;
    let gid = read_id("/Groups", SERVICE_GROUP, "PrimaryGroupID")?
        .ok_or_else(|| ServiceError::AccountConflict(format!("{SERVICE_GROUP} has no GID")))?;
    let user_created = ensure_user(gid)?;

    Ok(if group_created || user_created {
        EnsureAccountOutcome::Created
    } else {
        EnsureAccountOutcome::AlreadyExisted
    })
}

/// Reads the `_luminate` group's GID, for hardening directory ownership.
///
/// # Errors
///
/// Returns an error if `dscl` fails or the group has no GID (e.g. it does not
/// exist yet. Call after [`ensure_service_account`]).
pub fn group_gid() -> Result<u32, ServiceError> {
    read_id("/Groups", SERVICE_GROUP, "PrimaryGroupID")?
        .ok_or_else(|| ServiceError::AccountConflict(format!("{SERVICE_GROUP} has no GID")))
}

/// Reads the `_luminated` user's UID, for hardening directory ownership.
///
/// # Errors
///
/// Returns an error if `dscl` fails or the user has no UID (e.g. it does not
/// exist yet. Call after [`ensure_service_account`]).
pub fn user_uid() -> Result<u32, ServiceError> {
    read_id("/Users", SERVICE_USER, "UniqueID")?
        .ok_or_else(|| ServiceError::AccountConflict(format!("{SERVICE_USER} has no UID")))
}

/// Removes `_luminated`/`_luminate`, refusing unless both still match the
/// attributes [`ensure_service_account`] sets.
///
/// # Errors
///
/// Returns [`ServiceError::UnsafeAccountPurge`] if either account's
/// attributes have changed since installation, so an administrator-modified
/// or reused account is never removed. Returns
/// [`ServiceError::Command`]/[`ServiceError::CommandFailed`] if `dscl` fails.
pub fn remove_service_account() -> Result<(), ServiceError> {
    match user_conflict()? {
        UserPresence::Absent => {}
        UserPresence::Matches => {
            run_dscl(&["-delete", &user_path()])?;
        }
        UserPresence::Conflicts(reason) => {
            return Err(ServiceError::UnsafeAccountPurge(format!(
                "{SERVICE_USER}: {reason}"
            )));
        }
    }

    match group_conflict()? {
        GroupPresence::Absent => {}
        GroupPresence::Matches => {
            run_dscl(&["-delete", &group_path()])?;
        }
        GroupPresence::Conflicts(reason) => {
            return Err(ServiceError::UnsafeAccountPurge(format!(
                "{SERVICE_GROUP}: {reason}"
            )));
        }
    }

    Ok(())
}

fn user_path() -> String {
    format!("/Users/{SERVICE_USER}")
}

fn group_path() -> String {
    format!("/Groups/{SERVICE_GROUP}")
}

fn ensure_group() -> Result<bool, ServiceError> {
    match group_conflict()? {
        GroupPresence::Matches => Ok(false),
        GroupPresence::Conflicts(reason) => Err(ServiceError::AccountConflict(format!(
            "{SERVICE_GROUP}: {reason}"
        ))),
        GroupPresence::Absent => {
            let used = list_ids("/Groups", "PrimaryGroupID")?;
            let gid = allocate_id(used.into_iter(), SYSTEM_ID_RANGE)
                .ok_or(ServiceError::IdRangeExhausted { kind: "GID" })?;
            run_dscl(&["-create", &group_path()])?;
            run_dscl(&["-create", &group_path(), "PrimaryGroupID", &gid.to_string()])?;
            run_dscl(&["-create", &group_path(), "RealName", GROUP_REAL_NAME])?;
            Ok(true)
        }
    }
}

fn ensure_user(gid: u32) -> Result<bool, ServiceError> {
    match user_conflict()? {
        UserPresence::Matches => Ok(false),
        UserPresence::Absent => {
            let used = list_ids("/Users", "UniqueID")?;
            let uid = allocate_id(used.into_iter(), SYSTEM_ID_RANGE)
                .ok_or(ServiceError::IdRangeExhausted { kind: "UID" })?;
            run_dscl(&["-create", &user_path()])?;
            run_dscl(&["-create", &user_path(), "UniqueID", &uid.to_string()])?;
            run_dscl(&["-create", &user_path(), "PrimaryGroupID", &gid.to_string()])?;
            run_dscl(&["-create", &user_path(), "UserShell", USER_SHELL])?;
            run_dscl(&["-create", &user_path(), "NFSHomeDirectory", HOME_DIRECTORY])?;
            run_dscl(&["-create", &user_path(), "RealName", REAL_NAME])?;
            Ok(true)
        }
        UserPresence::Conflicts(reason) => Err(ServiceError::AccountConflict(format!(
            "{SERVICE_USER}: {reason}"
        ))),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GroupPresence {
    Absent,
    Matches,
    Conflicts(String),
}

fn group_conflict() -> Result<GroupPresence, ServiceError> {
    if !dscl_path_exists("/Groups", SERVICE_GROUP)? {
        return Ok(GroupPresence::Absent);
    }

    let gid = read_id("/Groups", SERVICE_GROUP, "PrimaryGroupID")?;
    let real_name = read_attribute("/Groups", SERVICE_GROUP, "RealName")?;
    Ok(classify_group(gid, real_name.as_deref()))
}

fn classify_group(gid: Option<u32>, real_name: Option<&str>) -> GroupPresence {
    if gid.is_none() {
        return GroupPresence::Conflicts("has no GID".to_owned());
    }

    match mismatched_attribute("RealName", real_name, Some(GROUP_REAL_NAME)) {
        None => GroupPresence::Matches,
        Some(reason) => GroupPresence::Conflicts(reason),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum UserPresence {
    Absent,
    Matches,
    Conflicts(String),
}

fn user_conflict() -> Result<UserPresence, ServiceError> {
    if !dscl_path_exists("/Users", SERVICE_USER)? {
        return Ok(UserPresence::Absent);
    }

    let real_name = read_attribute("/Users", SERVICE_USER, "RealName")?;
    let shell = read_attribute("/Users", SERVICE_USER, "UserShell")?;
    let home = read_attribute("/Users", SERVICE_USER, "NFSHomeDirectory")?;
    let expected_gid = read_id("/Groups", SERVICE_GROUP, "PrimaryGroupID")?;
    let actual_gid = read_id("/Users", SERVICE_USER, "PrimaryGroupID")?;

    let mismatch = mismatched_attribute("RealName", real_name.as_deref(), Some(REAL_NAME))
        .or_else(|| mismatched_attribute("UserShell", shell.as_deref(), Some(USER_SHELL)))
        .or_else(|| mismatched_attribute("NFSHomeDirectory", home.as_deref(), Some(HOME_DIRECTORY)))
        .or_else(|| match (actual_gid, expected_gid) {
            (Some(actual), Some(expected)) if actual == expected => None,
            _ => Some("PrimaryGroupID does not match the Luminate group".to_owned()),
        });

    Ok(match mismatch {
        None => UserPresence::Matches,
        Some(reason) => UserPresence::Conflicts(reason),
    })
}

fn mismatched_attribute(
    name: &str,
    actual: Option<&str>,
    expected: Option<&str>,
) -> Option<String> {
    if actual == expected {
        None
    } else {
        Some(format!("{name} is {actual:?}, expected {expected:?}"))
    }
}

/// Picks the lowest value in `range` not present in `used`.
fn allocate_id(used: impl Iterator<Item = u32>, range: RangeInclusive<u32>) -> Option<u32> {
    let used: Vec<u32> = used.collect();
    range
        .into_iter()
        .find(|candidate| !used.contains(candidate))
}

fn dscl_path_exists(directory: &str, name: &str) -> Result<bool, ServiceError> {
    let output = Command::new("dscl")
        .args([".", "-read", &format!("{directory}/{name}")])
        .output()
        .map_err(ServiceError::Command)?;
    Ok(output.status.success())
}

/// Reads a single-valued `dscl` attribute, returning `None` if the record or
/// the attribute is absent.
fn read_attribute(directory: &str, name: &str, key: &str) -> Result<Option<String>, ServiceError> {
    let output = Command::new("dscl")
        .args([".", "-read", &format!("{directory}/{name}"), key])
        .output()
        .map_err(ServiceError::Command)?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(parse_single_valued_attribute(&stdout_lossy(&output), key))
}

fn read_id(directory: &str, name: &str, key: &str) -> Result<Option<u32>, ServiceError> {
    Ok(read_attribute(directory, name, key)?.and_then(|value| value.parse().ok()))
}

/// Lists every value of a numeric attribute across every record under
/// `directory`, e.g. every account's `UniqueID`.
fn list_ids(directory: &str, key: &str) -> Result<Vec<u32>, ServiceError> {
    let output = Command::new("dscl")
        .args([".", "-list", directory, key])
        .output()
        .map_err(ServiceError::Command)?;
    if !output.status.success() {
        return Err(command_failed(
            "dscl",
            &[".", "-list", directory, key],
            &output,
        ));
    }
    Ok(parse_id_list(&stdout_lossy(&output)))
}

fn run_dscl(args: &[&str]) -> Result<(), ServiceError> {
    let command_args = dscl_command_args(args);
    let output = Command::new("dscl")
        .args(&command_args)
        .output()
        .map_err(ServiceError::Command)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failed("dscl", &command_args, &output))
    }
}

fn dscl_command_args<'a>(args: &'a [&'a str]) -> Vec<&'a str> {
    iter::once(".").chain(args.iter().copied()).collect()
}

fn command_failed(program: &'static str, args: &[&str], output: &Output) -> ServiceError {
    ServiceError::CommandFailed {
        program,
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        stderr: stderr_lossy(output).trim().to_owned(),
    }
}

fn stdout_lossy(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_lossy(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Parses `dscl -read <path> <key>` output of the form `"<key>: value"`.
fn parse_single_valued_attribute(output: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    let mut lines = output.lines();
    while let Some(line) = lines.next() {
        let Some(value) = line.strip_prefix(&prefix) else {
            continue;
        };
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_owned());
        }

        return lines
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
    }
    None
}

/// Parses `dscl -list <directory> <key>` output: one `"<name> <value>"` line
/// per record, some possibly missing the value.
fn parse_id_list(output: &str) -> Vec<u32> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().next_back())
        .filter_map(|value| value.parse().ok())
        .collect()
}

impl fmt::Display for EnsureAccountOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Created => write!(formatter, "created"),
            Self::AlreadyExisted => write!(formatter, "already existed"),
        }
    }
}

#[cfg(test)]
#[path = "account_tests.rs"]
mod tests;
