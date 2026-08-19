// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! AW-ELC packet sequencing, validation, and persistence tests.

use std::cell::RefCell;
use std::rc::Rc;

use luminate_core::capability::HardwareEffectId;
use luminate_core::effect::EffectArguments;

use super::*;
use crate::aw_elc_profile::{AW_ELC_VENDOR_ID, M16_R2, PROFILES};
use crate::topology::{AW_ELC_DEVICE_ID, AW_ELC_SURFACE_REAR_LOGO, AW_ELC_SURFACE_TRACKPAD_RING};

fn m16_r2_identity() -> AwElcIdentity {
    resolve(AW_ELC_VENDOR_ID, 0x0551, 0x1102, 5)
        .expect("the registered m16 R2 identity should resolve")
}

fn imported_identity() -> AwElcIdentity {
    resolve(AW_ELC_VENDOR_ID, 0x0550, 0x0c01, 4).expect("the registered G5 identity should resolve")
}

#[test]
fn configuration_parser_resolves_every_supported_profile() {
    for profile in PROFILES.iter().chain([&M16_R2]) {
        let platform_id = profile.platform_id.expect("registered profile has an ID");
        let [platform_high, platform_low] = platform_id.to_be_bytes();
        let report = [
            0x00,
            0x03,
            0x20,
            0x02,
            platform_high,
            platform_low,
            profile.expected_raw_zone_count,
        ];

        let identity = parse_configuration_report(AW_ELC_VENDOR_ID, profile.product_id, &report)
            .expect("known configuration should resolve");
        assert_eq!(identity.profile, profile);
    }
}

#[test]
fn configuration_parser_rejects_short_and_wrong_marker_reports() {
    assert_eq!(
        parse_configuration_report(0x187c, 0x0551, &[0x00, 0x03, 0x20]),
        Err(ConfigurationQueryError::ShortReport { length: 3 })
    );
    assert!(matches!(
        parse_configuration_report(0x187c, 0x0551, &[0x00, 0x03, 0x21, 0x02, 0x11, 0x02, 0x05]),
        Err(ConfigurationQueryError::WrongMarkers { .. })
    ));
}

struct ConfigurationChannel {
    writes: Rc<RefCell<Vec<Vec<u8>>>>,
    response: [u8; 7],
}

impl HidChannel for ConfigurationChannel {
    fn write_feature_report(&self, payload: &[u8]) -> Result<(), String> {
        self.writes.borrow_mut().push(payload.to_vec());
        Ok(())
    }

    fn get_feature_report(&self, buffer: &mut [u8]) -> Result<usize, String> {
        buffer[..self.response.len()].copy_from_slice(&self.response);
        Ok(self.response.len())
    }
}

struct ConfigurationTransport {
    writes: Rc<RefCell<Vec<Vec<u8>>>>,
}

#[derive(Default)]
struct LiveTransport {
    writes: Rc<RefCell<Vec<Vec<u8>>>>,
    opens: Rc<RefCell<Vec<(u16, u16)>>>,
}

impl HidTransport for LiveTransport {
    fn open(
        &self,
        vendor_id: u16,
        product_id: u16,
        _usage: HidUsage,
    ) -> Result<Box<dyn HidChannel>, String> {
        self.opens.borrow_mut().push((vendor_id, product_id));
        Ok(Box::new(ConfigurationChannel {
            writes: Rc::clone(&self.writes),
            response: [0; 7],
        }))
    }
}

impl HidTransport for ConfigurationTransport {
    #[allow(
        clippy::panic_in_result_fn,
        reason = "test double assertions verify the exact controller and collection opened"
    )]
    fn open(
        &self,
        vendor_id: u16,
        product_id: u16,
        usage: HidUsage,
    ) -> Result<Box<dyn HidChannel>, String> {
        assert_eq!((vendor_id, product_id), (0x187c, 0x0551));
        assert_eq!(usage, VENDOR_USAGE);
        Ok(Box::new(ConfigurationChannel {
            writes: Rc::clone(&self.writes),
            response: [0x00, 0x03, 0x20, 0x02, 0x11, 0x02, 0x05],
        }))
    }
}

#[test]
fn configuration_query_selects_subreport_before_reading() {
    let writes = Rc::new(RefCell::new(Vec::new()));
    let identity = read_identity(
        &ConfigurationTransport {
            writes: Rc::clone(&writes),
        },
        0x187c,
        0x0551,
    )
    .expect("captured m16 R2 response should resolve");

    assert_eq!(identity.platform_id, 0x1102);
    assert_eq!(identity.reported_zone_count, 5);
    assert_eq!(&writes.borrow()[0][..4], &[0x00, 0x03, 0x20, 0x02]);
}

#[test]
fn imported_live_batch_coalesces_equal_operations_across_zones() {
    let transport = LiveTransport::default();
    let left = PluginTarget::Surface {
        device: AW_ELC_DEVICE_ID.to_owned(),
        surface: "left".to_owned(),
    };
    let middle = PluginTarget::Surface {
        device: AW_ELC_DEVICE_ID.to_owned(),
        surface: "middle".to_owned(),
    };
    let clear = PluginUpdateOperation::Clear;
    let outcomes = apply_live_batch(
        &transport,
        imported_identity(),
        &[(&left, &clear), (&middle, &clear)],
    )
    .expect("imported profiles use live batching");

    assert!(outcomes.iter().all(Result::is_ok));
    assert_eq!(transport.opens.borrow().len(), 1);
    let writes = transport.writes.borrow();
    assert_eq!(writes.len(), 4);
    assert_eq!(
        &writes[1][..8],
        &[0x00, 0x03, 0x23, 0x01, 0x00, 0x02, 0x00, 0x01]
    );
}

#[test]
fn imported_live_batch_preserves_distinct_operation_transactions() {
    let transport = LiveTransport::default();
    let left = PluginTarget::Surface {
        device: AW_ELC_DEVICE_ID.to_owned(),
        surface: "left".to_owned(),
    };
    let middle = PluginTarget::Surface {
        device: AW_ELC_DEVICE_ID.to_owned(),
        surface: "middle".to_owned(),
    };
    let red = PluginUpdateOperation::SetEffect {
        effect: Effect::Static {
            colour: Colour::rgb(Rgb::new(0xff, 0, 0)),
        },
    };
    let blue = PluginUpdateOperation::SetEffect {
        effect: Effect::Static {
            colour: Colour::rgb(Rgb::new(0, 0, 0xff)),
        },
    };
    let outcomes = apply_live_batch(
        &transport,
        imported_identity(),
        &[(&left, &red), (&middle, &blue)],
    )
    .expect("imported profiles use live batching");

    assert!(outcomes.iter().all(Result::is_ok));
    assert_eq!(transport.opens.borrow().len(), 2);
    assert_eq!(transport.writes.borrow().len(), 8);
}

#[test]
fn imported_morph_packet_selects_the_profile_zone_id() {
    let transport = LiveTransport::default();
    let numpad = PluginTarget::Surface {
        device: AW_ELC_DEVICE_ID.to_owned(),
        surface: "numpad".to_owned(),
    };
    let operation = PluginUpdateOperation::SetEffect {
        effect: Effect::Morph {
            colours: vec![Rgb::new(0xff, 0, 0), Rgb::new(0, 0, 0xff)],
            period_ms: 500,
        },
    };

    apply(&transport, imported_identity(), &numpad, &operation)
        .expect("imported morph should apply");

    let writes = transport.writes.borrow();
    assert_eq!(writes.len(), 4);
    assert_eq!(&writes[1][..7], &[0x00, 0x03, 0x23, 0x01, 0x00, 0x01, 0x03]);
}

#[test]
fn imported_hardware_effect_packet_selects_the_profile_zone_id() {
    let transport = LiveTransport::default();
    let right = PluginTarget::Surface {
        device: AW_ELC_DEVICE_ID.to_owned(),
        surface: "right".to_owned(),
    };
    let operation = PluginUpdateOperation::SetEffect {
        effect: Effect::Hardware {
            id: HardwareEffectId::new(AW_ELC_SPECTRUM_EFFECT_ID),
            arguments: EffectArguments::default(),
        },
    };

    apply(&transport, imported_identity(), &right, &operation)
        .expect("imported hardware effect should apply");

    let writes = transport.writes.borrow();
    assert_eq!(writes.len(), 6);
    assert_eq!(&writes[1][..7], &[0x00, 0x03, 0x23, 0x01, 0x00, 0x01, 0x02]);
}

#[test]
fn imported_profile_rejects_persistence_without_opening_the_controller() {
    let transport = LiveTransport::default();
    let left = PluginTarget::Surface {
        device: AW_ELC_DEVICE_ID.to_owned(),
        surface: "left".to_owned(),
    };

    let error = apply(
        &transport,
        imported_identity(),
        &left,
        &PluginUpdateOperation::SaveCurrent,
    )
    .expect_err("imported profiles expose volatile state only");

    assert!(error.contains("volatile state only"));
    assert!(transport.opens.borrow().is_empty());
    assert!(transport.writes.borrow().is_empty());
}

#[test]
fn m16_power_profile_never_enters_live_batching() {
    assert!(apply_live_batch(&LiveTransport::default(), m16_r2_identity(), &[]).is_none());
}

#[test]
fn spectrum_uses_documented_seven_keyframes() {
    let records = spectrum_records();
    assert_eq!(records.len(), 7);
    assert_eq!(records[0].rgb, Rgb::new(0xff, 0x00, 0x00));
    assert_eq!(records[6].rgb, Rgb::new(0x80, 0x00, 0x80));
}

#[test]
fn rainbow_uses_documented_fast_keyframe_duration() {
    let records = rainbow_records();
    assert_eq!(records.len(), 7);
    assert!(records.iter().all(|record| record.duration_ms == 428));
}

#[test]
fn live_zone_group_excludes_power_button() {
    let zones = zones_for_target(
        &M16_R2,
        &PluginTarget::Group {
            device: AW_ELC_DEVICE_ID.to_owned(),
            group: "live-zones".to_owned(),
        },
    )
    .expect("live zones should resolve");

    assert_eq!(zones, vec![Zone::TrackpadRing, Zone::RearLogo]);
}

#[test]
fn trackpad_ring_target_selects_only_trackpad_ring() {
    let zones = zones_for_target(
        &M16_R2,
        &PluginTarget::Surface {
            device: AW_ELC_DEVICE_ID.to_owned(),
            surface: AW_ELC_SURFACE_TRACKPAD_RING.to_owned(),
        },
    )
    .expect("trackpad ring should resolve");

    assert_eq!(zones, vec![Zone::TrackpadRing]);
}

#[test]
fn rear_logo_target_selects_only_rear_logo() {
    let zones = zones_for_target(
        &M16_R2,
        &PluginTarget::Surface {
            device: AW_ELC_DEVICE_ID.to_owned(),
            surface: AW_ELC_SURFACE_REAR_LOGO.to_owned(),
        },
    )
    .expect("rear logo should resolve");

    assert_eq!(zones, vec![Zone::RearLogo]);
}

#[test]
fn imported_profile_targets_resolve_declared_firmware_zone_ids() {
    for profile in PROFILES {
        let zones = zones_for_target(
            profile,
            &PluginTarget::Group {
                device: AW_ELC_DEVICE_ID.to_owned(),
                group: "all".to_owned(),
            },
        )
        .expect("imported all group should resolve");
        let expected = profile
            .zones
            .iter()
            .map(|zone| Zone::Live(zone.firmware_id))
            .collect::<Vec<_>>();

        assert_eq!(zones, expected, "{}", profile.model);
    }
}

#[test]
fn target_from_another_profile_is_rejected() {
    let four_zone_profile = PROFILES
        .iter()
        .find(|profile| profile.platform_id == Some(0x0c01))
        .expect("four-zone profile should be registered");

    let error = zones_for_target(
        four_zone_profile,
        &PluginTarget::Surface {
            device: AW_ELC_DEVICE_ID.to_owned(),
            surface: "light-bar-1".to_owned(),
        },
    )
    .expect_err("a surface from the G7 profile must not resolve on a G5");

    assert!(error.contains("unknown AW-ELC surface"));
}

#[test]
fn imported_opaque_surface_rejects_element_target() {
    let profile = &PROFILES[0];
    let error = zones_for_target(
        profile,
        &PluginTarget::Element {
            device: AW_ELC_DEVICE_ID.to_owned(),
            surface: "left".to_owned(),
            element: "led".to_owned(),
        },
    )
    .expect_err("imported opaque surfaces have no addressable elements");

    assert!(error.contains("unknown AW-ELC element"));
}

/// Panics if opened. `apply_save_current`'s power-button checks must
/// reject before ever touching the transport.
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
        panic!("transport should not be opened for a rejected save-current target");
    }
}

#[test]
fn save_current_on_power_button_alone_is_rejected_as_unneeded() {
    let error = apply_save_current(
        &UnreachableTransport,
        m16_r2_identity(),
        &[Zone::PowerButton(PowerButtonZone::Ac)],
    )
    .expect_err("power button never needs an explicit save");

    assert!(error.contains("already persistent"));

    let error = apply_save_current(
        &UnreachableTransport,
        m16_r2_identity(),
        &[Zone::PowerButton(PowerButtonZone::Battery)],
    )
    .expect_err("power button never needs an explicit save");

    assert!(error.contains("already persistent"));
}

#[test]
fn save_current_mixing_live_zones_and_power_button_is_rejected() {
    let error = apply_save_current(
        &UnreachableTransport,
        m16_r2_identity(),
        &[Zone::TrackpadRing, Zone::PowerButton(PowerButtonZone::Ac)],
    )
    .expect_err("mixed live/power targets should reject");

    assert!(error.contains("mixes transient live zones"));

    let error = apply_save_current(
        &UnreachableTransport,
        m16_r2_identity(),
        &[
            Zone::TrackpadRing,
            Zone::PowerButton(PowerButtonZone::Battery),
        ],
    )
    .expect_err("mixed live/power targets should reject");

    assert!(error.contains("mixes transient live zones"));
}

#[test]
fn device_spectrum_is_rejected_because_power_button_is_not_live_zone() {
    let target = PluginTarget::Device {
        device: AW_ELC_DEVICE_ID.to_owned(),
    };
    let zones = zones_for_target(&M16_R2, &target).expect("device target should resolve");
    let operation = PluginUpdateOperation::SetEffect {
        effect: Effect::Spectrum { period_ms: 1500 },
    };

    let error = ensure_power_button_operation_supported(&zones, &operation)
        .expect_err("device spectrum should reject");

    assert!(error.contains("target group:live-zones"));
}

#[test]
fn device_static_is_rejected_because_it_would_mix_live_and_persistent_paths() {
    let target = PluginTarget::Device {
        device: AW_ELC_DEVICE_ID.to_owned(),
    };
    let zones = zones_for_target(&M16_R2, &target).expect("device target should resolve");
    let operation = PluginUpdateOperation::SetEffect {
        effect: Effect::Static {
            colour: Colour::rgb(Rgb::new(1, 2, 3)),
        },
    };

    let error = ensure_power_button_operation_supported(&zones, &operation)
        .expect_err("device static should reject");

    assert!(error.contains("mixes transient live zones"));
}

#[test]
fn ordinary_power_button_static_is_rejected() {
    let zones = zones_for_target(
        &M16_R2,
        &PluginTarget::Surface {
            device: AW_ELC_DEVICE_ID.to_owned(),
            surface: AW_ELC_SURFACE_POWER_BUTTON.to_owned(),
        },
    )
    .expect_err("ordinary power-button appearance is not addressable");

    assert!(zones.contains("set-appearance-slots"));
}

#[test]
fn power_button_write_plan_skips_identical_shadow_state() {
    let now = Instant::now();
    let shadow = PowerButtonShadow {
        rgb_ac: Some(Rgb::new(1, 2, 3)),
        rgb_battery: Some(Rgb::new(1, 2, 3)),
        last_write_ac: Some(now),
        last_write_battery: Some(now),
        last_write_charging: Some(now),
    };

    // AC

    let plan =
        plan_power_button_write_with_shadow(PowerButtonZone::Ac, &shadow, Rgb::new(1, 2, 3), now)
            .expect("identical colour should not rate-limit");

    assert_eq!(plan, PowerButtonWritePlan::SkipAlreadyCurrent);

    // Battery

    let plan = plan_power_button_write_with_shadow(
        PowerButtonZone::Battery,
        &shadow,
        Rgb::new(1, 2, 3),
        now,
    )
    .expect("identical colour should not rate-limit");

    assert_eq!(plan, PowerButtonWritePlan::SkipAlreadyCurrent);
}

#[test]
fn power_button_write_plan_rejects_rapid_distinct_rewrite() {
    let now = Instant::now();
    let shadow = PowerButtonShadow {
        rgb_ac: Some(Rgb::new(1, 2, 3)),
        rgb_battery: Some(Rgb::new(1, 2, 3)),
        last_write_ac: Some(now),
        last_write_battery: Some(now),
        last_write_charging: Some(now),
    };

    // AC

    let error =
        plan_power_button_write_with_shadow(PowerButtonZone::Ac, &shadow, Rgb::new(4, 5, 6), now)
            .expect_err("distinct colour inside rate window should fail");

    assert!(error.contains("refusing another non-volatile rewrite"));

    // Battery

    let error = plan_power_button_write_with_shadow(
        PowerButtonZone::Battery,
        &shadow,
        Rgb::new(4, 5, 6),
        now,
    )
    .expect_err("distinct colour inside rate window should fail");

    assert!(error.contains("refusing another non-volatile rewrite"));
}

#[test]
fn power_button_write_plan_rate_limits_the_shared_charging_slot() {
    let now = Instant::now();
    let shadow = PowerButtonShadow {
        rgb_ac: Some(Rgb::new(1, 2, 3)),
        rgb_battery: Some(Rgb::new(4, 5, 6)),
        last_write_ac: now.checked_sub(POWER_WRITE_MIN_INTERVAL),
        last_write_battery: now.checked_sub(POWER_WRITE_MIN_INTERVAL),
        last_write_charging: Some(now),
    };

    let error = plan_power_button_write_with_shadow(
        PowerButtonZone::Battery,
        &shadow,
        Rgb::new(7, 8, 9),
        now,
    )
    .expect_err("the shared charging slot must not be rewritten immediately");

    assert!(error.contains("refusing another non-volatile rewrite"));
}

#[test]
fn power_button_write_plan_allows_distinct_rewrite_after_rate_window() {
    // AC
    let now = Instant::now();
    let mut shadow = PowerButtonShadow {
        rgb_ac: Some(Rgb::new(1, 2, 3)),
        rgb_battery: Some(Rgb::new(1, 2, 3)),
        last_write_ac: now.checked_sub(POWER_WRITE_MIN_INTERVAL),
        last_write_battery: now.checked_sub(POWER_WRITE_MIN_INTERVAL),
        last_write_charging: now.checked_sub(POWER_WRITE_MIN_INTERVAL),
    };

    let plan =
        plan_power_button_write_with_shadow(PowerButtonZone::Ac, &shadow, Rgb::new(4, 5, 6), now)
            .expect("rate window elapsed");

    assert!(matches!(plan, PowerButtonWritePlan::Write(_)));

    // Battery
    let now = Instant::now();
    shadow.last_write_battery = now.checked_sub(POWER_WRITE_MIN_INTERVAL);

    let plan = plan_power_button_write_with_shadow(
        PowerButtonZone::Battery,
        &shadow,
        Rgb::new(4, 5, 6),
        now,
    )
    .expect("rate window elapsed");

    assert!(matches!(plan, PowerButtonWritePlan::Write(_)));
}

#[test]
fn power_button_write_plan_requires_both_charging_endpoints() {
    let now = Instant::now();
    let shadow = PowerButtonShadow::default();

    let error =
        plan_power_button_write_with_shadow(PowerButtonZone::Ac, &shadow, Rgb::new(1, 2, 3), now)
            .expect_err("an unknown battery endpoint must not be guessed");
    assert!(error.contains("battery power-button colour is unknown"));

    let error = plan_power_button_write_with_shadow(
        PowerButtonZone::Battery,
        &shadow,
        Rgb::new(4, 5, 6),
        now,
    )
    .expect_err("an unknown AC endpoint must not be guessed");
    assert!(error.contains("AC power-button colour is unknown"));
}

#[test]
fn paired_power_button_write_resolves_both_endpoints() {
    let plan = plan_power_button_updates_with_shadow(
        &PowerButtonShadow::default(),
        Some(Rgb::new(1, 2, 3)),
        Some(Rgb::new(4, 5, 6)),
        Instant::now(),
    )
    .expect("paired colours should resolve the charging state");

    let PowerButtonWritePlan::Write(write) = plan else {
        panic!("new paired colours should require a write");
    };
    assert!(write.ac);
    assert!(write.battery);
    assert_eq!(write.colours.ac, Rgb::new(1, 2, 3));
    assert_eq!(write.colours.battery, Rgb::new(4, 5, 6));
}

#[test]
fn grouped_slot_values_resolve_a_cold_shadow_complete_update() {
    let values = [
        AppearanceSlotValue {
            slot: AppearanceSlotId::new(AW_ELC_SLOT_AC),
            effect: Effect::Static {
                colour: Colour::rgb(Rgb::new(1, 2, 3)),
            },
        },
        AppearanceSlotValue {
            slot: AppearanceSlotId::new(AW_ELC_SLOT_BATTERY),
            effect: Effect::Static {
                colour: Colour::rgb(Rgb::new(4, 5, 6)),
            },
        },
    ];
    let (ac, battery) = power_button_colours_from_slots(&values).expect("valid slots");

    let plan = plan_power_button_updates_with_shadow(
        &PowerButtonShadow::default(),
        ac,
        battery,
        Instant::now(),
    )
    .expect("complete update is safe with a cold shadow");
    assert!(matches!(plan, PowerButtonWritePlan::Write(_)));
}

#[test]
fn grouped_slot_values_support_a_warm_shadow_partial_update() {
    let values = [AppearanceSlotValue {
        slot: AppearanceSlotId::new(AW_ELC_SLOT_AC),
        effect: Effect::Static {
            colour: Colour::rgb(Rgb::new(7, 8, 9)),
        },
    }];
    let (ac, battery) = power_button_colours_from_slots(&values).expect("valid slot");
    let shadow = PowerButtonShadow {
        rgb_ac: Some(Rgb::new(1, 2, 3)),
        rgb_battery: Some(Rgb::new(4, 5, 6)),
        ..PowerButtonShadow::default()
    };

    let PowerButtonWritePlan::Write(write) =
        plan_power_button_updates_with_shadow(&shadow, ac, battery, Instant::now())
            .expect("known battery colour completes a partial update")
    else {
        panic!("changed AC slot should require a write");
    };
    assert_eq!(write.colours.battery, Rgb::new(4, 5, 6));
    assert!(write.ac);
    assert!(!write.battery);
}

#[test]
fn live_start_envelope_matches_documented_bytes() {
    let report = animation_envelope_report(OP_USER_ANIMATION, 0x0001, LIVE_ANIMATION_ID, 0x0000);

    assert_eq!(
        &report[..9],
        &[0x00, 0x03, 0x21, 0x00, 0x01, 0xff, 0xff, 0x00, 0x00]
    );
}

#[test]
fn select_trackpad_ring_matches_documented_bytes() {
    let report = select_zones_report(&[Zone::TrackpadRing]).expect("select one zone");

    assert_eq!(&report[..7], &[0x00, 0x03, 0x23, 0x01, 0x00, 0x01, 0x00]);
}

#[test]
fn select_rear_logo_selects_only_rear_logo() {
    let report = select_zones_report(&[Zone::RearLogo]).expect("select one zone");

    assert_eq!(&report[..7], &[0x00, 0x03, 0x23, 0x01, 0x00, 0x01, 0x02]);
}

#[test]
fn select_live_zones_selects_ring_then_rear_logo() {
    let report =
        select_zones_report(&[Zone::TrackpadRing, Zone::RearLogo]).expect("select live zones");

    assert_eq!(
        &report[..8],
        &[0x00, 0x03, 0x23, 0x01, 0x00, 0x02, 0x00, 0x02]
    );
}

#[test]
fn trackpad_spectrum_live_reports_select_only_trackpad_ring() {
    let reports =
        live_animation_reports(&[Zone::TrackpadRing], &spectrum_records()).expect("reports");

    assert_eq!(
        &reports[1][..7],
        &[0x00, 0x03, 0x23, 0x01, 0x00, 0x01, 0x00]
    );
}

#[test]
fn rear_logo_rainbow_live_reports_select_only_rear_logo() {
    let reports = live_animation_reports(&[Zone::RearLogo], &rainbow_records()).expect("reports");

    assert_eq!(
        &reports[1][..7],
        &[0x00, 0x03, 0x23, 0x01, 0x00, 0x01, 0x02]
    );
}

#[test]
fn static_action_record_matches_documented_bytes() {
    let report =
        action_records_report(&[static_record(Rgb::new(0xff, 0x00, 0x00))]).expect("static action");

    assert_eq!(
        &report[..11],
        &[
            0x00, 0x03, 0x24, 0x00, 0x07, 0xd0, 0x00, 0xfa, 0xff, 0x00, 0x00
        ]
    );
}

#[test]
fn morph_at_the_keyframe_cap_is_accepted() {
    let colours = vec![Rgb::new(0x10, 0x20, 0x30); MAX_MORPH_KEYFRAMES];
    let operation = PluginUpdateOperation::SetEffect {
        effect: Effect::Morph {
            colours,
            period_ms: 500,
        },
    };

    let records = records_for_operation(&operation).expect("cap-sized morph should resolve");
    assert_eq!(records.len(), MAX_MORPH_KEYFRAMES);
    assert!(records.iter().all(|record| record.mode == 0x02));
}

#[test]
fn morph_over_the_keyframe_cap_is_rejected() {
    let colours = vec![Rgb::new(0x10, 0x20, 0x30); MAX_MORPH_KEYFRAMES + 1];
    let operation = PluginUpdateOperation::SetEffect {
        effect: Effect::Morph {
            colours,
            period_ms: 500,
        },
    };

    let error = records_for_operation(&operation).expect_err("oversized morph should reject");
    assert!(error.contains("at most"));
    assert!(error.contains(&MAX_MORPH_KEYFRAMES.to_string()));
}

#[test]
fn empty_morph_is_rejected() {
    let operation = PluginUpdateOperation::SetEffect {
        effect: Effect::Morph {
            colours: Vec::new(),
            period_ms: 500,
        },
    };

    let error = records_for_operation(&operation).expect_err("empty morph should reject");
    assert!(error.contains("at least one colour"));
}

#[test]
fn pulse_action_record_matches_documented_shape() {
    let report = action_records_report(&[pulse_record(Rgb::new(0x00, 0xff, 0x00), 0x0064)])
        .expect("pulse action");

    assert_eq!(
        &report[..11],
        &[
            0x00, 0x03, 0x24, 0x01, 0x07, 0xd0, 0x00, 0x64, 0x00, 0xff, 0x00
        ]
    );
}

#[test]
fn resolve_zone_records_reports_the_missing_zone_by_id() {
    let shadow = BTreeMap::new();

    let error = resolve_zone_records(&shadow, m16_r2_identity(), &[Zone::RearLogo])
        .expect_err("empty shadow has nothing to save");

    assert!(
        error.contains("0x02"),
        "diagnostic should name the zone id: {error}"
    );
}

#[test]
fn resolve_zone_records_returns_each_zones_last_applied_records_in_order() {
    let mut shadow = BTreeMap::new();
    let identity = m16_r2_identity();
    shadow.insert(
        live_zone_key(identity, Zone::TrackpadRing.id()),
        vec![static_record(Rgb::new(1, 2, 3))],
    );
    shadow.insert(
        live_zone_key(identity, Zone::RearLogo.id()),
        vec![static_record(Rgb::new(4, 5, 6))],
    );

    let resolved = resolve_zone_records(&shadow, identity, &[Zone::RearLogo, Zone::TrackpadRing])
        .expect("both zones are in the shadow");

    assert_eq!(resolved[0].0, Zone::RearLogo);
    assert_eq!(resolved[0].1[0].rgb, Rgb::new(4, 5, 6));
    assert_eq!(resolved[1].0, Zone::TrackpadRing);
    assert_eq!(resolved[1].1[0].rgb, Rgb::new(1, 2, 3));
}

#[test]
fn live_shadow_keys_do_not_cross_controller_profiles() {
    let m16 = m16_r2_identity();
    let imported = imported_identity();
    assert_ne!(live_zone_key(m16, 0), live_zone_key(imported, 0));

    let mut shadow = BTreeMap::from([
        (
            live_zone_key(m16, 0),
            vec![static_record(Rgb::new(1, 2, 3))],
        ),
        (
            live_zone_key(imported, 0),
            vec![static_record(Rgb::new(4, 5, 6))],
        ),
    ]);
    retain_live_shadow_entries(&mut shadow, Some(imported));

    assert_eq!(shadow.len(), 1);
    assert!(shadow.contains_key(&live_zone_key(imported, 0)));
}

#[test]
fn live_shadow_disappearance_clears_every_entry() {
    let identity = m16_r2_identity();
    let mut shadow = BTreeMap::from([(
        live_zone_key(identity, 0),
        vec![static_record(Rgb::new(1, 2, 3))],
    )]);

    retain_live_shadow_entries(&mut shadow, None);

    assert!(shadow.is_empty());
}

#[test]
fn zone_selection_rejects_more_ids_than_fit_the_report() {
    let zones = (0_u8..29).map(Zone::Live).collect::<Vec<_>>();
    let error = select_zones_report(&zones).expect_err("29 zone IDs exceed the report bound");

    assert!(error.contains("at most 28"));
}

#[test]
fn power_button_ac_write_includes_two_endpoint_charging_morph() {
    let channel = RecordingChannel::default();
    let write = PowerButtonWrite {
        colours: PowerButtonColours {
            ac: Rgb::new(1, 2, 3),
            battery: Rgb::new(4, 5, 6),
        },
        ac: true,
        battery: false,
    };

    send_power_button_write(&channel, write).expect("AC power-button write should apply");

    let writes = channel.writes.into_inner();
    // Five writes per slot (REMOVE, START_NEW, SELECT, ADD_ACTION, FINISH_N_SAVE), three slots.
    assert_eq!(writes.len(), 15);
    let modes = [0, 5, 10].map(|action_report_index| writes[action_report_index + 3][3]);
    assert_eq!(
        modes,
        [0x02, 0x00, 0x02],
        "AC sleep/charging are morphs and AC active is static"
    );
    assert_eq!(writes[3][11], 0x02);
    assert_eq!(&writes[3][16..19], &[0, 0, 0]);
    assert_eq!(&writes[13][8..11], &[1, 2, 3]);
    assert_eq!(writes[13][11], 0x02);
    assert_eq!(&writes[13][16..19], &[4, 5, 6]);
}

#[test]
fn power_button_battery_write_updates_charging_and_battery_conditions() {
    let channel = RecordingChannel::default();
    let write = PowerButtonWrite {
        colours: PowerButtonColours {
            ac: Rgb::new(1, 2, 3),
            battery: Rgb::new(4, 5, 6),
        },
        ac: false,
        battery: true,
    };

    send_power_button_write(&channel, write).expect("battery power-button write should apply");

    let writes = channel.writes.into_inner();
    assert_eq!(writes.len(), 20);
    let modes = [0, 5, 10, 15].map(|action_report_index| writes[action_report_index + 3][3]);
    assert_eq!(
        modes,
        [0x02, 0x02, 0x00, 0x01],
        "charging/battery sleep are morphs, battery active is static, and critical is pulse"
    );
}

#[test]
fn paired_power_button_write_programs_charging_slot_once() {
    let channel = RecordingChannel::default();
    let write = PowerButtonWrite {
        colours: PowerButtonColours {
            ac: Rgb::new(1, 2, 3),
            battery: Rgb::new(4, 5, 6),
        },
        ac: true,
        battery: true,
    };

    send_power_button_write(&channel, write).expect("paired power-button write should apply");

    let writes = channel.writes.into_inner();
    assert_eq!(writes.len(), 30);
    let slot_ids = [0, 5, 10, 15, 20, 25].map(|envelope_index| writes[envelope_index][6]);
    assert_eq!(slot_ids, [0x5b, 0x5c, 0x5d, 0x5e, 0x5f, 0x60]);
}

#[test]
fn failed_power_button_write_does_not_advance_shadow() {
    struct FailingChannel;

    impl HidChannel for FailingChannel {
        fn write_feature_report(&self, _payload: &[u8]) -> Result<(), String> {
            Err("simulated write failure".to_owned())
        }

        fn get_feature_report(&self, _buffer: &mut [u8]) -> Result<usize, String> {
            Err("unused".to_owned())
        }
    }

    let write = PowerButtonWrite {
        colours: PowerButtonColours {
            ac: Rgb::new(1, 2, 3),
            battery: Rgb::new(4, 5, 6),
        },
        ac: true,
        battery: true,
    };
    let mut shadow = PowerButtonShadow::default();

    assert!(send_power_button_write(&FailingChannel, write).is_err());
    assert!(shadow.rgb_ac.is_none());
    assert!(shadow.rgb_battery.is_none());

    remember_power_button_write_in(&mut shadow, write, Instant::now());
    assert_eq!(shadow.rgb_ac, Some(Rgb::new(1, 2, 3)));
    assert_eq!(shadow.rgb_battery, Some(Rgb::new(4, 5, 6)));
}

#[derive(Default)]
struct RecordingChannel {
    writes: RefCell<Vec<Vec<u8>>>,
}

impl HidChannel for RecordingChannel {
    fn write_feature_report(&self, payload: &[u8]) -> Result<(), String> {
        self.writes.borrow_mut().push(payload.to_vec());
        Ok(())
    }

    fn get_feature_report(&self, _buffer: &mut [u8]) -> Result<usize, String> {
        Err("no simulated response".to_owned())
    }
}

#[test]
fn save_live_zones_writes_the_confirmed_transaction_shape() {
    let channel = RecordingChannel::default();
    let per_zone = vec![
        (Zone::RearLogo, vec![static_record(Rgb::new(0, 0, 0xff))]),
        (
            Zone::TrackpadRing,
            vec![static_record(Rgb::new(0xff, 0, 0))],
        ),
    ];

    save_live_zones(&channel, &per_zone).expect("save transaction should apply");

    let writes = channel.writes.into_inner();
    // REMOVE, START_NEW, (SELECT + ADD_ACTION) per zone (2 zones), FINISH_N_SAVE, SET_DEFAULT.
    assert_eq!(writes.len(), 8);
    assert_eq!(
        &writes[0][..9],
        &[0x00, 0x03, 0x21, 0x00, 0x04, 0x00, 0x61, 0x00, 0x00],
        "first write removes any existing saved animation in slot 0x0061"
    );
    assert_eq!(
        &writes[1][..9],
        &[0x00, 0x03, 0x21, 0x00, 0x01, 0x00, 0x61, 0x00, 0x00],
        "second write opens a new saved animation in slot 0x0061"
    );
    assert_eq!(
        &writes[2][..7],
        &[0x00, 0x03, 0x23, 0x01, 0x00, 0x01, 0x02],
        "rear logo is selected before its own action record"
    );
    assert_eq!(
        &writes[4][..7],
        &[0x00, 0x03, 0x23, 0x01, 0x00, 0x01, 0x00],
        "trackpad ring is selected before its own action record"
    );
    assert_eq!(
        &writes[6][..9],
        &[0x00, 0x03, 0x21, 0x00, 0x02, 0x00, 0x61, 0x00, 0x00],
        "seventh write finishes and saves slot 0x0061"
    );
    assert_eq!(
        &writes[7][..9],
        &[0x00, 0x03, 0x21, 0x00, 0x06, 0x00, 0x61, 0x00, 0x00],
        "last write marks slot 0x0061 as the default live profile"
    );
}
