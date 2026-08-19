// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Keyboard discovery, batching, persistence, and HID failure tests.

use std::cell::RefCell;
use std::rc::Rc;

use luminate_core::capability::HardwareEffectId;
use luminate_core::colour::Colour;
use luminate_core::effect::EffectArguments;

use super::*;

/// Records every feature-report write. `get_feature_report` always
/// answers with the M16 R2 US-ANSI `cc:93` identity payload (the only
/// layout with a populated key map), regardless of the preceding write,
/// mirroring how the real firmware ignores everything but the report id
/// on read.
#[derive(Clone, Default)]
struct RecordingChannel {
    writes: Rc<RefCell<Vec<Vec<u8>>>>,
}

impl HidChannel for RecordingChannel {
    fn write_feature_report(&self, payload: &[u8]) -> Result<(), String> {
        self.writes.borrow_mut().push(payload.to_vec());
        Ok(())
    }

    fn get_feature_report(&self, buffer: &mut [u8]) -> Result<usize, String> {
        let mut report = [0_u8; REPORT_LEN];
        report[0] = REPORT_ID;
        report[2] = 0x17;
        report[3] = 0x11;
        report[4] = 0x21;
        report[5] = 0x00;
        let length = report.len().min(buffer.len());
        buffer[..length].copy_from_slice(&report[..length]);
        Ok(length)
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

    fn writes(&self) -> Vec<Vec<u8>> {
        self.channel.writes.borrow().clone()
    }
}

impl HidTransport for RecordingTransport {
    fn open(
        &self,
        _vendor_id: u16,
        _product_id: u16,
        _usage: HidUsage,
    ) -> Result<Box<dyn HidChannel>, String> {
        Ok(Box::new(self.channel.clone()))
    }
}

/// Always fails to open, to exercise `read_layout_identity`'s and
/// `apply`'s error paths without a working channel.
struct UnavailableTransport;

impl HidTransport for UnavailableTransport {
    fn open(
        &self,
        _vendor_id: u16,
        _product_id: u16,
        _usage: HidUsage,
    ) -> Result<Box<dyn HidChannel>, String> {
        Err("simulated absence".to_owned())
    }
}

fn hardware_effect(id: &str, colour: Option<Rgb>) -> PluginUpdateOperation {
    PluginUpdateOperation::SetEffect {
        effect: Effect::Hardware {
            id: HardwareEffectId::new(id),
            arguments: EffectArguments {
                colours: colour.into_iter().collect(),
                ..EffectArguments::default()
            },
        },
    }
}

#[test]
fn read_layout_identity_parses_a_valid_report() {
    let transport = RecordingTransport::new();

    let identity = read_layout_identity(&transport, 0, 0).expect("fake report should parse");

    assert_eq!(
        identity.layout_id(),
        keyboard_layout::KeyboardLayoutId::M16R2UsAnsi
    );
    let writes = transport.writes();
    assert_eq!(
        writes.len(),
        1,
        "should select the identity sub-report first"
    );
    assert_eq!(writes[0][1], OP_LAYOUT_IDENTITY);
}

#[test]
fn read_layout_identity_propagates_an_open_failure() {
    let error = read_layout_identity(&UnavailableTransport, 0, 0)
        .expect_err("unavailable transport should surface its error");

    assert!(error.contains("simulated absence"));
}

#[test]
fn apply_static_effect_writes_builtin_effect_then_brightness() {
    let transport = RecordingTransport::new();
    let target = PluginTarget::Device {
        device: KEYBOARD_DEVICE_ID.to_owned(),
    };
    let operation = PluginUpdateOperation::SetEffect {
        effect: Effect::Static {
            colour: Colour::rgb(Rgb::new(10, 20, 30)),
        },
    };

    apply(&transport, 0, 0, &target, &operation).expect("set colour should apply");

    let writes = transport.writes();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0][1], OP_BUILTIN_EFFECT);
    assert_eq!(writes[1][1], OP_BRIGHTNESS_FINALIZER);
}

#[test]
fn apply_hardware_breathe_writes_the_breathe_builtin_effect() {
    let transport = RecordingTransport::new();
    let target = PluginTarget::Device {
        device: KEYBOARD_DEVICE_ID.to_owned(),
    };
    let operation = hardware_effect(AW_KEYBOARD_BREATHE_EFFECT_ID, Some(Rgb::new(1, 2, 3)));

    apply(&transport, 0, 0, &target, &operation).expect("breathe should apply");

    let writes = transport.writes();
    assert_eq!(writes[0][2], 0x02, "breathe uses builtin effect id 0x02");
}

#[test]
fn apply_hardware_breathe_requires_a_colour() {
    let transport = RecordingTransport::new();
    let target = PluginTarget::Device {
        device: KEYBOARD_DEVICE_ID.to_owned(),
    };
    let operation = hardware_effect(AW_KEYBOARD_BREATHE_EFFECT_ID, None);

    let error = apply(&transport, 0, 0, &target, &operation)
        .expect_err("breathe without a colour should be rejected");

    assert!(error.contains("requires one colour"));
}

#[test]
fn apply_hardware_spectrum_writes_the_spectrum_builtin_effect() {
    let transport = RecordingTransport::new();
    let target = PluginTarget::Device {
        device: KEYBOARD_DEVICE_ID.to_owned(),
    };
    let operation = hardware_effect(AW_KEYBOARD_SPECTRUM_EFFECT_ID, None);

    apply(&transport, 0, 0, &target, &operation).expect("spectrum should apply");

    assert_eq!(transport.writes()[0][2], 0x0e);
}

#[test]
fn apply_hardware_rainbow_writes_the_rainbow_builtin_effect() {
    let transport = RecordingTransport::new();
    let target = PluginTarget::Device {
        device: KEYBOARD_DEVICE_ID.to_owned(),
    };
    let operation = hardware_effect(AW_KEYBOARD_RAINBOW_EFFECT_ID, None);

    apply(&transport, 0, 0, &target, &operation).expect("rainbow should apply");

    assert_eq!(transport.writes()[0][2], 0x03);
}

#[test]
fn apply_hardware_sweeper_writes_the_scanner_builtin_effect() {
    let transport = RecordingTransport::new();
    let target = PluginTarget::Device {
        device: KEYBOARD_DEVICE_ID.to_owned(),
    };
    let operation = hardware_effect("aw-sweeper", Some(Rgb::new(4, 5, 6)));

    apply(&transport, 0, 0, &target, &operation).expect("sweeper should apply");

    assert_eq!(transport.writes()[0][2], 0x0a);
}

#[test]
fn apply_hardware_sweeper_requires_a_colour() {
    let transport = RecordingTransport::new();
    let target = PluginTarget::Device {
        device: KEYBOARD_DEVICE_ID.to_owned(),
    };
    let operation = hardware_effect("aw-sweeper", None);

    let error = apply(&transport, 0, 0, &target, &operation)
        .expect_err("sweeper without a colour should be rejected");

    assert!(error.contains("requires one colour"));
}

#[test]
fn apply_rejects_an_unknown_hardware_effect() {
    let transport = RecordingTransport::new();
    let target = PluginTarget::Device {
        device: KEYBOARD_DEVICE_ID.to_owned(),
    };
    let operation = hardware_effect("not-a-real-effect", None);

    let error = apply(&transport, 0, 0, &target, &operation)
        .expect_err("unknown hardware effect id should be rejected");

    assert!(error.contains("unknown keyboard hardware effect"));
}

#[test]
fn apply_rejects_typed_animated_effects_in_favour_of_hardware_effects() {
    let transport = RecordingTransport::new();
    let target = PluginTarget::Device {
        device: KEYBOARD_DEVICE_ID.to_owned(),
    };
    let operation = PluginUpdateOperation::SetEffect {
        effect: Effect::Breathe {
            colour: Rgb::new(0, 0, 0),
            period_ms: 500,
        },
    };

    let error = apply(&transport, 0, 0, &target, &operation)
        .expect_err("typed animated effects are exposed as hardware effects instead");

    assert!(error.contains("exposed as Alienware hardware effects"));
}

#[test]
fn apply_rejects_morph_as_not_yet_implemented() {
    let transport = RecordingTransport::new();
    let target = PluginTarget::Device {
        device: KEYBOARD_DEVICE_ID.to_owned(),
    };
    let operation = PluginUpdateOperation::SetEffect {
        effect: Effect::Morph {
            colours: vec![Rgb::new(0, 0, 0)],
            period_ms: 500,
        },
    };

    let error = apply(&transport, 0, 0, &target, &operation)
        .expect_err("morph is not implemented on the keyboard yet");

    assert!(error.contains("custom-animation path"));
}

#[test]
fn apply_set_brightness_writes_only_the_finalizer() {
    let transport = RecordingTransport::new();
    let target = PluginTarget::Device {
        device: KEYBOARD_DEVICE_ID.to_owned(),
    };
    let operation = PluginUpdateOperation::SetBrightness { value: 42 };

    apply(&transport, 0, 0, &target, &operation).expect("brightness should apply");

    let writes = transport.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0][1], OP_BRIGHTNESS_FINALIZER);
}

#[test]
fn ensure_whole_keyboard_target_accepts_device_surface_and_all_group() {
    ensure_whole_keyboard_target(&PluginTarget::Device {
        device: KEYBOARD_DEVICE_ID.to_owned(),
    })
    .expect("device target is whole-keyboard");
    ensure_whole_keyboard_target(&PluginTarget::Surface {
        device: KEYBOARD_DEVICE_ID.to_owned(),
        surface: "keyboard".to_owned(),
    })
    .expect("keyboard surface is whole-keyboard");
    ensure_whole_keyboard_target(&PluginTarget::Group {
        device: KEYBOARD_DEVICE_ID.to_owned(),
        group: "all".to_owned(),
    })
    .expect("the all group is whole-keyboard");
}

#[test]
fn ensure_whole_keyboard_target_rejects_other_surfaces_and_groups() {
    let wrong_surface = ensure_whole_keyboard_target(&PluginTarget::Surface {
        device: KEYBOARD_DEVICE_ID.to_owned(),
        surface: "not-keyboard".to_owned(),
    })
    .expect_err("only the keyboard surface is whole-keyboard");
    assert!(wrong_surface.contains("whole-keyboard targets only"));

    let wrong_group = ensure_whole_keyboard_target(&PluginTarget::Group {
        device: KEYBOARD_DEVICE_ID.to_owned(),
        group: "not-all".to_owned(),
    })
    .expect_err("only the all group is whole-keyboard");
    assert!(wrong_group.contains("whole-keyboard targets only"));
}

#[test]
fn resolve_static_per_key_computes_the_matrix_index_for_a_named_key() {
    let transport = RecordingTransport::new();

    let resolved = resolve_static_per_key(
        &transport,
        0,
        0,
        "keyboard",
        "escape",
        &PluginUpdateOperation::Clear,
    )
    .expect("escape is a known key on the fake identity's layout");

    assert_eq!(resolved.index, 0x01);
    assert_eq!(resolved.rgb, OFF_RGB);
    assert!(resolved.assignment_indices.contains(&0x01));
}

#[test]
fn resolve_static_per_key_rejects_an_unknown_key_name() {
    let transport = RecordingTransport::new();

    let error = resolve_static_per_key(
        &transport,
        0,
        0,
        "keyboard",
        "not-a-real-key",
        &PluginUpdateOperation::Clear,
    )
    .expect_err("the fake identity's layout has no such key");

    assert!(error.contains("unknown keyboard key"));
}

/// Panics if a HID session is ever opened. Used to prove a validation
/// failure short-circuits before touching hardware.
struct UnreachableTransport;

impl HidTransport for UnreachableTransport {
    #[allow(
        clippy::panic_in_result_fn,
        reason = "test double: the panic is the assertion that hardware was never touched"
    )]
    fn open(
        &self,
        _vendor_id: u16,
        _product_id: u16,
        _usage: HidUsage,
    ) -> Result<Box<dyn HidChannel>, String> {
        panic!("transport should not be opened for a rejected per-key target");
    }
}

#[test]
fn per_key_batch_returns_one_error_per_entry_without_touching_hardware() {
    let entries = [
        ("not-keyboard", "escape", &PluginUpdateOperation::Clear),
        (
            "keyboard",
            "escape",
            &PluginUpdateOperation::SetBrightness { value: 50 },
        ),
    ];

    let results = apply_per_key_batch(&UnreachableTransport, 0, 0, &entries);

    assert_eq!(results.len(), 2);
    assert!(
        results[0]
            .as_ref()
            .expect_err("wrong surface should be rejected")
            .contains("unknown keyboard surface target")
    );
    assert!(
        results[1]
            .as_ref()
            .expect_err("per-key brightness should be rejected")
            .contains("brightness")
    );
}

#[test]
fn staging_per_key_shadow_rejects_an_incomplete_current_shadow() {
    let mut current = BTreeMap::new();
    current.insert(0x01, Rgb::new(0xff, 0x00, 0x00));

    let error = stage_per_key_shadow(
        &current,
        &[(0x02, Rgb::new(0x00, 0xff, 0x00))],
        &[0x01, 0x02, 0x03],
    )
    .expect_err("incomplete shadow must be rejected");

    assert_eq!(current.len(), 1);
    assert!(!current.contains_key(&0x02));
    assert!(error.contains("complete keyboard frame"));
}

#[test]
fn staging_per_key_shadow_stages_a_recolour_from_complete_state() {
    let mut current = BTreeMap::new();
    current.insert(0x01, Rgb::new(0xff, 0x00, 0x00));
    current.insert(0x02, Rgb::new(0x00, 0xff, 0x00));
    current.insert(0x03, OFF_RGB);

    let staged = stage_per_key_shadow(
        &current,
        &[
            (0x01, Rgb::new(0x00, 0x00, 0xff)),
            (0x02, Rgb::new(0xff, 0xff, 0xff)),
        ],
        &[0x01, 0x02, 0x03],
    )
    .expect("complete shadow should stage");

    assert_eq!(staged.len(), 3);
    assert_eq!(staged.get(&0x01), Some(&Rgb::new(0x00, 0x00, 0xff)));
    assert_eq!(staged.get(&0x02), Some(&Rgb::new(0xff, 0xff, 0xff)));
}

#[test]
fn whole_keyboard_static_operation_invalidates_per_key_shadow() {
    assert!(operation_invalidates_per_key_shadow(
        &PluginUpdateOperation::Clear
    ));
}

#[test]
fn whole_keyboard_brightness_does_not_invalidate_per_key_shadow() {
    assert!(!operation_invalidates_per_key_shadow(
        &PluginUpdateOperation::SetBrightness { value: 50 }
    ));
}

#[test]
fn keyboard_brightness_scales_to_documented_range() {
    assert_eq!(scale_percent_to_keyboard_brightness(0), 0x00);
    assert_eq!(scale_percent_to_keyboard_brightness(100), 0xfe);
    assert_eq!(scale_percent_to_keyboard_brightness(200), 0xfe);
}

#[test]
fn per_key_target_rejects_wrong_surface() {
    let error = apply_per_key(
        &UnreachableTransport,
        0,
        0,
        "not-keyboard",
        "escape",
        &PluginUpdateOperation::Clear,
    )
    .expect_err("wrong surface should be rejected");

    assert!(error.contains("unknown keyboard surface target"));
}

#[test]
fn per_key_target_rejects_brightness() {
    let error = apply_per_key(
        &UnreachableTransport,
        0,
        0,
        "keyboard",
        "escape",
        &PluginUpdateOperation::SetBrightness { value: 50 },
    )
    .expect_err("per-key brightness should be rejected");

    assert!(error.contains("brightness"));
}

#[test]
fn per_key_target_rejects_animated_effect() {
    let error = apply_per_key(
        &UnreachableTransport,
        0,
        0,
        "keyboard",
        "escape",
        &PluginUpdateOperation::SetEffect {
            effect: Effect::Breathe {
                colour: Rgb::new(0, 0, 0),
                period_ms: 500,
            },
        },
    )
    .expect_err("per-key animated effect should be rejected");

    assert!(error.contains("animated effects"));
}

#[test]
fn per_key_target_rejects_save_current() {
    let error = apply_per_key(
        &UnreachableTransport,
        0,
        0,
        "keyboard",
        "escape",
        &PluginUpdateOperation::SaveCurrent,
    )
    .expect_err("per-key save-current should be rejected");

    assert!(error.contains("not implemented yet"));
}
