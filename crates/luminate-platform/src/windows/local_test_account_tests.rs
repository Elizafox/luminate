// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Throwaway local Windows accounts and per-thread impersonation, so a test
//! can prove pipe DACL enforcement against a genuinely different security
//! principal instead of trusting the SDDL string alone.
//!
//! Test-only: creating a local account and logging it on both ordinarily
//! require administrator privileges, so every test built on this module is
//! `#[ignore]`d with an explanation, the same convention
//! [`crate::windows::transport`]'s impostor test already uses for other
//! elevated, VM-only checks.

#![cfg(test)]

use std::io;
use std::ops::Deref;
use std::ptr;
use std::thread;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::NetworkManagement::NetManagement::{
    NetUserAdd, NetUserDel, UF_DONT_EXPIRE_PASSWD, UF_NORMAL_ACCOUNT, UF_SCRIPT, USER_INFO_1,
    USER_PRIV_USER,
};
use windows_sys::Win32::Security::{
    ImpersonateLoggedOnUser, LOGON32_LOGON_NETWORK, LOGON32_PROVIDER_DEFAULT, LogonUserW,
    RevertToSelf,
};

use tokio::runtime::Handle;

use crate::secure_random::fill_bytes;
use crate::windows::local_group::{add_local_group_member, delete_local_group, ensure_local_group};
use crate::windows::wide::to_wide;

/// A throwaway local group, removed on drop.
///
/// A plain "delete it at the end of the test" would leak the group if an
/// assertion earlier in the test panics; tying removal to `Drop` instead
/// means unwinding cleans it up the same as [`TestAccount`] already does for
/// accounts.
pub(crate) struct TestGroup(String);

impl TestGroup {
    /// Creates a throwaway local group named `name`.
    pub(crate) fn create(name: &str, comment: &str) -> io::Result<Self> {
        ensure_local_group(name, comment)?;
        Ok(Self(name.to_owned()))
    }
}

impl Deref for TestGroup {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl Drop for TestGroup {
    fn drop(&mut self) {
        let _ = delete_local_group(&self.0);
    }
}

/// A throwaway local user account, deleted on drop.
///
/// Logs the account on once at creation (`LOGON32_LOGON_NETWORK`, which
/// needs no roaming profile and is enough to exercise Windows's ordinary
/// access-check path) rather than re-authenticating for every use.
pub(crate) struct TestAccount {
    name: String,
    token: HANDLE,
}

impl TestAccount {
    /// Creates a throwaway local account named `name` with a random
    /// password, joins it to each of `groups`, and logs it on.
    ///
    /// Membership is granted before logon deliberately: Windows fixes a
    /// token's group membership at logon time, so joining a group only
    /// takes effect for a subsequent logon, the same reason the real
    /// installer tells an administrator that new client-group membership
    /// needs a sign-out or reboot before it takes effect.
    pub(crate) fn create(name: &str, groups: &[&str]) -> io::Result<Self> {
        let password = random_password()?;
        create_local_user(name, &password)?;

        if let Err(error) = join_groups(name, groups) {
            let _ = delete_local_user(name);
            return Err(error);
        }

        let token = match logon(name, &password) {
            Ok(token) => token,
            Err(error) => {
                let _ = delete_local_user(name);
                return Err(error);
            }
        };

        Ok(Self {
            name: name.to_owned(),
            token,
        })
    }

    /// Runs `f` on a dedicated OS thread impersonating this account, so any
    /// access check `f` performs (`CreateFile`, pipe-instance creation) is
    /// evaluated by Windows against this account's token rather than the
    /// test process's own.
    ///
    /// Impersonation is per-thread, not per-process; using a fresh scoped
    /// thread rather than a pooled one (a `tokio` worker, say) keeps this
    /// account's token from outliving `f` on a thread that later runs
    /// unrelated code. The caller's `tokio` runtime handle is entered on
    /// that same thread first: `tokio`'s named-pipe types register with the
    /// reactor even for their nominally synchronous constructors, so `f`
    /// must run somewhere that reactor is reachable, not just somewhere with
    /// the right impersonation token. Call this from within a `#[tokio::test]`.
    pub(crate) fn run_impersonated<T: Send>(&self, f: impl FnOnce() -> T + Send) -> T {
        let token = SendableHandle(self.token);
        let runtime = Handle::current();
        thread::scope(|scope| {
            scope
                .spawn(move || {
                    let token = token;
                    let _runtime_guard = runtime.enter();
                    #[allow(
                        unsafe_code,
                        reason = "ImpersonateLoggedOnUser is a Win32 API with no safe \
                                  standard-library wrapper; `token.0` is this account's live \
                                  logon token, kept alive by `TestAccount` for at least the \
                                  duration of this scoped thread."
                    )]
                    // SAFETY: see the reason above.
                    let ok = unsafe { ImpersonateLoggedOnUser(token.0) };
                    assert!(
                        ok != 0,
                        "impersonate throwaway test account: {}",
                        io::Error::last_os_error()
                    );
                    let _guard = RevertOnDrop;
                    f()
                })
                .join()
                .expect("impersonated closure panicked")
        })
    }
}

impl Drop for TestAccount {
    fn drop(&mut self) {
        #[allow(
            unsafe_code,
            reason = "CloseHandle is a Win32 API with no safe standard-library wrapper; \
                      `self.token` was returned by the successful LogonUserW call in `create` \
                      and is not used again after this point."
        )]
        // SAFETY: see the reason above.
        unsafe {
            CloseHandle(self.token)
        };
        let _ = delete_local_user(&self.name);
    }
}

/// Moves a Windows handle into exactly one scoped thread. A handle has no
/// thread affinity of its own; the safety obligation is only that nothing
/// else uses it concurrently, which `run_impersonated`'s single spawned
/// thread satisfies.
struct SendableHandle(HANDLE);

#[allow(
    unsafe_code,
    reason = "HANDLE is a raw pointer and so not Send by default, even though Windows handles \
              carry no thread affinity; this wrapper asserts that fact for the single scoped \
              thread `run_impersonated` moves one into."
)]
// SAFETY: see the reason above.
unsafe impl Send for SendableHandle {}

/// Reverts thread impersonation on drop, including on panic, so a failing
/// assertion inside an impersonated closure can never leave the spawned
/// thread stuck running as the throwaway account.
struct RevertOnDrop;

impl Drop for RevertOnDrop {
    #[allow(
        unsafe_code,
        reason = "RevertToSelf is a Win32 API with no safe standard-library wrapper; it takes no \
                  arguments and is always safe to call, whether or not this thread is currently \
                  impersonating."
    )]
    fn drop(&mut self) {
        // SAFETY: see the reason above.
        unsafe {
            RevertToSelf();
        }
    }
}

/// A password satisfying Windows's default local complexity policy
/// (mixed case, a digit, a symbol, minimum length), drawn from the platform
/// RNG so no two throwaway accounts share one.
fn random_password() -> io::Result<String> {
    let mut bytes = [0_u8; 24];
    fill_bytes(&mut bytes)?;

    // The fixed prefix guarantees every required character class regardless
    // of what the random bytes below happen to contain; the random suffix
    // guarantees uniqueness across accounts.
    let mut password = String::from("Lx9!");
    for byte in bytes {
        password.push(char::from(b'a' + byte % 26));
    }
    Ok(password)
}

fn join_groups(name: &str, groups: &[&str]) -> io::Result<()> {
    // A bare name resolves against the local machine, same as an unqualified
    // name typed at `net localgroup`; a leading `.\` is accepted by several
    // other Win32 identity APIs (`LogonUserW`'s domain argument among them)
    // but `NetLocalGroupAddMembers` rejects it outright.
    for group in groups {
        add_local_group_member(group, name)?;
    }
    Ok(())
}

fn create_local_user(name: &str, password: &str) -> io::Result<()> {
    let mut name = to_wide(name);
    let mut password = to_wide(password);
    let info = USER_INFO_1 {
        usri1_name: name.as_mut_ptr(),
        usri1_password: password.as_mut_ptr(),
        usri1_password_age: 0,
        usri1_priv: USER_PRIV_USER,
        usri1_home_dir: ptr::null_mut(),
        usri1_comment: ptr::null_mut(),
        usri1_flags: UF_SCRIPT | UF_NORMAL_ACCOUNT | UF_DONT_EXPIRE_PASSWD,
        usri1_script_path: ptr::null_mut(),
    };
    let mut parameter_error = 0_u32;

    #[allow(
        unsafe_code,
        reason = "NetUserAdd is a Win32 API with no safe standard-library wrapper; `name` and \
                  `password` are writable, NUL-terminated wide buffers that outlive the call, \
                  and `info`/`parameter_error` are local values this call alone writes to."
    )]
    // SAFETY: see the reason above.
    let status = unsafe {
        NetUserAdd(
            ptr::null(),
            1,
            ptr::from_ref(&info).cast::<u8>(),
            &raw mut parameter_error,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "NetUserAdd failed with status {status} (parameter {parameter_error})"
        )))
    }
}

fn delete_local_user(name: &str) -> io::Result<()> {
    let name = to_wide(name);

    #[allow(
        unsafe_code,
        reason = "NetUserDel is a Win32 API with no safe standard-library wrapper; `name` is a \
                  NUL-terminated wide buffer that outlives the call."
    )]
    // SAFETY: see the reason above.
    let status = unsafe { NetUserDel(ptr::null(), name.as_ptr()) };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "NetUserDel failed with status {status}"
        )))
    }
}

fn logon(name: &str, password: &str) -> io::Result<HANDLE> {
    let name = to_wide(name);
    let domain = to_wide(".");
    let password = to_wide(password);
    let mut token: HANDLE = ptr::null_mut();

    #[allow(
        unsafe_code,
        reason = "LogonUserW is a Win32 API with no safe standard-library wrapper; every string \
                  argument is a NUL-terminated wide buffer that outlives the call, and `token` \
                  is a local out-parameter this call alone writes to."
    )]
    // SAFETY: see the reason above.
    let ok = unsafe {
        LogonUserW(
            name.as_ptr(),
            domain.as_ptr(),
            password.as_ptr(),
            LOGON32_LOGON_NETWORK,
            LOGON32_PROVIDER_DEFAULT,
            &raw mut token,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(token)
    }
}
