// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Topology and target-validation tests for the standalone demo keyboard.

use super::*;
use luminate_core::effect::Effect;
use luminate_core::rgb::Rgb;

#[test]
fn topology_has_keyboard_shape() {
    let device = keyboard_device();

    assert_eq!(device.id, DEVICE_ID);
    assert_eq!(device.surfaces.len(), 1);
    assert_eq!(device.surfaces[0].elements.len(), KEYS.len());
    assert!(device.groups.iter().any(|group| group.id == "wasd"));
    assert!(device.groups.iter().any(|group| group.id == "arrows"));
}

#[test]
fn accepts_known_key_update() {
    let update = PluginUpdate {
        target: PluginTarget::Element {
            device: DEVICE_ID.to_owned(),
            surface: SURFACE_ID.to_owned(),
            element: "escape".to_owned(),
        },
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: Colour::rgb(Rgb::new(0xff, 0, 0)),
            },
        },
    };

    apply_update(&update).expect("known key should be accepted");
}

#[test]
fn rejects_unknown_key_update() {
    let update = PluginUpdate {
        target: PluginTarget::Element {
            device: DEVICE_ID.to_owned(),
            surface: SURFACE_ID.to_owned(),
            element: "macro-9000".to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };

    let error = apply_update(&update).expect_err("unknown key should be rejected");
    assert!(error.contains("unknown demo keyboard key"));
}

#[test]
fn save_current_is_rejected() {
    let update = PluginUpdate {
        target: PluginTarget::Device {
            device: DEVICE_ID.to_owned(),
        },
        operation: PluginUpdateOperation::SaveCurrent,
    };

    let error = apply_update(&update).expect_err("save-current should be rejected");
    assert!(error.contains("save-current"));
}
