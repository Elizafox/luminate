// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use luminate_core::appearance_slot::{
    AppearanceCapability, AppearanceSlotDescriptor, AppearanceSlotId, AppearanceSlotUpdatePolicy,
    AppearanceSlotsCapability,
};
use luminate_core::capability::{CapabilitySet, PersistenceCapability};
use luminate_core::element::ElementKind;
use luminate_core::group::GroupKind;
use luminate_core::surface::SurfaceKind;
use luminate_plugin_api::{ElementDescriptor, GroupDescriptor, SurfaceDescriptor};

use super::*;

fn sample_device() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: "device0".to_owned(),
        name: "Device Zero".to_owned(),
        vendor: None,
        model: None,
        surfaces: vec![SurfaceDescriptor {
            id: "main".to_owned(),
            name: "Main".to_owned(),
            kind: SurfaceKind::Opaque,
            physical_tags: Vec::new(),
            elements: vec![ElementDescriptor {
                id: "led0".to_owned(),
                name: Some("LED 0".to_owned()),
                kind: ElementKind::Led,
                geometry: None,
                physical_tags: Vec::new(),
                capabilities: CapabilitySet::default(),
                notes: Vec::new(),
                warnings: Vec::new(),
            }],
            capabilities: CapabilitySet::default(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: vec![
            GroupDescriptor {
                id: "all".to_owned(),
                name: "All".to_owned(),
                description: None,
                kind: GroupKind::Topology,
                members: vec![
                    GroupMemberDescriptor::Surface("main".to_owned()),
                    GroupMemberDescriptor::Element {
                        surface: "main".to_owned(),
                        element: "led0".to_owned(),
                    },
                ],
                capabilities: CapabilitySet::default(),
                notes: Vec::new(),
                warnings: Vec::new(),
            },
            GroupDescriptor {
                id: "root".to_owned(),
                name: "Root".to_owned(),
                description: None,
                kind: GroupKind::Topology,
                members: vec![GroupMemberDescriptor::Group("all".to_owned())],
                capabilities: CapabilitySet::default(),
                notes: Vec::new(),
                warnings: Vec::new(),
            },
        ],
        capabilities: CapabilitySet::default(),
        category: None,
        physical_tags: Vec::new(),
        host_attached: false,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

#[test]
fn normalize_valid_device() {
    let devices = normalize_devices(&[sample_device()]).expect("normalization should succeed");
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].surfaces.len(), 1);
    assert_eq!(devices[0].groups.len(), 2);
}

#[test]
fn normalize_preserves_ordered_independent_physical_tags() {
    let mut device = sample_device();
    device.physical_tags = vec![
        "shape:flexible-strip".to_owned(),
        "vendor:example".to_owned(),
    ];
    device.surfaces[0].physical_tags = vec!["layout:wrapped-horizontal".to_owned()];
    device.surfaces[0].elements[0].physical_tags = vec![
        "shape:flexible-strip".to_owned(),
        "position:left".to_owned(),
    ];

    let devices = normalize_devices(&[device]).expect("physical tags should normalize");

    assert_eq!(
        devices[0].physical_tags,
        ["shape:flexible-strip", "vendor:example"]
    );
    assert_eq!(
        devices[0].surfaces[0].physical_tags,
        ["layout:wrapped-horizontal"]
    );
    assert_eq!(
        devices[0].surfaces[0].elements[0].physical_tags,
        ["shape:flexible-strip", "position:left"]
    );
}

#[test]
fn normalize_rejects_invalid_physical_tags_at_every_scope() {
    for tags in [
        vec![String::new()],
        vec![" shape:cylinder".to_owned()],
        vec!["shape:cylinder ".to_owned()],
        vec!["shape:cylinder".to_owned(), "shape:cylinder".to_owned()],
    ] {
        let mut device = sample_device();
        device.physical_tags = tags.clone();
        normalize_devices(&[device]).expect_err("invalid device tags should be rejected");

        let mut device = sample_device();
        device.surfaces[0].physical_tags = tags.clone();
        normalize_devices(&[device]).expect_err("invalid surface tags should be rejected");

        let mut device = sample_device();
        device.surfaces[0].elements[0].physical_tags = tags;
        normalize_devices(&[device]).expect_err("invalid element tags should be rejected");
    }
}

#[test]
fn normalize_does_not_inherit_physical_tags() {
    let mut device = sample_device();
    device.physical_tags = vec!["shape:keyboard".to_owned()];
    device.surfaces[0].physical_tags = vec!["layout:matrix".to_owned()];

    let devices = normalize_devices(&[device]).expect("physical tags should normalize");

    assert!(devices[0].surfaces[0].elements[0].physical_tags.is_empty());
}

fn appearance_slots(ids_and_names: &[(&str, &str)]) -> AppearanceSlotsCapability {
    AppearanceSlotsCapability {
        slots: ids_and_names
            .iter()
            .map(|(id, name)| AppearanceSlotDescriptor {
                id: AppearanceSlotId::new(*id),
                name: (*name).to_owned(),
                appearance: AppearanceCapability::default(),
                persistence: PersistenceCapability::None,
                notes: Vec::new(),
                warnings: Vec::new(),
            })
            .collect(),
        update_policy: AppearanceSlotUpdatePolicy::Independent,
    }
}

#[test]
fn normalize_accepts_unique_appearance_slots_on_surface() {
    let mut device = sample_device();
    device.surfaces[0].capabilities.appearance_slots =
        Some(appearance_slots(&[("ac", "AC"), ("battery", "Battery")]));

    normalize_devices(&[device]).expect("valid surface appearance slots should normalize");
}

#[test]
fn normalize_rejects_duplicate_or_empty_appearance_slots() {
    for slots in [
        appearance_slots(&[("ac", "AC"), ("ac", "Battery")]),
        appearance_slots(&[("ac", "Power"), ("battery", "Power")]),
        appearance_slots(&[("", "Power")]),
        appearance_slots(&[("ac", "")]),
        appearance_slots(&[]),
    ] {
        let mut device = sample_device();
        device.surfaces[0].capabilities.appearance_slots = Some(slots);

        normalize_devices(&[device]).expect_err("invalid appearance slots should be rejected");
    }
}

#[test]
fn normalize_rejects_appearance_slots_outside_surfaces() {
    for target in ["device", "group", "element"] {
        let mut device = sample_device();
        let slots = Some(appearance_slots(&[("ac", "AC")]));
        match target {
            "device" => device.capabilities.appearance_slots = slots,
            "group" => device.groups[0].capabilities.appearance_slots = slots,
            "element" => device.surfaces[0].elements[0].capabilities.appearance_slots = slots,
            _ => unreachable!("fixed test cases"),
        }

        let error = normalize_devices(&[device])
            .expect_err("appearance slots outside a surface should be rejected");
        assert!(error.to_string().contains("only valid on surfaces"));
    }
}

#[test]
fn normalize_devices_rejects_duplicate_device_ids() {
    // Plugin loading relies on this validation: a plugin that advertises
    // duplicate device IDs must fail to load (or, when optional, be skipped)
    // rather than aborting the daemon later.
    let mut second = sample_device();
    second.name = "Device One".to_owned();
    let error = normalize_devices(&[sample_device(), second])
        .expect_err("duplicate device ids should be rejected");
    assert!(
        error.to_string().contains("duplicate device id"),
        "unexpected error: {error}"
    );
}

#[test]
fn normalize_devices_rejects_duplicate_device_names() {
    let mut second = sample_device();
    second.id = "device1".to_owned();
    let error = normalize_devices(&[sample_device(), second])
        .expect_err("duplicate device names should be rejected");
    assert!(
        error.to_string().contains("duplicate device name"),
        "unexpected error: {error}"
    );
}

#[test]
fn reject_group_with_unknown_surface() {
    let mut device = sample_device();
    device.groups[0].members = vec![GroupMemberDescriptor::Surface("missing".to_owned())];

    let error = normalize_devices(&[device]).expect_err("normalization should fail");
    assert!(error.to_string().contains("unknown surface"));
}

#[test]
fn group_may_reference_later_group() {
    let mut device = sample_device();
    device.groups[0].members = vec![GroupMemberDescriptor::Group("root".to_owned())];
    device.groups[1].members = Vec::new();

    let devices = normalize_devices(&[device]).expect("forward group reference should resolve");
    assert_eq!(
        devices[0].groups[0].members,
        vec![GroupMember::Group(GroupId::new("root"))]
    );
}

#[test]
fn group_may_not_reference_itself() {
    let mut device = sample_device();
    device.groups[0].members = vec![GroupMemberDescriptor::Group("all".to_owned())];

    let error = normalize_devices(&[device]).expect_err("self-reference should fail");
    assert!(error.to_string().contains("references itself"));
}

#[test]
fn group_may_not_reference_indirect_cycle() {
    let mut device = sample_device();
    device.groups[0].members = vec![GroupMemberDescriptor::Group("root".to_owned())];
    device.groups[1].members = vec![GroupMemberDescriptor::Group("all".to_owned())];

    let error = normalize_devices(&[device]).expect_err("indirect cycle should fail");
    assert!(
        error.to_string().contains("group reference cycle"),
        "unexpected error: {error}"
    );
}
