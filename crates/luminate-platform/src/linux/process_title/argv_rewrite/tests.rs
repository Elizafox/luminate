// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for Linux argv process-title rewriting.

#[test]
fn helper_paths_handle_null_entries_and_noncontiguous_strings() {
    use std::ffi::{CString, c_char};
    use std::ptr;

    let mut null_entry: *mut c_char = ptr::null_mut();
    assert!(super::duplicate_entry(&raw mut null_entry));
    assert!(super::duplicate_program_name(&raw mut null_entry));

    let first = CString::new("first").expect("literal has no NUL");
    let second = CString::new("second").expect("literal has no NUL");
    let end = super::string_end(first.as_ptr().cast_mut());
    assert_eq!(super::extend_contiguous(end, end), super::string_end(end));
    assert_eq!(
        super::extend_contiguous(end, second.as_ptr().cast_mut()),
        end
    );
}

#[cfg(not(miri))]
mod native {
    use std::env;
    use std::fs;
    use std::process::Command;
    use std::sync::Mutex;

    const CHILD_MARKER: &str = "LUMINATE_PROCESS_TITLE_TEST_CHILD";
    const ENVIRONMENT_PADDING: &str = "LUMINATE_PROCESS_TITLE_TEST_PADDING";
    const MUTATION_PROBE: &str = "LUMINATE_PROCESS_TITLE_TEST_MUTATION";
    const CONFIG_PROBE: &str = "LUMINATED_CONFIG";

    static ENVIRONMENT_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn rewrites_argument_storage_and_preserves_environment() {
        let _environment_guard = ENVIRONMENT_LOCK.lock().expect("lock process environment");

        if env::var_os(CHILD_MARKER).is_some() {
            let title = "luminate process title ".repeat(12);
            super::super::rewrite(&title);

            assert_eq!(env::var(CHILD_MARKER).as_deref(), Ok("1"));
            assert_eq!(env::var(CONFIG_PROBE).as_deref(), Ok("/tmp/luminated.toml"));
            assert_eq!(
                env::var(ENVIRONMENT_PADDING).as_deref(),
                Ok("x".repeat(512).as_str())
            );

            // SAFETY: the child runs only this exact test, and this test
            // serializes all of its process-environment access.
            unsafe { env::set_var(MUTATION_PROBE, "still works") };
            assert_eq!(env::var(MUTATION_PROBE).as_deref(), Ok("still works"));
            // SAFETY: protected by the same test-local serialization.
            unsafe { env::remove_var(MUTATION_PROBE) };
            assert_eq!(env::var_os(MUTATION_PROBE), None);

            let cmdline = fs::read("/proc/self/cmdline").expect("read process command line");
            let used = cmdline
                .iter()
                .position(|byte| *byte == 0)
                .expect("rewritten command line is NUL-terminated");
            assert_eq!(&cmdline[..used], &title.as_bytes()[..used]);
            assert!(cmdline[used..].iter().all(|byte| *byte == 0));
            return;
        }

        let executable = env::current_exe().expect("find test executable");
        let output = Command::new(executable)
                    .env_clear()
                    .env(CHILD_MARKER, "1")
                    .env(CONFIG_PROBE, "/tmp/luminated.toml")
                    .env(ENVIRONMENT_PADDING, "x".repeat(512))
                    .args([
                        "--exact",
                        "linux::process_title::argv_rewrite::tests::native::rewrites_argument_storage_and_preserves_environment",
                    ])
                    .output()
                    .expect("run process-title test child");

        assert!(
            output.status.success(),
            "child failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
