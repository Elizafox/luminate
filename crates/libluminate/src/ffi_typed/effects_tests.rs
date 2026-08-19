// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Effect projections and portable fake-daemon round trips.

#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "Safety is documented before each C-boundary assertion; Clippy cannot associate comments outside assertion macro expansions."
)]

use super::*;
use crate::ffi::{luminate_client_connect_path, luminate_client_free};
use crate::ffi_typed::colour_input::LuminateColourChannelInput;
use crate::ffi_typed::management::{
    luminate_management_change_set_view_count, luminate_management_change_set_view_revision,
};
use crate::ffi_typed::test_support::{FakeDaemon, acknowledge};
use luminate_core::device::DeviceId;
use luminate_protocol::{ManagementChangeSet, Request};
use std::ffi::CString;

fn bytes(view: LuminateStringView) -> Option<&'static [u8]> {
    if view.data.is_null() {
        None
    } else {
        // SAFETY: test inputs remain alive while their borrowed views are inspected.
        Some(unsafe { std::slice::from_raw_parts(view.data.cast(), view.len) })
    }
}

fn owned_effect() -> *mut LuminateEffect {
    let mut effect = ptr::null_mut();
    // SAFETY: `effect` is a writable output pointer.
    assert_eq!(
        unsafe { luminate_effect_create_off(&raw mut effect) },
        LuminateStatus::Ok
    );
    effect
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "The test covers the complete effect and colour variant matrices under one ownership contract."
)]
fn borrowed_effects_and_colours_clone_to_independent_owned_effects() {
    let rgb = Rgb::new(10, 20, 30);
    let portable = vec![
        Effect::Off,
        Effect::Static {
            colour: Colour::hsv(1, 2, 3),
        },
        Effect::Breathe {
            colour: rgb,
            period_ms: 100,
        },
        Effect::Pulse {
            colour: rgb,
            period_ms: 200,
        },
        Effect::Strobe {
            colour: rgb,
            period_ms: 300,
        },
        Effect::Scanner {
            colour: rgb,
            period_ms: 400,
        },
        Effect::Morph {
            colours: vec![rgb, Rgb::new(40, 50, 60)],
            period_ms: 500,
        },
        Effect::Spectrum { period_ms: 600 },
        Effect::Rainbow { period_ms: 700 },
    ];
    for source in portable {
        let expected = source.clone();
        let mut cloned = ptr::null_mut();
        // SAFETY: the borrowed effect and output pointer are live.
        assert_eq!(
            unsafe { luminate_effect_view_clone(cast_ref(&source), &raw mut cloned) },
            LuminateStatus::Ok
        );
        drop(source);
        assert_eq!(effect_ref(cloned), Some(&expected));
        // SAFETY: `cloned` is the uniquely owned result above.
        unsafe { luminate_effect_free(cloned) };
    }

    let effect = Effect::Hardware {
        id: HardwareEffectId::new("sparkle"),
        arguments: EffectArguments {
            colours: vec![Rgb::new(1, 2, 3)],
            speed: Some(4),
            duration_ms: Some(5),
            brightness: Some(6),
            direction: Some(EffectDirection::Clockwise),
            choice: Some("wide".to_owned()),
        },
    };
    let expected = effect.clone();
    let mut clone = ptr::null_mut();
    // SAFETY: the borrowed effect and output pointer are live.
    assert_eq!(
        unsafe { luminate_effect_view_clone(cast_ref(&effect), &raw mut clone) },
        LuminateStatus::Ok
    );
    drop(effect);
    assert_eq!(effect_ref(clone), Some(&expected));
    // SAFETY: `clone` is the uniquely owned result above.
    unsafe { luminate_effect_free(clone) };

    let rgbw = Colour::additive(vec![
        luminate_core::colour::ColourChannelValue::new(ColourChannel::Red, 17),
        luminate_core::colour::ColourChannelValue::new(ColourChannel::Green, 18),
        luminate_core::colour::ColourChannelValue::new(ColourChannel::Blue, 19),
        luminate_core::colour::ColourChannelValue::new(ColourChannel::White, 20),
    ])
    .expect("valid RGBW colour");
    for colour in [
        Colour::rgb(Rgb::new(7, 8, 9)),
        rgbw,
        Colour::hsv(10, 11, 12),
        Colour::hsl(13, 14, 15),
        Colour::cct(4_000),
        Colour::monochrome(16),
    ] {
        let mut owned = ptr::null_mut();
        // SAFETY: the borrowed colour and output pointer are live.
        assert_eq!(
            unsafe { luminate_effect_create_static_from_colour(cast_ref(&colour), &raw mut owned) },
            LuminateStatus::Ok
        );
        assert!(
            matches!(effect_ref(owned), Some(Effect::Static { colour: cloned }) if cloned == &colour)
        );
        // SAFETY: `owned` is the uniquely owned result above.
        unsafe { luminate_effect_free(owned) };
    }

    let sentinel = ptr::dangling_mut::<LuminateEffect>();
    let mut unchanged = sentinel;
    // SAFETY: null inputs exercise validation and must not write the output.
    unsafe {
        assert_eq!(
            luminate_effect_view_clone(ptr::null(), &raw mut unchanged),
            LuminateStatus::NullPointer
        );
        assert_eq!(unchanged, sentinel);
        assert_eq!(
            luminate_effect_create_static_from_colour(ptr::null(), &raw mut unchanged),
            LuminateStatus::NullPointer
        );
        assert_eq!(unchanged, sentinel);
        let effect = Effect::Off;
        assert_eq!(
            luminate_effect_view_clone(cast_ref(&effect), ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        let colour = Colour::monochrome(1);
        assert_eq!(
            luminate_effect_create_static_from_colour(cast_ref(&colour), ptr::null_mut()),
            LuminateStatus::NullPointer
        );
    }
}

#[test]
#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "The test exercises the common ownership contract of independent C constructors."
)]
fn portable_effect_constructors_preserve_arguments() {
    let colour = LuminateRgb { r: 1, g: 2, b: 3 };
    let static_channels = [
        LuminateColourChannelInput {
            channel: 0,
            value: 1,
        },
        LuminateColourChannelInput {
            channel: 1,
            value: 2,
        },
        LuminateColourChannelInput {
            channel: 2,
            value: 3,
        },
    ];
    let static_colour = LuminateColourInput {
        encoding: 0,
        channels: static_channels.as_ptr(),
        channel_count: static_channels.len(),
    };
    let mut effect = ptr::null_mut();
    // SAFETY: `effect` is writable and each returned effect is freed before reuse.
    unsafe {
        assert_eq!(
            luminate_effect_create_static(&raw const static_colour, &raw mut effect),
            LuminateStatus::Ok
        );
        assert!(
            matches!(effect_ref(effect), Some(Effect::Static { colour }) if *colour == Colour::rgb(Rgb::new(1, 2, 3)))
        );
        luminate_effect_free(effect);

        assert_eq!(
            luminate_effect_create_breathe(colour, 10, &raw mut effect),
            LuminateStatus::Ok
        );
        assert!(matches!(
            effect_ref(effect),
            Some(Effect::Breathe { period_ms: 10, .. })
        ));
        luminate_effect_free(effect);

        assert_eq!(
            luminate_effect_create_pulse(colour, 20, &raw mut effect),
            LuminateStatus::Ok
        );
        assert!(matches!(
            effect_ref(effect),
            Some(Effect::Pulse { period_ms: 20, .. })
        ));
        luminate_effect_free(effect);

        assert_eq!(
            luminate_effect_create_strobe(colour, 30, &raw mut effect),
            LuminateStatus::Ok
        );
        assert!(matches!(
            effect_ref(effect),
            Some(Effect::Strobe { period_ms: 30, .. })
        ));
        luminate_effect_free(effect);

        assert_eq!(
            luminate_effect_create_scanner(colour, 40, &raw mut effect),
            LuminateStatus::Ok
        );
        assert!(matches!(
            effect_ref(effect),
            Some(Effect::Scanner { period_ms: 40, .. })
        ));
        luminate_effect_free(effect);

        assert_eq!(
            luminate_effect_create_spectrum(50, &raw mut effect),
            LuminateStatus::Ok
        );
        assert!(matches!(
            effect_ref(effect),
            Some(Effect::Spectrum { period_ms: 50 })
        ));
        luminate_effect_free(effect);

        assert_eq!(
            luminate_effect_create_rainbow(60, &raw mut effect),
            LuminateStatus::Ok
        );
        assert!(matches!(
            effect_ref(effect),
            Some(Effect::Rainbow { period_ms: 60 })
        ));
        luminate_effect_free(effect);
    }
}

#[test]
fn morph_constructor_validates_and_decodes_colours() {
    let mut effect = ptr::null_mut();
    // SAFETY: null with a nonzero count is rejected before dereferencing.
    assert_eq!(
        unsafe { luminate_effect_create_morph(ptr::null(), 1, 70, &raw mut effect) },
        LuminateStatus::NullPointer
    );
    let colours = [
        LuminateRgb { r: 1, g: 2, b: 3 },
        LuminateRgb { r: 4, g: 5, b: 6 },
    ];
    // SAFETY: `colours` contains two live values and `effect` is writable.
    assert_eq!(
        unsafe {
            luminate_effect_create_morph(colours.as_ptr(), colours.len(), 70, &raw mut effect)
        },
        LuminateStatus::Ok
    );
    assert!(matches!(
        effect_ref(effect),
        Some(Effect::Morph { colours, period_ms: 70 })
            if colours == &[Rgb::new(1, 2, 3), Rgb::new(4, 5, 6)]
    ));
    // SAFETY: `effect` is the uniquely owned pointer returned above.
    unsafe { luminate_effect_free(effect) };

    // SAFETY: a zero count permits null and `effect` is writable.
    assert_eq!(
        unsafe { luminate_effect_create_morph(ptr::null(), 0, 0, &raw mut effect) },
        LuminateStatus::Ok
    );
    assert!(
        matches!(effect_ref(effect), Some(Effect::Morph { colours, .. }) if colours.is_empty())
    );
    // SAFETY: `effect` is the uniquely owned pointer returned above.
    unsafe { luminate_effect_free(effect) };
}

#[test]
#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "The test exercises all setters against one live owned C effect."
)]
fn hardware_effect_setters_cover_arguments_and_errors() {
    let id = CString::new("wave").expect("valid id");
    let choice = CString::new("sunset").expect("valid choice");
    let mut effect = ptr::null_mut();
    // SAFETY: inputs are live NUL-terminated strings and `effect` is writable.
    unsafe {
        assert_eq!(
            luminate_effect_create_hardware(id.as_ptr(), &raw mut effect),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_effect_hardware_add_colour(effect, LuminateRgb { r: 7, g: 8, b: 9 }),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_effect_hardware_set_speed(effect, 2),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_effect_hardware_set_duration_ms(effect, 300),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_effect_hardware_set_brightness(effect, 200),
            LuminateStatus::Ok
        );
        for direction in 0..=6 {
            assert_eq!(
                luminate_effect_hardware_set_direction(effect, direction),
                LuminateStatus::Ok
            );
        }
        assert_eq!(
            luminate_effect_hardware_set_direction(effect, 7),
            LuminateStatus::InvalidArgument
        );
        assert_eq!(
            luminate_effect_hardware_set_choice(effect, choice.as_ptr()),
            LuminateStatus::Ok
        );
    }
    let Some(Effect::Hardware { id, arguments }) = effect_ref(effect) else {
        panic!("expected hardware effect");
    };
    assert_eq!(id.as_str(), "wave");
    assert_eq!(arguments.colours, [Rgb::new(7, 8, 9)]);
    assert_eq!(arguments.speed, Some(2));
    assert_eq!(arguments.duration_ms, Some(300));
    assert_eq!(arguments.brightness, Some(200));
    assert_eq!(arguments.direction, Some(EffectDirection::Random));
    assert_eq!(arguments.choice.as_deref(), Some("sunset"));
    // SAFETY: `effect` is the uniquely owned pointer returned above.
    unsafe { luminate_effect_free(effect) };

    let portable = owned_effect();
    // SAFETY: `portable` is live but deliberately has the wrong effect variant.
    assert_eq!(
        unsafe { luminate_effect_hardware_set_speed(portable, 1) },
        LuminateStatus::InvalidArgument
    );
    // SAFETY: null is rejected before dereferencing.
    assert_eq!(
        unsafe { luminate_effect_hardware_set_speed(ptr::null_mut(), 1) },
        LuminateStatus::NullPointer
    );
    // SAFETY: `portable` is the uniquely owned pointer returned by `owned_effect`.
    unsafe { luminate_effect_free(portable) };
}

#[test]
fn target_decoding_covers_shapes_and_invalid_combinations() {
    let device = CString::new("device").expect("valid id");
    let surface = CString::new("surface").expect("valid id");
    let element = CString::new("element").expect("valid id");
    let group = CString::new("group").expect("valid id");
    let mut target = LuminateTarget {
        device_id: device.as_ptr(),
        surface_id: ptr::null(),
        element_id: ptr::null(),
        group_id: ptr::null(),
    };
    // SAFETY: every non-null component points to a live NUL-terminated string.
    assert_eq!(
        unsafe { read_target(&raw const target) },
        Ok(TargetId::device("device"))
    );
    target.surface_id = surface.as_ptr();
    // SAFETY: same live component contract.
    assert_eq!(
        unsafe { read_target(&raw const target) },
        Ok(TargetId::surface("device", "surface"))
    );
    target.element_id = element.as_ptr();
    // SAFETY: same live component contract.
    assert_eq!(
        unsafe { read_target(&raw const target) },
        Ok(TargetId::element("device", "surface", "element"))
    );
    target.surface_id = ptr::null();
    target.element_id = ptr::null();
    target.group_id = group.as_ptr();
    // SAFETY: same live component contract.
    assert_eq!(
        unsafe { read_target(&raw const target) },
        Ok(TargetId::group("device", "group"))
    );
    target.surface_id = surface.as_ptr();
    // SAFETY: the live but contradictory components are rejected by domain validation.
    assert_eq!(
        unsafe { read_target(&raw const target) },
        Err(LuminateStatus::InvalidArgument)
    );
    // SAFETY: null is rejected before dereferencing.
    assert_eq!(
        unsafe { read_target(ptr::null()) },
        Err(LuminateStatus::NullPointer)
    );
}

/// Exercises the successful dispatch branch of every target-addressed client
/// operation against a live mock daemon. A stopped or null client can only
/// ever reach the `Internal`/`NullPointer` early-return branches, so this is
/// the only way to cover the `Ok` match arms.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "The test exercises several independent client operations across one live connection."
)]
async fn target_operations_round_trip_through_a_live_daemon() {
    let daemon = FakeDaemon::bind("effects-ffi-target-ops");
    let path_c = daemon.c_path();
    let server = tokio::spawn(async move {
        let mut daemon = daemon;
        let mut stream = daemon.accept("effects-test-daemon").await;
        assert!(matches!(
            acknowledge(&mut stream).await,
            Request::SetBrightness(_)
        ));
        assert!(matches!(
            acknowledge(&mut stream).await,
            Request::ClearTarget { .. }
        ));
        assert!(matches!(
            acknowledge(&mut stream).await,
            Request::SetEffect(_)
        ));
        assert!(matches!(
            acknowledge(&mut stream).await,
            Request::RestoreAppearance { .. }
        ));
        assert!(matches!(
            acknowledge(&mut stream).await,
            Request::SetEffect(_)
        ));
        assert!(matches!(
            acknowledge(&mut stream).await,
            Request::RestoreAppearance { .. }
        ));
        assert!(matches!(
            acknowledge(&mut stream).await,
            Request::SetEffect(_)
        ));
    });

    let device = CString::new("keyboard").expect("valid device id");
    let target = LuminateTarget {
        device_id: device.as_ptr(),
        surface_id: ptr::null(),
        element_id: ptr::null(),
        group_id: ptr::null(),
    };
    let mut client = ptr::null_mut();

    // SAFETY: every pointer refers to live local storage or a NUL-terminated
    // string kept alive for the duration of this test, and every owned
    // resource is freed exactly once.
    unsafe {
        assert_eq!(
            luminate_client_connect_path(path_c.as_ptr(), &raw mut client),
            LuminateStatus::Ok
        );

        assert_eq!(
            luminate_client_set_brightness(client, &raw const target, 42),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_clear_target(client, &raw const target),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_set_off(client, &raw const target),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_restore_appearance(client, &raw const target),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_set_emission(client, &raw const target, EmissionState::Dark as u32),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_set_emission(client, &raw const target, EmissionState::Emitting as u32),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_set_emission(client, &raw const target, u32::MAX),
            LuminateStatus::InvalidArgument
        );
        let effect = owned_effect();
        assert_eq!(
            luminate_client_set_effect(client, &raw const target, effect),
            LuminateStatus::Ok
        );
        luminate_effect_free(effect);
        assert_eq!(
            luminate_client_set_effect(client, &raw const target, ptr::null()),
            LuminateStatus::NullPointer
        );

        luminate_client_free(client);
    }

    server.await.expect("effects server task");
}

#[test]
fn event_accessors_cover_variants_and_sentinels() {
    let topology = LuminateEvent(Event::TopologyChanged {
        devices: vec![DeviceId::new("keyboard")],
    });
    let state = LuminateEvent(Event::StateChanged {
        devices: vec![DeviceId::new("mouse")],
    });
    let configuration = LuminateEvent(Event::ConfigurationChanged {
        changes: ManagementChangeSet {
            revision: 1,
            changes: Vec::new(),
        },
    });
    let shm_ended = LuminateEvent(Event::ShmStreamEnded {
        target: TargetId::surface("keyboard", "keys"),
        generation: 0,
    });
    let topology = ptr::from_ref(&topology);
    let state = ptr::from_ref(&state);
    let configuration = ptr::from_ref(&configuration);
    let shm_ended = ptr::from_ref(&shm_ended);
    let mut generation = 7;
    // SAFETY: both pointers refer to live events for every accessor call.
    unsafe {
        assert_eq!(luminate_event_kind(topology), 0);
        assert_eq!(luminate_event_topology_device_count(topology), 1);
        assert_eq!(
            bytes(luminate_event_topology_device_at(topology, 0)),
            Some(b"keyboard".as_slice())
        );
        assert_eq!(bytes(luminate_event_topology_device_at(topology, 1)), None);
        assert_eq!(luminate_event_state_device_count(topology), 0);

        assert_eq!(luminate_event_kind(state), 1);
        assert_eq!(luminate_event_state_device_count(state), 1);
        assert_eq!(
            bytes(luminate_event_state_device_at(state, 0)),
            Some(b"mouse".as_slice())
        );
        assert_eq!(bytes(luminate_event_state_device_at(topology, 0)), None);

        assert_eq!(luminate_event_kind(configuration), 3);
        assert_eq!(luminate_event_topology_device_count(configuration), 0);
        assert_eq!(luminate_event_state_device_count(configuration), 0);
        let changes = luminate_event_configuration_changes(configuration);
        assert!(!changes.is_null());
        assert_eq!(luminate_management_change_set_view_revision(changes), 1);
        assert_eq!(luminate_management_change_set_view_count(changes), 0);
        assert!(luminate_event_configuration_changes(topology).is_null());

        assert_eq!(luminate_event_kind(shm_ended), 2);
        assert!(luminate_event_shm_stream_generation(
            shm_ended,
            &raw mut generation
        ));
        assert_eq!(generation, 0);
        generation = 7;
        assert!(!luminate_event_shm_stream_generation(
            topology,
            &raw mut generation
        ));
        assert_eq!(generation, 7);
        assert!(!luminate_event_shm_stream_generation(
            shm_ended,
            ptr::null_mut()
        ));

        assert_eq!(luminate_event_kind(ptr::null()), u32::MAX);
        assert_eq!(luminate_event_topology_device_count(ptr::null()), 0);
        assert_eq!(bytes(luminate_event_state_device_at(ptr::null(), 0)), None);
        assert!(luminate_event_configuration_changes(ptr::null()).is_null());
        assert!(!luminate_event_shm_stream_generation(
            ptr::null(),
            &raw mut generation
        ));
        assert_eq!(generation, 7);
    }
}
