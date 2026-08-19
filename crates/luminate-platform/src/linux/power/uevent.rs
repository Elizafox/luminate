// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Linux device-change source: a `NETLINK_KOBJECT_UEVENT` reader.
//!
//! Not a suspend source. It reports the thing the suspend/resume work
//! actually exists to react to (hardware appearing, disappearing, or coming
//! back under a different kernel device node), and it reports it whether the
//! cause was a resume, a hub reset, or an ordinary hotplug. On a machine with
//! no logind it is the only automatic trigger; on one with logind it
//! complements the resume signal, since re-enumeration can lag the resume.
//!
//! Reads the *udev* multicast group rather than the kernel one. The kernel
//! group requires `CAP_NET_ADMIN`, which `luminated` deliberately does not
//! have (see `packaging/systemd/luminated.service.in`: `NoNewPrivileges=yes`,
//! running as an unprivileged user); the udev group is readable by ordinary
//! processes. That does mean this source needs udev or eudev to be running:
//! mdev-only systems get nothing here and fall back to `Request::Rescan` or
//! `SIGUSR1`.
//!
//! Deliberately does not parse the payload. Every filter this could apply
//! (subsystem, vendor, action) would be a guess about which plugin cares,
//! and the daemon already answers that question properly by re-pulling every
//! plugin's topology and comparing. So this decodes nothing and reports only
//! "something changed", debounced.

#![allow(
    unsafe_code,
    reason = "binding and reading a netlink socket requires libc calls the standard library does \
              not wrap"
)]

use std::io;
use std::mem::zeroed;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::time::Duration;

use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio::time::sleep;

use crate::power::{PowerEventSender, SystemPowerEvent};

/// The udev-processed multicast group. Group 1 is the raw kernel broadcast
/// and needs `CAP_NET_ADMIN` to bind; group 2 is what udev re-broadcasts for
/// unprivileged consumers.
const UDEV_MULTICAST_GROUP: u32 = 2;

/// How long to keep absorbing uevents before reporting a change.
///
/// A single resume can produce dozens as the USB tree re-enumerates, and each
/// report costs a full topology re-pull across every plugin. Long enough to
/// coalesce a re-enumeration burst, short enough that a user plugging a
/// keyboard in doesn't wait on it.
const DEBOUNCE: Duration = Duration::from_millis(750);

/// Bound on one datagram read. Uevent messages are far smaller; this exists
/// so a malformed or hostile oversized message is truncated rather than
/// sized by the sender.
const MAX_MESSAGE: usize = 8 * 1024;

/// Starts the reader, or logs why it could not start and returns.
pub(crate) fn start(events: PowerEventSender) {
    let socket = match bind_udev_socket() {
        Ok(socket) => socket,
        Err(error) => {
            tracing::info!(
                error = %error,
                "no udev netlink source; device changes will not trigger an automatic rescan"
            );
            return;
        }
    };

    let socket = match AsyncFd::with_interest(socket, Interest::READABLE) {
        Ok(socket) => socket,
        Err(error) => {
            tracing::warn!(error = %error, "could not register the udev netlink socket");
            return;
        }
    };

    tokio::spawn(async move {
        tracing::info!("watching udev netlink for device changes");
        run(socket, events).await;
    });
}

/// Reads uevents forever, emitting one debounced [`SystemPowerEvent::DevicesChanged`]
/// per burst.
async fn run(socket: AsyncFd<OwnedFd>, events: PowerEventSender) {
    run_with_debounce(socket, events, DEBOUNCE).await;
}

async fn run_with_debounce(socket: AsyncFd<OwnedFd>, events: PowerEventSender, debounce: Duration) {
    loop {
        // Park until something actually arrives, so an idle machine costs
        // nothing.
        if !read_one(&socket).await {
            return;
        }

        // Absorb the rest of the burst. A resume re-enumerates the whole USB
        // tree; reporting each device separately would re-pull every plugin's
        // topology dozens of times for one event.
        let mut absorbed = 1_u32;
        let deadline = sleep(debounce);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                () = &mut deadline => break,
                read = read_one(&socket) => {
                    if !read {
                        return;
                    }
                    absorbed += 1;
                }
            }
        }

        tracing::debug!(
            uevents = absorbed,
            "device change burst; requesting a rescan"
        );
        if events.send(SystemPowerEvent::DevicesChanged).is_err() {
            tracing::debug!("daemon stopped draining power events; ending the udev reader");
            return;
        }
    }
}

/// Waits for and discards one message, returning `false` if the socket died.
///
/// The payload is deliberately not parsed; see the module docs.
async fn read_one(socket: &AsyncFd<OwnedFd>) -> bool {
    loop {
        let Ok(mut ready) = socket.readable().await else {
            return false;
        };
        let mut buffer = [0_u8; MAX_MESSAGE];
        let read = ready.try_io(|socket| {
            // SAFETY: `buffer` is a live, writable array of exactly
            // `MAX_MESSAGE` bytes, and `socket` is an open netlink socket
            // owned by the `AsyncFd`.
            let count = unsafe {
                libc::recv(
                    socket.get_ref().as_raw_fd(),
                    buffer.as_mut_ptr().cast::<libc::c_void>(),
                    buffer.len(),
                    libc::MSG_DONTWAIT,
                )
            };
            if count < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(count)
        });

        match read {
            Ok(Ok(_count)) => return true,
            // A false-positive readiness, or a racing reader took the
            // datagram: wait for the socket to become readable again.
            Err(_would_block) => {}
            Ok(Err(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
            Ok(Err(error)) => {
                tracing::warn!(error = %error, "udev netlink read failed; ending the reader");
                return false;
            }
        }
    }
}

/// `AF_NETLINK` narrowed to the address family width.
///
/// `libc` types the constant as `c_int` and the field as `sa_family_t`; the
/// value is 16, so this is lossless, and a `try_from` here would introduce an
/// error path that cannot occur.
#[allow(
    clippy::cast_possible_truncation,
    reason = "AF_NETLINK is 16, far inside sa_family_t"
)]
const fn netlink_family() -> libc::sa_family_t {
    libc::AF_NETLINK as libc::sa_family_t
}

/// `sockaddr_nl`'s size as the length `bind` expects.
///
/// The struct is 12 bytes, so narrowing to `socklen_t` cannot lose anything.
#[allow(
    clippy::cast_possible_truncation,
    reason = "sockaddr_nl is 12 bytes, far inside socklen_t"
)]
const fn address_length() -> libc::socklen_t {
    size_of::<libc::sockaddr_nl>() as libc::socklen_t
}

/// Binds a non-blocking netlink socket to the udev multicast group.
fn bind_udev_socket() -> io::Result<OwnedFd> {
    // SAFETY: `socket` takes only integer arguments and returns a file
    // descriptor or -1.
    let raw = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            libc::NETLINK_KOBJECT_UEVENT,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a fresh, exclusively owned descriptor from `socket`.
    let socket = unsafe { OwnedFd::from_raw_fd(raw) };

    // Zeroed rather than field-by-field: `sockaddr_nl` carries a private
    // padding field this crate cannot name, and the kernel expects the
    // padding zeroed anyway.
    // SAFETY: `sockaddr_nl` is a plain C struct of integers, for which an
    // all-zero bit pattern is valid.
    let mut address: libc::sockaddr_nl = unsafe { zeroed() };
    address.nl_family = netlink_family();

    // Leaving `nl_pid` zero lets the kernel assign the port ID, so two
    // readers in one process (or a restarted daemon) cannot collide on a
    // fixed one.
    address.nl_groups = UDEV_MULTICAST_GROUP;

    // SAFETY: `address` is a live, correctly typed `sockaddr_nl`, and its
    // length is that type's size.
    let bound = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            std::ptr::addr_of!(address).cast::<libc::sockaddr>(),
            address_length(),
        )
    };
    if bound < 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(socket)
}

#[cfg(test)]
#[path = "uevent_tests.rs"]
mod tests;
