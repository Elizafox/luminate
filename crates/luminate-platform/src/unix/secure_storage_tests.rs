// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::os::unix::net::UnixListener;

use super::*;
use crate::test_support::TestDir;

#[test]
fn private_and_service_directories_enforce_distinct_modes() {
    let root = TestDir::new("secure-storage-directories");
    let private = root.join("private");
    ensure_private_directory(&private).expect("create private directory");
    assert_eq!(mode(&private), 0o700);

    let service = root.join("service");
    fs::create_dir_all(&service).expect("create service directory");
    fs::set_permissions(&service, fs::Permissions::from_mode(0o750))
        .expect("set service directory mode");
    ensure_service_directory(&service).expect("accept group-traversable service directory");
    assert!(ensure_private_directory(&service).is_err());

    fs::set_permissions(&service, fs::Permissions::from_mode(0o770))
        .expect("make service directory group-writable");
    assert!(ensure_service_directory(&service).is_err());
}

#[test]
fn private_directory_rejects_a_final_symlink() {
    let root = TestDir::new("secure-storage-directory-symlink");
    let target = root.join("target");
    fs::create_dir_all(&target).expect("create target directory");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700))
        .expect("set target directory mode");
    let link = root.join("link");
    symlink(&target, &link).expect("create directory symlink");

    assert!(ensure_private_directory(&link).is_err());
}

#[test]
fn private_read_accepts_only_owner_private_regular_files() {
    let root = TestDir::new("secure-storage-private-read");
    let path = root.join("authority.json");
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(PRIVATE_FILE_MODE)
        .open(&path)
        .expect("create authority file");
    file.write_all(b"private").expect("write authority file");
    drop(file);
    open_private_file_for_read(&path).expect("open private regular file");

    fs::set_permissions(&path, fs::Permissions::from_mode(0o640))
        .expect("make authority file group-readable");
    assert!(open_private_file_for_read(&path).is_err());

    fs::set_permissions(&path, fs::Permissions::from_mode(PRIVATE_FILE_MODE))
        .expect("restore authority file mode");
    let link = root.join("authority-link");
    symlink(&path, &link).expect("create file symlink");
    assert!(open_private_file_for_read(&link).is_err());

    assert!(open_private_file_for_read(&root).is_err());

    let socket = root.join("socket");
    let _listener = UnixListener::bind(&socket).expect("bind Unix socket");
    assert!(open_private_file_for_read(&socket).is_err());
}

#[test]
#[allow(
    unsafe_code,
    reason = "mkfifo has no safe standard-library equivalent; the NUL-terminated path remains valid for the call"
)]
fn private_read_rejects_a_fifo_without_blocking() {
    let root = TestDir::new("secure-storage-fifo");
    let path = root.join("authority.fifo");
    let path_bytes = CString::new(path.as_os_str().as_bytes()).expect("path without NUL");
    // SAFETY: `path_bytes` is NUL-terminated and remains alive for this call.
    let result = unsafe { libc::mkfifo(path_bytes.as_ptr(), PRIVATE_FILE_MODE) };
    assert_eq!(result, 0, "mkfifo failed: {}", io::Error::last_os_error());

    let error = open_private_file_for_read(&path).expect_err("FIFO must be rejected");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).expect("read metadata").mode() & 0o777
}
