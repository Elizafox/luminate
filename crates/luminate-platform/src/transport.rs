// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Cross-platform duplex connection establishment for `luminated`'s IPC
//! boundary: a Unix domain socket on Unix, a named pipe on Windows.
//!
//! This module confines the platform split to connection setup and identity
//! capture. `luminate_protocol::framing::{send, receive}` are generic over
//! `AsyncRead`/`AsyncWrite`, so nothing downstream of a [`Connection`] needs
//! to know which transport produced it.

use std::io;
use std::path::{Path, PathBuf};

use tokio::io::{AsyncRead, AsyncWrite};

#[cfg(unix)]
use crate::unix::transport as platform;
#[cfg(windows)]
use crate::windows::transport as platform;

/// A duplex, framed byte stream between two IPC peers, type-erased over the
/// concrete platform transport.
pub type Connection = Box<dyn AsyncReadWrite + Unpin + Send>;

/// Marker trait combining [`AsyncRead`] and [`AsyncWrite`], blanket-implemented
/// for every type that has both. tokio has no built-in combined trait for
/// this, and [`Connection`] needs one to be `dyn`-safe.
pub trait AsyncReadWrite: AsyncRead + AsyncWrite {}
impl<T: AsyncRead + AsyncWrite + ?Sized> AsyncReadWrite for T {}

/// Where to establish, or listen for, a connection.
///
/// Both variants compile on every target; only construction and platform
/// dispatch are `cfg`-gated, matching [`crate::authorization::Principal`]'s
/// (in `luminated`) shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    /// A Unix domain socket filesystem path.
    Unix(PathBuf),
    /// A Windows named pipe, by name only (not the full `\\.\pipe\<name>`
    /// form, which is an implementation detail of the Windows transport).
    NamedPipe(String),
}

/// Access-control posture for a Windows named-pipe listener.
#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipeAccess {
    /// Restrict the pipe to the process's current user. This is the console-mode
    /// default and matches the historical Windows behaviour.
    OwnerOnly,
    /// Admit clients through a resolved Windows principal while retaining full
    /// access for the service identity and administrators.
    Service {
        /// Canonical SID for the service identity.
        service_sid: String,
        /// Canonical SID for the configured client principal.
        client_sid: String,
    },
}

impl Address {
    /// Derives a platform-appropriate address from a configured path.
    #[must_use]
    #[inline]
    pub fn from_configured_path(path: &Path) -> Self {
        platform::address_from_path(path)
    }
}

/// The longest address the current platform's transport accepts, measured
/// the way that platform's [`Address`] variant is itself expressed:
///
/// - Unix: the socket path's length in bytes. `sockaddr_un::sun_path` is a
///   fixed-size array that has to hold a NUL terminator too, so the usable
///   path is one byte shorter than the array: 107 on Linux, 103 on macOS
///   and the BSDs.
/// - Windows: the pipe name's length in characters. Windows caps the whole
///   `\\.\pipe\<name>` string at 256, so the name gets what the prefix
///   leaves.
///
/// [`connect`] and [`Listener::bind`] already enforce this and say so in
/// their errors; this is exposed for callers that would rather construct a
/// fitting address than handle a failure, such as tests choosing a
/// temporary socket directory (see
/// [`crate::test_support::unique_runtime_dir`]).
#[must_use]
#[inline]
pub fn max_address_len() -> usize {
    platform::MAX_ADDRESS_LEN
}

/// The identity captured from a peer at accept time, before it has been
/// interpreted for authorization. `luminated::authorization::Principal` is
/// built from this 1:1; kept separate so this crate never depends on
/// `luminated`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerCredential {
    /// Captured from `SO_PEERCRED` at accept time.
    Unix {
        /// Kernel-supplied user ID of the connected process.
        uid: u32,
        /// Kernel-supplied primary group ID of the connected process.
        gid: u32,
        /// Kernel-supplied process ID, retained for auditing only.
        pid: Option<u32>,
    },
    /// Captured from a Windows named-pipe client via impersonation.
    Windows {
        /// The client's security identifier, in its canonical string form.
        sid: String,
        /// The connected client process's ID, retained for auditing only.
        pid: Option<u32>,
    },
}

/// Connects to `address`, returning a type-erased duplex stream.
///
/// # Errors
///
/// Returns an error if the connection cannot be established.
pub async fn connect(address: &Address) -> io::Result<Connection> {
    platform::connect(address).await
}

/// A bound listener. Each accepted connection carries credentials captured
/// once at accept time rather than once per request.
pub struct Listener(platform::Listener);

impl Listener {
    /// Binds a listener at `address`, applying the platform's private
    /// access-control model (Unix: private socket file and parent
    /// directory; Windows: an owner-only DACL on the pipe).
    ///
    /// # Errors
    ///
    /// Returns an error if the listener cannot be bound, including because
    /// the access-control setup fails.
    pub fn bind(address: &Address) -> io::Result<Self> {
        platform::Listener::bind(address).map(Self)
    }

    /// Binds a Windows named-pipe listener with an explicit access-control
    /// posture.
    ///
    /// # Errors
    ///
    /// Returns an error if the listener cannot be bound, including because a
    /// supplied SID is invalid or the access-control setup fails.
    #[cfg(windows)]
    pub fn bind_with_pipe_access(address: &Address, access: PipeAccess) -> io::Result<Self> {
        platform::Listener::bind_with_access(address, access).map(Self)
    }

    /// Accepts the next connection, returning it alongside the peer
    /// credential captured for it.
    ///
    /// # Errors
    ///
    /// Returns an error if accepting fails. A transient per-connection
    /// failure (e.g. a credential read failure) is not necessarily fatal to
    /// the listener; callers should consult the platform's documented error
    /// kinds before deciding whether to retry.
    pub async fn accept(&mut self) -> io::Result<(Connection, PeerCredential)> {
        self.0.accept().await
    }
}
