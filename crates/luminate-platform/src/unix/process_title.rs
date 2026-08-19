// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(unsafe_code, reason = "setproctitle(3) is a raw variadic C FFI call.")]

//! Process-title support for the BSDs, via the native `setproctitle(3)`
//! libc call. Unlike the Linux argv rewrite, this needs no memory-layout
//! assumptions of our own: the kernel and libc do the work.

use std::ffi::CString;

pub(crate) fn set_process_title(title: &str) {
    // Titles with interior NULs can't be represented as a C string; skip
    // silently rather than truncating to something misleading.
    let Ok(title) = CString::new(title) else {
        return;
    };

    // SAFETY: the format string `c"%s"` consumes exactly one `%s` argument,
    // matching the single `title` pointer passed; `title` is a valid,
    // NUL-terminated C string for the duration of this call.
    unsafe {
        libc::setproctitle(c"%s".as_ptr(), title.as_ptr());
    }
}
