// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::io;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Power::{
    PowerClearRequest, PowerCreateRequest, PowerRequestSystemRequired, PowerSetRequest,
};
use windows_sys::Win32::System::SystemServices::POWER_REQUEST_CONTEXT_VERSION;
use windows_sys::Win32::System::Threading::{
    POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
};

use crate::windows::wide::to_wide;

/// A best-effort request that the system stay awake for as long as this
/// value lives, backed by `PowerCreateRequest`/`PowerSetRequest` with
/// `PowerRequestSystemRequired`.
///
/// # Caveats
///
/// This only ever influences whether the system's *idle* detector decides to
/// initiate a suspend. It must be held proactively, before that decision
/// is made, to do anything at all; it cannot un-commit a suspend already
/// under way, which is why [`super::interpret`] does not create one when
/// notified that a suspend is imminent; by then it is too late to matter.
/// It also has no effect on a suspend the user explicitly requested (closing
/// a laptop lid, choosing Sleep from the Start menu) or one forced by
/// policy: Windows has given services no way to veto those since Vista.
#[allow(
    dead_code,
    reason = "no operation currently needs the proactive keep-awake primitive"
)]
#[derive(Debug)]
pub(crate) struct SystemSleepRequest {
    handle: HANDLE,
}

#[allow(
    dead_code,
    reason = "no operation currently needs the proactive keep-awake primitive"
)]
impl SystemSleepRequest {
    /// Creates a new sleep-request guard.
    ///
    /// # Errors
    ///
    /// Returns an error if `PowerCreateRequest` or `PowerSetRequest` fails.
    #[allow(
        unsafe_code,
        reason = "PowerCreateRequest/PowerSetRequest have no safe standard-library wrapper; each \
                  call site documents why the arguments it passes are valid."
    )]
    pub(crate) fn new(reason: &str) -> io::Result<Self> {
        let mut reason = to_wide(reason);

        let context = REASON_CONTEXT {
            Version: POWER_REQUEST_CONTEXT_VERSION,
            Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
            Reason: REASON_CONTEXT_0 {
                SimpleReasonString: reason.as_mut_ptr(),
            },
        };

        // SAFETY: `context` points to an initialised `REASON_CONTEXT`.
        // `reason` is NUL-terminated and remains alive for this call.
        let handle = unsafe { PowerCreateRequest(&raw const context) };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        // SAFETY: `handle` is a valid power-request handle returned by
        // `PowerCreateRequest`.
        let set = unsafe { PowerSetRequest(handle, PowerRequestSystemRequired) };
        if set == 0 {
            // Capture the error before CloseHandle potentially changes the
            // calling thread's last-error value.
            let error = io::Error::last_os_error();

            // SAFETY: `handle` is a valid, owned Win32 handle.
            unsafe { CloseHandle(handle) };

            return Err(error);
        }

        Ok(Self { handle })
    }
}

impl Drop for SystemSleepRequest {
    #[allow(
        unsafe_code,
        reason = "PowerClearRequest/CloseHandle have no safe standard-library wrapper; `self.handle` \
                  is a valid, owned Win32 handle for the lifetime of this object."
    )]
    fn drop(&mut self) {
        // SAFETY: `self.handle` is valid and owned by this object.
        // `PowerRequestSystemRequired` was set exactly once during construction.
        // There is no viable recovery path if this fails.
        unsafe { PowerClearRequest(self.handle, PowerRequestSystemRequired) };

        // SAFETY: `self.handle` is valid and owned by this object.
        // There is no viable recovery path if this fails.
        unsafe { CloseHandle(self.handle) };
    }
}
