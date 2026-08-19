// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Polkit-based authorization for privileged D-Bus operations.

use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSnapshot {
    pub start_time: u64,
    pub groups: Vec<u32>,
}

/// Chooses whether group membership allows the request, Polkit should decide it,
/// or the request must be denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    AllowGroup,
    ConsultPolkit,
    Deny,
}

impl Decision {
    pub fn decide(groups: &[u32], required_gid: u32, polkit_enabled: bool) -> Decision {
        let in_group = groups.contains(&required_gid);
        if in_group {
            Decision::AllowGroup
        } else if polkit_enabled {
            Decision::ConsultPolkit
        } else {
            Decision::Deny
        }
    }
}

pub fn polkit_process_subject(pid: u32, start_time: u64, uid: u32) -> String {
    format!("{pid},{start_time},{uid}")
}

pub fn process_snapshot(
    proc_root: &Path,
    pid: u32,
    expected_uid: u32,
) -> io::Result<ProcessSnapshot> {
    let process_dir = proc_root.join(pid.to_string());
    let start_time = process_start_time(&fs::read_to_string(process_dir.join("stat"))?)?;
    let status = fs::read_to_string(process_dir.join("status"))?;
    let (effective_uid, primary_gid, mut groups) = parse_status(&status)?;
    let confirmed_start_time = process_start_time(&fs::read_to_string(process_dir.join("stat"))?)?;

    if start_time != confirmed_start_time {
        return Err(io::Error::other(
            "process identity changed while reading credentials",
        ));
    }
    if effective_uid != expected_uid {
        return Err(io::Error::other(
            "process UID does not match D-Bus credentials",
        ));
    }
    if !groups.contains(&primary_gid) {
        groups.push(primary_gid);
    }

    Ok(ProcessSnapshot { start_time, groups })
}

fn process_start_time(stat: &str) -> io::Result<u64> {
    let after_name = stat
        .rfind(')')
        .and_then(|end| stat.get(end + 1..))
        .ok_or_else(|| io::Error::other("process stat has no command field"))?;
    after_name
        .split_ascii_whitespace()
        .nth(19)
        .ok_or_else(|| io::Error::other("process stat has no start-time field"))?
        .parse::<u64>()
        .map_err(|error| io::Error::other(format!("invalid process start time: {error}")))
}

fn parse_status(status: &str) -> io::Result<(u32, u32, Vec<u32>)> {
    let effective_uid = status_field(status, "Uid:", 1, "effective UID")?;
    let primary_gid = status_field(status, "Gid:", 1, "effective GID")?;
    let groups = status
        .lines()
        .find_map(|line| line.strip_prefix("Groups:"))
        .ok_or_else(|| io::Error::other("process status has no Groups field"))?
        .split_ascii_whitespace()
        .map(|group| {
            group
                .parse::<u32>()
                .map_err(|error| io::Error::other(format!("invalid supplementary GID: {error}")))
        })
        .collect::<io::Result<Vec<_>>>()?;

    Ok((effective_uid, primary_gid, groups))
}

fn status_field(status: &str, name: &str, index: usize, description: &str) -> io::Result<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix(name))
        .and_then(|value| value.split_ascii_whitespace().nth(index))
        .ok_or_else(|| io::Error::other(format!("process status has no {description} field")))?
        .parse::<u32>()
        .map_err(|error| io::Error::other(format!("invalid {description}: {error}")))
}

pub fn group_gid(group_file: &Path, name: &str) -> io::Result<u32> {
    let groups = fs::read_to_string(group_file)?;
    groups
        .lines()
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| {
            let mut fields = line.split(':');
            (fields.next()? == name)
                .then(|| fields.nth(1)?.parse::<u32>().ok())
                .flatten()
        })
        .ok_or_else(|| io::Error::other(format!("group {name} does not exist")))
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
