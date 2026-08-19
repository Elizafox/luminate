// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Device identity, metadata, topology, and device-level capabilities.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::capability::CapabilitySet;
use crate::group::Group;
use crate::surface::Surface;
use crate::util::declare_opaque_id;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// A lighting device and its addressable topology.
pub struct Device {
    /// Stable identifier.
    pub id: DeviceId,

    /// Human-readable name.
    pub name: String,

    /// Vendor name, when known.
    pub vendor: Option<String>,

    /// Model name, when known.
    pub model: Option<String>,

    /// Configured provider instance currently responsible for this device.
    ///
    /// This is assigned by the daemon rather than supplied by the provider.
    /// `None` means the daemon cannot establish current ownership.
    #[serde(default)]
    pub provider_instance: Option<String>,

    /// Physical or logical surfaces on the device.
    pub surfaces: Vec<Surface>,

    /// Named groups within the device.
    pub groups: Vec<Group>,

    /// Operations supported at this scope.
    pub capabilities: CapabilitySet,

    /// A hint for presenting this device in a GUI (e.g. picking an icon),
    /// not a functional capability. `None` means no hint is available; a
    /// consumer should fall back to a generic icon rather than error.
    pub category: Option<DeviceCategory>,

    /// Open semantic hints about the physical form of the device as a whole,
    /// intended for presentation. Unknown tags must be preserved and ignored
    /// safely by consumers. An empty list means the form is unknown.
    pub physical_tags: Vec<String>,

    /// Whether this device is physically attached to (part of) the host
    /// machine running `luminated`. A fixed, plugin-declared fact carried
    /// straight through from `luminate_plugin_api::DeviceDescriptor`.
    pub host_attached: bool,

    /// Free-form informational text for display (e.g. "requires the vendor
    /// kernel module"). Never parsed or matched on for behaviour. Empty means
    /// none.
    pub notes: Vec<String>,

    /// Free-form cautionary text for display (e.g. "avoid a static colour
    /// for extended periods, this panel is prone to burn-in"). Never parsed
    /// or matched on for behaviour. Empty means none.
    pub warnings: Vec<String>,
}

/// An open, plugin-extensible device classification string. It drives GUI
/// presentation only (icon/silhouette selection) and is never matched on for
/// functional behaviour. Deliberately not a closed enum: new peripheral kinds
/// must not require a `luminate-core` change before a plugin can report them.
/// See `device_category` for well-known values a GUI can special-case;
/// anything else should fall back to a generic icon.
///
/// Plugin authors may use a more specific string than a well-known constant
/// when useful (e.g. `"case-light-strip"` vs. plain `"case-light"`). This is
/// a loose naming convention, not an enforced hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceCategory(String);

declare_opaque_id!(
    DeviceCategory,
    "Creates a presentation category from an open string identifier."
);

/// Well-known `DeviceCategory` string values a GUI can match against for a
/// built-in icon. This list is not exhaustive; plugin authors may use other
/// values, which a GUI should render with a generic fallback icon.
pub mod device_category {
    /// Keyboard lighting.
    pub const KEYBOARD: &str = "keyboard";

    /// Mouse lighting.
    pub const MOUSE: &str = "mouse";

    /// Illuminated fan.
    pub const FAN: &str = "fan";

    /// Illuminated memory module.
    pub const RAM: &str = "ram";

    /// Lighting installed in a computer case.
    pub const CASE_LIGHT: &str = "case-light";

    /// Monitor lighting.
    pub const MONITOR: &str = "monitor";

    /// Illuminated button.
    pub const BUTTON: &str = "button";

    /// Graphics-card lighting.
    pub const GPU: &str = "gpu";

    /// Motherboard lighting.
    pub const MOTHERBOARD: &str = "motherboard";

    /// CPU or system cooler lighting.
    pub const COOLER: &str = "cooler";

    /// Standalone LED strip.
    pub const LED_STRIP: &str = "led-strip";

    /// Speaker lighting.
    pub const SPEAKER: &str = "speaker";

    /// Microphone lighting.
    pub const MICROPHONE: &str = "microphone";

    /// Headset lighting.
    pub const HEADSET: &str = "headset";

    /// Game-controller lighting.
    pub const CONTROLLER: &str = "controller";

    /// Power-supply lighting.
    pub const POWER_SUPPLY: &str = "power-supply";
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
/// Opaque stable identifier for a device.
pub struct DeviceId(String);

declare_opaque_id!(DeviceId, "Creates a device identifier.");

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
