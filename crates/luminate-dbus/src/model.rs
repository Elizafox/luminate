// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Conversion of Luminate topology and capabilities into D-Bus values.

use std::collections::BTreeMap;

use luminate::{
    BrightnessCapability, CapabilitySet, Device, ElementGeometry, ElementKind, GroupKind,
    GroupMember, SurfaceKind, TargetId,
};

use crate::path::target_path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDetails {
    pub id: String,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub provider_instance: Option<String>,
    pub category: Option<String>,
    pub physical_tags: Vec<String>,
    pub host_attached: bool,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
    pub surfaces: Vec<String>,
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceDetails {
    pub id: String,
    pub kind: String,
    pub length: Option<f64>,
    pub dimensions: Option<(f64, f64)>,
    pub matrix: Option<(u16, u16)>,
    pub physical_tags: Vec<String>,
    pub elements: Vec<String>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ElementDetails {
    pub id: String,
    pub name: Option<String>,
    pub kind: String,
    pub geometry_kind: Option<String>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub position: Option<f64>,
    pub matrix_cell: Option<(u16, u16)>,
    pub surface: String,
    pub physical_tags: Vec<String>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupDetails {
    pub id: String,
    pub description: Option<String>,
    pub kind: String,
    pub members: Vec<String>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Details {
    Device(Box<DeviceDetails>),
    Surface(Box<SurfaceDetails>),
    Element(Box<ElementDetails>),
    Group(Box<GroupDetails>),
    #[cfg(test)]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Device,
    Surface,
    Group,
    Element,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    pub path: String,
    pub target: TargetId,
    pub kind: Kind,
    pub name: String,
    pub capabilities: CapabilitySet,
    pub details: Details,
}

impl Object {
    pub fn brightness_maximum(&self) -> u32 {
        match self.capabilities.brightness {
            BrightnessCapability::Independent { maximum, .. } => maximum,
            BrightnessCapability::None => 0,
        }
    }
}

pub fn objects(devices: &[Device]) -> BTreeMap<String, Object> {
    let mut result = BTreeMap::new();
    for device in devices {
        let device_target = TargetId::Device(device.id.clone());
        insert(
            &mut result,
            device_target,
            Kind::Device,
            device.name.clone(),
            device.capabilities.clone(),
            Details::Device(Box::new(DeviceDetails {
                id: device.id.as_str().to_owned(),
                vendor: device.vendor.clone(),
                model: device.model.clone(),
                provider_instance: device.provider_instance.clone(),
                category: device
                    .category
                    .as_ref()
                    .map(|category| category.as_str().to_owned()),
                physical_tags: device.physical_tags.clone(),
                host_attached: device.host_attached,
                notes: device.notes.clone(),
                warnings: device.warnings.clone(),
                surfaces: device
                    .surfaces
                    .iter()
                    .map(|surface| {
                        target_path(&TargetId::surface(device.id.as_str(), surface.id.as_str()))
                    })
                    .collect(),
                groups: device
                    .groups
                    .iter()
                    .map(|group| {
                        target_path(&TargetId::group(device.id.as_str(), group.id.as_str()))
                    })
                    .collect(),
            })),
        );
        for surface in &device.surfaces {
            let surface_target = TargetId::surface(device.id.as_str(), surface.id.as_str());
            insert(
                &mut result,
                surface_target,
                Kind::Surface,
                surface.name.clone(),
                surface.capabilities.clone(),
                Details::Surface(Box::new(surface_details(device, surface))),
            );
            for element in &surface.elements {
                let target =
                    TargetId::element(device.id.as_str(), surface.id.as_str(), element.id.as_str());
                insert(
                    &mut result,
                    target,
                    Kind::Element,
                    element
                        .name
                        .clone()
                        .unwrap_or_else(|| element.id.as_str().to_owned()),
                    element.capabilities.clone(),
                    Details::Element(Box::new(element_details(device, surface, element))),
                );
            }
        }
        for group in &device.groups {
            let target = TargetId::group(device.id.as_str(), group.id.as_str());
            insert(
                &mut result,
                target,
                Kind::Group,
                group.name.clone(),
                group.capabilities.clone(),
                Details::Group(Box::new(group_details(device, group))),
            );
        }
    }
    result
}

fn surface_details(device: &Device, surface: &luminate::Surface) -> SurfaceDetails {
    let (kind, length, dimensions, matrix) = match surface.kind {
        SurfaceKind::Opaque => ("opaque", None, None, None),
        SurfaceKind::Zone => ("zone", None, None, None),
        SurfaceKind::Linear { length } => ("linear", Some(f64::from(length)), None, None),
        SurfaceKind::Sparse2d { width, height } => (
            "sparse-2d",
            None,
            Some((f64::from(width), f64::from(height))),
            None,
        ),
        SurfaceKind::Matrix { rows, cols } => ("matrix", None, None, Some((rows, cols))),
    };
    SurfaceDetails {
        id: surface.id.as_str().to_owned(),
        kind: kind.to_owned(),
        length,
        dimensions,
        matrix,
        physical_tags: surface.physical_tags.clone(),
        elements: surface
            .elements
            .iter()
            .map(|element| {
                target_path(&TargetId::element(
                    device.id.as_str(),
                    surface.id.as_str(),
                    element.id.as_str(),
                ))
            })
            .collect(),
        notes: surface.notes.clone(),
        warnings: surface.warnings.clone(),
    }
}

fn element_details(
    device: &Device,
    surface: &luminate::Surface,
    element: &luminate::Element,
) -> ElementDetails {
    let kind = match element.kind {
        ElementKind::Key => "key",
        ElementKind::Led => "led",
        ElementKind::Zone => "zone",
        ElementKind::Logo => "logo",
        ElementKind::RingSegment => "ring-segment",
    };
    let mut details = ElementDetails {
        id: element.id.as_str().to_owned(),
        name: element.name.clone(),
        kind: kind.to_owned(),
        geometry_kind: None,
        x: None,
        y: None,
        width: None,
        height: None,
        position: None,
        matrix_cell: None,
        surface: target_path(&TargetId::surface(device.id.as_str(), surface.id.as_str())),
        physical_tags: element.physical_tags.clone(),
        notes: element.notes.clone(),
        warnings: element.warnings.clone(),
    };
    match element.geometry {
        None => {}
        Some(ElementGeometry::Rect { x, y, w, h }) => {
            details.geometry_kind = Some("rect".to_owned());
            details.x = Some(f64::from(x));
            details.y = Some(f64::from(y));
            details.width = Some(f64::from(w));
            details.height = Some(f64::from(h));
        }
        Some(ElementGeometry::Point { x, y }) => {
            details.geometry_kind = Some("point".to_owned());
            details.x = Some(f64::from(x));
            details.y = Some(f64::from(y));
        }
        Some(ElementGeometry::Linear { position }) => {
            details.geometry_kind = Some("linear".to_owned());
            details.position = Some(f64::from(position));
        }
        Some(ElementGeometry::MatrixCell { row, col }) => {
            details.geometry_kind = Some("matrix-cell".to_owned());
            details.matrix_cell = Some((row, col));
        }
    }
    details
}

fn group_details(device: &Device, group: &luminate::Group) -> GroupDetails {
    let kind = match group.kind {
        GroupKind::BuiltIn => "built-in",
        GroupKind::Topology => "topology",
        GroupKind::Driver => "driver",
        GroupKind::User => "user",
        GroupKind::Application => "application",
    };
    let members = group
        .members
        .iter()
        .map(|member| match member {
            GroupMember::Surface(surface) => {
                target_path(&TargetId::surface(device.id.as_str(), surface.as_str()))
            }
            GroupMember::Element { surface, element } => target_path(&TargetId::element(
                device.id.as_str(),
                surface.as_str(),
                element.as_str(),
            )),
            GroupMember::Group(group) => {
                target_path(&TargetId::group(device.id.as_str(), group.as_str()))
            }
        })
        .collect();
    GroupDetails {
        id: group.id.as_str().to_owned(),
        description: group.description.clone(),
        kind: kind.to_owned(),
        members,
        notes: group.notes.clone(),
        warnings: group.warnings.clone(),
    }
}

fn insert(
    objects: &mut BTreeMap<String, Object>,
    target: TargetId,
    kind: Kind,
    name: String,
    capabilities: CapabilitySet,
    details: Details,
) {
    let path = target_path(&target);
    let _ = objects.insert(
        path.clone(),
        Object {
            path,
            target,
            kind,
            name,
            capabilities,
            details,
        },
    );
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
