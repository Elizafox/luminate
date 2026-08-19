// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Ownership metadata for objects created by the Windows service installer.

use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
    RRF_RT_REG_SZ, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};

use super::registry::{RegistryKey, byte_len, check_status};
use super::wide::to_wide;

const INSTALLER_KEY: &str = r"Software\Luminate";
const OWNED_CLIENT_GROUP_VALUE: &str = "InstallerOwnedClientGroup";
const OWNED_EVENT_LOG_SOURCE_VALUE: &str = "InstallerOwnedEventLogSource";

/// Records that the installer created the named client group.
///
/// An existing marker for another group is preserved and reported as an
/// ownership conflict.
///
/// # Errors
///
/// Returns an error when the marker conflicts or Windows refuses the registry
/// operation. Writing under `HKEY_LOCAL_MACHINE` ordinarily requires
/// administrator privileges.
pub fn record_owned_client_group(group: &str) -> io::Result<()> {
    record_owned_object(
        OWNED_CLIENT_GROUP_VALUE,
        group,
        "client-group ownership metadata names a different group",
        "record client-group ownership",
    )
}

/// Reports whether the installer owns the named client group.
///
/// # Errors
///
/// Returns an error when Windows refuses to inspect the registry or the stored
/// value is not valid UTF-16.
pub fn owns_client_group(group: &str) -> io::Result<bool> {
    owns_object(OWNED_CLIENT_GROUP_VALUE, group)
}

/// Removes the ownership marker after the installer-owned group is deleted.
///
/// # Errors
///
/// Returns an error when the marker names another group or Windows refuses the
/// registry operation.
pub fn remove_owned_client_group(group: &str) -> io::Result<()> {
    remove_owned_object(
        OWNED_CLIENT_GROUP_VALUE,
        group,
        "client-group ownership metadata names a different group",
        "remove client-group ownership",
    )
}

/// Records that the installer created the named Event Log source.
///
/// An existing marker for another source is preserved and reported as an
/// ownership conflict.
///
/// # Errors
///
/// Returns an error when the marker conflicts or Windows refuses the registry
/// operation. Writing under `HKEY_LOCAL_MACHINE` ordinarily requires
/// administrator privileges.
pub fn record_owned_event_log_source(source: &str) -> io::Result<()> {
    record_owned_object(
        OWNED_EVENT_LOG_SOURCE_VALUE,
        source,
        "Event Log source ownership metadata names a different source",
        "record Event Log source ownership",
    )
}

/// Reports whether the installer owns the named Event Log source.
///
/// # Errors
///
/// Returns an error when Windows refuses to inspect the registry or the stored
/// value is not valid UTF-16.
pub fn owns_event_log_source(source: &str) -> io::Result<bool> {
    owns_object(OWNED_EVENT_LOG_SOURCE_VALUE, source)
}

/// Removes the ownership marker after the installer-owned Event Log source is
/// deleted.
///
/// # Errors
///
/// Returns an error when the marker names another source or Windows refuses
/// the registry operation.
pub fn remove_owned_event_log_source(source: &str) -> io::Result<()> {
    remove_owned_object(
        OWNED_EVENT_LOG_SOURCE_VALUE,
        source,
        "Event Log source ownership metadata names a different source",
        "remove Event Log source ownership",
    )
}

fn record_owned_object(
    value_name: &str,
    value: &str,
    conflict: &'static str,
    operation: &str,
) -> io::Result<()> {
    let key = create_installer_key()?;
    match read_string(key.raw(), value_name)? {
        Some(existing) if existing == value => return Ok(()),
        Some(existing) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{conflict}: expected {value:?}, found {existing:?}"),
            ));
        }
        None => {}
    }

    let name = to_wide(value_name);
    let value = to_wide(value);
    let value_bytes = byte_len(&value)?;
    #[allow(
        unsafe_code,
        reason = "RegSetValueExW has no safe standard-library wrapper"
    )]
    // SAFETY: the value name and data are NUL-terminated UTF-16 buffers that
    // remain live for the synchronous call.
    let status = unsafe {
        RegSetValueExW(
            key.raw(),
            name.as_ptr(),
            0,
            REG_SZ,
            value.as_ptr().cast(),
            value_bytes,
        )
    };
    check_status(status, operation)
}

fn owns_object(value_name: &str, value: &str) -> io::Result<bool> {
    let Some(key) = open_installer_key(KEY_QUERY_VALUE)? else {
        return Ok(false);
    };
    Ok(read_string(key.raw(), value_name)?.as_deref() == Some(value))
}

fn remove_owned_object(
    value_name: &str,
    value: &str,
    conflict: &'static str,
    operation: &str,
) -> io::Result<()> {
    let Some(key) = open_installer_key(KEY_QUERY_VALUE | KEY_SET_VALUE)? else {
        return Ok(());
    };
    match read_string(key.raw(), value_name)? {
        Some(existing) if existing == value => {}
        Some(existing) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{conflict}: expected {value:?}, found {existing:?}"),
            ));
        }
        None => return Ok(()),
    }

    let name = to_wide(value_name);
    #[allow(
        unsafe_code,
        reason = "RegDeleteValueW has no safe standard-library wrapper"
    )]
    // SAFETY: the value name is a NUL-terminated UTF-16 buffer and the key
    // remains live for the synchronous call.
    let status = unsafe { RegDeleteValueW(key.raw(), name.as_ptr()) };
    check_status(status, operation)
}

fn create_installer_key() -> io::Result<RegistryKey> {
    let path = to_wide(INSTALLER_KEY);
    let mut raw = ptr::null_mut();
    #[allow(
        unsafe_code,
        reason = "RegCreateKeyExW has no safe standard-library wrapper"
    )]
    // SAFETY: the path is NUL-terminated and the output pointer refers to a
    // local variable. The returned key is immediately placed in an RAII guard.
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            path.as_ptr(),
            0,
            ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            ptr::null(),
            &raw mut raw,
            ptr::null_mut(),
        )
    };
    check_status(status, "open Luminate installer metadata")?;
    Ok(RegistryKey(raw))
}

fn open_installer_key(access: u32) -> io::Result<Option<RegistryKey>> {
    let path = to_wide(INSTALLER_KEY);
    let mut raw = ptr::null_mut();
    #[allow(
        unsafe_code,
        reason = "RegOpenKeyExW has no safe standard-library wrapper"
    )]
    // SAFETY: the path is NUL-terminated and `raw` is a valid output pointer.
    let status =
        unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, access, &raw mut raw) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check_status(status, "open Luminate installer metadata")?;
    Ok(Some(RegistryKey(raw)))
}

fn read_string(key: HKEY, value_name: &str) -> io::Result<Option<String>> {
    let name = to_wide(value_name);
    let mut size = 0_u32;
    #[allow(
        unsafe_code,
        reason = "RegGetValueW has no safe standard-library wrapper"
    )]
    // SAFETY: the value name is NUL-terminated and `size` is a valid output.
    let status = unsafe {
        RegGetValueW(
            key,
            ptr::null(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut size,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check_status(status, "read installer ownership metadata")?;

    let mut value = vec![0_u16; usize::try_from(size).map_err(io::Error::other)? / 2];
    #[allow(
        unsafe_code,
        reason = "RegGetValueW has no safe standard-library wrapper"
    )]
    // SAFETY: `value` has the byte size returned by the probe and remains
    // writable for the call.
    let status = unsafe {
        RegGetValueW(
            key,
            ptr::null(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            value.as_mut_ptr().cast(),
            &raw mut size,
        )
    };
    check_status(status, "read installer ownership metadata")?;
    value.truncate(usize::try_from(size).map_err(io::Error::other)? / size_of::<u16>());
    let value = value.strip_suffix(&[0]).unwrap_or(&value);
    String::from_utf16(value)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
#[path = "installer_metadata_tests.rs"]
mod tests;
