// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;
use luminate_core::shm_frame::ShmPixelFormat;
use luminate_core::util::DiscreteRange;

fn bytes(view: LuminateStringView) -> Vec<u8> {
    if view.data.is_null() {
        return Vec::new();
    }

    // SAFETY: typed accessors return a view into a value kept alive by
    // the test for the duration of this copy.
    unsafe { std::slice::from_raw_parts(view.data.cast(), view.len) }.to_vec()
}

#[test]
fn fixed_colour_capability_bits_are_available_by_channel() {
    let capability = ColourCapability::Hsl {
        hue_bits: 16,
        saturation_bits: 8,
        lightness_bits: 10,
    };
    let capability = cast_ref(&capability);
    let mut bits = 0;

    // SAFETY: the capability and output pointers remain valid for each call.
    unsafe {
        assert!(luminate_colour_capability_bits(
            capability,
            ColourChannel::Lightness as u32,
            &raw mut bits
        ));
        assert_eq!(bits, 10);
        assert!(!luminate_colour_capability_bits(
            capability,
            ColourChannel::Blue as u32,
            &raw mut bits
        ));
    }
}

fn assert_shm_capability(frame: *const LuminateFrameUploadCapability) {
    let mut pixel_count = 0;
    let mut width = 0;
    let mut height = 0;

    // SAFETY: `frame` is a borrowed view into the live capability set in the
    // calling test.
    unsafe {
        let shm = luminate_frame_upload_shm(frame);
        assert!(!shm.is_null());
        assert_eq!(luminate_shm_frame_pixel_format_count(shm), 2);
        assert_eq!(luminate_shm_frame_pixel_format_at(shm, 0), 0);
        assert_eq!(luminate_shm_frame_pixel_format_at(shm, 1), 3);
        assert_eq!(luminate_shm_frame_pixel_format_at(shm, 2), u32::MAX);
        assert_eq!(luminate_shm_pixel_format_bytes_per_pixel(0), 3);
        assert_eq!(luminate_shm_pixel_format_bytes_per_pixel(3), 4);
        assert_eq!(luminate_shm_pixel_format_bytes_per_pixel(u32::MAX), 0);
        assert_eq!(luminate_shm_frame_shape_kind(shm), 1);
        assert!(luminate_shm_frame_pixel_count(shm, &raw mut pixel_count));
        assert!(luminate_shm_frame_matrix_width(shm, &raw mut width));
        assert!(luminate_shm_frame_matrix_height(shm, &raw mut height));
        assert!(luminate_shm_frame_has_max_rate_hz(shm));
        assert_eq!(luminate_shm_frame_max_rate_hz(shm), 120);
    }
    assert_eq!((pixel_count, width, height), (6, 3, 2));
}

#[test]
fn linear_shm_shape_exposes_only_its_pixel_count() {
    let capability = ShmFrameCapability {
        pixel_formats: vec![ShmPixelFormat::Rgb8],
        shape: ShmFrameShape::Linear { pixel_count: 5 },
        max_rate_hz: None,
    };
    let capability = cast_ref::<_, LuminateShmFrameCapability>(&capability);
    let mut pixel_count = 0;
    let mut width = 7;
    let mut height = 8;

    // SAFETY: the capability and output pointers remain valid for each call.
    unsafe {
        assert!(luminate_shm_frame_pixel_count(
            capability,
            &raw mut pixel_count
        ));
        assert!(!luminate_shm_frame_matrix_width(capability, &raw mut width));
        assert!(!luminate_shm_frame_matrix_height(
            capability,
            &raw mut height
        ));
        assert!(!luminate_shm_frame_pixel_count(
            ptr::null(),
            &raw mut pixel_count
        ));
        assert!(!luminate_shm_frame_pixel_count(capability, ptr::null_mut()));
    }
    assert_eq!((pixel_count, width, height), (5, 7, 8));
}

fn assert_independent_brightness(root: *const LuminateCapabilitySet) {
    let mut bits = 0;
    let mut maximum = 0;
    let mut scope = u32::MAX;

    // SAFETY: `root` is a correctly typed view kept alive by the caller.
    unsafe {
        assert_eq!(luminate_capability_set_brightness_kind(root), 1);
        assert!(luminate_capability_set_brightness_bits(root, &raw mut bits));
        assert!(luminate_capability_set_brightness_maximum(
            root,
            &raw mut maximum
        ));
        assert!(luminate_capability_set_brightness_scope(
            root,
            &raw mut scope
        ));
    }
    assert_eq!((bits, maximum, scope), (12, 4095, 1));
}

fn assert_profile_persistence(root: *const LuminateCapabilitySet) {
    let mut requirement = u32::MAX;
    let mut slots = 0;
    let mut explicit_commit = false;
    let mut readback = false;

    // SAFETY: `root` is a correctly typed view kept alive by the caller.
    unsafe {
        assert_eq!(luminate_capability_set_persistence_kind(root), 2);
        assert!(luminate_capability_set_persistence_requirement(
            root,
            &raw mut requirement
        ));
        assert!(luminate_capability_set_persistence_slots(
            root,
            &raw mut slots
        ));
        assert!(luminate_capability_set_persistence_explicit_commit(
            root,
            &raw mut explicit_commit
        ));
        assert!(luminate_capability_set_persistence_readback(
            root,
            &raw mut readback
        ));
    }
    assert_eq!(
        (requirement, slots, explicit_commit, readback),
        (1, 4, true, true)
    );
}

#[test]
fn capability_set_accessors_expose_nested_capabilities() {
    let capabilities = CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Disabled,
        brightness: BrightnessCapability::Independent {
            bits: 12,
            maximum: 4095,
            scope: CapabilityScope::Surface,
        },
        frame_upload: Some(FrameUploadCapability {
            scope: CapabilityScope::Element,
            update_mode: FrameUpdateMode::Both,
            max_rate_hz: Some(60),
            atomic: true,
            buffering: BufferingMode::DoubleBuffered,
            shm: Some(ShmFrameCapability {
                pixel_formats: vec![ShmPixelFormat::Rgb8, ShmPixelFormat::Rgbx8],
                shape: ShmFrameShape::Matrix {
                    width: 3,
                    height: 2,
                },
                max_rate_hz: Some(120),
            }),
        }),
        persistence: PersistenceCapability::Profiles {
            requirement: PersistenceRequirement::Required,
            slots: 4,
            explicit_commit: true,
            readback: true,
        },
        state_readback: StateReadbackCapability::Readable {
            facets: vec![ReadableFacet {
                facet: StateFacetKind::Appearance,
                fidelity: ReadbackFidelity::Exact,
            }],
            read_disturbs_output: true,
            notifies_external_changes: true,
        },
        emission: true,
        physical_power: Some(PhysicalPowerCapability {
            scope: CapabilityScope::Device,
        }),
        power_domain: Some(PowerDomainRef::Surface {
            surface: "panel".to_owned(),
        }),
        ..CapabilitySet::default()
    };
    let root = cast_ref::<_, LuminateCapabilitySet>(&capabilities);

    // SAFETY: every opaque pointer is a view of its matching live native
    // value, and borrowed children remain within `capabilities`' lifetime.
    unsafe {
        assert_eq!(luminate_capability_set_colour_count(root), 1);
        assert!(luminate_capability_set_colour_at(root, 1).is_null());
        assert!(luminate_capability_set_emission(root));
        assert!(!luminate_capability_set_off_is_wear_safe(root));
        assert_eq!(
            luminate_capability_set_cct_emulation(root),
            CctEmulation::Disabled as u32
        );
        assert_independent_brightness(root);

        let colour = luminate_capability_set_colour_at(root, 0);
        assert_eq!(luminate_colour_capability_encoding(colour), 0);
        assert_eq!(luminate_colour_capability_channel_count(colour), 3);
        let channel = luminate_colour_capability_channel_at(colour, 2);
        assert_eq!(luminate_colour_channel_capability_channel(channel), 2);
        assert_eq!(luminate_colour_channel_capability_bits(channel), 8);

        let frame = luminate_capability_set_frame_upload(root);
        assert_eq!(luminate_frame_upload_scope(frame), 0);
        assert_eq!(luminate_frame_upload_update_mode(frame), 2);
        assert!(luminate_frame_upload_has_max_rate_hz(frame));
        assert_eq!(luminate_frame_upload_max_rate_hz(frame), 60);
        assert!(luminate_frame_upload_atomic(frame));
        assert_eq!(luminate_frame_upload_buffering(frame), 2);
        assert_shm_capability(frame);

        assert_profile_persistence(root);

        assert_eq!(luminate_capability_set_state_readback_kind(root), 1);
        assert_eq!(luminate_capability_set_readable_facet_count(root), 1);
        assert!(luminate_capability_set_readable_facet_at(root, 1).is_null());
        let facet = luminate_capability_set_readable_facet_at(root, 0);
        assert_eq!(luminate_readable_facet_kind(facet), 0);
        assert_eq!(luminate_readable_facet_fidelity(facet), 1);
        assert!(luminate_capability_set_read_disturbs_output(root));
        assert!(luminate_capability_set_notifies_external_changes(root));

        let power = luminate_capability_set_physical_power(root);
        assert_eq!(luminate_physical_power_scope(power), 2);
        let domain = luminate_capability_set_power_domain(root);
        assert_eq!(luminate_power_domain_kind(domain), 1);
        assert_eq!(bytes(luminate_power_domain_surface_id(domain)), b"panel");
    }
}

#[test]
fn hardware_effect_accessors_cover_parameter_variants() {
    let hardware = HardwareEffectsCapability {
        effects: vec![HardwareEffectDescriptor {
            id: HardwareEffectId::new("rainbow"),
            name: "Rainbow".to_owned(),
            parameters: vec![
                EffectParameter::Colour {
                    minimum_colours: 1,
                    maximum_colours: 3,
                },
                EffectParameter::Direction {
                    values: vec![EffectDirection::Forward, EffectDirection::Reverse],
                },
                EffectParameter::Brightness { bits: 8 },
                EffectParameter::Choice {
                    options: vec![EffectChoice {
                        id: "soft".to_owned(),
                        name: "Soft".to_owned(),
                    }],
                },
            ],
        }],
        scope: CapabilityScope::Device,
        concurrent_with_streaming: true,
    };
    let hardware = cast_ref::<_, LuminateHardwareEffectsCapability>(&hardware);

    // SAFETY: every opaque pointer is a correctly typed view into the
    // live `hardware` value.
    unsafe {
        assert_eq!(luminate_hardware_effects_effect_count(hardware), 1);
        assert_eq!(luminate_hardware_effects_scope(hardware), 2);
        assert!(luminate_hardware_effects_concurrent_with_streaming(
            hardware
        ));
        let effect = luminate_hardware_effects_effect_at(hardware, 0);
        assert_eq!(
            bytes(luminate_hardware_effect_descriptor_id(effect)),
            b"rainbow"
        );
        assert_eq!(
            bytes(luminate_hardware_effect_descriptor_name(effect)),
            b"Rainbow"
        );
        assert_eq!(
            luminate_hardware_effect_descriptor_parameter_count(effect),
            4
        );

        let colour = luminate_hardware_effect_descriptor_parameter_at(effect, 0);
        assert_eq!(luminate_effect_parameter_kind(colour), 0);
        let mut minimum = 0;
        let mut maximum = 0;
        assert!(luminate_effect_parameter_colour_count_range(
            colour,
            &raw mut minimum,
            &raw mut maximum
        ));
        assert_eq!((minimum, maximum), (1, 3));

        let direction = luminate_hardware_effect_descriptor_parameter_at(effect, 1);
        assert_eq!(luminate_effect_parameter_kind(direction), 2);
        assert_eq!(luminate_effect_parameter_direction_count(direction), 2);
        assert_eq!(luminate_effect_parameter_direction_at(direction, 1), 1);
        assert_eq!(
            luminate_effect_parameter_direction_at(direction, 2),
            u32::MAX
        );

        let brightness = luminate_hardware_effect_descriptor_parameter_at(effect, 2);
        let mut bits = 0;
        assert!(luminate_effect_parameter_brightness_bits(
            brightness,
            &raw mut bits
        ));
        assert_eq!(bits, 8);
        let choice = luminate_hardware_effect_descriptor_parameter_at(effect, 3);
        assert_eq!(luminate_effect_parameter_choice_count(choice), 1);
        let option = luminate_effect_parameter_choice_at(choice, 0);
        assert_eq!(bytes(luminate_effect_choice_id(option)), b"soft");
        assert_eq!(bytes(luminate_effect_choice_name(option)), b"Soft");
    }
}

#[test]
fn persistence_current_state_and_false_flags_are_reported() {
    let capabilities = CapabilitySet {
        persistence: PersistenceCapability::CurrentState {
            requirement: PersistenceRequirement::Optional,
            explicit_commit: false,
            readback: false,
        },
        state_readback: StateReadbackCapability::Readable {
            facets: Vec::new(),
            read_disturbs_output: false,
            notifies_external_changes: false,
        },
        ..CapabilitySet::default()
    };
    let root = cast_ref::<_, LuminateCapabilitySet>(&capabilities);
    let mut requirement = u32::MAX;
    let mut slots = 11;
    let mut explicit_commit = true;
    let mut readback = true;

    // SAFETY: `root` is a matching live native value for the duration of
    // these accessor calls.
    unsafe {
        assert_eq!(luminate_capability_set_persistence_kind(root), 1);
        assert!(luminate_capability_set_persistence_requirement(
            root,
            &raw mut requirement
        ));
        assert!(!luminate_capability_set_persistence_slots(
            root,
            &raw mut slots
        ));
        assert!(luminate_capability_set_persistence_explicit_commit(
            root,
            &raw mut explicit_commit
        ));
        assert!(luminate_capability_set_persistence_readback(
            root,
            &raw mut readback
        ));
        assert_eq!((requirement, slots), (0, 11));
        assert!(!explicit_commit);
        assert!(!readback);
        assert!(!luminate_capability_set_read_disturbs_output(root));
        assert!(!luminate_capability_set_notifies_external_changes(root));
        assert!(luminate_capability_set_hardware_effects(root).is_null());

        // A `None` brightness capability preserves both outputs.
        let mut bits = 7;
        let mut maximum = 8;
        assert!(!luminate_capability_set_brightness_bits(
            root,
            &raw mut bits
        ));
        assert!(!luminate_capability_set_brightness_maximum(
            root,
            &raw mut maximum
        ));
        assert_eq!((bits, maximum), (7, 8));
    }
}

#[test]
fn power_domain_device_variant_has_no_surface_id() {
    let capabilities = CapabilitySet {
        power_domain: Some(PowerDomainRef::Device),
        ..CapabilitySet::default()
    };
    let root = cast_ref::<_, LuminateCapabilitySet>(&capabilities);

    // SAFETY: `root` is a matching live native value.
    unsafe {
        let domain = luminate_capability_set_power_domain(root);
        assert_eq!(luminate_power_domain_kind(domain), 0);
        assert!(bytes(luminate_power_domain_surface_id(domain)).is_empty());
    }
}

#[test]
fn effect_parameter_speed_and_duration_variants_expose_their_ranges() {
    let parameters = [
        EffectParameter::Speed {
            range: DiscreteRange {
                min: 1,
                max: 10,
                step: 1,
            },
        },
        EffectParameter::Duration {
            milliseconds: DiscreteRange {
                min: 100,
                max: 5000,
                step: 100,
            },
        },
    ];

    // SAFETY: each pointer is a correctly typed view into a live local value.
    unsafe {
        let speed = cast_ref::<_, LuminateEffectParameter>(&parameters[0]);
        assert_eq!(luminate_effect_parameter_kind(speed), 1);
        let mut range = LuminateU16Range {
            minimum: 0,
            maximum: 0,
            step: 0,
        };
        assert!(luminate_effect_parameter_speed_range(speed, &raw mut range));
        assert_eq!((range.minimum, range.maximum, range.step), (1, 10, 1));
        let mut minimum = 7;
        let mut maximum = 8;
        assert!(!luminate_effect_parameter_colour_count_range(
            speed,
            &raw mut minimum,
            &raw mut maximum
        ));
        assert_eq!((minimum, maximum), (7, 8));
        let mut duration_range = LuminateU32Range {
            minimum: 1,
            maximum: 2,
            step: 3,
        };
        assert!(!luminate_effect_parameter_duration_range(
            speed,
            &raw mut duration_range
        ));
        assert_eq!(
            (
                duration_range.minimum,
                duration_range.maximum,
                duration_range.step
            ),
            (1, 2, 3)
        );

        let duration = cast_ref::<_, LuminateEffectParameter>(&parameters[1]);
        assert_eq!(luminate_effect_parameter_kind(duration), 3);
        assert!(luminate_effect_parameter_duration_range(
            duration,
            &raw mut duration_range
        ));
        assert_eq!(
            (
                duration_range.minimum,
                duration_range.maximum,
                duration_range.step
            ),
            (100, 5000, 100)
        );
        assert!(!luminate_effect_parameter_speed_range(
            duration,
            &raw mut range
        ));
    }
}

#[test]
#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "The test checks a matrix of independent null-pointer accessor inputs."
)]
fn null_pointers_return_documented_sentinels_across_nested_accessors() {
    // SAFETY: null is a documented, explicitly supported input for every
    // accessor exercised here.
    unsafe {
        assert_eq!(luminate_colour_capability_encoding(ptr::null()), u32::MAX);
        assert_eq!(luminate_colour_capability_channel_count(ptr::null()), 0);
        assert!(luminate_colour_capability_channel_at(ptr::null(), 0).is_null());
        assert_eq!(
            luminate_colour_channel_capability_channel(ptr::null()),
            u32::MAX
        );
        assert_eq!(luminate_colour_channel_capability_bits(ptr::null()), 0);
        assert_eq!(luminate_frame_upload_scope(ptr::null()), u32::MAX);
        assert_eq!(luminate_frame_upload_update_mode(ptr::null()), u32::MAX);
        assert!(!luminate_frame_upload_has_max_rate_hz(ptr::null()));
        assert_eq!(luminate_frame_upload_max_rate_hz(ptr::null()), 0);
        assert!(!luminate_frame_upload_atomic(ptr::null()));
        assert_eq!(luminate_frame_upload_buffering(ptr::null()), u32::MAX);
        assert_eq!(luminate_hardware_effects_scope(ptr::null()), u32::MAX);
        assert!(!luminate_hardware_effects_concurrent_with_streaming(
            ptr::null()
        ));
        assert_eq!(luminate_hardware_effects_effect_count(ptr::null()), 0);
        assert!(luminate_hardware_effects_effect_at(ptr::null(), 0).is_null());
        assert!(bytes(luminate_hardware_effect_descriptor_id(ptr::null())).is_empty());
        assert!(bytes(luminate_hardware_effect_descriptor_name(ptr::null())).is_empty());
        assert_eq!(
            luminate_hardware_effect_descriptor_parameter_count(ptr::null()),
            0
        );
        assert!(luminate_hardware_effect_descriptor_parameter_at(ptr::null(), 0).is_null());
        assert_eq!(luminate_effect_parameter_kind(ptr::null()), u32::MAX);
        assert!(!luminate_effect_parameter_colour_count_range(
            ptr::null(),
            ptr::null_mut(),
            ptr::null_mut()
        ));
        assert_eq!(luminate_effect_parameter_direction_count(ptr::null()), 0);
        assert_eq!(
            luminate_effect_parameter_direction_at(ptr::null(), 0),
            u32::MAX
        );
        assert!(!luminate_effect_parameter_brightness_bits(
            ptr::null(),
            ptr::null_mut()
        ));
        assert_eq!(luminate_effect_parameter_choice_count(ptr::null()), 0);
        assert!(luminate_effect_parameter_choice_at(ptr::null(), 0).is_null());
        assert!(bytes(luminate_effect_choice_id(ptr::null())).is_empty());
        assert!(bytes(luminate_effect_choice_name(ptr::null())).is_empty());
        assert_eq!(luminate_readable_facet_kind(ptr::null()), u32::MAX);
        assert_eq!(luminate_readable_facet_fidelity(ptr::null()), u32::MAX);
        assert_eq!(luminate_physical_power_scope(ptr::null()), u32::MAX);
    }
}

#[test]
fn absent_capabilities_return_documented_sentinels() {
    let capabilities = CapabilitySet::default();
    let root = cast_ref::<_, LuminateCapabilitySet>(&capabilities);

    // SAFETY: `root` is a matching live native value and null is an
    // explicitly supported accessor input.
    unsafe {
        assert_eq!(luminate_capability_set_brightness_kind(root), 0);
        let mut scope = 9;
        assert!(!luminate_capability_set_brightness_scope(
            root,
            &raw mut scope
        ));
        assert_eq!(scope, 9);
        assert!(luminate_capability_set_frame_upload(root).is_null());
        assert_eq!(luminate_capability_set_persistence_kind(root), 0);
        let mut requirement = 7;
        let mut slots = 8;
        let mut explicit_commit = true;
        let mut readback = true;
        assert!(!luminate_capability_set_persistence_requirement(
            root,
            &raw mut requirement
        ));
        assert!(!luminate_capability_set_persistence_slots(
            root,
            &raw mut slots
        ));
        assert!(!luminate_capability_set_persistence_explicit_commit(
            root,
            &raw mut explicit_commit
        ));
        assert!(!luminate_capability_set_persistence_readback(
            root,
            &raw mut readback
        ));
        assert_eq!(
            (requirement, slots, explicit_commit, readback),
            (7, 8, true, true)
        );
        assert_eq!(luminate_capability_set_state_readback_kind(root), 0);
        assert_eq!(luminate_capability_set_readable_facet_count(root), 0);
        assert_eq!(luminate_capability_set_colour_count(ptr::null()), 0);
        assert_eq!(luminate_capability_set_cct_emulation(ptr::null()), u32::MAX);
        assert_eq!(
            luminate_capability_set_brightness_kind(ptr::null()),
            u32::MAX
        );
        assert!(luminate_capability_set_physical_power(ptr::null()).is_null());
        assert_eq!(luminate_power_domain_kind(ptr::null()), u32::MAX);
    }
}
