// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Canonical identifiers for addressable devices, surfaces, groups, and elements.

use serde::{Deserialize, Serialize};

use crate::device::DeviceId;
use crate::element::ElementId;
use crate::group::GroupId;
use crate::surface::SurfaceId;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// A concrete mutation target: either a device itself, or a piece of
/// physical hardware that is part of one (a surface, element, or
/// device-scoped group). Every variant has exactly one owning device; see
/// [`Self::device_id`].
pub enum TargetId {
    /// An entire device.
    Device(DeviceId),

    /// One surface on a device.
    Surface {
        /// Device identifier.
        device: DeviceId,

        /// Surface identifier.
        surface: SurfaceId,
    },

    /// One addressable element on a surface.
    Element {
        /// Device identifier.
        device: DeviceId,

        /// Surface identifier.
        surface: SurfaceId,

        /// Element identifier.
        element: ElementId,
    },

    /// A named group on a device.
    Group {
        /// Device identifier.
        device: DeviceId,

        /// Group identifier.
        group: GroupId,
    },
}

impl TargetId {
    /// Targets an entire device.
    #[must_use]
    pub fn device(device: impl Into<String>) -> Self {
        Self::Device(DeviceId::new(device))
    }

    /// Targets one surface on a device.
    #[must_use]
    pub fn surface(device: impl Into<String>, surface: impl Into<String>) -> Self {
        Self::Surface {
            device: DeviceId::new(device),
            surface: SurfaceId::new(surface),
        }
    }

    /// Targets one element on a device surface.
    #[must_use]
    pub fn element(
        device: impl Into<String>,
        surface: impl Into<String>,
        element: impl Into<String>,
    ) -> Self {
        Self::Element {
            device: DeviceId::new(device),
            surface: SurfaceId::new(surface),
            element: ElementId::new(element),
        }
    }

    /// Targets one named group on a device.
    #[must_use]
    pub fn group(device: impl Into<String>, group: impl Into<String>) -> Self {
        Self::Group {
            device: DeviceId::new(device),
            group: GroupId::new(group),
        }
    }

    /// Builds a `TargetId` from device/surface/element/group components,
    /// enforcing the combination rules shared by every place that parses a
    /// target from separate parts (the CLI's `--device`/`--surface`/
    /// `--element`/`--group` flags, the C FFI's equivalent parameters): a
    /// group cannot be combined with a surface or element, and an element
    /// requires a surface.
    ///
    /// # Errors
    ///
    /// Returns `Err` with a static, human-readable reason if the
    /// combination of components doesn't form a valid target; see above.
    pub fn from_parts(
        device: impl Into<String>,
        surface: Option<impl Into<String>>,
        element: Option<impl Into<String>>,
        group: Option<impl Into<String>>,
    ) -> Result<Self, &'static str> {
        if group.is_some() && (surface.is_some() || element.is_some()) {
            return Err("group target cannot be combined with a surface or element target");
        }

        if element.is_some() && surface.is_none() {
            return Err("element target requires a surface identifier");
        }

        let device = DeviceId::new(device);

        Ok(match (surface, element, group) {
            (_, _, Some(group)) => Self::Group {
                device,
                group: GroupId::new(group),
            },
            (Some(surface), Some(element), None) => Self::Element {
                device,
                surface: SurfaceId::new(surface),
                element: ElementId::new(element),
            },
            (Some(surface), None, None) => Self::Surface {
                device,
                surface: SurfaceId::new(surface),
            },
            (None, None, None) => Self::Device(device),
            (None, Some(_), None) => {
                return Err("element target requires a surface identifier");
            }
        })
    }

    /// Returns the device that owns this target.
    #[must_use]
    pub const fn device_id(&self) -> &DeviceId {
        match self {
            Self::Device(device)
            | Self::Surface { device, .. }
            | Self::Element { device, .. }
            | Self::Group { device, .. } => device,
        }
    }
}

#[cfg(test)]
#[path = "target_tests.rs"]
mod tests;
