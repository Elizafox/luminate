// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Discovery, routing, and process-global registry tests for the Govee plugin.

use std::sync::atomic::{AtomicU32, Ordering};

use super::*;
use luminate_core::effect::EffectArguments;
use transport::fakes::{RecordingTransport, UnavailableTransport};

/// Registry state lives in a process-global [`runtime()`] singleton, so
/// tests running in parallel under the default `cargo test` harness must
/// not share a device identity: a fixed id let concurrent tests
/// overwrite each other's registry entry mid-test.
fn test_device(sku: &str) -> DiscoveredDevice {
    static NEXT_ID: AtomicU32 = AtomicU32::new(0);
    let unique = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    DiscoveredDevice {
        id: format!("govee-test-{unique:08x}"),
        control_address: SocketAddr::from(([192, 0, 2, 10], CONTROL_PORT)),
        sku: sku.to_owned(),
        profile: sku::profile_for(sku),
        ble_version: "1.2".to_owned(),
    }
}

fn register(device: DiscoveredDevice) {
    let fingerprint = topology_fingerprint(&device);
    runtime()
        .devices
        .refresh(Instant::now(), [(device.id.clone(), device, fingerprint)])
        .expect("test registry should be available");
}

#[test]
fn discovery_uses_sender_ip_for_control_traffic() {
    let payload = serde_json::to_vec(&serde_json::json!({
        "msg": {
            "cmd": "scan",
            "data": {
                "ip": "198.51.100.20",
                "device": "AA:BB:CC:DD:EE:FF",
                "sku": "H6022",
                "bleVersionHard": "1",
                "bleVersionSoft": "1.2"
            }
        }
    }))
    .expect("serialize scan reply");
    let sender = SocketAddr::from(([192, 0, 2, 10], 40_002));

    let device = parse_candidate(&payload, sender).expect("parse scan reply");

    assert_eq!(
        device.control_address,
        SocketAddr::from(([192, 0, 2, 10], CONTROL_PORT))
    );
}

fn additive(rgb: Rgb) -> Colour {
    Colour::rgb(rgb)
}

fn cct(kelvin: u32) -> Colour {
    Colour::cct(kelvin)
}

fn full_frame(pixels: Vec<Colour>) -> FrameEnvelope {
    FrameEnvelope {
        generation: 1,
        sequence: 0,
        payload: FramePayload::Full(pixels),
        commit: false,
    }
}

#[test]
fn brightness_zero_maps_to_turn_off_not_a_zero_value() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();

    set_brightness(&device, 0, &transport).expect("brightness 0 succeeds");

    let sent = transport.sent.borrow();
    assert_eq!(sent.len(), 1);
    let body: Value = serde_json::from_slice(&sent[0].1).expect("sent JSON");
    assert_eq!(body, protocol::turn_command(false));
}

#[test]
fn brightness_in_range_sends_the_documented_command() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();

    set_brightness(&device, 42, &transport).expect("brightness 42 succeeds");

    let sent = transport.sent.borrow();
    assert_eq!(sent.len(), 1);
    let body: Value = serde_json::from_slice(&sent[0].1).expect("sent JSON");
    assert_eq!(
        body,
        protocol::brightness_command(42).expect("valid brightness")
    );
}

#[test]
fn additive_colour_sends_colorwc_then_turns_on() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();

    set_colour(&device, &additive(Rgb::new(10, 20, 30)), &transport)
        .expect("additive colour succeeds");

    let sent = transport.sent.borrow();
    assert_eq!(sent.len(), 2);
    let first: Value = serde_json::from_slice(&sent[0].1).expect("sent JSON");
    assert_eq!(first, protocol::colorwc_rgb_command(Rgb::new(10, 20, 30)));
    let second: Value = serde_json::from_slice(&sent[1].1).expect("sent JSON");
    assert_eq!(second, protocol::turn_command(true));
}

#[test]
fn cct_colour_is_rejected_on_a_sku_without_cct_support() {
    let profile: &'static SkuProfile = Box::leak(Box::new(SkuProfile {
        model_name: "no-cct",
        supports_cct: false,
        native_cct_range: None,
        supports_scenes: false,
        scenes: &[],
        matrix: None,
    }));
    let device = DiscoveredDevice {
        profile,
        ..test_device("unknown-sku")
    };
    let transport = RecordingTransport::default();

    let result = set_colour(&device, &cct(4000), &transport);

    assert!(matches!(result, Err(GoveeError::Unsupported(_))));
    assert!(transport.sent.borrow().is_empty());
}

#[test]
fn cct_colour_uses_native_command_within_h6022_range() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();

    set_colour(&device, &cct(6500), &transport).expect("native CCT succeeds");

    let sent = transport.sent.borrow();
    let first: Value = serde_json::from_slice(&sent[0].1).expect("sent JSON");
    assert_eq!(first, protocol::colorwc_cct_command(6500));
}

#[test]
fn out_of_range_cct_colour_uses_rgb_approximation() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();

    set_colour(&device, &cct(8000), &transport).expect("RGB CCT approximation succeeds");

    let sent = transport.sent.borrow();
    let first: Value = serde_json::from_slice(&sent[0].1).expect("sent JSON");
    assert_eq!(first, protocol::colorwc_rgb_command(kelvin_to_rgb(8000)));
    assert_eq!(sent.len(), 2);
}

#[test]
fn effect_off_and_static_are_accepted_other_variants_are_rejected() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();

    apply_effect(&device, &Effect::Off, &transport).expect("off succeeds");
    apply_effect(
        &device,
        &Effect::Static {
            colour: Colour::rgb(Rgb::new(1, 2, 3)),
        },
        &transport,
    )
    .expect("static succeeds");
    assert_eq!(transport.sent.borrow().len(), 3);

    let rejected = apply_effect(
        &device,
        &Effect::Breathe {
            colour: Rgb::new(1, 2, 3),
            period_ms: 1000,
        },
        &transport,
    );
    assert!(matches!(rejected, Err(GoveeError::Unsupported(_))));
}

#[test]
fn known_hardware_scene_sends_scene_frame_then_turns_on() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();

    apply_effect(
        &device,
        &Effect::Hardware {
            id: HardwareEffectId::new("fire"),
            arguments: EffectArguments::default(),
        },
        &transport,
    )
    .expect("known scene succeeds");

    let sent = transport.sent.borrow();
    assert_eq!(sent.len(), 2);
    let first: Value = serde_json::from_slice(&sent[0].1).expect("sent JSON");
    let expected_frame = ptreal::scene_frame(0x28).expect("build scene frame");
    assert_eq!(
        first,
        ptreal::ptreal_command(&[expected_frame]),
        "Fire is code 0x28"
    );
    let second: Value = serde_json::from_slice(&sent[1].1).expect("sent JSON");
    assert_eq!(second, protocol::turn_command(true));
}

#[test]
fn unknown_hardware_scene_is_rejected_without_touching_the_transport() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();

    let result = apply_effect(
        &device,
        &Effect::Hardware {
            id: HardwareEffectId::new("rainbow"),
            arguments: EffectArguments::default(),
        },
        &transport,
    );

    assert!(matches!(result, Err(GoveeError::Invalid(_))));
    assert!(transport.sent.borrow().is_empty());
}

#[test]
fn hardware_scene_on_a_sku_with_no_scene_table_is_rejected() {
    let device = test_device("unknown-sku");
    let transport = RecordingTransport::default();

    let result = apply_effect(
        &device,
        &Effect::Hardware {
            id: HardwareEffectId::new("fire"),
            arguments: EffectArguments::default(),
        },
        &transport,
    );

    assert!(matches!(result, Err(GoveeError::Invalid(_))));
    assert!(transport.sent.borrow().is_empty());
}

#[test]
fn first_matrix_cell_update_rejects_an_unknown_shadow() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();
    register(device.clone());
    let update = PluginUpdate {
        target: PluginTarget::Element {
            device: device.id.clone(),
            surface: MATRIX_SURFACE_ID.to_owned(),
            element: "cell-r0-c0".to_owned(),
        },
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: additive(Rgb::new(10, 20, 30)),
            },
        },
    };

    let error = apply_update(&update, &transport).expect_err("unknown shadow must be rejected");

    assert!(matches!(error, GoveeError::Unsupported(_)));
    assert!(transport.sent.borrow().is_empty());
    let shadows = runtime().matrix_shadows.lock().expect("shadow lock");
    assert!(!shadows.contains_key(&device.id));
}

#[test]
fn successive_matrix_updates_preserve_prior_cells() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();
    register(device.clone());
    apply_device_operation(
        &device,
        &PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: additive(Rgb::BLACK),
            },
        },
        &transport,
    )
    .expect("complete colour establishes the shadow");
    let first = PluginUpdate {
        target: PluginTarget::Element {
            device: device.id.clone(),
            surface: MATRIX_SURFACE_ID.to_owned(),
            element: "cell-r0-c0".to_owned(),
        },
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: additive(Rgb::new(1, 2, 3)),
            },
        },
    };
    let second = PluginUpdate {
        target: PluginTarget::Element {
            device: device.id.clone(),
            surface: MATRIX_SURFACE_ID.to_owned(),
            element: "cell-r10-c11".to_owned(),
        },
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: additive(Rgb::new(4, 5, 6)),
            },
        },
    };

    apply_update(&first, &transport).expect("first cell succeeds");
    apply_update(&second, &transport).expect("second cell succeeds");

    let shadows = runtime().matrix_shadows.lock().expect("shadow lock");
    let framebuffer = shadows.get(&device.id).expect("shadow exists");
    assert_eq!(framebuffer[0], Rgb::new(1, 2, 3));
    assert_eq!(framebuffer[131], Rgb::new(4, 5, 6));
}

#[test]
fn matrix_cell_update_preserves_a_whole_matrix_rgb_colour() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();
    register(device.clone());
    let background = Rgb::new(10, 20, 30);
    let foreground = Rgb::new(40, 50, 60);

    apply_update(
        &PluginUpdate {
            target: PluginTarget::Surface {
                device: device.id.clone(),
                surface: MATRIX_SURFACE_ID.to_owned(),
            },
            operation: PluginUpdateOperation::SetEffect {
                effect: Effect::Static {
                    colour: additive(background),
                },
            },
        },
        &transport,
    )
    .expect("whole-matrix colour succeeds");
    apply_update(
        &PluginUpdate {
            target: PluginTarget::Element {
                device: device.id.clone(),
                surface: MATRIX_SURFACE_ID.to_owned(),
                element: "cell-r0-c0".to_owned(),
            },
            operation: PluginUpdateOperation::SetEffect {
                effect: Effect::Static {
                    colour: additive(foreground),
                },
            },
        },
        &transport,
    )
    .expect("matrix cell colour succeeds");

    let shadows = runtime().matrix_shadows.lock().expect("shadow lock");
    let framebuffer = shadows.get(&device.id).expect("shadow exists");
    assert_eq!(framebuffer[0], foreground);
    assert!(framebuffer[1..].iter().all(|colour| *colour == background));
}

#[test]
fn matrix_transport_failure_invalidates_the_shadow() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();
    register(device.clone());
    apply_device_operation(
        &device,
        &PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: additive(Rgb::BLACK),
            },
        },
        &transport,
    )
    .expect("complete colour establishes the shadow");
    let successful = PluginUpdate {
        target: PluginTarget::Element {
            device: device.id.clone(),
            surface: MATRIX_SURFACE_ID.to_owned(),
            element: "cell-r0-c0".to_owned(),
        },
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: additive(Rgb::new(1, 2, 3)),
            },
        },
    };
    apply_update(&successful, &transport).expect("initial cell succeeds");

    let failing = PluginUpdate {
        target: PluginTarget::Element {
            device: device.id.clone(),
            surface: MATRIX_SURFACE_ID.to_owned(),
            element: "cell-r0-c1".to_owned(),
        },
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: additive(Rgb::new(4, 5, 6)),
            },
        },
    };
    let result = apply_update(&failing, &UnavailableTransport);

    assert!(matches!(result, Err(GoveeError::Io(_))));
    let shadows = runtime().matrix_shadows.lock().expect("shadow lock");
    assert!(!shadows.contains_key(&device.id));
}

#[test]
fn full_matrix_frame_is_uploaded_and_replaces_the_shadow() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();
    register(device.clone());
    let mut pixels = vec![additive(Rgb::BLACK); h6022_matrix::CELL_COUNT];
    pixels[0] = additive(Rgb::new(1, 2, 3));
    pixels[131] = additive(Rgb::new(4, 5, 6));
    let target = PluginTarget::Surface {
        device: device.id.clone(),
        surface: MATRIX_SURFACE_ID.to_owned(),
    };

    apply_frame(&target, &full_frame(pixels), &transport).expect("frame upload succeeds");

    let sent = transport.sent.borrow();
    assert_eq!(sent.len(), 1);
    let body: Value = serde_json::from_slice(&sent[0].1).expect("sent JSON");
    assert_eq!(body["msg"]["cmd"], "ptReal");
    let shadows = runtime().matrix_shadows.lock().expect("shadow lock");
    let framebuffer = shadows.get(&device.id).expect("frame shadow");
    assert_eq!(framebuffer[0], Rgb::new(1, 2, 3));
    assert_eq!(framebuffer[131], Rgb::new(4, 5, 6));
}

#[test]
fn matrix_frame_rejects_partial_and_incorrectly_sized_payloads() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();
    register(device.clone());
    let target = PluginTarget::Surface {
        device: device.id,
        surface: MATRIX_SURFACE_ID.to_owned(),
    };
    let partial = FrameEnvelope {
        generation: 1,
        sequence: 0,
        payload: FramePayload::Partial(vec![(0, additive(Rgb::WHITE))]),
        commit: false,
    };

    assert!(matches!(
        apply_frame(&target, &partial, &transport),
        Err(GoveeError::Unsupported(_))
    ));
    assert!(matches!(
        apply_frame(
            &target,
            &full_frame(vec![additive(Rgb::BLACK); h6022_matrix::CELL_COUNT - 1]),
            &transport
        ),
        Err(GoveeError::Invalid(_))
    ));
    assert!(transport.sent.borrow().is_empty());
}

#[test]
fn failed_matrix_frame_invalidates_the_shadow() {
    let device = test_device("H6022");
    register(device.clone());
    runtime()
        .matrix_shadows
        .lock()
        .expect("shadow lock")
        .insert(
            device.id.clone(),
            vec![Rgb::new(7, 8, 9); h6022_matrix::CELL_COUNT],
        );
    let target = PluginTarget::Surface {
        device: device.id.clone(),
        surface: MATRIX_SURFACE_ID.to_owned(),
    };

    let result = apply_frame(
        &target,
        &full_frame(vec![additive(Rgb::new(1, 2, 3)); h6022_matrix::CELL_COUNT]),
        &UnavailableTransport,
    );

    assert!(matches!(result, Err(GoveeError::Io(_))));
    let shadows = runtime().matrix_shadows.lock().expect("shadow lock");
    assert!(!shadows.contains_key(&device.id));
}

#[test]
fn invalid_matrix_cell_ids_are_rejected_without_transport() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();
    register(device.clone());
    for element in [
        "cell-r11-c0",
        "cell-r0-c12",
        "cell-r-1-c0",
        "cell-r0-c0-extra",
        "segment-0",
    ] {
        let update = PluginUpdate {
            target: PluginTarget::Element {
                device: device.id.clone(),
                surface: MATRIX_SURFACE_ID.to_owned(),
                element: element.to_owned(),
            },
            operation: PluginUpdateOperation::SetEffect {
                effect: Effect::Static {
                    colour: additive(Rgb::new(1, 2, 3)),
                },
            },
        };
        assert!(matches!(
            apply_update(&update, &transport),
            Err(GoveeError::Invalid(_))
        ));
    }
    assert!(transport.sent.borrow().is_empty());
}

#[test]
fn unknown_surface_id_is_rejected() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();
    register(device.clone());
    let update = PluginUpdate {
        target: PluginTarget::Element {
            device: device.id.clone(),
            surface: "not-matrix".to_owned(),
            element: "cell-r0-c0".to_owned(),
        },
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: additive(Rgb::new(1, 2, 3)),
            },
        },
    };

    let result = apply_update(&update, &transport);

    assert!(matches!(result, Err(GoveeError::Invalid(_))));
    assert!(transport.sent.borrow().is_empty());
}

#[test]
fn matrix_target_on_a_sku_with_no_matrix_is_rejected() {
    let device = test_device("unknown-sku");
    let transport = RecordingTransport::default();
    register(device.clone());
    let update = PluginUpdate {
        target: PluginTarget::Element {
            device: device.id.clone(),
            surface: MATRIX_SURFACE_ID.to_owned(),
            element: "cell-r0-c0".to_owned(),
        },
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: additive(Rgb::new(1, 2, 3)),
            },
        },
    };

    let result = apply_update(&update, &transport);

    assert!(matches!(result, Err(GoveeError::Invalid(_))));
    assert!(transport.sent.borrow().is_empty());
}

#[test]
fn matrix_target_only_supports_static_effect() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();
    register(device.clone());
    let update = PluginUpdate {
        target: PluginTarget::Element {
            device: device.id.clone(),
            surface: MATRIX_SURFACE_ID.to_owned(),
            element: "cell-r0-c0".to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };

    let result = apply_update(&update, &transport);

    assert!(matches!(result, Err(GoveeError::Unsupported(_))));
    assert!(transport.sent.borrow().is_empty());
}

#[test]
fn clear_turns_the_device_off() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();
    runtime()
        .matrix_shadows
        .lock()
        .expect("shadow lock")
        .insert(
            device.id.clone(),
            vec![Rgb::new(1, 2, 3); h6022_matrix::CELL_COUNT],
        );

    let update = PluginUpdate {
        target: PluginTarget::Device {
            device: device.id.clone(),
        },
        operation: PluginUpdateOperation::Clear,
    };
    register(device);

    apply_update(&update, &transport).expect("clear succeeds");

    let sent = transport.sent.borrow();
    assert_eq!(sent.len(), 1);
    let body: Value = serde_json::from_slice(&sent[0].1).expect("sent JSON");
    assert_eq!(body, protocol::turn_command(false));
    assert!(
        !runtime()
            .matrix_shadows
            .lock()
            .expect("shadow lock")
            .contains_key(update.target.device_id())
    );
}

#[test]
fn save_current_is_a_no_op_success() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();
    let update = PluginUpdate {
        target: PluginTarget::Device {
            device: device.id.clone(),
        },
        operation: PluginUpdateOperation::SaveCurrent,
    };
    register(device);

    apply_update(&update, &transport).expect("save-current succeeds");

    assert!(transport.sent.borrow().is_empty());
}

#[test]
fn group_targets_are_rejected_without_touching_the_transport() {
    let transport = RecordingTransport::default();
    let update = PluginUpdate {
        target: PluginTarget::Group {
            device: "govee-aabbccddeeff".to_owned(),
            group: "all".to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };

    let result = apply_update(&update, &transport);

    assert!(matches!(result, Err(GoveeError::Invalid(_))));
    assert!(transport.sent.borrow().is_empty());
}

#[test]
fn unknown_device_id_is_unavailable() {
    let transport = RecordingTransport::default();
    let update = PluginUpdate {
        target: PluginTarget::Device {
            device: "govee-does-not-exist".to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };

    let result = apply_update(&update, &transport);

    assert!(matches!(result, Err(GoveeError::Unavailable(_))));
}

#[test]
fn transport_failure_propagates_as_io_or_unavailable() {
    let device = test_device("H6022");

    let result = set_brightness(&device, 50, &UnavailableTransport);

    assert!(matches!(
        result,
        Err(GoveeError::Unavailable(_) | GoveeError::Io(_))
    ));
}

#[test]
fn classify_maps_timeouts_to_unavailable() {
    assert!(matches!(
        classify_govee_error(GoveeError::Io("request timed out".to_owned())),
        PluginError::Unavailable(_)
    ));
    assert!(matches!(
        classify_govee_error(GoveeError::Io("connection refused".to_owned())),
        PluginError::Io(_)
    ));
}

#[test]
fn fallback_sku_profile_is_surfaced_as_a_warning() {
    let mut device = test_device("H6022");
    device.sku = "H1234-UNKNOWN".to_owned();
    device.profile = sku::profile_for(&device.sku);

    let descriptor = device_descriptor(&device);

    assert!(
        descriptor
            .warnings
            .iter()
            .any(|warning| warning.contains("Unrecognized Govee SKU"))
    );
}

#[test]
fn known_sku_advertises_cct_capability() {
    let capabilities = device_capabilities(sku::profile_for("H6022"));
    assert_eq!(capabilities.colour.len(), 2);
}

#[test]
fn govee_off_is_wear_safe_even_with_write_through_persistence() {
    let capabilities = device_capabilities(sku::profile_for("H6022"));

    assert!(capabilities.off_is_wear_safe);
    assert!(matches!(
        capabilities.persistence,
        PersistenceCapability::CurrentState {
            requirement: PersistenceRequirement::Required,
            ..
        }
    ));
}

#[test]
fn known_sku_advertises_its_scene_table_as_hardware_effects() {
    let capabilities = device_capabilities(sku::profile_for("H6022"));
    let effects = capabilities
        .hardware_effects
        .expect("H6022 advertises hardware effects");
    assert_eq!(effects.effects.len(), 12);
    assert!(
        effects
            .effects
            .iter()
            .any(|effect| effect.id.as_str() == "fire" && effect.name == "Fire")
    );
    assert!(effects.effects.iter().any(|effect| {
        effect.id.as_str() == "rainbow-drawing" && effect.name == "Rainbow Drawing"
    }));
    assert!(effects.effects.iter().any(|effect| {
        effect.id.as_str() == "rainbow-striped" && effect.name == "Rainbow Striped"
    }));
}

#[test]
fn fallback_sku_advertises_no_hardware_effects() {
    let capabilities = device_capabilities(sku::profile_for("H1234-UNKNOWN"));
    assert!(capabilities.hardware_effects.is_none());
    assert_eq!(capabilities.colour, [ColourCapability::rgb8()]);
    assert_eq!(capabilities.cct_emulation, CctEmulation::Auto);
}

#[test]
fn known_sku_descriptor_has_the_h6022_matrix_surface() {
    let descriptor = device_descriptor(&test_device("H6022"));
    assert_eq!(descriptor.surfaces.len(), 1);
    let surface = &descriptor.surfaces[0];
    assert_eq!(surface.id, MATRIX_SURFACE_ID);
    assert_eq!(surface.kind, SurfaceKind::Matrix { rows: 11, cols: 12 });
    assert_eq!(descriptor.physical_tags, ["shape:cylinder"]);
    assert_eq!(surface.physical_tags, ["layout:wrapped-horizontal"]);
    assert_eq!(surface.elements.len(), 132);
    assert_eq!(surface.elements[0].id, "cell-r0-c0");
    assert_eq!(surface.elements[0].kind, ElementKind::Led);
    assert_eq!(
        surface.elements[0].geometry,
        Some(ElementGeometry::MatrixCell { row: 0, col: 0 })
    );
    assert_eq!(surface.elements[131].id, "cell-r10-c11");
    assert_eq!(
        surface.capabilities.frame_upload,
        Some(FrameUploadCapability {
            scope: CapabilityScope::Surface,
            update_mode: FrameUpdateMode::FullFrameOnly,
            max_rate_hz: Some(H6022_MAX_FRAME_RATE_HZ),
            atomic: true,
            buffering: BufferingMode::Immediate,
            shm: None,
        })
    );
    assert_eq!(surface.capabilities.colour, descriptor.capabilities.colour);
    assert_eq!(
        surface.capabilities.brightness,
        BrightnessCapability::Independent {
            bits: 7,
            maximum: 100,
            scope: CapabilityScope::Surface,
        }
    );
    assert_eq!(
        surface
            .capabilities
            .hardware_effects
            .as_ref()
            .expect("matrix surface advertises hardware effects")
            .scope,
        CapabilityScope::Surface
    );
    assert!(surface.capabilities.emission);
    assert!(surface.capabilities.physical_power.is_none());
    assert_eq!(
        surface.capabilities.power_domain,
        Some(PowerDomainRef::Device)
    );
    assert_eq!(surface.elements[0].capabilities.colour.len(), 1);
    assert!(matches!(
        surface.elements[0].capabilities.brightness,
        BrightnessCapability::None
    ));
}

#[test]
fn matrix_surface_routes_appearance_updates_to_the_whole_device() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();
    register(device.clone());

    apply_update(
        &PluginUpdate {
            target: PluginTarget::Surface {
                device: device.id,
                surface: MATRIX_SURFACE_ID.to_owned(),
            },
            operation: PluginUpdateOperation::SetEffect {
                effect: Effect::Static { colour: cct(4000) },
            },
        },
        &transport,
    )
    .expect("matrix surface CCT update succeeds");

    let sent = transport.sent.borrow();
    assert_eq!(sent.len(), 2);
    let first: Value = serde_json::from_slice(&sent[0].1).expect("sent JSON");
    assert_eq!(first, protocol::colorwc_cct_command(4000));
    let second: Value = serde_json::from_slice(&sent[1].1).expect("sent JSON");
    assert_eq!(second, protocol::turn_command(true));
}

#[test]
fn unknown_surface_update_is_rejected_without_transport_io() {
    let device = test_device("H6022");
    let transport = RecordingTransport::default();
    register(device.clone());

    let result = apply_update(
        &PluginUpdate {
            target: PluginTarget::Surface {
                device: device.id,
                surface: "unknown".to_owned(),
            },
            operation: PluginUpdateOperation::Clear,
        },
        &transport,
    );

    assert!(matches!(result, Err(GoveeError::Invalid(_))));
    assert!(transport.sent.borrow().is_empty());
}

#[test]
fn fallback_sku_descriptor_has_no_surfaces() {
    let descriptor = device_descriptor(&test_device("unknown-sku"));
    assert!(descriptor.surfaces.is_empty());
}

#[test]
fn device_id_normalizes_the_reported_identity() {
    assert_eq!(
        device_id("AA:BB:CC:DD:EE:FF").as_deref(),
        Some("govee-aabbccddeeff")
    );
    assert_eq!(
        device_id("aa-bb-cc-dd-ee-ff").as_deref(),
        Some("govee-aabbccddeeff")
    );
    for invalid in [
        "",
        "bad",
        "GG:BB:CC:DD:EE:FF",
        "AA.BB.CC.DD.EE.FF",
        "AA:BB:CC:DD:EE:FF:00",
    ] {
        assert_eq!(device_id(invalid), None, "{invalid:?} must be rejected");
    }
}

#[test]
fn duplicate_identity_from_different_addresses_is_omitted() {
    let mut discovered = HashMap::new();
    let mut ambiguous = HashSet::new();
    let first = DiscoveredDevice {
        id: "govee-aabbccddeeff".to_owned(),
        ..test_device("H6022")
    };
    let second = DiscoveredDevice {
        control_address: SocketAddr::from(([192, 0, 2, 11], CONTROL_PORT)),
        ..first.clone()
    };

    record_candidate(&mut discovered, &mut ambiguous, first);
    record_candidate(&mut discovered, &mut ambiguous, second);

    assert!(discovered.is_empty());
    assert!(ambiguous.contains("govee-aabbccddeeff"));
}
