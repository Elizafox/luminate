// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;
use crate::convert::{direction_id, effect_parameter, parse_direction};
use crate::effect_request::{MAX_EFFECT_COLOURS, StaticColourRequest};
use crate::model::{Details, DeviceDetails, ElementDetails, GroupDetails, SurfaceDetails};

use luminate::capability::{
    EffectDirection, EffectParameter, HardwareEffectDescriptor, HardwareEffectId,
};
use luminate::effect::EffectArguments;
use luminate::{AppearanceState, Effect, EffectiveAppearanceState, FacetValue, Rgb};

use std::collections::HashMap;
use std::fs;

use luminate::AppearanceSlotUpdatePolicy;
use luminate::CapabilitySet;
use luminate::appearance_slot::{
    AppearanceCapability, AppearanceSlotDescriptor, AppearanceSlotId, AppearanceSlotsCapability,
};
use luminate::capability::{
    CapabilityScope, ColourCapability, EffectChoice, HardwareEffectsCapability,
    PersistenceCapability, PhysicalPowerCapability,
};
use luminate::{Colour, EmissionState, PhysicalPowerState};
use luminate_platform::test_support::TestDir;
use std::sync::atomic::AtomicU64;

fn permission_denial(error: MethodError) -> String {
    let MethodError::PermissionDenied(message) = error else {
        panic!("expected permission denial, got {error:?}");
    };
    message
}

fn test_shared() -> Arc<Shared> {
    Arc::new(Shared {
        client: RwLock::new(None),
        socket_path: None,
        attestation_sequence: AtomicU64::new(1),
        attested_clients: Mutex::new(HashMap::new()),
        objects: RwLock::new(BTreeMap::new()),
        required_gid: 0,
        polkit: false,
        proc_root: PathBuf::from("/proc"),
    })
}

fn test_target(target_id: TargetId, capabilities: CapabilitySet) -> Target {
    Target::new(
        test_shared(),
        Object {
            path: "/org/luminate/Luminate1/devices/test".into(),
            target: target_id,
            kind: Kind::Device,
            name: "Test".into(),
            capabilities,
            details: Details::Other,
        },
    )
}

#[test]
fn facet_strings_reports_streaming_effective_appearance() {
    let (facet, value) = facet_strings(&FacetValue::EffectiveAppearance(
        EffectiveAppearanceState::Streaming,
    ));
    assert_eq!(facet, "effective-appearance");
    assert_eq!(value, "streaming");
}

#[test]
fn facet_strings_reports_mixed_aggregate_appearance() {
    assert_eq!(
        facet_strings(&FacetValue::Appearance(AppearanceState::Mixed)),
        ("appearance".to_owned(), "mixed".to_owned())
    );
    assert_eq!(
        facet_strings(&FacetValue::EffectiveAppearance(
            EffectiveAppearanceState::Mixed
        )),
        ("effective-appearance".to_owned(), "mixed".to_owned())
    );
}

#[test]
fn facet_strings_reports_static_and_animated_appearance() {
    let colour = Colour::rgb(Rgb::new(1, 2, 3));
    let (facet, value) = facet_strings(&FacetValue::Appearance(AppearanceState::Static(
        colour.clone(),
    )));
    assert_eq!(facet, "appearance");
    assert_eq!(value, format!("{colour:?}"));

    let (facet, value) = facet_strings(&FacetValue::Appearance(AppearanceState::Effect(
        Effect::Off,
    )));
    assert_eq!(facet, "appearance");
    assert_eq!(value, "effect");
}

#[test]
fn facet_strings_reports_brightness_emission_and_physical_power() {
    let (facet, value) = facet_strings(&FacetValue::Brightness(42));
    assert_eq!(facet, "brightness");
    assert_eq!(value, "42");

    let (facet, value) = facet_strings(&FacetValue::Emission(EmissionState::Emitting));
    assert_eq!(facet, "emission");
    assert_eq!(value, format!("{:?}", EmissionState::Emitting));

    let (facet, value) = facet_strings(&FacetValue::PhysicalPower(PhysicalPowerState::On));
    assert_eq!(facet, "physical-power");
    assert_eq!(value, format!("{:?}", PhysicalPowerState::On));
}

#[test]
fn facet_strings_reports_effective_appearance_off_static_and_effect() {
    let (facet, value) = facet_strings(&FacetValue::EffectiveAppearance(
        EffectiveAppearanceState::Off,
    ));
    assert_eq!(facet, "effective-appearance");
    assert_eq!(value, "off");

    let colour = Colour::rgb(Rgb::new(9, 8, 7));
    let (facet, value) = facet_strings(&FacetValue::EffectiveAppearance(
        EffectiveAppearanceState::Static(colour.clone()),
    ));
    assert_eq!(facet, "effective-appearance");
    assert_eq!(value, format!("{colour:?}"));

    let (facet, value) = facet_strings(&FacetValue::EffectiveAppearance(
        EffectiveAppearanceState::Effect(Effect::Off),
    ));
    assert_eq!(facet, "effective-appearance");
    assert_eq!(value, "effect");
}

#[test]
fn direction_strings_round_trip_through_parse_and_id() {
    let directions = [
        EffectDirection::Forward,
        EffectDirection::Reverse,
        EffectDirection::Clockwise,
        EffectDirection::CounterClockwise,
        EffectDirection::Inward,
        EffectDirection::Outward,
        EffectDirection::Random,
    ];
    for direction in directions {
        let id = direction_id(direction);
        assert_eq!(
            parse_direction(id).expect("known direction ids should parse"),
            direction
        );
    }

    let error = parse_direction("sideways").expect_err("unknown directions must be rejected");
    assert!(matches!(error, MethodError::InvalidArgument(_)));
}

#[test]
fn effect_parameter_maps_colour_bounds_and_brightness_bit_widths() {
    let colour = EffectParameter::Colour {
        minimum_colours: 1,
        maximum_colours: 4,
    };
    assert_eq!(
        effect_parameter(&colour),
        ("colour".into(), 1, 4, 1, Vec::new())
    );

    assert_eq!(
        effect_parameter(&EffectParameter::Brightness { bits: 0 }),
        ("brightness".into(), 0, 0, 1, Vec::new())
    );
    assert_eq!(
        effect_parameter(&EffectParameter::Brightness { bits: 8 }),
        ("brightness".into(), 0, 255, 1, Vec::new())
    );
    assert_eq!(
        effect_parameter(&EffectParameter::Brightness { bits: 32 }),
        ("brightness".into(), 0, u32::MAX, 1, Vec::new())
    );
}

#[test]
fn polkit_check_uses_absolute_path_and_noninteractive_arguments() {
    let command = polkit_command(Path::new(PKCHECK_PATH), "123,456,789");

    assert_eq!(command.as_std().get_program(), PKCHECK_PATH);
    assert_eq!(
        command.as_std().get_args().collect::<Vec<_>>(),
        [
            "--action-id",
            "org.luminate.control",
            "--process",
            "123,456,789"
        ]
    );
}

#[test]
fn executable_lookup_uses_the_first_match_then_falls_back() {
    let root = TestDir::new("pkcheck-path");
    let first = root.join("first");
    let second = root.join("second");
    fs::create_dir_all(&first).expect("create first search directory");
    fs::create_dir_all(&second).expect("create second search directory");
    fs::write(second.join("pkcheck"), "").expect("create later candidate");
    fs::write(first.join("pkcheck"), "").expect("create earlier candidate");

    let first = first.to_str().expect("test path is UTF-8");
    let second = second.to_str().expect("test path is UTF-8");
    let fallback = root.join("fallback/pkcheck");

    assert_eq!(
        find_executable("pkcheck", &[first, second], &fallback),
        Path::new(first).join("pkcheck")
    );
    assert_eq!(
        find_executable("other", &[first, second], &fallback),
        fallback
    );
}

#[cfg(unix)]
#[tokio::test]
async fn polkit_check_denies_unsuccessful_status() {
    let command = Command::new("/usr/bin/false");

    let error = run_polkit_command(command, Duration::from_secs(1))
        .await
        .expect_err("unsuccessful pkcheck should deny authorization");

    assert_eq!(permission_denial(error), "Polkit denied the mutation");
}

#[cfg(unix)]
#[tokio::test]
async fn polkit_check_allows_successful_status() {
    let command = Command::new("/usr/bin/true");

    run_polkit_command(command, Duration::from_secs(1))
        .await
        .expect("successful pkcheck should authorize the mutation");
}

#[tokio::test]
async fn polkit_check_denies_spawn_failure() {
    let error = authorize_with_polkit(
        Path::new("/path/that/does/not/exist/pkcheck"),
        "123,456,789",
        Duration::from_secs(1),
    )
    .await
    .expect_err("a missing pkcheck should deny authorization");

    assert!(!permission_denial(error).is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn polkit_check_denies_timeout() {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg("while :; do :; done")
        .kill_on_drop(true);

    let error = run_polkit_command(command, Duration::from_millis(20))
        .await
        .expect_err("a hung pkcheck should deny authorization");

    assert_eq!(permission_denial(error), "Polkit authorization timed out");
}

#[test]
fn canonical_identifiers_include_scope() {
    assert_eq!(canonical_id(&TargetId::group("d", "g")), "device:d/group:g");
}

#[test]
fn portable_effect_requests_map_to_consumer_types() {
    let effect = EffectRequest {
        kind: "morph".into(),
        colours: vec![(1, 2, 3), (4, 5, 6)].into(),
        period_ms: Some(750),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect("valid morph request");

    assert_eq!(
        effect,
        Effect::Morph {
            colours: vec![Rgb::new(1, 2, 3), Rgb::new(4, 5, 6)],
            period_ms: 750,
        }
    );
}

#[test]
fn hardware_effect_requests_map_all_arguments() {
    let effect = EffectRequest {
        kind: "hardware".into(),
        colours: vec![(10, 20, 30)].into(),
        hardware_id: Some("scene".into()),
        speed: Some(3),
        direction: Some("counter-clockwise".into()),
        duration_ms: Some(1_000),
        brightness: Some(127),
        choice: Some("aurora".into()),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect("valid hardware request");

    assert_eq!(
        effect,
        Effect::Hardware {
            id: HardwareEffectId::new("scene"),
            arguments: EffectArguments {
                colours: vec![Rgb::new(10, 20, 30)],
                speed: Some(3),
                direction: Some(EffectDirection::CounterClockwise),
                duration_ms: Some(1_000),
                brightness: Some(127),
                choice: Some("aurora".into()),
            },
        }
    );
}

#[test]
fn malformed_effect_requests_are_rejected_at_the_dbus_boundary() {
    let error = EffectRequest {
        kind: "rainbow".into(),
        period_ms: Some(500),
        speed: Some(2),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect_err("portable effects cannot carry hardware arguments");
    assert!(matches!(error, MethodError::InvalidArgument(_)));

    let error = EffectRequest {
        kind: "hardware".into(),
        hardware_id: Some("scene".into()),
        direction: Some("sideways".into()),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect_err("unknown directions must fail");
    assert!(matches!(error, MethodError::InvalidArgument(_)));
}

#[test]
fn off_effect_requests_reject_colours_and_period_but_accept_bare_off() {
    let error = EffectRequest {
        kind: "off".into(),
        colours: vec![(1, 2, 3)].into(),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect_err("off does not accept colours");
    assert!(matches!(error, MethodError::InvalidArgument(_)));

    let error = EffectRequest {
        kind: "off".into(),
        period_ms: Some(10),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect_err("off does not accept a period");
    assert!(matches!(error, MethodError::InvalidArgument(_)));

    let effect = EffectRequest {
        kind: "off".into(),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect("bare off request is valid");
    assert_eq!(effect, Effect::Off);
}

#[test]
fn static_effect_requests_use_the_typed_colour_dictionary() {
    let error = EffectRequest {
        kind: "static".into(),
        colours: vec![(1, 2, 3), (4, 5, 6)].into(),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect_err("static rejects the animated-effect RGB field");
    assert!(matches!(error, MethodError::InvalidArgument(_)));

    let effect = EffectRequest {
        kind: "static".into(),
        static_colour: Some(StaticColourRequest {
            model: "hsv".to_owned(),
            channels: HashMap::from([
                ("hue".to_owned(), 7),
                ("saturation".to_owned(), 8),
                ("value".to_owned(), 9),
            ]),
        }),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect("typed static request is valid");
    assert_eq!(
        effect,
        Effect::Static {
            colour: Colour::hsv(7, 8, 9)
        }
    );
}

#[test]
fn periodic_single_colour_effect_requests_map_to_matching_variants() {
    let colour = (11, 22, 33);
    let cases = [
        (
            "breathe",
            Effect::Breathe {
                colour: Rgb::new(11, 22, 33),
                period_ms: 100,
            },
        ),
        (
            "pulse",
            Effect::Pulse {
                colour: Rgb::new(11, 22, 33),
                period_ms: 100,
            },
        ),
        (
            "strobe",
            Effect::Strobe {
                colour: Rgb::new(11, 22, 33),
                period_ms: 100,
            },
        ),
        (
            "scanner",
            Effect::Scanner {
                colour: Rgb::new(11, 22, 33),
                period_ms: 100,
            },
        ),
    ];
    for (kind, expected) in cases {
        let effect = EffectRequest {
            kind: kind.into(),
            colours: vec![colour].into(),
            period_ms: Some(100),
            ..EffectRequest::default()
        }
        .into_effect()
        .unwrap_or_else(|error| panic!("{kind} request should be valid: {error:?}"));
        assert_eq!(effect, expected, "{kind} mapped to the wrong effect");
    }

    let error = EffectRequest {
        kind: "breathe".into(),
        colours: vec![colour].into(),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect_err("breathe requires PeriodMs");
    assert!(matches!(error, MethodError::InvalidArgument(_)));
}

#[test]
fn morph_effect_requests_require_at_least_one_colour() {
    let error = EffectRequest {
        kind: "morph".into(),
        period_ms: Some(500),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect_err("morph requires at least one colour");
    assert!(matches!(error, MethodError::InvalidArgument(_)));
}

#[test]
fn spectrum_and_rainbow_effect_requests_reject_colours_and_map_correctly() {
    for kind in ["spectrum", "rainbow"] {
        let error = EffectRequest {
            kind: kind.into(),
            colours: vec![(1, 2, 3)].into(),
            period_ms: Some(500),
            ..EffectRequest::default()
        }
        .into_effect()
        .expect_err("spectrum and rainbow do not accept colours");
        assert!(matches!(error, MethodError::InvalidArgument(_)));
    }

    let spectrum = EffectRequest {
        kind: "spectrum".into(),
        period_ms: Some(200),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect("spectrum request is valid");
    assert_eq!(spectrum, Effect::Spectrum { period_ms: 200 });

    let rainbow = EffectRequest {
        kind: "rainbow".into(),
        period_ms: Some(300),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect("rainbow request is valid");
    assert_eq!(rainbow, Effect::Rainbow { period_ms: 300 });
}

#[test]
fn unknown_effect_kinds_are_rejected() {
    let error = EffectRequest {
        kind: "sparkle".into(),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect_err("unknown effect kinds must be rejected");
    assert!(matches!(error, MethodError::InvalidArgument(_)));
}

#[test]
fn hardware_effect_requests_require_hardware_id_and_reject_a_period() {
    let error = EffectRequest {
        kind: "hardware".into(),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect_err("hardware effects require HardwareId");
    assert!(matches!(error, MethodError::InvalidArgument(_)));

    let error = EffectRequest {
        kind: "hardware".into(),
        hardware_id: Some("scene".into()),
        period_ms: Some(500),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect_err("hardware effects do not accept PeriodMs");
    assert!(matches!(error, MethodError::InvalidArgument(_)));
}

#[test]
fn oversized_effect_colour_bags_are_rejected_at_the_dbus_boundary() {
    let error = EffectRequest {
        kind: "morph".into(),
        colours: vec![(1, 2, 3); MAX_EFFECT_COLOURS + 1].into(),
        period_ms: Some(500),
        ..EffectRequest::default()
    }
    .into_effect()
    .expect_err("oversized colour bags must fail");

    assert!(matches!(error, MethodError::InvalidArgument(_)));
}

#[test]
fn hardware_descriptors_preserve_ranges_and_named_choices() {
    let descriptor = HardwareEffectDescriptor {
        id: HardwareEffectId::new("scene"),
        name: "Scene".into(),
        parameters: vec![
            EffectParameter::Direction {
                values: vec![EffectDirection::Forward, EffectDirection::Reverse],
            },
            EffectParameter::Choice {
                options: vec![EffectChoice {
                    id: "aurora".into(),
                    name: "Aurora".into(),
                }],
            },
        ],
    };

    assert_eq!(
        effect_descriptor(&descriptor),
        (
            "scene".into(),
            "Scene".into(),
            vec![
                (
                    "direction".into(),
                    0,
                    0,
                    0,
                    vec![
                        ("forward".into(), "forward".into()),
                        ("reverse".into(), "reverse".into()),
                    ],
                ),
                (
                    "choice".into(),
                    0,
                    0,
                    0,
                    vec![("aurora".into(), "Aurora".into())],
                ),
            ],
        )
    );
}

#[test]
fn typed_interfaces_report_their_constructed_name() {
    assert_eq!(DeviceInterface::new("Device".into()).name(), "Device");
    assert_eq!(SurfaceInterface::new("Surface".into()).name(), "Surface");
    assert_eq!(GroupInterface::new("Group".into()).name(), "Group");
    assert_eq!(ElementInterface::new("Element".into()).name(), "Element");
}

#[test]
fn legacy_element_interface_remains_name_only() {
    let mut xml = String::new();
    ElementInterface::new("Element".into()).introspect_to_writer(&mut xml, 0);

    assert!(xml.contains("<property name=\"Name\""));
    assert!(!xml.contains("PhysicalTags"));
}

#[test]
fn device2_preserves_optional_metadata_and_object_path_relationships() {
    let interface = Device2::new(
        "Keyboard".into(),
        DeviceDetails {
            id: "keyboard".into(),
            vendor: Some("Acme".into()),
            model: None,
            provider_instance: Some("usb".into()),
            category: Some("keyboard".into()),
            physical_tags: vec!["host-peripheral".into()],
            host_attached: true,
            notes: vec!["note".into()],
            warnings: vec!["warning".into()],
            surfaces: vec!["/org/luminate/Luminate1/devices/d/surfaces/s".into()],
            groups: vec!["/org/luminate/Luminate1/devices/d/groups/g".into()],
        },
    )
    .expect("fixture paths are valid D-Bus object paths");

    assert_eq!(interface.id(), "keyboard");
    assert_eq!(interface.name(), "Keyboard");
    assert!(interface.has_vendor());
    assert_eq!(interface.vendor(), "Acme");
    assert!(!interface.has_model());
    assert_eq!(interface.model(), "");
    assert!(interface.has_provider_instance());
    assert_eq!(interface.provider_instance(), "usb");
    assert!(interface.has_category());
    assert_eq!(interface.category(), "keyboard");
    assert_eq!(interface.physical_tags(), ["host-peripheral"]);
    assert!(interface.host_attached());
    assert_eq!(interface.notes(), ["note"]);
    assert_eq!(interface.warnings(), ["warning"]);
    assert_eq!(
        interface.surfaces()[0].as_str(),
        "/org/luminate/Luminate1/devices/d/surfaces/s"
    );
    assert_eq!(
        interface.groups()[0].as_str(),
        "/org/luminate/Luminate1/devices/d/groups/g"
    );
}

#[test]
fn surface2_preserves_layout_and_ordered_element_paths() {
    let interface = Surface2::new(
        "Matrix".into(),
        SurfaceDetails {
            id: "keys".into(),
            kind: "matrix".into(),
            length: None,
            dimensions: None,
            matrix: Some((6, 22)),
            physical_tags: vec!["keyboard-keys".into()],
            elements: vec![
                "/org/luminate/Luminate1/devices/d/surfaces/s/elements/a".into(),
                "/org/luminate/Luminate1/devices/d/surfaces/s/elements/b".into(),
            ],
            notes: vec!["note".into()],
            warnings: Vec::new(),
        },
    )
    .expect("fixture element paths are valid");

    assert_eq!(interface.kind(), "matrix");
    assert!(!interface.has_length());
    assert!(!interface.has_dimensions());
    assert!(interface.has_matrix());
    assert_eq!((interface.rows(), interface.columns()), (6, 22));
    assert!(interface.elements()[0].as_str().ends_with("/elements/a"));
    assert!(interface.elements()[1].as_str().ends_with("/elements/b"));
}

#[test]
fn element2_preserves_optional_name_and_rect_geometry() {
    let interface = Element2::new(ElementDetails {
        id: "escape".into(),
        name: Some("Escape".into()),
        kind: "key".into(),
        geometry_kind: Some("rect".into()),
        x: Some(0.1),
        y: Some(0.2),
        width: Some(0.3),
        height: Some(0.4),
        position: None,
        matrix_cell: None,
        surface: "/org/luminate/Luminate1/devices/d/surfaces/s".into(),
        physical_tags: vec!["shape:keycap".into(), "position:escape".into()],
        notes: Vec::new(),
        warnings: Vec::new(),
    })
    .expect("fixture surface path is valid");

    assert!(interface.has_name());
    assert_eq!(interface.name(), "Escape");
    assert!(interface.has_geometry());
    assert_eq!(interface.geometry_kind(), "rect");
    assert!((interface.x() - 0.1).abs() < f64::EPSILON);
    assert!((interface.y() - 0.2).abs() < f64::EPSILON);
    assert!((interface.width() - 0.3).abs() < f64::EPSILON);
    assert!((interface.height() - 0.4).abs() < f64::EPSILON);
    assert!(interface.position().abs() < f64::EPSILON);
    assert_eq!(
        interface.physical_tags(),
        ["shape:keycap", "position:escape"]
    );
}

#[test]
fn group2_preserves_description_kind_and_ordered_members() {
    let interface = Group2::new(
        "Media keys".into(),
        GroupDetails {
            id: "media".into(),
            description: Some("Keyboard media controls".into()),
            kind: "topology".into(),
            members: vec![
                "/org/luminate/Luminate1/devices/d/surfaces/s".into(),
                "/org/luminate/Luminate1/devices/d/surfaces/s/elements/e".into(),
                "/org/luminate/Luminate1/devices/d/groups/nested".into(),
            ],
            notes: vec!["note".into()],
            warnings: vec!["warning".into()],
        },
    )
    .expect("fixture member paths are valid");

    assert_eq!(interface.id(), "media");
    assert_eq!(interface.name(), "Media keys");
    assert!(interface.has_description());
    assert_eq!(interface.description(), "Keyboard media controls");
    assert_eq!(interface.kind(), "topology");
    assert!(interface.members()[0].as_str().ends_with("/surfaces/s"));
    assert!(interface.members()[1].as_str().ends_with("/elements/e"));
    assert!(interface.members()[2].as_str().ends_with("/groups/nested"));
}

#[test]
fn introspection_exposes_effect_method_and_capability_properties() {
    let target = test_target(TargetId::device("test"), CapabilitySet::default());
    let mut xml = String::new();
    target.introspect_to_writer(&mut xml, 0);

    assert!(xml.contains("<method name=\"SetEffect\">"));
    assert!(xml.contains("name=\"request\" type=\"a{sv}\" direction=\"in\""));
    assert!(xml.contains("<property name=\"CanSetEffect\" type=\"b\" access=\"read\"/>"));
    assert!(xml.contains("<property name=\"EffectDescriptors\""));
    assert!(xml.contains("<method name=\"SetAppearanceSlots\">"));
    assert!(xml.contains("<property name=\"AppearanceSlotUpdatePolicy\""));
    assert!(xml.contains("<property name=\"AppearanceSlotDescriptors\""));
}

#[test]
fn target_appearance_slot_properties_preserve_descriptor_order_and_policy() {
    let capabilities = CapabilitySet {
        appearance_slots: Some(AppearanceSlotsCapability {
            slots: vec![AppearanceSlotDescriptor {
                id: AppearanceSlotId::new("ac"),
                name: "AC".to_owned(),
                appearance: AppearanceCapability {
                    colour: vec![ColourCapability::rgb8()],
                    ..AppearanceCapability::default()
                },
                persistence: PersistenceCapability::None,
                notes: vec!["note".to_owned()],
                warnings: vec!["warning".to_owned()],
            }],
            update_policy: AppearanceSlotUpdatePolicy::PartialIfKnown,
        }),
        ..CapabilitySet::default()
    };
    let target = test_target(TargetId::surface("test", "power"), capabilities);

    assert_eq!(target.appearance_slot_update_policy(), "partial-if-known");
    let descriptors = target.appearance_slot_descriptors();
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].0, "ac");
    assert_eq!(descriptors[0].1, "AC");
    assert_eq!(descriptors[0].4, vec!["note"]);
    assert_eq!(descriptors[0].5, vec!["warning"]);
}

#[test]
fn manager_introspection_exposes_management_and_scene_surface() {
    let manager = Manager::new(test_shared());
    let mut xml = String::new();
    manager.introspect_to_writer(&mut xml, 0);

    assert!(xml.contains("<method name=\"GetManagement\">"));
    assert!(xml.contains("type=\"a{sv}\" direction=\"out\""));
    assert!(xml.contains("<method name=\"PatchManagement\">"));
    assert!(xml.contains("name=\"mutations\" type=\"aa{sv}\" direction=\"in\""));
    assert!(xml.contains("<signal name=\"ConfigurationChanged\">"));
    for method in [
        "ListScenes",
        "GetScene",
        "CreateScene",
        "CaptureScene",
        "ReplaceScene",
        "RecaptureScene",
        "DeleteScene",
        "ApplyScene",
    ] {
        assert!(xml.contains(&format!("<method name=\"{method}\">")));
    }
    assert!(xml.contains("<signal name=\"ScenesChanged\">"));
}

fn append_topology_introspection(contract: &mut String, mut xml: &mut String) {
    DeviceInterface::new("Device".into()).introspect_to_writer(&mut xml, 0);
    contract.push_str("DEVICE\n");
    contract.push_str(xml);
    xml.clear();
    Device2::new(
        "Device".into(),
        DeviceDetails {
            id: "test".into(),
            vendor: None,
            model: None,
            provider_instance: None,
            category: None,
            physical_tags: Vec::new(),
            host_attached: false,
            notes: Vec::new(),
            warnings: Vec::new(),
            surfaces: Vec::new(),
            groups: Vec::new(),
        },
    )
    .expect("empty object-path arrays are valid")
    .introspect_to_writer(&mut xml, 0);
    contract.push_str("DEVICE2\n");
    contract.push_str(xml);
    xml.clear();
    SurfaceInterface::new("Surface".into()).introspect_to_writer(&mut xml, 0);
    contract.push_str("SURFACE\n");
    contract.push_str(xml);
    xml.clear();
    Surface2::new(
        "Surface".into(),
        SurfaceDetails {
            id: "surface".into(),
            kind: "zone".into(),
            length: None,
            dimensions: None,
            matrix: None,
            physical_tags: Vec::new(),
            elements: Vec::new(),
            notes: Vec::new(),
            warnings: Vec::new(),
        },
    )
    .expect("empty object-path arrays are valid")
    .introspect_to_writer(&mut xml, 0);
    contract.push_str("SURFACE2\n");
    contract.push_str(xml);
    xml.clear();
    GroupInterface::new("Group".into()).introspect_to_writer(&mut xml, 0);
    contract.push_str("GROUP\n");
    contract.push_str(xml);
    xml.clear();
    Group2::new(
        "Group".into(),
        GroupDetails {
            id: "group".into(),
            description: None,
            kind: "built-in".into(),
            members: Vec::new(),
            notes: Vec::new(),
            warnings: Vec::new(),
        },
    )
    .expect("empty object-path arrays are valid")
    .introspect_to_writer(&mut xml, 0);
    contract.push_str("GROUP2\n");
    contract.push_str(xml);
    xml.clear();
    ElementInterface::new("Element".into()).introspect_to_writer(&mut xml, 0);
    contract.push_str("ELEMENT\n");
    contract.push_str(xml);
    xml.clear();
    Element2::new(ElementDetails {
        id: "element".into(),
        name: None,
        kind: "led".into(),
        geometry_kind: None,
        x: None,
        y: None,
        width: None,
        height: None,
        position: None,
        matrix_cell: None,
        surface: "/org/luminate/Luminate1/devices/d/surfaces/s".into(),
        physical_tags: Vec::new(),
        notes: Vec::new(),
        warnings: Vec::new(),
    })
    .expect("fixture surface path is valid")
    .introspect_to_writer(&mut xml, 0);
    contract.push_str("ELEMENT2\n");
    contract.push_str(xml);
}

#[test]
fn introspection_contract_matches_snapshot() {
    let mut xml = String::new();
    Manager::new(test_shared()).introspect_to_writer(&mut xml, 0);
    let mut contract = format!("MANAGER\n{xml}");

    xml.clear();
    Manager2::new(test_shared()).introspect_to_writer(&mut xml, 0);
    contract.push_str("MANAGER2\n");
    contract.push_str(&xml);

    xml.clear();
    test_target(TargetId::device("test"), CapabilitySet::default())
        .introspect_to_writer(&mut xml, 0);
    contract.push_str("TARGET\n");
    contract.push_str(&xml);

    xml.clear();
    let target = test_target(TargetId::device("test"), CapabilitySet::default());
    Target3::new(test_shared(), target.object)
        .expect("default capabilities have a D-Bus representation")
        .introspect_to_writer(&mut xml, 0);
    contract.push_str("TARGET3\n");
    contract.push_str(&xml);

    xml.clear();
    append_topology_introspection(&mut contract, &mut xml);

    let snapshot = include_str!("../testdata/introspection-contract.xml");
    let (_, expected_contract) = snapshot
        .split_once("\n\n")
        .expect("the introspection snapshot has an SPDX header");
    assert_eq!(contract, expected_contract);
}

#[test]
fn target_identifier_matches_the_canonical_target_id() {
    let target = test_target(TargetId::group("d", "g"), CapabilitySet::default());
    assert_eq!(target.identifier(), "device:d/group:g");
}

#[test]
fn target_capability_properties_are_false_and_empty_with_no_capabilities() {
    let target = test_target(TargetId::device("test"), CapabilitySet::default());
    assert!(target.static_colour_capabilities().is_empty());
    assert!(!target.can_set_brightness());
    assert_eq!(target.brightness_maximum(), 0);
    assert!(!target.can_set_off());
    assert!(!target.can_set_effect());
    assert!(target.effect_descriptors().is_empty());
}

#[test]
fn target_capability_properties_reflect_colour_and_brightness_capabilities() {
    let capabilities = CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: 255,
            scope: CapabilityScope::Device,
        },
        ..CapabilitySet::default()
    };
    let target = test_target(TargetId::device("test"), capabilities);
    assert_eq!(
        target.static_colour_capabilities(),
        vec![(
            "additive".to_owned(),
            vec![
                ("red".to_owned(), 8),
                ("green".to_owned(), 8),
                ("blue".to_owned(), 8),
            ],
        )]
    );
    assert!(target.can_set_brightness());
    assert_eq!(target.brightness_maximum(), 255);
    // A colour capability is enough to advertise SetEffect because portable
    // colour effects do not require hardware-effect support.
    assert!(target.can_set_effect());
}

#[test]
fn target_can_set_off_reflects_emission_and_physical_power_independently() {
    let emission_only = CapabilitySet {
        emission: true,
        ..CapabilitySet::default()
    };
    assert!(test_target(TargetId::device("test"), emission_only).can_set_off());

    let physical_power_only = CapabilitySet {
        physical_power: Some(PhysicalPowerCapability {
            scope: CapabilityScope::Device,
        }),
        ..CapabilitySet::default()
    };
    assert!(test_target(TargetId::device("test"), physical_power_only).can_set_off());

    assert!(!test_target(TargetId::device("test"), CapabilitySet::default()).can_set_off());
}

#[test]
fn target_can_set_effect_and_descriptors_reflect_hardware_effects() {
    let descriptor = HardwareEffectDescriptor {
        id: HardwareEffectId::new("scene"),
        name: "Scene".into(),
        parameters: Vec::new(),
    };
    let with_effects = CapabilitySet {
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![descriptor.clone()],
            scope: CapabilityScope::Device,
            concurrent_with_streaming: false,
        }),
        ..CapabilitySet::default()
    };
    let target = test_target(TargetId::device("test"), with_effects);
    assert!(target.can_set_effect());
    assert_eq!(
        target.effect_descriptors(),
        vec![effect_descriptor(&descriptor)]
    );

    let empty_effects = CapabilitySet {
        hardware_effects: Some(HardwareEffectsCapability {
            effects: Vec::new(),
            scope: CapabilityScope::Device,
            concurrent_with_streaming: false,
        }),
        ..CapabilitySet::default()
    };
    assert!(!test_target(TargetId::device("test"), empty_effects).can_set_effect());
}

#[tokio::test]
async fn shared_client_reports_daemon_unavailable_when_disconnected() {
    let shared = test_shared();
    let error = shared
        .client()
        .await
        .expect_err("no client should be daemon-unavailable");
    assert!(matches!(error, MethodError::DaemonUnavailable(_)));
}

#[tokio::test]
async fn manager_available_reflects_client_presence() {
    let manager = Manager::new(test_shared());
    assert!(!manager.available().await);
}

#[test]
fn session_records_expose_only_sanitized_identity_and_lease_metadata() {
    let sources = [
        AuthenticationSource::Peer,
        AuthenticationSource::Bearer,
        AuthenticationSource::Attestation {
            name: "frontend".into(),
        },
        AuthenticationSource::External {
            provider: "sso".into(),
        },
    ];
    for source in sources {
        let encoded = session_record(&luminate::SessionMetadata {
            subject: PrincipalId::new("local", "operator").expect("principal"),
            verified_groups: vec!["lighting".into()],
            source,
            credential_id: Some("non-secret-id".into()),
            expires_at: Some(UNIX_EPOCH + Duration::from_secs(1)),
        })
        .expect("encode session");

        assert!(encoded.contains_key("Authority"));
        assert!(encoded.contains_key("CredentialId"));
        assert!(!encoded.contains_key("Credential"));
        assert!(!encoded.contains_key("Secret"));
    }
}

#[test]
fn attestation_records_do_not_contain_display_once_secrets() {
    let encoded = attestation_record(luminate::AttestationMetadata {
        name: "frontend".into(),
        subject: PrincipalId::new("local", "operator").expect("principal"),
        verified_groups: vec!["lighting".into()],
        credential_id: "non-secret-id".into(),
        expires_at: None,
    });

    assert_eq!(encoded.name, "frontend");
    assert_eq!(encoded.credential_id, "non-secret-id");
    assert!(!encoded.has_expiry);
}
