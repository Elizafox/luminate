// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Registration of the `luminated` Windows Event Log source.

use std::env;
use std::io;
use std::path::PathBuf;
use std::ptr;

use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_CREATED_NEW_KEY, REG_DWORD,
    REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_VALUE_TYPE, RRF_NOEXPAND,
    RRF_RT_REG_DWORD, RRF_RT_REG_EXPAND_SZ, RegCreateKeyExW, RegDeleteKeyW, RegGetValueW,
    RegOpenKeyExW, RegSetValueExW,
};

use super::registry::{RegistryKey, byte_len, check_status};
use super::wide::to_wide;

/// A tracing layer that writes curated operator events to the `luminated`
/// Windows Event Log source.
///
/// Keeping the dependency wrapper here makes replacing the underlying Event
/// Log implementation a contained platform change.
pub struct EventLogLayer {
    inner: tracing_layer_win_eventlog::EventLogLayer,
    target: &'static str,
}

impl EventLogLayer {
    /// Opens the registered `luminated` Event Log source.
    ///
    /// # Errors
    ///
    /// Returns an error when Windows refuses to register a writer handle.
    pub fn new(target: &'static str) -> io::Result<Self> {
        tracing_layer_win_eventlog::EventLogLayer::new("luminated")
            .map(|inner| Self { inner, target })
            .map_err(io::Error::other)
    }
}

impl<S> tracing_subscriber::Layer<S> for EventLogLayer
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(&self, event: &tracing::Event<'_>, context: Context<'_, S>) {
        if event.metadata().target() == self.target {
            self.inner.on_event(event, context);
        }
    }
}

const SOURCE_KEY: &str = r"SYSTEM\CurrentControlSet\Services\EventLog\Application\luminated";
/// Stable name of the Windows Event Log source created by the installer.
pub const SOURCE_NAME: &str = "luminated";
const MESSAGE_FILE_VALUE: &str = "EventMessageFile";
const MESSAGE_FILE: &str =
    r"%SystemRoot%\Microsoft.NET\Framework64\v4.0.30319\EventLogMessages.dll";
const TYPES_SUPPORTED_VALUE: &str = "TypesSupported";
const TYPES_SUPPORTED: u32 = 0x7;

/// Outcome of ensuring that the `luminated` Event Log source exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnsureEventSourceOutcome {
    /// This invocation created and populated the source key.
    Created,
    /// A source with exactly the expected values already existed.
    AlreadyRegistered,
}

/// Creates the `luminated` Event Log source or verifies an existing source.
///
/// The source uses the in-box .NET Framework passthrough message table. The
/// target DLL is checked before the registry is modified. An existing source
/// with different values is preserved and reported as an error.
///
/// # Errors
///
/// Returns an error if the message-table DLL is unavailable, the source has
/// unexpected metadata, or Windows refuses a registry operation. Registration
/// ordinarily requires administrator privileges.
pub fn ensure_event_source() -> io::Result<EnsureEventSourceOutcome> {
    ensure_event_source_at(SOURCE_KEY)
}

fn ensure_event_source_at(source_key: &str) -> io::Result<EnsureEventSourceOutcome> {
    verify_message_file()?;

    if let Some(key) = open_source(source_key, KEY_QUERY_VALUE)? {
        let matches = source_values_match(key.raw())?;
        drop(key);
        if matches {
            return Ok(EnsureEventSourceOutcome::AlreadyRegistered);
        }

        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the luminated Event Log source exists with unexpected metadata",
        ));
    }

    let path = to_wide(source_key);
    let mut raw = ptr::null_mut();
    let mut disposition = 0;
    #[allow(
        unsafe_code,
        reason = "RegCreateKeyExW has no safe standard-library wrapper"
    )]
    // SAFETY: `path` is NUL-terminated, all output pointers refer to local
    // variables, and the returned key is immediately placed in an RAII guard.
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
            &raw mut disposition,
        )
    };
    check_status(status, "create the luminated Event Log source")?;
    let key = RegistryKey(raw);

    if disposition != REG_CREATED_NEW_KEY {
        return if source_values_match(key.raw())? {
            Ok(EnsureEventSourceOutcome::AlreadyRegistered)
        } else {
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "the luminated Event Log source was concurrently created with unexpected metadata",
            ))
        };
    }

    if let Err(error) = write_source_values(key.raw()) {
        drop(key);
        let _ = delete_source_key(source_key);
        return Err(error);
    }

    Ok(EnsureEventSourceOutcome::Created)
}

/// Reports whether the Event Log source exists with the expected metadata.
///
/// # Errors
///
/// Returns an error when Windows refuses to inspect the source.
pub fn event_source_is_registered() -> io::Result<bool> {
    event_source_is_registered_at(SOURCE_KEY)
}

/// Removes the `luminated` Event Log source if its values still match the
/// installer's registration.
///
/// An administrator-modified source is preserved and reported as a conflict.
/// Absence is treated as success.
///
/// # Errors
///
/// Returns an error when the source has unexpected metadata or Windows refuses
/// a registry operation.
pub fn remove_event_source() -> io::Result<()> {
    let Some(key) = open_source(SOURCE_KEY, KEY_QUERY_VALUE)? else {
        return Ok(());
    };
    let matches = source_values_match(key.raw())?;
    drop(key);
    if !matches {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the luminated Event Log source has unexpected metadata",
        ));
    }

    delete_source_key(SOURCE_KEY)
}

fn event_source_is_registered_at(source_key: &str) -> io::Result<bool> {
    open_source(source_key, KEY_QUERY_VALUE)?
        .map_or(Ok(false), |key| source_values_match(key.raw()))
}

fn verify_message_file() -> io::Result<()> {
    let system_root = env::var_os("SystemRoot")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "SystemRoot is not defined"))?;
    let path = PathBuf::from(system_root)
        .join(r"Microsoft.NET\Framework64\v4.0.30319\EventLogMessages.dll");
    if path.is_file() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "Event Log message table does not exist at {}",
                path.display()
            ),
        ))
    }
}

fn open_source(source_key: &str, access: REG_SAM_FLAGS) -> io::Result<Option<RegistryKey>> {
    let path = to_wide(source_key);
    let mut raw = ptr::null_mut();
    #[allow(
        unsafe_code,
        reason = "RegOpenKeyExW has no safe standard-library wrapper"
    )]
    // SAFETY: `path` is NUL-terminated and `raw` is a valid output pointer.
    let status =
        unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, access, &raw mut raw) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check_status(status, "open the luminated Event Log source")?;
    Ok(Some(RegistryKey(raw)))
}

fn write_source_values(key: HKEY) -> io::Result<()> {
    let message_name = to_wide(MESSAGE_FILE_VALUE);
    let message_file = to_wide(MESSAGE_FILE);
    let message_bytes = byte_len(&message_file)?;
    #[allow(
        unsafe_code,
        reason = "RegSetValueExW has no safe standard-library wrapper"
    )]
    // SAFETY: both strings are NUL-terminated and remain alive for the call;
    // `message_bytes` describes the complete UTF-16 value including its NUL.
    let status = unsafe {
        RegSetValueExW(
            key,
            message_name.as_ptr(),
            0,
            REG_EXPAND_SZ,
            message_file.as_ptr().cast(),
            message_bytes,
        )
    };
    check_status(status, "write the EventMessageFile value")?;

    let types_name = to_wide(TYPES_SUPPORTED_VALUE);
    let types = TYPES_SUPPORTED.to_ne_bytes();
    #[allow(
        unsafe_code,
        reason = "RegSetValueExW has no safe standard-library wrapper"
    )]
    // SAFETY: `types` is a four-byte DWORD buffer that remains alive for the call.
    let status = unsafe {
        RegSetValueExW(
            key,
            types_name.as_ptr(),
            0,
            REG_DWORD,
            types.as_ptr(),
            u32::try_from(types.len()).map_err(io::Error::other)?,
        )
    };
    check_status(status, "write the TypesSupported value")
}

fn source_values_match(key: HKEY) -> io::Result<bool> {
    Ok(read_value(key, MESSAGE_FILE_VALUE, REG_EXPAND_SZ)?
        == to_wide(MESSAGE_FILE)
            .into_iter()
            .flat_map(u16::to_ne_bytes)
            .collect::<Vec<_>>()
        && read_value(key, TYPES_SUPPORTED_VALUE, REG_DWORD)? == TYPES_SUPPORTED.to_ne_bytes())
}

fn read_value(key: HKEY, name: &str, expected_type: REG_VALUE_TYPE) -> io::Result<Vec<u8>> {
    let name = to_wide(name);
    let flags = if expected_type == REG_EXPAND_SZ {
        RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND
    } else {
        RRF_RT_REG_DWORD
    };
    let mut actual_type = 0;
    let mut size = 0;
    #[allow(
        unsafe_code,
        reason = "RegGetValueW has no safe standard-library wrapper"
    )]
    // SAFETY: the name is NUL-terminated and both output pointers are valid.
    let status = unsafe {
        RegGetValueW(
            key,
            ptr::null(),
            name.as_ptr(),
            flags,
            &raw mut actual_type,
            ptr::null_mut(),
            &raw mut size,
        )
    };
    check_status(status, "read Event Log source metadata")?;
    if actual_type != expected_type {
        return Ok(Vec::new());
    }

    let mut value = vec![0_u8; usize::try_from(size).map_err(io::Error::other)?];
    #[allow(
        unsafe_code,
        reason = "RegGetValueW has no safe standard-library wrapper"
    )]
    // SAFETY: `value` has `size` writable bytes and all other pointers remain valid.
    let status = unsafe {
        RegGetValueW(
            key,
            ptr::null(),
            name.as_ptr(),
            flags,
            &raw mut actual_type,
            value.as_mut_ptr().cast(),
            &raw mut size,
        )
    };
    check_status(status, "read Event Log source metadata")?;
    value.truncate(usize::try_from(size).map_err(io::Error::other)?);
    Ok(value)
}

fn delete_source_key(source_key: &str) -> io::Result<()> {
    let path = to_wide(source_key);
    #[allow(
        unsafe_code,
        reason = "RegDeleteKeyW has no safe standard-library wrapper"
    )]
    // SAFETY: `path` is a NUL-terminated registry path.
    let status = unsafe { RegDeleteKeyW(HKEY_LOCAL_MACHINE, path.as_ptr()) };
    check_status(status, "roll back the luminated Event Log source")
}

#[cfg(test)]
#[path = "event_log_tests.rs"]
mod tests;
