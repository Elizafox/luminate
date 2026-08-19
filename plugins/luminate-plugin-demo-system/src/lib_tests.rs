// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Ownership and device-specific update tests for the synthetic system.

use luminate_core::colour::Colour;
use luminate_core::rgb::Rgb;
use luminate_plugin_api::PluginTarget;

use super::*;

fn device_target(device: &str) -> PluginTarget {
    PluginTarget::Device {
        device: device.to_owned(),
    }
}

fn black() -> Colour {
    Colour::rgb(Rgb::new(0, 0, 0))
}

#[test]
fn save_current_is_rejected_for_ordinary_device() {
    let update = PluginUpdate {
        target: device_target("demo-mouse"),
        operation: PluginUpdateOperation::SaveCurrent,
    };

    assert!(!log_update(&update));
}

#[test]
fn cpu_cooler_rejects_pure_black_static_colour() {
    let update = PluginUpdate {
        target: device_target("demo-cpu-cooler"),
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static { colour: black() },
        },
    };

    assert!(!log_update(&update));
}

#[test]
fn cpu_cooler_accepts_non_black_colour() {
    let update = PluginUpdate {
        target: device_target("demo-cpu-cooler"),
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: Colour::rgb(Rgb { r: 10, g: 0, b: 0 }),
            },
        },
    };

    assert!(log_update(&update));
}

#[test]
fn other_devices_accept_pure_black_colour() {
    let update = PluginUpdate {
        target: device_target("demo-mouse"),
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static { colour: black() },
        },
    };

    assert!(log_update(&update));
}

#[test]
fn ensure_owned_device_rejects_an_unknown_device_id() {
    let error =
        ensure_owned_device("not-a-demo-device").expect_err("unowned device should be rejected");

    assert!(matches!(error, PluginError::InvalidTarget(_)));
}

#[test]
fn ensure_owned_device_accepts_every_declared_device() {
    for device in demo_topology() {
        ensure_owned_device(&device.id).expect("declared device should be accepted");
    }
}

#[test]
fn addressable_strip_uses_surface_scoped_physical_form() {
    let strip = addressable_strip_device();

    assert!(strip.physical_tags.is_empty());
    assert_eq!(strip.surfaces[0].physical_tags, ["shape:flexible-strip"]);
}

#[test]
fn monochrome_brightness_scopes_match_each_target() {
    let power_supply = power_supply_device();
    let surface = &power_supply.surfaces[0];
    let element = &surface.elements[0];

    assert_eq!(
        brightness_scope(&power_supply.capabilities),
        CapabilityScope::Device
    );
    assert_eq!(
        brightness_scope(&surface.capabilities),
        CapabilityScope::Surface
    );
    assert_eq!(
        brightness_scope(&element.capabilities),
        CapabilityScope::Element
    );
}

#[test]
fn every_capability_scope_matches_its_target() {
    for device in demo_topology() {
        assert_capability_scope(&device.capabilities, CapabilityScope::Device, &device.id);
        for surface in device.surfaces {
            assert_capability_scope(&surface.capabilities, CapabilityScope::Surface, &surface.id);
            for element in surface.elements {
                assert_capability_scope(
                    &element.capabilities,
                    CapabilityScope::Element,
                    &element.id,
                );
            }
        }
        for group in device.groups {
            assert_capability_scope(&group.capabilities, CapabilityScope::Device, &group.id);
        }
    }
}

fn assert_capability_scope(capabilities: &CapabilitySet, expected: CapabilityScope, target: &str) {
    let brightness = match capabilities.brightness {
        BrightnessCapability::Independent { scope, .. } => Some(scope),
        BrightnessCapability::None => None,
    };
    let scopes = brightness
        .into_iter()
        .chain(capabilities.frame_upload.as_ref().map(|value| value.scope))
        .chain(
            capabilities
                .hardware_effects
                .as_ref()
                .map(|value| value.scope),
        )
        .chain(
            capabilities
                .physical_power
                .as_ref()
                .map(|value| value.scope),
        )
        .chain(
            capabilities
                .appearance_slots
                .iter()
                .flat_map(|slots| slots.slots.iter())
                .filter_map(|slot| slot.appearance.hardware_effects.as_ref())
                .map(|effects| effects.scope),
        );

    for scope in scopes {
        assert_eq!(scope, expected, "capability scope for {target}");
    }
}

fn brightness_scope(capabilities: &CapabilitySet) -> CapabilityScope {
    match capabilities.brightness {
        BrightnessCapability::Independent { scope, .. } => scope,
        BrightnessCapability::None => panic!("expected independent brightness"),
    }
}

#[test]
fn monitor_exposes_coupled_stored_power_indicator_appearances() {
    let monitor = demo_topology()
        .into_iter()
        .find(|device| device.id == "demo-monitor")
        .expect("demo monitor");
    let indicator = monitor
        .surfaces
        .iter()
        .find(|surface| surface.id == "power-indicator")
        .expect("power indicator surface");
    let slots = indicator
        .capabilities
        .appearance_slots
        .as_ref()
        .expect("stored appearance slots");

    assert_eq!(
        slots.update_policy,
        AppearanceSlotUpdatePolicy::PartialIfKnown
    );
    assert_eq!(
        slots
            .slots
            .iter()
            .map(|slot| slot.id.as_str())
            .collect::<Vec<_>>(),
        ["active", "standby"]
    );
    assert!(indicator.capabilities.colour.is_empty());
    assert!(indicator.capabilities.hardware_effects.is_none());
    assert!(slots.slots.iter().all(|slot| {
        slot.appearance.colour == [ColourCapability::rgb8()]
            && slot.appearance.cct_emulation == CctEmulation::Disabled
            && matches!(
                slot.persistence,
                PersistenceCapability::CurrentState {
                    requirement: PersistenceRequirement::Required,
                    explicit_commit: false,
                    readback: false,
                }
            )
    }));
}
