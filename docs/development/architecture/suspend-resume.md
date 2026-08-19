<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Suspend, resume, and hardware rescanning

> Status: landed. This document is the permanent design record.

## Why this exists

When a machine suspends and resumes, USB and HID devices frequently
re-enumerate: the same physical hardware comes back under a different kernel
device node or bus address. Hardware may also have been power-cycled, reset,
or changed by another controller while the machine was down.

Before this work the daemon had no idea any of that had happened. It kept
presenting pre-suspend observations as current, and nothing prompted plugins
to look at the hardware again. The topology-refresh machinery was complete and
correct — but the only thing that could *start* it was a plugin noticing a
change on its own, and plugins that poll only notice on their own schedule.
The Alienware plugin has no poller at all.

This is the reconciliation trigger
[`state-reconciliation.md`](state-reconciliation.md) already anticipated:
"resume from suspend when hardware state may have changed."

## The shape of it

Two halves, deliberately separated:

- **`luminate_platform::power`** finds out that something happened. It lives
  in `luminate-platform` (`crates/luminate-platform`), not `luminated` itself,
  because a suspend/resume source is a platform fact other host applications
  would want too, not something specific to this daemon. One module per
  platform mechanism, each emitting `SystemPowerEvent` and nothing else; the
  cross-platform `SystemPowerEvent`/`start_sources` surface lives at
  `luminate_platform::power`, and each platform's sources live under
  `luminate_platform::{linux,macos,windows}` (see "Windows" below for why
  Windows's sources are reached
  differently — through the service control handler rather than
  `start_sources` — but still without touching the daemon).
- **`luminated::daemon::suspend`** decides what to do about it.

Adding a platform therefore means writing a source under
`crates/luminate-platform/src/{linux,macos,windows}/power/`, not touching the
daemon.

### The rescan primitive

Everything funnels into one operation: ask every plugin to re-enumerate, then
reconcile whatever changed.

```text
resume ─┐
uevent ─┤
Rescan  ├─► TopologyNotification::Rescan ─► topology coordinator
SIGUSR1 ┘                                    │
                                             ├─ PluginManager::rescan_all
                                             │    (invalidate plugin caches)
                                             ├─ reconcile_plugin_topology
                                             │    (re-pull, re-arbitrate ownership)
                                             └─ reconcile_device per change
                                                  (existing policy: restore/adopt/leave)
```

The last step is the same path daemon startup and device hotplug take. Resume
is not a special kind of reconciliation — it is one more moment where Luminate
regains control of hardware whose state it can no longer vouch for.

A rescan widens the batch to every loaded plugin and absorbs any
plugin-observed notifications arriving in the same debounce window, so a resume
that also produces a flurry of uevents does one pass rather than several.

### On suspend

While holding the inhibitor lease, the daemon:

1. **Ends every active frame stream** and publishes `Event::ShmStreamEnded`.
   The daemon cannot revoke a client's shared-memory mapping, so the event is
   the only way a client learns to stop writing into a machine that is going
   down.
2. **Marks every observation stale.** Hardware can be power-cycled or reset
   while the machine is down, so pre-suspend readings stop being evidence.
   They are marked, not erased — a stale value is still the best available
   knowledge until a readback replaces it, mirroring how a daemon restart
   treats a restored adopted baseline.
3. **Leaves desired state alone.** Intent survives a suspend; only knowledge
   of hardware doesn't.

It deliberately does not drive hardware dark. That would be a policy
decision about what the user wants, and quiescing's job is only to stop
claiming things that are about to stop being true.

### On resume

Request a rescan and get out of the way. Everything else is the topology
coordinator's existing job.

## Platform sources

### Linux: logind / elogind (`luminate_platform::linux::power::logind`)

Subscribes to `PrepareForSleep` on `org.freedesktop.login1` and holds a
`delay` inhibitor lock. elogind exposes the identical interface, so Devuan,
Artix, Gentoo/OpenRC, and Alpine-with-elogind are covered by the same code.

The inhibitor is what makes quiescing meaningful. `PrepareForSleep(true)` on
its own is only a heads-up — logind proceeds regardless — but while a `delay`
lock is held it waits, bounded by `InhibitDelayMaxSec`. The lock's file
descriptor rides inside the `SuspendLease` handed to the daemon, so the delay
ends when the daemon drops it, not on a timer anyone guessed. Verified with
`systemd-inhibit --list`:

```text
WHO       UID   USER      PID     COMM      WHAT   WHY                                         MODE
Luminate  1000  elizabeth 480184  luminated sleep  Quiescing lighting hardware before suspend  delay
```

`delay` rather than `block`: Luminate needs a moment, and has no business
preventing a suspend outright.

Built behind `luminate-platform`'s `logind` feature (forwarded by `luminated`'s
own `logind` feature), on by default for Linux. Turn it off for a build that
must not link zbus.

### Linux: udev netlink (`luminate_platform::linux::power::uevent`)

Not a suspend source. It reports the thing this work exists to react to —
hardware appearing, disappearing, or returning under a different device node —
whether the cause was a resume, a hub reset, or an ordinary hotplug.

Reads the **udev** multicast group, not the kernel one. Binding the kernel
group needs `CAP_NET_ADMIN`, which `luminated` deliberately lacks
(`NoNewPrivileges=yes`, unprivileged user); the udev group is readable by
ordinary processes. `RestrictAddressFamilies=… AF_NETLINK` and
`After=systemd-udevd.service` in the unit already accommodate this.

It parses nothing. Every filter it could apply (subsystem, vendor, action)
would be a guess about which plugin cares, and the daemon already answers that
question properly by re-pulling topology and comparing. Bursts are debounced,
since one resume can produce dozens of uevents and each report costs a full
re-pull.

### Windows: `SERVICE_CONTROL_POWEREVENT` / `SERVICE_CONTROL_DEVICEEVENT` (`luminate_platform::windows::power`, `luminate_platform::windows::service`)

Unlike Linux's sources, neither reaches a background task on its own — both
are controls SCM delivers only to a running service's control handler, so
there is no Windows counterpart to `power::start_sources` self-driving
anything. `daemon::RunContext` instead gains a `PowerEventSource::External`
carrying an already-open `PowerEventReceiver` that `register_handler`
(`windows/service.rs`) created and fed from the control handler; `Platform`
(`start_sources()`) remains what console mode and every other platform use.

`SERVICE_CONTROL_POWEREVENT` delivers `PBT_APMSUSPEND`/`PBT_APMRESUMEAUTOMATIC`,
mapped by `windows::power::interpret` onto `Suspending`/`Resumed`.
`SERVICE_CONTROL_DEVICEEVENT` delivers `DBT_DEVICEARRIVAL`/
`DBT_DEVICEREMOVECOMPLETE` for the interface classes `register_handler`
registered for (`GUID_DEVINTERFACE_HID`, `GUID_DEVINTERFACE_USB_DEVICE`, via
`RegisterDeviceNotificationW` with `DEVICE_NOTIFY_SERVICE_HANDLE`), mapped by
`windows::power::interpret_device_event` onto `DevicesChanged`. Both funnel
through the same `power_events` sender and the same daemon-side handling as
every other platform. Like the udev source, the device-event interpreter
parses nothing beyond arrival-versus-removal and leans on the existing
300 ms `TOPOLOGY_DEBOUNCE` for burst coalescing rather than debouncing at the
source.

Console mode gets neither automatically, for the same reason: both controls
only ever reach a running service. Windows cannot delay or veto suspend through
this callback, so its `SuspendLease` is a genuine no-op: the daemon still
quiesces immediately, but SCM does not wait for an acknowledgement from it.

### macOS: IOKit power and service notifications (`luminate_platform::macos::power`)

`IORegisterForSystemPower` reports pending sleep and completed wake on a
dedicated Core Foundation run-loop thread. Its sleep acknowledgement is held
inside `SuspendLease`, so dropping the lease after daemon quiescing calls
`IOAllowPowerChange` and lets the pending sleep proceed.

`IOServiceAddMatchingNotification` watches broad `IOService` appearance and
termination notifications on a second run-loop thread. The source deliberately
does not guess which device classes plugins care about; it debounces the noisy
signal for 750 ms, then emits one `DevicesChanged` event for the ordinary
topology re-pull.

### Coverage summary

| Environment                        | Suspend/resume             | Device change               |
| ---------------------------------- | -------------------------- | --------------------------- |
| systemd Linux                      | logind                     | udev netlink                |
| elogind + eudev (Devuan, Artix, …) | elogind                    | udev netlink                |
| Alpine (elogind + eudev)           | elogind                    | udev netlink                |
| Alpine (mdev, no elogind)          | —                          | —                           |
| Windows (service)                  | SERVICE_CONTROL_POWEREVENT | SERVICE_CONTROL_DEVICEEVENT |
| Windows (console)                  | —                          | —                           |
| macOS                              | IOKit system power         | IOKit service notifications |

Rows with no automatic source still work; they need an explicit trigger.

## Operator triggers

Both do exactly what a resume does, with `RescanReason::Operator`:

- **`luminatectl rescan`** — the real interface, authorized like any other
  daemon-administration request (`Request::Rescan`). Reports
  `rescan scheduled`, because it acknowledges scheduling rather than
  completion: re-enumerating network hardware can outlast the command.
- **`kill -USR1 $(pidof luminated)`** — for callers that cannot speak the
  protocol. An init system, a sleep hook, a shell script. No feedback beyond
  the daemon's own logs.

Both exist so a platform with no native hook is degraded, not broken.

## Writing a plugin that needs a rescan

Most plugins need nothing. If `topology()` enumerates hardware on every call,
the daemon's re-pull already sees current hardware.

Implement `RescanPlugin` only when `topology()` answers from a cache that a
background discovery thread owns — otherwise a rescan just replays the
pre-suspend view until the plugin's own poll interval catches up. LIFX, WLED,
and Govee are the in-tree examples: each wakes its discovery thread through
`DiscoveryPacer`, which replaces the inter-cycle `thread::sleep`.

Such a plugin should not clear its device registry. Dropping entries makes
every known device vanish and reappear, and `DynamicDeviceRegistry`'s expiry
already retires hardware that really is gone — while its stable identities are
what let persisted state survive a re-enumeration at all.

Which brings up the requirement underneath this whole document: **derive
`DeviceDescriptor::id` from stable physical identity, never from an
enumeration-order path.** An ID built from `/dev/hidraw2` becomes a different
device the moment the kernel hands out `hidraw5`, and the daemon correctly
concludes that one device withdrew and an unrelated one arrived — losing the
association with persisted state. See
[`../plugin-authoring.md`](../plugin-authoring.md).

## What is not covered

- **Hibernate versus suspend** is not distinguished. Both produce the same
  logind signals and both invalidate observations identically.
- **Plugin pollers still exist.** The uevent source could replace most of
  them, but migrating each plugin off its own loop is follow-up work.
- **mdev-only Alpine** gets no automatic trigger; this is an accepted
  limitation, with eudev/elogind the supported configuration.
