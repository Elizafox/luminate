// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    unsafe_code,
    reason = "Renaming a process's own argv/comm requires raw syscalls and, \
              on glibc, a startup hook that relocates the kernel-provided \
              argv and environment strings."
)]

//! Linux process-title support.
//!
//! Always renames the kernel "comm" field (`prctl(PR_SET_NAME)`), visible in
//! `top`'s default view and `ps -o comm`, truncated to 15 bytes. On glibc,
//! [`argv_rewrite`] relocates argument strings that follow `argv[0]`, reclaiming
//! their contiguous storage for a later title. Environment storage is left
//! untouched: launchers may arrange it in ways that are unsafe to reclaim, and
//! preserving configuration is more important than an unusually long title.
//! This storage is what `ps aux`/`ps -ef` and `/proc/<pid>/cmdline` display.

/// Stable kernel UAPI constant from `<linux/prctl.h>`; fixed since its
/// introduction and not expected to change.
#[cfg(not(miri))]
const PR_SET_NAME: libc::c_long = 15;

pub(crate) fn set_process_title(title: &str) {
    #[cfg(not(miri))]
    set_comm(title);

    #[cfg(target_env = "gnu")]
    argv_rewrite::rewrite(title);
}

/// Renames the kernel "comm" field via `prctl(PR_SET_NAME)`.
///
/// `libc` does not expose a `prctl` binding or `PR_SET_NAME` for ordinary
/// Linux targets (only for Android/L4Re), so this goes through the raw
/// `syscall(2)` wrapper instead, using the syscall number `libc` does
/// provide (`SYS_prctl`) for every Linux target/architecture combination.
#[cfg(not(miri))]
fn set_comm(title: &str) {
    let bytes = title.as_bytes();
    let copy_len = bytes.len().min(15);

    // `comm` is 15 bytes plus a NUL terminator; zero-initializing and
    // copying at most 15 bytes guarantees the buffer stays NUL-terminated.
    let mut buf = [0_u8; 16];
    for (dest, src) in buf.iter_mut().zip(bytes.iter()).take(copy_len) {
        *dest = *src;
    }

    // SAFETY: `buf` is a valid, NUL-terminated 16-byte buffer for the
    // duration of this call; `prctl(PR_SET_NAME, ...)` reads at most 16
    // bytes from the pointer we pass it.
    unsafe {
        libc::syscall(
            libc::SYS_prctl,
            PR_SET_NAME,
            buf.as_ptr(),
            0_isize,
            0_isize,
            0_isize,
        );
    }
}

#[cfg(target_env = "gnu")]
mod argv_rewrite {
    use std::ffi::{CStr, c_char, c_int};
    use std::ptr;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Populated once by `initialize`, before `main` runs; read thereafter.
    static TITLE_START: AtomicUsize = AtomicUsize::new(0);
    static TITLE_LEN: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" {
        static mut program_invocation_name: *mut c_char;
        static mut program_invocation_short_name: *mut c_char;
    }

    /// Prepares the kernel-provided argument and environment storage so
    /// [`rewrite`] can overwrite it later.
    ///
    /// glibc's CSU startup code invokes every `.init_array` entry as
    /// `(argument_count, argv, envp)` regardless of the function's declared
    /// signature; this is a documented glibc extension, not standard C,
    /// which is why this whole module is gated to `target_env = "gnu"`.
    /// musl calls `.init_array` constructors with no arguments, so reading
    /// argc/argv there would read uninitialized registers or stack memory.
    #[cfg_attr(
        miri,
        allow(
            dead_code,
            reason = "Miri does not implement glibc's three-argument .init_array extension"
        )
    )]
    extern "C" fn initialize(
        argument_count: c_int,
        argv: *mut *mut c_char,
        _envp: *mut *mut c_char,
    ) {
        if argument_count <= 0 || argv.is_null() {
            return;
        }

        // SAFETY: a positive `argument_count` means the kernel-provided argv
        // array contains an argv[0] entry.
        let start = unsafe { *argv };
        if start.is_null() {
            return;
        }

        // Start with argv[0], then extend the title area only across strings
        // that are exactly contiguous with it. This is libbsd's conservative
        // rule: a gap could contain storage owned by something else.
        let mut end = string_end(start);
        for i in 1..argument_count as isize {
            // SAFETY: `i` is within the kernel-provided argv array.
            let entry = unsafe { argv.offset(i) };
            // SAFETY: `entry` is within that same argv array.
            let string = unsafe { *entry };
            end = extend_contiguous(end, string);
        }

        // libbsd preserves glibc's public program-name pointers before
        // reclaiming the storage to which they normally point.
        if !duplicate_program_names() {
            return;
        }

        // Relocate the remaining arguments. Keeping argv[0] in place is
        // intentional: procfs tracks its original address as the start of
        // the displayed command line.
        for i in 1..argument_count as isize {
            // SAFETY: `i` is within the argv array.
            let entry = unsafe { argv.offset(i) };
            if !duplicate_entry(entry) {
                return;
            }
        }

        TITLE_START.store(start as usize, Ordering::Relaxed);
        TITLE_LEN.store(end as usize - start as usize, Ordering::Relaxed);
    }

    // Miri invokes `.init_array` entries through the standard zero-argument
    // constructor ABI rather than glibc's three-argument extension.
    #[cfg(not(miri))]
    #[used]
    #[unsafe(link_section = ".init_array")]
    static INITIALIZE: extern "C" fn(c_int, *mut *mut c_char, *mut *mut c_char) = initialize;

    fn string_end(string: *mut c_char) -> *mut c_char {
        // SAFETY: callers pass a non-NULL pointer from argv or envp, whose
        // entries are NUL-terminated C strings.
        let len = unsafe { CStr::from_ptr(string) }.to_bytes().len();

        // SAFETY: the terminator is one byte beyond the string contents.
        unsafe { string.add(len + 1) }
    }

    fn extend_contiguous(end: *mut c_char, string: *mut c_char) -> *mut c_char {
        if string == end {
            string_end(string)
        } else {
            end
        }
    }

    fn duplicate_entry(entry: *mut *mut c_char) -> bool {
        // SAFETY: callers provide a valid argv or envp array entry.
        let string = unsafe { *entry };
        if string.is_null() {
            return true;
        }

        // SAFETY: `string` is NUL-terminated. `strdup` returns independent
        // malloc-owned storage or NULL without changing the original.
        let copy = unsafe { libc::strdup(string) };
        if copy.is_null() {
            return false;
        }

        // SAFETY: `entry` is writable process-startup storage.
        unsafe { *entry = copy };
        true
    }

    fn duplicate_program_names() -> bool {
        let long_name = &raw mut program_invocation_name;
        if !duplicate_program_name(long_name) {
            return false;
        }

        let short_name = &raw mut program_invocation_short_name;
        duplicate_program_name(short_name)
    }

    fn duplicate_program_name(name: *mut *mut c_char) -> bool {
        // SAFETY: `name` points to a glibc program-name global.
        let original = unsafe { *name };
        if original.is_null() {
            return true;
        }

        // SAFETY: glibc's non-NULL program-name pointers refer to
        // NUL-terminated strings.
        let copy = unsafe { libc::strdup(original) };
        if copy.is_null() {
            return false;
        }

        // SAFETY: `name` points to the writable glibc global read above.
        unsafe { *name = copy };
        true
    }

    /// Overwrites the reclaimed startup string range with `title`, truncated
    /// to the contiguous space originally occupied by argv and environment.
    ///
    /// Does nothing if no argv range was captured (which should not happen
    /// in practice, since `.init_array` runs before `main`).
    pub(crate) fn rewrite(title: &str) {
        let start = TITLE_START.load(Ordering::Relaxed);
        let len = TITLE_LEN.load(Ordering::Relaxed);
        if len == 0 {
            return;
        }

        let bytes = title.as_bytes();
        let usable = len - 1; // leave room for a final NUL
        let copy_len = bytes.len().min(usable);

        let base = start as *mut u8;

        // SAFETY: `start`/`len` describe the contiguous startup string area
        // whose live contents were relocated by `initialize`. We only write
        // within that reclaimed area, which procfs reads as the command line.
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), base, copy_len) };

        // SAFETY: still within `[start, start + len)` since `copy_len <=
        // len - 1`.
        let remainder = unsafe { base.add(copy_len) };

        // SAFETY: as above.
        unsafe { ptr::write_bytes(remainder, 0, len - copy_len) };
    }

    // Miri cannot spawn the child process or inspect Linux procfs.
    #[cfg(test)]
    #[path = "tests.rs"]
    mod tests;
}
