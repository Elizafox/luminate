// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Fake UDP sockets and clocks for deterministic transport tests.

use super::{SocketAddr, Transport, io};
use std::cell::RefCell;

#[derive(Default)]
pub(crate) struct RecordingTransport {
    pub(crate) sent: RefCell<Vec<(SocketAddr, Vec<u8>)>>,
}

impl Transport for RecordingTransport {
    fn send(&self, address: SocketAddr, payload: &[u8]) -> io::Result<()> {
        self.sent.borrow_mut().push((address, payload.to_vec()));
        Ok(())
    }
}

pub(crate) struct UnavailableTransport;

impl Transport for UnavailableTransport {
    fn send(&self, _address: SocketAddr, _payload: &[u8]) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "device unavailable",
        ))
    }
}
