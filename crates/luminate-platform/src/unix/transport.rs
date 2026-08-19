// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Unix implementation of `transport`: Unix domain sockets.
//!
//! `SOCKET_MODE` (mode `0660`, group-writable) is a deliberate
//! multi-principal trust decision (which local group may connect to the
//! daemon), distinct from [`crate::secure_storage`]'s current-user-only
//! model. Windows service mode mirrors this boundary with a local security
//! group in the named-pipe DACL; Windows console mode remains owner-only.

use std::fs;
use std::io;
use std::mem::offset_of;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::net::{UnixListener, UnixStream};
use tokio::time::sleep;

use crate::transport::{Address, Connection, PeerCredential};

/// Group-writable: the daemon's local multi-principal trust boundary. See
/// the module doc comment.
const SOCKET_MODE: u32 = 0o660;
/// Private to the daemon's own user: the socket's parent runtime directory.
const PRIVATE_RUNTIME_DIR_MODE: u32 = 0o700;

/// See [`crate::transport::max_address_len`].
///
/// Derived from `sockaddr_un` rather than hardcoded per platform, because
/// the sizes genuinely differ (108 bytes on Linux, 104 on macOS and the
/// BSDs) and an off-by-one here is invisible until a path lands in exactly
/// the wrong range. `sun_path` is the final field on every supported
/// platform, so the array's size is what the struct has left after it; one
/// byte of that belongs to the NUL terminator, never to the path.
pub(crate) const MAX_ADDRESS_LEN: usize =
    size_of::<libc::sockaddr_un>() - offset_of!(libc::sockaddr_un, sun_path) - 1;

pub(crate) fn address_from_path(path: &Path) -> Address {
    Address::Unix(path.to_owned())
}

pub(crate) async fn connect(address: &Address) -> io::Result<Connection> {
    let Address::Unix(path) = address else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a Unix client must connect to a Unix domain socket address",
        ));
    };
    validate_address_len(path)?;
    let stream = UnixStream::connect(path).await?;
    Ok(Box::new(stream))
}

fn other_error(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

/// Rejects a socket path too long for `sockaddr_un`, before the kernel gets
/// a chance to fail it as a bare `EINVAL`/`ENAMETOOLONG` that names neither
/// the offending path nor the limit it broke. The socket path is
/// administrator-configurable (`LUMINATED_SOCKET_PATH`), so this is a
/// diagnostic a real deployment can hit, not only a test concern.
fn validate_address_len(path: &Path) -> io::Result<()> {
    let length = path.as_os_str().as_bytes().len();
    if length > MAX_ADDRESS_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "socket path is {length} bytes, but this platform allows at most \
                 {MAX_ADDRESS_LEN}: {}",
                path.display()
            ),
        ));
    }

    Ok(())
}

fn socket_parent(path: &Path) -> io::Result<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| other_error("socket path must have a parent directory"))
}

fn prepare_socket_path(path: &Path) -> io::Result<()> {
    let parent = socket_parent(path)?;

    if !parent.exists() {
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(PRIVATE_RUNTIME_DIR_MODE))?;
    }

    let metadata = fs::metadata(parent)?;
    if !metadata.is_dir() {
        return Err(other_error(format!(
            "socket parent {} is not a directory",
            parent.display()
        )));
    }
    if metadata.mode() & 0o022 != 0 {
        return Err(other_error(format!(
            "socket directory {} must not be writable by group or others (mode {:04o})",
            parent.display(),
            metadata.mode() & 0o7777
        )));
    }

    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            fs::remove_file(path)?;
        }
        Ok(_) => {
            return Err(other_error(format!(
                "refusing to replace non-socket filesystem node at {}",
                path.display()
            )));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    Ok(())
}

fn validate_socket_parent_owner(path: &Path) -> io::Result<()> {
    let parent = socket_parent(path)?;
    let parent_uid = fs::metadata(parent)?.uid();
    let socket_uid = fs::symlink_metadata(path)?.uid();
    if parent_uid != socket_uid {
        return Err(other_error(format!(
            "socket directory {} must be owned by daemon uid {socket_uid} (found {parent_uid})",
            parent.display()
        )));
    }
    Ok(())
}

fn set_socket_permissions(path: &Path) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(SOCKET_MODE))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SocketIdentity {
    device: u64,
    inode: u64,
}

impl SocketIdentity {
    fn from_path(path: &Path) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_socket() {
            return Err(other_error(format!("{} is not a socket", path.display())));
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

/// Removes the socket file at `path`, but only if it is still the exact
/// socket this listener created, not a stale one left by a previous run,
/// nor a replacement bound by a different process in the meantime.
fn remove_owned_socket(path: &Path, expected: SocketIdentity) -> io::Result<()> {
    match SocketIdentity::from_path(path) {
        Ok(actual) if actual == expected => fs::remove_file(path),
        Ok(_) => Err(other_error(format!(
            "refusing to remove replaced socket path {}",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Returns whether an accept failure reflects temporary resource pressure
/// rather than a broken listener.
///
/// Tokio already handles `EAGAIN` and `EWOULDBLOCK`.
fn is_transient_accept_error(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EMFILE | libc::ENFILE | libc::ENOBUFS | libc::ENOMEM)
    )
}

/// See [`crate::transport::Listener`].
pub(crate) struct Listener {
    socket: UnixListener,
    path: PathBuf,
    identity: SocketIdentity,
}

impl Listener {
    pub(crate) fn bind(address: &Address) -> io::Result<Self> {
        let Address::Unix(path) = address else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a Unix listener needs a Unix domain socket address",
            ));
        };

        validate_address_len(path)?;
        prepare_socket_path(path)?;
        let socket = UnixListener::bind(path)?;
        let identity = SocketIdentity::from_path(path)?;
        let bound = Self {
            socket,
            path: path.clone(),
            identity,
        };
        set_socket_permissions(path)?;
        validate_socket_parent_owner(path)?;

        Ok(bound)
    }

    pub(crate) async fn accept(&mut self) -> io::Result<(Connection, PeerCredential)> {
        loop {
            match self.socket.accept().await {
                Ok((stream, _)) => match stream.peer_cred() {
                    Ok(cred) => {
                        let credential = PeerCredential::Unix {
                            uid: cred.uid(),
                            gid: cred.gid(),
                            pid: cred.pid().and_then(|pid| u32::try_from(pid).ok()),
                        };
                        return Ok((Box::new(stream), credential));
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "failed to read peer credentials; rejecting client");
                    }
                },
                Err(error) if is_transient_accept_error(&error) => {
                    tracing::warn!(error = %error, "transient socket accept failure");
                    sleep(Duration::from_millis(100)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Err(error) = remove_owned_socket(&self.path, self.identity) {
            tracing::warn!(
                path = %self.path.display(),
                error = %error,
                "failed to remove socket file"
            );
        }
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
