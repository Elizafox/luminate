// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Discovery, capability, targeting, and state-readback tests for LIFX devices.

use std::panic::{self, AssertUnwindSafe};

use luminate_core::effect::EffectArguments;

use super::*;

#[test]
fn plugin_configuration_defaults_to_broadcast_and_validates_overrides() {
    assert_eq!(
        parse_lifx_configuration(&serde_json::json!({}))
            .expect("default configuration")
            .discovery_address,
        SocketAddrV4::new(Ipv4Addr::BROADCAST, DISCOVERY_PORT)
    );
    assert_eq!(
        parse_lifx_configuration(&serde_json::json!({
            "discovery_address": "127.0.0.1:12345"
        }))
        .expect("loopback override")
        .discovery_address,
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, 12_345)
    );
    assert!(
        parse_lifx_configuration(&serde_json::json!({
            "discovery_address": "[::1]:56700"
        }))
        .is_err()
    );
    assert!(
        parse_lifx_configuration(&serde_json::json!({
            "unknown": true
        }))
        .is_err()
    );
}

#[test]
fn rgb_primary_colours_convert_to_expected_hues() {
    assert_eq!(rgb_to_hsbk(Rgb::new(255, 0, 0)).hue, 0);
    assert_eq!(rgb_to_hsbk(Rgb::new(0, 255, 0)).hue, 21_845);
    assert_eq!(rgb_to_hsbk(Rgb::new(0, 0, 255)).hue, 43_691);
    assert_eq!(rgb_to_hsbk(Rgb::new(255, 255, 255)).saturation, 0);
}

#[test]
fn hsbk_primary_colours_convert_back_to_rgb() {
    assert_eq!(
        hsbk_to_rgb(Hsbk {
            hue: 0,
            saturation: u16::MAX,
            brightness: u16::MAX,
            kelvin: DEFAULT_KELVIN,
        }),
        Rgb::new(255, 0, 0)
    );
    assert_eq!(hsbk_to_rgb(off_hsbk()), Rgb::BLACK);
}

#[test]
fn appearance_reports_cct_for_desaturated_readback() {
    let appearance = appearance_from_hsbk(Hsbk {
        hue: 0,
        saturation: 0,
        brightness: u16::MAX,
        kelvin: 3_000,
    });
    assert_eq!(appearance, Colour::cct(3_000));
}

#[test]
fn appearance_reports_additive_rgb_for_saturated_readback() {
    let appearance = appearance_from_hsbk(Hsbk {
        hue: 0,
        saturation: u16::MAX,
        brightness: u16::MAX,
        kelvin: DEFAULT_KELVIN,
    });
    assert_eq!(appearance, Colour::rgb(Rgb::new(255, 0, 0)));
}

#[test]
fn physical_power_is_advertised_only_at_the_device_scope() {
    assert!(bulb_capabilities().physical_power.is_some());
    assert!(
        linear_capabilities(CapabilityScope::Surface)
            .physical_power
            .is_none()
    );
    assert!(zone_capabilities().physical_power.is_none());
}

#[test]
fn service_response_uses_advertised_port_and_sender_ip() {
    let target = [0xd0, 0x73, 0xd5, 1, 2, 3, 0, 0];
    let mut payload = vec![1];
    payload.extend_from_slice(&56_701_u32.to_le_bytes());
    let packet = protocol::packet(
        42,
        target,
        0,
        protocol::STATE_SERVICE,
        &payload,
        false,
        false,
    )
    .expect("packet should encode");
    let sender = "192.0.2.10:1234".parse().expect("address should parse");
    assert_eq!(
        parse_service(&packet, 42, sender),
        Some((
            target,
            "192.0.2.10:56701".parse().expect("address should parse")
        ))
    );
}

#[test]
fn discovery_candidate_collection_is_bounded_and_updates_duplicates() {
    let mut candidates = HashMap::new();
    for index in 0..(MAX_CANDIDATES_PER_CYCLE + 20) {
        let mut target = [0_u8; 8];
        target[..8].copy_from_slice(&(index as u64).to_le_bytes());
        let last_octet = u8::try_from(index + 1).expect("test address fits");
        let address = SocketAddr::from(([192, 0, 2, last_octet], DISCOVERY_PORT));
        insert_discovery_candidate(&mut candidates, target, address);
    }
    assert_eq!(candidates.len(), MAX_CANDIDATES_PER_CYCLE);

    let replacement: SocketAddr = "192.0.2.2:56700".parse().expect("replacement address");
    insert_discovery_candidate(&mut candidates, [0; 8], replacement);
    assert_eq!(candidates.len(), MAX_CANDIDATES_PER_CYCLE);
    assert_eq!(candidates.get(&[0; 8]), Some(&replacement));
}

#[test]
fn discovery_candidate_collection_is_bounded_per_source() {
    let mut candidates = HashMap::new();
    let address = SocketAddr::from(([192, 0, 2, 1], DISCOVERY_PORT));
    for index in 0..=MAX_DISCOVERED_DEVICES_PER_SOURCE {
        let mut target = [0_u8; 8];
        target[..8].copy_from_slice(&(index as u64).to_le_bytes());
        insert_discovery_candidate(&mut candidates, target, address);
    }

    assert_eq!(candidates.len(), MAX_DISCOVERED_DEVICES_PER_SOURCE);
}

#[test]
fn unvalidated_requests_do_not_allocate_sequence_state() {
    let target = [0xd0, 0x73, 0xd5, 9, 8, 7, 0, 0];
    lock_sequences(&runtime().sequences)
        .expect("sequence state should be available")
        .remove(&target);

    assert_eq!(
        sequence_for_request(target, false).expect("non-persistent request should succeed"),
        0
    );

    assert!(
        !lock_sequences(&runtime().sequences)
            .expect("sequence state should be available")
            .contains_key(&target)
    );
}

#[test]
fn poisoned_sequence_state_is_not_recovered() {
    let sequences = Mutex::new(HashMap::new());
    let poisoned = panic::catch_unwind(AssertUnwindSafe(|| {
        let _sequences = sequences.lock().expect("fresh sequence lock");
        panic!("deliberately poison sequence lock");
    }));
    assert!(poisoned.is_err());

    let access = panic::catch_unwind(AssertUnwindSafe(|| lock_sequences(&sequences)));
    assert!(access.is_err());
}

#[test]
fn sequence_state_is_pruned_with_expired_devices() {
    let active = [1_u8; 8];
    let expired = [2_u8; 8];
    let mut sequences = HashMap::from([(active, 7), (expired, 9)]);
    let active_targets = HashSet::from([active]);

    prune_sequences(&mut sequences, &active_targets);

    assert_eq!(sequences, HashMap::from([(active, 7)]));
}

#[test]
fn topology_exposes_only_plain_bulb_capabilities() {
    let descriptor = device_descriptor(&DiscoveredDevice {
        id: "lifx-d073d5001337".to_owned(),
        target: [0xd0, 0x73, 0xd5, 0, 0x13, 0x37, 0, 0],
        address: "192.0.2.1:56700".parse().expect("address should parse"),
        vendor: 1,
        product: 23,
        product_name: "LIFX (A19)",
        topology: ProductTopology::Plain,
        zone_count: None,
        label: "Desk".to_owned(),
    });
    assert_eq!(descriptor.name, "Desk (d073d5001337)");
    assert_eq!(descriptor.physical_tags, ["shape:a19"]);
    assert!(descriptor.surfaces.is_empty());
    let effects = descriptor
        .capabilities
        .hardware_effects
        .expect("bulb should advertise effects");
    assert_eq!(effects.effects.len(), 3);
    assert_eq!(effects.effects[0].id.as_str(), "breathe");
    assert_eq!(effects.effects[1].id.as_str(), "pulse");
    assert_eq!(effects.effects[2].id.as_str(), "strobe");
}

#[test]
fn parser_handles_full_width_non_nul_label() {
    assert_eq!(parse_label(&[b'x'; 32]), "x".repeat(32));
}

#[test]
fn legacy_zone_count_accepts_dynamic_protocol_range() {
    assert_eq!(parse_legacy_zone_count(&[1]).expect("one zone"), 1);
    assert_eq!(
        parse_legacy_zone_count(&[u8::MAX]).expect("maximum legacy zones"),
        MAX_LINEAR_ZONES
    );
    assert!(parse_legacy_zone_count(&[0]).is_err());
    assert!(parse_legacy_zone_count(&[]).is_err());
}

#[test]
fn linear_topology_uses_dynamic_zone_count_and_normalized_positions() {
    let descriptor = device_descriptor(&DiscoveredDevice {
        id: "lifx-d073d5001338".to_owned(),
        target: [0xd0, 0x73, 0xd5, 0, 0x13, 0x38, 0, 0],
        address: "192.0.2.2:56700".parse().expect("address should parse"),
        vendor: 1,
        product: 38,
        product_name: "LIFX Beam",
        topology: ProductTopology::Linear,
        zone_count: Some(80),
        label: "Beam".to_owned(),
    });
    assert_eq!(descriptor.surfaces.len(), 1);
    let surface = &descriptor.surfaces[0];
    assert!(
        matches!(surface.kind, SurfaceKind::Linear { length } if (length - 80.0).abs() < f32::EPSILON)
    );
    assert_eq!(surface.elements.len(), 80);
    assert_eq!(descriptor.physical_tags, ["shape:modular-light-bar"]);
    assert!(surface.physical_tags.is_empty());
    assert_eq!(surface.elements[0].id, "zone-0");
    assert!(matches!(
        surface.elements[79].geometry,
        Some(ElementGeometry::Linear { position }) if (position - 1.0).abs() < f32::EPSILON
    ));
    assert!(
        surface
            .warnings
            .iter()
            .any(|warning| warning.contains("not been live-tested"))
    );
}

#[test]
fn target_scope_accepts_only_the_discovered_device_shape() {
    let plain = transition_device(
        "127.0.0.1:56700".parse().expect("address"),
        ProductTopology::Plain,
    );
    assert_eq!(
        resolve_scope(
            &plain,
            &PluginTarget::Device {
                device: plain.id.clone(),
            },
        ),
        Ok(TargetScope::Whole)
    );
    assert!(
        resolve_scope(
            &plain,
            &PluginTarget::Surface {
                device: plain.id.clone(),
                surface: ZONES_SURFACE_ID.to_owned(),
            },
        )
        .is_err()
    );

    let linear = transition_device(
        "127.0.0.1:56700".parse().expect("address"),
        ProductTopology::Linear,
    );
    assert_eq!(
        resolve_scope(
            &linear,
            &PluginTarget::Surface {
                device: linear.id.clone(),
                surface: ZONES_SURFACE_ID.to_owned(),
            },
        ),
        Ok(TargetScope::Whole)
    );
    assert_eq!(
        resolve_scope(
            &linear,
            &PluginTarget::Element {
                device: linear.id.clone(),
                surface: ZONES_SURFACE_ID.to_owned(),
                element: "zone-3".to_owned(),
            },
        ),
        Ok(TargetScope::Zone(3))
    );
    for element in ["pixel-0", "zone-nope", "zone-4"] {
        assert!(
            resolve_scope(
                &linear,
                &PluginTarget::Element {
                    device: linear.id.clone(),
                    surface: ZONES_SURFACE_ID.to_owned(),
                    element: element.to_owned(),
                },
            )
            .is_err(),
            "{element} must not resolve"
        );
    }
}

#[test]
fn unsupported_effects_fail_before_network_io() {
    let plain = transition_device(
        "127.0.0.1:9".parse().expect("address"),
        ProductTopology::Plain,
    );
    let linear = transition_device(
        "127.0.0.1:9".parse().expect("address"),
        ProductTopology::Linear,
    );

    assert!(apply_effect(&plain, &Effect::Rainbow { period_ms: 500 }).is_err());
    assert!(
        apply_effect(
            &plain,
            &Effect::Hardware {
                id: HardwareEffectId::new("test"),
                arguments: EffectArguments::default(),
            },
        )
        .is_err()
    );
    assert!(
        apply_zone_effect(
            &linear,
            0,
            &Effect::Breathe {
                colour: Rgb::new(1, 2, 3),
                period_ms: 500,
            },
        )
        .is_err()
    );
    assert!(
        apply_zone_effect(
            &linear,
            0,
            &Effect::Hardware {
                id: HardwareEffectId::new("test"),
                arguments: EffectArguments::default(),
            },
        )
        .is_err()
    );
}

#[test]
fn colour_and_zone_validation_fail_before_network_io() {
    let device = transition_device(
        "127.0.0.1:9".parse().expect("address"),
        ProductTopology::Linear,
    );

    assert!(hsbk_from_colour(&device, &Colour::monochrome(10)).is_err());
    assert!(hsbk_from_zone_colour(&device, 0, &Colour::monochrome(10)).is_err());
    assert!(kelvin_from_colour(&Colour::cct(u32::from(u16::MAX) + 1)).is_err());
    assert!(set_brightness(&device, u32::from(u8::MAX) + 1).is_err());
    assert!(set_zone_brightness(&device, 0, u32::from(u8::MAX) + 1).is_err());
    assert!(get_zone_colour(&device, u16::from(u8::MAX) + 1).is_err());
    assert!(set_zone_colour(&device, u16::from(u8::MAX) + 1, off_hsbk(), 1).is_err());
    assert!(set_zone_range(&device, 0, u16::from(u8::MAX) + 1, off_hsbk(), 1).is_err());

    let mut no_zones = device;
    no_zones.zone_count = Some(0);
    assert!(linear_zone_count(&no_zones).is_err());
    no_zones.zone_count = None;
    assert!(linear_zone_count(&no_zones).is_err());
}

#[test]
fn acknowledged_request_ignores_unrelated_udp_frame() {
    let server = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind mock LIFX device");
    server
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set mock timeout");
    let address = server.local_addr().expect("read mock address");
    let target = [0xd0, 0x73, 0xd5, 1, 2, 3, 0, 0];
    let mock = thread::spawn(move || {
        let mut buffer = [0; 256];
        let (length, client) = server.recv_from(&mut buffer).expect("receive request");
        let (header, _) = protocol::parse(&buffer[..length]).expect("parse request");
        let unrelated = protocol::packet(
            header.source,
            header.target,
            header.sequence,
            protocol::STATE_LABEL,
            &[0; 32],
            false,
            false,
        )
        .expect("encode unrelated response");
        server
            .send_to(&unrelated, client)
            .expect("send unrelated response");
        let acknowledgement = protocol::packet(
            header.source,
            header.target,
            header.sequence,
            protocol::ACKNOWLEDGEMENT,
            &[],
            false,
            false,
        )
        .expect("encode acknowledgement");
        server
            .send_to(&acknowledgement, client)
            .expect("send acknowledgement");
    });

    let response = request(
        address,
        target,
        protocol::SET_POWER,
        &protocol::set_power_payload(true),
        protocol::ACKNOWLEDGEMENT,
        true,
    )
    .expect("acknowledged request should succeed");
    assert_eq!(
        protocol::parse(&response)
            .expect("parse acknowledgement")
            .0
            .message_type,
        protocol::ACKNOWLEDGEMENT
    );
    mock.join().expect("mock thread should finish");
}

#[test]
fn request_ignores_matching_response_from_wrong_endpoint() {
    let server = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind mock LIFX device");
    server
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set mock timeout");
    let wrong_endpoint =
        UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind unrelated endpoint");
    let address = server.local_addr().expect("read mock address");
    let target = [0xd0, 0x73, 0xd5, 6, 5, 4, 0, 0];
    let mock = thread::spawn(move || {
        let mut buffer = [0; 256];
        let (length, client) = server.recv_from(&mut buffer).expect("receive request");
        let (header, _) = protocol::parse(&buffer[..length]).expect("parse request");
        let wrong_response = protocol::packet(
            header.source,
            header.target,
            header.sequence,
            protocol::STATE_LABEL,
            &[b'w'; 32],
            false,
            false,
        )
        .expect("encode wrong-endpoint response");
        wrong_endpoint
            .send_to(&wrong_response, client)
            .expect("send wrong-endpoint response");

        thread::sleep(Duration::from_millis(50));
        let real_response = protocol::packet(
            header.source,
            header.target,
            header.sequence,
            protocol::STATE_LABEL,
            &[b'r'; 32],
            false,
            false,
        )
        .expect("encode real response");
        server
            .send_to(&real_response, client)
            .expect("send real response");
    });

    let response = request(
        address,
        target,
        protocol::GET_LABEL,
        &[],
        protocol::STATE_LABEL,
        false,
    )
    .expect("request should wait for the expected endpoint");
    let (_, payload) = protocol::parse(&response).expect("parse label response");
    assert_eq!(payload, &[b'r'; 32]);
    mock.join().expect("mock thread should finish");
}

fn transition_device(address: SocketAddr, topology: ProductTopology) -> DiscoveredDevice {
    DiscoveredDevice {
        id: "lifx-transition-test".to_owned(),
        target: [0xd0, 0x73, 0xd5, 4, 5, 6, 0, 0],
        address,
        vendor: 1,
        product: 38,
        product_name: "test",
        topology,
        zone_count: (topology == ProductTopology::Linear).then_some(4),
        label: String::new(),
    }
}

type CapturedPackets = Vec<(u16, Vec<u8>)>;

fn mock_acknowledgements(count: usize) -> (SocketAddr, thread::JoinHandle<CapturedPackets>) {
    let server = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind mock LIFX device");
    server
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set mock timeout");
    let address = server.local_addr().expect("read mock address");
    let mock = thread::spawn(move || {
        let mut received = Vec::new();
        let mut buffer = [0; 256];
        for _ in 0..count {
            let (length, client) = server.recv_from(&mut buffer).expect("receive request");
            let (header, payload) = protocol::parse(&buffer[..length]).expect("parse request");
            received.push((header.message_type, payload.to_vec()));
            let acknowledgement = protocol::packet(
                header.source,
                header.target,
                header.sequence,
                protocol::ACKNOWLEDGEMENT,
                &[],
                false,
                false,
            )
            .expect("encode acknowledgement");
            server
                .send_to(&acknowledgement, client)
                .expect("send acknowledgement");
        }
        received
    });
    (address, mock)
}

#[test]
fn visible_plain_transitions_restore_power_after_configuration() {
    for effect in [
        Effect::Static {
            colour: Colour::rgb(Rgb::new(1, 2, 3)),
        },
        Effect::Breathe {
            colour: Rgb::new(1, 2, 3),
            period_ms: 500,
        },
        Effect::Pulse {
            colour: Rgb::new(1, 2, 3),
            period_ms: 500,
        },
        Effect::Strobe {
            colour: Rgb::new(1, 2, 3),
            period_ms: 100,
        },
    ] {
        let (address, mock) = mock_acknowledgements(2);
        apply_effect(&transition_device(address, ProductTopology::Plain), &effect)
            .expect("transition should succeed");
        let packets = mock.join().expect("mock should finish");
        assert_eq!(
            packets.last().map(|packet| packet.0),
            Some(protocol::SET_POWER)
        );
        assert_eq!(packets.last().expect("power packet").1, [u8::MAX, u8::MAX]);
    }
}

#[test]
fn move_to_static_stops_firmware_effect_then_configures_and_powers_on() {
    let (address, mock) = mock_acknowledgements(3);
    apply_effect(
        &transition_device(address, ProductTopology::Linear),
        &Effect::Static {
            colour: Colour::rgb(Rgb::new(1, 2, 3)),
        },
    )
    .expect("transition should succeed");
    let packets = mock.join().expect("mock should finish");
    assert_eq!(
        packets.iter().map(|packet| packet.0).collect::<Vec<_>>(),
        [
            protocol::SET_MULTI_ZONE_EFFECT,
            protocol::SET_COLOR,
            protocol::SET_POWER
        ]
    );
    assert_eq!(packets[0].1[4], 0, "the first effect packet must be OFF");
    assert_eq!(packets[2].1, [u8::MAX, u8::MAX]);
}

#[test]
fn move_to_zone_static_and_off_stop_move_in_safe_order() {
    let (address, mock) = mock_acknowledgements(3);
    let device = transition_device(address, ProductTopology::Linear);
    apply_zone_effect(
        &device,
        2,
        &Effect::Static {
            colour: Colour::rgb(Rgb::new(1, 2, 3)),
        },
    )
    .expect("zone transition should succeed");
    let packets = mock.join().expect("mock should finish");
    assert_eq!(
        packets.iter().map(|packet| packet.0).collect::<Vec<_>>(),
        [
            protocol::SET_MULTI_ZONE_EFFECT,
            protocol::SET_COLOR_ZONES,
            protocol::SET_POWER
        ]
    );

    let (address, mock) = mock_acknowledgements(2);
    transition_to_off(&transition_device(address, ProductTopology::Linear))
        .expect("off transition should succeed");
    let packets = mock.join().expect("mock should finish");
    assert_eq!(
        packets.iter().map(|packet| packet.0).collect::<Vec<_>>(),
        [protocol::SET_MULTI_ZONE_EFFECT, protocol::SET_POWER]
    );
    assert_eq!(packets[1].1, [0, 0]);
}

#[test]
fn move_effect_stops_previous_move_and_powers_on_after_starting() {
    let (address, mock) = mock_acknowledgements(5);
    apply_effect(
        &transition_device(address, ProductTopology::Linear),
        &Effect::Scanner {
            colour: Rgb::new(1, 2, 3),
            period_ms: 500,
        },
    )
    .expect("scanner should succeed");
    let packets = mock.join().expect("mock should finish");
    assert_eq!(
        packets.iter().map(|packet| packet.0).collect::<Vec<_>>(),
        [
            protocol::SET_MULTI_ZONE_EFFECT,
            protocol::SET_COLOR_ZONES,
            protocol::SET_COLOR_ZONES,
            protocol::SET_MULTI_ZONE_EFFECT,
            protocol::SET_POWER
        ]
    );
    assert_eq!(packets[0].1[4], 0);
    assert_eq!(packets[3].1[4], 1);
    assert_eq!(packets[4].1, [u8::MAX, u8::MAX]);
}

#[test]
fn update_diagnostics_map_to_typed_error_categories() {
    assert!(matches!(
        classify_lifx_error("LIFX request timed out waiting for response".to_owned()),
        PluginError::Unavailable(_)
    ));
    assert!(matches!(
        classify_lifx_error("effect is not supported by this LIFX shape".to_owned()),
        PluginError::Unsupported(_)
    ));
    assert!(matches!(
        classify_lifx_error("brightness exceeds 8-bit range".to_owned()),
        PluginError::InvalidArgument(_)
    ));
    assert!(matches!(
        classify_lifx_error("LIFX linear device has no usable zones".to_owned()),
        PluginError::Internal(_)
    ));
    assert!(matches!(
        classify_lifx_error("connection refused".to_owned()),
        PluginError::Io(_)
    ));
}
