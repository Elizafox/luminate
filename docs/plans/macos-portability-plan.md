<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# macOS portability plan

## Status

Retained as a historical implementation and validation record. The port,
native paths, IOKit event sources, launchd integration, and source installer
are implemented. Current gaps are CI automation, signed and notarized
distribution, reboot-level `RunAtLoad` validation, and hands-on sleep and
hotplug checks. Current operational guidance lives in
`docs/development/packaging.md`,
`docs/development/architecture/macos-service.md`,
`docs/development/architecture/suspend-resume.md`, and
`docs/development/cross-platform-testing.md`.

### Current checklist

- [ ] Add automated Intel and Apple Silicon CI coverage equivalent to the
      existing manual contribution workflow.
- [ ] Validate launchd `RunAtLoad` across a real reboot.
- [ ] Exercise a real sleep/wake cycle and physical hotplug event end to end.
- [ ] Validate WLED and Govee against real devices on macOS.
- [ ] Design and ship signed/notarized package-manager distribution.

Luminate's cross-platform groundwork was driven almost entirely by the Windows
port. Much of that work carries over to macOS because it replaced Linux-specific
assumptions with portable ones, and macOS is a Unix. The socket-based IPC
transport, POSIX peer-credential authorization, private-storage guarantees,
shared-library naming, and `luminate-platform`'s common crate-root surface with
platform-specific bodies all already have a Unix branch that macOS compiles.

The macOS implementation and cross-platform code have now had real runs on both
`x86_64-apple-darwin` and `aarch64-apple-darwin` (see item 1). This document
records the remaining gaps and the work that established "works, tested, on a
Mac" on both architectures.

Cross-platform work shared across Unix (transport, auth, persistence, signals,
and dynamic-library naming) is not repeated here; this document covers only the
macOS-specific gaps. The dated gap descriptions below preserve the
implementation history; use the permanent guides above for current behaviour
and procedures.

---

## What macOS already inherits from the Unix work

None of this is macOS-specific effort. It is listed so the port doesn't
re-investigate ground the Windows work already cleared. All of it lives behind a
`cfg(unix)` or Unix-generic path that macOS compiles into unchanged:

- **Core IPC transport.** `luminate_platform::unix::transport`: Unix domain
  sockets, `SO_PEERCRED`-style peer credentials via `UnixStream::peer_cred()`,
  and socket-file permission checks (`PermissionsExt`/`MetadataExt`/
  `FileTypeExt`). macOS supports all of it. (`peer_cred` resolves to
  `LOCAL_PEERCRED`/`getpeereid` semantics on Darwin; the returned uid/pid are
  populated as the code expects; see "Verified on real hardware" below.)
- **Principal / authorization.** `Principal` construction from peer credentials
  and `luminate_platform::unix::identity::daemon_own_uid` (`libc::getuid()`,
  now a `cfg(unix)` dependency precisely so it compiles on macOS) are already
  Unix-generic.
- **Persistence security model.** Owner-private files and directories, plus
  owner-controlled service directories, have a Unix implementation that
  applies on macOS. Authority-file reads verify the opened regular file's owner
  and mode without following the final path component; service directories may
  be group-traversable but never group-writable. Administrators retain the
  trust boundary for ancestor directories. The Windows sweep also fixed every
  call site that pre-created a runtime directory to route through it, so there
  is no umask-versus-explicit-permission trap left to fall into on macOS either.
- **Shutdown signals.** `luminate_platform::process_signals` uses the Unix
  branch (`SIGTERM`/`SIGINT`/`SIGUSR1`) on macOS with no change.
- **Shared-library naming.** `dynamic_library_extension()` returns `"dylib"`
  and `dynamic_library_candidates()` produces `libfoo.dylib`/`foo.dylib` on
  macOS already; plugin discovery and the tests that write real dylib fixtures
  go through it.
- **Build-set gating.** `luminate-dbus` is opt-in behind the `dbus` feature and
  the hardware plugins are out of the default member set, so a plain
  `cargo build` on macOS never drags in D-Bus or hardware SDKs of doubtful
  relevance.

---

## macOS-specific gaps

Ordered roughly by how much stands between the current state and a working,
tested port. Each entry says what exists, what is missing, and where.

### 1. No macOS CI runner

**Partially resolved (2026-07-26):** there is now an occasional manual
runner: a macOS 14.8.7 VM (`x86_64-apple-darwin`, Xcode Command Line Tools,
rustup with the pinned `1.96.1` toolchain) reachable over SSH.
`cargo build --workspace`,
`cargo test --workspace --all-features`, `cargo fmt --all -- --check`, and
`cargo clippy --workspace --all-targets --all-features -- -D warnings` all run
clean on it (see "Verified on real hardware" below for what that exercised).
An M1 MacBook Pro running macOS Tahoe provides a second manual test host.
On 2026-07-27 it passed the full contribution workflow, the source-installer
and launchd service lifecycle, and the plugin tests. The LIFX plugin also
worked against real hardware.

Remaining gap: wire an equivalent runner into CI. Both Intel and Apple Silicon
coverage currently depend on manually maintained hosts.

### 2. `crates/luminate-platform/src/macos/mod.rs` is mostly still a stub

It now carries a real `default_path` body (item 5, resolved) alongside the
`test-support`-gated `dylib` submodule (the `otool -D` install-name reader
used by the C ABI smoke test), but most macOS-specific runtime facts the
crate is meant to own are still absent. The module layout uses platform bodies
with no internal `#[cfg]`, dispatched to from the cross-platform module at the
crate root. The next thing macOS needs there is a body for power and device
change events (item 3).

### 3. No suspend/resume or device-change (hotplug) source

**Resolved (2026-07-27):** `luminate_platform::power::start_sources` now
dispatches to `crate::macos::power::start` on macOS
(`crates/luminate-platform/src/macos/power/`), alongside the pre-existing
Linux and Windows sources. Both halves are backed by IOKit notifications
delivered through a `CFRunLoop` running on a dedicated `std::thread` (not a
tokio task: the run loop parks the thread for the rest of the process's
life, which a tokio worker has nothing to gain from blocking on):

- **Suspend/resume** (`macos/power/sleep.rs`): `IORegisterForSystemPower` with
  an `IOServiceInterestCallback` watching for `kIOMessageSystemWillSleep` and
  `kIOMessageSystemHasPoweredOn`. `IOAllowPowerChange` is the acknowledgement
  that lets a pending sleep proceed; it is wrapped in a `PowerChangeAck` held
  inside the `SuspendLease` returned to the caller (a new `#[cfg(target_os =
"macos")]` field on the cross-platform `SuspendLease` type in
  `power/mod.rs`), so the acknowledgement fires exactly when that lease
  drops, the same ownership model logind's delay inhibitor already uses on
  Linux, but with a different resource underneath.
- **Hotplug / `DevicesChanged`** (`macos/power/hotplug.rs`):
  `IOServiceAddMatchingNotification` against the generic `"IOService"` class
  for both `kIOFirstMatchNotification` and `kIOTerminatedNotification`,
  deliberately not narrowed to a specific device class (USB, HID, ...),
  mirroring the Linux uevent reader and the Windows device-event handling,
  neither of which filter by class either, on the same reasoning that
  guessing which plugin cares is the daemon's topology-repull's job, not this
  source's. A 750ms debounce (matching the Linux uevent reader's) absorbs
  the resulting burstier signal, running on a second dedicated thread reading
  a blocking `std::sync::mpsc` channel the `CFRunLoop` thread pings.

**Dependency decision:** `objc2-io-kit` + `objc2-core-foundation` (both
0.3.2, part of the actively maintained `madsmtm/objc2` project, Zlib/Apache-2.0/MIT).
Considered and rejected first: `io-kit-sys`, which has no binding at all for
`IORegisterForSystemPower`/`IOAllowPowerChange`/the `kIOMessage*` constants
(only the device-matching half), so it would not have saved the hand-written
`extern "C"` surface a narrow FFI shim needs anyway. `objc2-io-kit` covers the
whole surface (power notifications and service matching) directly. Default
features are left on for now rather than curated down, since this workspace
has no macOS CI and only occasional manual VM runs (item 1) to catch a
feature-trimming mistake; revisit once this has had a real build/test pass
there.

**Verified on real hardware (2026-07-27):** written and first checked with
`cargo check`/`cargo clippy --all-features --all-targets -D warnings` against
both the `x86_64-apple-darwin` and `aarch64-apple-darwin` targets from this
Linux workspace (catching real compile and lint errors without a Mac), then
confirmed on the `x86_64-apple-darwin` VM from item 1: `cargo build`,
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
--all-features -- -D warnings`, and `cargo test --workspace --all-features`
all ran clean there (the only failures are the two pre-existing, expected
Polkit ones (see "Known, expected macOS test failure" below) unrelated to
this item). Beyond the test suite, a throwaway example calling
`luminate_platform::power::start_sources()` and logging every event for a
few seconds showed both sources registering and arming cleanly with no
errors or panics:

```
INFO luminate_platform::macos::power::sleep: watching IOKit for suspend and resume
INFO luminate_platform::macos::power::hotplug: watching IOKit for device changes
```

That confirms `IORegisterForSystemPower` and both
`IOServiceAddMatchingNotification` registrations succeed, the initial
existing-match drain terminates rather than hanging, and the `CFRunLoop`
source attachment works, all on real Darwin. Not exercised: an actual
suspend/resume cycle (the VM was left running rather than risking an SSH-losing
sleep) or a real USB/HID hotplug event feeding through the debounce. Both
still need a hands-on pass whenever that's convenient.

### 4. `libluminate`'s `build.rs` emits no versioned install name on macOS

**Resolved (2026-07-27):** on Linux, `build.rs` emits
`-Wl,-soname,libluminate.so.<ABI>` so the loader refuses an ABI-mismatched
library at load time. macOS has a direct equivalent (a Mach-O `LC_ID_DYLIB`
install name, settable with `-install_name` at link time or
`install_name_tool -id` after) and `build.rs` now emits it too:
`-Wl,-install_name,@rpath/libluminate.<ABI>.dylib`. It is `@rpath`-relative so
the library keeps resolving under whatever `-rpath` a consumer sets, instead of
being pinned to its build-time location.
`luminate_platform::test_support::ensure_dylib_runtime_link`'s macOS branch
(the `otool -D` reader) already expected this format (its test fixture is
`@rpath/libluminate.4.dylib`) so no change was needed on that side. Re-run on
the VM from item 1: all 14 `c_api_smoke.rs` tests pass with the versioned
install name in place.

While verifying this, running the suite with its default parallel test
threads (rather than `--test-threads=1`) surfaced a pre-existing, unrelated
race: every test calls `build_libluminate()`, and concurrent threads each
launched their own `cargo build -p libluminate` and could observe
`target/debug/libluminate.dylib` mid-replacement by another thread's build,
failing `otool -D` with "No such file or directory". Fixed by wrapping the
build-and-link step in `build_libluminate()` in a `OnceLock`, so only the
first caller does the work and the rest block until it finishes. Confirmed
fixed with three consecutive default-parallelism runs on the VM.

**Toolchain note found while testing (2026-07-26):** the `otool -D` reader and
`c_api_smoke.rs` run and pass on the VM from item 1, but only with a
newer-than-Xcode clang. Apple's shipped Xcode 16 clang (16.0.0) rejects
`-std=c23` ("use ... 'c2x' for 'Working Draft for ISO C2x'") even though it
implements the same draft under that older spelling, and
`generated_header_compiles_and_links_as_strict_c23` compiles against it. Fixed
by having `c_api_smoke.rs` honour `CC`/`CXX` from the environment (mirroring
the existing `CARGO` lookup in `build_libluminate`) instead of hardcoding
`Command::new("cc")`/`Command::new("c++")`, so a contributor with a newer
compiler installed, e.g. Homebrew's `llvm` formula (clang 22.1.8, which
accepts `-std=c23`), can point at it directly:
`CC=$(brew --prefix llvm)/bin/clang CXX=$(brew --prefix llvm)/bin/clang++
cargo test -p libluminate --test c_api_smoke`. No `PATH` symlinking needed.
Decided: require a newer-than-Xcode clang for that specific test on macOS
going forward, documented here, rather than loosening the test's C standard.
Worth a line in whatever macOS contributor setup docs eventually exist.

### 5. Default paths are Linux-FHS-shaped

**Resolved (2026-07-27):** the defaults now owned by `luminate-platform` resolve through
`luminate_platform::default_path`, which now has a real
`crate::macos::default_path` body instead of falling into the generic Unix
(Linux-FHS) branch. The daemon-versus-agent question this depended on is
decided: macOS runs `luminated` as a system `LaunchDaemon`, the same
multi-user, machine-wide shape Linux's systemd unit and the Windows service
already use, rather than a per-user `LaunchAgent`. That keeps the existing
`Principal`/peer-credential authorization design intact, instead of stretching
it to cover a per-user model it was never built for. Chosen defaults:

| Purpose          | Path                                                     |
| ---------------- | -------------------------------------------------------- |
| Config           | `/Library/Application Support/Luminate/luminated.toml`   |
| Socket           | `/var/run/luminated/luminated.sock`                      |
| State            | `/Library/Application Support/Luminate/state/state.json` |
| Plugins (local)  | `/usr/local/lib/luminate/plugins`                        |
| Plugins (system) | `/Library/Application Support/Luminate/plugins`          |

`/Library/Application Support/Luminate` is the macOS analogue of
`windows::default_path::program_data_root()`'s `%ProgramData%\Luminate`.
`/Library/Preferences` was considered and rejected, since it's conventionally
for `.plist`-shaped preferences rather than an arbitrary config file. The
local/system plugin split mirrors the Unix FHS distinction it replaces
(locally built/installed vs. distro- or installer-packaged). All of these
remain overridable through the existing `LUMINATE_DEFAULT_*` environment
variables, unchanged.

**Corrected (2026-07-27, during item 6):** the socket path is
`/var/run/luminated/luminated.sock`, a dedicated runtime subdirectory, not
`/var/run/luminated.sock` directly as originally chosen here. Item 6 decided
`luminated` runs as the unprivileged `_luminated` account, and `/var/run`
itself is `root:wheel` mode `0755`, so that account cannot create a socket
file there directly. `service install` creates `/var/run/luminated` `0750`,
owned by `_luminated:_luminate`, mirroring the Linux packaged
`RUNTIME_DIR=/run/luminated` layout exactly. See
`docs/development/architecture/macos-service.md`, "Directory and socket
layout," for the full reasoning.

### 6. No launchd service integration or packaging

**Resolved (2026-07-27):** see `docs/development/architecture/macos-service.md`
for the full design. `luminate_platform::macos::service` (the dedicated
`_luminated`/`_luminate` account via `dscl`, plist rendering, and `launchctl`
wrappers) and `luminated`'s `service install|uninstall|start|stop|status`
subcommand are implemented, mirroring the Windows self-install subcommand at
the much smaller scope launchd needs — there is no SCM-equivalent
control-handler bridge or `RunContext` variant, since launchd execs the
configured program as an ordinary foreground process and stops it with
`SIGTERM`, which the existing `RunContext::console()` path and
`process_signals` already handle. Checked with `cargo check`/`clippy` against
both Darwin targets from this Linux workspace and with build/lint/test plus
the elevated service lifecycle on the macOS VM from item 1. The two known
Polkit tests fail as expected; the remainder passes when they are skipped. A
reboot-level `RunAtLoad` check remains untested. See
`docs/development/architecture/macos-service.md`.

### 7. Hardware plugins: portable, with partial real-device coverage

**Runtime verified on Intel and Apple Silicon macOS (2026-07-27):** all eight
portable plugin crates (`lifx`, `wled`, `govee`, and the five `demo-*` plugins)
pass `cargo check` for both `x86_64-apple-darwin` and
`aarch64-apple-darwin`. On the Intel macOS VM from item 1, their hermetic test
suites pass (142 tests, including WLED's normally ignored loopback HTTP test).
All 12 ignored plugin-conformance tests also pass there through the real
out-of-process plugin host: LIFX and WLED
communicate with mock network devices, Govee boots with an empty stable
topology, and the demo plugins load as real shared libraries and satisfy the
host protocol. The demo-display client/daemon integration test additionally
delivers frames through a real iceoryx2 shared-memory stream.

The same plugin tests pass on the M1 MacBook Pro from item 1, establishing
Apple Silicon runtime coverage too. The LIFX plugin was also tested against
real lights and worked correctly. No physical controller was available to the
Intel VM for a real-device check. It uses QEMU user-mode NAT (`10.0.2.15` behind
`10.0.2.2`), so broadcast and mDNS discovery cannot reach controllers on the
host LAN either.

- **Network / no-OS-access plugins** (`lifx`, `wled`, `govee`, `demo-*`) are
  already portable in principle (`std::net`, `socket2`, or no OS access) and
  now have cross-compile and Intel and Apple Silicon macOS runtime coverage.
  LIFX discovery and control are verified against real hardware on Apple
  Silicon; WLED and Govee real-device operation remain untested on macOS.
- **`alienware`** is a `hidapi` HID plugin. `hidapi` has a macOS backend, so it
  compiles and could work, but Alienware hardware only appears on a Mac under a
  Hackintosh, so it is effectively irrelevant on macOS and low priority
  regardless of code readiness.
- **`luminate-plugin-linux-leds`** is Linux sysfs-specific by name and design;
  it has no macOS counterpart and needs none. The macOS analogue, if ever
  wanted, would be a separate IOKit-based plugin rather than a port of this one.

No plugin needs macOS-specific code to build. LIFX behaviour is verified on
Apple Silicon macOS; WLED and Govee still need real-device checks on a Mac.

### 8. `luminate-dbus` is an optional, unusual macOS configuration

The crate remains Linux-oriented in its caller-identity internals
(`/proc/<pid>/stat` and `/etc/group`) and is opt-in behind the `dbus` feature,
off by default. D-Bus and Polkit can nevertheless be installed on macOS.
`luminate-dbus` therefore searches the trusted Homebrew and system binary
directories for `pkcheck` before falling back to Linux's conventional
`/usr/bin/pkcheck`. A usable macOS deployment must also provide the process
and group information those caller-identity checks require.

### 9. IPC transport: keep Unix domain sockets, not Mach IPC

Decided, and recorded here so it isn't re-litigated: the macOS port keeps the
existing Unix-domain-socket transport (`luminate_platform::unix::transport`) and
does not adopt raw Mach IPC.

Reasoning:

- UDS already works on Darwin, so Mach would be net-new platform-forking code
  for no transport-level benefit.
- Mach is a port-and-message model rather than a byte stream; adopting it means
  either tunnelling the existing framed protocol over Mach messages or forking
  the protocol per platform, working against the "common surface, thin platform
  body" shape this crate is built around.
- There is no performance case: the pixel hot path already goes over iceoryx2
  shared memory, so the socket carries only low-throughput control traffic
  (requests/events).
- launchd does not force it: launchd socket activation works with UNIX domain
  sockets via the plist `Sockets` key, so item 6 doesn't drag Mach in either.

What Mach would buy is race-free, kernel-attested peer identity, and that argues
for XPC (the high-level framework built on Mach, whose connections carry an
`audit_token`) rather than hand-rolled Mach ports. XPC is a macOS-native
front-end and a substantial investment, worth making only for a specific goal:
sandboxed or App-Store-style distribution, or an auth posture stronger than UDS
peer credentials can give. It stays deferred until such a goal exists.

The trigger for reopening this is the `UnixStream::peer_cred()`
verification in "Verified on real hardware" below: macOS peer creds
(`LOCAL_PEERCRED`/`getpeereid`) are weaker than the Linux `SO_PEERCRED` the auth
path is modelled on, and PID-based lookups carry the usual PID-reuse TOCTOU
risk. If that check turns out insufficient for the authorization model, the fix
is audit tokens via XPC.

---

## Verified on real hardware

Confirmed on both architectures: on 2026-07-26 on the
`x86_64-apple-darwin` VM, and on 2026-07-27 on an M1 MacBook Pro running
macOS Tahoe. The full contribution workflow passed on both hosts.

- The macOS branch of `luminate_platform::test_support::ensure_dylib_runtime_link`
  and therefore `c_api_smoke.rs`: the `otool -D` install-name parsing runs and
  all 14 `c_api_smoke.rs` tests pass, including the C23 and C++17 header-smoke
  tests (see item 4's toolchain note for the clang-version caveat).
- `UnixStream::peer_cred()` on Darwin: the
  `unix::transport::tests::accept_captures_peer_credentials_and_yields_a_connection`
  test and the rest of `unix::transport`'s test suite pass, so the uid/pid
  `Principal` construction depends on are populated as expected under macOS's
  `LOCAL_PEERCRED`/`getpeereid` semantics. This covers only the happy path the
  existing tests exercise, and says nothing about the strength of the auth
  model; item 9's note about PID-reuse TOCTOU and the XPC audit-token
  alternative still stands.
- private-directory, service-directory, and private-file guarantees: exercised
  (and passing) via `unix::secure_storage::tests` for distinct directory modes,
  final-component symlinks, permissive files, sockets, and non-blocking FIFO
  rejection, alongside `unix::transport::tests` for the group-traversable
  socket layout.
- The whole default no-source power path:
  `power::tests::starting_sources_never_fails_and_reports_no_spurious_events`
  passes on macOS; `start_sources` parks cleanly and never emits an
  unprompted event, matching the Linux-verified behaviour.

## D-Bus and Polkit tests on macOS

The Polkit command tests use the shared `/usr/bin/true` and `/usr/bin/false`
utilities rather than Linux's `/bin` paths. The production lookup separately
tests its ordered trusted-directory search and `/usr/bin/pkcheck` fallback.

---

## Suggested order of work

- [x] Stand up macOS build/test paths (item 1) for
      `x86_64-apple-darwin` via a manual VM (2026-07-26) and
      `aarch64-apple-darwin` via an M1 MacBook Pro running macOS Tahoe
      (2026-07-27). CI automation is still open.
- [x] Emit the versioned `-install_name` in `build.rs` (item 4)
      (2026-07-27), mirroring the Linux SONAME logic, and confirmed with a
      re-run of `c_api_smoke.rs` on the VM from item 1. That run also surfaced
      and fixed an unrelated `build_libluminate()` test-parallelism race (see
      item 4).
- [x] Decide the daemon-versus-agent model and default paths (item 5)
      (2026-07-27): system `LaunchDaemon` model, defaults implemented in
      `crate::macos::default_path` (the socket path was later corrected; see
      item 5's "Corrected" note).
- [x] Implement the IOKit power/device-change source (item 3)
      (2026-07-27): `objc2-io-kit`/`objc2-core-foundation`, checked with
      `cargo check`/`clippy` against both Darwin targets from this Linux
      workspace, then confirmed with the full contribution workflow plus a live
      registration smoke test on the VM from item 1. An actual suspend/resume
      cycle and a real USB/HID hotplug event are still unexercised; worth a
      hands-on pass whenever convenient.
- [x] Implement launchd service integration and packaging (item 6)
      (2026-07-27): `luminate_platform::macos::service` and `luminated service
 install|uninstall|start|stop|status`; see
      `docs/development/architecture/macos-service.md`.
      Checked with `cargo check`/`clippy` against both Darwin targets and with
      build/lint/test plus the elevated lifecycle on the VM from item 1. A
      reboot-level `RunAtLoad` check remains.
- [x] Repeat the portable-plugin hermetic and conformance tests on macOS on the
      Intel VM and M1 MacBook Pro (2026-07-27): 142 hermetic plugin
      tests, all 12
      out-of-process conformance tests, and the demo-display shared-memory
      integration test pass. LIFX discovery and control also work against real
      lights on the M1; WLED and Govee still need equivalent real-hardware checks.

---

## Notes for whoever resumes this

- `luminate-platform` is the home for macOS-specific runtime facts. Put IOKit
  code under `crate::macos` with no internal `#[cfg]`; the module boundary is
  the gate. Don't scatter `cfg(target_os = "macos")` across consumers.
- Before adding any IOKit-binding crate, get dependency sign-off per
  `CONTRIBUTING.md` and run `cargo audit`.
- Keep the permanent architecture documents clear about which behaviour is
  shared across Unix and which is platform-specific.
- macOS and Windows now have platform event sources feeding the shared daemon
  rescan plumbing. The remaining real-event validation is summarized in
  `docs/TODO.md`.
