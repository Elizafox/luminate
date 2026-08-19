// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Topology-to-D-Bus object conversion tests.

use luminate::{
    CapabilitySet, DeviceId, Element, ElementGeometry, ElementId, ElementKind, Group, GroupId,
    GroupKind, GroupMember, Surface, SurfaceId, SurfaceKind,
};

use super::*;

#[test]
fn complete_topology_becomes_typed_target_objects() {
    let devices = vec![Device {
        id: DeviceId::new("device"),
        name: "Device".into(),
        vendor: None,
        model: None,
        provider_instance: None,
        surfaces: vec![Surface {
            id: SurfaceId::new("surface"),
            name: "Surface".into(),
            kind: SurfaceKind::Zone,
            physical_tags: Vec::new(),
            elements: vec![Element {
                id: ElementId::new("element"),
                name: None,
                kind: ElementKind::Led,
                geometry: None,
                physical_tags: vec!["shape:round".into(), "position:left".into()],
                capabilities: CapabilitySet::default(),
                notes: Vec::new(),
                warnings: Vec::new(),
            }],
            capabilities: CapabilitySet::default(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: vec![Group {
            id: GroupId::new("group"),
            name: "Group".into(),
            description: None,
            kind: GroupKind::BuiltIn,
            members: vec![
                GroupMember::Surface(SurfaceId::new("surface")),
                GroupMember::Element {
                    surface: SurfaceId::new("surface"),
                    element: ElementId::new("element"),
                },
            ],
            capabilities: CapabilitySet::default(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        capabilities: CapabilitySet::default(),
        category: None,
        physical_tags: vec!["shape:modular-light-bar".to_owned()],
        host_attached: false,
        notes: Vec::new(),
        warnings: Vec::new(),
    }];

    let objects = objects(&devices);
    assert_eq!(objects.len(), 4);
    assert!(objects.values().any(|object| object.kind == Kind::Device));
    assert!(objects.values().any(|object| object.kind == Kind::Surface));
    assert!(objects.values().any(|object| object.kind == Kind::Group));
    assert!(objects.values().any(|object| object.kind == Kind::Element));

    let device = objects
        .values()
        .find(|object| object.kind == Kind::Device)
        .expect("device object");
    let Details::Device(details) = &device.details else {
        panic!("device object should retain complete device details");
    };
    assert_eq!(details.id, "device");
    assert_eq!(details.physical_tags, ["shape:modular-light-bar"]);
    assert_eq!(details.surfaces.len(), 1);
    assert_eq!(details.groups.len(), 1);
    assert!(details.surfaces[0].contains("/surfaces/"));
    assert!(details.groups[0].contains("/groups/"));

    let surface = objects
        .values()
        .find(|object| object.kind == Kind::Surface)
        .expect("surface object");
    let Details::Surface(details) = &surface.details else {
        panic!("surface object should retain complete surface details");
    };
    assert_eq!(details.kind, "zone");
    assert_eq!(details.elements.len(), 1);

    let element = objects
        .values()
        .find(|object| object.kind == Kind::Element)
        .expect("element object");
    let Details::Element(details) = &element.details else {
        panic!("element object should retain complete element details");
    };
    assert_eq!(details.kind, "led");
    assert!(details.name.is_none());
    assert!(details.geometry_kind.is_none());
    assert_eq!(details.physical_tags, ["shape:round", "position:left"]);
    assert!(details.surface.contains("/surfaces/"));

    let group = objects
        .values()
        .find(|object| object.kind == Kind::Group)
        .expect("group object");
    let Details::Group(details) = &group.details else {
        panic!("group object should retain complete group details");
    };
    assert_eq!(details.kind, "built-in");
    assert_eq!(details.members.len(), 2);
    assert!(details.members[0].contains("/surfaces/"));
    assert!(details.members[1].contains("/elements/"));
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the single table-like fixture keeps every topology variant visibly paired with its expected projection"
)]
fn topology_conversion_exercises_every_kind_geometry_and_member_variant() {
    let mut device = Device {
        id: DeviceId::new("device"),
        name: "Device".into(),
        vendor: None,
        model: None,
        provider_instance: None,
        surfaces: Vec::new(),
        groups: Vec::new(),
        capabilities: CapabilitySet::default(),
        category: None,
        physical_tags: Vec::new(),
        host_attached: false,
        notes: Vec::new(),
        warnings: Vec::new(),
    };
    let surface_kinds = [
        SurfaceKind::Opaque,
        SurfaceKind::Zone,
        SurfaceKind::Linear { length: 2.0 },
        SurfaceKind::Sparse2d {
            width: 3.0,
            height: 4.0,
        },
        SurfaceKind::Matrix { rows: 2, cols: 3 },
    ];
    let element_kinds = [
        ElementKind::Key,
        ElementKind::Led,
        ElementKind::Zone,
        ElementKind::Logo,
        ElementKind::RingSegment,
    ];
    let geometries = [
        Some(ElementGeometry::Rect {
            x: 0.1,
            y: 0.2,
            w: 0.3,
            h: 0.4,
        }),
        Some(ElementGeometry::Point { x: 0.1, y: 0.2 }),
        Some(ElementGeometry::Linear { position: 0.5 }),
        Some(ElementGeometry::MatrixCell { row: 1, col: 2 }),
        None,
    ];
    for (index, ((kind, element_kind), geometry)) in surface_kinds
        .into_iter()
        .zip(element_kinds)
        .zip(geometries)
        .enumerate()
    {
        device.surfaces.push(Surface {
            id: SurfaceId::new(format!("surface-{index}")),
            name: format!("Surface {index}"),
            kind,
            physical_tags: Vec::new(),
            elements: vec![Element {
                id: ElementId::new(format!("element-{index}")),
                name: None,
                kind: element_kind,
                geometry,
                physical_tags: Vec::new(),
                capabilities: CapabilitySet::default(),
                notes: Vec::new(),
                warnings: Vec::new(),
            }],
            capabilities: CapabilitySet::default(),
            notes: Vec::new(),
            warnings: Vec::new(),
        });
    }
    for (index, kind) in [
        GroupKind::BuiltIn,
        GroupKind::Topology,
        GroupKind::Driver,
        GroupKind::User,
        GroupKind::Application,
    ]
    .into_iter()
    .enumerate()
    {
        device.groups.push(Group {
            id: GroupId::new(format!("group-{index}")),
            name: format!("Group {index}"),
            description: None,
            kind,
            members: Vec::new(),
            capabilities: CapabilitySet::default(),
            notes: Vec::new(),
            warnings: Vec::new(),
        });
    }
    device.groups[0].members = vec![
        GroupMember::Surface(SurfaceId::new("surface-0")),
        GroupMember::Element {
            surface: SurfaceId::new("surface-1"),
            element: ElementId::new("element-1"),
        },
        GroupMember::Group(GroupId::new("group-1")),
    ];

    let objects = objects(&[device]);
    let mut surface_kinds = objects
        .values()
        .filter_map(|object| match &object.details {
            Details::Surface(details) => Some(details.kind.as_str()),
            Details::Device(_) | Details::Element(_) | Details::Group(_) | Details::Other => None,
        })
        .collect::<Vec<_>>();
    let mut element_kinds = objects
        .values()
        .filter_map(|object| match &object.details {
            Details::Element(details) => Some((
                details.kind.as_str(),
                details.geometry_kind.as_deref().unwrap_or("none"),
            )),
            Details::Device(_) | Details::Surface(_) | Details::Group(_) | Details::Other => None,
        })
        .collect::<Vec<_>>();
    let mut group_kinds = objects
        .values()
        .filter_map(|object| match &object.details {
            Details::Group(details) => Some(details.kind.as_str()),
            Details::Device(_) | Details::Surface(_) | Details::Element(_) | Details::Other => None,
        })
        .collect::<Vec<_>>();
    surface_kinds.sort_unstable();
    element_kinds.sort_unstable();
    group_kinds.sort_unstable();

    assert_eq!(
        surface_kinds,
        ["linear", "matrix", "opaque", "sparse-2d", "zone"]
    );
    assert_eq!(
        element_kinds,
        [
            ("key", "rect"),
            ("led", "point"),
            ("logo", "matrix-cell"),
            ("ring-segment", "none"),
            ("zone", "linear"),
        ]
    );
    assert_eq!(
        group_kinds,
        ["application", "built-in", "driver", "topology", "user"]
    );
    let group = objects
        .values()
        .find_map(|object| match &object.details {
            Details::Group(details) if details.id == "group-0" => Some(details),
            Details::Device(_)
            | Details::Surface(_)
            | Details::Element(_)
            | Details::Group(_)
            | Details::Other => None,
        })
        .expect("group fixture");
    assert_eq!(group.members.len(), 3);
    assert!(group.members[0].contains("/surfaces/"));
    assert!(group.members[1].contains("/elements/"));
    assert!(group.members[2].contains("/groups/"));
}
