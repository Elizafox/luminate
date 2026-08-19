// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Individually addressable elements and their geometry within a surface.

use serde::{Deserialize, Serialize};

use crate::capability::CapabilitySet;
use crate::util::declare_opaque_id;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// One independently addressable part of a surface.
pub struct Element {
    /// Stable identifier.
    pub id: ElementId,

    /// Human-readable name.
    pub name: Option<String>,

    /// Presentation and layout kind.
    pub kind: ElementKind,

    /// Optional layout coordinates.
    pub geometry: Option<ElementGeometry>,

    /// Open semantic hints about this individual physical object, intended for
    /// presentation. Unknown tags must be preserved and ignored safely by
    /// consumers. An empty list means the form is unknown.
    #[serde(default)]
    pub physical_tags: Vec<String>,

    /// Operations supported at this scope.
    pub capabilities: CapabilitySet,

    /// Cosmetic display text, see `crate::device::Device::notes`. Empty
    /// means none.
    pub notes: Vec<String>,

    /// Cosmetic display text, see `crate::device::Device::warnings`. Empty
    /// means none.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Presentation hint for an addressable element.
pub enum ElementKind {
    /// A keyboard key.
    Key,

    /// An individual LED.
    Led,

    /// A single logical lighting zone.
    Zone,

    /// An illuminated logo.
    Logo,

    /// One segment of a lighting ring.
    RingSegment,
}

/// `Rect`/`Point`/`Linear` coordinates are normalized `[0, 1]` relative to
/// the owning surface's own bounding box (not pixels, not physical units),
/// so a consumer can lay a surface out at any render size without needing
/// per-device unit conversion. When no element in a surface has geometry,
/// the surface's `elements` array order is the intended visual/physical
/// order (e.g. reading order for a keyboard, wiring order for a strip).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ElementGeometry {
    /// A normalized rectangle.
    Rect {
        /// Normalized horizontal position.
        x: f32,

        /// Normalized vertical position.
        y: f32,

        /// Normalized width.
        w: f32,

        /// Normalized height.
        h: f32,
    },

    /// A normalized point.
    Point {
        /// Normalized horizontal position.
        x: f32,

        /// Normalized vertical position.
        y: f32,
    },

    /// A normalized position on a line.
    Linear {
        /// Normalized position along the surface.
        position: f32,
    },

    /// A discrete cell in a `SurfaceKind::Matrix { rows, cols }` surface,
    /// addressed by integer row/column rather than a normalized float
    /// position. This is the natural fit for grid-addressed hardware (e.g.
    /// keyboards), where positions are inherently discrete.
    MatrixCell {
        /// Zero-based matrix row.
        row: u16,

        /// Zero-based matrix column.
        col: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Opaque stable identifier for an element within its surface.
pub struct ElementId(String);

declare_opaque_id!(ElementId, "Creates an element identifier.");

#[cfg(test)]
mod tests {
    use super::*;

    fn element() -> Element {
        Element {
            id: ElementId::new("led"),
            name: None,
            kind: ElementKind::Led,
            geometry: None,
            physical_tags: vec!["shape:round".to_owned(), "position:left".to_owned()],
            capabilities: CapabilitySet::default(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn physical_tags_round_trip_in_order() {
        let value = serde_json::to_value(element()).expect("serialize element");
        let decoded: Element = serde_json::from_value(value).expect("deserialize element");

        assert_eq!(decoded.physical_tags, ["shape:round", "position:left"]);
    }

    #[test]
    fn missing_physical_tags_default_to_empty() {
        let mut value = serde_json::to_value(element()).expect("serialize element");
        value
            .as_object_mut()
            .expect("element serializes as an object")
            .remove("physical_tags");
        let decoded: Element = serde_json::from_value(value).expect("deserialize older element");

        assert!(decoded.physical_tags.is_empty());
    }
}
