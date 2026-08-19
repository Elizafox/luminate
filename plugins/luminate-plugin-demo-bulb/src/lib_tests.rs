// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Persistence, target-validation, and single-pixel frame tests for the bulb.

use super::*;
use luminate_core::colour;
use luminate_core::rgb::Rgb;

fn device_target() -> PluginTarget {
    PluginTarget::Device {
        device: DEVICE_ID.to_owned(),
    }
}

#[test]
fn topology_has_single_persisting_device() {
    let device = bulb_device();

    assert_eq!(device.id, DEVICE_ID);
    assert_eq!(device.physical_tags, ["shape:a19"]);
    assert!(device.surfaces.is_empty());
    assert!(device.groups.is_empty());
    assert!(matches!(
        device.capabilities.persistence,
        PersistenceCapability::CurrentState {
            requirement: PersistenceRequirement::Required,
            ..
        }
    ));
    assert!(matches!(
        device.capabilities.state_readback,
        StateReadbackCapability::None
    ));
}

#[test]
fn save_current_is_accepted_as_durable_no_op() {
    let update = PluginUpdate {
        target: device_target(),
        operation: PluginUpdateOperation::SaveCurrent,
    };

    apply_update(&update).expect("write-through bulb should accept save-current");
}

#[test]
fn accepts_device_colour_update() {
    let update = PluginUpdate {
        target: device_target(),
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: colour::Colour::rgb(Rgb::new(10, 20, 30)),
            },
        },
    };

    apply_update(&update).expect("device colour update should be accepted");
}

#[test]
fn rejects_non_device_target() {
    let update = PluginUpdate {
        target: PluginTarget::Surface {
            device: DEVICE_ID.to_owned(),
            surface: "panel".to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };

    let error = apply_update(&update).expect_err("surface target should be rejected");
    assert!(error.contains("device-scoped"));
}

#[test]
fn rejects_foreign_device() {
    let update = PluginUpdate {
        target: PluginTarget::Device {
            device: "demo-keyboard".to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };

    let error = apply_update(&update).expect_err("foreign device should be rejected");
    assert!(error.contains("not owned"));
}

fn full_frame(pixels: Vec<colour::Colour>) -> FrameEnvelope {
    FrameEnvelope {
        generation: 1,
        sequence: 0,
        payload: FramePayload::Full(pixels),
        commit: false,
    }
}

#[test]
fn accepts_a_single_pixel_full_frame() {
    let envelope = full_frame(vec![colour::Colour::rgb(Rgb::new(10, 20, 30))]);

    apply_frame(&device_target(), &envelope).expect("single-pixel full frame should apply");
}

#[test]
fn rejects_a_frame_with_more_than_one_pixel() {
    let envelope = full_frame(vec![
        colour::Colour::rgb(Rgb::new(1, 2, 3)),
        colour::Colour::rgb(Rgb::new(4, 5, 6)),
    ]);

    let error =
        apply_frame(&device_target(), &envelope).expect_err("a two-pixel frame should be rejected");
    assert!(error.contains("exactly 1 pixel"));
}

#[test]
fn rejects_a_partial_frame() {
    let envelope = FrameEnvelope {
        generation: 1,
        sequence: 0,
        payload: FramePayload::Partial(vec![(0, colour::Colour::rgb(Rgb::new(1, 2, 3)))]),
        commit: false,
    };

    let error =
        apply_frame(&device_target(), &envelope).expect_err("partial frame should be rejected");
    assert!(error.contains("full frames"));
}

#[test]
fn rejects_a_frame_for_a_foreign_device() {
    let envelope = full_frame(vec![colour::Colour::rgb(Rgb::new(1, 2, 3))]);
    let target = PluginTarget::Device {
        device: "demo-keyboard".to_owned(),
    };

    let error =
        apply_frame(&target, &envelope).expect_err("foreign device frame should be rejected");
    assert!(error.contains("not owned"));
}
