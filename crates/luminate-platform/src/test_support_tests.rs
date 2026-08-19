// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::collections::HashSet;
use std::iter;
use std::thread;

use super::*;

#[test]
fn a_runtime_directory_leaves_room_for_a_socket_filename() {
    let directory = unique_runtime_dir("room-for-a-socket");
    let socket = directory.join("luminated.sock.events");

    assert!(
        socket.as_os_str().len() <= max_address_len(),
        "{} does not fit this platform's socket path budget",
        socket.display()
    );
}

#[test]
fn a_long_label_is_truncated_rather_than_overrunning_the_budget() {
    // The longest label in the workspace today comfortably exceeds
    // `MAX_LABEL_LEN`; truncation, not a panic, is the intended outcome.
    let directory = unique_runtime_dir("resolve-resources-collections");
    let name = directory
        .file_name()
        .expect("generated directory has a file name")
        .to_string_lossy()
        .into_owned();

    assert!(name.contains("resolve-resources-colle"));
    assert!(!name.contains("collections"));
}

#[test]
fn successive_calls_do_not_collide() {
    let first = unique_runtime_dir("collide");
    let second = unique_runtime_dir("collide");

    assert_ne!(first, second);
}

#[test]
fn concurrent_calls_do_not_collide() {
    let paths: Vec<_> = iter::repeat_with(|| {
        thread::spawn(|| {
            iter::repeat_with(|| unique_runtime_dir("concurrent"))
                .take(64)
                .collect::<Vec<_>>()
        })
    })
    .take(16)
    .flat_map(|thread| thread.join().expect("generate paths in worker thread"))
    .collect();
    let unique: HashSet<_> = paths.iter().collect();

    assert_eq!(unique.len(), paths.len());
}

#[test]
fn an_empty_label_still_produces_a_usable_directory() {
    let directory = unique_runtime_dir("");

    assert!(directory.starts_with(temp_root()));
    assert!(directory.as_os_str().len() + SOCKET_FILE_NAME_HEADROOM <= max_address_len());
}

#[test]
fn directory_guard_removes_nested_contents_on_drop() {
    let path = {
        let directory = TestDir::new("guard-cleanup");
        let nested = directory.join("nested");
        fs::create_dir_all(&nested).expect("create nested test directory");
        fs::write(nested.join("fixture"), b"temporary").expect("write nested fixture");
        directory.path().to_path_buf()
    };

    assert!(!path.exists());
}
