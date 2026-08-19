// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Sysfs discovery, topology, update, and partial-write failure tests.

use super::*;
use luminate_core::rgb::Rgb;
use std::cell::RefCell;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = env::temp_dir().join(format!(
            "luminate-linux-leds-{}-{}",
            process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).expect("create fixture root");
        Self { root }
    }

    fn led(&self, name: &str, maximum: u32, brightness: u32) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir_all(&path).expect("create LED fixture");
        fs::write(path.join("max_brightness"), maximum.to_string()).expect("write maximum");
        fs::write(path.join("brightness"), brightness.to_string()).expect("write brightness");
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.root);
    }
}

/// Records every write instead of touching the filesystem. Can be told
/// to fail on the write at a given index, to exercise a multi-LED apply
/// that stops partway through.
#[derive(Default)]
struct RecordingTransport {
    writes: RefCell<Vec<(PathBuf, String)>>,
    fail_after: Option<usize>,
}

impl Transport for RecordingTransport {
    fn write(&self, path: &Path, contents: &str) -> Result<(), ApplyError> {
        let mut writes = self.writes.borrow_mut();
        if self.fail_after == Some(writes.len()) {
            return Err(ApplyError::Io(format!(
                "cannot write {}: simulated I/O failure",
                path.display()
            )));
        }
        writes.push((path.to_path_buf(), contents.to_owned()));
        Ok(())
    }
}

#[test]
fn write_failure_classifies_as_io_without_touching_the_filesystem() {
    let fixture = Fixture::new();
    let path = fixture.led("platform::kbd_backlight", 20, 0);
    let keyboards = keyboards(scan(&fixture.root).expect("scan fixture"));
    let device = device_id(keyboards.first().expect("one keyboard"));
    let update = PluginUpdate {
        target: PluginTarget::Device { device },
        operation: PluginUpdateOperation::SetBrightness { value: 25 },
    };
    let transport = RecordingTransport {
        fail_after: Some(0),
        ..Default::default()
    };

    let error = apply_update(&transport, &keyboards, &update)
        .expect_err("simulated write failure should reject the apply");

    assert!(matches!(error, ApplyError::Io(_)));
    assert_eq!(
        fs::read_to_string(path.join("brightness")).expect("read brightness"),
        "0",
        "real sysfs brightness should be untouched since the transport never wrote to disk"
    );
}

#[test]
fn group_target_write_failure_stops_after_the_failed_led() {
    let fixture = Fixture::new();
    let _left = fixture.led("input8:rgb:kbd_zoned_backlight-left", 255, 10);
    let _right = fixture.led("input8:rgb:kbd_zoned_backlight-right", 255, 10);
    let keyboards = keyboards(scan(&fixture.root).expect("scan fixture"));
    let device = device_id(keyboards.first().expect("one keyboard"));
    let update = PluginUpdate {
        target: PluginTarget::Group {
            device,
            group: ALL_ZONES_GROUP_ID.to_owned(),
        },
        operation: PluginUpdateOperation::SetBrightness { value: 50 },
    };
    let transport = RecordingTransport {
        fail_after: Some(1),
        ..Default::default()
    };

    let error = apply_update(&transport, &keyboards, &update)
        .expect_err("second led's write failure should reject the whole apply");

    assert!(matches!(error, ApplyError::Io(_)));
    assert_eq!(
        transport.writes.borrow().len(),
        1,
        "only the first led's write should have been recorded before the failure"
    );
}

#[test]
fn discovers_only_keyboard_backlights() {
    let fixture = Fixture::new();
    let _keyboard = fixture.led("input3::kbd_backlight", 3, 2);
    let _caps = fixture.led("input3::capslock", 1, 0);
    let leds = scan(&fixture.root).expect("scan fixture");
    assert_eq!(leds.len(), 1);
    assert_eq!(leds.first().expect("one keyboard LED").device_key, "input3");
}

#[test]
fn groups_standard_zoned_keyboard_names() {
    let fixture = Fixture::new();
    let _left = fixture.led("input8:rgb:kbd_zoned_backlight-left", 255, 10);
    let _right = fixture.led("input8:rgb:kbd_zoned_backlight-right", 255, 10);
    let keyboards = keyboards(scan(&fixture.root).expect("scan fixture"));
    assert_eq!(keyboards.len(), 1);
    let descriptor = keyboard_descriptor(keyboards.first().expect("one keyboard"));
    assert_eq!(
        descriptor
            .surfaces
            .first()
            .expect("one surface")
            .elements
            .len(),
        2
    );
    assert_eq!(
        descriptor.groups.first().expect("one group").members.len(),
        2
    );
    assert!(matches!(
        descriptor
            .groups
            .first()
            .expect("one group")
            .capabilities
            .brightness,
        BrightnessCapability::Independent {
            scope: CapabilityScope::Device,
            ..
        }
    ));
    assert!(descriptor.host_attached);
}

#[test]
fn topology_fingerprint_tracks_identity_channels_maxima_and_trigger() {
    let fixture = Fixture::new();
    let path = fixture.led("input8:rgb:kbd_zoned_backlight-left", 255, 10);
    fs::write(path.join("multi_index"), "red green blue").expect("write indices");
    fs::write(path.join("multi_max_intensity"), "255 255 255").expect("write maxima");
    fs::write(path.join("trigger"), "[none] timer").expect("write trigger");
    fs::create_dir_all(path.join("device")).expect("create device metadata");
    fs::write(path.join("device/uevent"), "UNIQ=first\n").expect("write identity");
    let original = scan_fingerprint(&fixture.root);

    fs::write(path.join("device/uevent"), "UNIQ=second\n").expect("replace identity");
    let replaced = scan_fingerprint(&fixture.root);
    assert_ne!(original, replaced);

    fs::write(path.join("multi_max_intensity"), "200 210 220").expect("change maxima");
    let changed_channels = scan_fingerprint(&fixture.root);
    assert_ne!(replaced, changed_channels);

    fs::write(path.join("trigger"), "none [timer]").expect("change trigger");
    assert_ne!(changed_channels, scan_fingerprint(&fixture.root));
}

#[test]
fn brightness_scales_and_leaves_trigger_unchanged() {
    let fixture = Fixture::new();
    let path = fixture.led("platform::kbd_backlight", 20, 0);
    fs::write(path.join("trigger"), "none [timer] heartbeat").expect("write trigger");
    let keyboards = keyboards(scan(&fixture.root).expect("scan fixture"));
    let device = device_id(keyboards.first().expect("one keyboard"));
    let update = PluginUpdate {
        target: PluginTarget::Device { device },
        operation: PluginUpdateOperation::SetBrightness { value: 25 },
    };
    apply_update(&SysfsTransport, &keyboards, &update).expect("apply brightness");
    assert_eq!(
        fs::read_to_string(path.join("brightness")).expect("read brightness"),
        "5"
    );
    assert_eq!(
        fs::read_to_string(path.join("trigger")).expect("read trigger"),
        "none [timer] heartbeat"
    );
}

#[test]
fn writes_rgb_in_kernel_index_order() {
    let fixture = Fixture::new();
    let path = fixture.led("input9:rgb:kbd_zoned_backlight-left", 255, 255);
    fs::write(path.join("multi_index"), "green blue red\n").expect("write indices");
    fs::write(path.join("multi_intensity"), "0 0 0").expect("write intensity");
    let keyboards = keyboards(scan(&fixture.root).expect("scan fixture"));
    let device = device_id(keyboards.first().expect("one keyboard"));
    let element = zone_id(
        keyboards
            .first()
            .and_then(|keyboard| keyboard.leds.first())
            .expect("one keyboard LED"),
    );
    let update = PluginUpdate {
        target: PluginTarget::Element {
            device,
            surface: SURFACE_ID.to_owned(),
            element,
        },
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: Colour::rgb(Rgb::new(30, 20, 10)),
            },
        },
    };
    apply_update(&SysfsTransport, &keyboards, &update).expect("apply colour");
    assert_eq!(
        fs::read_to_string(path.join("multi_intensity")).expect("read intensity"),
        "20 10 30"
    );
}

#[test]
fn reads_normalized_brightness() {
    let fixture = Fixture::new();
    let _path = fixture.led("platform::kbd_backlight", 20, 5);
    let keyboards = keyboards(scan(&fixture.root).expect("scan fixture"));
    let keyboard = keyboards.first().expect("one keyboard");
    let target = PluginTarget::Device {
        device: device_id(keyboard),
    };
    let observations = read_target(
        &target,
        &[StateFacetKind::Brightness, StateFacetKind::Emission],
        &[keyboard.leds.first().expect("one LED")],
    )
    .expect("read target");
    assert_eq!(
        observations.first().expect("brightness observation").value,
        FacetValue::Brightness(25)
    );
    assert_eq!(
        observations.get(1).expect("emission observation").value,
        FacetValue::Emission(EmissionState::Emitting)
    );
}

#[test]
fn apply_error_display_matches_the_wrapped_message() {
    assert_eq!(ApplyError::Invalid("bad".to_owned()).to_string(), "bad");
    assert_eq!(
        ApplyError::Unsupported("nope".to_owned()).to_string(),
        "nope"
    );
    assert_eq!(ApplyError::Io("boom".to_owned()).to_string(), "boom");
}

#[test]
fn apply_error_converts_to_the_matching_plugin_error_variant() {
    assert!(matches!(
        PluginError::from(ApplyError::Invalid("x".to_owned())),
        PluginError::InvalidArgument(_)
    ));
    assert!(matches!(
        PluginError::from(ApplyError::Unsupported("x".to_owned())),
        PluginError::Unsupported(_)
    ));
    assert!(matches!(
        PluginError::from(ApplyError::Io("x".to_owned())),
        PluginError::Io(_)
    ));
}

#[test]
fn parse_keyboard_name_rejects_names_with_no_recognized_suffix() {
    assert_eq!(parse_keyboard_name("platform::capslock"), None);
}

#[test]
fn parse_keyboard_name_rejects_an_empty_plain_device() {
    assert_eq!(parse_keyboard_name(":kbd_backlight"), None);
}

#[test]
fn parse_keyboard_name_rejects_an_empty_zone() {
    assert_eq!(parse_keyboard_name("input8:rgb:kbd_zoned_backlight-"), None);
}

#[test]
fn parse_keyboard_name_rejects_a_zoned_name_with_no_device_prefix() {
    assert_eq!(
        parse_keyboard_name(":kbd_zoned_backlight-left"),
        None,
        "there is no ':' left to split the device from the colour tag"
    );
}

#[test]
fn parse_keyboard_name_parses_the_plain_and_zoned_forms() {
    assert_eq!(
        parse_keyboard_name("platform::kbd_backlight"),
        Some(("platform".to_owned(), None))
    );
    assert_eq!(
        parse_keyboard_name("input8:rgb:kbd_zoned_backlight-left"),
        Some(("input8".to_owned(), Some("left".to_owned())))
    );
}

#[test]
fn selected_trigger_returns_none_when_nothing_is_bracketed() {
    assert_eq!(selected_trigger("none timer heartbeat"), None);
}

#[test]
fn selected_trigger_returns_the_bracketed_entry() {
    assert_eq!(
        selected_trigger("none [timer] heartbeat"),
        Some("timer".to_owned())
    );
}

fn dummy_led(trigger: Option<String>) -> Led {
    Led {
        sysfs_name: "platform::kbd_backlight".to_owned(),
        path: PathBuf::from("/nonexistent"),
        device_key: "platform".to_owned(),
        stable_identity: "platform".to_owned(),
        zone: None,
        maximum: 100,
        rgb: None,
        trigger,
    }
}

#[test]
fn trigger_notes_is_empty_without_an_active_trigger() {
    assert!(trigger_notes(&dummy_led(None)).is_empty());
}

#[test]
fn trigger_notes_is_empty_when_the_active_trigger_is_none() {
    assert!(trigger_notes(&dummy_led(Some("none".to_owned()))).is_empty());
}

#[test]
fn trigger_notes_reports_a_non_none_active_trigger() {
    let notes = trigger_notes(&dummy_led(Some("timer".to_owned())));
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("timer"));
}

#[test]
fn display_name_replaces_dashes_and_underscores_with_spaces() {
    assert_eq!(display_name("left-side_zone"), "left side zone");
}

#[test]
fn zone_name_falls_back_to_main_for_the_unzoned_led() {
    assert_eq!(zone_name(&dummy_led(None)), "Main");
}

#[test]
fn stable_component_lowercases_and_collapses_separators() {
    assert_eq!(stable_component("Left Side!!Zone"), "left-side-zone");
}

#[test]
fn stable_component_falls_back_to_unnamed_for_no_alphanumerics() {
    assert_eq!(stable_component("---"), "unnamed");
}

#[test]
fn capabilities_advertises_rgb_colour_and_appearance_readback_when_rgb() {
    let capabilities = capabilities(CapabilityScope::Device, true, true);
    assert!(!capabilities.colour.is_empty());
    let StateReadbackCapability::Readable { facets, .. } = &capabilities.state_readback else {
        panic!("readback was requested");
    };
    assert!(
        facets
            .iter()
            .any(|facet| facet.facet == StateFacetKind::Appearance)
    );
}

#[test]
fn capabilities_has_no_colour_and_no_readback_when_neither_is_requested() {
    let capabilities = capabilities(CapabilityScope::Surface, false, false);
    assert!(capabilities.colour.is_empty());
    assert!(matches!(
        capabilities.state_readback,
        StateReadbackCapability::None
    ));
}

#[test]
fn keyboard_descriptor_for_a_single_unzoned_led_has_no_elements_or_groups() {
    let fixture = Fixture::new();
    let _path = fixture.led("platform::kbd_backlight", 100, 0);
    let keyboards = keyboards(scan(&fixture.root).expect("scan fixture"));
    let descriptor = keyboard_descriptor(keyboards.first().expect("one keyboard"));

    assert!(
        descriptor
            .surfaces
            .first()
            .expect("one surface")
            .elements
            .is_empty()
    );
    assert!(descriptor.groups.is_empty());
}

#[test]
fn scan_treats_a_missing_root_as_no_leds() {
    let leds = scan(Path::new("/nonexistent/luminate-linux-leds-missing-root"))
        .expect("a missing root is not an error");
    assert!(leds.is_empty());
}

#[test]
fn scan_ignores_a_led_with_an_unparsable_max_brightness() {
    let fixture = Fixture::new();
    let path = fixture.root.join("platform::kbd_backlight");
    fs::create_dir_all(&path).expect("create LED fixture");
    fs::write(path.join("max_brightness"), "not-a-number").expect("write maximum");
    fs::write(path.join("brightness"), "0").expect("write brightness");

    let leds = scan(&fixture.root).expect("scan should skip the invalid LED, not fail");

    assert!(leds.is_empty());
}

#[test]
fn scan_reads_custom_multi_max_intensity_when_present() {
    let fixture = Fixture::new();
    let path = fixture.led("input9:rgb:kbd_zoned_backlight-left", 255, 0);
    fs::write(path.join("multi_index"), "red green blue").expect("write indices");
    fs::write(path.join("multi_max_intensity"), "200 210 220").expect("write maxima");

    let leds = scan(&fixture.root).expect("scan fixture");
    let rgb = leds
        .first()
        .expect("one led")
        .rgb
        .as_ref()
        .expect("rgb attributes parsed");

    assert_eq!(rgb.maxima, vec![200, 210, 220]);
}

#[test]
fn scan_treats_duplicate_multi_index_channels_as_no_rgb() {
    let fixture = Fixture::new();
    let path = fixture.led("input9:rgb:kbd_zoned_backlight-left", 255, 0);
    fs::write(path.join("multi_index"), "red red blue").expect("write indices");

    let leds = scan(&fixture.root).expect("scan fixture");

    assert_eq!(leds.first().expect("one led").rgb, None);
}

#[test]
fn stable_device_identity_prefers_uniq_over_product() {
    let fixture = Fixture::new();
    let path = fixture.led("platform::kbd_backlight", 100, 0);
    fs::create_dir_all(path.join("device")).expect("create device dir");
    fs::write(
        path.join("device/uevent"),
        "PRODUCT=abcd/1234/1\nUNIQ=serial-42\n",
    )
    .expect("write uevent");

    let identity = stable_device_identity(&path, "fallback");

    assert_eq!(identity, "UNIQ:serial-42");
}

#[test]
fn stable_device_identity_falls_back_to_product_then_the_key() {
    let fixture = Fixture::new();
    let path = fixture.led("platform::kbd_backlight", 100, 0);
    fs::create_dir_all(path.join("device")).expect("create device dir");
    fs::write(path.join("device/uevent"), "PRODUCT=abcd/1234/1\n").expect("write uevent");

    assert_eq!(
        stable_device_identity(&path, "fallback"),
        "PRODUCT:abcd/1234/1:fallback"
    );

    let no_uevent = fixture.led("platform2::kbd_backlight", 100, 0);
    assert_eq!(stable_device_identity(&no_uevent, "fallback"), "fallback");
}

#[test]
fn resolve_target_rejects_an_unknown_device() {
    let fixture = Fixture::new();
    let _path = fixture.led("platform::kbd_backlight", 100, 0);
    let keyboards = keyboards(scan(&fixture.root).expect("scan fixture"));

    let error = resolve_target(
        &keyboards,
        &PluginTarget::Device {
            device: "unknown-device".to_owned(),
        },
    )
    .expect_err("unknown device should be rejected");

    assert!(matches!(error, ApplyError::Invalid(_)));
}

#[test]
fn resolve_target_rejects_an_unknown_zone_element() {
    let fixture = Fixture::new();
    let _path = fixture.led("platform::kbd_backlight", 100, 0);
    let keyboards = keyboards(scan(&fixture.root).expect("scan fixture"));
    let device = device_id(keyboards.first().expect("one keyboard"));

    let error = resolve_target(
        &keyboards,
        &PluginTarget::Element {
            device,
            surface: SURFACE_ID.to_owned(),
            element: "no-such-zone".to_owned(),
        },
    )
    .expect_err("unknown zone should be rejected");

    assert!(matches!(error, ApplyError::Invalid(_)));
}

#[test]
fn resolve_target_rejects_an_unrecognized_surface() {
    let fixture = Fixture::new();
    let _path = fixture.led("platform::kbd_backlight", 100, 0);
    let keyboards = keyboards(scan(&fixture.root).expect("scan fixture"));
    let device = device_id(keyboards.first().expect("one keyboard"));

    let error = resolve_target(
        &keyboards,
        &PluginTarget::Surface {
            device,
            surface: "not-backlight".to_owned(),
        },
    )
    .expect_err("unrecognized surface should be rejected");

    assert!(matches!(error, ApplyError::Invalid(_)));
}

#[test]
fn apply_to_led_supports_static_effect_and_rejects_animated_and_save_current() {
    let led = dummy_led(None);
    let transport = RecordingTransport::default();

    let animated = apply_to_led(
        &transport,
        &led,
        &PluginUpdateOperation::SetEffect {
            effect: Effect::Breathe {
                colour: Rgb::new(1, 2, 3),
                period_ms: 100,
            },
        },
    )
    .expect_err("only static colour is supported");
    assert!(matches!(animated, ApplyError::Unsupported(_)));

    let save = apply_to_led(&transport, &led, &PluginUpdateOperation::SaveCurrent)
        .expect_err("there is no save-current operation");
    assert!(matches!(save, ApplyError::Unsupported(_)));
}

#[test]
fn apply_to_led_clear_writes_zero_brightness() {
    let led = dummy_led(None);
    let transport = RecordingTransport::default();

    apply_to_led(&transport, &led, &PluginUpdateOperation::Clear).expect("clear should apply");

    let writes = transport.writes.borrow();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].1, "0");
}

#[test]
fn write_colour_rejects_a_brightness_only_led() {
    let led = dummy_led(None);
    let transport = RecordingTransport::default();

    let error = write_colour(&transport, &led, &Colour::rgb(Rgb::new(1, 2, 3)))
        .expect_err("brightness-only LEDs have no RGB attributes");

    assert!(matches!(error, ApplyError::Unsupported(_)));
}

#[test]
fn write_colour_rejects_an_out_of_range_channel() {
    let mut led = dummy_led(None);
    led.rgb = Some(RgbAttributes {
        indices: vec!["red".to_owned(), "green".to_owned(), "blue".to_owned()],
        maxima: vec![255, 255, 255],
    });
    let transport = RecordingTransport::default();
    let colour = Colour::additive(vec![
        ColourChannelValue::new(ColourChannel::Red, 1),
        ColourChannelValue::new(ColourChannel::Green, 2),
        ColourChannelValue::new(ColourChannel::Blue, 300),
    ])
    .expect("test colour is structurally valid");

    let error = write_colour(&transport, &led, &colour).expect_err("300 exceeds an 8-bit channel");

    assert!(matches!(error, ApplyError::Invalid(_)));
}

#[test]
fn write_colour_rejects_a_missing_channel() {
    let mut led = dummy_led(None);
    led.rgb = Some(RgbAttributes {
        indices: vec!["red".to_owned(), "green".to_owned(), "blue".to_owned()],
        maxima: vec![255, 255, 255],
    });
    let transport = RecordingTransport::default();
    let colour = Colour::additive(vec![
        ColourChannelValue::new(ColourChannel::Red, 1),
        ColourChannelValue::new(ColourChannel::Green, 2),
    ])
    .expect("test colour is structurally valid");

    let error = write_colour(&transport, &led, &colour).expect_err("blue is missing");

    assert!(matches!(error, ApplyError::Invalid(_)));
}

#[test]
fn read_target_rejects_physical_power_and_effective_appearance() {
    let fixture = Fixture::new();
    let _path = fixture.led("platform::kbd_backlight", 100, 10);
    let keyboards = keyboards(scan(&fixture.root).expect("scan fixture"));
    let keyboard = keyboards.first().expect("one keyboard");
    let target = PluginTarget::Device {
        device: device_id(keyboard),
    };
    let leds: Vec<&Led> = keyboard.leds.iter().collect();

    let power_error = read_target(&target, &[StateFacetKind::PhysicalPower], &leds)
        .expect_err("LED class has no physical-power facet");
    assert!(matches!(power_error, ApplyError::Unsupported(_)));

    let effective_error = read_target(&target, &[StateFacetKind::EffectiveAppearance], &leds)
        .expect_err("effective appearance cannot be requested from a plugin");
    assert!(matches!(effective_error, ApplyError::Unsupported(_)));
}
