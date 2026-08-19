// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn capability_id_covers_every_variant() {
    let colour = Rgb::new(1, 2, 3);
    assert_eq!(Effect::Off.capability_id(), "off");
    assert_eq!(
        Effect::Static {
            colour: Colour::rgb(colour)
        }
        .capability_id(),
        "static"
    );
    assert_eq!(
        Effect::Breathe {
            colour,
            period_ms: 1
        }
        .capability_id(),
        "breathe"
    );
    assert_eq!(
        Effect::Pulse {
            colour,
            period_ms: 1
        }
        .capability_id(),
        "pulse"
    );
    assert_eq!(
        Effect::Strobe {
            colour,
            period_ms: 1
        }
        .capability_id(),
        "strobe"
    );
    assert_eq!(
        Effect::Scanner {
            colour,
            period_ms: 1
        }
        .capability_id(),
        "scanner"
    );
    assert_eq!(
        Effect::Morph {
            colours: vec![colour],
            period_ms: 1
        }
        .capability_id(),
        "morph"
    );
    assert_eq!(
        Effect::Spectrum { period_ms: 1 }.capability_id(),
        "spectrum"
    );
    assert_eq!(Effect::Rainbow { period_ms: 1 }.capability_id(), "rainbow");
    assert_eq!(
        Effect::Hardware {
            id: HardwareEffectId::new("vendor-scene"),
            arguments: EffectArguments::default(),
        }
        .capability_id(),
        "vendor-scene"
    );
}
