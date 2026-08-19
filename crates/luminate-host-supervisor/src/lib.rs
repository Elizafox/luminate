// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Shared supervisor ↔ host IPC primitives for `luminated`'s plugin and
//! policy-provider child processes.
//!
//! Both stacks re-exec `luminated` itself into a disposable child process
//! and exchange length-prefixed CBOR frames with it. Their transports remain
//! separate: the synchronous plugin-host transport supports concurrent device
//! mutations, while the asynchronous policy-host transport permits one
//! in-flight request at a time. Combining them would serialize plugin work.
//!
//! This crate provides the shared frame format and size cap ([`framing`] and
//! [`sync_io`]), version-negotiating handshake ([`handshake`]), and bounded
//! respawn backoff ([`RespawnBackoff`]). Host-specific symbol validation and
//! process spawning stay with their respective transports.

/// Exact compatibility version for the supervisor ↔ host handshake and framed
/// command protocol.
pub const HOST_SUPERVISOR_PROTOCOL_VERSION: u32 = 5;

pub mod backoff;
pub mod framing;
pub mod handshake;
pub mod shm;
pub mod sync_io;

pub use backoff::RespawnBackoff;
pub use framing::{FramingError, MAX_FRAME_LEN};
pub use handshake::{Compatibility, HostHello, SupervisorHello};
pub use shm::{
    NodeCreationError, ShmServiceNames, client_service_names, create_node, service_names,
};
