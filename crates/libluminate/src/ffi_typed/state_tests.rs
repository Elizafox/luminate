// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

fn bytes(view: LuminateStringView) -> Vec<u8> {
    if view.data.is_null() {
        return Vec::new();
    }

    // SAFETY: typed accessors return a view into a value kept alive by
    // the test for the duration of this copy.
    unsafe { std::slice::from_raw_parts(view.data.cast(), view.len) }.to_vec()
}

#[test]
fn collection_appearance_payloads_match_their_discriminants() {
    let colour = Colour::hsv(10, 20, 30);
    let effect = Effect::Rainbow { period_ms: 400 };
    let configured = [
        AppearanceState::Static(colour.clone()),
        AppearanceState::Effect(effect.clone()),
        AppearanceState::Mixed,
    ];
    let effective = [
        EffectiveAppearanceState::Off,
        EffectiveAppearanceState::Static(colour),
        EffectiveAppearanceState::Effect(effect),
        EffectiveAppearanceState::Streaming,
        EffectiveAppearanceState::Mixed,
    ];

    // SAFETY: every snapshot and all payloads remain live for the calls.
    unsafe {
        let unknown = CollectionStateStatus {
            collection: CollectionId::new("desk"),
            appearance: None,
            effective_appearance: None,
        };
        let unknown = cast_ref(&unknown);
        assert_eq!(luminate_collection_state_appearance_kind(unknown), u32::MAX);
        assert!(luminate_collection_state_appearance_colour(unknown).is_null());
        assert!(luminate_collection_state_appearance_effect(unknown).is_null());
        assert_eq!(
            luminate_collection_state_effective_appearance_kind(unknown),
            u32::MAX
        );
        assert!(luminate_collection_state_effective_appearance_colour(unknown).is_null());
        assert!(luminate_collection_state_effective_appearance_effect(unknown).is_null());

        for (value, kind) in configured.into_iter().zip(0_u32..) {
            let snapshot = CollectionStateStatus {
                collection: CollectionId::new("desk"),
                appearance: Some(AggregateAppearanceObservation {
                    value,
                    confidence: ObservationConfidence::Confirmed,
                    observed_at_ms: 1,
                    stale: false,
                }),
                effective_appearance: None,
            };
            let snapshot = cast_ref(&snapshot);
            assert_eq!(luminate_collection_state_appearance_kind(snapshot), kind);
            assert_eq!(
                luminate_collection_state_appearance_colour(snapshot).is_null(),
                kind != 0
            );
            assert_eq!(
                luminate_collection_state_appearance_effect(snapshot).is_null(),
                kind != 1
            );
        }

        for (value, kind) in effective.into_iter().zip(0_u32..) {
            let snapshot = CollectionStateStatus {
                collection: CollectionId::new("desk"),
                appearance: None,
                effective_appearance: Some(AggregateEffectiveAppearanceObservation {
                    value,
                    confidence: ObservationConfidence::Confirmed,
                    observed_at_ms: 1,
                    stale: false,
                }),
            };
            let snapshot = cast_ref(&snapshot);
            assert_eq!(
                luminate_collection_state_effective_appearance_kind(snapshot),
                kind
            );
            assert_eq!(
                luminate_collection_state_effective_appearance_colour(snapshot).is_null(),
                kind != 1
            );
            assert_eq!(
                luminate_collection_state_effective_appearance_effect(snapshot).is_null(),
                kind != 2
            );
        }

        assert!(luminate_collection_state_appearance_colour(ptr::null()).is_null());
        assert!(luminate_collection_state_appearance_effect(ptr::null()).is_null());
        assert!(luminate_collection_state_effective_appearance_colour(ptr::null()).is_null());
        assert!(luminate_collection_state_effective_appearance_effect(ptr::null()).is_null());
    }
}

#[test]
fn zero_brightness_and_incomplete_slots_are_present_values() {
    let brightness = FacetValue::Brightness(0);
    let brightness = cast_ref::<_, LuminateFacetValue>(&brightness);
    let slots = FacetValue::AppearanceSlots(AppearanceSlotsState {
        values: Vec::new(),
        complete: false,
    });
    let slots = cast_ref::<_, LuminateFacetValue>(&slots);
    let mut brightness_value = u32::MAX;
    let mut complete = true;

    // SAFETY: both facet views and outputs remain live for each call.
    unsafe {
        assert!(luminate_facet_value_brightness(
            brightness,
            &raw mut brightness_value
        ));
        assert!(luminate_facet_value_appearance_slots_complete(
            slots,
            &raw mut complete
        ));
        assert_eq!(brightness_value, 0);
        assert!(!complete);

        brightness_value = 19;
        complete = true;
        assert!(!luminate_facet_value_brightness(
            slots,
            &raw mut brightness_value
        ));
        assert!(!luminate_facet_value_appearance_slots_complete(
            brightness,
            &raw mut complete
        ));
    }
    assert_eq!((brightness_value, complete), (19, true));
}

#[test]
fn colour_enumeration_covers_every_encoding_and_preserves_outputs_on_failure() {
    let colours = [
        Colour::rgb(Rgb::new(10, 20, 30)),
        Colour::hsv(100, 200, 300),
        Colour::hsl(400, 500, 600),
        Colour::cct(4200),
        Colour::monochrome(80),
    ];
    let expected = [
        &[
            (ColourChannel::Red, 10),
            (ColourChannel::Green, 20),
            (ColourChannel::Blue, 30),
        ][..],
        &[
            (ColourChannel::Hue, 100),
            (ColourChannel::Saturation, 200),
            (ColourChannel::Value, 300),
        ],
        &[
            (ColourChannel::Hue, 400),
            (ColourChannel::Saturation, 500),
            (ColourChannel::Lightness, 600),
        ],
        &[(ColourChannel::Temperature, 4200)],
        &[(ColourChannel::Intensity, 80)],
    ];

    // SAFETY: each colour pointer and both output pointers remain live for
    // every call. Null pointers exercise the documented failure paths.
    unsafe {
        for (colour, expected) in colours.iter().zip(expected) {
            let colour = cast_ref(colour);
            assert_eq!(luminate_colour_channel_count(colour), expected.len());

            for (index, (expected_channel, expected_value)) in expected.iter().enumerate() {
                let mut channel = u32::MAX;
                let mut value = u32::MAX;
                assert!(luminate_colour_channel_at(
                    colour,
                    index,
                    &raw mut channel,
                    &raw mut value
                ));
                assert_eq!(channel, *expected_channel as u32);
                assert_eq!(value, *expected_value);
            }

            let mut channel = 123;
            let mut value = 456;
            assert!(!luminate_colour_channel_at(
                colour,
                expected.len(),
                &raw mut channel,
                &raw mut value
            ));
            assert_eq!((channel, value), (123, 456));
            assert!(!luminate_colour_channel_at(
                colour,
                0,
                ptr::null_mut(),
                &raw mut value
            ));
            assert_eq!(value, 456);
        }

        let mut channel = 123;
        let mut value = 456;
        assert!(!luminate_colour_channel_at(
            ptr::null(),
            0,
            &raw mut channel,
            &raw mut value
        ));
        assert_eq!((channel, value), (123, 456));
    }
}

#[test]
fn state_accessors_expose_observations_and_adoption() {
    let target = TargetId::element("keyboard", "keys", "escape");
    let state = DeviceStateStatus {
        device: DeviceId::new("keyboard"),
        observations: vec![FacetObservation {
            target: target.clone(),
            value: FacetValue::Brightness(42),
            confidence: ObservationConfidence::Confirmed,
            source: ObservationSource::Readback,
            observed_at_ms: 1234,
            stale: true,
        }],
        reachability: Reachability::Reachable,
        reconciliation: ReconciliationStatus::Complete,
        adoption: vec![(
            target.clone(),
            StateFacetKind::Brightness,
            AdoptionStatus::Durable,
        )],
        latest_error: Some("briefly grumpy".to_owned()),
        latest_attempt_ms: Some(5678),
    };
    let state_ptr = cast_ref::<_, LuminateState>(&state);

    // SAFETY: all opaque pointers are views of the matching live native
    // values, and each borrowed child remains within `state`'s lifetime.
    unsafe {
        assert_eq!(bytes(luminate_state_device_id(state_ptr)), b"keyboard");
        assert_eq!(luminate_state_observation_count(state_ptr), 1);
        assert!(luminate_state_observation_at(state_ptr, 1).is_null());
        assert_eq!(luminate_state_adoption_count(state_ptr), 1);
        assert_eq!(
            luminate_state_reachability(state_ptr),
            Reachability::Reachable as u32
        );
        assert_eq!(
            luminate_state_reconciliation(state_ptr),
            ReconciliationStatus::Complete as u32
        );
        assert_eq!(
            bytes(luminate_state_latest_error(state_ptr)),
            b"briefly grumpy"
        );
        assert!(luminate_state_has_latest_attempt_ms(state_ptr));
        assert_eq!(luminate_state_latest_attempt_ms(state_ptr), 5678);

        let observation = luminate_state_observation_at(state_ptr, 0);
        let observation_target = luminate_observation_target(observation);
        assert_eq!(luminate_observation_confidence(observation), 2);
        assert_eq!(luminate_observation_source(observation), 1);
        assert_eq!(luminate_observation_observed_at_ms(observation), 1234);
        assert!(luminate_observation_stale(observation));
        assert_eq!(luminate_target_view_kind(observation_target), 2);
        assert_eq!(
            bytes(luminate_target_view_device_id(observation_target)),
            b"keyboard"
        );
        assert_eq!(
            bytes(luminate_target_view_surface_id(observation_target)),
            b"keys"
        );
        assert_eq!(
            bytes(luminate_target_view_element_id(observation_target)),
            b"escape"
        );
        assert!(bytes(luminate_target_view_group_id(observation_target)).is_empty());

        let value = luminate_observation_value(observation);
        assert_eq!(
            luminate_facet_value_kind(value),
            StateFacetKind::Brightness as u32
        );
        let mut brightness = 0;
        assert!(luminate_facet_value_brightness(value, &raw mut brightness));
        assert_eq!(brightness, 42);
        assert_eq!(luminate_facet_value_emission(value), u32::MAX);
        assert_eq!(
            luminate_state_find_observation(
                state_ptr,
                observation_target,
                StateFacetKind::Brightness as u32,
            ),
            observation
        );
        assert!(
            luminate_state_find_observation(
                state_ptr,
                observation_target,
                StateFacetKind::Appearance as u32,
            )
            .is_null()
        );

        let adoption = luminate_state_adoption_at(state_ptr, 0);
        assert_eq!(
            luminate_adoption_facet(adoption),
            StateFacetKind::Brightness as u32
        );
        assert_eq!(
            luminate_adoption_status(adoption),
            AdoptionStatus::Durable as u32
        );
        assert_eq!(
            luminate_target_view_kind(luminate_adoption_target(adoption)),
            2
        );
    }
}

#[test]
fn facet_and_target_variants_have_stable_views() {
    let colour = Colour::rgb(Rgb::new(10, 20, 30));
    let effect = Effect::Rainbow { period_ms: 900 };
    let values = [
        FacetValue::Appearance(AppearanceState::Static(colour.clone())),
        FacetValue::EffectiveAppearance(EffectiveAppearanceState::Off),
        FacetValue::EffectiveAppearance(EffectiveAppearanceState::Static(colour.clone())),
        FacetValue::Emission(EmissionState::Emitting),
        FacetValue::PhysicalPower(PhysicalPowerState::On),
        FacetValue::Appearance(AppearanceState::Effect(effect.clone())),
        FacetValue::EffectiveAppearance(EffectiveAppearanceState::Effect(effect)),
        FacetValue::EffectiveAppearance(EffectiveAppearanceState::Streaming),
        FacetValue::Appearance(AppearanceState::Mixed),
        FacetValue::EffectiveAppearance(EffectiveAppearanceState::Mixed),
    ];
    let targets = [
        TargetId::device("device"),
        TargetId::surface("device", "surface"),
        TargetId::group("device", "group"),
    ];

    // SAFETY: pointers are correctly typed views of live local values.
    unsafe {
        let appearance = cast_ref::<_, LuminateFacetValue>(&values[0]);
        assert_eq!(luminate_facet_value_appearance_kind(appearance), 0);
        let colour = luminate_facet_value_colour(appearance);
        assert!(!colour.is_null());
        assert_eq!(luminate_colour_encoding(colour), 0);
        assert_eq!(luminate_colour_channel_count(colour), 3);
        let mut channel = u32::MAX;
        let mut value = u32::MAX;
        assert!(luminate_colour_channel_at(
            colour,
            0,
            &raw mut channel,
            &raw mut value
        ));
        assert_eq!((channel, value), (0, 10));
        assert!(!luminate_colour_channel_at(
            colour,
            3,
            &raw mut channel,
            &raw mut value
        ));

        assert_eq!(
            luminate_facet_value_effective_appearance_kind(cast_ref(&values[1])),
            0
        );
        assert_eq!(
            luminate_facet_value_effective_appearance_kind(cast_ref(&values[2])),
            1
        );
        assert_eq!(luminate_facet_value_emission(cast_ref(&values[3])), 1);
        assert_eq!(luminate_facet_value_physical_power(cast_ref(&values[4])), 1);
        assert_eq!(
            luminate_facet_value_appearance_kind(cast_ref(&values[5])),
            1
        );
        assert_eq!(
            luminate_facet_value_effective_appearance_kind(cast_ref(&values[6])),
            2
        );
        assert_eq!(
            luminate_facet_value_effective_appearance_kind(cast_ref(&values[7])),
            3
        );
        assert!(luminate_facet_value_colour(cast_ref(&values[7])).is_null());
        assert!(luminate_facet_value_effect(cast_ref(&values[7])).is_null());
        assert_eq!(
            luminate_facet_value_appearance_kind(cast_ref(&values[8])),
            2
        );
        assert_eq!(
            luminate_facet_value_effective_appearance_kind(cast_ref(&values[9])),
            4
        );
        let borrowed_effect = luminate_facet_value_effect(cast_ref(&values[5]));
        assert_eq!(luminate_effect_view_kind(borrowed_effect), 7);
        assert!(luminate_facet_value_effect(cast_ref(&values[0])).is_null());

        assert_eq!(luminate_target_view_kind(cast_ref(&targets[0])), 0);
        assert_eq!(luminate_target_view_kind(cast_ref(&targets[1])), 1);
        assert_eq!(luminate_target_view_kind(cast_ref(&targets[2])), 3);
        assert_eq!(
            bytes(luminate_target_view_group_id(cast_ref(&targets[2]))),
            b"group"
        );
    }
}

#[test]
#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "The test checks a matrix of independent null-pointer accessor inputs."
)]
fn null_state_views_return_documented_sentinels() {
    // SAFETY: every typed accessor explicitly accepts null and returns a
    // documented empty or invalid sentinel.
    unsafe {
        assert_eq!(luminate_state_observation_count(ptr::null()), 0);
        assert!(luminate_state_observation_at(ptr::null(), 0).is_null());
        assert_eq!(luminate_state_reachability(ptr::null()), u32::MAX);
        assert!(!luminate_state_has_latest_attempt_ms(ptr::null()));
        assert_eq!(luminate_state_latest_attempt_ms(ptr::null()), 0);
        assert!(luminate_observation_target(ptr::null()).is_null());
        assert_eq!(luminate_facet_value_kind(ptr::null()), u32::MAX);
        assert!(luminate_facet_value_colour(ptr::null()).is_null());
        assert_eq!(luminate_colour_encoding(ptr::null()), u32::MAX);
        assert_eq!(luminate_colour_channel_count(ptr::null()), 0);

        assert_eq!(luminate_state_reconciliation(ptr::null()), u32::MAX);
        assert!(bytes(luminate_state_latest_error(ptr::null())).is_empty());
        assert_eq!(luminate_state_adoption_count(ptr::null()), 0);
        assert!(luminate_state_adoption_at(ptr::null(), 0).is_null());
        assert!(bytes(luminate_state_device_id(ptr::null())).is_empty());
        assert!(luminate_state_find_observation(ptr::null(), ptr::null(), 0).is_null());

        assert!(luminate_observation_value(ptr::null()).is_null());
        assert_eq!(luminate_observation_confidence(ptr::null()), u32::MAX);
        assert_eq!(luminate_observation_source(ptr::null()), u32::MAX);
        assert_eq!(luminate_observation_observed_at_ms(ptr::null()), 0);
        assert!(!luminate_observation_stale(ptr::null()));

        assert!(luminate_adoption_target(ptr::null()).is_null());
        assert_eq!(luminate_adoption_facet(ptr::null()), u32::MAX);
        assert_eq!(luminate_adoption_status(ptr::null()), u32::MAX);

        assert_eq!(luminate_target_view_kind(ptr::null()), u32::MAX);
        assert!(bytes(luminate_target_view_device_id(ptr::null())).is_empty());
        assert!(bytes(luminate_target_view_surface_id(ptr::null())).is_empty());
        assert!(bytes(luminate_target_view_element_id(ptr::null())).is_empty());
        assert!(bytes(luminate_target_view_group_id(ptr::null())).is_empty());

        assert_eq!(luminate_facet_value_appearance_kind(ptr::null()), u32::MAX);
        assert_eq!(
            luminate_facet_value_effective_appearance_kind(ptr::null()),
            u32::MAX
        );
        assert!(luminate_facet_value_effect(ptr::null()).is_null());
        let mut brightness = 17;
        let mut complete = true;
        assert!(!luminate_facet_value_brightness(
            ptr::null(),
            &raw mut brightness
        ));
        assert!(!luminate_facet_value_appearance_slots_complete(
            ptr::null(),
            &raw mut complete
        ));
        assert_eq!((brightness, complete), (17, true));
        assert_eq!(luminate_facet_value_emission(ptr::null()), u32::MAX);
        assert_eq!(luminate_facet_value_physical_power(ptr::null()), u32::MAX);

        assert!(!luminate_colour_channel_at(
            ptr::null(),
            0,
            ptr::null_mut(),
            ptr::null_mut()
        ));
    }
}

#[test]
fn facet_value_physical_power_and_emission_ignore_mismatched_variants() {
    let brightness = FacetValue::Brightness(10);
    let physical_power = FacetValue::PhysicalPower(PhysicalPowerState::Off);

    // SAFETY: pointers are correctly typed views of live local values.
    unsafe {
        // A brightness facet carries neither emission nor physical-power
        // data, so both accessors fall back to their invalid-discriminant
        // sentinel rather than misreading the union.
        assert_eq!(
            luminate_facet_value_physical_power(cast_ref(&brightness)),
            u32::MAX
        );
        assert_eq!(
            luminate_facet_value_emission(cast_ref(&brightness)),
            u32::MAX
        );
        assert_eq!(
            luminate_facet_value_physical_power(cast_ref(&physical_power)),
            0
        );
    }
}

#[test]
fn effect_accessors_cover_portable_effects() {
    let red = Rgb::new(200, 10, 20);
    let effects = [
        Effect::Off,
        Effect::Static {
            colour: Colour::rgb(red),
        },
        Effect::Breathe {
            colour: red,
            period_ms: 100,
        },
        Effect::Pulse {
            colour: red,
            period_ms: 200,
        },
        Effect::Scanner {
            colour: red,
            period_ms: 300,
        },
        Effect::Morph {
            colours: vec![red, Rgb::new(1, 2, 3)],
            period_ms: 400,
        },
        Effect::Spectrum { period_ms: 500 },
        Effect::Rainbow { period_ms: 600 },
        Effect::Strobe {
            colour: red,
            period_ms: 700,
        },
        Effect::Hardware {
            id: HardwareEffectId::new("scene"),
            arguments: EffectArguments {
                colours: vec![red],
                speed: Some(8),
                direction: Some(EffectDirection::Reverse),
                duration_ms: Some(900),
                brightness: Some(77),
                choice: Some("soft".to_owned()),
            },
        },
    ];

    // SAFETY: each pointer is a correctly typed view of a live effect.
    unsafe {
        for (kind, effect) in [0, 1, 2, 3, 4, 5, 6, 7, 9, 8].into_iter().zip(&effects) {
            assert_eq!(luminate_effect_kind(cast_ref(effect)), kind);
        }
        for (index, period) in [100, 200, 300, 400, 500, 600, 700].into_iter().enumerate() {
            let mut actual = u32::MAX;
            assert!(luminate_effect_period_ms(
                cast_ref(&effects[index + 2]),
                &raw mut actual
            ));
            assert_eq!(actual, period);
        }

        let static_colour = luminate_effect_static_colour(cast_ref(&effects[1]));
        assert!(!static_colour.is_null());
        assert_eq!(luminate_colour_encoding(static_colour), 0);
        assert_eq!(luminate_effect_rgb_count(cast_ref(&effects[5])), 2);
        let mut second = LuminateRgb { r: 0, g: 0, b: 0 };
        assert!(luminate_effect_rgb_at(
            cast_ref(&effects[5]),
            1,
            &raw mut second
        ));
        assert_eq!((second.r, second.g, second.b), (1, 2, 3));

        assert_eq!(luminate_effect_kind(ptr::null()), u32::MAX);
        let mut absent = 123;
        assert!(!luminate_effect_period_ms(
            cast_ref(&effects[0]),
            &raw mut absent
        ));
        assert_eq!(absent, 123);
        assert!(!luminate_effect_speed(
            cast_ref(&effects[0]),
            ptr::null_mut()
        ));
        assert_eq!(luminate_effect_direction(cast_ref(&effects[0])), u32::MAX);
        assert!(bytes(luminate_effect_hardware_id(cast_ref(&effects[0]))).is_empty());

        // A portable (non-hardware) effect carries none of the hardware
        // argument fields, and a colour-less effect has no colour or morph
        // list, so every optional accessor falls back to its documented
        // default rather than misreading an unrelated variant's payload.
        let spectrum = cast_ref(&effects[6]);
        let off = cast_ref(&effects[0]);
        let mut colour = LuminateRgb { r: 1, g: 2, b: 3 };
        assert!(!luminate_effect_rgb(spectrum, &raw mut colour));
        assert_eq!((colour.r, colour.g, colour.b), (1, 2, 3));
        assert_eq!(luminate_effect_rgb_count(spectrum), 0);
        assert!(!luminate_effect_rgb_at(spectrum, 0, &raw mut colour));
        assert_eq!((colour.r, colour.g, colour.b), (1, 2, 3));
        assert_eq!(luminate_effect_direction(off), u32::MAX);
        assert!(!luminate_effect_duration_ms(off, &raw mut absent));
        assert_eq!(absent, 123);
        assert!(!luminate_effect_brightness(off, &raw mut absent));
        assert_eq!(absent, 123);
        assert!(bytes(luminate_effect_choice(off)).is_empty());
    }
}

#[test]
fn effect_accessors_cover_hardware_arguments() {
    let effect = Effect::Hardware {
        id: HardwareEffectId::new("scene"),
        arguments: EffectArguments {
            colours: vec![Rgb::new(200, 10, 20)],
            speed: Some(8),
            direction: Some(EffectDirection::Reverse),
            duration_ms: Some(900),
            brightness: Some(77),
            choice: Some("soft".to_owned()),
        },
    };

    // SAFETY: the pointer is a correctly typed view of a live effect.
    unsafe {
        let hardware = cast_ref(&effect);
        assert_eq!(bytes(luminate_effect_hardware_id(hardware)), b"scene");
        assert_eq!(luminate_effect_rgb_count(hardware), 1);

        let mut speed = 0;
        let mut duration = 0;
        let mut brightness = 0;
        assert!(luminate_effect_speed(hardware, &raw mut speed));
        assert_eq!(speed, 8);
        assert!(luminate_effect_duration_ms(hardware, &raw mut duration));
        assert_eq!(duration, 900);
        assert!(luminate_effect_brightness(hardware, &raw mut brightness));
        assert_eq!(brightness, 77);
        assert_eq!(luminate_effect_direction(hardware), 1);
        assert_eq!(bytes(luminate_effect_choice(hardware)), b"soft");
    }
}
