// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Shared DACL construction and owner-only verification, expressed as SDDL
//! (Security Descriptor Definition Language) strings rather than by walking
//! ACL/ACE structures by hand.
//!
//! Used by both [`crate::windows::secure_storage`] (directories and files)
//! and [`crate::windows::transport`] (named pipes). Private storage remains
//! owner-only, while service pipes additionally admit administrators and one
//! configured client principal. Keeping both postures here prevents their
//! security-sensitive access masks and ACE ordering from drifting apart.
//! Owner-only DACLs are compared after Windows renders both descriptors into
//! canonical SDDL. This preserves exact comparison while accounting for
//! well-known SID aliases such as `LA` for the local Administrator account.
//!
//! [`grant_process_query_access`] is the one function here that edits
//! existing DACLs by ACE rather than constructing one from SDDL: unlike a
//! pipe, whose whole security descriptor this code owns from creation, the
//! daemon's own process object and its primary token already have whatever
//! DACL Windows assigned them by default, and only one ACE needs adding to
//! each.

use std::ffi::c_void;
use std::fs::create_dir_all;
use std::io;
use std::mem::size_of;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW,
    EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, GetSecurityInfo, NO_MULTIPLE_TRUSTEE,
    SDDL_REVISION_1, SE_FILE_OBJECT, SE_KERNEL_OBJECT, SetEntriesInAclW, SetNamedSecurityInfoW,
    SetSecurityInfo, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, EqualSid, GetSecurityDescriptorDacl, NO_INHERITANCE,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_ATTRIBUTES, TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{READ_CONTROL, WRITE_DAC};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::windows::identity::current_process_sid;
use crate::windows::wide::{to_wide, wide_string_from_nul_terminated};

/// Named-pipe client rights, with `FILE_CREATE_PIPE_INSTANCE` deliberately
/// absent so an ordinary client cannot create another server instance.
const SERVICE_PIPE_CLIENT_ACCESS: u32 = 0x0012_019b;
const FILE_CREATE_PIPE_INSTANCE: u32 = 0x0000_0004;
const _: () = assert!(
    SERVICE_PIPE_CLIENT_ACCESS & FILE_CREATE_PIPE_INSTANCE == 0,
    "ordinary pipe clients must not be able to create server instances"
);

/// Frees a `PSECURITY_DESCRIPTOR`/wide-string buffer allocated by a Win32
/// security or SDDL conversion API on drop, so an early return can never
/// leak it.
pub(crate) struct LocalAllocGuard(pub(crate) *mut c_void);

impl Drop for LocalAllocGuard {
    #[allow(
        unsafe_code,
        reason = "LocalFree is a Win32 API with no safe standard-library wrapper; `self.0` is \
                  always either null (skipped) or a pointer this guard exclusively owns, allocated \
                  by exactly one of the LocalAlloc-family security/SDDL conversion APIs below."
    )]
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` was allocated by a `LocalAlloc`-family Win32
            // API (`ConvertStringSecurityDescriptorToSecurityDescriptorW`,
            // `ConvertSecurityDescriptorToStringSecurityDescriptorW`,
            // `GetNamedSecurityInfoW`, `GetSecurityInfo`,
            // `ConvertStringSidToSidW`, or `SetEntriesInAclW`) and is freed
            // here exactly once.
            unsafe { LocalFree(self.0.cast::<c_void>() as HLOCAL) };
        }
    }
}

/// The single-ACE SDDL string this module treats as "private": full access
/// for `owner_sid`, protected so no inherited ACE from a parent container
/// can widen it.
fn owner_only_sddl(owner_sid: &str) -> String {
    format!("D:P(A;;FA;;;{owner_sid})")
}

/// The owner-only creation descriptor explicitly names its owner because an
/// elevated token may otherwise default new objects to the Administrators
/// group rather than the current user.
fn owner_only_creation_sddl(owner_sid: &str) -> String {
    format!("O:{owner_sid}{}", owner_only_sddl(owner_sid))
}

/// The service named-pipe DACL: full access for the service identity and
/// administrators, and read/write access without pipe-instance creation for
/// the configured client principal.
fn service_pipe_sddl(service_sid: &str, client_sid: &str) -> String {
    format!(
        "D:P(A;;FA;;;{service_sid})(A;;FA;;;BA)(A;;{SERVICE_PIPE_CLIENT_ACCESS:#x};;;{client_sid})"
    )
}

/// The machine-wide data root's DACL: full access for `LocalSystem` (`SY`,
/// which the service runs as) and `Administrators` (`BA`), protected so no
/// inherited ACE from `%ProgramData%` itself (which by default grants
/// ordinary users limited write access so unprivileged installers can
/// create their own subtree) survives underneath it. The client group is
/// not named and receives no filesystem access anywhere in this tree.
///
/// Each ACE carries `OICI` (object-inherit, container-inherit) so the same
/// grant propagates to every file and subdirectory underneath. This includes
/// children that existed before the DACL was applied because
/// `SetNamedSecurityInfoW` immediately re-derives already-inherited ACEs on
/// existing children from the parent's new inheritable ACEs.
///
/// Without `OICI`, the grants apply only to the root. Applying that DACL to
/// an existing tree would strip inherited access from children such as the
/// active log directory, turning a security fix into an outage.
/// `secure_storage`'s owner-only directories remain narrower by replacing
/// their own DACL outright.
fn machine_data_root_sddl() -> &'static str {
    "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"
}

#[allow(
    unsafe_code,
    reason = "ConvertStringSecurityDescriptorToSecurityDescriptorW is a Win32 API with no safe \
              standard-library wrapper; the output pointer is a local out-parameter and the wide \
              SDDL string outlives the call."
)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "SECURITY_ATTRIBUTES is a small, fixed-size struct whose size can never approach \
              u32::MAX."
)]
fn security_attributes(sddl: &str) -> io::Result<(SECURITY_ATTRIBUTES, LocalAllocGuard)> {
    let sddl = to_wide(sddl);

    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `sddl` is a valid, NUL-terminated wide string for the duration
    // of this call; `descriptor` is a local out-parameter this function alone
    // writes to.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &raw mut descriptor,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    let guard = LocalAllocGuard(descriptor);
    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>())
            .expect("SECURITY_ATTRIBUTES is far smaller than u32::MAX"),
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    Ok((attributes, guard))
}

/// Builds a `SECURITY_ATTRIBUTES` granting full access to the current
/// process's own user and no one else, for use with `CreateDirectoryW`/
/// `CreateFileW`/`CreateNamedPipeW`. The returned guard owns the underlying
/// security descriptor allocation and must outlive the creation call.
pub(crate) fn owner_only_security_attributes() -> io::Result<(SECURITY_ATTRIBUTES, LocalAllocGuard)>
{
    let owner_sid = current_process_sid()?;
    security_attributes(&owner_only_creation_sddl(&owner_sid))
}

/// Builds security attributes for a service named pipe. The service and
/// administrators receive full access; `client_sid` receives only the rights
/// needed to connect and exchange data, not the right to create a pipe
/// instance. The returned guard must outlive the pipe creation call.
pub(crate) fn service_pipe_security_attributes(
    service_sid: &str,
    client_sid: &str,
) -> io::Result<(SECURITY_ATTRIBUTES, LocalAllocGuard)> {
    security_attributes(&service_pipe_sddl(service_sid, client_sid))
}

/// Creates `path` if absent and replaces its DACL with the machine-data-root
/// posture (`LocalSystem` and `Administrators` only, protected against
/// inherited ACEs), regardless of whatever ACL an earlier install or the
/// parent `%ProgramData%` container left there. Idempotent: safe to call on
/// every install, not only a fresh one.
///
/// # Errors
///
/// Returns the underlying `io::Error` if the directory cannot be created, the
/// fixed SDDL cannot be parsed, or the new DACL cannot be applied.
#[allow(
    unsafe_code,
    reason = "ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityDescriptorDacl, \
              and SetNamedSecurityInfoW are Win32 APIs with no safe standard-library wrapper. Every \
              out-parameter is a local variable this function alone writes to, and the security \
              descriptor allocation is freed exactly once via its guard; the DACL pointer it yields \
              points inside that single allocation rather than owning one of its own, so it is never \
              freed directly."
)]
pub fn protect_machine_data_root(path: &Path) -> io::Result<()> {
    create_dir_all(path)?;

    let sddl = to_wide(machine_data_root_sddl());
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `sddl` is a valid, NUL-terminated wide string for the duration
    // of this call; `descriptor` is a local out-parameter this function alone
    // writes to.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &raw mut descriptor,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let descriptor_guard = LocalAllocGuard(descriptor);

    let mut dacl: *mut ACL = ptr::null_mut();
    let mut dacl_present = 0;
    let mut dacl_defaulted = 0;
    // SAFETY: `descriptor` was just populated by the successful conversion
    // above and remains valid until `descriptor_guard` drops, after this
    // call; every out-parameter is local and written only here. The fixed
    // SDDL above always encodes a DACL, so `dacl` is non-null on success.
    let ok = unsafe {
        GetSecurityDescriptorDacl(
            descriptor,
            &raw mut dacl_present,
            &raw mut dacl,
            &raw mut dacl_defaulted,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    let wide_path = to_wide(&path.display().to_string());
    // SAFETY: `wide_path` is a valid, NUL-terminated wide string for the
    // duration of this call; `dacl` is valid because `descriptor_guard` has
    // not yet dropped; every other security-information pointer is
    // explicitly opted out of with null.
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide_path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            dacl,
            ptr::null_mut(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(
            i32::try_from(status).unwrap_or(i32::MAX),
        ));
    }

    drop(descriptor_guard);
    Ok(())
}

/// Reads back the DACL Windows actually attached to the filesystem object at
/// `path` and renders it as an SDDL string, for comparison against the
/// canonical owner-only descriptor this module writes.
#[allow(
    unsafe_code,
    reason = "GetNamedSecurityInfoW and ConvertSecurityDescriptorToStringSecurityDescriptorW are \
              Win32 APIs with no safe standard-library wrapper; both out-parameters are local and \
              written only by the call that owns them, and both LocalAlloc-family allocations are \
              released by their own guard."
)]
fn read_dacl_sddl(path: &Path) -> io::Result<String> {
    let wide_path = to_wide(&path.display().to_string());

    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `wide_path` is a valid, NUL-terminated wide string for the
    // duration of this call; every out-parameter but `descriptor` is
    // explicitly opted out of with a null pointer, and `descriptor` is a
    // local out-parameter this function alone writes to.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide_path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(
            i32::try_from(status).unwrap_or(i32::MAX),
        ));
    }
    let descriptor_guard = LocalAllocGuard(descriptor);

    let mut sddl_ptr: *mut u16 = ptr::null_mut();
    // SAFETY: `descriptor` was just populated by the successful
    // `GetNamedSecurityInfoW` call above and remains valid until
    // `descriptor_guard` drops, after this call returns; `sddl_ptr` is a
    // local out-parameter this function alone writes to.
    let ok = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &raw mut sddl_ptr,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let sddl_guard = LocalAllocGuard(sddl_ptr.cast::<c_void>());

    // SAFETY: `sddl_ptr` was just populated by the successful conversion
    // call above and is NUL-terminated per its documented contract; it
    // remains valid until `sddl_guard` drops, after this read.
    let sddl = unsafe { wide_string_from_nul_terminated(sddl_ptr) };

    drop(sddl_guard);
    drop(descriptor_guard);
    Ok(sddl)
}

/// Renders the expected owner-only descriptor through the same Windows API
/// used for an object's actual DACL. Windows may replace a well-known SID
/// with its SDDL alias while rendering, so comparing against the input text
/// would reject an otherwise identical descriptor.
#[allow(
    unsafe_code,
    reason = "ConvertSecurityDescriptorToStringSecurityDescriptorW is a Win32 API with no safe \
              standard-library wrapper; the descriptor and output allocation remain guarded for \
              the complete conversion."
)]
fn canonical_owner_only_sddl(owner_sid: &str) -> io::Result<String> {
    let (attributes, _descriptor_guard) = security_attributes(&owner_only_sddl(owner_sid))?;
    let mut sddl_ptr: *mut u16 = ptr::null_mut();
    // SAFETY: `attributes.lpSecurityDescriptor` remains valid through
    // `_descriptor_guard`; `sddl_ptr` is a local out-parameter.
    let ok = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            attributes.lpSecurityDescriptor,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &raw mut sddl_ptr,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let sddl_guard = LocalAllocGuard(sddl_ptr.cast::<c_void>());
    // SAFETY: the successful conversion returned a NUL-terminated string
    // which remains alive through `sddl_guard`.
    let sddl = unsafe { wide_string_from_nul_terminated(sddl_ptr) };

    drop(sddl_guard);
    Ok(sddl)
}

/// Verifies `path`'s DACL grants access to the current user only, erroring
/// if it grants anything to anyone else. Deliberately does not attempt to
/// repair a mismatched DACL: an unexpectedly permissive directory or file
/// is treated as a possible misconfiguration or compromise, not something
/// to silently tighten, mirroring the Unix implementation's refusal to
/// re-`chmod` an existing directory.
pub(crate) fn verify_private(path: &Path) -> io::Result<()> {
    let owner_sid = current_process_sid()?;
    let expected = canonical_owner_only_sddl(&owner_sid)?;
    let actual = read_dacl_sddl(path)?;
    if actual != expected {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} has an unexpected access control list (found `{actual}`, expected only owner \
                 access `{expected}`)",
                path.display()
            ),
        ));
    }
    Ok(())
}

/// Verifies the owner and DACL of an already-open filesystem object.
///
/// Handle-relative verification binds the checks to the object the caller
/// will use even if its directory entry is concurrently replaced.
#[allow(
    unsafe_code,
    reason = "GetSecurityInfo, ConvertStringSidToSidW, EqualSid, and the SDDL conversion APIs are Win32 APIs; all pointers come from live handles or guarded allocations"
)]
pub(crate) fn verify_private_handle(handle: HANDLE, path: &Path) -> io::Result<()> {
    let mut actual_owner: PSID = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `handle` remains valid for this call and every output pointer is
    // local. `actual_owner` points within `descriptor` and is not freed alone.
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &raw mut actual_owner,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(
            i32::try_from(status).unwrap_or(i32::MAX),
        ));
    }
    let descriptor_guard = LocalAllocGuard(descriptor);

    let owner_sid = current_process_sid()?;
    let owner_sid_wide = to_wide(&owner_sid);
    let mut expected_owner: PSID = ptr::null_mut();
    // SAFETY: the SID string is NUL-terminated and the output allocation is
    // captured by its guard immediately after success.
    let ok = unsafe { ConvertStringSidToSidW(owner_sid_wide.as_ptr(), &raw mut expected_owner) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let expected_owner_guard = LocalAllocGuard(expected_owner);
    // SAFETY: both SID pointers are valid until their guards drop.
    if unsafe { EqualSid(actual_owner, expected_owner) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not owned by the current user", path.display()),
        ));
    }

    let mut sddl_ptr: *mut u16 = ptr::null_mut();
    // SAFETY: `descriptor` remains valid through `descriptor_guard`; the
    // output is a local allocation captured by a guard below.
    let ok = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &raw mut sddl_ptr,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let sddl_guard = LocalAllocGuard(sddl_ptr.cast::<c_void>());
    // SAFETY: the successful conversion returned a NUL-terminated string
    // which remains alive through `sddl_guard`.
    let actual = unsafe { wide_string_from_nul_terminated(sddl_ptr) };
    let expected = canonical_owner_only_sddl(&owner_sid)?;
    if actual != expected {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} has an unexpected access control list (found `{actual}`, expected only owner access `{expected}`)",
                path.display()
            ),
        ));
    }

    drop(sddl_guard);
    drop(expected_owner_guard);
    drop(descriptor_guard);
    Ok(())
}

/// Adds one ACE granting `access_mask` to `client_sid` on the kernel object
/// behind `handle`, merged into whatever DACL is already there rather than
/// replacing it. `handle` must already carry `READ_CONTROL` and `WRITE_DAC`
/// access, since reading the existing DACL and applying the merged one both
/// need them.
///
/// # Errors
///
/// Returns the underlying `io::Error` if `client_sid` cannot be parsed, the
/// object's existing DACL cannot be read, the merged ACL cannot be built, or
/// the new DACL cannot be applied.
#[allow(
    unsafe_code,
    reason = "GetSecurityInfo, ConvertStringSidToSidW, SetEntriesInAclW, and SetSecurityInfo are \
              Win32 APIs with no safe standard-library wrapper. Every out-parameter is a local \
              variable this function alone writes to, the SID and the two ACL allocations are each \
              freed exactly once via their own guard, and `existing_dacl` is never freed directly \
              because it points inside `descriptor`'s single allocation rather than owning one of \
              its own."
)]
fn add_ace_to_kernel_object_dacl(
    handle: HANDLE,
    access_mask: u32,
    client_sid: &str,
) -> io::Result<()> {
    let mut existing_dacl: *mut ACL = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `handle` is valid for the duration of this call per the
    // caller's documented contract; every out-parameter but `existing_dacl`
    // and `descriptor` is explicitly opted out of with a null pointer, and
    // both are local out-parameters this call alone writes to.
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut existing_dacl,
            ptr::null_mut(),
            &raw mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(
            i32::try_from(status).unwrap_or(i32::MAX),
        ));
    }
    let descriptor_guard = LocalAllocGuard(descriptor);

    let client_sid_wide = to_wide(client_sid);
    let mut client_binary_sid: PSID = ptr::null_mut();
    // SAFETY: `client_sid_wide` is a valid, NUL-terminated wide string for
    // the duration of this call; `client_binary_sid` is a local out-parameter.
    let ok =
        unsafe { ConvertStringSidToSidW(client_sid_wide.as_ptr(), &raw mut client_binary_sid) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let sid_guard = LocalAllocGuard(client_binary_sid);

    // `TRUSTEE_W::ptstrName` is documented to instead hold a `PSID` when
    // `TrusteeForm` is `TRUSTEE_IS_SID`; this is a Win32 API-level
    // reinterpretation of the field, not a type mismatch.
    let trustee = TRUSTEE_W {
        pMultipleTrustee: ptr::null_mut(),
        MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
        TrusteeForm: TRUSTEE_IS_SID,
        TrusteeType: TRUSTEE_IS_UNKNOWN,
        ptstrName: client_binary_sid.cast(),
    };
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: access_mask,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: trustee,
    };

    let mut new_dacl: *mut ACL = ptr::null_mut();
    // SAFETY: `entry` is a single, fully initialized `EXPLICIT_ACCESS_W`
    // valid for the call; `existing_dacl` is still valid because
    // `descriptor_guard` has not yet dropped; `new_dacl` is a local
    // out-parameter.
    let status = unsafe { SetEntriesInAclW(1, &raw const entry, existing_dacl, &raw mut new_dacl) };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(
            i32::try_from(status).unwrap_or(i32::MAX),
        ));
    }
    let new_dacl_guard = LocalAllocGuard(new_dacl.cast());

    // SAFETY: `handle` is valid per the caller's contract; `new_dacl` is
    // valid because `new_dacl_guard` has not yet dropped; every other
    // security-information pointer is explicitly opted out of with null.
    let status = unsafe {
        SetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            new_dacl,
            ptr::null_mut(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(
            i32::try_from(status).unwrap_or(i32::MAX),
        ));
    }

    drop(new_dacl_guard);
    drop(sid_guard);
    drop(descriptor_guard);
    Ok(())
}

/// Grants `client_sid` `PROCESS_QUERY_LIMITED_INFORMATION` on the current
/// process and `TOKEN_QUERY` on its primary token, so a client authenticated
/// only by the pipe DACL can go on to open this process, open its token, and
/// read that token back to confirm it is really talking to `luminated`.
///
/// The pipe DACL and these two grants answer different questions. Connecting
/// to the pipe proves `client_sid` is allowed to talk to something named
/// `luminated`; opening the daemon's own process and reading its token is
/// what proves that something really is the daemon (service SID and all)
/// rather than an impostor that squatted the pipe name before the real
/// daemon started (see the module doc on
/// [`crate::windows::transport::authenticate_server`]).
///
/// The process and its primary token are separate kernel objects with
/// separate DACLs. For a `LocalSystem` service, Windows grants these query
/// rights to SYSTEM and Administrators by default, but not to an arbitrary
/// authenticated user or security group. A client in the configured group
/// therefore needs an ACE on both objects before it can read the daemon's
/// token.
///
/// Live Windows testing confirmed that both grants are required. With only
/// the process-level grant, `OpenProcess` succeeded but `OpenProcessToken`
/// was still refused. Each grant adds one ACE to the existing DACL rather
/// than replacing it, preserving the access Windows, the SCM, and
/// Administrators already have.
///
/// # Errors
///
/// Returns the underlying `io::Error` if `client_sid` cannot be parsed, this
/// process's own primary token cannot be opened, either object's existing
/// DACL cannot be read, either merged ACL cannot be built, or either new DACL
/// cannot be applied.
#[allow(
    unsafe_code,
    reason = "GetCurrentProcess, OpenProcessToken, and CloseHandle are Win32 APIs with no safe \
              standard-library wrapper; the token handle is only ever the one this function itself \
              just opened, and it is closed on every path."
)]
pub fn grant_process_query_access(client_sid: &str) -> io::Result<()> {
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle that needs no
    // closing and is valid for the whole call.
    let process = unsafe { GetCurrentProcess() };
    add_ace_to_kernel_object_dacl(process, PROCESS_QUERY_LIMITED_INFORMATION, client_sid)?;

    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: `process` is the pseudo-handle above; `token` is a local
    // out-parameter.
    let ok = unsafe {
        OpenProcessToken(
            process,
            TOKEN_QUERY | READ_CONTROL | WRITE_DAC,
            &raw mut token,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = add_ace_to_kernel_object_dacl(token, TOKEN_QUERY, client_sid);
    // SAFETY: `token` was returned by the successful `OpenProcessToken` call
    // above and is not used again after this point.
    unsafe { CloseHandle(token) };
    result
}

#[cfg(test)]
#[path = "security_descriptor_tests.rs"]
mod tests;
