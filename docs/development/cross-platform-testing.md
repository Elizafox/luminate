<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Cross-platform integration testing

Local smoke-test entry points cover Linux, macOS, and Windows development, with
convenience runners for the manually maintained macOS and Windows libvirt
guests. The VM runners remain deliberately opt-in: they depend on local virtual
machines, SSH credentials, guest toolchains, and permission to alter
machine-wide service state.

## Goals

- Give each supported host a native smoke-test entry point.
- Keep independently useful test phases runnable on their own.
- Use the same platform-local test recipe on physical hosts and VM guests.
- Exercise process-level integration suites which the ordinary workspace test
  command leaves ignored.
- Keep privileged service lifecycle tests explicit on local machines.
- Shut down every VM guest gracefully after success or failure.
- Test the current working tree, including uncommitted and untracked files,
  without copying build output or repository metadata.

## Local interface

The top-level entry points always test the machine on which they run:

```sh
scripts/smoke-test-linux.sh
scripts/smoke-test-macos.sh
```

```powershell
scripts/smoke-test-windows.ps1
```

They are thin wrappers around drivers and independently executable phases in
`scripts/linux/`, `scripts/macos/`, and `scripts/windows/`. Phases execute in
their own process instead of being sourced, so their traps, shell options, and
environment changes cannot leak into later phases.

The ordinary local run never installs, removes, starts, or stops a
machine-wide service. Pass `--privileged` on macOS or `-Privileged` on Windows
to opt in to that platform's service lifecycle test. Linux currently has no
host-mutating service phase; its packaging and init-system coverage remains in
the focused scripts under `packaging/`.

## D-Bus support

D-Bus has deliberately different support expectations on each platform:

- **Linux (tier 1):** the private-bus integration phase is required.
- **macOS (tier 2):** the phase runs when `dbus-run-session` is installed and
  is otherwise reported as skipped. A failure is reported but does not block
  the default smoke run.
- **Windows (tier 3):** the phase runs opportunistically when
  `dbus-run-session.exe` is installed. Missing prerequisites and test failures
  are reported but do not fail the default smoke run.

Pass `--require-dbus` or `-RequireDBus` to make the D-Bus phase and its
prerequisites mandatory on macOS or Windows. Pass `--skip-dbus` or `-SkipDBus`
to omit it explicitly. The two options are mutually exclusive.

Windows D-Bus support is experimental compatibility, not a promise of native
service integration or a conventional Windows system bus. Straightforward
portability fixes are welcome, but Windows D-Bus failures do not block
unrelated development or releases.

The private-bus suite exercises the complete versioned interface contract,
including ObjectManager ordering, authorization, reconnects, control,
collections, scenes, transitions, ordinary frames, setup, and administration.
See the [D-Bus consumer guide](dbus.md) for the contract being tested.

## VM interface

The Unix/libvirt host-side conveniences are named separately from the local
entry points:

```sh
scripts/vm-smoke-test-macos.sh
scripts/vm-smoke-test-windows.sh
```

Each accepts its VM name, address, SSH account, remote workspace, and timeouts
through environment variables documented in its opening comments. Defaults
describe the local development guests, but no guest-specific value is part of
a public interface.

Each VM runner:

1. Refuses to start when its domain is already running. This avoids taking
   ownership of a guest the caller may be using.
2. Starts the domain through the configured libvirt connection.
3. Waits for network and SSH readiness.
4. Archives the current workspace while excluding `.git` and `target`, then
   replaces and extracts the disposable remote workspace. Cargo build output
   lives in a guest-local cache outside that workspace, exposed through a
   workspace-local symlink or directory junction.
5. Invokes the checked-in platform-local smoke driver with its privileged phase
   enabled. The VM runner contains no Cargo test recipe of its own.
6. Asks the guest OS to power off and waits until libvirt reports `shut off`.

An exit trap requests the same graceful shutdown after an interrupted or
failed phase. It intentionally does not destroy the domain: forced power-off
can corrupt a guest and is not a truthful substitute for testing shutdown.
Failure to reach `shut off` is itself a failed smoke test and requires manual
recovery.

The two scripts do not invoke one another. Sequential execution remains an
operator choice for hosts without enough memory to run both guests.

## Coverage script layout

Coverage implementations live under `scripts/coverage/`. The top-level
`scripts/coverage.sh` dispatcher accepts `--workspace`, `--luminated`, or
`--libluminate`; it defaults to the workspace report.

## Test matrix

Every platform runs formatting, Clippy, the full all-feature workspace suite,
and the portable ignored CLI, plugin-conformance, and shared-memory process
tests. The shared-memory test uses the platform transport for its control
connection, then exercises the complete iceoryx2 client-to-daemon-to-plugin
host relay.

Linux additionally requires the ignored D-Bus integration suite on a private
session bus. macOS and Windows use the tiered D-Bus behaviour described above.
The macOS workspace phase selects Homebrew LLVM's `clang` and `clang++`, as
required by the strict C23 header test; Xcode 16's bundled compiler is too old
for that test.

The privileged macOS phase stages and installs the source layout, exercises the
LaunchDaemon lifecycle, and uninstalls it. The privileged Windows phase runs
the ignored `windows_service_install` suite serially from an elevated session.

Linux package formats, D-Bus packaging, systemd, OpenRC, containers, generated
headers, dependency auditing, and other major-change checks remain in
`scripts/major-check.local.sh` and the focused `packaging/` scripts. Hardware
writing tests remain excluded; their explicit safety gates are not enabled by
any smoke driver.

## Security and compatibility

SSH host-key checking remains enabled. A previously unseen guest key is added
to the caller's normal known-hosts file, while a changed key is rejected. The
scripts do not copy private keys or accept passwords.

Remote cleanup is confined to the configured workspace path, whose default is
inside the configured account's profile. VM service tests change real
guest-wide state and must only target disposable development machines. Local
service tests require an explicit option because they make the same changes on
the developer's machine.

Renaming the former host-side `scripts/smoke-test-macos.sh` and
`scripts/smoke-test-windows.sh` entry points is a pre-release developer
interface break. The old Windows `.sh` name is intentionally not retained: a
script with that name should not misleadingly require a Unix VM host. These
scripts add no product API, ABI, protocol, persistence, or packaging
compatibility commitment.
