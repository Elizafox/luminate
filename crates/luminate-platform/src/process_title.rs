// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Sets a process's displayed title, so `ps`/`top`/equivalent tools can tell
//! Luminate's daemon and its re-exec'd plugin/policy hosts apart at a glance.
//!
//! This is best-effort by nature: there is no single portable mechanism.
//! Linux gets the fullest treatment (a real `ps aux`-visible title on glibc,
//! a truncated kernel "comm" name everywhere else); the BSDs get the native
//! `setproctitle(3)` call; macOS and Windows have no equivalent, so
//! [`set_process_title`] is a documented no-op there.

#[cfg(target_os = "linux")]
use crate::linux::process_title::set_process_title as platform_set_process_title;
#[cfg(any(
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
))]
use crate::unix::process_title::set_process_title as platform_set_process_title;

/// Sets this process's displayed title, on platforms that support it.
///
/// Call this once, early in the process's life, before other threads start
/// (the Linux implementation captures process-startup state that is not
/// safe to read once other code may have observed or raced it).
pub fn set_process_title(title: &str) {
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    platform_set_process_title(title);

    // No public API renames a process's displayed title or command line on
    // macOS.
    #[cfg(target_os = "macos")]
    let _ = title;

    // No per-process equivalent exists on Windows. The Windows service's
    // description (`windows::service::install`, `SERVICE_DESCRIPTION`) is a
    // separate, static, install-time-only string in the SCM database and is
    // unrelated to this per-process title.
    #[cfg(windows)]
    let _ = title;
}
