// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! WLED discovery, topology, update, and state-readback tests.

use super::*;
use std::cell::RefCell;
use std::io::Error as IoError;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener};
use std::str;
use std::sync::mpsc;

fn test_device() -> Device {
    Device {
        id: "wled-aabbccddeeff".to_owned(),
        endpoint: Endpoint::new(SocketAddr::from(([127, 0, 0, 1], 80))),
        name: "Desk".to_owned(),
        version: "0.15.0".to_owned(),
        mac: "aabbccddeeff".to_owned(),
        led_count: 60,
        segments: vec![
            Segment {
                id: 0,
                start: 0,
                stop: 30,
                name: "Left".to_owned(),
                on: true,
                bri: 200,
                col: vec![vec![255, 0, 0]],
                fx: 0,
                sx: 128,
                ix: 128,
                pal: 0,
            },
            Segment {
                id: 1,
                start: 30,
                stop: 60,
                name: "Right".to_owned(),
                on: true,
                bri: 200,
                col: vec![vec![255, 0, 0]],
                fx: 0,
                sx: 128,
                ix: 128,
                pal: 0,
            },
        ],
        effects: vec![
            "Solid".to_owned(),
            "Breathe".to_owned(),
            "Rainbow".to_owned(),
        ],
    }
}

#[derive(Default)]
struct RecordingTransport {
    plans: RefCell<Vec<CommandPlan>>,
}

impl Transport for RecordingTransport {
    fn execute(&self, plan: &CommandPlan) -> Result<(), ApplyError> {
        self.plans.borrow_mut().push(CommandPlan {
            endpoint: plan.endpoint.clone(),
            path: plan.path,
            body: plan.body.clone(),
        });
        Ok(())
    }
}

#[test]
fn discovered_endpoints_are_bounded_per_source_but_configured_endpoints_are_not() {
    let source = IpAddr::from([192, 0, 2, 1]);
    let mut endpoints = (0..=MAX_DISCOVERED_DEVICES_PER_SOURCE)
        .map(|offset| {
            let port = 8_000 + u16::try_from(offset).expect("test port fits");
            (Endpoint::new(SocketAddr::new(source, port)), Some(source))
        })
        .collect::<Vec<_>>();
    endpoints.push((Endpoint::new(SocketAddr::new(source, 9_000)), None));

    limit_discovered_endpoints_per_source(&mut endpoints);

    assert_eq!(endpoints.len(), MAX_DISCOVERED_DEVICES_PER_SOURCE + 1);
    assert!(endpoints.iter().any(|(_, source)| source.is_none()));
}

struct UnavailableTransport;

impl Transport for UnavailableTransport {
    fn execute(&self, _plan: &CommandPlan) -> Result<(), ApplyError> {
        Err(ApplyError::Io("controller unavailable".to_owned()))
    }
}

#[test]
fn normalizes_wled_mac_identity() {
    assert_eq!(
        normalize_mac("AA:BB:CC:DD:EE:FF").as_deref(),
        Some("aabbccddeeff")
    );
    assert_eq!(normalize_mac("bad"), None);
}

#[test]
fn plugin_configuration_is_typed_and_rejects_unknown_fields() {
    let configuration: WledConfig = serde_json::from_value(json!({
        "mdns": false,
        "endpoints": ["192.0.2.10"],
        "physical_tags": ["shape:planar"]
    }))
    .expect("deserialize WLED configuration");
    assert!(!configuration.mdns);
    assert_eq!(configuration.endpoints, ["192.0.2.10"]);
    assert_eq!(configuration.physical_tags, ["shape:planar"]);
    assert!(
        serde_json::from_value::<WledConfig>(json!({"mdsn": false})).is_err(),
        "misspelled plugin settings must not silently use defaults"
    );
}

#[test]
fn configuration_rejects_invalid_or_excessive_endpoints() {
    let invalid = WledConfig {
        endpoints: vec!["https://192.0.2.10".to_owned()],
        ..WledConfig::default()
    };
    assert!(validate_configuration(invalid).is_err());

    let excessive = WledConfig {
        endpoints: vec!["192.0.2.10".to_owned(); MAX_ENDPOINTS + 1],
        ..WledConfig::default()
    };
    assert!(validate_configuration(excessive).is_err());
}

#[test]
fn sanitizes_duplicate_and_out_of_range_segments() {
    let mut segments = test_device().segments;
    segments.push(Segment {
        id: 1,
        start: 10,
        stop: 20,
        name: "Duplicate".to_owned(),
        on: true,
        bri: 1,
        col: Vec::new(),
        fx: 0,
        sx: 0,
        ix: 0,
        pal: 0,
    });
    segments.push(Segment {
        id: 2,
        start: 59,
        stop: 61,
        name: "Past end".to_owned(),
        on: true,
        bri: 1,
        col: Vec::new(),
        fx: 0,
        sx: 0,
        ix: 0,
        pal: 0,
    });

    let valid = valid_segments(segments, 60);

    assert_eq!(
        valid.iter().map(|segment| segment.id).collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(valid_segments(valid.clone(), 0).is_empty());
}

#[test]
fn omits_generic_effect_when_controller_has_no_valid_choices() {
    let descriptors = effect_descriptors(&[
        "RSVD".to_owned(),
        "-".to_owned(),
        String::new(),
        "   ".to_owned(),
    ]);

    assert!(
        descriptors
            .iter()
            .all(|effect| effect.id.as_str() != "wled-effect")
    );
}

#[test]
fn builds_segment_topology_and_effect_choices() {
    let descriptor = device_descriptor(&test_device());
    assert_eq!(descriptor.id, "wled-aabbccddeeff");
    assert_eq!(descriptor.surfaces[0].elements.len(), 2);
    let effects = descriptor.capabilities.hardware_effects.expect("effects");
    assert!(
        effects
            .effects
            .iter()
            .any(|effect| effect.id.as_str() == "breathe")
    );
    let breathe = effects
        .effects
        .iter()
        .find(|effect| effect.id.as_str() == "breathe")
        .expect("breathe descriptor");
    assert!(matches!(
        breathe.parameters.as_slice(),
        [
            EffectParameter::Colour { .. },
            EffectParameter::Duration { .. }
        ]
    ));
    assert!(
        effects
            .effects
            .iter()
            .any(|effect| effect.id.as_str() == "wled-effect")
    );
}

#[test]
fn broad_static_update_addresses_every_segment() {
    let device = test_device();
    let payload = operation_payload(
        &device,
        Scope::Device,
        &PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: Colour::rgb(Rgb::new(1, 2, 3)),
            },
        },
    )
    .expect("static payload");
    let segments = payload["seg"].as_array().expect("segments");
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0]["id"], 0);
    assert_eq!(segments[1]["col"], json!([[1, 2, 3]]));
}

#[test]
fn executes_planned_update_through_injected_transport() {
    let device = test_device();
    let update = PluginUpdate {
        target: PluginTarget::Element {
            device: device.id.clone(),
            surface: SEGMENTS_SURFACE.to_owned(),
            element: "segment-1".to_owned(),
        },
        operation: PluginUpdateOperation::SetBrightness { value: 42 },
    };
    let transport = RecordingTransport::default();

    execute_update(&device, &update, &transport).expect("execute update");

    let plans = transport.plans.borrow();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].endpoint, device.endpoint);
    assert_eq!(plans[0].path, "/json/state");
    assert_eq!(
        serde_json::from_slice::<Value>(&plans[0].body).expect("planned JSON"),
        json!({"on": true, "seg": {"id": 1, "bri": 42, "on": true}})
    );
}

#[test]
fn planning_failure_does_not_invoke_transport() {
    let device = test_device();
    let update = PluginUpdate {
        target: PluginTarget::Device {
            device: device.id.clone(),
        },
        operation: PluginUpdateOperation::SaveCurrent,
    };
    let transport = RecordingTransport::default();

    assert!(matches!(
        execute_update(&device, &update, &transport),
        Err(ApplyError::Unsupported(_))
    ));
    assert!(transport.plans.borrow().is_empty());
}

#[test]
fn transport_failure_is_preserved() {
    let device = test_device();
    let update = PluginUpdate {
        target: PluginTarget::Device {
            device: device.id.clone(),
        },
        operation: PluginUpdateOperation::Clear,
    };

    assert!(matches!(
        execute_update(&device, &update, &UnavailableTransport),
        Err(ApplyError::Io(message)) if message == "controller unavailable"
    ));
}

#[test]
fn transport_errors_map_to_typed_categories() {
    assert!(matches!(
        classify_transport_error(http::HttpError::Io(IoError::from(
            ErrorKind::ConnectionRefused
        ))),
        ApplyError::Unavailable(_)
    ));
    assert!(matches!(
        classify_transport_error(http::HttpError::Io(IoError::from(ErrorKind::TimedOut))),
        ApplyError::Io(message) if message.contains("timed out")
    ));
    assert!(matches!(
        classify_transport_error(http::HttpError::Status {
            code: 429,
            reason: "Too Many Requests".to_owned(),
            retry_after: None,
        }),
        ApplyError::RateLimited { retry_after, .. }
            if retry_after == Duration::from_secs(1)
    ));
    assert!(matches!(
        classify_transport_error(http::HttpError::Status {
            code: 503,
            reason: "Service Unavailable".to_owned(),
            retry_after: Some(Duration::from_secs(9)),
        }),
        ApplyError::RateLimited { retry_after, .. }
            if retry_after == Duration::from_secs(9)
    ));
    assert!(matches!(
        classify_transport_error(http::HttpError::Status {
            code: 504,
            reason: "Gateway Timeout".to_owned(),
            retry_after: None,
        }),
        ApplyError::Unavailable(_)
    ));
    assert!(matches!(
        classify_transport_error(http::HttpError::Status {
            code: 400,
            reason: "Bad Request".to_owned(),
            retry_after: None,
        }),
        ApplyError::Io(_)
    ));
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
fn plans_a_full_frame_into_one_segmented_command() {
    let device = test_device();
    let mut pixels = vec![Colour::rgb(Rgb::new(0, 0, 0)); device.led_count as usize];
    pixels[0] = Colour::rgb(Rgb::new(255, 0, 0));
    pixels[30] = Colour::rgb(Rgb::new(0, 255, 0));
    let transport = RecordingTransport::default();

    execute_frame(&device, &full_frame(pixels), &transport).expect("execute frame");

    let plans = transport.plans.borrow();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].endpoint, device.endpoint);
    assert_eq!(plans[0].path, "/json/state");
    let body: Value = serde_json::from_slice(&plans[0].body).expect("planned JSON");
    assert_eq!(body["seg"][0]["id"], 0);
    assert_eq!(body["seg"][0]["i"][0], "FF0000");
    assert_eq!(body["seg"][1]["id"], 1);
    assert_eq!(body["seg"][1]["i"][0], "00FF00");
}

#[test]
fn rejects_a_frame_with_the_wrong_pixel_count() {
    let device = test_device();
    let transport = RecordingTransport::default();

    let result = execute_frame(
        &device,
        &full_frame(vec![Colour::rgb(Rgb::new(0, 0, 0)); 3]),
        &transport,
    );

    assert!(matches!(result, Err(ApplyError::Invalid(_))));
    assert!(transport.plans.borrow().is_empty());
}

#[test]
fn rejects_a_partial_frame() {
    let device = test_device();
    let envelope = FrameEnvelope {
        generation: 1,
        sequence: 0,
        payload: FramePayload::Partial(vec![(0, Colour::rgb(Rgb::new(1, 2, 3)))]),
        commit: false,
    };
    let transport = RecordingTransport::default();

    let result = execute_frame(&device, &envelope, &transport);

    assert!(matches!(result, Err(ApplyError::Unsupported(_))));
    assert!(transport.plans.borrow().is_empty());
}

#[test]
fn rejects_frame_streaming_at_a_non_device_target() {
    let device = test_device();
    let target = PluginTarget::Element {
        device: device.id.clone(),
        surface: SEGMENTS_SURFACE.to_owned(),
        element: "segment-0".to_owned(),
    };
    let envelope = full_frame(vec![
        Colour::rgb(Rgb::new(0, 0, 0));
        device.led_count as usize
    ]);

    let result = apply_frame_with_transport(&target, &envelope, &UnavailableTransport);

    assert!(matches!(result, Err(ApplyError::Unsupported(_))));
}

#[test]
fn reads_static_segment_facets() {
    let device = test_device();
    let state = State {
        on: true,
        bri: 220,
        segments: device.segments.clone(),
    };
    let target = PluginTarget::Element {
        device: device.id.clone(),
        surface: SEGMENTS_SURFACE.to_owned(),
        element: "segment-1".to_owned(),
    };
    let observations = read_target(
        &device,
        &state,
        &target,
        &[StateFacetKind::Appearance, StateFacetKind::Brightness],
    )
    .expect("read facets");
    assert_eq!(observations.len(), 2);
    assert_eq!(observations[1].value, FacetValue::Brightness(200));
}

#[test]
fn period_maps_inversely_to_wled_speed() {
    assert!(period_speed(100) > period_speed(60_000));
}

#[test]
fn endpoint_parser_accepts_ipv4_and_bracketed_ipv6() {
    assert_eq!(
        split_host_port("192.0.2.1:8080"),
        Ok(("192.0.2.1".to_owned(), 8080))
    );
    assert_eq!(split_host_port("[::1]:8080"), Ok(("::1".to_owned(), 8080)));
}

#[test]
#[ignore = "requires loopback TCP sockets, which the default sandbox denies"]
fn inspects_controller_through_json_endpoints() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind mock WLED");
    let address = listener.local_addr().expect("mock WLED address");
    let (paths_tx, paths_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let responses = [
            r#"{"name":"Desk","ver":"0.15.0","mac":"AA:BB:CC:DD:EE:FF","leds":{"count":60}}"#,
            r#"{"on":true,"bri":200,"seg":[{"id":0,"start":0,"stop":60,"on":true,"bri":200,"col":[[1,2,3]],"fx":0}]}"#,
            r#"["Solid","Breathe","RSVD"]"#,
        ];
        for response in responses {
            let (mut stream, _peer) = listener.accept().expect("accept mock request");
            let mut request = Vec::new();
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let mut chunk = [0_u8; 512];
                let length = stream.read(&mut chunk).expect("read mock request");
                assert!(length != 0, "mock request ended before its headers");
                request.extend_from_slice(
                    chunk
                        .get(..length)
                        .expect("read length fits request buffer"),
                );
            }
            let request = str::from_utf8(&request).expect("request UTF-8");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("request path")
                .to_owned();
            paths_tx.send(path).expect("record request path");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            )
            .expect("write mock response");
        }
    });

    let device = inspect(&Endpoint::new(address)).expect("inspect mock WLED");
    server.join().expect("join mock WLED");
    assert_eq!(device.id, "wled-aabbccddeeff");
    assert_eq!(device.led_count, 60);
    assert_eq!(device.segments.len(), 1);
    assert_eq!(device.effects.len(), 3);
    assert_eq!(
        paths_rx.iter().collect::<Vec<_>>(),
        ["/json/info", "/json/state", "/json/eff"]
    );
}
