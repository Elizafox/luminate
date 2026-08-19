// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for native D-Bus scene conversion.

use super::*;
use luminate::capability::HardwareEffectId;
use luminate::colour::ColourChannelValue;
use luminate::effect::EffectArguments;
use luminate::rgb::Rgb;
use luminate::target::TargetId;

fn scene(effect: Effect) -> Scene {
    Scene {
        id: luminate::SceneId::new("scene"),
        revision: 1,
        name: "Test".to_owned(),
        description: None,
        owner: OwnerIdentity::Uid(1000),
        bindings: vec![SceneBinding::Frozen {
            target: TargetId::device("desk"),
            state: SceneTargetState {
                appearance: Some(effect),
                brightness: Some(50),
                emission: Some(EmissionState::Emitting),
                appearance_slots: None,
            },
        }],
    }
}

#[test]
fn binding_requests_validate_targets_states_and_collection_modes() {
    let frozen = SceneBindingRequest {
        target: "device:desk".to_owned(),
        dynamic_collection: None,
        appearance: None,
        brightness: Some(10),
        emission: Some("dark".to_owned()),
        appearance_slots: None,
    }
    .into_binding()
    .expect("valid frozen binding");
    assert!(matches!(frozen, SceneBinding::Frozen { .. }));

    let dynamic = SceneBindingRequest {
        target: "device:desk/surface:front".to_owned(),
        dynamic_collection: Some("lights".to_owned()),
        appearance: None,
        brightness: Some(10),
        emission: Some("dark".to_owned()),
        appearance_slots: None,
    }
    .into_binding()
    .expect("valid dynamic binding");
    assert!(matches!(
        dynamic,
        SceneBinding::DynamicCollectionMember { .. }
    ));

    let invalid = SceneBindingRequest {
        target: "device:desk".to_owned(),
        dynamic_collection: None,
        appearance: None,
        brightness: None,
        emission: Some("unknown".to_owned()),
        appearance_slots: None,
    }
    .into_binding();
    assert!(invalid.is_err());
}

#[test]
fn records_all_owner_and_effect_shapes() {
    let effects = vec![
        Effect::Off,
        Effect::Static {
            colour: Colour::rgb(Rgb::new(1, 2, 3)),
        },
        Effect::Breathe {
            colour: Rgb::new(1, 2, 3),
            period_ms: 10,
        },
        Effect::Pulse {
            colour: Rgb::new(1, 2, 3),
            period_ms: 10,
        },
        Effect::Strobe {
            colour: Rgb::new(1, 2, 3),
            period_ms: 10,
        },
        Effect::Scanner {
            colour: Rgb::new(1, 2, 3),
            period_ms: 10,
        },
        Effect::Morph {
            colours: vec![Rgb::new(1, 2, 3), Rgb::new(4, 5, 6)],
            period_ms: 10,
        },
        Effect::Spectrum { period_ms: 10 },
        Effect::Rainbow { period_ms: 10 },
        Effect::Hardware {
            id: HardwareEffectId::new("custom"),
            arguments: EffectArguments {
                colours: vec![Rgb::new(1, 2, 3)],
                speed: Some(1),
                direction: None,
                duration_ms: Some(10),
                brightness: Some(20),
                choice: Some("one".to_owned()),
            },
        },
    ];
    for effect in effects {
        let record = record(scene(effect)).expect("scene record");
        assert_eq!(record.0, "scene");
        assert_eq!(record.7.len(), 1);
    }

    let mut sid = scene(Effect::Off);
    sid.owner = OwnerIdentity::Sid("S-1-5-21".to_owned());
    let record = record(sid).expect("SID scene record");
    assert_eq!(record.5, "sid");
    assert_eq!(record.6, "S-1-5-21");
    assert!(!record.3);
}

#[test]
fn static_colour_records_cover_every_model_and_channel_name() {
    let additive_channels = [
        ColourChannel::Red,
        ColourChannel::Green,
        ColourChannel::Blue,
        ColourChannel::White,
        ColourChannel::WarmWhite,
        ColourChannel::CoolWhite,
        ColourChannel::Amber,
        ColourChannel::Ultraviolet,
    ];
    let additive = Colour::additive(
        additive_channels
            .into_iter()
            .enumerate()
            .map(|(index, channel)| {
                ColourChannelValue::new(
                    channel,
                    u32::try_from(index).expect("eight channels fit in u32"),
                )
            })
            .collect(),
    )
    .expect("valid additive colour");
    for colour in [
        additive,
        Colour::hsv(1, 2, 3),
        Colour::hsl(4, 5, 6),
        Colour::cct(4_000),
        Colour::monochrome(7),
    ] {
        let record = record(scene(Effect::Static { colour })).expect("static colour scene record");
        assert_eq!(record.7.len(), 1);
    }

    let names = [
        ColourChannel::Red,
        ColourChannel::Green,
        ColourChannel::Blue,
        ColourChannel::White,
        ColourChannel::WarmWhite,
        ColourChannel::CoolWhite,
        ColourChannel::Amber,
        ColourChannel::Ultraviolet,
        ColourChannel::Hue,
        ColourChannel::Saturation,
        ColourChannel::Value,
        ColourChannel::Lightness,
        ColourChannel::Temperature,
        ColourChannel::Intensity,
    ]
    .map(channel);
    assert_eq!(
        names,
        [
            "red",
            "green",
            "blue",
            "white",
            "warm-white",
            "cool-white",
            "amber",
            "ultraviolet",
            "hue",
            "saturation",
            "value",
            "lightness",
            "temperature",
            "intensity",
        ]
    );
}
