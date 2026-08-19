// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use luminate_core::appearance_slot::{
    AppearanceCapability, AppearanceSlotDescriptor, AppearanceSlotId, AppearanceSlotUpdatePolicy,
    AppearanceSlotsCapability,
};
use luminate_core::capability::{
    CapabilityScope, ColourChannel, ColourChannelCapability, EffectChoice, EffectDirection,
    HardwareEffectDescriptor, HardwareEffectId, PersistenceRequirement,
};
use luminate_core::element::ElementKind;
use luminate_core::surface::SurfaceKind;
use luminate_core::util::discrete::DiscreteRange;
use luminate_plugin_api::{ElementDescriptor, HardwareBus, HardwareClaim, SurfaceDescriptor};

use super::*;

fn descriptor(id: &str) -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: id.to_owned(),
        name: id.to_owned(),
        vendor: None,
        model: None,
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

fn descriptor_with_slot(appearance: AppearanceCapability) -> DeviceDescriptor {
    let mut device = descriptor("slots");
    device.surfaces.push(SurfaceDescriptor {
        id: "power-button".to_owned(),
        name: "Power button".to_owned(),
        kind: SurfaceKind::Opaque,
        physical_tags: Vec::new(),
        elements: Vec::new(),
        capabilities: CapabilitySet {
            appearance_slots: Some(AppearanceSlotsCapability {
                slots: vec![AppearanceSlotDescriptor {
                    id: AppearanceSlotId::new("ac"),
                    name: "AC".to_owned(),
                    appearance,
                    persistence: PersistenceCapability::None,
                    notes: Vec::new(),
                    warnings: Vec::new(),
                }],
                update_policy: AppearanceSlotUpdatePolicy::Independent,
            }),
            ..CapabilitySet::default()
        },
        notes: Vec::new(),
        warnings: Vec::new(),
    });
    device
}

#[test]
fn topology_contract_accepts_ordered_independent_physical_tags() {
    let mut device = descriptor("tagged");
    device.physical_tags = vec![
        "shape:flexible-strip".to_owned(),
        "vendor:example".to_owned(),
    ];
    device.surfaces.push(SurfaceDescriptor {
        id: "main".to_owned(),
        name: "Main".to_owned(),
        kind: SurfaceKind::Opaque,
        physical_tags: vec!["layout:wrapped-horizontal".to_owned()],
        elements: vec![ElementDescriptor {
            id: "led".to_owned(),
            name: None,
            kind: ElementKind::Led,
            geometry: None,
            physical_tags: vec![
                "shape:flexible-strip".to_owned(),
                "position:left".to_owned(),
            ],
            capabilities: CapabilitySet::default(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        capabilities: CapabilitySet::default(),
        notes: Vec::new(),
        warnings: Vec::new(),
    });

    validate_topology_contract(
        &[device],
        PluginCallbacks {
            apply: false,
            read_state: false,
            frame_upload: false,
            shm_frame: false,
        },
    )
    .expect("valid physical tags should satisfy the plugin contract");
}

#[test]
fn topology_contract_rejects_invalid_physical_tags_at_every_scope() {
    let callbacks = PluginCallbacks {
        apply: false,
        read_state: false,
        frame_upload: false,
        shm_frame: false,
    };

    for tags in [
        vec![String::new()],
        vec![" shape:cylinder".to_owned()],
        vec!["shape:cylinder ".to_owned()],
        vec!["shape:cylinder".to_owned(), "shape:cylinder".to_owned()],
    ] {
        let mut device = descriptor("device-tags");
        device.physical_tags = tags.clone();
        validate_topology_contract(&[device], callbacks)
            .expect_err("invalid device tags should violate the plugin contract");

        let mut device = descriptor("surface-tags");
        device.surfaces.push(SurfaceDescriptor {
            id: "main".to_owned(),
            name: "Main".to_owned(),
            kind: SurfaceKind::Opaque,
            physical_tags: tags.clone(),
            elements: Vec::new(),
            capabilities: CapabilitySet::default(),
            notes: Vec::new(),
            warnings: Vec::new(),
        });
        validate_topology_contract(&[device], callbacks)
            .expect_err("invalid surface tags should violate the plugin contract");

        let mut device = descriptor("element-tags");
        device.surfaces.push(SurfaceDescriptor {
            id: "main".to_owned(),
            name: "Main".to_owned(),
            kind: SurfaceKind::Opaque,
            physical_tags: Vec::new(),
            elements: vec![ElementDescriptor {
                id: "led".to_owned(),
                name: None,
                kind: ElementKind::Led,
                geometry: None,
                physical_tags: tags,
                capabilities: CapabilitySet::default(),
                notes: Vec::new(),
                warnings: Vec::new(),
            }],
            capabilities: CapabilitySet::default(),
            notes: Vec::new(),
            warnings: Vec::new(),
        });
        validate_topology_contract(&[device], callbacks)
            .expect_err("invalid element tags should violate the plugin contract");
    }
}

#[test]
fn slot_appearance_requires_an_update_callback() {
    let device = descriptor_with_slot(AppearanceCapability::default());
    let error = validate_topology_contract(
        &[device],
        PluginCallbacks {
            apply: false,
            read_state: false,
            frame_upload: false,
            shm_frame: false,
        },
    )
    .expect_err("appearance slots are mutable even with an empty appearance declaration");

    assert!(error.to_string().contains("no update callback"));
}

#[test]
fn slot_appearance_reuses_colour_contract_validation() {
    let device = descriptor_with_slot(AppearanceCapability {
        colour: vec![ColourCapability::Additive(vec![
            ColourChannelCapability::new(ColourChannel::Red, 8),
            ColourChannelCapability::new(ColourChannel::Red, 8),
        ])],
        ..AppearanceCapability::default()
    });
    let error = validate_topology_contract(
        &[device],
        PluginCallbacks {
            apply: true,
            read_state: false,
            frame_upload: false,
            shm_frame: false,
        },
    )
    .expect_err("duplicate slot colour channels must be rejected");

    assert!(error.to_string().contains("appearance slot ac"));
}

#[test]
fn slot_appearance_reuses_hardware_effect_contract_validation() {
    let device = descriptor_with_slot(AppearanceCapability {
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![HardwareEffectDescriptor {
                id: HardwareEffectId::new(""),
                name: "Effect".to_owned(),
                parameters: Vec::new(),
            }],
            scope: CapabilityScope::Surface,
            concurrent_with_streaming: false,
        }),
        ..AppearanceCapability::default()
    });
    let error = validate_topology_contract(
        &[device],
        PluginCallbacks {
            apply: true,
            read_state: false,
            frame_upload: false,
            shm_frame: false,
        },
    )
    .expect_err("malformed slot hardware effects must be rejected");

    assert!(error.to_string().contains("appearance slot ac"));
    assert!(error.to_string().contains("empty id"));
}

#[test]
fn rejects_invalid_colour_capabilities() {
    let callbacks = PluginCallbacks {
        apply: true,
        read_state: false,
        frame_upload: false,
        shm_frame: false,
    };

    let mut duplicate = descriptor("duplicate");
    duplicate.capabilities.colour = vec![ColourCapability::Additive(vec![
        ColourChannelCapability::new(ColourChannel::Red, 8),
        ColourChannelCapability::new(ColourChannel::Red, 8),
    ])];
    assert!(validate_topology_contract(&[duplicate], callbacks).is_err());

    for bits in [0, 33] {
        let mut invalid = descriptor("invalid-bits");
        invalid.capabilities.colour = vec![ColourCapability::Monochrome { bits }];
        assert!(validate_topology_contract(&[invalid], callbacks).is_err());
    }
}

fn claim(identity: &str, domain: &str, exclusivity: ClaimExclusivity) -> HardwareClaim {
    HardwareClaim {
        bus: HardwareBus::Hid,
        physical_identity: identity.to_owned(),
        control_domain: domain.to_owned(),
        exclusivity,
    }
}

#[test]
fn rejects_empty_or_non_normalized_claim_fields() {
    let mut empty_identity = descriptor("a");
    empty_identity
        .claims
        .push(claim("", "lighting", ClaimExclusivity::Shared));
    assert!(validate_hardware_claims(&[empty_identity]).is_err());

    let mut padded_identity = descriptor("a");
    padded_identity
        .claims
        .push(claim(" 1234:5678 ", "lighting", ClaimExclusivity::Shared));
    assert!(validate_hardware_claims(&[padded_identity]).is_err());

    let mut empty_domain = descriptor("a");
    empty_domain
        .claims
        .push(claim("1234:5678", "", ClaimExclusivity::Shared));
    assert!(validate_hardware_claims(&[empty_domain]).is_err());

    let mut padded_domain = descriptor("a");
    padded_domain
        .claims
        .push(claim("1234:5678", " lighting", ClaimExclusivity::Shared));
    assert!(validate_hardware_claims(&[padded_domain]).is_err());
}

#[test]
fn rejects_a_claim_repeated_within_one_device() {
    let mut device = descriptor("a");
    device
        .claims
        .push(claim("1234:5678", "lighting", ClaimExclusivity::Shared));
    device
        .claims
        .push(claim("1234:5678", "lighting", ClaimExclusivity::Shared));
    assert!(validate_hardware_claims(&[device]).is_err());
}

#[test]
fn shared_claims_on_the_same_hardware_across_devices_coexist() {
    let mut first = descriptor("a");
    first
        .claims
        .push(claim("1234:5678", "lighting", ClaimExclusivity::Shared));
    let mut second = descriptor("b");
    second
        .claims
        .push(claim("1234:5678", "lighting", ClaimExclusivity::Shared));
    validate_hardware_claims(&[first, second]).expect("two shared claims may coexist");
}

#[test]
fn an_exclusive_claim_conflicts_with_any_other_claim_on_the_same_hardware() {
    let mut first = descriptor("a");
    first
        .claims
        .push(claim("1234:5678", "lighting", ClaimExclusivity::Exclusive));
    let mut second = descriptor("b");
    second
        .claims
        .push(claim("1234:5678", "lighting", ClaimExclusivity::Shared));
    assert!(
        validate_hardware_claims(&[first.clone(), second]).is_err(),
        "exclusive vs. shared on the same hardware must conflict"
    );

    let mut third = descriptor("c");
    third
        .claims
        .push(claim("1234:5678", "lighting", ClaimExclusivity::Exclusive));
    assert!(
        validate_hardware_claims(&[first, third]).is_err(),
        "two exclusive claims on the same hardware must conflict"
    );
}

#[test]
fn readback_advertised_through_persistence_requires_a_read_callback() {
    let mut device = descriptor("readable");
    device.capabilities.persistence = PersistenceCapability::CurrentState {
        requirement: PersistenceRequirement::Optional,
        explicit_commit: false,
        readback: true,
    };
    let error = validate_topology_contract(
        &[device],
        PluginCallbacks {
            apply: true,
            read_state: false,
            frame_upload: false,
            shm_frame: false,
        },
    )
    .expect_err("persistence readback without a read-state callback must fail");
    assert!(error.to_string().contains("no state-readback callback"));
}

#[test]
fn explicit_persistence_commit_requires_an_update_callback() {
    let mut device = descriptor("commits");
    device.capabilities.persistence = PersistenceCapability::CurrentState {
        requirement: PersistenceRequirement::Optional,
        explicit_commit: true,
        readback: false,
    };
    let error = validate_topology_contract(
        &[device],
        PluginCallbacks {
            apply: false,
            read_state: false,
            frame_upload: false,
            shm_frame: false,
        },
    )
    .expect_err("explicit persistence commit without an update callback must fail");
    assert!(error.to_string().contains("explicit persistence commit"));
}

fn effect(parameters: Vec<EffectParameter>) -> HardwareEffectsCapability {
    HardwareEffectsCapability {
        effects: vec![HardwareEffectDescriptor {
            id: HardwareEffectId::new("effect"),
            name: "Effect".to_owned(),
            parameters,
        }],
        scope: CapabilityScope::Device,
        concurrent_with_streaming: false,
    }
}

#[test]
fn effect_contract_rejects_an_empty_effect_id() {
    let mut device = descriptor("a");
    device.capabilities.hardware_effects = Some(HardwareEffectsCapability {
        effects: vec![HardwareEffectDescriptor {
            id: HardwareEffectId::new(""),
            name: "Effect".to_owned(),
            parameters: Vec::new(),
        }],
        scope: CapabilityScope::Device,
        concurrent_with_streaming: false,
    });
    let error = validate_topology_contract(
        &[device],
        PluginCallbacks {
            apply: true,
            read_state: false,
            frame_upload: false,
            shm_frame: false,
        },
    )
    .expect_err("an empty effect id must be rejected");
    assert!(error.to_string().contains("empty id"));
}

#[test]
fn effect_contract_rejects_duplicate_effect_ids() {
    let mut device = descriptor("a");
    let duplicate = HardwareEffectDescriptor {
        id: HardwareEffectId::new("same"),
        name: "Effect".to_owned(),
        parameters: Vec::new(),
    };
    device.capabilities.hardware_effects = Some(HardwareEffectsCapability {
        effects: vec![duplicate.clone(), duplicate],
        scope: CapabilityScope::Device,
        concurrent_with_streaming: false,
    });
    let error = validate_topology_contract(
        &[device],
        PluginCallbacks {
            apply: true,
            read_state: false,
            frame_upload: false,
            shm_frame: false,
        },
    )
    .expect_err("a duplicate effect id must be rejected");
    assert!(error.to_string().contains("duplicate hardware effect id"));
}

#[test]
fn effect_contract_rejects_an_empty_effect_name() {
    let mut device = descriptor("a");
    device.capabilities.hardware_effects = Some(HardwareEffectsCapability {
        effects: vec![HardwareEffectDescriptor {
            id: HardwareEffectId::new("effect"),
            name: String::new(),
            parameters: Vec::new(),
        }],
        scope: CapabilityScope::Device,
        concurrent_with_streaming: false,
    });
    let error = validate_topology_contract(
        &[device],
        PluginCallbacks {
            apply: true,
            read_state: false,
            frame_upload: false,
            shm_frame: false,
        },
    )
    .expect_err("an empty effect name must be rejected");
    assert!(error.to_string().contains("empty name"));
}

#[test]
fn effect_contract_validates_every_parameter_shape() {
    let valid_parameters = [
        EffectParameter::Colour {
            minimum_colours: 1,
            maximum_colours: 2,
        },
        EffectParameter::Speed {
            range: DiscreteRange::new(0, 10, 1),
        },
        EffectParameter::Duration {
            milliseconds: DiscreteRange::new(0, 1000, 10),
        },
        EffectParameter::Direction {
            values: vec![EffectDirection::Forward],
        },
        EffectParameter::Choice {
            options: vec![EffectChoice {
                id: "one".to_owned(),
                name: "One".to_owned(),
            }],
        },
        EffectParameter::Brightness { bits: 8 },
    ];
    for parameter in valid_parameters {
        let mut device = descriptor("valid");
        device.capabilities.hardware_effects = Some(effect(vec![parameter.clone()]));
        validate_topology_contract(
            &[device],
            PluginCallbacks {
                apply: true,
                read_state: false,
                frame_upload: false,
                shm_frame: false,
            },
        )
        .unwrap_or_else(|error| panic!("valid parameter {parameter:?} rejected: {error}"));
    }

    let invalid_parameters = [
        EffectParameter::Colour {
            minimum_colours: 2,
            maximum_colours: 1,
        },
        EffectParameter::Speed {
            range: DiscreteRange::new(0, 10, 0),
        },
        EffectParameter::Duration {
            milliseconds: DiscreteRange::new(1000, 0, 10),
        },
        EffectParameter::Direction { values: Vec::new() },
        EffectParameter::Choice {
            options: Vec::new(),
        },
        EffectParameter::Brightness { bits: 0 },
    ];
    for parameter in invalid_parameters {
        let mut device = descriptor("invalid");
        device.capabilities.hardware_effects = Some(effect(vec![parameter.clone()]));
        assert!(
            validate_topology_contract(
                &[device],
                PluginCallbacks {
                    apply: true,
                    read_state: false,
                    frame_upload: false,
                    shm_frame: false
                }
            )
            .is_err(),
            "invalid parameter {parameter:?} should be rejected"
        );
    }
}

#[test]
fn effect_contract_rejects_duplicate_choice_ids() {
    let mut device = descriptor("a");
    device.capabilities.hardware_effects = Some(effect(vec![EffectParameter::Choice {
        options: vec![
            EffectChoice {
                id: "one".to_owned(),
                name: "One".to_owned(),
            },
            EffectChoice {
                id: "one".to_owned(),
                name: "One again".to_owned(),
            },
        ],
    }]));
    let error = validate_topology_contract(
        &[device],
        PluginCallbacks {
            apply: true,
            read_state: false,
            frame_upload: false,
            shm_frame: false,
        },
    )
    .expect_err("duplicate choice ids must be rejected");
    assert!(error.to_string().contains("invalid or duplicate choices"));
}
