// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;
use crate::test_support::TestDir;
use std::os::unix::fs::MetadataExt as _;

fn unique_dir(name: &str) -> TestDir {
    TestDir::uncreated(name)
}

#[allow(
    unsafe_code,
    reason = "libc::getuid() takes no arguments, touches no memory, and cannot fail; used \
                  here only to chown a throwaway test directory to the current process's own \
                  identity, a no-op ownership change any test-runner user can make."
)]
fn current_uid() -> u32 {
    // SAFETY: getuid() takes no arguments, touches no memory, and cannot fail.
    unsafe { libc::getuid() }
}

#[allow(
    unsafe_code,
    reason = "libc::getgid() takes no arguments, touches no memory, and cannot fail; used \
                  here only to chown a throwaway test directory to the current process's own \
                  identity, a no-op ownership change any test-runner user can make."
)]
fn current_gid() -> u32 {
    // SAFETY: getgid() takes no arguments, touches no memory, and cannot fail.
    unsafe { libc::getgid() }
}

fn current_identity() -> (u32, u32) {
    (current_uid(), current_gid())
}

#[test]
fn ensure_directory_sets_the_requested_mode() {
    let dir = unique_dir("directories-mode");
    let (uid, gid) = current_identity();
    ensure_directory(&dir, 0o750, uid, gid).expect("ensure directory");
    let mode = fs::metadata(&dir).expect("stat directory").mode() & 0o7777;
    assert_eq!(mode, 0o750);
}

#[test]
fn ensure_directory_is_idempotent() {
    let dir = unique_dir("directories-idempotent");
    let (uid, gid) = current_identity();
    ensure_directory(&dir, 0o700, uid, gid).expect("first ensure");
    ensure_directory(&dir, 0o700, uid, gid).expect("second ensure");
}
