// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use luminate_core::appearance_slot::{
    AppearanceCapability, AppearanceSlotDescriptor, AppearanceSlotId, AppearanceSlotUpdatePolicy,
    AppearanceSlotValue, AppearanceSlotsCapability,
};
use luminate_core::capability::{
    BrightnessCapability, CapabilityScope, CctEmulation, ColourCapability, ColourChannel,
    ColourEncoding, EffectChoice, EffectDirection, EffectParameter, HardwareEffectDescriptor,
    HardwareEffectId, HardwareEffectsCapability, PersistenceCapability,
};
use luminate_core::colour::{Colour, ColourChannelValue};
use luminate_core::device::DeviceId;
use luminate_core::effect::{Effect, EffectArguments};
use luminate_core::element::ElementId;
use luminate_core::rgb::Rgb;
use luminate_core::surface::SurfaceId;
use luminate_core::target::TargetId;
use luminate_core::util::DiscreteRange;

use crate::error::DaemonError;
use crate::state::DaemonState;

use super::super::tests_support::*;

pub(crate) fn appearance_slot_state(policy: AppearanceSlotUpdatePolicy) -> (DaemonState, TargetId) {
    let mut descriptor = demo_device_descriptor();
    descriptor.surfaces[0].capabilities.appearance_slots = Some(AppearanceSlotsCapability {
        slots: ["ac", "battery"]
            .into_iter()
            .map(|id| AppearanceSlotDescriptor {
                id: AppearanceSlotId::new(id),
                name: id.to_owned(),
                appearance: AppearanceCapability {
                    colour: vec![ColourCapability::rgb8()],
                    cct_emulation: CctEmulation::Disabled,
                    hardware_effects: None,
                },
                persistence: PersistenceCapability::None,
                notes: Vec::new(),
                warnings: Vec::new(),
            })
            .collect(),
        update_policy: policy,
    });
    let target = TargetId::Surface {
        device: DeviceId::new(&descriptor.id),
        surface: SurfaceId::new(&descriptor.surfaces[0].id),
    };
    (
        DaemonState::from_descriptors(&[descriptor]).expect("valid slotted topology"),
        target,
    )
}

pub(crate) fn slot(id: &str, effect: Effect) -> AppearanceSlotValue {
    AppearanceSlotValue {
        slot: AppearanceSlotId::new(id),
        effect,
    }
}

#[test]
fn appearance_slot_mutation_rejects_invalid_collections() {
    let (state, target) = appearance_slot_state(AppearanceSlotUpdatePolicy::CompleteSet);
    let effect = Effect::Static {
        colour: Colour::rgb(Rgb::new(1, 2, 3)),
    };
    for values in [
        Vec::new(),
        vec![slot("ac", effect.clone()), slot("ac", effect.clone())],
        vec![slot("ac", effect.clone()), slot("unknown", effect.clone())],
        vec![slot("ac", effect.clone())],
    ] {
        assert!(matches!(
            state.validate_appearance_slot_values(&target, &values),
            Err(DaemonError::InvalidArgument { .. })
        ));
    }
}

#[test]
fn appearance_slot_mutation_validates_every_effect_and_preserves_order() {
    let (state, target) = appearance_slot_state(AppearanceSlotUpdatePolicy::CompleteSet);
    let values = vec![
        slot("battery", Effect::Off),
        slot(
            "ac",
            Effect::Static {
                colour: Colour::rgb(Rgb::new(1, 2, 3)),
            },
        ),
    ];
    assert_eq!(
        state
            .validate_appearance_slot_values(&target, &values)
            .expect("complete supported mutation"),
        values
    );
    assert!(matches!(
        state.validate_appearance_slot_values(
            &target,
            &[
                slot("ac", Effect::Rainbow { period_ms: 500 }),
                slot("battery", Effect::Off)
            ],
        ),
        Err(DaemonError::UnsupportedCapability { .. })
    ));
}

#[test]
fn partial_if_known_requires_complete_values_until_slot_state_exists() {
    let (mut state, target) = appearance_slot_state(AppearanceSlotUpdatePolicy::PartialIfKnown);
    assert!(matches!(
        state.validate_appearance_slot_values(&target, &[slot("ac", Effect::Off)]),
        Err(DaemonError::UnknownState { .. })
    ));
    state
        .set_appearance_slots(
            target.clone(),
            vec![slot("ac", Effect::Off), slot("battery", Effect::Off)],
        )
        .expect("store complete slot state");
    assert_eq!(
        state
            .validate_appearance_slot_values(
                &target,
                &[slot(
                    "ac",
                    Effect::Static {
                        colour: Colour::rgb(Rgb::new(3, 2, 1)),
                    },
                )],
            )
            .expect("fill coupled value"),
        vec![
            slot(
                "ac",
                Effect::Static {
                    colour: Colour::rgb(Rgb::new(3, 2, 1)),
                },
            ),
            slot("battery", Effect::Off),
        ]
    );
}

#[test]
fn colour_validation_requires_exact_shape_and_range() {
    let state = validation_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));

    state
        .validate_colour(&target, &Colour::rgb(Rgb::new(1, 2, 3)))
        .expect("valid RGB colour");

    let wrong_encoding = Colour::monochrome(1);
    assert!(matches!(
        state.validate_colour(&target, &wrong_encoding),
        Err(DaemonError::InvalidArgument { .. })
    ));

    let duplicate = Colour::additive(vec![
        ColourChannelValue::new(ColourChannel::Red, 1),
        ColourChannelValue::new(ColourChannel::Red, 2),
        ColourChannelValue::new(ColourChannel::Blue, 3),
    ]);
    assert!(duplicate.is_err());

    let overflow = Colour::additive(vec![
        ColourChannelValue::new(ColourChannel::Red, 256),
        ColourChannelValue::new(ColourChannel::Green, 2),
        ColourChannelValue::new(ColourChannel::Blue, 3),
    ])
    .expect("channels are structurally valid");
    assert!(matches!(
        state.validate_colour(&target, &overflow),
        Err(DaemonError::InvalidArgument { .. })
    ));
}

#[test]
fn absent_colour_and_brightness_are_unsupported() {
    let state = demo_state();
    let target = TargetId::Element {
        device: DeviceId::new("demo-kbd"),
        surface: SurfaceId::new("main"),
        element: ElementId::new("topology-only"),
    };

    assert!(matches!(
        state.validate_colour(&target, &Colour::rgb(Rgb::new(1, 2, 3))),
        Err(DaemonError::UnsupportedCapability { .. })
    ));
    assert!(matches!(
        state.validate_brightness(&target, 0),
        Err(DaemonError::UnsupportedCapability { .. })
    ));
}

#[test]
fn brightness_validation_enforces_scope_and_maximum() {
    let state = validation_state();
    let device = TargetId::Device(DeviceId::new("demo-kbd"));
    state
        .validate_brightness(&device, 100)
        .expect("advertised maximum is valid");
    assert!(matches!(
        state.validate_brightness(&device, 101),
        Err(DaemonError::InvalidArgument { .. })
    ));

    let mut descriptor = demo_device_descriptor();
    descriptor.capabilities.brightness = BrightnessCapability::Independent {
        bits: 8,
        maximum: 255,
        scope: CapabilityScope::Surface,
    };
    let mismatched = DaemonState::from_descriptors(&[descriptor]).expect("valid topology");
    assert!(matches!(
        mismatched.validate_brightness(&device, 1),
        Err(DaemonError::UnsupportedCapability { .. })
    ));
}

#[test]
fn effect_validation_enforces_support_and_bounded_parameters() {
    let state = validation_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));

    state
        .validate_effect(
            &target,
            &Effect::Morph {
                colours: vec![Rgb::new(1, 2, 3)],
                period_ms: 500,
            },
        )
        .expect("bounded advertised morph is valid");
    assert!(matches!(
        state.validate_effect(
            &target,
            &Effect::Morph {
                colours: Vec::new(),
                period_ms: 500,
            }
        ),
        Err(DaemonError::InvalidArgument { .. })
    ));
    assert!(matches!(
        state.validate_effect(
            &target,
            &Effect::Morph {
                colours: vec![Rgb::new(1, 2, 3); 4],
                period_ms: 500,
            }
        ),
        Err(DaemonError::InvalidArgument { .. })
    ));
    assert!(matches!(
        state.validate_effect(
            &target,
            &Effect::Morph {
                colours: vec![Rgb::new(1, 2, 3)],
                period_ms: 550,
            }
        ),
        Err(DaemonError::InvalidArgument { .. })
    ));
    assert!(matches!(
        state.validate_effect(
            &target,
            &Effect::Scanner {
                colour: Rgb::new(1, 2, 3),
                period_ms: 500,
            }
        ),
        Err(DaemonError::UnsupportedCapability { .. })
    ));
}

#[test]
fn static_effect_must_match_target_colour_model() {
    let state = validation_state();
    let surface = TargetId::Surface {
        device: DeviceId::new("demo-kbd"),
        surface: SurfaceId::new("main"),
    };
    assert!(matches!(
        state.validate_effect(
            &surface,
            &Effect::Static {
                colour: Colour::rgb(Rgb::new(1, 2, 3)),
            }
        ),
        Err(DaemonError::InvalidArgument { .. })
    ));
}

/// A device advertising a vendor-specific `Hardware` effect: a `Choice` of
/// named scenes plus a bounded `Speed`, with no typed equivalent.
fn hardware_effect_state() -> DaemonState {
    use luminate_core::capability::CapabilitySet;

    let mut descriptor = demo_device_descriptor();
    descriptor.capabilities = CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![HardwareEffectDescriptor {
                id: HardwareEffectId::new("scene-show"),
                name: "Scene Show".to_owned(),
                parameters: vec![
                    EffectParameter::Choice {
                        options: vec![
                            EffectChoice {
                                id: "aurora".to_owned(),
                                name: "Aurora".to_owned(),
                            },
                            EffectChoice {
                                id: "nebula".to_owned(),
                                name: "Nebula".to_owned(),
                            },
                        ],
                    },
                    EffectParameter::Speed {
                        range: DiscreteRange::new(1, 10, 1),
                    },
                ],
            }],
            scope: CapabilityScope::Device,
            concurrent_with_streaming: false,
        }),
        ..CapabilitySet::default()
    };
    DaemonState::from_descriptors(&[descriptor]).expect("hardware effect state should build")
}

fn scene(choice: Option<&str>, speed: Option<u16>) -> Effect {
    Effect::Hardware {
        id: HardwareEffectId::new("scene-show"),
        arguments: EffectArguments {
            choice: choice.map(str::to_owned),
            speed,
            ..EffectArguments::default()
        },
    }
}

#[test]
fn hardware_effect_accepts_advertised_arguments() {
    let state = hardware_effect_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));
    state
        .validate_effect(&target, &scene(Some("aurora"), Some(5)))
        .expect("advertised scene id with in-range speed is valid");
}

#[test]
fn hardware_effect_rejects_unadvertised_id() {
    let state = hardware_effect_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));
    let unknown = Effect::Hardware {
        id: HardwareEffectId::new("does-not-exist"),
        arguments: EffectArguments::default(),
    };
    assert!(matches!(
        state.validate_effect(&target, &unknown),
        Err(DaemonError::UnsupportedCapability { .. })
    ));
}

#[test]
fn hardware_effect_enforces_choice_and_speed_bounds() {
    let state = hardware_effect_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));

    // Missing the required choice argument.
    assert!(matches!(
        state.validate_effect(&target, &scene(None, Some(5))),
        Err(DaemonError::InvalidArgument { .. })
    ));
    // Choice not among the advertised options.
    assert!(matches!(
        state.validate_effect(&target, &scene(Some("sunset"), Some(5))),
        Err(DaemonError::InvalidArgument { .. })
    ));
    // Speed outside the advertised range.
    assert!(matches!(
        state.validate_effect(&target, &scene(Some("aurora"), Some(99))),
        Err(DaemonError::InvalidArgument { .. })
    ));
}

#[test]
fn cct_emulation_resolves_to_an_approximate_additive_colour() {
    // `validation_state` advertises `Additive` colour with `CctEmulation::Auto`
    // and no native `Cct` capability, so a `Cct` request must resolve to a
    // kelvin-derived `Additive` colour rather than being rejected outright.
    let state = validation_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));
    let resolved = state
        .resolve_colour(&target, &Colour::cct(4_000))
        .expect("Auto emulation should approximate Cct via the additive capability");
    assert_eq!(resolved.encoding(), ColourEncoding::Additive);
}

#[test]
fn cct_emulation_disabled_rejects_cct_requests() {
    use luminate_core::capability::CapabilitySet;

    let mut descriptor = demo_device_descriptor();
    descriptor.capabilities = CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Disabled,
        ..CapabilitySet::default()
    };
    let state = DaemonState::from_descriptors(&[descriptor]).expect("valid topology");
    let target = TargetId::Device(DeviceId::new("demo-kbd"));

    assert!(matches!(
        state.resolve_colour(&target, &Colour::cct(4_000)),
        Err(DaemonError::InvalidArgument { .. })
    ));
}

#[test]
fn set_cct_emulation_override_takes_effect_on_the_next_resolution() {
    use luminate_core::capability::CapabilitySet;

    // With the target's own `cct_emulation` left at `Auto`, a daemon-wide
    // override to `Disabled` must still block emulation.
    let mut descriptor = demo_device_descriptor();
    descriptor.capabilities = CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        ..CapabilitySet::default()
    };
    let mut state = DaemonState::from_descriptors(&[descriptor]).expect("valid topology");
    let target = TargetId::Device(DeviceId::new("demo-kbd"));

    state
        .resolve_colour(&target, &Colour::cct(4_000))
        .expect("Auto emulation resolves before any override is installed");

    state.set_cct_emulation_override(Some(CctEmulation::Disabled));
    assert!(matches!(
        state.resolve_colour(&target, &Colour::cct(4_000)),
        Err(DaemonError::InvalidArgument { .. })
    ));
}

#[test]
fn colour_channel_count_mismatch_is_rejected() {
    let state = validation_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));

    // The device advertises 3-channel `Additive` (rgb8); a 2-channel value
    // of the same encoding must fail on channel count, not on encoding.
    let too_few = Colour::additive(vec![
        ColourChannelValue::new(ColourChannel::Red, 1),
        ColourChannelValue::new(ColourChannel::Green, 2),
    ])
    .expect("channels are structurally valid");
    assert!(matches!(
        state.validate_colour(&target, &too_few),
        Err(DaemonError::InvalidArgument { .. })
    ));
}

#[test]
fn colour_channel_not_advertised_is_rejected() {
    let state = validation_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));

    // Same encoding, same channel count, but `White` is not among the
    // device's advertised `Additive` channels (Red, Green, Blue).
    let unadvertised_channel = Colour::additive(vec![
        ColourChannelValue::new(ColourChannel::Red, 1),
        ColourChannelValue::new(ColourChannel::Green, 2),
        ColourChannelValue::new(ColourChannel::White, 3),
    ])
    .expect("channels are structurally valid");
    assert!(matches!(
        state.validate_colour(&target, &unadvertised_channel),
        Err(DaemonError::InvalidArgument { .. })
    ));
}

#[test]
fn colour_channel_with_invalid_bit_width_is_unsupported() {
    use luminate_core::capability::CapabilitySet;

    // A `bits: 0` channel capability can't represent any value, so
    // validation must reject it as unsupported hardware rather than
    // silently accepting or panicking on the shift.
    let mut descriptor = demo_device_descriptor();
    descriptor.capabilities = CapabilitySet {
        colour: vec![ColourCapability::Monochrome { bits: 0 }],
        cct_emulation: CctEmulation::Auto,
        ..CapabilitySet::default()
    };
    let state = DaemonState::from_descriptors(&[descriptor]).expect("valid topology");
    let target = TargetId::Device(DeviceId::new("demo-kbd"));

    assert!(matches!(
        state.validate_colour(&target, &Colour::monochrome(1)),
        Err(DaemonError::UnsupportedCapability { .. })
    ));
}

#[test]
fn brightness_with_no_independent_capability_is_unsupported() {
    let state = demo_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));
    assert!(matches!(
        state.validate_brightness(&target, 0),
        Err(DaemonError::UnsupportedCapability { .. })
    ));
}

#[test]
fn brightness_with_invalid_bit_width_is_unsupported() {
    let mut descriptor = demo_device_descriptor();
    descriptor.capabilities.brightness = BrightnessCapability::Independent {
        bits: 0,
        maximum: 10,
        scope: CapabilityScope::Device,
    };
    let state = DaemonState::from_descriptors(&[descriptor]).expect("valid topology");
    let target = TargetId::Device(DeviceId::new("demo-kbd"));
    assert!(matches!(
        state.validate_brightness(&target, 0),
        Err(DaemonError::UnsupportedCapability { .. })
    ));
}

#[test]
fn brightness_maximum_beyond_bit_width_is_unsupported() {
    // 4 bits represents at most 15; advertising a maximum of 100 is an
    // internally-inconsistent capability the daemon must reject rather
    // than trust.
    let mut descriptor = demo_device_descriptor();
    descriptor.capabilities.brightness = BrightnessCapability::Independent {
        bits: 4,
        maximum: 100,
        scope: CapabilityScope::Device,
    };
    let state = DaemonState::from_descriptors(&[descriptor]).expect("valid topology");
    let target = TargetId::Device(DeviceId::new("demo-kbd"));
    assert!(matches!(
        state.validate_brightness(&target, 5),
        Err(DaemonError::UnsupportedCapability { .. })
    ));
}

#[test]
fn off_effect_is_allowed_when_colour_is_advertised() {
    let state = demo_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));
    state
        .validate_effect(&target, &Effect::Off)
        .expect("a colour-capable target can always be turned off");
}

#[test]
fn off_effect_is_unsupported_with_no_colour_or_matching_hardware_effect() {
    let state = demo_state();
    let target = TargetId::Element {
        device: DeviceId::new("demo-kbd"),
        surface: SurfaceId::new("main"),
        element: ElementId::new("topology-only"),
    };
    assert!(matches!(
        state.validate_effect(&target, &Effect::Off),
        Err(DaemonError::UnsupportedCapability { .. })
    ));
}

#[test]
fn rainbow_effect_is_validated_against_its_descriptor() {
    // `Rainbow` takes no colour argument at all (`colour_count` is
    // `None`), so it exercises a distinct match arm from the
    // fixed/variable-colour effects covered elsewhere in this suite.
    let state = validation_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));
    assert!(matches!(
        state.validate_effect(&target, &Effect::Rainbow { period_ms: 500 }),
        Err(DaemonError::UnsupportedCapability { .. })
    ));
}

#[test]
fn effect_descriptor_missing_a_bound_wire_parameter_is_unsupported() {
    use luminate_core::capability::{CapabilitySet, HardwareEffectsCapability};

    // `Breathe` carries both a colour and a period, but this descriptor
    // only bounds the colour parameter; `validate_effect_parameters` must
    // reject it rather than silently accepting the unbounded duration.
    let mut descriptor = demo_device_descriptor();
    descriptor.capabilities = CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![HardwareEffectDescriptor {
                id: HardwareEffectId::new("breathe"),
                name: "Breathe".to_owned(),
                parameters: vec![EffectParameter::Colour {
                    minimum_colours: 1,
                    maximum_colours: 1,
                }],
            }],
            scope: CapabilityScope::Device,
            concurrent_with_streaming: false,
        }),
        ..CapabilitySet::default()
    };
    let state = DaemonState::from_descriptors(&[descriptor]).expect("valid topology");
    let target = TargetId::Device(DeviceId::new("demo-kbd"));
    assert!(matches!(
        state.validate_effect(
            &target,
            &Effect::Breathe {
                colour: Rgb::new(1, 2, 3),
                period_ms: 500,
            }
        ),
        Err(DaemonError::UnsupportedCapability { .. })
    ));
}

/// A hardware effect descriptor declaring one of every `EffectParameter`
/// kind, for exercising `validate_hardware_arguments` and
/// `reject_undeclared_arguments` branch-by-branch.
fn full_hardware_effect_state() -> DaemonState {
    use luminate_core::capability::{CapabilitySet, HardwareEffectsCapability};

    let mut descriptor = demo_device_descriptor();
    descriptor.capabilities = CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![HardwareEffectDescriptor {
                id: HardwareEffectId::new("full"),
                name: "Full".to_owned(),
                parameters: vec![
                    EffectParameter::Colour {
                        minimum_colours: 1,
                        maximum_colours: 1,
                    },
                    EffectParameter::Speed {
                        range: DiscreteRange::new(1, 10, 1),
                    },
                    EffectParameter::Direction {
                        values: vec![EffectDirection::Forward, EffectDirection::Reverse],
                    },
                    EffectParameter::Duration {
                        milliseconds: DiscreteRange::new(100, 1_000, 100),
                    },
                    EffectParameter::Brightness { bits: 8 },
                    EffectParameter::Choice {
                        options: vec![EffectChoice {
                            id: "solo".to_owned(),
                            name: "Solo".to_owned(),
                        }],
                    },
                ],
            }],
            scope: CapabilityScope::Device,
            concurrent_with_streaming: false,
        }),
        ..CapabilitySet::default()
    };
    DaemonState::from_descriptors(&[descriptor]).expect("valid topology")
}

fn full_effect(arguments: EffectArguments) -> Effect {
    Effect::Hardware {
        id: HardwareEffectId::new("full"),
        arguments,
    }
}

fn valid_full_arguments() -> EffectArguments {
    EffectArguments {
        colours: vec![Rgb::new(1, 2, 3)],
        speed: Some(5),
        direction: Some(EffectDirection::Forward),
        duration_ms: Some(500),
        brightness: Some(200),
        choice: Some("solo".to_owned()),
    }
}

#[test]
fn hardware_effect_accepts_every_declared_argument_kind() {
    let state = full_hardware_effect_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));
    state
        .validate_effect(&target, &full_effect(valid_full_arguments()))
        .expect("every argument is within its declared bound");
}

#[test]
fn hardware_effect_requires_each_declared_argument() {
    let state = full_hardware_effect_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));

    let missing_direction = EffectArguments {
        direction: None,
        ..valid_full_arguments()
    };
    assert!(matches!(
        state.validate_effect(&target, &full_effect(missing_direction)),
        Err(DaemonError::InvalidArgument { .. })
    ));

    let missing_duration = EffectArguments {
        duration_ms: None,
        ..valid_full_arguments()
    };
    assert!(matches!(
        state.validate_effect(&target, &full_effect(missing_duration)),
        Err(DaemonError::InvalidArgument { .. })
    ));

    let missing_brightness = EffectArguments {
        brightness: None,
        ..valid_full_arguments()
    };
    assert!(matches!(
        state.validate_effect(&target, &full_effect(missing_brightness)),
        Err(DaemonError::InvalidArgument { .. })
    ));
}

#[test]
fn hardware_effect_rejects_out_of_range_declared_arguments() {
    let state = full_hardware_effect_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));

    let bad_direction = EffectArguments {
        direction: Some(EffectDirection::Clockwise),
        ..valid_full_arguments()
    };
    assert!(matches!(
        state.validate_effect(&target, &full_effect(bad_direction)),
        Err(DaemonError::InvalidArgument { .. })
    ));

    let bad_duration = EffectArguments {
        duration_ms: Some(50),
        ..valid_full_arguments()
    };
    assert!(matches!(
        state.validate_effect(&target, &full_effect(bad_duration)),
        Err(DaemonError::InvalidArgument { .. })
    ));

    let bad_brightness = EffectArguments {
        brightness: Some(9_999),
        ..valid_full_arguments()
    };
    assert!(matches!(
        state.validate_effect(&target, &full_effect(bad_brightness)),
        Err(DaemonError::InvalidArgument { .. })
    ));
}

#[test]
fn hardware_effect_with_invalid_brightness_bit_width_is_unsupported() {
    use luminate_core::capability::{CapabilitySet, HardwareEffectsCapability};

    let mut descriptor = demo_device_descriptor();
    descriptor.capabilities = CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![HardwareEffectDescriptor {
                id: HardwareEffectId::new("bad-brightness"),
                name: "Bad Brightness".to_owned(),
                parameters: vec![EffectParameter::Brightness { bits: 0 }],
            }],
            scope: CapabilityScope::Device,
            concurrent_with_streaming: false,
        }),
        ..CapabilitySet::default()
    };
    let state = DaemonState::from_descriptors(&[descriptor]).expect("valid topology");
    let target = TargetId::Device(DeviceId::new("demo-kbd"));
    let effect = Effect::Hardware {
        id: HardwareEffectId::new("bad-brightness"),
        arguments: EffectArguments {
            brightness: Some(1),
            ..EffectArguments::default()
        },
    };
    assert!(matches!(
        state.validate_effect(&target, &effect),
        Err(DaemonError::UnsupportedCapability { .. })
    ));
}

#[test]
fn hardware_effect_rejects_each_kind_of_undeclared_argument() {
    // `scene-show` (from `hardware_effect_state`) declares only `Choice`
    // and `Speed`; supplying any other argument kind must be rejected by
    // `reject_undeclared_arguments`, one branch at a time.
    let state = hardware_effect_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));

    let with_direction = Effect::Hardware {
        id: HardwareEffectId::new("scene-show"),
        arguments: EffectArguments {
            choice: Some("aurora".to_owned()),
            speed: Some(5),
            direction: Some(EffectDirection::Forward),
            ..EffectArguments::default()
        },
    };
    assert!(matches!(
        state.validate_effect(&target, &with_direction),
        Err(DaemonError::InvalidArgument { .. })
    ));

    let with_duration = Effect::Hardware {
        id: HardwareEffectId::new("scene-show"),
        arguments: EffectArguments {
            choice: Some("aurora".to_owned()),
            speed: Some(5),
            duration_ms: Some(500),
            ..EffectArguments::default()
        },
    };
    assert!(matches!(
        state.validate_effect(&target, &with_duration),
        Err(DaemonError::InvalidArgument { .. })
    ));

    let with_brightness = Effect::Hardware {
        id: HardwareEffectId::new("scene-show"),
        arguments: EffectArguments {
            choice: Some("aurora".to_owned()),
            speed: Some(5),
            brightness: Some(10),
            ..EffectArguments::default()
        },
    };
    assert!(matches!(
        state.validate_effect(&target, &with_brightness),
        Err(DaemonError::InvalidArgument { .. })
    ));
}

#[test]
fn hardware_effect_rejects_undeclared_arguments() {
    let state = hardware_effect_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));

    // `scene-show` declares no colour parameter, so supplying colours is an error.
    let extra = Effect::Hardware {
        id: HardwareEffectId::new("scene-show"),
        arguments: EffectArguments {
            choice: Some("aurora".to_owned()),
            speed: Some(5),
            colours: vec![Rgb::new(1, 2, 3)],
            ..EffectArguments::default()
        },
    };
    assert!(matches!(
        state.validate_effect(&target, &extra),
        Err(DaemonError::InvalidArgument { .. })
    ));
}

#[test]
fn shared_appearance_validator_covers_portable_effect_boundaries() {
    let target = TargetId::Surface {
        device: DeviceId::new("slots"),
        surface: SurfaceId::new("power-button"),
    };
    let rgb = AppearanceCapability {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        hardware_effects: None,
    };

    for effect in [
        Effect::Off,
        Effect::Static {
            colour: Colour::rgb(Rgb::new(1, 2, 3)),
        },
        Effect::Static {
            colour: Colour::cct(4_000),
        },
    ] {
        super::validate_effect_for_appearance(&target, &effect, &rgb, None)
            .expect("RGB appearance should accept off, static RGB, and emulated CCT");
    }

    for effect in [
        Effect::Static {
            colour: Colour::monochrome(1),
        },
        Effect::Rainbow { period_ms: 1_000 },
    ] {
        assert!(super::validate_effect_for_appearance(&target, &effect, &rgb, None).is_err());
    }
}

#[test]
fn shared_appearance_validator_preserves_cct_override_semantics() {
    let target = TargetId::Surface {
        device: DeviceId::new("slots"),
        surface: SurfaceId::new("power-button"),
    };
    let appearance = AppearanceCapability {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        hardware_effects: None,
    };
    let effect = Effect::Static {
        colour: Colour::cct(4_000),
    };

    assert!(
        super::validate_effect_for_appearance(
            &target,
            &effect,
            &appearance,
            Some(CctEmulation::Disabled),
        )
        .is_err()
    );
}
