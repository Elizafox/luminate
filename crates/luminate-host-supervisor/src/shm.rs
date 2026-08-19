// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Deterministic iceoryx2 service naming and node creation.
//!
//! Service names are derived independently by `luminated` and its
//! plugin-host child from the same plugin name and caller-provided target
//! identifier. They are never negotiated or transmitted.
//!
//! Inputs are stable strings so this crate does not depend on plugin types.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::PathBuf;

use iceoryx2::config::Config;
use iceoryx2::node::{Node, NodeBuilder, NodeCreationFailure};
use iceoryx2::prelude::{Path as IceoryxPath, SemanticString as _, SemanticStringError};
use iceoryx2::service::ipc_threadsafe;
use iceoryx2::service::service_name::{ServiceName, ServiceNameError};

/// Service names for a shared-memory frame stream.
///
/// One names the publish-subscribe service carrying pixel data; the other
/// names the event service used to notify the subscriber that a new
/// sample is ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShmServiceNames {
    pub publish_subscribe: ServiceName,
    pub event: ServiceName,
}

/// Fixed namespace prefix for every shared-memory frame service, so these
/// names can't collide with any other iceoryx2 service `luminated` might
/// use for an unrelated purpose.
const NAMESPACE: &str = "luminate/shm-frame/v1";

/// Fixed namespace prefix for the client → daemon leg's shared-memory frame
/// services (see [`client_service_names`]). Distinct from [`NAMESPACE`] so
/// the two legs' services, which are otherwise both keyed by the same
/// target identifier, can never collide.
const CLIENT_NAMESPACE: &str = "luminate/shm-client-frame/v1";

/// Derives the service names for one shared-memory frame stream.
///
/// Inputs are escaped independently before being joined, preventing embedded
/// separators from creating collisions.
///
/// # Errors
///
/// Returns [`ServiceNameError::ExceedsMaximumLength`] if the escaped,
/// namespaced name would exceed iceoryx2's maximum service name length.
/// Pathologically long plugin or target identifiers are the only realistic
/// trigger; every other `ServiceNameError` variant is structurally
/// unreachable here because [`NAMESPACE`] guarantees the name is always
/// non-empty and never begins with iceoryx2's own reserved `iox2://`
/// prefix.
pub fn service_names(
    plugin_name: &str,
    target_path: &str,
) -> Result<ShmServiceNames, ServiceNameError> {
    let mut base = String::from(NAMESPACE);
    base.push('/');
    escape_into(plugin_name, &mut base);
    base.push('/');
    escape_into(target_path, &mut base);

    names_from_base(&base)
}

/// Derives the service names for one client → daemon shared-memory frame
/// stream.
///
/// This leg is keyed only by target because the daemon is its sole creator.
///
/// # Errors
///
/// Returns [`ServiceNameError::ExceedsMaximumLength`] on a pathologically
/// long `target_path`; see [`service_names`]'s docs for why every other
/// `ServiceNameError` variant is structurally unreachable here.
pub fn client_service_names(target_path: &str) -> Result<ShmServiceNames, ServiceNameError> {
    let mut base = String::from(CLIENT_NAMESPACE);
    base.push('/');
    escape_into(target_path, &mut base);

    names_from_base(&base)
}

/// Why an iceoryx2 [`Node`] could not be created.
#[derive(Debug, thiserror::Error)]
pub enum NodeCreationError {
    /// The directory iceoryx2 keeps its own management files in could not
    /// be created. Carries the attempted path so configuration and permission
    /// failures can be diagnosed.
    #[error("creating the iceoryx2 root path {path}")]
    RootPath {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// No per-user directory could be found to host the root path. Windows
    /// only in practice, where it means the local application-data known
    /// folder could not be resolved; see [`create_node`] for why this build
    /// wants a per-user location.
    #[error("no per-user application data directory is available for the iceoryx2 root path")]
    NoPerUserBase,

    /// The chosen root path is not expressible as an iceoryx2 path, whose
    /// own `Path` type is fixed-capacity. An unusually deep user profile
    /// directory is the only realistic trigger.
    #[error("{path} cannot be used as an iceoryx2 root path")]
    RootPathRejected {
        path: PathBuf,
        #[source]
        source: SemanticStringError,
    },

    /// iceoryx2 itself declined to create the node.
    #[error("creating iceoryx2 node")]
    Node(#[source] NodeCreationFailure),
}

/// Creates the iceoryx2 [`Node`] that every shared-memory frame stream in
/// this workspace hangs off, after making sure iceoryx2's configured root
/// path exists.
///
/// iceoryx2 does not create its configured root. This matters on Windows,
/// where its management storage is file-backed and node creation otherwise
/// fails with an opaque internal error.
///
/// On Unix the root path is left exactly where iceoryx2 puts it. On
/// Windows it is redirected under the current user's profile; see
/// [`node_config`] for that reasoning, which is a security one.
///
/// The root keeps inherited permissions: Unix uses iceoryx2's shared root,
/// while the Windows root is private through `%LOCALAPPDATA%` inheritance.
///
/// # Errors
///
/// Returns [`NodeCreationError::NoPerUserBase`] or
/// [`NodeCreationError::RootPathRejected`] if a per-user root path cannot
/// be determined, [`NodeCreationError::RootPath`] if it cannot be created,
/// or [`NodeCreationError::Node`] if iceoryx2 declines the node.
pub fn create_node() -> Result<Node<ipc_threadsafe::Service>, NodeCreationError> {
    let config = node_config()?;

    let root_path = as_std_path(config.global.root_path());
    fs::create_dir_all(&root_path).map_err(|source| NodeCreationError::RootPath {
        path: root_path,
        source,
    })?;

    NodeBuilder::new()
        .config(&config)
        .create::<ipc_threadsafe::Service>()
        .map_err(NodeCreationError::Node)
}

/// The iceoryx2 configuration every node in this workspace is built from.
///
/// On Unix this is iceoryx2's own global configuration, untouched. Its
/// default root path is `/tmp/iceoryx2/`, which is not private, but `/tmp`
/// is sticky, so another local user can neither delete nor replace what we
/// put there, and the packaged Linux deployment genuinely does span users,
/// with the daemon running as `luminated` and clients as whoever is logged
/// in. A per-user root path would break that outright, so the shared
/// location stays.
#[cfg(unix)]
#[allow(
    clippy::unnecessary_wraps,
    reason = "signature is shared with the Windows branch below, which genuinely can fail; only \
              that branch has anything to report."
)]
fn node_config() -> Result<Config, NodeCreationError> {
    Ok(Config::global_config().clone())
}

/// The iceoryx2 configuration every node in this workspace is built from.
///
/// Windows gets a per-user root path under `%LOCALAPPDATA%` instead of
/// iceoryx2's default `C:\Temp\iceoryx2\`. `C:\Temp` inherits `C:\`'s DACL,
/// granting
/// `Authenticated Users` Modify on the directory and, through an
/// inherit-only ACE, on everything created inside it. Any local account
/// could therefore pre-create the root path, or delete and replace the node
/// and service registries a running daemon depends on. `%LOCALAPPDATA%` is
/// private to its user by inheritance and needs no ACL work from us.
///
/// The cost is that shared-memory streaming no longer spans user accounts
/// on Windows. The daemon ↔ plugin-host leg already runs under one account,
/// and cross-user Windows deployments are not currently supported. Such a
/// deployment would require an explicitly ACL'd directory under
/// `%ProgramData%`, not a return to `C:\Temp`.
#[cfg(windows)]
fn node_config() -> Result<Config, NodeCreationError> {
    let base = luminate_platform::windows::current_user_local_data_directory()
        .map_err(|_| NodeCreationError::NoPerUserBase)?;
    let root_path = PathBuf::from(base).join("luminate").join("iceoryx2");

    let encoded =
        IceoryxPath::new(root_path.display().to_string().as_bytes()).map_err(|source| {
            NodeCreationError::RootPathRejected {
                path: root_path.clone(),
                source,
            }
        })?;

    let mut config = Config::global_config().clone();
    config.global.set_root_path(&encoded);
    Ok(config)
}

/// An iceoryx2 [`IceoryxPath`] as a std [`PathBuf`].
///
/// iceoryx2 stores paths as raw bytes in its own fixed-capacity string
/// type. Every path this module ever puts there originates as a Rust
/// `String`, so the round trip cannot lose anything real.
fn as_std_path(path: &IceoryxPath) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(path.as_bytes()).into_owned())
}

/// Finalizes a fully-escaped `base` name into the pub/sub and event name pair
/// every stream needs. The event service is the base name with a `/notify`
/// suffix, so the two never collide.
///
/// # Errors
///
/// Returns [`ServiceNameError::ExceedsMaximumLength`] if either name exceeds
/// iceoryx2's maximum service name length; see [`service_names`] for why no
/// other variant is reachable.
fn names_from_base(base: &str) -> Result<ShmServiceNames, ServiceNameError> {
    let event = format!("{base}/notify");

    Ok(ShmServiceNames {
        publish_subscribe: ServiceName::new(base)?,
        event: ServiceName::new(&event)?,
    })
}

/// Appends `raw`'s percent-escaped form to `out`. Only
/// `[A-Za-z0-9._-]` bytes pass through unescaped.
fn escape_into(raw: &str, out: &mut String) {
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(byte as char),
            _ => {
                // Formatting into a String is infallible.
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
}

#[cfg(test)]
#[path = "shm_tests.rs"]
mod tests;
