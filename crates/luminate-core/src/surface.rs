// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Addressable device surfaces and their constituent elements.

use serde::{Deserialize, Serialize};

use crate::capability::CapabilitySet;
use crate::element::Element;
use crate::util::declare_opaque_id;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// A physical or logical lighting area on a device.
pub struct Surface {
    /// Stable identifier.
    pub id: SurfaceId,

    /// Human-readable name.
    pub name: String,

    /// Presentation and layout kind.
    pub kind: SurfaceKind,

    /// Open semantic hints about the physical form, intended for presentation.
    /// Unknown tags must be preserved and ignored safely by consumers.
    #[serde(default)]
    pub physical_tags: Vec<String>,

    /// Addressable elements in provider-defined order.
    pub elements: Vec<Element>,

    /// Operations supported at this scope.
    pub capabilities: CapabilitySet,

    /// Cosmetic display text, see `Device::notes`. Empty means none.
    pub notes: Vec<String>,

    /// Cosmetic display text, see `Device::warnings`. Empty means none.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Opaque stable identifier for a surface within its device.
pub struct SurfaceId(String);

declare_opaque_id!(SurfaceId, "Creates a surface identifier.");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Physical layout of a surface.
pub enum SurfaceKind {
    /// A surface with no useful spatial layout.
    Opaque,

    /// A single logical lighting zone.
    Zone,

    /// A one-dimensional surface with positions measured from zero to `length`.
    Linear {
        /// Logical surface length; must be finite and positive.
        length: f32,
    },

    /// A sparse two-dimensional surface in logical coordinates.
    Sparse2d {
        /// Logical width; must be finite and positive.
        width: f32,

        /// Logical height; must be finite and positive.
        height: f32,
    },

    /// A discrete row-major grid.
    Matrix {
        /// Number of rows; matrix-cell rows must be smaller than this value.
        rows: u16,

        /// Number of columns; matrix-cell columns must be smaller than this value.
        cols: u16,
    },
}
