// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use luminate_core::capability::CapabilitySet;
use luminate_core::element::ElementKind;

use crate::{decode_cbor, encode_cbor};

use super::{ElementDescriptor, PluginStateSnapshot, PluginTarget, write_state_snapshot};

fn element_descriptor() -> ElementDescriptor {
    ElementDescriptor {
        id: "led".to_owned(),
        name: None,
        kind: ElementKind::Led,
        geometry: None,
        physical_tags: vec!["shape:round".to_owned(), "position:left".to_owned()],
        capabilities: CapabilitySet::default(),
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

#[test]
fn element_physical_tags_round_trip_in_order() {
    let value = serde_json::to_value(element_descriptor()).expect("serialize element descriptor");
    let decoded: ElementDescriptor =
        serde_json::from_value(value).expect("deserialize element descriptor");

    assert_eq!(decoded.physical_tags, ["shape:round", "position:left"]);
}

#[test]
fn element_physical_tags_round_trip_through_plugin_cbor_codec() {
    let encoded = encode_cbor(&element_descriptor()).expect("encode element descriptor");
    let decoded: ElementDescriptor = decode_cbor(&encoded).expect("decode element descriptor");

    assert_eq!(decoded.physical_tags, ["shape:round", "position:left"]);
}

#[test]
fn missing_element_physical_tags_default_to_empty() {
    let mut value = serde_json::to_value(element_descriptor()).expect("serialize descriptor");
    value
        .as_object_mut()
        .expect("element descriptor serializes as an object")
        .remove("physical_tags");
    let decoded: ElementDescriptor =
        serde_json::from_value(value).expect("deserialize older element descriptor");

    assert!(decoded.physical_tags.is_empty());
}

#[test]
fn state_snapshot_writer_never_overruns_a_small_buffer() {
    let snapshot = PluginStateSnapshot::default();
    let mut expected = Vec::new();
    ciborium::into_writer(&snapshot, &mut expected).expect("serialize snapshot");
    let mut output = [0x5a_u8; 4];
    // SAFETY: `output` is a writable buffer of the declared capacity.
    let required = unsafe { write_state_snapshot(output.as_mut_ptr(), output.len(), &snapshot) };
    assert_eq!(required, expected.len());
    assert_eq!(output, [0x5a; 4]);
}

#[test]
fn plugin_targets_have_stable_compact_paths() {
    let targets = [
        (
            PluginTarget::Device {
                device: "lamp".to_owned(),
            },
            "lamp",
        ),
        (
            PluginTarget::Surface {
                device: "lamp".to_owned(),
                surface: "shade".to_owned(),
            },
            "lamp/shade",
        ),
        (
            PluginTarget::Element {
                device: "lamp".to_owned(),
                surface: "shade".to_owned(),
                element: "left".to_owned(),
            },
            "lamp/shade/left",
        ),
        (
            PluginTarget::Group {
                device: "lamp".to_owned(),
                group: "reading".to_owned(),
            },
            "lamp/group:reading",
        ),
    ];

    for (target, path) in targets {
        assert_eq!(target.device_id(), "lamp");
        assert_eq!(target.to_string(), path);
    }
}
