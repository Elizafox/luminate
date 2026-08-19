// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Windows implementation of `transport`: named pipes, restricted to the
//! current user via the same owner-only DACL model as
//! [`crate::windows::secure_storage`] (see
//! [`crate::windows::security_descriptor`]).
//!
//! Peer identity is captured once per accepted connection, mirroring how
//! `SO_PEERCRED` is read once at accept time on Unix: `named_pipe_client_process_id`
//! for the PID and a brief `ImpersonateNamedPipeClient`/`GetTokenInformation`
//! round trip (reverted unconditionally before this function returns) for
//! the SID, both already built and live-validated in
//! [`crate::windows::identity`].

use std::ffi::c_void;
use std::io;
use std::mem;
use std::os::windows::io::AsRawHandle as _;
use std::path::Path;
use std::ptr;
use std::time::Duration;

use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use tokio::time::sleep;

use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;

use crate::transport::{Address, Connection, PeerCredential, PipeAccess};
use crate::windows::identity::{
    LUMINATED_SERVICE_ACCOUNT, account_sid, captured_client_sid, current_process_sid,
    local_system_sid, named_pipe_client_process_id, named_pipe_server_process_id,
    process_sid_and_enabled_groups,
};
use crate::windows::security_descriptor::{
    owner_only_security_attributes, service_pipe_security_attributes,
};

/// How long to wait before retrying a client connect attempt that found
/// every existing pipe instance busy. Pipe instances turn over quickly (a
/// fresh one is created immediately after each accept), so a short retry is
/// enough; this is not a substitute for the caller's own connect timeout.
const PIPE_BUSY_RETRY_DELAY: Duration = Duration::from_millis(20);

/// The `\\.\pipe\` prefix [`pipe_path`] prepends. Windows counts it against
/// the pipe name limit, so it comes out of the budget callers get.
const PIPE_PATH_PREFIX: &str = r"\\.\pipe\";

/// See [`crate::transport::max_address_len`].
///
/// Windows limits the whole `\\.\pipe\<name>` string to 256 characters; the
/// name itself gets whatever the prefix leaves. Unlike the Unix limit this
/// is a documented constant rather than something derivable from a struct,
/// so it is spelled out here.
pub(crate) const MAX_ADDRESS_LEN: usize = 256 - PIPE_PATH_PREFIX.len();

/// See [`crate::transport::Address::from_configured_path`].
///
/// Windows derives its pipe name from the configured path's file stem. The
/// derived name is internal and crosses neither the public Rust API nor the C
/// ABI.
pub(crate) fn address_from_path(path: &Path) -> Address {
    let name = path.file_stem().map_or_else(
        || "luminated".to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    Address::NamedPipe(name)
}

fn pipe_path(name: &str) -> String {
    format!("{PIPE_PATH_PREFIX}{name}")
}

/// Rejects a pipe name too long for Windows to represent, before it becomes
/// an opaque `ERROR_INVALID_NAME` that names neither the offending pipe nor
/// the limit it broke. Mirrors the Unix transport's socket path check.
fn validate_address_len(name: &str) -> io::Result<()> {
    let length = name.chars().count();
    if length > MAX_ADDRESS_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "pipe name is {length} characters, but Windows allows at most \
                 {MAX_ADDRESS_LEN}: {name}"
            ),
        ));
    }

    Ok(())
}

pub(crate) async fn connect(address: &Address) -> io::Result<Connection> {
    let Address::NamedPipe(name) = address else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a Windows client must connect to a named pipe address",
        ));
    };
    validate_address_len(name)?;
    let path = pipe_path(name);

    loop {
        match ClientOptions::new().open(&path) {
            Ok(client) => {
                authenticate_server(&client)?;
                return Ok(Box::new(client));
            }
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY.cast_signed()) => {
                sleep(PIPE_BUSY_RETRY_DELAY).await;
            }
            Err(error) => return Err(error),
        }
    }
}

fn authenticate_server(client: &NamedPipeClient) -> io::Result<()> {
    let pid = named_pipe_server_process_id(client.as_raw_handle())
        .map_err(|error| untrusted_server_error(None, Some(error)))?;
    let authentication = || -> io::Result<bool> {
        let (server_sid, enabled_groups) = process_sid_and_enabled_groups(pid)?;
        if server_sid == current_process_sid()? {
            return Ok(true);
        }

        if server_sid == local_system_sid()? {
            let service_sid = account_sid(LUMINATED_SERVICE_ACCOUNT)?;
            return Ok(enabled_groups.iter().any(|group| group == &service_sid));
        }

        Ok(false)
    };

    match authentication() {
        Ok(true) => Ok(()),
        Ok(false) => Err(untrusted_server_error(Some(pid), None)),
        Err(error) => Err(untrusted_server_error(Some(pid), Some(error))),
    }
}

fn untrusted_server_error(pid: Option<u32>, cause: Option<io::Error>) -> io::Error {
    let process = if let Some(pid) = pid {
        format!("process {pid}")
    } else {
        "of unknown process identity".to_owned()
    };
    let cause = cause.map_or_else(String::new, |error| format!(" ({error})"));

    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "refusing named-pipe server {process}: its token could not be verified as the current \
             user or the {LUMINATED_SERVICE_ACCOUNT} service{cause}"
        ),
    )
}

/// See [`crate::transport::Listener`].
pub(crate) struct Listener {
    name: String,
    access: PipeAccess,
    next: NamedPipeServer,
}

impl Listener {
    pub(crate) fn bind(address: &Address) -> io::Result<Self> {
        Self::bind_with_access(address, PipeAccess::OwnerOnly)
    }

    pub(crate) fn bind_with_access(address: &Address, access: PipeAccess) -> io::Result<Self> {
        let Address::NamedPipe(name) = address else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a Windows listener needs a named pipe address",
            ));
        };
        validate_address_len(name)?;
        let next = create_instance(name, true, &access)?;
        Ok(Self {
            name: name.clone(),
            access,
            next,
        })
    }

    pub(crate) async fn accept(&mut self) -> io::Result<(Connection, PeerCredential)> {
        // A named pipe "listener" is really a chain of single-use server
        // instances: each instance services exactly one client connection.
        // The next instance must exist and be listening *before* this one
        // finishes accepting, or a client that dials in during the gap gets
        // ERROR_PIPE_BUSY with nothing behind it, mirroring the way a Unix
        // listener keeps accepting on the same socket without a handoff gap.
        let following = create_instance(&self.name, false, &self.access)?;
        let incoming = mem::replace(&mut self.next, following);

        incoming.connect().await?;

        let handle = incoming.as_raw_handle();
        let pid = named_pipe_client_process_id(handle).ok();
        let sid = captured_client_sid(handle)?;

        Ok((Box::new(incoming), PeerCredential::Windows { sid, pid }))
    }
}

#[allow(
    unsafe_code,
    reason = "create_with_security_attributes_raw is a Win32-backed tokio API with no safe \
              equivalent that accepts a DACL; the security-attributes pointer is valid for the \
              duration of the call, kept alive by `_guard`."
)]
fn create_instance(name: &str, first: bool, access: &PipeAccess) -> io::Result<NamedPipeServer> {
    let (attributes, _guard) = match access {
        PipeAccess::OwnerOnly => owner_only_security_attributes()?,
        PipeAccess::Service {
            service_sid,
            client_sid,
        } => service_pipe_security_attributes(service_sid, client_sid)?,
    };
    // SAFETY: `attributes` is a valid `SECURITY_ATTRIBUTES` for the
    // duration of this call, and its `lpSecurityDescriptor` allocation is
    // kept alive by `_guard`, which outlives the call.
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .create_with_security_attributes_raw(
                pipe_path(name),
                ptr::addr_of!(attributes).cast::<c_void>().cast_mut(),
            )
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
