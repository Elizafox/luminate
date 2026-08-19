// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::fs;
use std::io::Write as _;

use super::*;

use crate::test_support::{TestDir, unique_runtime_dir as unique_path};

#[test]
fn ensure_private_directory_creates_and_reverifies() {
    let dir = TestDir::uncreated("dir");
    ensure_private_directory(&dir).expect("create private directory");
    assert!(dir.is_dir());
    // Calling again on the same, already-private directory it just
    // created must still succeed: verification, not just creation.
    ensure_private_directory(&dir).expect("re-verify private directory");
}

#[test]
fn ensure_private_directory_rejects_a_directory_with_a_wider_dacl() {
    let dir = TestDir::uncreated("wide-dir");
    // A plain directory inherits its parent's (the temp directory's)
    // DACL, which is not the single owner-only ACE this module expects.
    // This exercises the fail-safe "verify, don't repair" path
    // against a DACL this test did not construct by hand.
    fs::create_dir_all(&dir).expect("create plain directory");
    let error =
        ensure_private_directory(&dir).expect_err("a wider DACL must be rejected, not fixed");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}

#[test]
fn create_private_file_round_trips_and_refuses_to_overwrite() {
    let path = unique_path("file.txt");

    let mut file = create_private_file(&path).expect("create private file");
    file.write_all(b"hello").expect("write contents");
    drop(file);

    let error = create_private_file(&path).expect_err("second create must fail");
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);

    let mut opened = open_private_file_for_read(&path).expect("open the file just created");
    let contents = io::read_to_string(&mut opened).expect("read contents");
    assert_eq!(contents, "hello");
    drop(opened);

    // Unlike Unix, Windows refuses to remove a file while a handle to
    // it is still open, so `opened` must be dropped first.
    fs::remove_file(&path).expect("clean up test file");
}

#[test]
fn private_read_rejects_a_directory() {
    let path = TestDir::uncreated("read-directory");
    ensure_private_directory(&path).expect("create private directory");

    let error = open_private_file_for_read(&path).expect_err("directory must be rejected");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}
