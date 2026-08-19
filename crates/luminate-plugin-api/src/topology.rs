// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! A plugin's device topology, the mutation and read-back request/response
//! model exchanged over it, and the hardware claims that resolve ownership
//! conflicts between plugins.

use std::fmt;
use std::ptr;

use luminate_core::appearance_slot::AppearanceSlotValue;
use luminate_core::capability::CapabilitySet;
use luminate_core::device::DeviceCategory;
use luminate_core::effect::Effect;
use luminate_core::element::{ElementGeometry, ElementKind};
use luminate_core::frame::FrameEnvelope;
use luminate_core::group::GroupKind;
use luminate_core::state;
use luminate_core::surface::SurfaceKind;
use serde::{Deserialize, Serialize};

/// Canonical physical targets and facets the daemon asks a plugin to read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginReadRequest {
    pub targets: Vec<PluginReadTarget>,
}

/// Facets requested at one device, surface, or element target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginReadTarget {
    pub target: PluginTarget,
    pub facets: Vec<state::StateFacetKind>,
}

/// A plugin reports values only; confidence is assigned by the daemon from
/// the facet's advertised readback fidelity after validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginFacetObservation {
    pub target: PluginTarget,
    pub value: state::FacetValue,
}

/// A target-scoped read failure that does not discard other successful reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginReadError {
    pub target: PluginTarget,
    pub diagnostic: String,
}

/// One bounded hardware snapshot, allowing successful and failed targets to coexist.
///
/// `observations` carries no ordering contract: a plugin may return facets in
/// any order, and the daemon looks each one up by target and
/// `StateFacetKind` rather than by position.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginStateSnapshot {
    pub observations: Vec<PluginFacetObservation>,
    pub errors: Vec<PluginReadError>,
}

/// Serializes a state snapshot into a caller-provided ABI buffer.
///
/// The required size is returned even when the buffer is too small, allowing
/// the host to reject the snapshot without asking hardware to read twice.
///
/// # Safety
///
/// `output` must point to `output_capacity` writable bytes when capacity is
/// nonzero.
pub unsafe fn write_state_snapshot(
    output: *mut u8,
    output_capacity: usize,
    snapshot: &PluginStateSnapshot,
) -> usize {
    let mut payload = Vec::new();
    if ciborium::into_writer(snapshot, &mut payload).is_err() {
        return usize::MAX;
    }
    if payload.len() <= output_capacity && !output.is_null() {
        // SAFETY: the caller supplies a writable buffer of the declared size.
        unsafe { ptr::copy_nonoverlapping(payload.as_ptr(), output, payload.len()) };
    }
    payload.len()
}

/// One mutation dispatched to a plugin's `apply_update_cbor`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginUpdate {
    pub target: PluginTarget,
    pub operation: PluginUpdateOperation,
}

/// One frame dispatched to a plugin's `frame_upload_cbor`, mirroring how
/// [`PluginUpdate`] bundles its target with its payload rather than passing
/// the target as a separate raw ABI parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginFrameUpload {
    pub target: PluginTarget,
    pub envelope: FrameEnvelope,
}

/// A batch of updates to apply together, in order. A plugin that implements
/// `apply_batch_cbor` may coalesce hardware work across the whole batch
/// (e.g. rebuild an on-device table once instead of once per update)
/// instead of treating each entry independently. There is no atomicity
/// guarantee across entries: a plugin may accept some and reject others.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginUpdateBatch {
    pub updates: Vec<PluginUpdate>,
}

/// Which part of a plugin's own topology an update addresses. Field values
/// are the same `id`s the plugin gave in its `topology_cbor` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PluginTarget {
    Device {
        device: String,
    },
    Surface {
        device: String,
        surface: String,
    },
    Element {
        device: String,
        surface: String,
        element: String,
    },
    Group {
        device: String,
        group: String,
    },
}

impl PluginTarget {
    /// Returns the plugin-defined identifier of the owning device.
    #[must_use]
    pub fn device_id(&self) -> &str {
        match self {
            Self::Device { device }
            | Self::Surface { device, .. }
            | Self::Element { device, .. }
            | Self::Group { device, .. } => device,
        }
    }
}

impl fmt::Display for PluginTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device { device } => formatter.write_str(device),
            Self::Surface { device, surface } => write!(formatter, "{device}/{surface}"),
            Self::Element {
                device,
                surface,
                element,
            } => write!(formatter, "{device}/{surface}/{element}"),
            Self::Group { device, group } => write!(formatter, "{device}/group:{group}"),
        }
    }
}

/// The requested mutation, represented with the same serializable
/// `luminate-core` types used by the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PluginUpdateOperation {
    SetEffect { effect: Effect },
    SetAppearanceSlots { values: Vec<AppearanceSlotValue> },
    SetBrightness { value: u32 },
    Clear,
    SaveCurrent,
}

impl PluginUpdateOperation {
    /// Returns the stable operation name used in diagnostics and logs.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::SetEffect { .. } => "set-effect",
            Self::SetAppearanceSlots { .. } => "set-appearance-slots",
            Self::SetBrightness { .. } => "set-brightness",
            Self::Clear => "clear",
            Self::SaveCurrent => "save-current",
        }
    }
}

impl fmt::Display for PluginUpdateOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SetEffect { effect } => {
                let effect = serde_json::to_string(effect).map_err(|_| fmt::Error)?;
                formatter.write_str(&effect)
            }
            Self::SetAppearanceSlots { values } => {
                let values = serde_json::to_string(values).map_err(|_| fmt::Error)?;
                formatter.write_str(&values)
            }
            Self::SetBrightness { value } => write!(formatter, "brightness={value}"),
            Self::Clear => formatter.write_str("clear"),
            Self::SaveCurrent => formatter.write_str("save-current"),
        }
    }
}

/// Hardware transport used by a physical-resource claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HardwareBus {
    Usb,
    Hid,
    I2c,
    Platform,
    Network,
}

/// Whether providers may intentionally share one hardware control domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClaimExclusivity {
    /// At most one provider may control the resource.
    Exclusive,
    /// Multiple providers may coexist only when every claimant is shared.
    Shared,
}

/// A plugin's claim on one physical hardware control domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardwareClaim {
    pub bus: HardwareBus,
    pub physical_identity: String,
    pub control_domain: String,
    pub exclusivity: ClaimExclusivity,
}

/// One physical device a plugin owns, as reported by `topology_cbor`. `id`
/// must be unique across the whole daemon topology, not just this plugin.
/// The daemon rejects a plugin whose topology collides with another
/// already-loaded plugin's IDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceDescriptor {
    pub id: String,
    pub name: String,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub surfaces: Vec<SurfaceDescriptor>,
    pub groups: Vec<GroupDescriptor>,
    pub capabilities: CapabilitySet,

    /// Physical resources controlled by this logical device. Claims are
    /// daemon-internal routing metadata and are not exposed to clients.
    #[serde(default)]
    pub claims: Vec<HardwareClaim>,

    /// GUI presentation hint, see `luminate_core::device::DeviceCategory`.
    /// `None` if the plugin doesn't have (or care to report) one.
    pub category: Option<DeviceCategory>,

    /// Open semantic hints about the physical form of the device as a whole,
    /// intended for presentation. Unknown tags must be preserved and ignored
    /// safely by consumers. An empty list means the form is unknown.
    #[serde(default)]
    pub physical_tags: Vec<String>,

    /// Whether this device is physically attached to (part of) the machine
    /// running `luminated`. This is a fixed, plugin-declared fact, the sole
    /// signal the daemon uses to compute an ACL-facing resource's
    /// `host_attached` (`luminated::authorization::Resource`). A plugin
    /// author must state it explicitly for every device.
    pub host_attached: bool,

    /// Cosmetic display text, see `luminate_core::device::Device::notes`.
    /// Empty means none.
    pub notes: Vec<String>,

    /// Cosmetic display text, see `luminate_core::device::Device::warnings`.
    /// Empty means none.
    pub warnings: Vec<String>,
}

/// A named region of a device (a zoned panel, a strip, a single-zone area),
/// containing the finer-grained `ElementDescriptor`s. `id` must be unique
/// within its owning device; `name` must be unique within its scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceDescriptor {
    pub id: String,
    pub name: String,
    pub kind: SurfaceKind,
    /// Open semantic hints about the physical form, intended for presentation.
    /// Tags should be namespaced when they are provider-specific. Unknown tags
    /// must be preserved and ignored safely by consumers.
    #[serde(default)]
    pub physical_tags: Vec<String>,
    pub elements: Vec<ElementDescriptor>,
    pub capabilities: CapabilitySet,

    /// Cosmetic display text, see `luminate_core::device::Device::notes`.
    /// Empty means none.
    pub notes: Vec<String>,

    /// Cosmetic display text, see `luminate_core::device::Device::warnings`.
    /// Empty means none.
    pub warnings: Vec<String>,
}

/// The finest-grained addressable lighting unit (a key, a zone, a logo,
/// ...). `id` must be unique within its owning surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementDescriptor {
    pub id: String,
    pub name: Option<String>,
    pub kind: ElementKind,
    pub geometry: Option<ElementGeometry>,

    /// Open semantic hints about this individual physical object, intended for
    /// presentation. Tags should be namespaced when they are provider-specific.
    /// Unknown tags must be preserved and ignored safely by consumers.
    #[serde(default)]
    pub physical_tags: Vec<String>,

    pub capabilities: CapabilitySet,

    /// Cosmetic display text, see `luminate_core::device::Device::notes`.
    /// Empty means none.
    pub notes: Vec<String>,

    /// Cosmetic display text, see `luminate_core::device::Device::warnings`.
    /// Empty means none.
    pub warnings: Vec<String>,
}

/// A named, possibly cross-surface collection of surfaces/elements/other
/// groups within a device, for convenience targeting (e.g. "all gamer
/// keys"). Every `GroupMemberDescriptor` reference must resolve to a real
/// surface/element/group on the same device, or the plugin fails to load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupDescriptor {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub kind: GroupKind,
    pub members: Vec<GroupMemberDescriptor>,
    pub capabilities: CapabilitySet,

    /// Cosmetic display text, see `luminate_core::device::Device::notes`.
    /// Empty means none.
    pub notes: Vec<String>,

    /// Cosmetic display text, see `luminate_core::device::Device::warnings`.
    /// Empty means none.
    pub warnings: Vec<String>,
}

/// One member of a `GroupDescriptor`, referenced by ID within the same
/// device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GroupMemberDescriptor {
    Surface(String),
    Element { surface: String, element: String },
    Group(String),
}

/// Serializes a plugin's device topology into the CBOR bytes the daemon reads
/// back through the `topology_cbor` callback.
///
/// On serialization failure it logs the error and falls back to an empty
/// topology (`[]`) rather than propagating the error, so an unexpected failure
/// never unwinds through the `extern "C"` boundary. The event is tagged with
/// the plugin's own name by the [`crate::logging`] bridge, so a generic
/// message is enough to attribute it.
///
/// Cache the returned bytes in a `OnceLock` (or, for a topology that changes at
/// runtime, a `Mutex`) so the pointer handed back to the daemon remains valid
/// until its next serialized `topology_cbor` call, as required by
/// [`crate::descriptor::PluginTopologyCborFn`].
#[must_use]
pub fn topology_cbor(devices: &[DeviceDescriptor]) -> Vec<u8> {
    let mut encoded = Vec::new();
    if let Err(error) = ciborium::into_writer(devices, &mut encoded) {
        tracing::error!(error = %error, "failed to serialize plugin topology");
        encoded.clear();
        if ciborium::into_writer(&Vec::<DeviceDescriptor>::new(), &mut encoded).is_err() {
            encoded.clear();
        }
    }
    encoded
}

#[cfg(test)]
#[path = "topology_tests.rs"]
mod tests;
