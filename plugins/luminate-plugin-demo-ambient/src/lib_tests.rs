// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Readback, emission-state, and target-validation tests for the ambient panel.

use super::*;
use luminate_core::rgb::Rgb;
use std::sync::Mutex;

// Tests share the plugin's process-global state.
static SHADOW_STATE_LOCK: Mutex<()> = Mutex::new(());

fn surface_target() -> PluginTarget {
    PluginTarget::Surface {
        device: DEVICE_ID.to_owned(),
        surface: SURFACE_ID.to_owned(),
    }
}

#[test]
fn topology_has_exact_panel_surface_readback() {
    let device = ambient_device();

    assert_eq!(device.id, DEVICE_ID);
    assert_eq!(device.surfaces.len(), 1);
    assert_eq!(device.surfaces[0].id, SURFACE_ID);
    assert!(matches!(
        &device.surfaces[0].capabilities.state_readback,
        StateReadbackCapability::Readable {
            facets,
            ..
        }
        if facets.iter().all(|facet| facet.fidelity == ReadbackFidelity::Exact)
    ));
}

#[test]
fn accepts_panel_surface_update() {
    let _fixture = SHADOW_STATE_LOCK.lock().unwrap();
    let update = PluginUpdate {
        target: surface_target(),
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: colour::Colour::rgb(Rgb::new(10, 20, 30)),
            },
        },
    };

    apply_update(&update).expect("panel surface update should be accepted");
}

#[test]
fn state_snapshot_reports_requested_shadow_facets() {
    let _fixture = SHADOW_STATE_LOCK.lock().unwrap();
    let target = surface_target();
    apply_update(&PluginUpdate {
        target: target.clone(),
        operation: PluginUpdateOperation::SetBrightness { value: 23 },
    })
    .expect("apply brightness");
    let snapshot = read_snapshot(&PluginReadRequest {
        targets: vec![luminate_plugin_api::PluginReadTarget {
            target,
            facets: vec![StateFacetKind::Brightness, StateFacetKind::Emission],
        }],
    });
    assert!(snapshot.errors.is_empty());
    assert!(
        snapshot
            .observations
            .iter()
            .any(|observation| observation.value == FacetValue::Brightness(23))
    );
}

#[test]
fn off_effect_preserves_configured_appearance_and_darkens_emission() {
    let _fixture = SHADOW_STATE_LOCK.lock().unwrap();
    let target = surface_target();
    apply_update(&PluginUpdate {
        target: target.clone(),
        operation: PluginUpdateOperation::Clear,
    })
    .expect("clear");
    let red = colour::Colour::rgb(Rgb::new(255, 0, 0));
    apply_update(&PluginUpdate {
        target: target.clone(),
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: red.clone(),
            },
        },
    })
    .expect("apply colour");
    apply_update(&PluginUpdate {
        target: target.clone(),
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Off,
        },
    })
    .expect("apply off");

    let snapshot = read_snapshot(&PluginReadRequest {
        targets: vec![luminate_plugin_api::PluginReadTarget {
            target,
            facets: vec![StateFacetKind::Appearance, StateFacetKind::Emission],
        }],
    });

    assert!(
        snapshot
            .observations
            .iter()
            .any(|observation| observation.value
                == FacetValue::Appearance(AppearanceState::Static(red.clone()))),
        "Off must not overwrite the configured appearance"
    );
    assert!(
        snapshot
            .observations
            .iter()
            .any(|observation| observation.value == FacetValue::Emission(EmissionState::Dark)),
        "Off must darken emission"
    );
}

#[test]
fn brightness_restored_to_nonzero_re_emits_without_a_separate_command() {
    let _fixture = SHADOW_STATE_LOCK.lock().unwrap();
    let target = surface_target();
    apply_update(&PluginUpdate {
        target: target.clone(),
        operation: PluginUpdateOperation::Clear,
    })
    .expect("clear");
    apply_update(&PluginUpdate {
        target: target.clone(),
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: colour::Colour::rgb(Rgb::new(0, 255, 0)),
            },
        },
    })
    .expect("apply colour");
    apply_update(&PluginUpdate {
        target: target.clone(),
        operation: PluginUpdateOperation::SetBrightness { value: 0 },
    })
    .expect("apply zero brightness");
    apply_update(&PluginUpdate {
        target: target.clone(),
        operation: PluginUpdateOperation::SetBrightness { value: 50 },
    })
    .expect("apply nonzero brightness");

    let snapshot = read_snapshot(&PluginReadRequest {
        targets: vec![luminate_plugin_api::PluginReadTarget {
            target,
            facets: vec![StateFacetKind::Emission],
        }],
    });

    assert!(
        snapshot
            .observations
            .iter()
            .any(|observation| observation.value == FacetValue::Emission(EmissionState::Emitting)),
        "restoring nonzero brightness must re-emit without any other command"
    );
}

#[test]
fn rejects_unknown_surface() {
    let update = PluginUpdate {
        target: PluginTarget::Surface {
            device: DEVICE_ID.to_owned(),
            surface: "backlight".to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };

    let error = apply_update(&update).expect_err("unknown surface should be rejected");
    assert!(error.contains("unknown demo ambient panel surface"));
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
