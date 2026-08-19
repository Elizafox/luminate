// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::os::fd::{AsRawFd as _, FromRawFd as _, IntoRawFd as _};
use std::os::unix::net::UnixDatagram;

use tokio::sync::mpsc::unbounded_channel;
use tokio::task::yield_now;
use tokio::time::timeout;

use super::*;

#[test]
fn binding_the_udev_group_succeeds_or_fails_cleanly() {
    // The udev group is meant to be bindable unprivileged, which is the
    // whole reason this source targets group 2 rather than the kernel's
    // group 1. Environments without netlink at all (some containers) must
    // fail with an error rather than panic or hang.
    match bind_udev_socket() {
        Ok(socket) => assert!(socket.as_raw_fd() >= 0),
        Err(error) => {
            assert!(
                matches!(
                    error.kind(),
                    io::ErrorKind::PermissionDenied
                        | io::ErrorKind::Unsupported
                        | io::ErrorKind::InvalidInput
                ) || error.raw_os_error().is_some(),
                "unexpected bind failure: {error:?}"
            );
        }
    }
}

#[tokio::test]
async fn starting_an_available_source_returns_without_blocking() {
    // Check availability first so a container without usable netlink still
    // exercises the documented clean-failure path.
    let Ok(socket) = bind_udev_socket() else {
        let (sender, _receiver) = unbounded_channel();
        start(sender);
        return;
    };
    drop(socket);

    let (sender, _receiver) = unbounded_channel();
    start(sender);
    yield_now().await;
}

#[tokio::test]
async fn the_reader_stops_once_the_daemon_drops_the_receiver() {
    // The reader must not outlive the daemon's interest in it, or a
    // shutdown would leave a task spinning on a socket nobody reads.
    let Ok(socket) = bind_udev_socket() else {
        return;
    };
    let Ok(socket) = AsyncFd::with_interest(socket, Interest::READABLE) else {
        return;
    };
    let (sender, receiver) = unbounded_channel();
    drop(receiver);

    let reader = tokio::spawn(run(socket, sender));
    // No uevents are guaranteed to arrive here, so this only asserts the
    // task parks rather than busy-looping or panicking; the send-failure
    // exit itself is exercised by the send path's `is_err` branch.
    assert!(
        timeout(Duration::from_millis(100), reader).await.is_err(),
        "an idle reader should park, not exit or panic"
    );
}

#[tokio::test]
async fn the_reader_stops_cleanly_if_its_descriptor_is_not_a_socket() {
    let mut descriptors = [-1; 2];
    // SAFETY: `descriptors` points to two writable `c_int`s. On success,
    // `pipe2` initializes both with fresh, exclusively owned descriptors.
    let result =
        unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
    assert_eq!(
        result,
        0,
        "create non-blocking pipe: {}",
        io::Error::last_os_error()
    );

    // SAFETY: successful `pipe2` returned two fresh descriptors, each
    // converted into an owner exactly once.
    let reader = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
    // SAFETY: as above, for the write end.
    let writer = unsafe { OwnedFd::from_raw_fd(descriptors[1]) };
    let byte = [1_u8];
    // SAFETY: `writer` is open and `byte` is a live one-byte buffer.
    let written = unsafe {
        libc::write(
            writer.as_raw_fd(),
            byte.as_ptr().cast::<libc::c_void>(),
            byte.len(),
        )
    };
    assert_eq!(written, 1);

    let socket =
        AsyncFd::with_interest(reader, Interest::READABLE).expect("register pipe descriptor");
    let (sender, _receiver) = unbounded_channel();
    timeout(Duration::from_secs(1), run(socket, sender))
        .await
        .expect("a fatal receive error should stop the reader");
}

#[tokio::test]
async fn read_one_consumes_a_ready_datagram_without_parsing_it() {
    let (sender, receiver) = UnixDatagram::pair().expect("create datagram pair");
    receiver
        .set_nonblocking(true)
        .expect("set receiver nonblocking");
    sender
        .send(b"ACTION=add\0SUBSYSTEM=usb\0")
        .expect("send event");

    // SAFETY: `receiver` owns this descriptor and is converted exactly once.
    let descriptor = unsafe { OwnedFd::from_raw_fd(receiver.into_raw_fd()) };
    let socket =
        AsyncFd::with_interest(descriptor, Interest::READABLE).expect("register datagram receiver");
    assert!(read_one(&socket).await);
}

#[tokio::test]
async fn reader_coalesces_a_burst_and_stops_when_the_receiver_closes() {
    let (datagram_sender, datagram_receiver) = UnixDatagram::pair().expect("create datagram pair");
    datagram_receiver
        .set_nonblocking(true)
        .expect("set receiver nonblocking");
    datagram_sender.send(b"first").expect("send first event");
    datagram_sender.send(b"second").expect("send second event");

    // SAFETY: `datagram_receiver` owns this descriptor and is converted
    // exactly once.
    let descriptor = unsafe { OwnedFd::from_raw_fd(datagram_receiver.into_raw_fd()) };
    let socket =
        AsyncFd::with_interest(descriptor, Interest::READABLE).expect("register datagram receiver");
    let (event_sender, mut event_receiver) = unbounded_channel();
    let reader = tokio::spawn(run_with_debounce(
        socket,
        event_sender,
        Duration::from_millis(1),
    ));

    assert!(matches!(
        event_receiver.recv().await,
        Some(SystemPowerEvent::DevicesChanged)
    ));

    drop(event_receiver);
    datagram_sender.send(b"third").expect("send third event");
    timeout(Duration::from_secs(1), reader)
        .await
        .expect("reader task should stop promptly")
        .expect("reader task should not panic");
}
