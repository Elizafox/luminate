// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::env;

#[path = "src/c_abi.rs"]
mod c_abi;

use c_abi::LUMINATE_C_ABI_VERSION;

// This is the SOVERSION baked into the cdylib's SONAME. C consumers that
// link against `libluminate.so.<N>` therefore get the dynamic loader's
// guarantee that the ABI they built against still matches at runtime.
//
// Bumping this value is not just a source change. Packaging embeds the
// concrete SONAME (`libluminate.so.<N>`) as literal paths and RPM
// `Provides`/`Requires` strings, none of which are derived from this
// constant at build time.
//
// Every ABI version bump must also update:
//   - `LUMINATE_C_ABI_VERSION` in `packaging/install-linux.sh`
//   - the asset paths and `[...provides]` entries (both the base package
//     and the `lib32` RPM variant) in `crates/luminated/Cargo.toml`
//
// The `c_abi_version` integration test verifies that these literals remain
// in sync.
fn main() {
    // `-soname` is an ELF/GNU ld concept; Windows has no equivalent, and
    // packaging only ships this cdylib on Linux. macOS has a direct analogue,
    // the Mach-O `LC_ID_DYLIB` install name, set via `-install_name`: without
    // it the cdylib links under whatever unversioned default name the
    // toolchain assigns, and consumers lose the loader-level ABI-mismatch
    // guarantee Linux gets from `DT_SONAME`.
    match env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("linux") => {
            println!(
                "cargo:rustc-cdylib-link-arg=-Wl,-soname,libluminate.so.{LUMINATE_C_ABI_VERSION}"
            );
        }
        // `@rpath`-relative rather than absolute, so the library keeps
        // resolving under any `-rpath` a consumer sets at link time instead
        // of being pinned to its build-time install location.
        Ok("macos") => {
            println!(
                "cargo:rustc-cdylib-link-arg=-Wl,-install_name,@rpath/libluminate.{LUMINATE_C_ABI_VERSION}.dylib"
            );
        }
        _ => {}
    }
}
