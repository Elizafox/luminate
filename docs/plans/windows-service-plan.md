<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Running `luminated` as a Windows service

Status: **phases 1 and 2 complete; phase 3 active**. The machine-path
prerequisite, explicit pipe postures, typed pipe-access configuration, phase
1.1 runtime inversion, shutdown injection, cancellation-aware startup, daemon
lifecycle reporting, the Windows SCM lifecycle adapter, the self-install
subcommand, and the Event Log registration and logging path are implemented;
see 1.0–1.6 for the detailed, dated changelog. The automated pipe-DACL
enforcement test
(1.0/1.7), the adversarial pre-created-pipe daemon-refusal test, and the full
elevated installer test suite (fresh install, idempotent reinstall,
partial-failure rollback, pre-existing-group/Event-Log-source preservation,
guarded purge, a quoted path containing spaces, production-path rejection,
and uninstall while running) landed and passed on the Windows VM on
2026-07-24, alongside a real quoted-path comparison bug that suite surfaced
and fixed (see 1.7). An integration test driving `daemon::run` through
cancellation at each concrete startup milestone landed on 2026-07-24, runs on
every platform (not just Windows), and passed as part of the full workspace
suite (see 1.7).

The manual Windows VM checklist in 1.7 (restart during a real OS reboot,
forced-kill failure-action recovery, ProgramData/Program Files ACL
inspection, log rotation, a client connecting from an ordinary user session
after reboot, and an EventID-by-EventID Event Viewer review) ran on
2026-07-26. The ACL inspection surfaced a previously unimplemented gap:
`%ProgramData%\Luminate` had never had its own ACL set, so it and everything
under it (including the log directory) inherited `%ProgramData%`'s default,
which grants ordinary local users limited write access. `service install` now
creates and hardens that root (`protect_machine_data_root`, in
`security_descriptor.rs`) to `LocalSystem` and `Administrators` only,
inheritably, so every subdirectory — including ones like `logs` that the
daemon creates itself at runtime — picks up the restricted posture rather
than the `%ProgramData%` default. Covered by a unit test on the fixed SDDL
string and by a new elevated installer test,
`service_install_hardens_the_machine_data_root_against_ordinary_users`. Fixed
and verified on the Windows VM on 2026-07-26; the remaining checklist items
were exercised manually and passed. Phase 2 (power and device events) is
implemented; see "Phase 2 — power and device events" below.

This plan builds on the cross-platform work that got the workspace _compiling
and testing_ on Windows but did not address how the daemon should be launched
or supervised there. This is that discussion.

## Current checklist

- [ ] Exercise the service device-arrival-to-rescan path with supported physical
      hardware and confirm one debounced `RescanReason::DeviceChange` per event.
- [ ] Build the WiX v4 MSI around the existing service installer, including
      Program Files payload staging, install/start options, safe major upgrades,
      and non-purging uninstall behaviour.
- [ ] Add Windows MSI build, install, upgrade, and removal coverage to CI.

## Context

`luminated` on Windows currently runs the same way it does in a developer
shell on Linux: a foreground process, launched by hand, logging to stdout,
listening on a named pipe. That works for tests and works for nobody else.

The argument for the Service Control Manager is only incidentally about install
tidiness. SCM is the only mechanism that supplies three things the daemon
already consumes on Linux, none of which have a working Windows source today:

1. **Shutdown notification.** `luminate_platform::windows::process_signals`
   builds `ShutdownSignals` from `ctrl_c`/`ctrl_break`/`ctrl_close`/
   `ctrl_shutdown`. Those are _console_ control events. A service has no
   console, so under any real service host those four handlers can never fire
   and the daemon would only ever die by being killed, skipping the drain in
   `cleanup_connection_tasks` (`daemon/listener.rs:358`) that allows in-flight
   successful mutations to finish persisting. `SERVICE_CONTROL_STOP` and
   `SERVICE_CONTROL_PRESHUTDOWN` are the real signals.

2. **Suspend/resume.** `power::start_sources`
   (`luminate-platform/src/power/mod.rs:71`) has a logind source and a uevent
   source on Linux, and on every other platform falls through to
   `start_platform_sources`' "no suspend/resume source for this platform"
   log. `SERVICE_CONTROL_POWEREVENT` is the Windows analogue, and it is only
   deliverable to a service or to a process with a window and a message pump.
   We are not adding a message pump to a daemon.

3. **Device hotplug.** `SERVICE_CONTROL_DEVICEEVENT`, via
   `RegisterDeviceNotification` with `DEVICE_NOTIFY_SERVICE_HANDLE`, is the
   udev-uevent analogue. Today `RescanSignal::recv` on Windows never resolves
   and the control protocol's `Rescan` request is the only rescan path.

One mechanism, three gaps closed. Session-0 execution also means the daemon
runs before any user logs in, which is the point of a lighting daemon that is
supposed to have the machine lit when you sit down at it.

## Non-goals

- A GUI, tray applet, or per-user agent. The daemon stays headless.
- ETW/TraceLogging. Deferred; see "Deferred" below.
- Windows device _plugins_. No device plugin exists in-tree yet. This plan
  records the constraints a future one will hit but implements none of it.

## Decisions

Settled in discussion, recorded here so they survive context loss.

### Dependencies

Three new crates. The first two are Windows-only
(`[target.'cfg(windows)'.dependencies]`):

- **`windows-service` 0.8.1** — GPL-3.0-or-later, matching the project's own
  licence. MSRV 1.71.0, comfortably under the workspace's 1.88. Maintained by
  Mullvad. Notably it depends on **`windows-sys` 0.61**, the exact pin
  `luminate-platform` already uses, so it adds no second Win32 binding tree.
  It wraps `StartServiceCtrlDispatcherW`, `RegisterServiceCtrlHandlerExW`,
  `SetServiceStatus`, and the `CreateService`/`ChangeServiceConfig2` side, all
  of which we would otherwise hand-roll in `unsafe`. Also pulls `bitflags` and
  `widestring`.

- **`tracing-layer-win-eventlog` 1.0.1** — MIT. 213 lines total across two
  files, three `unsafe` blocks around `RegisterEventSourceW`/`ReportEventW`/
  `DeregisterEventSource`, plus an `unsafe impl Send + Sync` on the source
  handle. That assertion is correct (`ReportEvent` is documented thread-safe
  on a shared handle) but it is a soundness claim we would be adopting, so the
  crate gets read in full before it lands; at this size that is cheap.

  Two known costs. It pulls `windows` 0.61 and `windows-result` 0.3, which is
  a _second_ binding tree alongside `windows-sys`; they coexist fine (same
  metadata, different codegen) but it is real compile time. And it is a
  single-maintainer crate hosted on Codeberg with one release.

  Mitigation and requirement: the dependency is wrapped
  behind our own type in `luminate_platform::windows::event_log` and is
  referenced from exactly one file. Replacing it with roughly 80 lines against
  the `windows-sys` we already depend on is then a contained change rather
  than a refactor, should the maintenance story sour.

- **`tracing-appender`** for the rotating file sink. Time-based rotation
  (hourly/daily/never) plus `max_log_files` for retention; **no size-based
  rotation**, which is accepted; time-based is what we want. Unlike the other
  two this is not Windows-only in principle, but phase 1 wires it only into the
  service path.

`luminate-platform` also gains the `windows-sys` feature
`Win32_NetworkManagement_NetManagement`, for the client-group management
described below. No new crate: the dependency is already present, only the
feature list grows.

The resulting dependency graph needs a `cargo audit` pass before landing, per
`CONTRIBUTING.md`; that covers `widestring` and the other transitive additions
as well as the three direct dependencies.

`windows-service` 0.8.1 does not expose every API this plan needs. Phase 1 uses
narrow `windows-sys` wrappers for `SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO`
and `SERVICE_CONFIG_PRESHUTDOWN_INFO`. Its `ServiceControl` enum also has no
`SERVICE_CONTROL_DEVICEEVENT` variant, so phase 2's device-event arm is a raw
`u32` match on `control_handler`'s own `control` parameter rather than routed
through that enum: the same narrow `RegisterServiceCtrlHandlerExW` wrapper
phase 1 already owns (see below), extended rather than replaced. Do not
silently treat an unknown control as a device event.

`windows-service` 0.8.1 releases its registered handler closure after the
first STOP, SHUTDOWN, or PRESHUTDOWN callback. That lifetime is too short for
this plan's repeated-terminal-control contract: a concurrent later callback
could observe a freed context. The lifecycle implementation therefore uses
the crate for dispatcher support but owns a narrow
`RegisterServiceCtrlHandlerExW`/`SetServiceStatus` wrapper. Its callback state
is intentionally retained until process exit because SCM provides no handler
unregistration API. The wrapper is the single documented `unsafe` boundary for
this lifecycle path and can also support phase 2's currently unexposed device
control.

### Service account: LocalSystem, deliberately de-privileged

A virtual service account (`NT SERVICE\luminated`) is the tidy analogue of the
packaged unprivileged Linux `luminated` user, and it is the wrong fit here.
Device interface DACLs are set by each driver's INF; the common defaults grant
`SYSTEM` and `Administrators`, sometimes interactive users, and essentially
never an arbitrary service SID. Every new device class would become a
DACL-grant support burden on hardware we do not own.

Instead: run as **LocalSystem**, and use the two SCM levers that recover most
of the least-privilege benefit.

- `SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO` — declare the minimal privilege
  list and SCM strips every privilege _not_ listed from the process token.
  LocalSystem holding only `SeChangeNotifyPrivilege` is a very different
  proposition from default LocalSystem: measured on the VM, that is **1
  privilege instead of 28**, dropping `SeTcbPrivilege`, `SeDebugPrivilege`,
  `SeLoadDriverPrivilege`, and `SeTakeOwnershipPrivilege` among others, with
  HID device access still working. See "Measured on the Windows VM" below.

  Note what this does _not_ do: the stripped token still carries
  `BUILTIN\Administrators` and the System mandatory label. `RequiredPrivileges`
  trims privileges, not group membership, so it is real defence in depth but it
  is not a sandbox. Restricting the groups would mean
  `SERVICE_SID_TYPE_RESTRICTED`, which is ruled out just below.

- `SERVICE_CONFIG_SERVICE_SID_INFO` with **`SERVICE_SID_TYPE_UNRESTRICTED`** —
  the process token gains an `NT SERVICE\luminated` SID. The named-pipe DACL
  grants its server access to that SID rather than to LocalSystem generally,
  and clients require the SID when authenticating a service-mode server.
  Identity benefit without the access loss.

Do not use `SERVICE_SID_TYPE_RESTRICTED`. A write-restricted token will
fight device access, and debugging that is not a good use of anyone's evening.

The account is a registration parameter, not a compile-time constant, so this
can be revisited without touching the daemon.

### Pipe access control: a client group

This is a prerequisite, not a refinement. Moving to LocalSystem breaks
client connectivity outright unless the named-pipe DACL changes with it.

`owner_only_sddl` (`windows/security_descriptor.rs:71`) emits a single ACE:

```text
D:P(A;;FA;;;{owner_sid})
```

Full access, the daemon's own SID, nobody else, and `windows/transport.rs`
uses it for the pipes. That works today only because the daemon runs as the
same user as its clients. Under LocalSystem the pipes would grant access to
`S-1-5-18` alone and every interactive-user client would get `ACCESS_DENIED`:
a daemon that starts healthy and is reachable by nothing.

The module comment at `unix/transport.rs:6` says this has "no Windows
analogue", since Windows has no equivalent of "the socket's group", and calls
the Windows posture "strictly more restrictive, never less". That reasoning
held while the daemon ran as the client's own user. It does not survive the
move to a service, and the premise is wrong anyway: **a Windows local security
group is the equivalent of the socket's group.** That comment needs updating
as part of this work.

#### The model, mirroring Linux

The packaged Linux daemon already solves this same problem, with a dedicated
`luminated` account and ordinary users as clients:

- socket mode `0660`, owned by the daemon user, group `luminate`
  (`docs/development/packaging.md:33`)
- parent runtime directory `0700`, so the grant sits on the socket itself
- membership grants _connect_ access only, explicitly not hardware access
  (`packaging.md:57`)
- per-request authorization then happens in the policy layer

Windows takes the same shape. The pipe DACL becomes three ACEs:

| Principal               | Access                      |
| ----------------------- | --------------------------- |
| `NT SERVICE\luminated`  | Full                        |
| `Administrators` (`BA`) | Full                        |
| The client group        | Restricted mask — see below |

An empty group on a fresh install means only administrators can connect, which
mirrors Linux, where installing does not add anyone to `luminate`. Safe by
construction rather than by remembering to lock it down.

#### The client access mask, and why `FA`/`GRGW` are wrong

`FILE_GENERIC_WRITE` on a named pipe includes **`FILE_CREATE_PIPE_INSTANCE`**.
A principal holding that bit can create _further instances of the same pipe
name_, squat the endpoint, and impersonate the daemon to other clients. SDDL's
convenient `GRGW` shorthand runs through the generic mapping and picks the bit
up, so the natural-looking ACE is the unsafe one.

The client ACE therefore needs an **explicit hex access mask** with the
create-instance bit cleared, not a generic-rights shorthand. The constant is
**`0x0012019B`**, measured on the Windows VM (see "Measured on the Windows VM"
below) rather than copied from memory:

```text
FILE_GENERIC_READ                     0x00120089
FILE_GENERIC_WRITE                    0x00120116
FILE_GENERIC_READ | FILE_GENERIC_WRITE 0x0012019F   <- contains 0x4, unsafe
FILE_CREATE_PIPE_INSTANCE             0x00000004
client mask, create-instance cleared  0x0012019B   <- use this
```

Reassuringly, this is not a novel invention: .NET's own
`PipeAccessRights.ReadWrite` is `0x0002019B`, which deliberately omits
`CreateNewInstance` for the same reason. Adding `SYNCHRONIZE` (`0x00100000`)
gives the same `0x0012019B` by an independent route.

The resulting SDDL, with `<service>` the canonical service SID and `<group>`
the client group's canonical SID:

```text
D:P(A;;FA;;;<service>)(A;;FA;;;BA)(A;;0x12019b;;;<group>)
```

This is also why DACL construction stays centralized in
`security_descriptor.rs` instead of being open-coded at each call site.

Administrators deliberately retain full access. An administrator can therefore
create another pipe instance; the no-squat property asserted here is for the
ordinary client principal, not a claim to defend the service from a hostile
administrator.

#### Configuration and installation

The client principal is configurable, defaulting to the local group. The
`[authorization]` config section already exists and already selects policy
providers, so it is the natural home for a typed pipe-access setting. A
single-user desktop can select `Interactive` (`S-1-5-4`) and skip the group
entirely. Configuration never accepts raw SDDL or interpolates an unvalidated
SID string. Account names are resolved through Windows account APIs and
converted back to canonical SID text before DACL construction.

The intended shape is:

```toml
[authorization.pipe_access]
principal = "local-group" # or "interactive"
group = "Luminate Clients"
```

The platform transport receives an explicit posture, along the lines of
`PipeAccess::OwnerOnly` or `PipeAccess::Service { service_sid, client_sid }`.
It does not infer service mode from the current process user. This keeps
console mode owner-only and makes the security choice visible at the bind call.

`Interactive` is deliberately not the _default_. The shipped
`SocketAccessPolicy` currently always allows (`authorization.rs:16`, `:281`) —
no policy denies anything yet — so a broad connect grant has nothing behind
it. `Authenticated Users` is broader still and is not offered.

The group is named **`Luminate Clients`**, spelled for the Windows UI rather
than matching the Linux `luminate` group character for character. The two are
never compared programmatically — nothing resolves one platform's group name on
the other — so a name that reads naturally in `lusrmgr.msc` beats a literal
match, and the documentation states both.

`service install` creates the local group via `NetLocalGroupAdd`
(`windows-sys` feature `Win32_NetworkManagement_NetManagement`) and offers
`--add-user DOMAIN\\User` plus the explicit convenience
`--add-current-user`, both implemented with `NetLocalGroupAddMembers`. Group
membership is baked into the logon token, so it takes effect after a sign-out
or reboot; the installer says so plainly. This is Windows, where a reboot after
installing a system service is expected rather than remarkable.

Uninstall does not remove the client group by default. It may predate
Luminate or contain administrator-managed membership. An explicit
`--purge-client-group` removes it only when it is empty and installation
metadata establishes that Luminate created it.

#### Server identity — the DACL is not enough

Clearing `FILE_CREATE_PIPE_INSTANCE` prevents an ordinary client from adding
an instance after the legitimate server owns the name. It cannot stop that
client from creating the well-known pipe _before_ the daemon starts. The
existing `FILE_FLAG_FIRST_PIPE_INSTANCE` makes `luminated` fail safely in that
case, but without a client-side check a client could still connect to the
impostor while the real daemon is absent.

Every Windows client therefore authenticates the connected server before the
protocol handshake. It obtains the server PID with
`GetNamedPipeServerProcessId`, opens the process token, and accepts either:

- LocalSystem with the `NT SERVICE\luminated` SID enabled, for service mode; or
- the client's own user SID, preserving owner-only console mode.

Failure to obtain or validate the server identity fails the connection closed.
The check applies identically to the primary and event pipes. Tests include an
adversarial process pre-creating each well-known pipe and prove that clients
refuse it; the legitimate daemon's first-instance failure is tested too.

#### Scope of the change

Only the primary and event pipes move. `secure_storage`'s owner-only posture
for directories and files is unchanged; that model is correct and stays
correct. The event pipe gets identical treatment to the primary; there is no
reason for them to diverge.

ACE ordering is canonical and deterministic. The existing exact-SDDL
verification remains an owner-only filesystem check; it does not verify named
pipes. If service pipe verification is added, it reads the descriptor from the
pipe handle with `GetSecurityInfo` and either compares a canonical rendering or
validates the ACEs semantically rather than pretending the filesystem helper
already covers it.

### Consequence: no client-to-daemon shm fast path on Windows

`Principal::is_same_user_as_daemon` (`authorization.rs:133`) compares the peer
SID against `daemon_own_sid()`. Under LocalSystem no interactive user ever
equals `S-1-5-18`, so it returns `false` for every real client, permanently.

That predicate is the single gate on the client-to-daemon iceoryx2
frame-streaming fast path, uid-scoped because iceoryx2 0.9.3 has no
cross-principal access control of its own. So the fast path becomes unavailable
on Windows: clients fall back to
`Request::BeginFrameStream`, which is documented as a strict superset, so this
is a performance regression rather than a correctness bug.

Recorded as a **deliberate, accepted consequence** rather than discovered
later. The existing doc comment already anticipates the fix — "a future
increment adding real cross-principal support (POSIX ACL grants on Unix, an
equivalent on Windows, each with its own sign-off)" — and that increment now
has a concrete motivating case.

### Logging: Event Log for operators, file for detail

A service has no stdout; today's `tracing_subscriber::fmt()` in
`luminated/src/main.rs:39` writes into the void under SCM. Split by audience:

- **Windows Event Log**, source under the existing `Application` log, carrying
  a _curated_ set of operationally significant events at INFO, WARN, and ERROR
  covering service start and stop, config load failure, pipe bind failure, and
  plugin host crash. Bounded vocabulary by design; it is not a mirror of every
  tracing event at those levels.
- **Rotating file sink** under `%ProgramData%\Luminate\logs` via
  `tracing-appender`, carrying the full `tracing` output at whatever
  `RUST_LOG` selects. Daily rotation with **7 days** of retention in phase 1;
  this is the parity target for what journald sees on Linux. Retention becomes
  configurable only with a design that can report a malformed logging setting
  to an already-working sink; phase 1 avoids that bootstrap loop.
- **Console mode keeps today's stdout formatter unchanged**, so developer
  ergonomics do not regress.

Two supporting notes:

_No message-compiler dependency._ An event source needs an `EventMessageFile`
registry value pointing at a module with a `MESSAGETABLE` resource, or Event
Viewer renders every entry as "The description for Event ID … cannot be
found." We do not need to author one. Point `EventMessageFile` at the .NET
Framework's generic passthrough table, in-box on every supported Windows since
8:

```text
%SystemRoot%\Microsoft.NET\Framework64\v4.0.30319\EventLogMessages.dll
```

That DLL contains a `%1` passthrough template for each EventID from 0 through
65535. The plan's small numeric IDs therefore render the formatted tracing
message without a Luminate-specific resource. That reduces the whole problem to
one registry value written at install time: no `.mc` file, no `mc.exe`, and no
Windows SDK in CI. Phase 1 supports x86-64 Windows and uses the `Framework64`
path above; adding another Windows architecture requires selecting and testing
the appropriate in-box message module rather than blindly reusing that path.

_Source under `Application`, not a custom log — confirmed for v1._ A custom
classic log needs more registry surface and interacts worse with existing log
collection. If dedicated filtering is wanted later, an ETW channel is the
better answer, and that is already deferred.

_How the source gets registered: direct registry API, in-process._ Registering
an event source means creating
`HKLM\SYSTEM\CurrentControlSet\Services\EventLog\Application\luminated` and
setting `EventMessageFile` (to the path above) and `TypesSupported`. Three
candidate mechanisms, and the choice matters more than it looks:

- **Shell out to PowerShell `New-EventLog`.** Rejected. It drags in execution
  policy, quoting, and an external process spawned from an elevated context,
  and an exit code cannot cleanly distinguish "already registered" from
  "access denied", which are the two outcomes we most need to tell apart.
- **A separate installation wrapper script.** Rejected for the same reasons the
  subcommand won over a script generally: two privileged code paths, a second
  artefact to keep in sync, and no uninstall symmetry.
- **Direct `RegCreateKeyExW`/`RegSetValueExW` via the `windows-sys` we already
  depend on.** Chosen. Roughly 40 lines, three narrowly scoped `unsafe` calls,
  living in `luminate_platform::windows::event_log` next to the layer wrapper
  it supports. `EventMessageFile` is written as `REG_EXPAND_SZ`, its target is
  verified to exist, and errors carry the actual `LSTATUS`. Deregistration on
  uninstall occurs only when the existing values still match Luminate's
  installation metadata; an unrelated or administrator-modified source is not
  deleted. This remains one elevated code path shared with `CreateService`.

No fourth dependency: the `eventlog` crate exposes an equivalent `register()`,
but adding a crate for one function we can write in forty lines against an
existing dependency is not a trade worth making.

_Level mapping is coarse and that is fine._ The Event Log has only
`EVENTLOG_ERROR_TYPE`, `EVENTLOG_WARNING_TYPE`, and
`EVENTLOG_INFORMATION_TYPE`. DEBUG and TRACE both collapse to INFORMATION.
The layer receives only explicit operator events through a dedicated tracing
target or typed helper, so admitting INFO for normal lifecycle events does not
flood the log with ordinary informational tracing.

### Registration: a self-install subcommand

`luminated.exe service install|uninstall|start|stop|status`, calling
`CreateService`/`DeleteService`/`ControlService` through `windows-service`.
The `service status`, `service start`, and `service stop` commands, together
with strict service-command argument dispatch, were implemented on 2026-07-22.
Basic idempotent SCM registration was implemented on 2026-07-22, including
canonical executable paths, the Program Files production-path guard, automatic
start, LocalSystem, display metadata, and refusal to replace a same-named
service pointing elsewhere. The explicit `--allow-development-path` escape
hatch exists only for throwaway development machines. Idempotent SCM
uninstallation, including a stop request before deletion, was implemented on
2026-07-22. Unrestricted service-SID configuration and the explicit 30-second
preshutdown timeout were implemented on 2026-07-22. Required-privilege
configuration retaining only `SeChangeNotifyPrivilege` was implemented on
2026-07-22. The failure-action policy was implemented on 2026-07-22.
Client-group creation and the `--add-user` and `--add-current-user` membership
options were implemented on 2026-07-22. Client-group ownership metadata and
the guarded `uninstall --purge-client-group` option were implemented on
2026-07-22. Event Log source ownership metadata, fresh-install rollback, and
guarded uninstall removal were implemented on 2026-07-22.

Chosen over a separate PowerShell script because it is the same elevated
context that already has to call `CreateService`: one privileged code path
rather than two, uninstall symmetry falls out for free, there is no execution
policy friction, and no second artefact to keep in sync. Phase 3's WiX MSI
shells to the same subcommand from a custom action instead of reimplementing
the logic. `New-EventLog` remains a fine developer convenience; it is not the
supported path.

Installation is idempotent and transactional. It creates or verifies each
object, updates only the service fields this installer owns, and on failure
rolls back only objects created by that invocation. Reinstall and upgrade do
not convert an unrelated service, group, or Event Log source into Luminate's
property. Uninstall stops the service before requesting deletion and accounts
for SCM deletion remaining pending until all service handles close.

Successful fresh installation records ownership of the group and Event Log
source in a Luminate installer key under `HKLM\Software\Luminate`; rollback
removes only markers written by that invocation. Matching names or registry
values alone are not treated as proof that Luminate created a pre-existing
object.

The registered executable path is fully qualified, canonicalized, and quoted
correctly when it contains spaces. Production installation expects the binary
to have already been placed under `%ProgramFiles%\Luminate`; a user-writable
path is rejected unless an explicit development override is supplied. Unknown
command-line arguments fail instead of accidentally falling through to daemon
startup.

Three distinct names, easily conflated:

- **Service name** — `luminated`. Machine-facing: it is the SCM key, it is what
  `sc` and `Start-Service` take, and it is baked into the `NT SERVICE\luminated`
  SID that the pipe DACL names. Matches the binary and the Linux unit, so the
  machine-facing identifier is identical on both platforms.
- **Display name** — **`Luminate Lighting`**. Human-facing, shown in
  `services.msc`, and must be unique on the machine. It follows the convention
  Windows' own services use — "Print Spooler", "Task Scheduler", "Plug and
  Play" — rather than the vendor habit of appending "Service", which adds no
  information in a list where everything is one. Purely cosmetic: nothing
  persisted or security-relevant references it, so unlike the service name it
  stays cheap to change.
- **Description** — the longer text in the details pane, which display names
  are often stretched to cover. "Manages lighting devices, restores saved
  lighting state at start-up, and serves client applications." Set at
  registration alongside the rest; the exact wording matters only insofar as it
  is descriptive.

Settings applied at registration:

- **Start type** — `SERVICE_AUTO_START`. Lighting before login is a real use
  case and the point of session-0. Overridable with `--start-type`.
- **Start on install** — off by default; `--start` opts in. Matches the Linux
  packaging convention only in not starting the daemon immediately. Unlike the
  current Linux packages, the Windows service is enabled for the next boot by
  its default auto-start type. See the convergence note below.
- **Account** — LocalSystem, per the account decision above.
- **Failure actions** — restart after 5 seconds, 30 seconds, and 5 minutes,
  then stop restarting. Reset the failure count after 24 hours without a
  failure. Apply the actions to crashes and service-specific non-zero exits,
  but not clean stops. The SCM action array ends with `none` because Windows
  repeats its final action for subsequent failures.
- **Service SID** — `SERVICE_SID_TYPE_UNRESTRICTED`, giving the pipe DACLs a
  principal to name.
- **Required privileges** — `SeChangeNotifyPrivilege` alone, measured as
  sufficient for HID device access (see "Measured on the Windows VM").
- **Preshutdown timeout** — 30 seconds, set explicitly rather than relying on
  the system default.
- **Event Log source** — created here and removed on uninstall only when still
  owned by this installation. See the logging decision above.
- **Client group** — created here when absent and retained on ordinary
  uninstall. `--add-user DOMAIN\\User` and `--add-current-user` add membership;
  the installer reports that a sign-out or reboot is needed before it takes
  effect.

Starting immediately on install is a flag,
defaulting off. The Windows service is nevertheless registered for automatic
start at the next boot. That temporarily differs from Linux packaging, which
neither enables nor starts the unit; the intended direction is to make
auto-start the Linux default too. When Linux changes, decide separately whether
both installers should also start immediately rather than conflating enablement
with start-on-install.

### Windows machine layout

Service mode needs machine-wide paths resolved before phase 1 can work. It
must not consume the compiled Unix `/etc`, `/var/lib`, and `/usr/lib` defaults
as if they were meaningful Windows paths. The layout is:

```text
%ProgramFiles%\Luminate\
  luminated.exe
  plugins\

%ProgramData%\Luminate\
  luminated.toml
  state\
    state.json
  logs\
    luminated.YYYY-MM-DD.log
  iceoryx2\
```

Resolve Program Files and ProgramData at runtime with
`SHGetKnownFolderPath`, not an environment variable or a hard-coded `C:`
path. The `luminate_platform::default_path` surface owns that platform fact.
The default-path functions retain their existing `LUMINATE_DEFAULT_*`
build-time overrides in `luminate-platform`.
Unix retains the existing FHS layout. Windows resolves configuration and state
under ProgramData and plugins under Program Files. Both Windows plugin-directory
functions intentionally resolve to the single machine plugin tree, and daemon
configuration deduplicates it. This prerequisite was implemented on 2026-07-22.

The executable and plugin tree is administrator-writable only. The ProgramData
root is protected from ordinary users, while each subtree keeps the posture its
contents need:

- configuration is administrator-writable and service-readable;
- logs are service-writable and administrator-readable;
- state and iceoryx2 retain `secure_storage`'s current-process-owner-only
  posture, created by the daemon as LocalSystem rather than pre-created with a
  broader installer ACL; and
- the client group receives no filesystem access anywhere in the tree.

`service install` creates and hardens only the ProgramData root itself
(`protect_machine_data_root`, `security_descriptor.rs`) to `LocalSystem` and
`Administrators` full access, inheritably, with `PROTECTED_DACL_SECURITY_INFORMATION`
so no ACE from `%ProgramData%`'s own default — which otherwise grants ordinary
users limited write access — survives underneath it. It does not pre-create
owner-only runtime data or invent a config file; there are no other
installer-owned subdirectories today. An absent implicit config still uses
built-in defaults. Administrators can recover owner-only state using their
existing system authority when necessary; routine operation goes through the
service. Implemented and verified on the Windows VM on 2026-07-26, after the
1.7 manual ACL-inspection checklist item surfaced that this hardening had
never been implemented. The directory previously existed only by inheriting
`%ProgramData%`'s own default ACL.

Phase 2.1 (power events) is implemented; see 2.1 below for the final shape.
`SuspendLease` on Windows is confirmed to be a true no-op, as originally
planned. A `PowerRequestSystemRequired` power request was considered for the
quiesce window and rejected, since by the time
`PBT_APMSUSPEND` reaches the service the suspend is already committed and the
request cannot delay it (see 2.1 for the mechanics). The Windows power-event
source is injected through `RunContext`/`accept_loop` alongside the existing
shutdown and lifecycle plumbing, rather than by adding an injection point to
`power::start_sources` itself.

Built, formatted, linted, and tested clean on the Windows VM on 2026-07-26,
including the full elevated installer suite (1.7) run again after this
change with no regressions. **Not verified: actual `SERVICE_CONTROL_POWEREVENT`
delivery.** `powercfg /a` initially reported that this VM's firmware supported
no sleep state at all. Enabling `<pm><suspend-to-mem enabled='yes'/></pm>` in
the libvirt domain XML (and removing the now-redundant `firmware='efi'`
auto-select attribute it had been paired with) got the ACPI tables to
advertise S3, but `powercfg /a` then reported S3 blocked one layer down: "the
hypervisor does not support this standby state." QEMU/KVM's S3 support is a
known rough edge — `ICH9-LPC.disable_s3=0` (what the libvirt flag sets) isn't
sufficient on its own in a lot of configurations, and the remaining blocker
depends on machine type and attached device models in ways not worth chasing
further here. Real suspend/resume delivery therefore remains unverified;
`interpret`'s event-type mapping is covered by unit tests instead, and real
hardware or a VM host with working S3 passthrough would be needed to close
this gap.

Named pipes are not files and no longer derive production names from pretend
filesystem paths. Service mode uses:

```text
\\.\pipe\luminated
\\.\pipe\luminated.events
```

If pipe names become configurable, use validated name fields rather than full
`\\.\pipe\...` strings. Existing path-derived names may remain for private
tests and console-mode compatibility while the public address representation
is settled.

Service-mode daemon ↔ plugin-host iceoryx2 data uses the ProgramData directory
above, protected for the service SID. Console mode retains the current-user
`%LOCALAPPDATA%\luminate\iceoryx2` root. This does not re-enable the
client-to-daemon fast path; it only preserves the same-principal daemon ↔ host
path.

### Scope

Phase 1 is service lifecycle plus the pipe security, machine paths, logging,
and installation work a usable lifecycle depends on. Phase 2 adds power and
device events. Phase 3 packages install/uninstall as a WiX MSI, promoted ahead
of the deferred items below because Add/Remove Programs and upgrade sequencing
matter more, once a real service exists, than the phase 2 event plumbing does.
Splitting phases 1 and 2 gets a real service running on the VM before the
subtler event plumbing starts.

## Phase 1 — service lifecycle

### 1.0 Pipe access control — prerequisite

Numbered zero because it gates everything else: without it the service starts
and no client can reach it. Implements the client-group model decided above.

- Generalize `security_descriptor.rs` from one fixed owner-only SDDL string to
  two postures: the existing owner-only one, still used verbatim by
  `secure_storage`, and a service posture composing the service-SID,
  Administrators, and client-group ACEs. Keep ACE ordering deterministic.
- The client access mask `0x0012019B`, with `FILE_CREATE_PIPE_INSTANCE`
  cleared, was added earlier (see the Dependencies section above). The
  automated test proving a client-group principal can connect but cannot
  create a second instance of the pipe name, and that a principal outside the
  group is refused the connect outright, was implemented and run elevated on
  the Windows VM on 2026-07-24, porting the "Measured on the Windows VM" probe
  into the suite rather than resting on that one-off run. It creates two
  throwaway local accounts and a throwaway local group
  (`windows::local_test_account`, test-only), joins one account to the group
  before logon (group membership is fixed at logon time), and impersonates
  each account on a dedicated thread to exercise `CreateFile` and pipe-instance
  creation under its own token. `#[ignore]`d like the other elevated,
  VM-only checks in 1.7, since creating local accounts needs administrator
  privileges.
- Typed client-principal configuration, defaulting to the local group, was
  implemented on 2026-07-22. Canonical account-SID resolution and the explicit
  pipe posture for both service listeners were implemented on 2026-07-22;
  console mode remains owner-only.
- Idempotent local-group creation and account-membership primitives, together
  with `--add-user` and `--add-current-user`, were implemented on 2026-07-22.
  Installer ownership metadata and guarded explicit purge were implemented on
  2026-07-22. Purge requires the `InstallerOwnedClientGroup` registry value to
  name `Luminate Clients` exactly and refuses a group that has members.
- Connected-server authentication for both Windows client paths was
  implemented on 2026-07-22 at their shared transport boundary. Same-user
  console servers remain valid; service servers must run as LocalSystem with
  the `NT SERVICE\luminated` SID enabled. The focused VM regression rejects a
  reachable LocalSystem impostor without that service SID.
- The module comment at `unix/transport.rs:6` now records the Windows local
  group analogue rather than claiming none exists.

Doing this first also means the console-mode daemon keeps working throughout:
running as the invoking user with the owner-only posture stays available and
stays the default outside service mode.

### 1.1 Entry point and runtime inversion

Implemented on 2026-07-22. `main` is synchronous and constructs a Tokio runtime
only for the asynchronous policy-host and daemon paths, leaving a synchronous
dispatch point for the service subcommand and SCM entry point.

This is the one structural consequence of the whole plan, so it goes first.

`ServiceMain` is invoked on a thread the SCM creates, and the control handler
callback runs on another SCM thread. The handler must return promptly;
`ServiceMain` deliberately remains blocked on the daemon runtime until it has
reported `SERVICE_STOPPED`. The tokio runtime is therefore constructed
inside `ServiceMain`, not before it, so `main` can no longer be
`#[tokio::main]` (`luminated/src/main.rs:24`).

Proposed shape:

```text
fn main() -> Result<()>
├── plugin_host::invocation_from_args()  → sync, unchanged
├── policy_host::invocation_from_args()  → build runtime, block_on
├── service subcommand (install/…)       → sync, Windows only
├── #[cfg(windows)] try SCM dispatch
│     ERROR_FAILED_SERVICE_CONTROLLER_CONNECT → fall through to console
│     otherwise blocks until the service stops
└── console mode → build runtime, block_on(daemon::run(…))
```

Mode selection needs no new flag. `StartServiceCtrlDispatcherW` fails with
`ERROR_FAILED_SERVICE_CONTROLLER_CONNECT` precisely when the process was not
launched by the SCM, which is the "run in the foreground" case. One
binary, no `--service` flag to get wrong. This also slots into the existing
arg-dispatch pattern `main.rs` already uses for the plugin and policy hosts,
rather than introducing a new one.

Confirmed working on the VM, with one trap: `windows_service::Error` renders as
`"IO error in winapi call"`, which tells you nothing. Match on
`Error::Winapi(io).raw_os_error() == Some(1063)`, never on the message.

### 1.2 Where the platform code lives

A new `luminate_platform::windows::service` module, holding the SCM entry
point, the status reporter, and the control-handler-to-runtime bridge.

This is deliberately not generalized. It is tempting to define a cross-platform
`ServiceLifecycle` trait now, mirroring systemd's `sd_notify`. There is no
second implementation — the Linux side does not use `Type=notify` today — and
inventing a two-platform abstraction with one implementor would cross the
"model special cases through common abstractions where that remains truthful"
line in the untruthful direction. The seam gets hoisted when `sd_notify`
support arrives and there is a second implementor to shape it. See "Readiness
reporting" below, which is designed to make that hoist easy.

### 1.3 Cancellation and shutdown source injection

Implemented on 2026-07-22. The accept loop's shutdown source is injected
through a daemon-local `RunContext`; console mode supplies the existing
platform process signals, while SCM mode and tests supply a deterministic
channel. Startup observes externally requested shutdown between synchronous
phases and races it against long asynchronous phases. The SCM control handler
reports `STOP_PENDING` and enqueues exactly one request after the first STOP or
PRESHUTDOWN control.

Previously, `accept_loop` installed its own signal handlers and selected on
them directly. The first slice moved that choice into `RunContext`; under SCM,
the injected request will arrive from the handler thread instead.

Rather than `cfg`-splitting the select arm, make cancellation injectable.
A small daemon-local `RunContext` carries cancellation, readiness, and progress
reporting; this is a composition parameter, not a cross-platform service
abstraction. Console mode supplies the existing `ShutdownSignals` and no-op
readiness/progress hooks. Service mode supplies a receiver fed by the control
handler. The tests gain a deterministic way to drive both startup and
steady-state shutdown.

The control handler is registered while the service is `START_PENDING`, and
initialization checks cancellation between every phase while long asynchronous
phases select against it directly. A synchronous operation already in progress
may finish before cancellation is observed. Windows nevertheless ignores the
accepted-controls mask for pending states: SCM reports the service as
`NOT_STOPPABLE` and ordinarily refuses a STOP request until it reaches
`RUNNING`. Do not report `RUNNING` early merely to make startup stoppable; that
would turn readiness into a lie. The injected startup-cancellation seam remains
useful for controls that have reached the handler, deterministic tests, and a
future controller with stronger cancellation semantics.

Once running, controls accepted are `SERVICE_ACCEPT_STOP` and
**`SERVICE_ACCEPT_PRESHUTDOWN`**. The first terminal control reports
`STOP_PENDING` and initiates shutdown; the transition is idempotent if more
than one source observes it. `SERVICE_CONTROL_INTERROGATE` always returns the
current status. Preshutdown is not a preliminary notification: it tells the
daemon to stop immediately. SCM is configured with a 30-second preshutdown
timeout.

There is no separate persisted-state flush. Successful mutations persist as
they complete; graceful shutdown drains in-flight connection work so those
mutations can finish. The current cleanup has two consecutive ten-second
bounds, so its STOP_PENDING wait hint covers slightly more than 20 seconds and
advances checkpoints at real cleanup milestones.

### 1.4 Readiness reporting

Implemented on 2026-07-22. A typed daemon-local lifecycle reporter emits named
startup milestones and reports readiness after both accept workers are live.
Console mode discards these events, while tests and the Windows SCM adapter
consume them deterministically. SCM advances `START_PENDING` checkpoints only
for real milestones, reports `RUNNING` on readiness, and reports either a clean
Win32 exit or a service-specific failure at `SERVICE_STOPPED`.

SCM requires `SERVICE_START_PENDING` promptly and `SERVICE_RUNNING` within the
start timeout. `daemon::run` does config loading, socket binding, plugin
discovery, persisted-state restore, and startup reconciliation before it is
serving, plausibly slower than the default 30s on a machine with many
plugins.

So `daemon::run` gains **readiness and progress signals** through the run
context. Under SCM, the service reports `START_PENDING`, accepts STOP, and
increments its checkpoint only at real milestones such as config load, plugin
discovery, listener binding, persisted-state restore, policy-host start, and
startup reconciliation. It does not run a blind checkpoint timer that could
claim progress while startup is stuck. It reports `RUNNING` only after the
accept workers are live. In console mode the hooks are no-ops.

This is the same primitive `sd_notify`'s `READY=1` needs, which is the main
reason the abstraction seam in 1.2 can stay Windows-only for now without
painting us into a corner.

### 1.5 Logging implementation

The idempotent Event Log source registration and verification primitive was
implemented on 2026-07-22. The typed operator-event emission boundary and its
service start, ready, stop-request, preshutdown-request, failure, and stopped
lifecycle calls were wired on 2026-07-22. Service-only configuration-load,
persisted-state-load, and listener-bind failure calls were wired on 2026-07-22.
Persisted-state-save and unexpected plugin-host termination calls were wired
on 2026-07-22. The service-only daily file sink with seven-file retention and
the curated Event Log layer were implemented on 2026-07-22. Installer
ownership tracking, fresh-install rollback, and guarded removal of an
unchanged Event Log source were implemented on 2026-07-22.

- `luminate_platform::windows::event_log` — the wrapper type over
  `tracing-layer-win-eventlog`, and the only file that names that crate.
- `luminated`'s logging setup branches on launch mode: console and service file
  tracing both escape C0, DEL, and C1 controls at the field boundary;
  service mode composes the file layer (full `RUST_LOG` output) with the Event
  Log layer filtered to a dedicated operator-event target. Console ANSI styling
  remains formatter-owned and cannot be introduced by field values. A typed
  `OperatorEvent` vocabulary is the only route to that target, so normal INFO,
  WARN, and ERROR tracing continues to the file without automatically becoming
  an operator event.
- Service logging is initialized before config loading with the fixed phase-1
  retention policy. The boundary around `daemon::run` logs the full error chain
  for config, bind, plugin, policy, persistence, and runtime failures, then
  reports `SERVICE_STOPPED` with an appropriate Win32 or service-specific exit
  code. Returning `Result` from `main` is not treated as service logging.
- If `tracing-appender` uses its non-blocking writer, the service entry point
  retains its worker guard through `SERVICE_STOPPED` so final events are
  flushed. Failure to open the file sink is surfaced through the Event Log when
  available; failure of both sinks remains a startup error reported to SCM.
- **Startup ceremony is verify, not create.** Registering the event source is a
  write under `HKLM`. Doing it at daemon startup would succeed under
  LocalSystem, which is what makes it tempting and also why it is wrong:
  privileged self-modification on every boot, failing confusingly in
  console mode as a normal user. At startup the daemon _checks_ whether the
  source is registered, and if it is not, logs a warning to the file sink and
  continues with file logging alone. Registration is an install-time concern;
  the daemon reports the uncertainty rather than escalating to fix it.

### 1.6 Event ID vocabulary — a compatibility surface

Implemented on 2026-07-22. The typed vocabulary and stable event catalogue are
defined in `operator_event.rs` and
`docs/development/architecture/windows-event-log.md`; phase 1.5 wires those
definitions to the dedicated Event Log tracing target.

`tracing-layer-win-eventlog` maps a `tracing` field `id = N` to the Windows
EventID, falling back to the level when absent. **EventIDs are
compatibility-sensitive**: administrators write Event Viewer filters and
monitoring rules against the `(source, EventID)` pair, so they are closer to
protocol constants than incidental values and are covered by the interface
rules in `AGENTS.md` and `CONTRIBUTING.md`. Event type/severity is a separate
field; changing INFO to WARN does not change an event's identity.

Phase 1 allocates ranges and documents them in a table under `docs/`:

| Range | Meaning                                             |
| ----- | --------------------------------------------------- |
| 1xx   | Service lifecycle (start, ready, stop, preshutdown) |
| 2xx   | Configuration and persisted state                   |
| 3xx   | Transport and listeners                             |
| 4xx   | Plugin host and supervision                         |
| 5xx   | Reserved for phase 2 (power, device events)         |

Numbers assigned within a range are additive-only thereafter; retire an ID
rather than repurposing it. All IDs remain in the generic message resource's
0–65535 range. Code represents the vocabulary as a typed enum or named
constants rather than scattering numeric `id` fields through ordinary tracing
calls.

### 1.7 Tests

Honest constraints first: SCM interaction cannot be unit tested, service
install needs elevation, and the Event Log needs a registered source. So:

- **Unit-testable, and worth it:** the shutdown-source injection (1.3) and the
  readiness signal (1.4) become deterministically drivable from tests on every
  platform, an improvement over the current signal-only path.
- **The pipe DACL is testable on Windows and must be tested**, not eyeballed:
  a client-group member connects, a non-member is refused, and no client can
  create a second instance of the pipe name. Implemented and run elevated on
  the Windows VM on 2026-07-24 (`windows::transport::tests::
service_pipe_dacl_enforces_client_group_membership`, `#[ignore]`d for
  elevation like the rest of this list). Separate adversarial processes
  pre-create the primary and event names; the daemon must refuse to start on
  them and clients must refuse to authenticate them — connected-server
  authentication itself is implemented and covered (see 1.0). The daemon-side
  half — pre-creating the well-known pipe name first and asserting the
  daemon's own bind fails closed on `FILE_FLAG_FIRST_PIPE_INSTANCE` — was
  implemented on 2026-07-24 (`windows::transport::tests::
bind_fails_when_the_pipe_name_is_already_claimed`). Unlike the DACL test
  above, this one needs no elevation or distinct security principal, since
  the refusal comes from the first-instance flag rather than from any DACL,
  so it runs unconditionally. These are the central security assertions of
  phase 1.
- **Lifecycle tests:** cancellation during each startup phase, readiness only
  after accept workers start, idempotent repeated terminal controls, error-to-
  `STOPPED` mapping, and honest progress/checkpoint sequencing.
  `daemon::startup_cancellation_tests::cancellation_stops_daemon_run_at_each_startup_milestone`,
  implemented on 2026-07-24, drives the real `run()` end to end for each of
  the six `StartupMilestone`s and asserts a shutdown request injected right
  after that milestone stops `run()` there, cleanly, with no later milestone
  or `Ready` reported. Four of the six milestones (`ConfigurationLoaded`,
  `PluginsLoaded`, `ListenersBound`, `PersistedStateRestored`) are reported and
  checkpointed with no `.await` between them, so an external test task has no
  scheduling opportunity to land a shutdown request at a specific one of
  them — on a current-thread runtime it would only ever observe the first;
  on a multi-thread runtime, landing it at a chosen later one would be a
  data race. The test therefore drives `run()` through a test-only
  pause hook (`TestMilestonePause`/`TestPause`, gated behind `#[cfg(test)]`)
  that parks `run()` immediately after it reports the target milestone and
  before that milestone's checkpoint, so the test can deterministically queue
  the shutdown request and release it. `load_config` reads its configuration
  and path overrides from process-wide environment variables, so — following
  the existing convention in
  `listener::load_config_honours_explicit_files_and_path_overrides` — each of
  the six milestones is driven in its own isolated child process rather than
  racing environment mutation against other tests in the same binary.
- **Installer tests on the elevated VM:** fresh install, idempotent reinstall,
  partial-failure rollback, preservation of a pre-existing group or Event Log
  source, guarded purge, a quoted path containing spaces, rejection of a
  user-writable production binary, and uninstall while running. Implemented
  and run elevated on the Windows VM on 2026-07-24
  (`crates/luminated/tests/windows_service_install.rs`, one `#[ignore]`d test
  per scenario above plus a non-elevated test for the path rejection, since
  that check runs before anything privileged is touched). These drive the
  real `luminated.exe service ...` subcommand as a subprocess rather than
  calling `luminate_platform::windows::service` in-process, because `install`
  always installs `env::current_exe()`; the whole suite is built and run from
  a directory whose path contains a space, so the fresh-install test's
  round-trip assertion on the registered binary path is the "quoted path"
  case, not a separate scenario. Each test resets the real `luminated`
  service, `Luminate Clients` group, and `luminated` Event Log source via
  `sc`/`net`/`reg` before it runs, so the suite is safe to run individually or
  as a sequential batch (`--test-threads=1`) but never in parallel with itself
  or other elevated work on the same machine.

  Writing the fresh-reinstall test surfaced a bug rather than a test artefact:
  `windows-service` 0.8.1's `ServiceConfig::from_raw` copies
  `lpBinaryPathName` into `executable_path` verbatim, without stripping the
  quoting its own `ServiceInfo` encoder adds when a path contains a space.
  `install_service`'s existing-service comparison was comparing that
  still-quoted path against a freshly canonicalized, unquoted one, so it
  always disagreed and reported an unrelated-service conflict, for every
  path containing a space, which is to say for every real `%ProgramFiles%`
  installation, not just this test's spaced checkout directory. Fixed by
  `unquote_binary_path` (`windows/service.rs`), which strips one matching
  pair of surrounding quotes before comparing; a canonical filesystem path
  can never itself contain `"` (NTFS forbids it) or end in a bare `\`, so
  that is an exact inverse for every path this installer registers, not a
  general command-line unescaper. Covered by its own unit test alongside the
  VM-run integration coverage above.

- **Windows VM, manual, documented in the plan's verification section:**
  install, start, stop, preshutdown during a real OS restart, failure-action
  restart after a forced kill and, if selected, a reported non-crash failure;
  ProgramData and Program Files ACLs; log file rotation; and a client connecting
  from an ordinary user session after a reboot. Exercise every allocated
  EventID and confirm that none renders the "description … cannot be found"
  preamble. Run manually on the Windows VM on 2026-07-26 and passed, with one
  finding: the ACL inspection surfaced that `%ProgramData%\Luminate` had never
  had its own ACL hardened, fixed the same day (see "Windows machine layout"
  above) and covered going forward by
  `service_install_hardens_the_machine_data_root_against_ordinary_users`.
- **Not tested, stated plainly:** anything requiring the SCM cannot run in CI
  without a Windows runner with elevation. Called out rather than papered over.

## Phase 2 — power and device events

Phase 2.1 (power events) is implemented and passed the full build/lint/test
cycle on the Windows VM on 2026-07-26; see "The plumbing" below for the one
thing that verification could not reach. Phase 2.2 (device events) is
implemented; see below for the final shape and what verification could not
reach.

### 2.1 Power events — implemented

`SERVICE_CONTROL_POWEREVENT` delivers `PBT_APMSUSPEND` and
`PBT_APMRESUMEAUTOMATIC`. These map onto the existing
`SystemPowerEvent::Suspending` and `::Resumed`. Windows may follow the automatic
resume event with `PBT_APMRESUMESUSPEND` when user activity caused the wake, so
the latter is ignored rather than emitting a duplicate resume/reconciliation.
`luminate_platform::windows::power::interpret` (`windows/power/mod.rs`) is the
translation function; it and any other event type return `None` rather than
forwarding something the daemon has no reaction to.

`power::start_sources` remains self-driving and unchanged:
the Windows source has nothing of its own to reach, so extending
`start_sources` with an injection point would have meant threading a
Windows-only concern through a cross-platform entry point that every other
caller ignores. Instead, `daemon::RunContext` gained a `PowerEventSource`
(`Platform`, resolved via `start_sources()` as before, or `External`, an
already-open `PowerEventReceiver`), the same shape already used for
`ShutdownSource` and the lifecycle reporter. `windows::service::ServiceContext`
gained a `power_events: PowerEventReceiver` field; `register_handler` creates
the channel, keeps the sender in `HandlerState`, and hands the receiver out
through `ServiceContext`. The control handler's `SERVICE_CONTROL_POWEREVENT`
arm calls `interpret` and forwards anything it returns over that sender. A
dropped receiver (daemon already tearing down) is not an error, since there is
nothing left to notify. `daemon::run` resolves whichever `PowerEventSource`
it was given immediately before starting `accept_loop`, which now takes the
resolved `PowerEventReceiver` as a parameter instead of calling
`start_sources()` itself.

**`SuspendLease` is a true no-op on Windows, per the original framing
below**. A `PowerRequestSystemRequired` request (`PowerCreateRequest`/
`PowerSetRequest`, wrapped as `SystemSleepRequest` in
`windows/power/request.rs`) was considered for this role and rejected.
`PowerRequestSystemRequired` only influences whether the system's _idle_
detector decides to initiate a suspend; it does not un-commit one already
under way. By the time `PBT_APMSUSPEND` reaches the service, Windows has
already made that decision — there has been no query/veto phase before it
since Vista — so creating the request at that point would not delay the
suspend it was created in response to. `SystemSleepRequest` exists as
unwired groundwork for a different future feature — holding it proactively
while a known operation, such as an effect playing, is in progress, which is
the case where it would help — rather than for the suspend-notification path.
`windows::power::interpret` builds
`SuspendLease::default()` directly, same as any other source with nothing to
release.

### 2.2 Device events — implemented

`register_handler` (`windows/service.rs`) calls `RegisterDeviceNotificationW`
with `DEVICE_NOTIFY_SERVICE_HANDLE` on the service status handle, once per
interface class: `GUID_DEVINTERFACE_HID` and `GUID_DEVINTERFACE_USB_DEVICE`.
Each registration is independently best-effort: a class that fails to
register is logged and contributes no `DevicesChanged` of its own,
mirroring the "a machine with no reachable source still runs" posture of the
Linux udev source. The resulting `SERVICE_CONTROL_DEVICEEVENT` arm in
`control_handler` calls `windows::power::interpret_device_event`, which maps
`DBT_DEVICEARRIVAL` and `DBT_DEVICEREMOVECOMPLETE` to
`SystemPowerEvent::DevicesChanged` and forwards it over the same
`power_events` sender `SERVICE_CONTROL_POWEREVENT` already uses: no new
channel, no daemon-side change. It deliberately does not inspect the
accompanying `DEV_BROADCAST_HDR`: like the uevent source, it reports only
"something changed" and lets the daemon's topology re-pull decide what.

As anticipated under Dependencies, `windows-service` 0.8.1's `ServiceControl`
enum has no `SERVICE_CONTROL_DEVICEEVENT` variant, but this was never a
blocker here: `control_handler` already matches on the raw `u32` control
code from `RegisterServiceCtrlHandlerExW` for every control it handles, not
through that enum, so adding this arm needed no new escape hatch.

The existing `TOPOLOGY_DEBOUNCE` (300 ms, `daemon/mod.rs`) covers the
notification bursts a re-enumeration produces; no separate debounce was added
at the source. Both notification handles are unregistered exactly once, when
`request_shutdown` first runs (guarded by the same `shutdown_sent`
compare-exchange that makes shutdown itself idempotent).

`RescanSignal` on Windows remains a permanently-pending placeholder; hotplug
now drives its own rescan directly rather than through that signal, the same
way resume already does. The control protocol's `Rescan` request remains the
operator-facing path with real feedback, and is the only automatic-equivalent
path in console mode, since `SERVICE_CONTROL_DEVICEEVENT`, like
`SERVICE_CONTROL_POWEREVENT`, only ever reaches a running service.

This has not yet been verified on the Windows VM. Unlike 2.1's suspend/resume gap, this
one is not a VM/hypervisor limitation — plugging in an ordinary USB or HID
device needs no ACPI S3 support — it has just not been exercised there yet.
`interpret_device_event`'s event-type mapping is covered by unit tests;
`register_device_interface_notification`/`unregister_device_interface_notification`
and the full arrival-to-rescan path still need a real run: install the
service, plug/unplug a supported device, and confirm exactly one debounced
rescan (`RescanReason::DeviceChange`) appears in the logs.

## Phase 3 — MSI packaging

Add/Remove Programs visibility and upgrade sequencing are ordinary
expectations for a Windows service, not a nice-to-have, so this is a phase
rather than a deferred item. It does not reimplement installer logic: the MSI
shells out to the same `luminated.exe service install|uninstall` subcommand
phase 1 built, keeping exactly one privileged code path.

- **Tooling: WiX Toolset** (v4, MIT), invoked from CI on the Windows runner
  already needed for the elevated test suite in 1.7. No new licence family;
  confirm the WiX CLI version pinned in CI against `CONTRIBUTING.md`'s
  dependency expectations before landing.
- **Custom actions** wrap `service install` (with `--start` when the "start
  after install" MSI UI option is selected) on install, and `service uninstall`
  on remove/upgrade. The MSI does not duplicate `CreateService`,
  `NetLocalGroupAdd`, or Event Log registration itself; it only decides when to
  invoke the subcommand that already owns those.
- **Upgrade sequencing.** A `MajorUpgrade` element with a stable `UpgradeCode`
  runs the old version's `service uninstall` before the new version's `service
install`, so installer-owned objects (client group, Event Log source) are
  re-evaluated by the same idempotent, ownership-checked logic phase 1 already
  has rather than by new MSI-specific state.
- **Payload.** The MSI stages `luminated.exe` and the plugin tree under
  `%ProgramFiles%\Luminate`, matching the layout phase 1 already assumes; it
  does not stage `%ProgramData%\Luminate` contents, which remain the
  installer subcommand's and the daemon's own responsibility.
- **Uninstall via Add/Remove Programs** must behave like `service uninstall`
  without `--purge-client-group`: the client group and Event Log source are
  left in place by default, for the same reasons given under "Registration: a
  self-install subcommand" above. There is no MSI UI for the purge option in
  this phase; an administrator who wants it still runs the subcommand
  directly.
- **Not in scope for this phase:** a bundled installer UI beyond WiX's
  standard dialogs, silent-install documentation for enterprise deployment
  tooling, and code-signing policy; each is a real question before a public
  release but orthogonal to getting install/uninstall packaged at all.

## Deferred

- **ETW / TraceLogging.** The modern native tracing path, self-describing so it
  needs no manifest, consumed by WPR and PerfView. Excellent for detailed
  structured tracing and invisible to an ordinary administrator, which is why
  it does not replace the Event Log. Revisit if frame-timing traces are wanted.
- **MSIX.** Ruled out: service support is heavily restricted.
- **Virtual service account.** Revisit only if device DACL grants turn out to
  be tractable across the hardware we target.
- **`CTRL_LOGOFF_EVENT`.** Still unhandled. `SIGHUP` on Unix now reloads every
  loaded plugin, but Windows has no equivalent signal to drive the same
  `ReloadSignal` trigger; the control protocol's `ReloadPlugin` request is
  this platform's only path to a plugin reload.

## Known constraints

Collected so a future implementor does not rediscover them the hard way.

- **Windows opens system keyboard and mouse HID top-level collections
  exclusively.** You cannot obtain read/write on them from user mode at any
  privilege level, LocalSystem included. The universal workaround, and what
  OpenRGB does, is `CreateFile` with `dwDesiredAccess = 0` followed by
  `HidD_SetFeature`/`HidD_GetFeature` on the resulting handle. Vendor-specific
  collections (usage page `0xFF00`) are not grabbed and open normally. This
  will matter to whoever ports the Razer plugin.
- **No suspend veto.** See 2.1.
- **`EventLogMessages.dll` depends on .NET Framework 4** being present. It is
  in-box on every supported Windows, but it is a dependency on something we do
  not ship, and worth a graceful message if the registry write finds it absent.
- **The control handler must return promptly.** All real work goes over a
  channel into the runtime. No blocking, no awaiting, in the handler.
- **Client-group membership needs a sign-out or reboot** before it appears in a
  logon token. Expected behaviour on this platform, not a defect; the installer
  states it and the documentation should too.
- **Granting a client `FA` or SDDL `GRGW` on a named pipe is a security bug**,
  not a shortcut: it confers `FILE_CREATE_PIPE_INSTANCE` and permits endpoint
  squatting. Always an explicit mask.
- **A pipe DACL does not authenticate the server.** Keep
  `FILE_FLAG_FIRST_PIPE_INSTANCE`, but also validate the connected server token
  from both client paths. Neither control replaces the other.
- **`iceoryx2` needs LLVM/libclang on any Windows build host.** This work does
  not change that requirement, and any CI runner added
  for the tests in 1.7 inherits it.

## Measured on the Windows VM

Windows 11 build 26200, `x86_64-pc-windows-msvc`, rustc 1.97.1. Run with a
throwaway probe service plus two throwaway local accounts, all removed
afterwards. These results provide evidence for the access-mask and privilege
decisions; they do not replace the final service-SID and server-authentication
tests.

### Pipe DACL behaviour

A probe held a named pipe open with `D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;<mask>;;;
<group>)` and unlimited server instances, so that only the ACL — never instance
exhaustion — could refuse a second instance. A non-admin principal then
attempted both a legitimate connect and an endpoint squat.

The probe used `SY` before the final service-SID DACL decision. Its result
establishes the client-mask behaviour only; phase 1 separately verifies the
final `NT SERVICE\luminated` server ACE.

| principal  | in client group | mask       | connect | create 2nd instance  |
| ---------- | --------------- | ---------- | ------- | -------------------- |
| `lumtest`  | yes             | `0x12019F` | success | succeeded            |
| `lumtest`  | yes             | `0x12019B` | success | denied, `0x80070005` |
| `lumouter` | no              | `0x12019B` | denied  | denied               |

Three things this establishes. The squatting hazard is real and reachable by an
ordinary non-admin client, rather than a theoretical reading of the generic
mapping. Clearing the single bit stops it while leaving normal client traffic
unaffected. And a principal outside the client group is refused outright, which
is what makes "empty group on a fresh install means administrators only" safe by
construction rather than by convention.

The squat denial was confirmed to be a true `ERROR_ACCESS_DENIED`
(`0x80070005`) rather than an incidental failure that happened to look like one.

Group membership resolved correctly in a freshly obtained logon token, which is
the mechanism the reboot-after-install guidance relies on.

### Service privileges and device access

| configuration                                   | privileges | HID enumerate + open |
| ----------------------------------------------- | ---------- | -------------------- |
| console, interactive admin                      | 24         | works                |
| service, LocalSystem, unrestricted              | 28         | works                |
| service, LocalSystem, `SeChangeNotifyPrivilege` | 1          | works                |

HID access here means the full path the plan depends on: `SetupDiGetClassDevsW`
over `GUID_DEVINTERFACE_HID`, `CreateFileW` with `dwDesiredAccess = 0`, then
`HidD_GetAttributes` returning a real vendor and product ID. It keeps working
with the token stripped to a single privilege, which confirms the starting
hypothesis: **device access does not depend on any privilege we would be
stripping.**

The caveat matters. This VM presents exactly one HID interface, a virtualized
QEMU device. The result establishes the _architectural_ claim — HID access is
gated by device DACLs, not by token privileges — and does not establish that
every real vendor device behaves the same way. Confirm against real hardware
when a Windows device plugin exists.

Also observed: `sc privs <service> ""` does not restore the full privilege set;
the previous restriction survives. Undoing `RequiredPrivileges` is not as simple
as passing an empty list, which is worth knowing before relying on it during
debugging.

### Crate and entry-point checks

`windows-service` 0.8.1 resolves to **`windows-sys` 0.61.2**, matching the
workspace pin exactly, so no second binding tree appears. Its only other
dependencies are `bitflags` 2.13.0 and `widestring` 1.2.1; `widestring` is new to
the tree and should be covered by `cargo audit` at adoption.

The console-versus-service auto-detection works as designed, with one
implementation detail worth recording: `windows_service::Error`'s `Display` is
an unhelpful `"IO error in winapi call"`, so the discrimination must match on
`Error::Winapi(io).raw_os_error() == Some(1063)`. That was confirmed to return
exactly `Some(1063)` when the binary is launched from a console.

### Service lifecycle

The implemented daemon was built with `x86_64-pc-windows-msvc` and registered
temporarily as a demand-start LocalSystem service. It reached `RUNNING` with
STOP and PRESHUTDOWN accepted, answered INTERROGATE, reported `STOP_PENDING`
with checkpoint 1 and the planned 21-second wait hint, and reached `STOPPED`
with a zero exit code after graceful shutdown. A second start/interrogate/stop
cycle behaved the same way. The temporary service registration was deleted
afterwards.

The failure-action policy was installed through the production service
subcommand and queried back from SCM. It recorded a 24-hour reset period,
restart delays of 5 seconds, 30 seconds, and 5 minutes, and recovery for
non-crash failures. The raw action array contained the required terminal
no-op after the three restarts; `sc.exe qfailure` omits that no-op from its
rendered output.

### Client-group ownership

The guarded purge path was exercised against the VM's pre-existing
`Luminate Clients` group, which has a member and no installer ownership
marker. `service uninstall --purge-client-group` refused the operation before
touching SCM. The existing stopped service and client group both remained
present. This covers the fail-closed upgrade case without claiming ownership
of an administrator-created object.

### Pipe server authentication

A scheduled task running as LocalSystem created the test pipe with a DACL that
admitted the ordinary development account. The production client connected to
the pipe, obtained the server PID, and rejected it because the task token did
not contain the `NT SERVICE\luminated` SID. This distinguishes the identity
check from an incidental DACL refusal. The same-user console path remains
covered by the ordinary named-pipe transport tests. The temporary task was
deleted after the test.

## Verification

Before phase 1 is considered done:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

run on both the Linux host and the Windows VM, plus `cargo audit` over the
resulting dependency graph and the manual Windows VM checks listed in 1.7.
Anything not run gets stated explicitly in the completion report rather than
assumed.
