// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Windows security-identity helpers: named-pipe client impersonation and
//! SID string conversion.
//!
//! Shared between the live validation binary
//! (`examples/windows_named_pipe_identity.rs`) and `luminated`'s eventual
//! `Principal::Windows` construction, so the `unsafe` FFI wrappers exist in
//! exactly one place. Scope is deliberately narrow: this module proves that
//! a client SID can be captured from a live named-pipe connection. It does
//! not do connection pooling or request dispatch.

use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::os::windows::io::RawHandle;
use std::ptr;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INSUFFICIENT_BUFFER, HANDLE, HLOCAL, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSidToSidW};
use windows_sys::Win32::Security::{
    CreateWellKnownSid, GetTokenInformation, LookupAccountNameW, PSID, RevertToSelf,
    SID_AND_ATTRIBUTES, SID_NAME_USE, TOKEN_GROUPS, TOKEN_QUERY, TOKEN_USER, TokenGroups,
    TokenUser, WELL_KNOWN_SID_TYPE, WinInteractiveSid, WinLocalSystemSid,
};
use windows_sys::Win32::System::Pipes::{
    GetNamedPipeClientProcessId, GetNamedPipeServerProcessId, ImpersonateNamedPipeClient,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, OpenProcess, OpenProcessToken, OpenThreadToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::wide::{to_wide, wide_string_from_nul_terminated};

/// Account name whose service SID identifies an SCM-launched `luminated`.
pub const LUMINATED_SERVICE_ACCOUNT: &str = r"NT SERVICE\luminated";

// These flags live in windows-sys's broad SystemServices feature even though
// they describe `TOKEN_GROUPS`. Keeping the two values here avoids enabling a
// large unrelated feature for constants fixed by the Windows ABI.
const SE_GROUP_ENABLED: u32 = 0x0000_0004;
const SE_GROUP_USE_FOR_DENY_ONLY: u32 = 0x0000_0010;

/// Reverts a thread impersonation on drop, so an early return out of
/// [`captured_client_sid`] can never leave the server thread stuck running
/// as the client.
struct ImpersonationGuard;

impl Drop for ImpersonationGuard {
    #[allow(
        unsafe_code,
        reason = "RevertToSelf is a Win32 API with no safe standard-library wrapper; it takes no \
                  arguments, touches no memory Rust owns, and is always safe to call regardless of \
                  whether this thread is currently impersonating."
    )]
    fn drop(&mut self) {
        // SAFETY: `RevertToSelf` takes no arguments and is always safe to
        // call, whether or not this thread is currently impersonating. A
        // failure here has no memory-safety consequence, only a security
        // one, and there is nothing more this destructor can do about it.
        unsafe {
            RevertToSelf();
        }
    }
}

/// Returns the process ID of the client connected to `pipe`.
///
/// `pipe` must be a valid, open named-pipe server handle for the duration
/// of this call.
///
/// # Errors
///
/// Returns the underlying `io::Error` if the Win32 call fails.
#[allow(
    unsafe_code,
    reason = "GetNamedPipeClientProcessId is a Win32 API with no safe standard-library wrapper; \
              `pipe` is guaranteed valid for the call's duration by this function's documented \
              caller contract."
)]
pub fn named_pipe_client_process_id(pipe: RawHandle) -> io::Result<u32> {
    let mut pid = 0_u32;
    // SAFETY: `pipe` is a valid named-pipe server handle per the caller's
    // contract; `pid` is a local out-parameter of the size the API expects.
    let ok = unsafe { GetNamedPipeClientProcessId(pipe.cast::<c_void>(), &raw mut pid) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(pid)
}

/// Returns the process ID of the server connected to `pipe`.
///
/// `pipe` must be a valid, open named-pipe client handle for the duration
/// of this call.
///
/// # Errors
///
/// Returns the underlying `io::Error` if the Win32 call fails.
#[allow(
    unsafe_code,
    reason = "GetNamedPipeServerProcessId is a Win32 API with no safe standard-library wrapper; \
              `pipe` is guaranteed valid for the call's duration by this function's documented \
              caller contract."
)]
pub fn named_pipe_server_process_id(pipe: RawHandle) -> io::Result<u32> {
    let mut pid = 0_u32;
    // SAFETY: `pipe` is a valid named-pipe client handle per the caller's
    // contract; `pid` is a local out-parameter of the size the API expects.
    let ok = unsafe { GetNamedPipeServerProcessId(pipe.cast::<c_void>(), &raw mut pid) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(pid)
}

/// Returns the primary user SID of process `pid` in canonical string form.
///
/// # Errors
///
/// Returns the underlying `io::Error` if the process no longer exists or
/// its query-limited process handle or primary token cannot be opened.
#[allow(
    unsafe_code,
    reason = "OpenProcess, OpenProcessToken, and CloseHandle are Win32 APIs with no safe \
              standard-library wrapper; both handles are closed before returning and are only \
              used by calls documented to accept their respective handle types."
)]
pub fn process_sid(pid: u32) -> io::Result<String> {
    // SAFETY: `OpenProcess` accepts any process ID. Failure is represented
    // by a null handle and handled below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }

    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: `process` is the live process handle returned above; `token`
    // is a local out-parameter.
    let ok = unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) };
    if ok == 0 {
        let error = io::Error::last_os_error();
        // SAFETY: `process` is the handle returned by `OpenProcess` and is
        // not used again after this point.
        unsafe { CloseHandle(process) };
        return Err(error);
    }

    let sid = token_user_sid_string(token);
    // SAFETY: `token` was returned by the successful `OpenProcessToken`
    // call above and is not used again after being closed here.
    unsafe { CloseHandle(token) };
    // SAFETY: `process` was returned by the successful `OpenProcess` call
    // above and is not used again after being closed here.
    unsafe { CloseHandle(process) };
    sid
}

/// Returns whether process `pid` has `sid` as an enabled, non-deny-only group.
///
/// # Errors
///
/// Returns the underlying `io::Error` if the process or token cannot be
/// opened, its groups cannot be read, or Windows returns an invalid group
/// count.
#[allow(
    unsafe_code,
    reason = "OpenProcess, OpenProcessToken, GetTokenInformation, and CloseHandle are Win32 APIs \
              with no safe standard-library wrappers; handles are closed on every path and the \
              variable-length token buffer is bounded by the size Windows reports."
)]
pub fn process_has_enabled_group_sid(pid: u32, sid: &str) -> io::Result<bool> {
    process_sid_and_enabled_groups(pid).map(|(_, groups)| groups.iter().any(|group| group == sid))
}

/// Returns process `pid`'s user SID and enabled, non-deny-only group SIDs from
/// the same primary token snapshot.
///
/// Reading both facts through one token handle prevents a recycled process ID
/// from combining one process's user with another process's groups.
///
/// # Errors
///
/// Returns the underlying `io::Error` if the process or token cannot be
/// opened, its identity cannot be read, or Windows returns invalid token data.
#[allow(
    unsafe_code,
    reason = "OpenProcess, OpenProcessToken, and CloseHandle are Win32 APIs with no safe \
              standard-library wrappers; handles are closed on every path and both identity \
              queries use the same live token."
)]
pub fn process_sid_and_enabled_groups(pid: u32) -> io::Result<(String, Vec<String>)> {
    // SAFETY: `OpenProcess` accepts any process ID. A null result is handled
    // below without dereferencing it.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }

    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: `process` is live and `token` is a local out-parameter.
    let ok = unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) };
    if ok == 0 {
        let error = io::Error::last_os_error();
        // SAFETY: `process` is live and is not used after this close.
        unsafe { CloseHandle(process) };
        return Err(error);
    }

    let result = token_user_sid_string(token)
        .and_then(|user_sid| token_enabled_group_sids(token).map(|groups| (user_sid, groups)));
    // SAFETY: `token` is live and is not used after this close.
    unsafe { CloseHandle(token) };
    // SAFETY: `process` is live and is not used after this close.
    unsafe { CloseHandle(process) };
    result
}

#[allow(
    unsafe_code,
    reason = "GetTokenInformation returns a variable-length TOKEN_GROUPS value; the buffer is \
              sized by a probing call, the fixed header is read unaligned, and every group entry \
              is bounded by both the returned byte length and group count."
)]
fn token_enabled_group_sids(token: HANDLE) -> io::Result<Vec<String>> {
    let mut needed = 0_u32;
    // SAFETY: null with zero length is the documented size-probe form.
    unsafe { GetTokenInformation(token, TokenGroups, ptr::null_mut(), 0, &raw mut needed) };
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut buffer = vec![0_u8; needed as usize];
    // SAFETY: `buffer` has exactly the capacity reported by the probe.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenGroups,
            buffer.as_mut_ptr().cast(),
            needed,
            &raw mut needed,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: a successful TokenGroups query writes TOKEN_GROUPS at the
    // beginning. The allocation need not be aligned, hence read_unaligned.
    let groups = unsafe { buffer.as_ptr().cast::<TOKEN_GROUPS>().read_unaligned() };
    let entries_offset = std::mem::offset_of!(TOKEN_GROUPS, Groups);
    let available = (needed as usize)
        .checked_sub(entries_offset)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid token group buffer"))?;
    let maximum_groups = available / size_of::<SID_AND_ATTRIBUTES>();
    let group_count = usize::try_from(groups.GroupCount)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid token group count"))?;
    if group_count > maximum_groups {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "token group count exceeds the returned buffer",
        ));
    }

    let mut enabled_groups = Vec::with_capacity(group_count);
    for index in 0..group_count {
        let offset = entries_offset + index * size_of::<SID_AND_ATTRIBUTES>();
        // SAFETY: `offset` is bounded by the validated number of complete
        // entries, so this remains within `buffer`.
        let entry_bytes = unsafe { buffer.as_ptr().add(offset) };
        // This deliberately produces a possibly unaligned pointer which is
        // read only with `read_unaligned` immediately below.
        #[allow(
            clippy::cast_ptr_alignment,
            reason = "the pointer is consumed only by read_unaligned immediately below"
        )]
        let entry_pointer = entry_bytes.cast::<SID_AND_ATTRIBUTES>();
        // SAFETY: one complete entry begins at this validated offset and an
        // unaligned read makes no alignment assumption about `Vec<u8>`.
        let entry = unsafe { entry_pointer.read_unaligned() };
        if entry.Attributes & SE_GROUP_ENABLED != 0
            && entry.Attributes & SE_GROUP_USE_FOR_DENY_ONLY == 0
        {
            enabled_groups.push(sid_to_string(entry.Sid)?);
        }
    }

    Ok(enabled_groups)
}

/// Resolves a Windows account name to its canonical SID string.
///
/// # Errors
///
/// Returns the underlying `io::Error` if Windows cannot resolve `account` or
/// return its SID.
#[allow(
    unsafe_code,
    reason = "LookupAccountNameW is a Win32 API with no safe standard-library wrapper; both calls use local size and output parameters, and the second call's buffers have exactly the sizes reported by the first."
)]
pub fn account_sid(account: &str) -> io::Result<String> {
    let account = to_wide(account);
    let mut sid_size = 0_u32;
    let mut domain_size = 0_u32;
    let mut use_kind: SID_NAME_USE = 0;

    // SAFETY: null output buffers with zero sizes are the documented probe
    // form. The API writes only their required sizes and `use_kind`.
    unsafe {
        LookupAccountNameW(
            ptr::null(),
            account.as_ptr(),
            ptr::null_mut(),
            &raw mut sid_size,
            ptr::null_mut(),
            &raw mut domain_size,
            &raw mut use_kind,
        )
    };
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER.cast_signed()) || sid_size == 0 {
        return Err(error);
    }

    let mut sid = vec![0_u8; sid_size as usize];
    let mut domain = vec![0_u16; domain_size as usize];
    // SAFETY: both output buffers have exactly the sizes returned by the
    // probing call, and every other pointer refers to a live local value.
    let ok = unsafe {
        LookupAccountNameW(
            ptr::null(),
            account.as_ptr(),
            sid.as_mut_ptr().cast(),
            &raw mut sid_size,
            domain.as_mut_ptr(),
            &raw mut domain_size,
            &raw mut use_kind,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    sid_to_string(sid.as_mut_ptr().cast())
}

/// Validates and canonicalizes a string-form Windows SID.
///
/// # Errors
///
/// Returns the underlying Windows error when `sid` is malformed.
#[allow(
    unsafe_code,
    reason = "Windows owns the SID allocation returned by ConvertStringSidToSidW; the pointer is converted immediately and released with LocalFree."
)]
pub fn canonical_sid(sid: &str) -> io::Result<String> {
    let sid = to_wide(sid);
    let mut parsed: PSID = ptr::null_mut();
    // SAFETY: `sid` is NUL-terminated and `parsed` is a live output pointer.
    let ok = unsafe { ConvertStringSidToSidW(sid.as_ptr(), &raw mut parsed) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = sid_to_string(parsed);
    // SAFETY: ConvertStringSidToSidW allocated `parsed` with LocalAlloc.
    unsafe { LocalFree(parsed.cast::<c_void>()) };
    result
}

#[allow(
    unsafe_code,
    reason = "CreateWellKnownSid is a Win32 API with no safe standard-library wrapper; the output buffer is sized by a probing call before being populated."
)]
fn well_known_sid(kind: WELL_KNOWN_SID_TYPE) -> io::Result<String> {
    let mut sid_size = 0_u32;
    // SAFETY: a null output buffer is the documented size-probe form.
    unsafe { CreateWellKnownSid(kind, ptr::null_mut(), ptr::null_mut(), &raw mut sid_size) };
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER.cast_signed()) || sid_size == 0 {
        return Err(error);
    }

    let mut sid = vec![0_u8; sid_size as usize];
    // SAFETY: `sid` has the exact size reported by the probing call.
    let ok = unsafe {
        CreateWellKnownSid(
            kind,
            ptr::null_mut(),
            sid.as_mut_ptr().cast(),
            &raw mut sid_size,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    sid_to_string(sid.as_mut_ptr().cast())
}

/// Returns the well-known interactive-logon SID in canonical string form.
///
/// # Errors
///
/// Returns the underlying `io::Error` if Windows cannot construct the SID.
pub fn interactive_sid() -> io::Result<String> {
    well_known_sid(WinInteractiveSid)
}

/// Returns the `LocalSystem` SID in canonical string form.
///
/// # Errors
///
/// Returns the underlying `io::Error` if Windows cannot construct the SID.
pub fn local_system_sid() -> io::Result<String> {
    well_known_sid(WinLocalSystemSid)
}

/// Impersonates the client connected to `pipe` long enough to read back its
/// `TokenUser` SID in canonical string form (e.g. `S-1-5-21-...`), then
/// unconditionally reverts before returning, including on error.
///
/// `pipe` must be a valid, open named-pipe server handle for the duration
/// of this call.
///
/// # Errors
///
/// Returns the underlying `io::Error` if impersonation, the token lookup,
/// or SID conversion fails.
#[allow(
    unsafe_code,
    reason = "ImpersonateNamedPipeClient, GetCurrentThread, OpenThreadToken, and CloseHandle are \
              Win32 APIs with no safe standard-library wrapper; `pipe` is guaranteed valid for the \
              call's duration by this function's documented caller contract, and `token` is only \
              ever the handle this function itself just opened."
)]
pub fn captured_client_sid(pipe: RawHandle) -> io::Result<String> {
    // SAFETY: `pipe` is a valid named-pipe server handle per the caller's
    // contract.
    let ok = unsafe { ImpersonateNamedPipeClient(pipe.cast::<c_void>()) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let _guard = ImpersonationGuard;

    // SAFETY: `GetCurrentThread` returns a pseudo-handle that needs no
    // closing.
    let thread = unsafe { GetCurrentThread() };
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: `thread` is a valid pseudo-handle; `token` is a local
    // out-parameter.
    let ok = unsafe { OpenThreadToken(thread, TOKEN_QUERY, 0, &raw mut token) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let sid = token_user_sid_string(token);
    // SAFETY: `token` was returned by the successful `OpenThreadToken`
    // above and is not used again after this point.
    unsafe { CloseHandle(token) };
    sid
}

/// Returns the calling process's own SID in canonical string form, so a
/// caller can compare a captured client SID against it without repeating
/// this lookup itself (a token query is heavier than a Unix `getuid()`
/// call, so callers are expected to cache this).
///
/// # Errors
///
/// Returns the underlying `io::Error` if the Win32 call fails.
#[allow(
    unsafe_code,
    reason = "GetCurrentProcess, OpenProcessToken, and CloseHandle are Win32 APIs with no safe \
              standard-library wrapper; `token` is only ever the handle this function itself just \
              opened."
)]
pub fn current_process_sid() -> io::Result<String> {
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle that needs no
    // closing.
    let process = unsafe { GetCurrentProcess() };
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: `process` is a valid pseudo-handle; `token` is a local
    // out-parameter.
    let ok = unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let sid = token_user_sid_string(token);
    // SAFETY: `token` was returned by the successful `OpenProcessToken`
    // above and is not used again after this point.
    unsafe { CloseHandle(token) };
    sid
}

/// The daemon's own SID, computed once and cached for the life of the
/// process. A token query is comparatively heavy, unlike a Unix `getuid()`
/// call, and the daemon's own identity never changes while it is running,
/// so `luminated`'s `Principal::is_same_user_as_daemon` compares against
/// this rather than repeating the lookup.
///
/// # Panics
///
/// Panics if the underlying token lookup fails. A process unable to read
/// its own primary token indicates a broken host environment the daemon
/// cannot meaningfully continue in, not a per-call condition to propagate;
/// the same treatment the Unix side gives its own `getuid()` call, which is
/// documented as unable to fail rather than handled as fallible.
#[must_use]
#[allow(
    clippy::expect_used,
    reason = "A process's own primary token is always readable in practice; a failure here means \
              the host environment is broken in a way the daemon cannot recover from, matching how \
              the Unix side treats its own getuid() call as infallible."
)]
pub fn daemon_own_sid() -> &'static str {
    static SID: OnceLock<String> = OnceLock::new();
    SID.get_or_init(|| current_process_sid().expect("read the daemon's own process SID"))
}

/// Reads the `TokenUser` SID out of an open token handle and converts it to
/// its canonical string form.
#[allow(
    unsafe_code,
    reason = "GetTokenInformation, ConvertSidToStringSidW, and LocalFree are Win32 APIs with no \
              safe standard-library wrapper; the intermediate buffer is sized exactly to what the \
              probing call reports before it is ever read as a `TOKEN_USER`, and the allocation \
              from ConvertSidToStringSidW is freed exactly once, per its documented contract."
)]
fn token_user_sid_string(token: HANDLE) -> io::Result<String> {
    let mut needed = 0_u32;
    // SAFETY: a null buffer with zero length is the documented way to ask
    // `GetTokenInformation` for the required buffer size; on the expected
    // `ERROR_INSUFFICIENT_BUFFER` failure it writes only to `needed`.
    unsafe { GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &raw mut needed) };
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut buffer = vec![0_u8; needed as usize];
    // SAFETY: `buffer` is sized to exactly the `needed` length the probing
    // call above reported.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &raw mut needed,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful call filled `buffer` with a `TOKEN_USER` at its
    // start, per the documented contract for a `TokenUser`-class query, and
    // `buffer` is large enough to hold one by construction above. `buffer`
    // is a `Vec<u8>` with no alignment guarantee beyond one byte, so this
    // reads through an unaligned read rather than a direct dereference,
    // which would be undefined behaviour if the allocation happened to fall
    // on an address not aligned for `TOKEN_USER`.
    let sid = unsafe { buffer.as_ptr().cast::<TOKEN_USER>().read_unaligned() }
        .User
        .Sid;

    sid_to_string(sid)
}

#[allow(
    unsafe_code,
    reason = "ConvertSidToStringSidW and LocalFree are Win32 APIs with no safe standard-library wrapper; the input is a live SID owned by the caller, and the returned string allocation is read and freed exactly once."
)]
fn sid_to_string(sid: PSID) -> io::Result<String> {
    let mut string_sid = ptr::null_mut();
    // SAFETY: the caller keeps `sid` alive for this call; the string written
    // to `string_sid` is freed via `LocalFree` below, per
    // `ConvertSidToStringSidW`'s documented ownership contract.
    let ok = unsafe { ConvertSidToStringSidW(sid, &raw mut string_sid) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `string_sid` is non-null and NUL-terminated on a successful
    // call, and remains valid until freed immediately below.
    let result = unsafe { wide_string_from_nul_terminated(string_sid) };
    // SAFETY: `string_sid` was allocated by `ConvertSidToStringSidW`, which
    // documents `LocalFree` as the correct way to release it, and is not
    // used again after this call.
    unsafe { LocalFree(string_sid as HLOCAL) };
    Ok(result)
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
