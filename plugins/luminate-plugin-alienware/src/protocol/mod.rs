// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Shared HID transport and colour conversion for Alienware protocols.

use hidapi::{HidApi, HidDevice};
use luminate_core::capability::ColourEncoding;
use luminate_core::colour::{Colour, Rgb8Error};
use luminate_core::effect::Effect;
use luminate_core::rgb::Rgb;

pub mod aw_elc;
pub mod keyboard;
pub mod keyboard_custom;
#[cfg(test)]
// Retained as tested protocol reference data until an exact M14x profile can
// establish truthful discovery and routing.
mod legacy_v2;

/// One open HID session. Real devices go through hidapi; tests inject a fake
/// that records writes and can simulate I/O failure partway through a
/// multi-report sequence.
pub(crate) trait HidChannel {
    fn write_feature_report(&self, payload: &[u8]) -> Result<(), String>;
    fn get_feature_report(&self, buffer: &mut [u8]) -> Result<usize, String>;
}

impl HidChannel for HidDevice {
    fn write_feature_report(&self, payload: &[u8]) -> Result<(), String> {
        self.send_feature_report(payload)
            .map_err(|error| format!("failed to send HID feature report: {error}"))
    }

    fn get_feature_report(&self, buffer: &mut [u8]) -> Result<usize, String> {
        HidDevice::get_feature_report(self, buffer)
            .map_err(|error| format!("failed to read HID feature report: {error}"))
    }
}

/// Identifies one HID top-level collection by its usage page and usage,
/// within a device that may expose several. A vendor/product pair alone is
/// not enough to pick a collection: Windows opens each top-level collection
/// as a separate device and refuses feature reports that don't belong to the
/// one actually opened, while Linux's `hidraw` happens to expose every
/// collection on a device through the same node regardless of which one is
/// named. Filtering by usage on both platforms keeps the two behaviourally
/// aligned instead of depending on which collection hidapi's enumeration
/// happens to return first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HidUsage {
    pub(crate) page: u16,
    pub(crate) usage: u16,
}

/// Opens a HID session for one top-level collection of a vendor/product
/// pair. The default implementation goes through hidapi; tests inject a fake
/// to exercise unavailable-device and mid-sequence failure paths without
/// hardware.
pub(crate) trait HidTransport {
    fn open(
        &self,
        vendor_id: u16,
        product_id: u16,
        usage: HidUsage,
    ) -> Result<Box<dyn HidChannel>, String>;
}

pub(crate) struct HidApiTransport;

impl HidTransport for HidApiTransport {
    fn open(
        &self,
        vendor_id: u16,
        product_id: u16,
        usage: HidUsage,
    ) -> Result<Box<dyn HidChannel>, String> {
        let api = HidApi::new().map_err(|error| format!("failed to initialize hidapi: {error}"))?;
        let info = api
            .device_list()
            .find(|device| {
                device.vendor_id() == vendor_id
                    && device.product_id() == product_id
                    && device.usage_page() == usage.page
                    && device.usage() == usage.usage
            })
            .ok_or_else(|| {
                format!(
                    "no HID collection for {vendor_id:04x}:{product_id:04x} usage page \
                     {:#06x} usage {:#06x}",
                    usage.page, usage.usage
                )
            })?;
        let device = api.open_path(info.path()).map_err(|error| {
            format!("failed to open HID device {vendor_id:04x}:{product_id:04x}: {error}")
        })?;
        Ok(Box::new(device))
    }
}

fn rgb_from_colour(colour: &Colour) -> Result<Rgb, String> {
    if colour.encoding() != ColourEncoding::Additive {
        return Err("only additive RGB colours are supported".to_owned());
    }

    let Colour::Additive(channels) = colour else {
        return Err("only additive RGB colours are supported".to_owned());
    };
    for channel in channels {
        u8::try_from(channel.value)
            .map_err(|_| format!("colour channel value {} exceeds 8-bit RGB", channel.value))?;
    }

    colour.try_as_rgb().map_err(|error| match error {
        Rgb8Error::MissingChannel(channel) => {
            format!("missing {} channel", channel.as_str())
        }
        Rgb8Error::ChannelOutOfRange { value, .. } => {
            format!("colour channel value {value} exceeds 8-bit RGB")
        }
        Rgb8Error::NonAdditive(_) => "only additive RGB colours are supported".to_owned(),
    })
}

fn effect_to_rgb_static(
    operation: &luminate_plugin_api::PluginUpdateOperation,
) -> Result<Rgb, String> {
    match operation {
        luminate_plugin_api::PluginUpdateOperation::SetEffect {
            effect: Effect::Static { colour },
        } => rgb_from_colour(colour),
        luminate_plugin_api::PluginUpdateOperation::SetEffect {
            effect: Effect::Off,
        }
        | luminate_plugin_api::PluginUpdateOperation::Clear => Ok(Rgb::new(0, 0, 0)),
        luminate_plugin_api::PluginUpdateOperation::SetBrightness { .. }
        | luminate_plugin_api::PluginUpdateOperation::SetAppearanceSlots { .. }
        | luminate_plugin_api::PluginUpdateOperation::SaveCurrent
        | luminate_plugin_api::PluginUpdateOperation::SetEffect { .. } => {
            Err("operation cannot be lowered to a static RGB write".to_owned())
        }
    }
}

fn write_feature_report(device: &dyn HidChannel, payload: &[u8]) -> Result<(), String> {
    device.write_feature_report(payload)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
