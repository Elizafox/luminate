// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! The current process's own OS identity, for comparing against a captured
//! client credential.
//!
//! Only [`daemon_own_uid`] lives here cross-platform. Windows SID capture
//! needs several more entry points with no Unix equivalent (impersonation,
//! named-pipe client capture), so that surface stays directly named at
//! [`crate::windows::identity`] rather than being folded in behind a
//! same-named counterpart here.

#[cfg(unix)]
pub use crate::unix::identity::{daemon_own_uid, uid_for_user};
