// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Named target groups and the members they collect.

use serde::{Deserialize, Serialize};

use crate::capability::CapabilitySet;
use crate::element::ElementId;
use crate::surface::SurfaceId;
use crate::util::declare_opaque_id;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// A named collection of targets within one device.
pub struct Group {
    /// Stable identifier.
    pub id: GroupId,

    /// Human-readable name.
    pub name: String,

    /// Optional human-readable description.
    pub description: Option<String>,

    /// Presentation and layout kind.
    pub kind: GroupKind,

    /// Targets included in the group.
    pub members: Vec<GroupMember>,

    /// Operations supported at this scope.
    pub capabilities: CapabilitySet,

    /// Cosmetic display text, see `crate::device::Device::notes`. Empty
    /// means none.
    pub notes: Vec<String>,

    /// Cosmetic display text, see `crate::device::Device::warnings`. Empty
    /// means none.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Opaque stable identifier for a group within its device.
pub struct GroupId(String);

declare_opaque_id!(GroupId, "Creates a group identifier.");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
/// Origin and purpose of a group.
pub enum GroupKind {
    /// Defined by the hardware itself.
    BuiltIn,

    /// Derived from physical topology.
    Topology,

    /// Defined by the hardware driver or plugin.
    Driver,

    /// Created by a user.
    User,

    /// Created by an application.
    Application,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// A reference to a surface, element, or nested group.
pub enum GroupMember {
    /// One surface on a device.
    Surface(SurfaceId),

    /// One addressable element on a surface.
    Element {
        /// Surface identifier.
        surface: SurfaceId,

        /// Element identifier.
        element: ElementId,
    },

    /// A named group on a device.
    Group(GroupId),
}
