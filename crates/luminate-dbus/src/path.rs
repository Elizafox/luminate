// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Stable D-Bus object-path construction for Luminate targets.

use luminate::{DeviceId, ElementId, GroupId, SurfaceId, TargetId};

use crate::convert::invalid_argument;
use crate::error::MethodError;

pub const ROOT: &str = "/org/luminate/Luminate1";

pub(crate) fn canonical_id(target: &TargetId) -> String {
    match target {
        TargetId::Device(device) => format!("device:{}", device.as_str()),
        TargetId::Surface { device, surface } => {
            format!("device:{}/surface:{}", device.as_str(), surface.as_str())
        }
        TargetId::Group { device, group } => {
            format!("device:{}/group:{}", device.as_str(), group.as_str())
        }
        TargetId::Element {
            device,
            surface,
            element,
        } => format!(
            "device:{}/surface:{}/element:{}",
            device.as_str(),
            surface.as_str(),
            element.as_str()
        ),
    }
}

pub(crate) fn parse_canonical_id(value: &str) -> Result<TargetId, MethodError> {
    let parts = value.split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        [device] => Ok(TargetId::Device(DeviceId::new(component_value(
            device, "device",
        )?))),
        [device, group] if group.starts_with("group:") => Ok(TargetId::Group {
            device: DeviceId::new(component_value(device, "device")?),
            group: GroupId::new(component_value(group, "group")?),
        }),
        [device, surface] => Ok(TargetId::Surface {
            device: DeviceId::new(component_value(device, "device")?),
            surface: SurfaceId::new(component_value(surface, "surface")?),
        }),
        [device, surface, element] => Ok(TargetId::Element {
            device: DeviceId::new(component_value(device, "device")?),
            surface: SurfaceId::new(component_value(surface, "surface")?),
            element: ElementId::new(component_value(element, "element")?),
        }),
        _ => Err(invalid_argument(format!(
            "invalid canonical target identifier {value:?}"
        ))),
    }
}

fn component_value<'a>(value: &'a str, kind: &str) -> Result<&'a str, MethodError> {
    value
        .strip_prefix(kind)
        .and_then(|value| value.strip_prefix(':'))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid_argument(format!("invalid {kind} target component {value:?}")))
}

fn component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() * 2 + 1);
    encoded.push('x');
    for byte in value.as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

pub fn target_path(target: &TargetId) -> String {
    match target {
        TargetId::Device(device) => format!("{ROOT}/devices/{}", component(device.as_str())),
        TargetId::Surface { device, surface } => format!(
            "{ROOT}/devices/{}/surfaces/{}",
            component(device.as_str()),
            component(surface.as_str())
        ),
        TargetId::Group { device, group } => format!(
            "{ROOT}/devices/{}/groups/{}",
            component(device.as_str()),
            component(group.as_str())
        ),
        TargetId::Element {
            device,
            surface,
            element,
        } => format!(
            "{ROOT}/devices/{}/surfaces/{}/elements/{}",
            component(device.as_str()),
            component(surface.as_str()),
            component(element.as_str())
        ),
    }
}

#[cfg(test)]
#[path = "path_tests.rs"]
mod tests;
