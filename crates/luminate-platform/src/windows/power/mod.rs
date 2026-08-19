// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Windows-specific implementations of power and device-change controls.
//!
//! Windows delivers suspend/resume as `SERVICE_CONTROL_POWEREVENT` and device
//! hotplug as `SERVICE_CONTROL_DEVICEEVENT`, both to the service control
//! handler and neither to something a background task can listen for on its
//! own; [`interpret`] and [`interpret_device_event`] are what the handler in
//! [`crate::windows::service`] calls to turn a raw `dwEventType` into the
//! cross-platform [`SystemPowerEvent`]. Unlike power events, which arrive
//! unconditionally once the service accepts them, device events only arrive
//! for interface classes the service registered for; that registration lives
//! in [`crate::windows::service`] alongside the status handle it is
//! registered against, not here.
//!
//! [`request::SystemSleepRequest`] lives alongside this module but is
//! deliberately not used here. Windows has given services no way to veto a
//! suspend since Vista, and by the time `PBT_APMSUSPEND` reaches this
//! handler, the transition is already committed. A `PowerRequestSystemRequired`
//! request created at that point cannot un-commit it, since it only
//! influences whether the *idle* detector decides to start a transition, not
//! whether one already under way completes. It would need to be held
//! proactively, before the idle timer fires, to do anything at all.

use windows_sys::Win32::UI::WindowsAndMessaging::{
    DBT_DEVICEARRIVAL, DBT_DEVICEREMOVECOMPLETE, PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND,
};

use crate::power::{SuspendLease, SystemPowerEvent};

pub(crate) mod request;

/// Translates a `SERVICE_CONTROL_POWEREVENT` `dwEventType` into the
/// cross-platform power event it corresponds to, if any.
///
/// Windows delivers several event types a service has no reason to act on,
/// including battery status and power-source changes. It may also follow
/// `PBT_APMRESUMEAUTOMATIC` with `PBT_APMRESUMESUSPEND` when user activity
/// caused the wake. Those, and any event type this daemon doesn't recognise,
/// return `None` rather than triggering a spurious or duplicate rescan.
pub(crate) fn interpret(event_type: u32) -> Option<SystemPowerEvent> {
    match event_type {
        PBT_APMSUSPEND => Some(SystemPowerEvent::Suspending(SuspendLease::default())),
        PBT_APMRESUMEAUTOMATIC => Some(SystemPowerEvent::Resumed),
        _ => None,
    }
}

/// Translates a `SERVICE_CONTROL_DEVICEEVENT` `dwEventType` into the
/// cross-platform device-change event it corresponds to, if any.
///
/// Deliberately does not inspect the accompanying `DEV_BROADCAST_HDR`: like
/// the Linux uevent source (`crate::linux::power::uevent`), this reports only
/// "something changed" and lets the daemon's topology re-pull decide what,
/// rather than guessing which plugin cares from the event's device details.
pub(crate) fn interpret_device_event(event_type: u32) -> Option<SystemPowerEvent> {
    match event_type {
        DBT_DEVICEARRIVAL | DBT_DEVICEREMOVECOMPLETE => Some(SystemPowerEvent::DevicesChanged),
        _ => None,
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
