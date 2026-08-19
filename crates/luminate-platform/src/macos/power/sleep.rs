// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! macOS suspend/resume source: `IORegisterForSystemPower`, `IOKit`'s
//! root-power-domain client API, delivered through a `CFRunLoop` on a
//! dedicated thread.
//!
//! `IOAllowPowerChange` is the acknowledgement that lets a pending sleep
//! proceed, so it plays the same role here that closing the inhibitor file
//! descriptor plays for logind (`crate::linux::power::logind`): held inside
//! the [`SuspendLease`] returned to the caller, and fired only once that
//! lease drops, so the sleep waits exactly as long as quiescing takes.

#![allow(
    unsafe_code,
    reason = "registering for and acknowledging IOKit power notifications requires calling into \
              IOKit and CoreFoundation C APIs the standard library does not wrap"
)]

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr;
use std::thread;

use objc2_core_foundation::{CFRunLoop, kCFRunLoopDefaultMode};
use objc2_io_kit::{
    IOAllowPowerChange, IONotificationPort, IONotificationPortRef, IORegisterForSystemPower,
    io_connect_t, io_object_t, io_service_t, kIOMessageSystemHasPoweredOn,
    kIOMessageSystemWillSleep,
};

use crate::power::{PowerEventSender, SuspendLease, SystemPowerEvent};

/// Starts the watcher on a dedicated OS thread.
///
/// A dedicated `std::thread`, not a tokio task: `CFRunLoop::run` parks the
/// calling thread for as long as the registration is alive, which here is
/// the rest of the process's life. Nothing in this path is async, so a
/// tokio worker has nothing to gain and everything to lose by blocking on
/// it.
pub(crate) fn start(events: PowerEventSender) {
    let spawned = thread::Builder::new()
        .name("luminate-iokit-power".into())
        .spawn(move || run(events));
    if let Err(error) = spawned {
        tracing::warn!(
            error = %error,
            "could not start the IOKit suspend/resume watcher thread"
        );
    }
}

/// Per-registration state the callback reads back through its `refcon`.
///
/// `kernel_port` is filled in after registration succeeds (see [`run`]): it
/// is the return value of `IORegisterForSystemPower` itself, so it cannot be
/// known before that call, but the callback needs it to acknowledge a
/// sleep. A [`Cell`] rather than an atomic is enough because `IOKit` only ever
/// invokes the callback from the thread already running the `CFRunLoop`
/// that services it, i.e. the same thread that sets it.
struct Context {
    events: PowerEventSender,
    kernel_port: Cell<io_connect_t>,
}

fn run(events: PowerEventSender) {
    let context = Box::into_raw(Box::new(Context {
        events,
        kernel_port: Cell::new(0),
    }));

    let mut notify_port: IONotificationPortRef = ptr::null_mut();
    let mut notifier: io_object_t = 0;

    // SAFETY: `context` is a live, exclusively owned allocation this
    // function just created; `notify_port`/`notifier` are valid
    // out-parameters for IOKit to populate; `power_callback` matches
    // `IOServiceInterestCallback`'s signature exactly.
    let kernel_port = unsafe {
        IORegisterForSystemPower(
            context.cast::<c_void>(),
            &raw mut notify_port,
            Some(power_callback),
            &raw mut notifier,
        )
    };

    if kernel_port == 0 {
        tracing::info!(
            "could not register for IOKit system power notifications; suspend/resume will not \
             be handled automatically"
        );
        // SAFETY: registration failed, so IOKit never received this pointer
        // and never will; this function is its only owner.
        drop(unsafe { Box::from_raw(context) });
        return;
    }

    // SAFETY: `context` is still the same live allocation from above.
    // Nothing but this thread can touch it before `CFRunLoop::run` starts
    // invoking `power_callback` on it.
    unsafe { (*context).kernel_port.set(kernel_port) };

    // SAFETY: `notify_port` is the handle `IORegisterForSystemPower` just
    // populated on success.
    let Some(run_loop_source) = (unsafe { IONotificationPort::run_loop_source(notify_port) })
    else {
        tracing::warn!("IOKit gave no run-loop source for its power notification port");
        return;
    };

    let Some(run_loop) = CFRunLoop::current() else {
        tracing::warn!("no current CFRunLoop; cannot watch for IOKit power notifications");
        return;
    };

    // SAFETY: `kCFRunLoopDefaultMode` is a process-lifetime static both
    // IOKit and CoreFoundation expect callers to read directly.
    let mode = unsafe { kCFRunLoopDefaultMode };
    run_loop.add_source(Some(&run_loop_source), mode);

    tracing::info!("watching IOKit for suspend and resume");
    // Blocks for the rest of this thread's life. There is no graceful stop:
    // the daemon's own shutdown tears the whole process, and this thread
    // with it, down together.
    CFRunLoop::run();
}

/// Acknowledges an `IOKit` sleep notification when dropped.
///
/// Held inside a [`SuspendLease`]; see the module docs for why the
/// acknowledgement belongs there rather than firing immediately.
#[derive(Debug)]
pub(crate) struct PowerChangeAck {
    kernel_port: io_connect_t,
    notification_id: isize,
}

impl Drop for PowerChangeAck {
    fn drop(&mut self) {
        // `kernel_port` came from this thread's own
        // `IORegisterForSystemPower` call and stays valid for the life of
        // the registration; `notification_id` is the value IOKit handed the
        // callback for this specific sleep notification, which is exactly
        // what `IOAllowPowerChange` expects back.
        IOAllowPowerChange(self.kernel_port, self.notification_id);
    }
}

unsafe extern "C-unwind" fn power_callback(
    refcon: *mut c_void,
    _service: io_service_t,
    message_type: u32,
    message_argument: *mut c_void,
) {
    // SAFETY: IOKit hands back exactly the pointer `run` passed as `refcon`
    // to `IORegisterForSystemPower`, and this callback can only fire while
    // that registration (and the thread owning it) is alive.
    let context = unsafe { &*refcon.cast::<Context>() };

    // Not a `match` on the `kIOMessage*` constants: they are lowercase-led
    // names from IOKit's own headers, and matching on them as patterns reads
    // to rustc as shadowed bindings rather than constant comparisons.
    if message_type == kIOMessageSystemWillSleep {
        let lease = SuspendLease {
            allow_change: Some(PowerChangeAck {
                kernel_port: context.kernel_port.get(),
                // IOKit passes the notification ID as an integer disguised
                // as a pointer; see Apple's own `IORegisterForSystemPower`
                // sample code.
                notification_id: message_argument as isize,
            }),
        };
        // A send failure drops `lease` immediately, which drops the
        // `PowerChangeAck` inside it and acknowledges the sleep right away
        // -- the correct behaviour when nobody is listening to quiesce for.
        let _ = context.events.send(SystemPowerEvent::Suspending(lease));
    } else if message_type == kIOMessageSystemHasPoweredOn {
        let _ = context.events.send(SystemPowerEvent::Resumed);
    }
}

#[cfg(test)]
#[path = "sleep_tests.rs"]
mod tests;
