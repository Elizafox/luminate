<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Running `luminated` under launchd

`luminated` installs and runs as a system `LaunchDaemon` on macOS, the
counterpart to the Linux systemd unit and the Windows Service Control Manager
integration. This document records the settled service design; native event
sources are documented in [Suspend, resume, and hardware
rescanning](suspend-resume.md), and installation procedures live in the
[packaging guide](../packaging.md).

## Context

Windows needed the Service Control Manager because nothing else on that
platform could deliver shutdown notification, suspend/resume, or device
hotplug to a headless process.
macOS is a different shape: IOKit delivers suspend/resume and hotplug
self-driven, with no service-manager cooperation required, and
`luminate_platform::process_signals` already handles `SIGTERM`/`SIGINT` on
every Unix target including Darwin. launchd therefore closes no signal or
event gap that the IOKit and Unix work do not already close.

What launchd actually buys is what systemd's unit already buys on Linux:
supervised lifecycle (start at boot, restart on crash) for a process that
otherwise has to be launched and babysat by hand. `launchd` execs the
configured program as an ordinary foreground process and stops it by sending
`SIGTERM` (escalating to `SIGKILL` after its timeout) — there is no
`SERVICE_CONTROL_STOP`-style callback, no separate control-handler thread, and
no equivalent of `StartServiceCtrlDispatcherW`. The existing
`daemon::RunContext::console()` path — Tokio-installed `SIGTERM`/`SIGINT`
handling, no readiness/progress reporting, no operator-event target — is
already the right shape for a launchd-run process. There is no service-
control-handler bridge and no runtime-inversion equivalent to
Windows's SCM entry-point inversion. This is the biggest structural difference
from the Windows integration, and why macOS needs far less machinery.

launchd socket activation, a `.pkg`/notarization pipeline, XPC, and a
privileged helper are all deliberately out of scope; the transport stays Unix
domain sockets and packaging is covered separately by
`docs/development/packaging.md`.

## Service account

`luminated` runs as a dedicated unprivileged **`_luminated`** user and
**`_luminate`** group, mirroring the packaged Linux model
(`docs/development/packaging.md`'s `luminated`/`luminate` pair) rather than
the Windows LocalSystem choice. This is the opposite call from Windows because
the platforms' hardware-access constraints differ:

- Windows chose LocalSystem because Windows device-interface DACLs are set per
  driver INF and commonly grant only `SYSTEM`/`Administrators`, so an
  unprivileged service SID would need per-device-class ACL grants on hardware
  nobody involved in that decision owned.
- macOS's analogous access story is IOKit/`hidapi`, which does not require
  root for the plugins this project ships: `hidapi`'s macOS backend opens HID
  devices through `IOHIDDeviceOpen` using ordinary user-session access, and
  none of the in-tree Unix plugins (`lifx`, `wled`, `govee`, `demo-*`) touch
  host hardware at all. There is no known macOS device class among Luminate's
  current plugins that needs root, so running unprivileged is strictly better
  security posture.
- `alienware` (`hidapi`) is Hackintosh-only on macOS and low priority there. If
  it or a future macOS-only hardware plugin turns out to need root-only device
  access, that is grounds to revisit the account decision for that plugin
  specifically, not to make the whole daemon privileged again.

Account naming follows Apple convention for system accounts: a leading
underscore, no login shell, home directory `/var/empty`. `_luminated` and
`_luminate` are the underscore-prefixed counterparts of the Linux
`luminated`/`luminate` pair, kept recognizable as the same concept without
needing to literally match (the same reasoning gave the Windows integration
its distinct `Luminate Clients` group name).

### Creation mechanism

Account creation shells out to `dscl` rather than binding Open Directory
through FFI: macOS has no `useradd`/`sysusers.d` equivalent, shelling out to a
narrow external tool for a privileged, infrequently-run operation matches the
project's existing convention (`luminate-dbus`'s `pkcheck` invocation is the
precedent), and it needs no new dependency. `dscl .` gives precise,
individually-settable attributes and clean success/failure per call, which
matters for install's partial-failure rollback; the higher-level
`sysadminctl -addUser` is interactive-oriented with weaker scripting
guarantees.

### UID/GID allocation

macOS reserves roughly UID/GID 200–400 for system accounts by convention (no
enforced kernel range, unlike Linux's `SYS_UID_MIN`/`SYS_UID_MAX`), and has no
atomic "allocate me a free system UID" primitive the way `sysusers.d` provides
on Linux. Installation reads existing values with
`dscl . -list /Users UniqueID` and `dscl . -list /Groups PrimaryGroupID`,
picks the lowest unused value in `200..=400` for the user and, independently,
for the group (they need not match numerically), and refuses installation
with a clear error if the range is exhausted rather than silently spilling
outside it. This has a narrow TOCTOU race against a concurrent account
creation on the same machine, accepted because installation is a single
administrator-driven operation — the same non-concurrency assumption the
Windows installer makes for its own registry/SCM writes.

### Ownership and idempotency

macOS has no registry-key-style metadata store to record installer-owned
objects the way Windows does for its client group and Event Log source, so
ownership is determined by exact attribute match instead:

- Fresh install: if `_luminated`/`_luminate` do not exist, create them with
  the fixed attribute set below.
- Idempotent reinstall: if they exist and every attribute below matches
  exactly, treat them as Luminate's own and proceed without modification.
- Conflict: if the name exists with any differing attribute (shell, home
  directory, real name, primary group), refuse and report the conflict rather
  than overwriting a pre-existing, unrelated account — the same posture as
  Windows' refusal to replace a same-named service pointing elsewhere.

Fixed user attributes, all set at creation and checked at reinstall:

| Attribute          | Value                      |
| ------------------ | -------------------------- |
| `RealName`         | `Luminate Lighting Daemon` |
| `UserShell`        | `/sbin/nologin`            |
| `NFSHomeDirectory` | `/var/empty`               |
| `PrimaryGroupID`   | the `_luminate` GID        |

The `_luminate` group has a fixed `RealName` of
`Luminate Lighting Daemon Group`; this is its ownership marker, and a
same-named group without that exact attribute is treated as an unrelated
conflict.

No `Password` attribute is set: `dscl`'s password-setting subcommands are
interactive/prompt-oriented and not worth scripting around, since
`UserShell = /sbin/nologin` alone already refuses any interactive or remote
login attempt and nothing ever authenticates as this account.

Uninstall does not remove the account or group by default, for the same
reason Windows keeps its client group by default: it may have gained extra
permissions or memberships since install. `--purge-account` removes both, and
only when their attributes still exactly match the table above.

## Directory and socket layout

`service install` creates:

- `/Library/Application Support/Luminate` as `root:_luminate 0750` (deny by
  default against ordinary users, mirroring the Windows
  `protect_machine_data_root` hardening rationale, while still letting the
  daemon traverse to its state);
- `state/` beneath it as `_luminated:_luminate 0700` (daemon-created and
  owned, the same posture `secure_storage` gives Linux and Windows);
- `/Library/Logs/luminated` as `_luminated:_luminate 0750`; and
- `/var/run/luminated` as `_luminated:_luminate 0750`, holding the socket
  file.

The runtime directory matters because `/var/run` (`/private/var/run`) itself
is `root:wheel` mode `0755`, so the unprivileged `_luminated` account cannot
create a socket file directly inside it. `/var/run/luminated/luminated.sock`
mirrors the Linux packaged layout exactly (`RUNTIME_DIR=/run/luminated`,
`packaging/systemd/luminated.service.in`'s `ExecStartPre install -d -m 0750`).

Config (`luminated.toml`) is `root:_luminate 0640`: administrator-writable,
daemon-readable, matching the Linux packaged config file's posture. The
binary installs to `/usr/local/bin/luminated`, consistent with the
`PLUGIN_DIR_LOCAL=/usr/local/lib/luminate/plugins` default; `service install`
reads its own `std::env::current_exe()` and rejects a non-canonical or
user-writable install location unless `--allow-development-path` is passed,
the same posture as the Windows installer.

## The plist

`crates/luminate-platform/src/macos/service/com.wilcoxti.luminate.luminated.plist.in`
is an embedded template substituted the same way
`packaging/install-linux.sh` substitutes the systemd unit;
`luminate_platform::macos::service::plist::render` embeds it in the binary
rather than reading the source tree at runtime. No `plist`/`plist-rs`-style
crate dependency: a launchd plist here needs only a small, fixed set of
known-safe keys (a path, a user/group name, booleans) with no untrusted input
beyond what `validate_abs_path`/`validate_account_name` already check for the
equivalent systemd substitutions, so hand-written XML is the same "not worth a
dependency for this" call the Windows integration made for its Event Log
registration.

Key points of the rendered plist:

- `Label` (`com.wilcoxti.luminate.luminated`) matches the plist's own
  filename; launchd requires this correspondence and silently misbehaves if
  it drifts.
- `KeepAlive.SuccessfulExit = false` restarts only on a nonzero/signalled
  exit, the launchd analogue of systemd's `Restart=on-failure` and Windows'
  non-clean-stop-only failure actions. A clean `service stop` (`SIGTERM`, exit
  0) does not trigger a restart loop.
- No `Sockets` key: launchd socket activation is not adopted, for the same
  reason the Linux unit does not use `sd_notify`/socket activation — no
  existing implementor to generalize a seam for, and the daemon binds its own
  socket today on every platform.
- No Event-Log-style structured operator channel. `StandardOutPath`/
  `StandardErrorPath` capture the daemon's existing
  `tracing_subscriber::fmt()` console output into a rotating-free plain file.
  There is no macOS analogue of Windows Event Log in scope; if log rotation is
  wanted later, `newsyslog` (configured via `/etc/newsyslog.d`) is the natural
  fit.

## Installer mechanism

`luminated service install|uninstall|start|stop|status` extends
`crates/luminated/src/service_command.rs`'s dispatch with a macOS arm backed
by `luminate_platform::macos::service`, the same shape as
`luminate_platform::windows::service` minus everything that module needs
purely because of the SCM (no dispatcher, no control handler, no status
reporting; see "Context" above).

This was chosen over extending `packaging/install-linux.sh` with a `launchd`
`INIT_SYSTEM` branch: that script's logic (`sysusers.d`, `tmpfiles.d`, udev
rules, FHS paths) is Linux-specific enough that a macOS branch would mostly be
new code living awkwardly inside a shell script built for a different
platform's primitives. A self-install subcommand mirrors the Windows
precedent directly: one privileged code path for install and uninstall,
uninstall symmetry falls out for free, and there is no second artefact to
keep in sync.

Subcommands, mirroring the Windows names and flag shapes where the concepts
line up:

- **`install [--allow-development-path] [--start]`** — creates
  `_luminated`/`_luminate` if absent (idempotent; see "Ownership and
  idempotency"), creates and hardens the data/log/runtime directories, writes
  the plist to `/Library/LaunchDaemons/com.wilcoxti.luminate.luminated.plist`,
  and — only if `--start` is passed — runs
  `launchctl bootstrap system <plist>` to start it immediately. Without
  `--start`, `RunAtLoad` takes effect at the next boot on its own: unlike SCM,
  launchd needs no separate "register" step distinct from the plist file
  existing under `/Library/LaunchDaemons`.
- **`uninstall [--purge-account]`** — if currently loaded,
  `launchctl bootout system/com.wilcoxti.luminate.luminated` (tolerating "not
  loaded"), then removes the plist. `--purge-account` additionally removes
  `_luminated`/`_luminate`, only when their attributes still exactly match
  what installation set; otherwise it refuses and reports why.
- **`start`** / **`stop`** — `launchctl kickstart -k` /
  `launchctl kill SIGTERM`, thin wrappers reported the same way as the
  Windows `start`/`stop` subcommands.
- **`status`** — `launchctl print`, parsed into a small typed state
  (`NotLoaded`/`Loaded { running: bool }`), mirroring
  `windows::service::ServiceState`.

Rollback on partial install failure mirrors the Windows installer: if a later
step in `install` fails, only objects that invocation itself created are torn
down; an account or directory that pre-existed and matched is left alone.

Unlike a Windows service, a launchd `LaunchDaemon` never needs a
machine-facing install to exercise the daemon itself: the console path
(`cargo run -p luminated`, or the binary invoked directly under `sudo`) is
fully representative of what launchd will run, since there is no SCM
handshake to diverge from. Install/uninstall only need testing as themselves,
not as a proxy for whether the daemon runs correctly under supervision.

## Notes for future work

- Keep the account/plist/launchctl logic under `luminate_platform::macos`,
  with no internal `#[cfg]`; the module boundary is the gate.
- If a future macOS hardware plugin genuinely needs root, revisit the account
  decision for that plugin's device access specifically (a narrower
  entitlement or a small privileged helper for just that class), not by
  reverting the whole daemon to root.
- Preserve the self-install command's idempotency, rollback, and ownership
  metadata. The Windows implementation follows the same principles even
  though SCM, Event Log, and pipe DACL details have no macOS counterpart.
- A reboot-level `RunAtLoad` check (does the daemon actually come up after a
  real reboot, not just `launchctl bootstrap`) remains untested.
