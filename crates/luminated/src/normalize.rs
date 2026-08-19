// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Validation and normalization of plugin topology descriptors.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail, ensure};

use luminate_core::capability::CapabilitySet;
use luminate_core::device::{Device, DeviceId};
use luminate_core::element::{Element, ElementId};
use luminate_core::group::{Group, GroupId, GroupMember};
use luminate_core::surface::{Surface, SurfaceId};
use luminate_plugin_api::{DeviceDescriptor, GroupMemberDescriptor};

/// Validates and normalizes device descriptors, rejecting duplicate identities.
pub fn normalize_devices(descriptors: &[DeviceDescriptor]) -> Result<Vec<Device>> {
    let mut devices = Vec::with_capacity(descriptors.len());
    let mut seen_device_ids = HashSet::new();
    let mut seen_device_names = HashSet::new();

    for descriptor in descriptors {
        ensure!(
            seen_device_ids.insert(descriptor.id.clone()),
            "duplicate device id: {}",
            descriptor.id
        );
        ensure!(
            seen_device_names.insert(descriptor.name.clone()),
            "duplicate device name: {}",
            descriptor.name
        );

        devices.push(normalize_device(descriptor)?);
    }

    Ok(devices)
}

fn normalize_device(descriptor: &DeviceDescriptor) -> Result<Device> {
    ensure!(!descriptor.id.is_empty(), "device id must not be empty");
    ensure!(!descriptor.name.is_empty(), "device name must not be empty");
    reject_appearance_slots(&descriptor.capabilities, "device")?;
    validate_physical_tags(&descriptor.physical_tags, "device")?;

    let mut surface_ids = HashSet::new();
    let mut surface_names = HashSet::new();
    let mut element_ids_by_surface = HashMap::<String, HashSet<String>>::new();
    let mut group_ids = HashSet::new();
    let mut group_names = HashSet::new();

    let surfaces = normalize_surfaces(
        descriptor,
        &mut surface_ids,
        &mut surface_names,
        &mut element_ids_by_surface,
    )?;

    let groups = normalize_groups(
        descriptor,
        &surface_ids,
        &element_ids_by_surface,
        &mut group_ids,
        &mut group_names,
    )?;

    Ok(Device {
        id: DeviceId::new(descriptor.id.clone()),
        name: descriptor.name.clone(),
        vendor: descriptor.vendor.clone(),
        model: descriptor.model.clone(),
        provider_instance: None,
        surfaces,
        groups,
        capabilities: descriptor.capabilities.clone(),
        category: descriptor.category.clone(),
        physical_tags: descriptor.physical_tags.clone(),
        host_attached: descriptor.host_attached,
        notes: descriptor.notes.clone(),
        warnings: descriptor.warnings.clone(),
    })
}

fn normalize_surfaces(
    descriptor: &DeviceDescriptor,
    surface_ids: &mut HashSet<String>,
    surface_names: &mut HashSet<String>,
    element_ids_by_surface: &mut HashMap<String, HashSet<String>>,
) -> Result<Vec<Surface>> {
    descriptor
        .surfaces
        .iter()
        .map(|surface| {
            ensure!(!surface.id.is_empty(), "surface id must not be empty");
            ensure!(!surface.name.is_empty(), "surface name must not be empty");
            validate_appearance_slots(&surface.capabilities, &surface.id)?;
            validate_physical_tags(&surface.physical_tags, &format!("surface {}", surface.id))?;
            ensure!(
                surface_ids.insert(surface.id.clone()),
                "duplicate surface id on device {}: {}",
                descriptor.id,
                surface.id
            );
            ensure!(
                surface_names.insert(surface.name.clone()),
                "duplicate surface name on device {}: {}",
                descriptor.id,
                surface.name
            );

            let mut element_ids = HashSet::new();
            let mut element_names = HashSet::new();

            let elements = surface
                .elements
                .iter()
                .map(|element| {
                    ensure!(!element.id.is_empty(), "element id must not be empty");
                    reject_appearance_slots(&element.capabilities, "element")?;
                    validate_physical_tags(
                        &element.physical_tags,
                        &format!("element {}", element.id),
                    )?;
                    ensure!(
                        element_ids.insert(element.id.clone()),
                        "duplicate element id on surface {}: {}",
                        surface.id,
                        element.id
                    );

                    if let Some(name) = &element.name {
                        ensure!(
                            element_names.insert(name.clone()),
                            "duplicate element name on surface {}: {}",
                            surface.id,
                            name
                        );
                    }

                    Ok(Element {
                        id: ElementId::new(element.id.clone()),
                        name: element.name.clone(),
                        kind: element.kind.clone(),
                        geometry: element.geometry.clone(),
                        physical_tags: element.physical_tags.clone(),
                        capabilities: element.capabilities.clone(),
                        notes: element.notes.clone(),
                        warnings: element.warnings.clone(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;

            element_ids_by_surface.insert(surface.id.clone(), element_ids);

            Ok(Surface {
                id: SurfaceId::new(surface.id.clone()),
                name: surface.name.clone(),
                kind: surface.kind.clone(),
                physical_tags: surface.physical_tags.clone(),
                elements,
                capabilities: surface.capabilities.clone(),
                notes: surface.notes.clone(),
                warnings: surface.warnings.clone(),
            })
        })
        .collect()
}

fn validate_physical_tags(tags: &[String], location: &str) -> Result<()> {
    let mut seen = HashSet::new();

    for tag in tags {
        ensure!(
            !tag.is_empty() && tag.trim() == tag,
            "{location} physical tags must not be empty or have leading or trailing whitespace"
        );
        ensure!(
            seen.insert(tag),
            "{location} has duplicate physical tag: {tag}"
        );
    }

    Ok(())
}

fn normalize_groups(
    descriptor: &DeviceDescriptor,
    surface_ids: &HashSet<String>,
    element_ids_by_surface: &HashMap<String, HashSet<String>>,
    group_ids: &mut HashSet<String>,
    group_names: &mut HashSet<String>,
) -> Result<Vec<Group>> {
    for group in &descriptor.groups {
        ensure!(!group.id.is_empty(), "group id must not be empty");
        ensure!(!group.name.is_empty(), "group name must not be empty");
        reject_appearance_slots(&group.capabilities, "group")?;
        ensure!(
            group_ids.insert(group.id.clone()),
            "duplicate group id on device {}: {}",
            descriptor.id,
            group.id
        );
        ensure!(
            group_names.insert(group.name.clone()),
            "duplicate group name on device {}: {}",
            descriptor.id,
            group.name
        );
    }
    let groups = descriptor
        .groups
        .iter()
        .map(|group| {
            let members = group
                .members
                .iter()
                .map(|member| {
                    normalize_group_member(
                        &group.id,
                        member,
                        surface_ids,
                        element_ids_by_surface,
                        group_ids,
                    )
                })
                .collect::<Result<Vec<_>>>()?;

            Ok(Group {
                id: GroupId::new(group.id.clone()),
                name: group.name.clone(),
                description: group.description.clone(),
                kind: group.kind,
                members,
                capabilities: group.capabilities.clone(),
                notes: group.notes.clone(),
                warnings: group.warnings.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    reject_group_cycles(descriptor)?;
    Ok(groups)
}

fn reject_appearance_slots(capabilities: &CapabilitySet, target_kind: &str) -> Result<()> {
    ensure!(
        capabilities.appearance_slots.is_none(),
        "appearance slots are only valid on surfaces, not on a {target_kind}"
    );
    Ok(())
}

fn validate_appearance_slots(capabilities: &CapabilitySet, surface: &str) -> Result<()> {
    let Some(appearance_slots) = &capabilities.appearance_slots else {
        return Ok(());
    };

    ensure!(
        !appearance_slots.slots.is_empty(),
        "surface {surface} advertises appearance slots with no slots"
    );
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for slot in &appearance_slots.slots {
        let id = slot.id.as_str();
        ensure!(
            !id.is_empty(),
            "surface {surface} has an appearance slot with an empty id"
        );
        ensure!(
            ids.insert(id),
            "surface {surface} advertises duplicate appearance slot id {id}"
        );
        ensure!(
            !slot.name.is_empty(),
            "surface {surface} appearance slot {id} has an empty name"
        );
        ensure!(
            names.insert(slot.name.as_str()),
            "surface {surface} advertises duplicate appearance slot name {}",
            slot.name
        );
    }
    Ok(())
}

fn normalize_group_member(
    current_group: &str,
    member: &GroupMemberDescriptor,
    surface_ids: &HashSet<String>,
    element_ids_by_surface: &HashMap<String, HashSet<String>>,
    group_ids: &HashSet<String>,
) -> Result<GroupMember> {
    match member {
        GroupMemberDescriptor::Surface(surface) => {
            ensure!(
                surface_ids.contains(surface),
                "group references unknown surface: {surface}"
            );
            Ok(GroupMember::Surface(SurfaceId::new(surface.clone())))
        }
        GroupMemberDescriptor::Element { surface, element } => {
            let Some(element_ids) = element_ids_by_surface.get(surface) else {
                bail!("group references unknown surface for element member: {surface}");
            };
            ensure!(
                element_ids.contains(element),
                "group references unknown element {element} on surface {surface}"
            );
            Ok(GroupMember::Element {
                surface: SurfaceId::new(surface.clone()),
                element: ElementId::new(element.clone()),
            })
        }
        GroupMemberDescriptor::Group(group) => {
            ensure!(
                group != current_group,
                "group references itself: {current_group}"
            );
            ensure!(
                group_ids.contains(group),
                "group references unknown group: {group}"
            );
            Ok(GroupMember::Group(GroupId::new(group.clone())))
        }
    }
}

fn reject_group_cycles(descriptor: &DeviceDescriptor) -> Result<()> {
    let graph = descriptor
        .groups
        .iter()
        .map(|group| {
            let members = group
                .members
                .iter()
                .filter_map(|member| match member {
                    GroupMemberDescriptor::Group(group) => Some(group.as_str()),
                    GroupMemberDescriptor::Surface(_) | GroupMemberDescriptor::Element { .. } => {
                        None
                    }
                })
                .collect::<Vec<_>>();
            (group.id.as_str(), members)
        })
        .collect::<HashMap<_, _>>();

    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    for group in graph.keys().copied() {
        reject_group_cycles_from(group, &graph, &mut visiting, &mut visited)?;
    }
    Ok(())
}

fn reject_group_cycles_from<'a>(
    group: &'a str,
    graph: &HashMap<&'a str, Vec<&'a str>>,
    visiting: &mut HashSet<&'a str>,
    visited: &mut HashSet<&'a str>,
) -> Result<()> {
    if visited.contains(group) {
        return Ok(());
    }
    ensure!(
        visiting.insert(group),
        "group reference cycle includes: {group}"
    );

    if let Some(members) = graph.get(group) {
        for member in members {
            reject_group_cycles_from(member, graph, visiting, visited)?;
        }
    }

    visiting.remove(group);
    visited.insert(group);
    Ok(())
}

#[cfg(test)]
#[path = "normalize_tests.rs"]
mod tests;
