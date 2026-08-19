// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Idempotent management of the local group allowed to connect to service
//! named pipes.
//!
//! Installation needs to distinguish objects it created from objects an
//! administrator created earlier. These operations therefore report that
//! distinction instead of flattening an already-existing object into an
//! uninformative success.

use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_ALIAS_EXISTS, ERROR_MEMBER_IN_ALIAS};
use windows_sys::Win32::NetworkManagement::NetManagement::{
    LOCALGROUP_INFO_1, LOCALGROUP_MEMBERS_INFO_3, NERR_GroupExists as NERR_GROUP_EXISTS,
    NERR_Success as NERR_SUCCESS, NetApiBufferFree, NetLocalGroupAdd, NetLocalGroupAddMembers,
    NetLocalGroupDel, NetLocalGroupDelMembers, NetLocalGroupGetMembers,
};
use windows_sys::Win32::Security::Authentication::Identity::{GetUserNameExW, NameSamCompatible};

use super::wide::to_wide_checked;

const LOCAL_GROUP_INFO_LEVEL: u32 = 1;
const LOCAL_GROUP_MEMBER_ACCOUNT_LEVEL: u32 = 3;
const MAX_PREFERRED_LENGTH: u32 = u32::MAX;

/// Outcome of ensuring that a local group exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnsureGroupOutcome {
    /// This invocation created the group.
    Created,
    /// A group with this name already existed.
    AlreadyExisted,
}

/// Outcome of adding an account to a local group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddMemberOutcome {
    /// This invocation added the account.
    Added,
    /// The account was already a member of the group.
    AlreadyMember,
}

/// Creates a local group if it does not already exist.
///
/// `comment` becomes the group's description in Windows administration tools.
///
/// # Errors
///
/// Returns an error when either string contains an embedded NUL or Windows
/// cannot create or inspect the group. Creating a local group ordinarily
/// requires administrator privileges.
pub fn ensure_local_group(name: &str, comment: &str) -> io::Result<EnsureGroupOutcome> {
    let mut name = to_wide_checked(name)?;
    let mut comment = to_wide_checked(comment)?;
    let info = LOCALGROUP_INFO_1 {
        lgrpi1_name: name.as_mut_ptr(),
        lgrpi1_comment: comment.as_mut_ptr(),
    };
    let mut parameter_error = 0_u32;

    #[allow(
        unsafe_code,
        reason = "NetLocalGroupAdd is a Win32 API with no safe standard-library wrapper; all \
                  pointers refer to local buffers that remain valid for the call."
    )]
    // SAFETY: both strings are writable, NUL-terminated UTF-16 buffers that
    // outlive the call. `info` has the layout required for information level
    // 1, and `parameter_error` is a local out-parameter.
    let status = unsafe {
        NetLocalGroupAdd(
            ptr::null(),
            LOCAL_GROUP_INFO_LEVEL,
            ptr::from_ref(&info).cast::<u8>(),
            &raw mut parameter_error,
        )
    };
    match status {
        NERR_SUCCESS => Ok(EnsureGroupOutcome::Created),
        NERR_GROUP_EXISTS | ERROR_ALIAS_EXISTS => Ok(EnsureGroupOutcome::AlreadyExisted),
        status => Err(net_api_error(status, Some(parameter_error))),
    }
}

/// Adds a domain-qualified account to an existing local group.
///
/// # Errors
///
/// Returns an error when either string contains an embedded NUL, the account
/// cannot be resolved, the group does not exist, or Windows refuses the
/// membership change. Changing local-group membership ordinarily requires
/// administrator privileges.
pub fn add_local_group_member(group: &str, account: &str) -> io::Result<AddMemberOutcome> {
    let group = to_wide_checked(group)?;
    let mut account = to_wide_checked(account)?;
    let member = LOCALGROUP_MEMBERS_INFO_3 {
        lgrmi3_domainandname: account.as_mut_ptr(),
    };

    #[allow(
        unsafe_code,
        reason = "NetLocalGroupAddMembers is a Win32 API with no safe standard-library wrapper; \
                  all pointers refer to local buffers that remain valid for the call."
    )]
    // SAFETY: `group` and `account` are NUL-terminated UTF-16 buffers that
    // outlive the call. `member` has the layout required for information
    // level 3 and names exactly one entry.
    let status = unsafe {
        NetLocalGroupAddMembers(
            ptr::null(),
            group.as_ptr(),
            LOCAL_GROUP_MEMBER_ACCOUNT_LEVEL,
            ptr::from_ref(&member).cast::<u8>(),
            1,
        )
    };
    match status {
        NERR_SUCCESS => Ok(AddMemberOutcome::Added),
        ERROR_MEMBER_IN_ALIAS => Ok(AddMemberOutcome::AlreadyMember),
        status => Err(net_api_error(status, None)),
    }
}

/// Returns the current process user's domain-qualified account name.
///
/// # Errors
///
/// Returns an error when Windows cannot determine the account name or returns
/// invalid UTF-16.
#[allow(
    unsafe_code,
    reason = "GetUserNameExW is a Win32 API with no safe standard-library wrapper; both calls \
              follow the documented probe-then-fill pattern."
)]
pub fn current_account_name() -> io::Result<String> {
    let mut length = 0_u32;

    // SAFETY: a null buffer with a zero length is the documented size-probe
    // form. The following call receives the resulting writable allocation.
    unsafe { GetUserNameExW(NameSamCompatible, ptr::null_mut(), &raw mut length) };
    if length == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut account = vec![0_u16; length as usize];
    // SAFETY: `account` has the size returned by the probe and remains
    // writable and live for the call.
    let ok = unsafe { GetUserNameExW(NameSamCompatible, account.as_mut_ptr(), &raw mut length) };
    if !ok {
        return Err(io::Error::last_os_error());
    }

    let account = account.strip_suffix(&[0]).unwrap_or(&account);
    String::from_utf16(account).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Removes a local group created by an incomplete installation.
///
/// # Errors
///
/// Returns an error when the group does not exist or Windows refuses to remove
/// it. Removing a local group ordinarily requires administrator privileges.
pub fn delete_local_group(name: &str) -> io::Result<()> {
    let name = to_wide_checked(name)?;

    #[allow(
        unsafe_code,
        reason = "NetLocalGroupDel is a Win32 API with no safe standard-library wrapper; the name \
                  pointer refers to a local NUL-terminated buffer valid for the call."
    )]
    // SAFETY: `name` is a NUL-terminated UTF-16 buffer that outlives the
    // synchronous call.
    let status = unsafe { NetLocalGroupDel(ptr::null(), name.as_ptr()) };
    if status == NERR_SUCCESS {
        Ok(())
    } else {
        Err(net_api_error(status, None))
    }
}

/// Reports whether a local group has no members.
///
/// # Errors
///
/// Returns an error when the group does not exist or Windows refuses to
/// enumerate its membership.
pub fn local_group_is_empty(name: &str) -> io::Result<bool> {
    let name = to_wide_checked(name)?;
    let mut buffer = ptr::null_mut();
    let mut entries_read = 0_u32;
    let mut total_entries = 0_u32;

    #[allow(
        unsafe_code,
        reason = "NetLocalGroupGetMembers allocates a buffer through the NetAPI contract; the \
                  returned allocation is released with NetApiBufferFree."
    )]
    // SAFETY: `name` is NUL-terminated and all output pointers refer to local
    // variables. A null resume handle requests the complete enumeration.
    let status = unsafe {
        NetLocalGroupGetMembers(
            ptr::null(),
            name.as_ptr(),
            0,
            &raw mut buffer,
            MAX_PREFERRED_LENGTH,
            &raw mut entries_read,
            &raw mut total_entries,
            ptr::null_mut(),
        )
    };
    let _buffer = NetApiBuffer(buffer);
    if status != NERR_SUCCESS {
        return Err(net_api_error(status, None));
    }

    Ok(entries_read == 0 && total_entries == 0)
}

/// Removes an account from an existing local group during rollback.
///
/// # Errors
///
/// Returns an error when either string contains an embedded NUL, the account
/// or group does not exist, or Windows refuses the membership change.
pub fn remove_local_group_member(group: &str, account: &str) -> io::Result<()> {
    let group = to_wide_checked(group)?;
    let mut account = to_wide_checked(account)?;
    let member = LOCALGROUP_MEMBERS_INFO_3 {
        lgrmi3_domainandname: account.as_mut_ptr(),
    };

    #[allow(
        unsafe_code,
        reason = "NetLocalGroupDelMembers is a Win32 API with no safe standard-library wrapper; \
                  all pointers refer to local buffers that remain valid for the call."
    )]
    // SAFETY: `group` and `account` are NUL-terminated UTF-16 buffers that
    // outlive the call. `member` has the layout required for level 3.
    let status = unsafe {
        NetLocalGroupDelMembers(
            ptr::null(),
            group.as_ptr(),
            LOCAL_GROUP_MEMBER_ACCOUNT_LEVEL,
            ptr::from_ref(&member).cast::<u8>(),
            1,
        )
    };
    if status == NERR_SUCCESS {
        Ok(())
    } else {
        Err(net_api_error(status, None))
    }
}

fn net_api_error(status: u32, parameter_error: Option<u32>) -> io::Error {
    let parameter = parameter_error
        .filter(|parameter| *parameter != 0)
        .map_or_else(String::new, |parameter| {
            format!(" (invalid parameter {parameter})")
        });
    io::Error::other(format!(
        "Windows local-group operation failed with status {status}{parameter}"
    ))
}

struct NetApiBuffer(*mut u8);

impl Drop for NetApiBuffer {
    #[allow(
        unsafe_code,
        reason = "NetApiBufferFree is the required destructor for buffers allocated by NetAPI"
    )]
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: this pointer was returned by NetLocalGroupGetMembers and
            // is released exactly once.
            let _ = unsafe { NetApiBufferFree(self.0.cast()) };
        }
    }
}

#[cfg(test)]
#[path = "local_group_tests.rs"]
mod tests;
