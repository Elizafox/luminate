// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Polkit-based authorization for privileged D-Bus operations.

use std::fs;
use std::io;
#[cfg(target_os = "macos")]
use std::mem::MaybeUninit;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::ptr;

#[cfg(target_os = "macos")]
const MAX_ACCOUNT_LOOKUP_BYTES: usize = 1024 * 1024;
#[cfg(target_os = "macos")]
const MAX_GROUPS: usize = 65_536;

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

#[cfg(any(test, not(target_os = "macos")))]
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

/// Resolves the authenticated macOS user's complete account group list.
///
/// # Errors
///
/// Returns an error when the UID has no local account, native account lookup
/// fails, or the returned group list cannot be represented safely.
#[cfg(target_os = "macos")]
#[allow(
    unsafe_code,
    reason = "getpwuid_r and getgrouplist are the native thread-safe account lookup APIs; all pointers refer to live storage for each call and returned pointers are not retained"
)]
pub(crate) fn account_snapshot(uid: u32) -> io::Result<ProcessSnapshot> {
    // SAFETY: sysconf reads one process-wide configuration value and neither
    // retains pointers nor accesses Rust-owned memory.
    let suggested = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let initial = usize::try_from(suggested)
        .unwrap_or(16 * 1024)
        .clamp(1024, MAX_ACCOUNT_LOOKUP_BYTES);
    let mut buffer = vec![0_u8; initial];

    loop {
        let mut entry = MaybeUninit::<libc::passwd>::uninit();
        let mut result = ptr::null_mut();
        // SAFETY: entry and result are valid output storage, and buffer is
        // writable for its reported length. No pointer escapes this loop.
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &raw mut result,
            )
        };
        if status == libc::ERANGE {
            let next = buffer
                .len()
                .checked_mul(2)
                .filter(|next| *next <= MAX_ACCOUNT_LOOKUP_BYTES)
                .ok_or_else(|| io::Error::other("account lookup buffer is too large"))?;
            buffer.resize(next, 0);
            continue;
        }
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        if result.is_null() {
            return Err(io::Error::other("D-Bus caller UID has no local account"));
        }

        // SAFETY: a non-null result from a successful getpwuid_r call means
        // that entry was initialized and its pointers refer into buffer.
        let entry = unsafe { entry.assume_init() };
        let primary_gid = libc::c_int::try_from(entry.pw_gid)
            .map_err(|_| io::Error::other("account primary group is out of range"))?;
        let mut group_capacity = libc::c_int::try_from(MAX_GROUPS)
            .map_err(|_| io::Error::other("maximum account group count is invalid"))?;
        let mut groups = vec![0; MAX_GROUPS];
        // SAFETY: pw_name remains valid because buffer is not changed, and
        // groups is writable for group_capacity entries.
        let status = unsafe {
            libc::getgrouplist(
                entry.pw_name,
                primary_gid,
                groups.as_mut_ptr(),
                &raw mut group_capacity,
            )
        };
        let count = usize::try_from(status)
            .ok()
            .filter(|count| *count <= groups.len())
            .ok_or_else(|| io::Error::other("account group count is invalid"))?;
        groups.truncate(count);
        let groups = groups
            .into_iter()
            .map(|group| {
                u32::try_from(group)
                    .map_err(|_| io::Error::other("account group ID is out of range"))
            })
            .collect::<io::Result<Vec<_>>>()?;
        return Ok(ProcessSnapshot {
            start_time: 0,
            groups,
        });
    }
}

#[cfg(any(test, not(target_os = "macos")))]
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

#[cfg(any(test, not(target_os = "macos")))]
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

#[cfg(any(test, not(target_os = "macos")))]
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
