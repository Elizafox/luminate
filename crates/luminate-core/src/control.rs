// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Consumer-facing control and selection types shared by clients and the wire
//! implementation. These are semantic API types, not protocol envelopes.

use serde::{Deserialize, Serialize};

use crate::collection::CollectionId;
use crate::target::TargetId;

/// Direction to reconcile hardware and persisted Luminate state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconciliationPolicy {
    /// Write the effective adopted baseline and desired overlays to hardware.
    Restore,

    /// Read exact hardware state and durably promote it as a baseline.
    Adopt,

    /// Avoid writes; read only when the hardware supports observation.
    Leave,
}

/// How a fan-out mutation treats a member that cannot apply the requested
/// state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnsupportedPolicy {
    /// Apply to capable members and leave unsupported members untouched.
    Skip,

    /// Reject unless every current member can apply the state.
    Reject,
}

/// Identifies one concrete target, or a user-created collection's resolved
/// membership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Selector {
    /// One concrete target on a device.
    Target(TargetId),

    /// Every leaf target a collection (transitively) covers.
    Collection(CollectionId),

    /// An explicit, already resolved set of concrete leaf targets.
    ///
    /// This prevents a collection membership change from altering the
    /// mutation set between authorization and daemon execution.
    Targets(Vec<TargetId>),
}
