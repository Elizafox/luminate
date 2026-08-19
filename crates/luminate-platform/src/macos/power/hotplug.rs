// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! macOS device-change source: `IOKit` service-matching notifications for
//! the generic `"IOService"` class, delivered through a `CFRunLoop` on a
//! dedicated thread.
//!
//! Matches broadly rather than against a specific device class (USB, HID,
//! ...), mirroring the Linux uevent reader (`crate::linux::power::uevent`)
//! and the Windows device-event handling
//! (`crate::windows::power::interpret_device_event`): neither of those
//! filters by device class either, on the same reasoning that guessing
//! which plugin cares is the daemon's job during its topology re-pull, not
//! this source's. The cost is a noisier signal than a narrower USB/HID-only
//! match would give, since any `IOKit` driver instantiation or termination
//! reports here, not just the hardware Luminate's plugins actually talk to;
//! the debounce below exists because of it.

#![allow(
    unsafe_code,
    reason = "registering for IOKit service-matching notifications requires calling into IOKit \
              and CoreFoundation C APIs the standard library does not wrap"
)]

use std::ffi::{CStr, c_char, c_void};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use objc2_core_foundation::{CFDictionary, CFRetained, CFRunLoop, kCFRunLoopDefaultMode};
use objc2_io_kit::{
    IOIteratorNext, IONotificationPort, IONotificationPortRef, IOObjectRelease,
    IOServiceAddMatchingNotification, IOServiceMatching, io_iterator_t, kIOFirstMatchNotification,
    kIOMainPortDefault, kIOTerminatedNotification,
};

use crate::power::{PowerEventSender, SystemPowerEvent};

/// How long to keep absorbing pings before reporting a change.
///
/// Mirrors `crate::linux::power::uevent::DEBOUNCE`: a resume or a hub reset
/// can produce a burst of matches and terminations, and each report costs a
/// full topology re-pull across every plugin.
const DEBOUNCE: Duration = Duration::from_millis(750);

/// Starts the watcher and its debounce stage, each on a dedicated OS thread.
///
/// Two threads rather than one: the watcher parks in `CFRunLoop::run` for
/// the rest of the process's life (see `crate::macos::power::sleep` for why
/// that rules out a tokio task), so debouncing has to happen somewhere else.
/// A second plain thread reading a blocking `std::sync::mpsc` channel is the
/// simplest "somewhere else" that doesn't need a tokio runtime handle
/// either, since sending on the tokio [`PowerEventSender`] this ultimately
/// reports through needs no runtime context to call.
pub(crate) fn start(events: PowerEventSender) {
    let (pings_tx, pings_rx) = mpsc::channel::<()>();

    let debounce_spawned = thread::Builder::new()
        .name("luminate-iokit-hotplug-debounce".into())
        .spawn(move || debounce(&pings_rx, &events));
    if let Err(error) = debounce_spawned {
        tracing::warn!(error = %error, "could not start the IOKit hotplug debounce thread");
        return;
    }

    let watcher_spawned = thread::Builder::new()
        .name("luminate-iokit-hotplug".into())
        .spawn(move || run(pings_tx));
    if let Err(error) = watcher_spawned {
        tracing::warn!(error = %error, "could not start the IOKit hotplug watcher thread");
    }
}

/// Absorbs a burst of pings into one [`SystemPowerEvent::DevicesChanged`].
fn debounce(pings: &mpsc::Receiver<()>, events: &PowerEventSender) {
    while pings.recv().is_ok() {
        while pings.recv_timeout(DEBOUNCE).is_ok() {}
        if events.send(SystemPowerEvent::DevicesChanged).is_err() {
            tracing::debug!(
                "daemon stopped draining power events; ending the IOKit hotplug debounce"
            );
            return;
        }
    }
}

fn run(pings: mpsc::Sender<()>) {
    // SAFETY: process-lifetime static both IOKit and CoreFoundation expect
    // callers to read directly.
    let main_port = unsafe { kIOMainPortDefault };
    let notify_port = IONotificationPort::create(main_port);
    if notify_port.is_null() {
        tracing::info!(
            "could not create an IOKit notification port; device changes will not trigger an \
             automatic rescan"
        );
        return;
    }

    let context = Box::into_raw(Box::new(pings));

    let Some(first_match) = add_matching_notification(
        notify_port,
        c"IOService",
        kIOFirstMatchNotification,
        context,
    ) else {
        tracing::warn!("could not watch for IOKit devices appearing");
        // SAFETY: neither registration below can have succeeded yet, so no
        // callback can be in flight and this function is `context`'s only
        // owner.
        drop(unsafe { Box::from_raw(context) });
        return;
    };
    drain(first_match);

    let Some(terminated) = add_matching_notification(
        notify_port,
        c"IOService",
        kIOTerminatedNotification,
        context,
    ) else {
        // The first-match watch armed above is already reporting; losing
        // the termination half is a degraded signal, not a reason to tear
        // down a source that is otherwise working.
        tracing::warn!("could not watch for IOKit devices disappearing");
        return;
    };
    drain(terminated);

    let Some(run_loop) = CFRunLoop::current() else {
        tracing::warn!("no current CFRunLoop; cannot watch for IOKit device changes");
        return;
    };
    // SAFETY: `notify_port` is the handle created above.
    let Some(run_loop_source) = (unsafe { IONotificationPort::run_loop_source(notify_port) })
    else {
        tracing::warn!("IOKit gave no run-loop source for its device notification port");
        return;
    };
    // SAFETY: process-lifetime static.
    let mode = unsafe { kCFRunLoopDefaultMode };
    run_loop.add_source(Some(&run_loop_source), mode);

    tracing::info!("watching IOKit for device changes");
    CFRunLoop::run();
}

/// Arms one matching notification, returning the iterator to drain.
fn add_matching_notification(
    notify_port: IONotificationPortRef,
    class_name: &CStr,
    notification_type: &CStr,
    context: *mut mpsc::Sender<()>,
) -> Option<io_iterator_t> {
    // SAFETY: `class_name` is a live, NUL-terminated C string for the
    // duration of this call.
    let matching = unsafe { IOServiceMatching(class_name.as_ptr())? };
    // SAFETY: `CFMutableDictionary` is declared to extend `CFDictionary` (see
    // `objc2_core_foundation`'s `cf_type!` invocation for it), so
    // reinterpreting the retained handle as the plain, non-mutable
    // supertype is exactly the upcast that relationship promises.
    let matching: CFRetained<CFDictionary> = unsafe { CFRetained::cast_unchecked(matching) };

    let mut notification_type_buffer = io_name_buffer(notification_type);
    let mut iterator: io_iterator_t = 0;
    // SAFETY: `notify_port` is a live notification port; `matching` is a
    // freshly created dictionary this call consumes exactly once;
    // `notification_type_buffer` is a live 128-byte buffer holding a
    // NUL-terminated copy of `notification_type`; `hotplug_callback`
    // matches `IOServiceMatchingCallback`'s signature exactly; `iterator` is
    // a valid out-parameter.
    let result = unsafe {
        IOServiceAddMatchingNotification(
            notify_port,
            &raw mut notification_type_buffer,
            Some(matching),
            Some(hotplug_callback),
            context.cast::<c_void>(),
            &raw mut iterator,
        )
    };

    (result == 0 && iterator != 0).then_some(iterator)
}

/// Copies a notification-type C string into the fixed-size buffer
/// `IOServiceAddMatchingNotification` expects (`io_name_t`, a private type
/// alias in `objc2_io_kit` for `*mut [c_char; 128]`).
fn io_name_buffer(value: &CStr) -> [c_char; 128] {
    let bytes = value.to_bytes_with_nul();
    let mut buffer = [0 as c_char; 128];
    for (slot, &byte) in buffer.iter_mut().zip(bytes) {
        *slot = byte.cast_signed();
    }
    buffer
}

/// Drains a notification iterator, releasing every object it hands back.
///
/// Mandatory, not cleanup: `IOKit` does not consider a notification re-armed,
/// and so will not deliver the next one, while its iterator still holds
/// unacknowledged entries.
fn drain(iterator: io_iterator_t) {
    loop {
        let object = IOIteratorNext(iterator);
        if object == 0 {
            return;
        }
        // Every object IOKitLib hands out must be released once the caller
        // is done with it, and this reader only drains the notification,
        // never inspecting the object itself.
        IOObjectRelease(object);
    }
}

unsafe extern "C-unwind" fn hotplug_callback(refcon: *mut c_void, iterator: io_iterator_t) {
    drain(iterator);

    // SAFETY: `refcon` is the raw `mpsc::Sender<()>` pointer this thread
    // boxed and handed to `IOServiceAddMatchingNotification`, live for as
    // long as the registration (and this thread) is.
    let pings = unsafe { &*refcon.cast::<mpsc::Sender<()>>() };
    let _ = pings.send(());
}

#[cfg(test)]
#[path = "hotplug_tests.rs"]
mod tests;
