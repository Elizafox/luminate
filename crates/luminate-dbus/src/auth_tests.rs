// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Authorization policy and caller-identity tests.

use super::*;
use luminate_platform::test_support::TestDir;
use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn group_authorization_precedes_optional_polkit() {
    assert_eq!(
        Decision::decide(&[10, 42, 1000], 42, true),
        Decision::AllowGroup
    );
    assert_eq!(
        Decision::decide(&[10, 1000], 42, true),
        Decision::ConsultPolkit
    );
    assert_eq!(Decision::decide(&[10, 1000], 42, false), Decision::Deny);
}

#[test]
fn process_snapshot_includes_primary_gid() {
    let proc_root = TestDir::new("dbus-proc");
    let process_dir = proc_root.join("123");
    fs::create_dir_all(&process_dir).expect("create fake process directory");
    fs::write(
        process_dir.join("status"),
        "Name:\ttest\nUid:\t1000\t1000\t1000\t1000\nGid:\t42\t42\t42\t42\nGroups:\t10 1000\n",
    )
    .expect("write fake process status");
    fs::write(
        process_dir.join("stat"),
        "123 (test process) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 4242 20\n",
    )
    .expect("write fake process stat");

    let snapshot = process_snapshot(&proc_root, 123, 1000).expect("read process snapshot");

    assert_eq!(snapshot.start_time, 4242);
    assert_eq!(
        Decision::decide(&snapshot.groups, 42, false),
        Decision::AllowGroup
    );
}

#[test]
fn polkit_subject_pins_pid_to_start_time_and_uid() {
    assert_eq!(polkit_process_subject(123, 4242, 1000), "123,4242,1000");
}

#[test]
fn process_snapshot_rejects_a_uid_mismatch() {
    let proc_root = TestDir::new("dbus-proc-uid");
    let process_dir = proc_root.join("321");
    fs::create_dir_all(&process_dir).expect("create fake process directory");
    fs::write(
        process_dir.join("status"),
        "Name:\ttest\nUid:\t1000\t1000\t1000\t1000\nGid:\t42\t42\t42\t42\nGroups:\t10 1000\n",
    )
    .expect("write fake process status");
    fs::write(
        process_dir.join("stat"),
        "321 (test process) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 4343 20\n",
    )
    .expect("write fake process stat");

    // A mismatch means the caller's credentials changed after D-Bus captured
    // them or the PID was reused, so this snapshot cannot safely identify the
    // caller.
    let error =
        process_snapshot(&proc_root, 321, 2000).expect_err("mismatched UID must be rejected");
    assert!(error.to_string().contains("does not match"));
}

#[test]
fn process_stat_parsing_rejects_malformed_command_and_missing_fields() {
    assert!(process_start_time("no closing paren here").is_err());
    assert!(process_start_time("123 (name) S 1 2 3").is_err());
    assert!(
        process_start_time(
            "123 (name) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 not-a-number 20"
        )
        .is_err()
    );
}

#[test]
fn process_status_parsing_reports_missing_fields() {
    assert!(parse_status("Groups:\t1 2 3\n").is_err(), "missing Uid/Gid");
    assert!(
        parse_status("Uid:\t1000\t1000\t1000\t1000\nGid:\t42\t42\t42\t42\n").is_err(),
        "missing Groups"
    );
    assert!(
        status_field("Uid:\tnot-a-number\n", "Uid:", 0, "effective UID").is_err(),
        "non-numeric field must fail to parse"
    );
    assert!(
        parse_status("Uid:\t1000\t1000\t1000\t1000\nGid:\t42\t42\t42\t42\nGroups:\tnot-a-number\n")
            .is_err(),
        "non-numeric supplementary GID must fail to parse"
    );
}

#[test]
fn group_gid_finds_named_group_and_reports_missing_or_malformed_entries() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after Unix epoch")
        .as_nanos();
    let group_file = env::temp_dir().join(format!("luminate-group-{unique}"));
    fs::write(
        &group_file,
        "# a comment line\nroot:x:0:\nwheel:x:10:alice,bob\nluminate:x:987:\n",
    )
    .expect("write fake group file");

    assert_eq!(
        group_gid(&group_file, "luminate").expect("find luminate group"),
        987
    );
    assert_eq!(
        group_gid(&group_file, "wheel").expect("find wheel group"),
        10
    );

    let error =
        group_gid(&group_file, "does-not-exist").expect_err("unknown group must be rejected");
    assert!(error.to_string().contains("does not exist"));

    assert!(
        group_gid(Path::new("/definitely/missing/group/file"), "root").is_err(),
        "an unreadable group file must surface its I/O error"
    );

    fs::remove_file(group_file).expect("remove fake group file");
}
