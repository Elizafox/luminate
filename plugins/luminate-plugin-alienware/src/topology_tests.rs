// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Descriptor and capability tests for supported Alienware topology.

use super::*;
use crate::aw_elc_profile::{AW_ELC_VENDOR_ID, PROFILES, resolve};

fn m16_r2_identity() -> AwElcIdentity {
    resolve(0x187c, 0x0551, 0x1102, 5).expect("the registered m16 R2 identity should resolve")
}

#[test]
fn unknown_layout_has_no_per_key_elements() {
    let device = keyboard_device(None);

    assert!(device.surfaces[0].elements.is_empty());
    assert_eq!(device.physical_tags, ["shape:keyboard", "form:laptop"]);
}

#[test]
fn known_layout_exposes_one_element_per_named_key() {
    let layout = KeyboardLayoutId::M16R2UsAnsi;
    let device = keyboard_device(Some(layout));

    assert_eq!(
        device.physical_tags,
        ["shape:keyboard", "layout:us-ansi", "form:laptop"]
    );
    let elements = &device.surfaces[0].elements;
    assert_eq!(elements.len(), 85);
    assert!(elements.iter().any(|element| element.id == "escape"));
    assert!(elements.iter().any(|element| element.id == "space"));
    assert!(
        elements
            .iter()
            .all(|element| matches!(element.kind, ElementKind::Key))
    );
    assert!(elements.iter().all(|element| {
        element.capabilities.colour == vec![ColourCapability::rgb8()]
            && matches!(element.capabilities.brightness, BrightnessCapability::None)
            && element.capabilities.hardware_effects.is_none()
    }));
    let expected_order = layout
        .key_map()
        .expect("known layout has a key map")
        .named_positions()
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    let published_order = elements
        .iter()
        .map(|element| element.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(published_order, expected_order);
}

#[test]
fn recognized_keyboard_surface_advertises_volatile_full_frames() {
    let device = keyboard_device(Some(KeyboardLayoutId::M16R2UsAnsi));
    let capability = device.surfaces[0]
        .capabilities
        .frame_upload
        .as_ref()
        .expect("recognized layout should advertise frame streaming");

    assert_eq!(capability.scope, CapabilityScope::Surface);
    assert_eq!(capability.update_mode, FrameUpdateMode::FullFrameOnly);
    assert_eq!(capability.max_rate_hz, Some(15));
    assert!(!capability.atomic);
    assert_eq!(capability.buffering, BufferingMode::Immediate);
    assert!(device.capabilities.frame_upload.is_none());
}

#[test]
fn unknown_keyboard_layout_does_not_advertise_frame_streaming() {
    let layout = KeyboardLayoutId::Unknown {
        hardware_variant: [0xff, 0xff],
        layout: 0xff,
        chassis_colour: 0xff,
    };
    let device = keyboard_device(Some(layout));

    assert!(device.surfaces[0].capabilities.frame_upload.is_none());
}

#[test]
fn keyboard_advertises_real_brightness_range() {
    let device = keyboard_device(None);
    assert!(matches!(
        device.capabilities.brightness,
        BrightnessCapability::Independent {
            bits: 7,
            maximum: 100,
            scope: CapabilityScope::Device,
        }
    ));
}

#[test]
fn keyboard_advertises_only_its_custom_fixed_cadence_effects() {
    let device = keyboard_device(None);
    let effects = device.capabilities.hardware_effects.as_ref();

    // The vendor sweep is exposed as the custom `aw-sweeper` hardware
    // effect, taking exactly the one colour the firmware animates.
    let sweeper = effects.and_then(|capability| {
        capability
            .effects
            .iter()
            .find(|effect| effect.id.as_str() == "aw-sweeper")
    });
    assert!(
        sweeper.is_some_and(|effect| effect.parameters
            == vec![EffectParameter::Colour {
                minimum_colours: 1,
                maximum_colours: 1,
            }]),
        "keyboard should advertise aw-sweeper with a single-colour parameter"
    );

    // None of these built-in programs accepts the duration required by the
    // portable wire effects. They must remain behind Alienware-specific
    // IDs, rather than claiming generic scanner/breathe/etc. support.
    assert!(
        effects.is_some_and(
            |capability| capability.effects.iter().all(|effect| !matches!(
                effect.id.as_str(),
                "scanner" | "breathe" | "pulse" | "spectrum" | "rainbow"
            ))
        ),
        "keyboard should not advertise fixed-cadence firmware programs as portable effects"
    );

    let ids = effects
        .expect("keyboard effects")
        .effects
        .iter()
        .map(|effect| effect.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            AW_KEYBOARD_BREATHE_EFFECT_ID,
            AW_KEYBOARD_PULSE_EFFECT_ID,
            AW_KEYBOARD_RAINBOW_EFFECT_ID,
            "aw-sweeper",
            AW_KEYBOARD_SPECTRUM_EFFECT_ID,
        ]
    );
}

#[test]
fn aw_elc_power_targets_do_not_advertise_live_effects() {
    let device = aw_elc_device(m16_r2_identity());
    assert!(device.capabilities.hardware_effects.is_none());
    assert!(
        device
            .groups
            .iter()
            .find(|group| group.id == "all")
            .expect("all group")
            .capabilities
            .hardware_effects
            .is_none()
    );
    let live_effects = device
        .groups
        .iter()
        .find(|group| group.id == "live-zones")
        .and_then(|group| group.capabilities.hardware_effects.as_ref())
        .expect("live-zone effects");
    assert!(
        live_effects
            .effects
            .iter()
            .any(|effect| effect.id.as_str() == "morph")
    );
    assert!(
        !live_effects
            .effects
            .iter()
            .any(|effect| effect.id.as_str() == "breathing")
    );
    assert!(
        live_effects.effects.iter().all(|effect| !matches!(
            effect.id.as_str(),
            "breathe" | "pulse" | "spectrum" | "rainbow"
        )),
        "AW-ELC fixed-cadence firmware programs must not masquerade as portable effects"
    );
    assert!(
        live_effects
            .effects
            .iter()
            .any(|effect| effect.id.as_str() == "morph"),
        "morph remains portable because AW-ELC honours its colour sequence and duration"
    );
}

#[test]
fn aw_elc_surfaces_publish_composable_physical_tags() {
    let device = aw_elc_device(m16_r2_identity());
    let tags = |id| {
        device
            .surfaces
            .iter()
            .find(|surface| surface.id == id)
            .expect("AW-ELC surface")
            .physical_tags
            .as_slice()
    };

    assert_eq!(
        tags(AW_ELC_SURFACE_TRACKPAD_RING),
        ["shape:ring", "form:trackpad-surround"]
    );
    assert_eq!(
        tags(AW_ELC_SURFACE_REAR_LOGO),
        [
            "shape:logo",
            "form:laptop-lid",
            "alienware:shape/alien-head"
        ]
    );
    assert_eq!(
        tags(AW_ELC_SURFACE_POWER_BUTTON),
        [
            "shape:logo",
            "form:power-button",
            "alienware:shape/alien-head"
        ]
    );
}

#[test]
fn aw_elc_persistence_is_split_between_live_and_power_paths() {
    let device = aw_elc_device(m16_r2_identity());
    assert!(matches!(
        device.capabilities.persistence,
        PersistenceCapability::None
    ));

    let live_group = device
        .groups
        .iter()
        .find(|group| group.id == "live-zones")
        .expect("live-zones group");
    assert!(matches!(
        live_group.capabilities.persistence,
        PersistenceCapability::CurrentState {
            requirement: PersistenceRequirement::Optional,
            explicit_commit: true,
            readback: false,
        }
    ));

    let all_group = device
        .groups
        .iter()
        .find(|group| group.id == "all")
        .expect("all group");
    assert!(matches!(
        all_group.capabilities.persistence,
        PersistenceCapability::None
    ));

    let power_button = device
        .surfaces
        .iter()
        .find(|surface| surface.id == AW_ELC_SURFACE_POWER_BUTTON)
        .expect("power-button surface");
    let slots = power_button
        .capabilities
        .appearance_slots
        .as_ref()
        .expect("power-button appearance slots");
    assert_eq!(
        slots.update_policy,
        AppearanceSlotUpdatePolicy::PartialIfKnown
    );
    assert_eq!(slots.slots.len(), 2);
    assert_eq!(slots.slots[0].id.as_str(), AW_ELC_SLOT_AC);
    assert_eq!(slots.slots[1].id.as_str(), AW_ELC_SLOT_BATTERY);
    assert!(slots.slots.iter().all(|slot| matches!(
        slot.persistence,
        PersistenceCapability::CurrentState {
            requirement: PersistenceRequirement::Required,
            explicit_commit: false,
            readback: false,
        }
    )));
}

#[test]
fn m16_r2_descriptor_identity_remains_compatible() {
    let device = aw_elc_device(m16_r2_identity());

    assert_eq!(device.name, "Alienware AW-ELC Lighting Controller");
    assert_eq!(device.model.as_deref(), Some("187C:0551 AW-ELC"));
    assert_eq!(device.claims[0].physical_identity, "187c:0551");
    assert!(device.notes.is_empty());
    assert!(device.warnings.is_empty());
}

#[test]
fn imported_profiles_publish_declared_opaque_surfaces() {
    for profile in PROFILES {
        let identity = resolve(
            AW_ELC_VENDOR_ID,
            profile.product_id,
            profile.platform_id.expect("imported profile has an ID"),
            profile.expected_raw_zone_count,
        )
        .expect("imported profile should resolve");
        let device = aw_elc_device(identity);
        let expected_ids = profile
            .zones
            .iter()
            .map(|zone| zone.surface_id)
            .collect::<Vec<_>>();
        let actual_ids = device
            .surfaces
            .iter()
            .map(|surface| surface.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(actual_ids, expected_ids, "{}", profile.model);
        assert!(device.surfaces.iter().all(|surface| {
            surface.kind == SurfaceKind::Opaque
                && surface.elements.is_empty()
                && matches!(
                    surface.capabilities.persistence,
                    PersistenceCapability::None
                )
                && surface.capabilities.appearance_slots.is_none()
        }));
        let all = &device.groups[0];
        assert_eq!(all.id, "all");
        assert_eq!(all.members.len(), profile.zones.len());
        assert!(matches!(
            all.capabilities.persistence,
            PersistenceCapability::None
        ));
        assert_eq!(device.claims[0].physical_identity, "187c:0550");
        assert!(device.warnings[0].contains("not been hardware-validated"));
        assert!(device.notes[0].contains("5e6d627f519a791487e766aeee51eed7bf7129ef"));
    }
}

#[test]
fn imported_profile_tags_preserve_keyboard_and_light_bar_distinctions() {
    let profile = PROFILES
        .iter()
        .find(|profile| profile.platform_id == Some(0x0a01))
        .expect("G7 profile should be registered");
    let identity = resolve(AW_ELC_VENDOR_ID, profile.product_id, 0x0a01, 16)
        .expect("G7 profile should resolve");
    let device = aw_elc_device(identity);

    assert_eq!(
        device.surfaces[1].physical_tags,
        ["form:laptop", "shape:keyboard-zone"]
    );
    assert_eq!(
        device.surfaces[4].physical_tags,
        ["form:laptop-light-bar", "shape:segment"]
    );
}
