// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;
use luminate_core::capability::CapabilitySet;
use std::path::PathBuf;

fn descriptor(id: &str) -> DeviceDescriptor {
    DeviceDescriptor {
        id: id.to_owned(),
        name: id.to_owned(),
        vendor: None,
        model: None,
        surfaces: Vec::new(),
        groups: Vec::new(),
        capabilities: CapabilitySet::default(),
        claims: Vec::new(),
        category: None,
        physical_tags: Vec::new(),
        host_attached: false,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

fn device(id: &str, name: &str) -> Device {
    Device {
        id: DeviceId::new(id),
        name: name.to_owned(),
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
    }
}

#[test]
fn ownership_summary_partitions_in_declaration_order() {
    let descriptors = [descriptor("owned"), descriptor("lost")];
    let owners = HashMap::from([("owned".to_owned(), 2)]);
    assert_eq!(
        ownership_summary(2, &descriptors, &owners),
        (vec!["owned", "lost"], vec!["owned"], vec!["lost"])
    );
}

#[test]
fn owned_descriptors_filters_and_preserves_plugin_order() {
    let first = [descriptor("first"), descriptor("lost")];
    let second = [descriptor("second")];
    let owners = HashMap::from([("first".to_owned(), 0), ("second".to_owned(), 1)]);
    let owned = owned_descriptors(
        &owners,
        [(0, first.as_slice()), (1, second.as_slice())].into_iter(),
    );
    assert_eq!(
        owned
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
}

#[test]
fn changed_device_ids_reports_additions_removals_and_updates_sorted() {
    let previous = [
        device("removed", "Removed"),
        device("changed", "Before"),
        device("same", "Same"),
    ];
    let current = [
        device("same", "Same"),
        device("changed", "After"),
        device("added", "Added"),
    ];
    assert_eq!(
        changed_device_ids(&previous, &current)
            .iter()
            .map(DeviceId::as_str)
            .collect::<Vec<_>>(),
        ["added", "changed", "removed"]
    );
    assert!(changed_device_ids(&current, &current).is_empty());
}

#[test]
fn missing_metadata_loses_priority_to_any_known_plugin() {
    let known = LoadedPluginMetadata {
        name: "known".to_owned(),
        version: "1".to_owned(),
        priority: i32::MIN,
        recommended_reconciliation: None,
        probe_outcome: luminate_plugin_api::ProbeOutcome::Ready,
        buses: Vec::new(),
        vendors: Vec::new(),
        probe_hints: Vec::new(),
        path: PathBuf::new(),
    };
    let descriptors = [vec![descriptor("shared")], vec![descriptor("shared")]];
    assert_eq!(
        arbitrate_ownership([&known], &descriptors),
        HashMap::from([("shared".to_owned(), 0)])
    );
}
