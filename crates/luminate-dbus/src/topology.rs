// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Complete device snapshots built from the same topology projection as `ObjectManager`.

use std::collections::BTreeMap;
use std::slice::from_ref;

use luminate::{Device, TargetId};
use zbus::zvariant::OwnedObjectPath;

use crate::capability;
use crate::error::MethodError;
use crate::management::{Dictionary, dictionary, owned};
use crate::model::{Details, Object, objects};
use crate::path::{canonical_id, target_path};

pub(crate) fn device(value: &Device) -> Result<Dictionary, MethodError> {
    let projected = objects(from_ref(value));
    let mut ordered = Vec::new();
    push_target(
        &mut ordered,
        &projected,
        &TargetId::Device(value.id.clone()),
    )?;
    for surface in &value.surfaces {
        push_target(
            &mut ordered,
            &projected,
            &TargetId::surface(value.id.as_str(), surface.id.as_str()),
        )?;
        for element in &surface.elements {
            push_target(
                &mut ordered,
                &projected,
                &TargetId::element(value.id.as_str(), surface.id.as_str(), element.id.as_str()),
            )?;
        }
    }
    for group in &value.groups {
        push_target(
            &mut ordered,
            &projected,
            &TargetId::group(value.id.as_str(), group.id.as_str()),
        )?;
    }
    Ok(dictionary([
        ("Device", owned(value.id.as_str().to_owned())?),
        ("Targets", owned(ordered)?),
    ]))
}

fn push_target(
    result: &mut Vec<Dictionary>,
    projected: &BTreeMap<String, Object>,
    target: &TargetId,
) -> Result<(), MethodError> {
    let path = target_path(target);
    let object = projected.get(&path).ok_or_else(|| {
        MethodError::Internal(format!("projected topology omitted target {target:?}"))
    })?;
    result.push(object_record(object)?);
    Ok(())
}

fn object_record(value: &Object) -> Result<Dictionary, MethodError> {
    Ok(dictionary([
        ("Path", owned(object_path(&value.path)?)?),
        ("Identifier", owned(canonical_id(&value.target))?),
        ("Type", owned(kind(&value.details))?),
        ("Name", owned(value.name.clone())?),
        (
            "Capabilities",
            owned(capability::capabilities(&value.capabilities)?)?,
        ),
        ("Topology", owned(details(&value.details)?)?),
    ]))
}

fn details(value: &Details) -> Result<Dictionary, MethodError> {
    match value {
        Details::Device(value) => {
            let mut result = dictionary([
                ("Id", owned(value.id.clone())?),
                ("HasVendor", owned(value.vendor.is_some())?),
                ("HasModel", owned(value.model.is_some())?),
                (
                    "HasProviderInstance",
                    owned(value.provider_instance.is_some())?,
                ),
                ("HasCategory", owned(value.category.is_some())?),
                ("PhysicalTags", owned(value.physical_tags.clone())?),
                ("HostAttached", owned(value.host_attached)?),
                ("Notes", owned(value.notes.clone())?),
                ("Warnings", owned(value.warnings.clone())?),
                ("Surfaces", owned(paths(&value.surfaces)?)?),
                ("Groups", owned(paths(&value.groups)?)?),
            ]);
            insert_optional(&mut result, "Vendor", value.vendor.as_deref())?;
            insert_optional(&mut result, "Model", value.model.as_deref())?;
            insert_optional(
                &mut result,
                "ProviderInstance",
                value.provider_instance.as_deref(),
            )?;
            insert_optional(&mut result, "Category", value.category.as_deref())?;
            Ok(result)
        }
        Details::Surface(value) => Ok(dictionary([
            ("Id", owned(value.id.clone())?),
            ("Kind", owned(value.kind.clone())?),
            ("HasLength", owned(value.length.is_some())?),
            ("Length", owned(value.length.unwrap_or_default())?),
            ("HasDimensions", owned(value.dimensions.is_some())?),
            (
                "Width",
                owned(value.dimensions.map_or(0.0, |dimensions| dimensions.0))?,
            ),
            (
                "Height",
                owned(value.dimensions.map_or(0.0, |dimensions| dimensions.1))?,
            ),
            ("HasMatrix", owned(value.matrix.is_some())?),
            ("Rows", owned(value.matrix.map_or(0, |matrix| matrix.0))?),
            ("Columns", owned(value.matrix.map_or(0, |matrix| matrix.1))?),
            ("PhysicalTags", owned(value.physical_tags.clone())?),
            ("Elements", owned(paths(&value.elements)?)?),
            ("Notes", owned(value.notes.clone())?),
            ("Warnings", owned(value.warnings.clone())?),
        ])),
        Details::Element(value) => {
            let mut result = dictionary([
                ("Id", owned(value.id.clone())?),
                ("HasName", owned(value.name.is_some())?),
                ("Kind", owned(value.kind.clone())?),
                ("HasGeometry", owned(value.geometry_kind.is_some())?),
                (
                    "GeometryKind",
                    owned(value.geometry_kind.clone().unwrap_or_default())?,
                ),
                ("X", owned(value.x.unwrap_or_default())?),
                ("Y", owned(value.y.unwrap_or_default())?),
                ("Width", owned(value.width.unwrap_or_default())?),
                ("Height", owned(value.height.unwrap_or_default())?),
                ("Position", owned(value.position.unwrap_or_default())?),
                (
                    "Row",
                    owned(value.matrix_cell.map_or(0, |matrix| matrix.0))?,
                ),
                (
                    "Column",
                    owned(value.matrix_cell.map_or(0, |matrix| matrix.1))?,
                ),
                ("Surface", owned(object_path(&value.surface)?)?),
                ("PhysicalTags", owned(value.physical_tags.clone())?),
                ("Notes", owned(value.notes.clone())?),
                ("Warnings", owned(value.warnings.clone())?),
            ]);
            insert_optional(&mut result, "Name", value.name.as_deref())?;
            Ok(result)
        }
        Details::Group(value) => {
            let mut result = dictionary([
                ("Id", owned(value.id.clone())?),
                ("HasDescription", owned(value.description.is_some())?),
                ("Kind", owned(value.kind.clone())?),
                ("Members", owned(paths(&value.members)?)?),
                ("Notes", owned(value.notes.clone())?),
                ("Warnings", owned(value.warnings.clone())?),
            ]);
            insert_optional(&mut result, "Description", value.description.as_deref())?;
            Ok(result)
        }
        #[cfg(test)]
        Details::Other => Err(MethodError::Internal(
            "test-only topology details cannot be serialized".into(),
        )),
    }
}

fn insert_optional(
    result: &mut Dictionary,
    field: &str,
    value: Option<&str>,
) -> Result<(), MethodError> {
    if let Some(value) = value {
        result.insert(field.to_owned(), owned(value.to_owned())?);
    }
    Ok(())
}

fn paths(values: &[String]) -> Result<Vec<OwnedObjectPath>, MethodError> {
    values.iter().map(|value| object_path(value)).collect()
}

fn object_path(value: &str) -> Result<OwnedObjectPath, MethodError> {
    OwnedObjectPath::try_from(value.to_owned()).map_err(|error| {
        MethodError::Internal(format!(
            "generated invalid D-Bus object path {value:?}: {error}"
        ))
    })
}

fn kind(value: &Details) -> &'static str {
    match value {
        Details::Device(_) => "device",
        Details::Surface(_) => "surface",
        Details::Element(_) => "element",
        Details::Group(_) => "group",
        #[cfg(test)]
        Details::Other => "test",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use luminate::{
        CapabilitySet, DeviceId, Element, ElementGeometry, ElementId, ElementKind, Group, GroupId,
        GroupKind, GroupMember, Surface, SurfaceId, SurfaceKind,
    };

    fn assert_element_physical_tags(targets: &[Dictionary]) {
        let element = targets
            .iter()
            .find(|target| {
                target
                    .get("Type")
                    .and_then(|value| value.try_clone().ok())
                    .and_then(|value| String::try_from(value).ok())
                    .is_some_and(|kind| kind == "element")
            })
            .expect("snapshot should contain an element target");
        let topology = Dictionary::try_from(
            element
                .get("Topology")
                .expect("element should contain topology")
                .try_clone()
                .expect("clone element topology"),
        )
        .expect("element topology should be a dictionary");
        let physical_tags = Vec::<String>::try_from(
            topology
                .get("PhysicalTags")
                .expect("element topology should contain physical tags")
                .try_clone()
                .expect("clone physical tags"),
        )
        .expect("physical tags should be strings");

        assert_eq!(physical_tags, ["shape:keycap", "position:escape"]);
    }

    #[test]
    fn snapshot_preserves_hierarchy_order_and_complete_relationships() {
        let value = Device {
            id: DeviceId::new("keyboard"),
            name: "Keyboard".into(),
            vendor: Some("Acme".into()),
            model: None,
            provider_instance: Some("usb".into()),
            surfaces: vec![Surface {
                id: SurfaceId::new("keys"),
                name: "Keys".into(),
                kind: SurfaceKind::Matrix { rows: 1, cols: 2 },
                physical_tags: vec!["keyboard-keys".into()],
                elements: vec![Element {
                    id: ElementId::new("escape"),
                    name: Some("Escape".into()),
                    kind: ElementKind::Key,
                    geometry: Some(ElementGeometry::MatrixCell { row: 0, col: 0 }),
                    physical_tags: vec!["shape:keycap".into(), "position:escape".into()],
                    capabilities: CapabilitySet::default(),
                    notes: Vec::new(),
                    warnings: Vec::new(),
                }],
                capabilities: CapabilitySet::default(),
                notes: Vec::new(),
                warnings: Vec::new(),
            }],
            groups: vec![Group {
                id: GroupId::new("all"),
                name: "All".into(),
                description: Some("Every key".into()),
                kind: GroupKind::Topology,
                members: vec![GroupMember::Surface(SurfaceId::new("keys"))],
                capabilities: CapabilitySet::default(),
                notes: Vec::new(),
                warnings: Vec::new(),
            }],
            capabilities: CapabilitySet::default(),
            category: Some(luminate::DeviceCategory::new("keyboard")),
            physical_tags: vec!["host-peripheral".into()],
            host_attached: true,
            notes: Vec::new(),
            warnings: Vec::new(),
        };

        let encoded = device(&value).expect("encode device snapshot");
        let targets = Vec::<Dictionary>::try_from(
            encoded
                .get("Targets")
                .expect("snapshot should contain targets")
                .try_clone()
                .expect("clone targets"),
        )
        .expect("targets should be dictionaries");
        let identifiers = targets
            .iter()
            .map(|target| {
                String::try_from(
                    target
                        .get("Identifier")
                        .expect("target should contain identifier")
                        .try_clone()
                        .expect("clone identifier"),
                )
                .expect("identifier should be a string")
            })
            .collect::<Vec<_>>();

        assert_eq!(
            identifiers,
            [
                "device:keyboard",
                "device:keyboard/surface:keys",
                "device:keyboard/surface:keys/element:escape",
                "device:keyboard/group:all",
            ]
        );
        assert!(
            targets
                .iter()
                .all(|target| target.contains_key("Capabilities"))
        );
        assert!(targets.iter().all(|target| target.contains_key("Topology")));
        assert_element_physical_tags(&targets);
    }
}
