// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Effect-request conversion and validation tests.

use super::*;

fn request(kind: &str) -> EffectRequest {
    EffectRequest {
        kind: kind.to_owned(),
        ..EffectRequest::default()
    }
}

fn rgb_request(kind: &str, colours: Vec<DbusRgb>) -> EffectRequest {
    EffectRequest {
        colours: Some(colours),
        ..request(kind)
    }
}

#[test]
fn converts_portable_effects_and_rejects_missing_arguments() {
    assert!(matches!(request("off").into_effect(), Ok(Effect::Off)));
    assert!(matches!(
        (EffectRequest {
            period_ms: Some(10),
            ..rgb_request("breathe", vec![(1, 2, 3)])
        })
        .into_effect(),
        Ok(Effect::Breathe { period_ms: 10, .. })
    ));
    for kind in ["pulse", "strobe", "scanner"] {
        assert!(rgb_request(kind, vec![(1, 2, 3)]).into_effect().is_err());
    }
    assert!(
        rgb_request("morph", vec![(1, 2, 3), (4, 5, 6)])
            .into_effect()
            .is_err()
    );
    assert!(request("spectrum").into_effect().is_err());
    assert!(request("rainbow").into_effect().is_err());
    assert!(request("unknown").into_effect().is_err());
}

#[test]
fn converts_hardware_effects_and_rejects_incompatible_fields() {
    let effect = EffectRequest {
        kind: "hardware".into(),
        hardware_id: Some("sparkle".into()),
        colours: Some(vec![(1, 2, 3)]),
        speed: Some(2),
        direction: Some("clockwise".into()),
        duration_ms: Some(100),
        brightness: Some(10),
        choice: Some("fast".into()),
        ..EffectRequest::default()
    }
    .into_effect();
    assert!(matches!(effect, Ok(Effect::Hardware { .. })));
    assert!(
        EffectRequest {
            kind: "hardware".into(),
            direction: Some("sideways".into()),
            ..EffectRequest::default()
        }
        .into_effect()
        .is_err()
    );
    assert!(
        EffectRequest {
            kind: "off".into(),
            hardware_id: Some("bad".into()),
            ..EffectRequest::default()
        }
        .into_effect()
        .is_err()
    );
    assert!(
        EffectRequest {
            kind: "static".into(),
            colours: Some(vec![(1, 2, 3)]),
            ..EffectRequest::default()
        }
        .into_effect()
        .is_err()
    );
}

#[test]
fn converts_every_static_colour_model_and_validates_channels() {
    for (model, channels) in [
        ("hsv", vec![("hue", 1), ("saturation", 2), ("value", 3)]),
        ("hsl", vec![("hue", 1), ("saturation", 2), ("lightness", 3)]),
        ("cct", vec![("temperature", 2_700)]),
        ("monochrome", vec![("intensity", 10)]),
    ] {
        let colour = StaticColourRequest {
            model: model.into(),
            channels: channels
                .into_iter()
                .map(|(name, value)| (name.into(), value))
                .collect(),
        };
        assert!(
            EffectRequest {
                kind: "static".into(),
                static_colour: Some(colour),
                ..EffectRequest::default()
            }
            .into_effect()
            .is_ok()
        );
    }
    let additive = StaticColourRequest {
        model: "additive".into(),
        channels: [("red", 1), ("cool-white", 2), ("ultraviolet", 3)]
            .into_iter()
            .map(|(name, value)| (name.into(), value))
            .collect(),
    };
    assert!(additive.to_colour().is_ok());
    for bad in ["", "unknown"] {
        assert!(
            StaticColourRequest {
                model: "additive".into(),
                channels: [(bad.into(), 1)].into_iter().collect(),
            }
            .to_colour()
            .is_err()
        );
    }
    assert!(
        StaticColourRequest {
            model: "hsv".into(),
            channels: [("hue".into(), 1)].into_iter().collect(),
        }
        .to_colour()
        .is_err()
    );
    assert!(
        StaticColourRequest {
            model: "unknown".into(),
            channels: HashMap::new(),
        }
        .to_colour()
        .is_err()
    );
}
