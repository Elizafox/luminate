// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Discovery caching, routing, and HID failure tests for the plugin entry points.

use std::cell::RefCell;
use std::rc::Rc;

use luminate_core::colour::Colour;
use luminate_core::frame::{FrameEnvelope, FramePayload};
use luminate_core::rgb::Rgb;
use luminate_plugin_api::{PluginApplyStatus, PluginUpdateOperation};

use super::*;

#[test]
fn metadata_lists_every_supported_hid_identity() {
    let vendor_products = VENDORS
        .iter()
        .map(|entry| (entry.vendor, entry.product))
        .collect::<Vec<_>>();

    assert_eq!(
        vendor_products,
        vec![
            (u32::from(KEYBOARD_VID), u32::from(KEYBOARD_PID)),
            (u32::from(AW_ELC_VID), u32::from(AW_ELC_LEGACY_PID)),
            (u32::from(AW_ELC_VID), u32::from(AW_ELC_M16_R2_PID)),
        ]
    );
    assert_eq!(HINTS.len(), 3);
    assert_eq!(HINTS[0].value, KEYBOARD_HINT.as_ptr().cast());
    assert_eq!(HINTS[1].value, AW_ELC_LEGACY_HINT.as_ptr().cast());
    assert_eq!(HINTS[2].value, AW_ELC_M16_R2_HINT.as_ptr().cast());
    assert!(
        HINTS
            .iter()
            .all(|hint| hint.kind == ProbeHintKind::HidVidPid)
    );
}

#[test]
fn udev_template_grants_only_the_supported_alienware_hid_ids() {
    let rules = include_str!("../packaging/udev/60-luminate-alienware.rules.in");
    let hardware_rules = rules
        .lines()
        .filter(|line| line.starts_with("SUBSYSTEM=="))
        .collect::<Vec<_>>();

    assert_eq!(hardware_rules.len(), 3);
    for (vendor, product) in [("0d62", "d2b1"), ("187c", "0550"), ("187c", "0551")] {
        assert!(hardware_rules.iter().any(|rule| {
            rule.contains(&format!("ATTRS{{idVendor}}==\"{vendor}\""))
                && rule.contains(&format!("ATTRS{{idProduct}}==\"{product}\""))
                && rule.contains("OWNER=\"@LUMINATE_USER@\"")
                && rule.contains("MODE=\"0600\"")
        }));
    }
}

/// Records every feature-report write for a HID session. Shared via `Rc`
/// so the writes made through a `Box<dyn HidChannel>` handed out by
/// `open` stay visible to the test after the call returns. Can be told to
/// fail on the write at a given index, to exercise a sequence that stops
/// partway through.
#[derive(Clone, Default)]
struct RecordingChannel {
    writes: Rc<RefCell<Vec<Vec<u8>>>>,
    fail_at: Option<usize>,
}

impl protocol::HidChannel for RecordingChannel {
    fn write_feature_report(&self, payload: &[u8]) -> Result<(), String> {
        let mut writes = self.writes.borrow_mut();
        if self.fail_at == Some(writes.len()) {
            return Err("failed to send HID feature report: simulated I/O failure".to_owned());
        }
        writes.push(payload.to_vec());
        Ok(())
    }

    fn get_feature_report(&self, _buffer: &mut [u8]) -> Result<usize, String> {
        Err("failed to read HID feature report: no simulated response".to_owned())
    }
}

struct RecordingTransport {
    channel: RecordingChannel,
}

impl RecordingTransport {
    fn new() -> Self {
        Self {
            channel: RecordingChannel::default(),
        }
    }

    fn failing_at(index: usize) -> Self {
        Self {
            channel: RecordingChannel {
                writes: Rc::default(),
                fail_at: Some(index),
            },
        }
    }

    fn writes(&self) -> Vec<Vec<u8>> {
        self.channel.writes.borrow().clone()
    }
}

impl protocol::HidTransport for RecordingTransport {
    fn open(
        &self,
        _vendor_id: u16,
        _product_id: u16,
        _usage: protocol::HidUsage,
    ) -> Result<Box<dyn protocol::HidChannel>, String> {
        Ok(Box::new(self.channel.clone()))
    }
}

struct UnavailableTransport;

impl protocol::HidTransport for UnavailableTransport {
    fn open(
        &self,
        vendor_id: u16,
        product_id: u16,
        _usage: protocol::HidUsage,
    ) -> Result<Box<dyn protocol::HidChannel>, String> {
        Err(format!(
            "failed to open HID device {vendor_id:04x}:{product_id:04x}: simulated absence"
        ))
    }
}

fn m16_r2_identity() -> aw_elc_profile::AwElcIdentity {
    aw_elc_profile::resolve(AW_ELC_VID, AW_ELC_M16_R2_PID, 0x1102, 5)
        .expect("the registered m16 R2 identity should resolve")
}

#[test]
fn keyboard_clear_writes_builtin_effect_then_brightness_finalizer() {
    let transport = RecordingTransport::new();
    let update = PluginUpdate {
        target: PluginTarget::Device {
            device: topology::KEYBOARD_DEVICE_ID.to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };

    apply_update_with_transport(&update, &transport, None).expect("clear should apply");

    let writes = transport.writes();
    assert_eq!(writes.len(), 2);
    assert_eq!(
        writes[0][1], 0x80,
        "first write is the builtin-effect report"
    );
    assert_eq!(
        writes[1][1], 0x83,
        "second write is the brightness finalizer"
    );
}

#[test]
fn keyboard_save_current_writes_documented_persist_command() {
    let transport = RecordingTransport::new();
    let update = PluginUpdate {
        target: PluginTarget::Device {
            device: topology::KEYBOARD_DEVICE_ID.to_owned(),
        },
        operation: PluginUpdateOperation::SaveCurrent,
    };

    apply_update_with_transport(&update, &transport, None).expect("save-current should apply");

    let writes = transport.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(
        &writes[0][..3],
        &[0xcc, 0x84, 0x03],
        "cc:84:03:00 persists the active keyboard effect (protocol spec §7)"
    );
}

#[test]
fn keyboard_per_key_save_current_is_still_rejected() {
    let update = PluginUpdate {
        target: PluginTarget::Element {
            device: topology::KEYBOARD_DEVICE_ID.to_owned(),
            surface: "keyboard".to_owned(),
            element: "escape".to_owned(),
        },
        operation: PluginUpdateOperation::SaveCurrent,
    };

    let error = apply_update_with_transport(&update, &UnavailableTransport, None)
        .expect_err("per-key save-current has no hardware command to persist one key");

    assert!(matches!(error, PluginError::Unsupported(_)));
}

fn keyboard_frame(value: u8) -> FrameEnvelope {
    let pixel_count = keyboard_layout::KeyboardLayoutId::M16R2UsAnsi
        .key_map()
        .expect("known layout has a key map")
        .named_positions()
        .count();
    FrameEnvelope {
        generation: 1,
        sequence: 0,
        payload: FramePayload::Full(vec![
            Colour::rgb(Rgb::new(value, value, value));
            pixel_count
        ]),
        commit: false,
    }
}

#[test]
fn keyboard_frame_uses_only_volatile_custom_colour_reports() {
    let transport = RecordingTransport::new();
    let target = PluginTarget::Surface {
        device: topology::KEYBOARD_DEVICE_ID.to_owned(),
        surface: "keyboard".to_owned(),
    };

    protocol::keyboard::apply_frame(
        &transport,
        KEYBOARD_VID,
        KEYBOARD_PID,
        keyboard_layout::KeyboardLayoutId::M16R2UsAnsi,
        &target,
        &keyboard_frame(0x40),
    )
    .expect("frame should apply");

    let writes = transport.writes();
    assert!(!writes.is_empty(), "frame should issue live HID writes");
    assert!(
        writes.iter().all(|write| write.get(1) != Some(&0x84)),
        "frame streaming must never issue the firmware persistence opcode"
    );
    assert!(
        writes
            .iter()
            .any(|write| write.get(1..3) == Some(&[0x8c, 0x02])),
        "frame should contain custom-colour records"
    );
}

#[test]
fn keyboard_frame_rejects_the_wrong_pixel_count_before_opening_hardware() {
    let target = PluginTarget::Surface {
        device: topology::KEYBOARD_DEVICE_ID.to_owned(),
        surface: "keyboard".to_owned(),
    };
    let frame = FrameEnvelope {
        generation: 1,
        sequence: 0,
        payload: FramePayload::Full(Vec::new()),
        commit: false,
    };

    let error = protocol::keyboard::apply_frame(
        &UnavailableTransport,
        KEYBOARD_VID,
        KEYBOARD_PID,
        keyboard_layout::KeyboardLayoutId::M16R2UsAnsi,
        &target,
        &frame,
    )
    .expect_err("malformed frame should be rejected");

    assert!(error.contains("expected 85 pixels"));
}

#[test]
fn aw_elc_trackpad_clear_writes_full_live_animation_sequence() {
    let transport = RecordingTransport::new();
    let update = PluginUpdate {
        target: PluginTarget::Surface {
            device: topology::AW_ELC_DEVICE_ID.to_owned(),
            surface: topology::AW_ELC_SURFACE_TRACKPAD_RING.to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };

    apply_update_with_transport(&update, &transport, Some(m16_r2_identity()))
        .expect("clear should apply");

    assert_eq!(
        transport.writes().len(),
        4,
        "start envelope, zone select, one action report, end envelope"
    );
}

#[test]
fn aw_elc_update_routes_through_the_cached_exact_product() {
    let transport = IdentityTransport::default();
    let update = PluginUpdate {
        target: PluginTarget::Surface {
            device: topology::AW_ELC_DEVICE_ID.to_owned(),
            surface: topology::AW_ELC_SURFACE_TRACKPAD_RING.to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };

    apply_update_with_transport(&update, &transport, Some(m16_r2_identity()))
        .expect("resolved m16 R2 update should route to its exact product");

    assert_eq!(
        transport.opens.into_inner(),
        vec![(AW_ELC_VID, aw_elc_profile::AW_ELC_M16_R2_PRODUCT_ID)]
    );
}

#[test]
fn aw_elc_update_without_a_cached_identity_rejects_before_hid_open() {
    let transport = IdentityTransport::default();
    let update = PluginUpdate {
        target: PluginTarget::Surface {
            device: topology::AW_ELC_DEVICE_ID.to_owned(),
            surface: topology::AW_ELC_SURFACE_TRACKPAD_RING.to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };

    let error = apply_update_with_transport(&update, &transport, None)
        .expect_err("an unavailable controller must fail closed");

    assert!(matches!(error, PluginError::Unavailable(_)));
    assert!(transport.opens.into_inner().is_empty());
}

#[test]
fn imported_profile_update_routes_through_its_exact_product() {
    let transport = IdentityTransport::default();
    let identity = aw_elc_profile::resolve(
        AW_ELC_VID,
        aw_elc_profile::AW_ELC_LEGACY_PRODUCT_ID,
        0x0c01,
        4,
    )
    .expect("the imported profile should resolve");
    let update = PluginUpdate {
        target: PluginTarget::Device {
            device: topology::AW_ELC_DEVICE_ID.to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };

    apply_update_with_transport(&update, &transport, Some(identity))
        .expect("a published imported profile should accept a live update");

    assert_eq!(
        transport.opens.into_inner(),
        vec![(AW_ELC_VID, aw_elc_profile::AW_ELC_LEGACY_PRODUCT_ID)]
    );
}

#[test]
fn open_failure_classifies_as_unavailable() {
    let update = PluginUpdate {
        target: PluginTarget::Device {
            device: topology::KEYBOARD_DEVICE_ID.to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };

    let error = apply_update_with_transport(&update, &UnavailableTransport, None)
        .expect_err("unopenable transport should reject");

    assert!(matches!(error, PluginError::Unavailable(_)));
}

#[test]
fn write_failure_partway_through_stops_after_the_failed_write() {
    let transport = RecordingTransport::failing_at(1);
    let update = PluginUpdate {
        target: PluginTarget::Device {
            device: topology::KEYBOARD_DEVICE_ID.to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };

    let error = apply_update_with_transport(&update, &transport, None)
        .expect_err("second write failure should reject the whole apply");

    assert!(matches!(error, PluginError::Io(_)));
    assert_eq!(
        transport.writes().len(),
        1,
        "only the first write should have been recorded"
    );
}

#[test]
fn apply_batch_routes_keyboard_element_targets_through_per_key_batch() {
    let updates = vec![
        PluginUpdate {
            target: PluginTarget::Element {
                device: topology::KEYBOARD_DEVICE_ID.to_owned(),
                surface: "not-keyboard".to_owned(),
                element: "escape".to_owned(),
            },
            operation: PluginUpdateOperation::Clear,
        },
        PluginUpdate {
            target: PluginTarget::Device {
                device: "unknown-device".to_owned(),
            },
            operation: PluginUpdateOperation::Clear,
        },
    ];

    let results = apply_batch_with_transport(&updates, &UnavailableTransport, None);

    assert_eq!(results.len(), 2);
    assert!(
        results[0]
            .as_ref()
            .expect_err("per-key path should reject the bad surface")
            .diagnostic()
            .contains("unknown keyboard surface target"),
        "expected the per-key batch path's error, got {:?}",
        results[0]
    );
    assert!(
        results[1]
            .as_ref()
            .expect_err("unowned device should be rejected")
            .diagnostic()
            .contains("not owned by the Alienware plugin")
    );
}

#[test]
fn apply_batch_coalesces_imported_aw_elc_live_zones() {
    let transport = RecordingTransport::new();
    let identity = aw_elc_profile::resolve(AW_ELC_VID, 0x0550, 0x0c01, 4)
        .expect("the imported G5 profile should resolve");
    let updates = ["left", "middle"].map(|surface| PluginUpdate {
        target: PluginTarget::Surface {
            device: topology::AW_ELC_DEVICE_ID.to_owned(),
            surface: surface.to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    });

    let outcomes = apply_batch_with_transport(&updates, &transport, Some(identity));

    assert!(outcomes.iter().all(Result::is_ok));
    assert_eq!(transport.writes().len(), 4);
}

#[test]
fn apply_batch_keeps_m16_live_updates_on_the_established_path() {
    let transport = RecordingTransport::new();
    let updates = [
        topology::AW_ELC_SURFACE_TRACKPAD_RING,
        topology::AW_ELC_SURFACE_REAR_LOGO,
    ]
    .map(|surface| PluginUpdate {
        target: PluginTarget::Surface {
            device: topology::AW_ELC_DEVICE_ID.to_owned(),
            surface: surface.to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    });

    let outcomes = apply_batch_with_transport(&updates, &transport, Some(m16_r2_identity()));

    assert!(outcomes.iter().all(Result::is_ok));
    assert_eq!(transport.writes().len(), 8);
}

#[test]
fn protocol_diagnostics_map_to_typed_abi_categories() {
    let cases = [
        (
            "failed to open HID device 0000:0000: absent",
            PluginApplyStatus::Unavailable,
        ),
        (
            "failed to send HID feature report: gone",
            PluginApplyStatus::Io,
        ),
        (
            "unknown keyboard surface target: bad",
            PluginApplyStatus::InvalidArgument,
        ),
        (
            "AW-ELC power-button shadow state is poisoned",
            PluginApplyStatus::Internal,
        ),
        (
            "AW-ELC firmware save-current is not implemented yet",
            PluginApplyStatus::Unsupported,
        ),
    ];
    for (diagnostic, expected) in cases {
        let result = classify_protocol_error(diagnostic.to_owned()).into_apply_result();
        assert_eq!(
            result.decode().expect("valid classified result").0,
            expected
        );
    }
}

#[test]
fn rate_limit_diagnostics_classify_as_rate_limited() {
    let error = classify_protocol_error("updated less than 100ms ago".to_owned());
    assert!(matches!(
        error,
        PluginError::RateLimited {
            retry_after,
            ..
        } if retry_after == Duration::from_secs(5)
    ));
}

#[test]
fn discovery_any_reflects_either_device() {
    assert!(!Discovery::empty().any());
    assert!(
        Discovery {
            keyboard: true,
            keyboard_layout: None,
            aw_elc: None,
        }
        .any()
    );
    assert!(
        Discovery {
            keyboard: false,
            keyboard_layout: None,
            aw_elc: Some(m16_r2_identity()),
        }
        .any()
    );
}

fn context() -> PluginRequestContext {
    PluginRequestContext::new(Duration::from_secs(1))
}

fn plugin_with(discovery: Discovery) -> Alienware {
    Alienware {
        discovery: Mutex::new(discovery),
    }
}

struct FakeEnumerator(Vec<(u16, u16)>);

impl HidDeviceEnumerator for FakeEnumerator {
    fn vendor_product_pairs(&self) -> Vec<(u16, u16)> {
        self.0.clone()
    }
}

#[derive(Clone)]
struct IdentityResponseChannel {
    response: [u8; 7],
}

impl protocol::HidChannel for IdentityResponseChannel {
    fn write_feature_report(&self, _payload: &[u8]) -> Result<(), String> {
        Ok(())
    }

    fn get_feature_report(&self, buffer: &mut [u8]) -> Result<usize, String> {
        buffer[..self.response.len()].copy_from_slice(&self.response);
        Ok(self.response.len())
    }
}

#[derive(Default)]
struct IdentityTransport {
    opens: RefCell<Vec<(u16, u16)>>,
}

impl protocol::HidTransport for IdentityTransport {
    fn open(
        &self,
        vendor_id: u16,
        product_id: u16,
        _usage: protocol::HidUsage,
    ) -> Result<Box<dyn protocol::HidChannel>, String> {
        self.opens.borrow_mut().push((vendor_id, product_id));
        let response = match product_id {
            aw_elc_profile::AW_ELC_LEGACY_PRODUCT_ID => [0x00, 0x03, 0x20, 0x02, 0x0c, 0x01, 0x04],
            aw_elc_profile::AW_ELC_M16_R2_PRODUCT_ID => [0x00, 0x03, 0x20, 0x02, 0x11, 0x02, 0x05],
            _ => return Err("unexpected controller requested".to_owned()),
        };
        Ok(Box::new(IdentityResponseChannel { response }))
    }
}

#[test]
fn discovery_resolves_and_caches_each_aw_elc_product_exactly() {
    for product_id in [
        aw_elc_profile::AW_ELC_LEGACY_PRODUCT_ID,
        aw_elc_profile::AW_ELC_M16_R2_PRODUCT_ID,
    ] {
        let transport = IdentityTransport::default();
        let discovery = discover_with(&FakeEnumerator(vec![(AW_ELC_VID, product_id)]), &transport);
        let identity = discovery.aw_elc.expect("known controller should resolve");

        assert_eq!(identity.product_id, product_id);
        assert_eq!(transport.opens.into_inner(), vec![(AW_ELC_VID, product_id)]);
    }
}

#[test]
fn multiple_aw_elc_candidates_fail_closed_in_either_order() {
    let orders = [
        vec![
            (AW_ELC_VID, aw_elc_profile::AW_ELC_LEGACY_PRODUCT_ID),
            (AW_ELC_VID, aw_elc_profile::AW_ELC_M16_R2_PRODUCT_ID),
        ],
        vec![
            (AW_ELC_VID, aw_elc_profile::AW_ELC_M16_R2_PRODUCT_ID),
            (AW_ELC_VID, aw_elc_profile::AW_ELC_LEGACY_PRODUCT_ID),
        ],
    ];

    for order in orders {
        let transport = IdentityTransport::default();
        let discovery = discover_with(&FakeEnumerator(order), &transport);

        assert!(discovery.aw_elc.is_none());
        assert!(transport.opens.into_inner().is_empty());
    }
}

#[test]
fn refresh_discovery_cache_overwrites_a_stale_hotplug_disconnect() {
    // Seed the cache as though a previous scan found both devices, then
    // rescan against an enumerator reporting nothing attached. A hotplug
    // disconnect must clear the cache, not leave the stale entries in
    // place underneath whatever the new scan finds (here, nothing).
    let cache = Mutex::new(Discovery {
        keyboard: true,
        keyboard_layout: Some(keyboard_layout::KeyboardLayoutId::M16R2UsAnsi),
        aw_elc: Some(m16_r2_identity()),
    });

    let discovery =
        refresh_discovery_cache(&cache, &FakeEnumerator(Vec::new()), &UnavailableTransport);

    assert!(
        !discovery.any(),
        "an empty rescan must report nothing found"
    );
    assert!(
        !cache.lock().expect("lock poisoned").any(),
        "the cache must be overwritten, not merged with stale entries"
    );
}

#[test]
fn refresh_discovery_cache_replaces_one_aw_elc_profile_with_another() {
    let cache = Mutex::new(Discovery {
        keyboard: false,
        keyboard_layout: None,
        aw_elc: Some(m16_r2_identity()),
    });
    let transport = IdentityTransport::default();

    let discovery = refresh_discovery_cache(
        &cache,
        &FakeEnumerator(vec![(AW_ELC_VID, aw_elc_profile::AW_ELC_LEGACY_PRODUCT_ID)]),
        &transport,
    );

    let identity = discovery
        .aw_elc
        .expect("replacement profile should resolve");
    assert_eq!(
        identity.product_id,
        aw_elc_profile::AW_ELC_LEGACY_PRODUCT_ID
    );
    assert_eq!(cache.lock().expect("lock poisoned").aw_elc, Some(identity));
}

#[test]
fn refresh_discovery_cache_picks_up_a_newly_attached_keyboard() {
    let cache = Mutex::new(Discovery::empty());

    let discovery = refresh_discovery_cache(
        &cache,
        &FakeEnumerator(vec![(KEYBOARD_VID, KEYBOARD_PID)]),
        &UnavailableTransport,
    );

    assert!(
        discovery.keyboard,
        "the newly attached keyboard should be found by vendor/product id alone"
    );
    assert_eq!(*cache.lock().expect("lock poisoned"), discovery,);
}

#[test]
fn upload_frame_rejects_a_non_surface_target() {
    let plugin = plugin_with(Discovery {
        keyboard: true,
        keyboard_layout: Some(keyboard_layout::KeyboardLayoutId::M16R2UsAnsi),
        aw_elc: None,
    });
    let target = PluginTarget::Device {
        device: topology::KEYBOARD_DEVICE_ID.to_owned(),
    };

    let error = plugin
        .upload_frame(&context(), &target, &keyboard_frame(0x10))
        .expect_err("a device target has no addressable frame surface");
    assert!(matches!(error, PluginError::InvalidTarget(_)));
}

#[test]
fn upload_frame_rejects_a_surface_on_another_device() {
    let plugin = plugin_with(Discovery {
        keyboard: true,
        keyboard_layout: Some(keyboard_layout::KeyboardLayoutId::M16R2UsAnsi),
        aw_elc: None,
    });
    let target = PluginTarget::Surface {
        device: topology::AW_ELC_DEVICE_ID.to_owned(),
        surface: "ring".to_owned(),
    };

    let error = plugin
        .upload_frame(&context(), &target, &keyboard_frame(0x10))
        .expect_err("the AW-ELC surface is not the keyboard surface");
    assert!(matches!(error, PluginError::InvalidTarget(_)));
}

#[test]
fn upload_frame_rejects_when_the_layout_was_never_identified() {
    let plugin = plugin_with(Discovery {
        keyboard: true,
        keyboard_layout: None,
        aw_elc: None,
    });
    let target = PluginTarget::Surface {
        device: topology::KEYBOARD_DEVICE_ID.to_owned(),
        surface: "keyboard".to_owned(),
    };

    let error = plugin
        .upload_frame(&context(), &target, &keyboard_frame(0x10))
        .expect_err("an unidentified layout has no known pixel order");
    assert!(matches!(error, PluginError::Unsupported(_)));
}
