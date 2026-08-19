// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford
// SPDX-FileCopyrightText: 2021 Adam Honse
// SPDX-FileCopyrightText: 2023 Chris M (Dr_No)

//! Conservative topology for hardware whose physical LED map is still under validation.

use luminate_core::capability::{
    BrightnessCapability, BufferingMode, CapabilityScope, CapabilitySet, CctEmulation,
    ColourCapability, EffectDirection, EffectParameter, FrameUpdateMode, FrameUploadCapability,
    HardwareEffectDescriptor, HardwareEffectId, HardwareEffectsCapability, PersistenceCapability,
    PersistenceRequirement, StateReadbackCapability,
};
use luminate_core::device::{DeviceCategory, device_category};
use luminate_core::element::{ElementGeometry, ElementKind};
use luminate_core::surface::SurfaceKind;
use luminate_core::util::DiscreteRange;
use luminate_plugin_api::{
    ClaimExclusivity, DeviceDescriptor, ElementDescriptor, HardwareBus, HardwareClaim,
    SurfaceDescriptor,
};

#[cfg(test)]
use crate::devices::PROFILES;
use crate::devices::{BLACKWIDOW_V4_PRO, Profile};

pub(crate) const DEVICE_ID: &str = "razer-blackwidow-v4-pro";
pub(crate) const SURFACE_ID: &str = "lighting";
pub(crate) const SPECTRUM_EFFECT_ID: &str = "razer-spectrum";
pub(crate) const WAVE_EFFECT_ID: &str = "razer-wave";
pub(crate) const WHEEL_EFFECT_ID: &str = "razer-wheel";
pub(crate) const REACTIVE_EFFECT_ID: &str = "razer-reactive";
pub(crate) const BREATHING_EFFECT_ID: &str = "razer-breathing";
pub(crate) const STARLIGHT_EFFECT_ID: &str = "razer-starlight";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoordinateEvidence {
    LiveValidated,
    Corroborated,
    Presumed,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ElementSpec {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) row: u8,
    pub(crate) column: u8,
    pub(crate) kind: ElementKind,
    pub(crate) evidence: CoordinateEvidence,
}

const fn key(id: &'static str, name: &'static str, row: u8, column: u8) -> ElementSpec {
    ElementSpec {
        id,
        name,
        row,
        column,
        kind: ElementKind::Key,
        evidence: CoordinateEvidence::Corroborated,
    }
}

const fn zone(
    id: &'static str,
    name: &'static str,
    row: u8,
    column: u8,
    evidence: CoordinateEvidence,
) -> ElementSpec {
    ElementSpec {
        id,
        name,
        row,
        column,
        kind: ElementKind::Zone,
        evidence,
    }
}

pub(crate) static ELEMENTS: &[ElementSpec] = &[
    zone(
        "command-dial",
        "Command dial",
        0,
        1,
        CoordinateEvidence::LiveValidated,
    ),
    key("escape", "Escape", 0, 2),
    key("f1", "F1", 0, 4),
    key("f2", "F2", 0, 5),
    key("f3", "F3", 0, 6),
    key("f4", "F4", 0, 7),
    key("f5", "F5", 0, 8),
    key("f6", "F6", 0, 9),
    key("f7", "F7", 0, 10),
    key("f8", "F8", 0, 11),
    key("f9", "F9", 0, 12),
    key("f10", "F10", 0, 13),
    key("f11", "F11", 0, 14),
    key("f12", "F12", 0, 15),
    key("print-screen", "Print Screen", 0, 16),
    key("scroll-lock", "Scroll Lock", 0, 17),
    key("pause", "Pause", 0, 18),
    key("media-previous", "Previous track", 0, 19),
    key("media-play-pause", "Play/pause", 0, 20),
    key("media-next", "Next track", 0, 21),
    key("media-mute", "Mute", 0, 22),
    key("macro-5", "M5", 1, 1),
    key("grave", "Grave", 1, 2),
    key("1", "1", 1, 3),
    key("2", "2", 1, 4),
    key("3", "3", 1, 5),
    key("4", "4", 1, 6),
    key("5", "5", 1, 7),
    key("6", "6", 1, 8),
    key("7", "7", 1, 9),
    key("8", "8", 1, 10),
    key("9", "9", 1, 11),
    key("0", "0", 1, 12),
    key("minus", "Minus", 1, 13),
    key("equals", "Equals", 1, 14),
    key("backspace", "Backspace", 1, 15),
    key("insert", "Insert", 1, 16),
    key("home", "Home", 1, 17),
    key("page-up", "Page Up", 1, 18),
    key("numpad-lock", "Numpad Lock", 1, 19),
    key("numpad-divide", "Numpad /", 1, 20),
    key("numpad-multiply", "Numpad *", 1, 21),
    key("numpad-minus", "Numpad -", 1, 22),
    key("macro-4", "M4", 2, 1),
    key("tab", "Tab", 2, 2),
    key("q", "Q", 2, 3),
    key("w", "W", 2, 4),
    key("e", "E", 2, 5),
    key("r", "R", 2, 6),
    key("t", "T", 2, 7),
    key("y", "Y", 2, 8),
    key("u", "U", 2, 9),
    key("i", "I", 2, 10),
    key("o", "O", 2, 11),
    key("p", "P", 2, 12),
    key("left-bracket", "Left bracket", 2, 13),
    key("right-bracket", "Right bracket", 2, 14),
    key("backslash", "Backslash", 2, 15),
    key("delete", "Delete", 2, 16),
    key("end", "End", 2, 17),
    key("page-down", "Page Down", 2, 18),
    key("numpad-7", "Numpad 7", 2, 19),
    key("numpad-8", "Numpad 8", 2, 20),
    key("numpad-9", "Numpad 9", 2, 21),
    key("numpad-plus", "Numpad +", 2, 22),
    key("macro-3", "M3", 3, 1),
    key("caps-lock", "Caps Lock", 3, 2),
    key("a", "A", 3, 3),
    key("s", "S", 3, 4),
    key("d", "D", 3, 5),
    key("f", "F", 3, 6),
    key("g", "G", 3, 7),
    key("h", "H", 3, 8),
    key("j", "J", 3, 9),
    key("k", "K", 3, 10),
    key("l", "L", 3, 11),
    key("semicolon", "Semicolon", 3, 12),
    key("apostrophe", "Apostrophe", 3, 13),
    key("enter", "Enter", 3, 15),
    key("numpad-4", "Numpad 4", 3, 19),
    key("numpad-5", "Numpad 5", 3, 20),
    key("numpad-6", "Numpad 6", 3, 21),
    key("macro-2", "M2", 4, 1),
    key("left-shift", "Left Shift", 4, 2),
    key("z", "Z", 4, 4),
    key("x", "X", 4, 5),
    key("c", "C", 4, 6),
    key("v", "V", 4, 7),
    key("b", "B", 4, 8),
    key("n", "N", 4, 9),
    key("m", "M", 4, 10),
    key("comma", "Comma", 4, 11),
    key("period", "Period", 4, 12),
    key("slash", "Slash", 4, 13),
    key("right-shift", "Right Shift", 4, 15),
    key("up", "Up", 4, 17),
    key("numpad-1", "Numpad 1", 4, 19),
    key("numpad-2", "Numpad 2", 4, 20),
    key("numpad-3", "Numpad 3", 4, 21),
    key("numpad-enter", "Numpad Enter", 4, 22),
    key("macro-1", "M1", 5, 1),
    key("left-control", "Left Control", 5, 2),
    key("left-meta", "Left Meta", 5, 3),
    key("left-alt", "Left Alt", 5, 4),
    key("space", "Space", 5, 8),
    key("right-alt", "Right Alt", 5, 12),
    key("fn", "Fn", 5, 13),
    key("menu", "Menu", 5, 14),
    key("right-control", "Right Control", 5, 15),
    key("left", "Left", 5, 16),
    key("down", "Down", 5, 17),
    key("right", "Right", 5, 18),
    key("numpad-0", "Numpad 0", 5, 20),
    key("numpad-period", "Numpad .", 5, 21),
    zone(
        "left-side-1",
        "Left side 1",
        6,
        0,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "left-side-2",
        "Left side 2",
        6,
        1,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "left-side-3",
        "Left side 3",
        6,
        2,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "left-side-4",
        "Left side 4",
        6,
        3,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "left-side-5",
        "Left side 5",
        6,
        4,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "left-side-6",
        "Left side 6",
        6,
        5,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "left-side-7",
        "Left side 7",
        6,
        6,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "left-side-8",
        "Left side 8",
        6,
        7,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "left-side-9",
        "Left side 9",
        6,
        8,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "right-side-1",
        "Right side 1",
        6,
        17,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "right-side-2",
        "Right side 2",
        6,
        16,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "right-side-3",
        "Right side 3",
        6,
        15,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "right-side-4",
        "Right side 4",
        6,
        14,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "right-side-5",
        "Right side 5",
        6,
        13,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "right-side-6",
        "Right side 6",
        6,
        12,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "right-side-7",
        "Right side 7",
        6,
        11,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "right-side-8",
        "Right side 8",
        6,
        10,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "right-side-9",
        "Right side 9",
        6,
        9,
        CoordinateEvidence::LiveValidated,
    ),
    zone(
        "wrist-rest-1",
        "Wrist rest 1",
        7,
        0,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-2",
        "Wrist rest 2",
        7,
        1,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-3",
        "Wrist rest 3",
        7,
        2,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-4",
        "Wrist rest 4",
        7,
        3,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-5",
        "Wrist rest 5",
        7,
        4,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-6",
        "Wrist rest 6",
        7,
        5,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-7",
        "Wrist rest 7",
        7,
        6,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-8",
        "Wrist rest 8",
        7,
        7,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-9",
        "Wrist rest 9",
        7,
        8,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-10",
        "Wrist rest 10",
        7,
        9,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-11",
        "Wrist rest 11",
        7,
        10,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-12",
        "Wrist rest 12",
        7,
        11,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-13",
        "Wrist rest 13",
        7,
        12,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-14",
        "Wrist rest 14",
        7,
        13,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-15",
        "Wrist rest 15",
        7,
        14,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-16",
        "Wrist rest 16",
        7,
        15,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-17",
        "Wrist rest 17",
        7,
        16,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-18",
        "Wrist rest 18",
        7,
        17,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-19",
        "Wrist rest 19",
        7,
        18,
        CoordinateEvidence::Presumed,
    ),
    zone(
        "wrist-rest-20",
        "Wrist rest 20",
        7,
        19,
        CoordinateEvidence::Presumed,
    ),
];

pub(crate) fn blackwidow_v4_pro(profile: Profile) -> DeviceDescriptor {
    DeviceDescriptor {
        id: DEVICE_ID.to_owned(),
        name: profile.name.to_owned(),
        vendor: Some("Razer".to_owned()),
        model: Some("1532:028d".to_owned()),
        surfaces: vec![SurfaceDescriptor {
            id: SURFACE_ID.to_owned(),
            name: "Keyboard lighting".to_owned(),
            kind: SurfaceKind::Matrix {
                rows: u16::from(profile.rows),
                cols: u16::from(profile.columns),
            },
            physical_tags: Vec::new(),
            elements: ELEMENTS.iter().map(element_descriptor).collect(),
            capabilities: capabilities(
                CapabilityScope::Surface,
                true,
                profile.supports_wheel,
                profile.supports_starlight,
            ),
            notes: vec![
                "The US-ANSI key map is corroborated by OpenRGB; command-dial and side-light coordinates have direct hardware validation.".to_owned(),
                "Wrist-rest coordinates are presumed from OpenRGB and remain unverified on this unit.".to_owned(),
            ],
            warnings: Vec::new(),
        }],
        groups: Vec::new(),
        capabilities: capabilities(
            CapabilityScope::Device,
            true,
            profile.supports_wheel,
            profile.supports_starlight,
        ),
        claims: vec![HardwareClaim {
            bus: HardwareBus::Hid,
            physical_identity: "1532:028d/interface-3".to_owned(),
            control_domain: "lighting".to_owned(),
            exclusivity: ClaimExclusivity::Exclusive,
        }],
        category: Some(DeviceCategory::new(device_category::KEYBOARD)),
        physical_tags: vec![
            "shape:keyboard".to_owned(),
            "layout:us-ansi".to_owned(),
            "form:standalone".to_owned(),
        ],
        host_attached: true,
        notes: vec![
            format!(
                "Experimental {:?} profile. Static colour, firmware effects, brightness, full frames, identity, and transport have passed tests on one physical unit.",
                profile.validation
            ),
        ],
        warnings: vec![
            "Razer support is experimental; topology and capabilities may change as hardware validation continues.".to_owned(),
        ],
    }
}

pub(crate) fn untested_matrix_keyboard(profile: Profile) -> DeviceDescriptor {
    let elements = (0..profile.rows)
        .flat_map(|row| {
            (0..profile.columns).map(move |column| coordinate_element(row, column, profile))
        })
        .collect();
    DeviceDescriptor {
        id: format!("razer-{}", profile.slug),
        name: profile.name.to_owned(),
        vendor: Some("Razer".to_owned()),
        model: Some(format!("{:04x}:{:04x}", profile.vendor_id, profile.product_id)),
        surfaces: vec![SurfaceDescriptor {
            id: SURFACE_ID.to_owned(),
            name: "Keyboard lighting".to_owned(),
            kind: SurfaceKind::Matrix {
                rows: u16::from(profile.rows),
                cols: u16::from(profile.columns),
            },
            physical_tags: Vec::new(),
            elements,
            capabilities: capabilities(
                CapabilityScope::Surface,
                false,
                profile.supports_wheel,
                profile.supports_starlight,
            ),
            notes: vec!["Coordinates and matrix dimensions are inherited from upstream protocol research and have not been checked on physical hardware.".to_owned()],
            warnings: vec!["This profile is provisional and untested. Hardware reports are welcome.".to_owned()],
        }],
        groups: Vec::new(),
        capabilities: capabilities(
            CapabilityScope::Device,
            false,
            profile.supports_wheel,
            profile.supports_starlight,
        ),
        claims: vec![HardwareClaim {
            bus: HardwareBus::Hid,
            physical_identity: format!(
                "{:04x}:{:04x}/interface-{}",
                profile.vendor_id, profile.product_id, profile.control_interface
            ),
            control_domain: "lighting".to_owned(),
            exclusivity: ClaimExclusivity::Exclusive,
        }],
        category: Some(DeviceCategory::new(device_category::KEYBOARD)),
        physical_tags: vec!["shape:keyboard".to_owned(), "form:standalone".to_owned()],
        host_attached: true,
        notes: vec![format!(
            "Experimental {:?} profile imported from pinned OpenRazer protocol data.",
            profile.validation
        )],
        warnings: vec![
            "Razer support is experimental; topology and capabilities may change as hardware validation continues.".to_owned(),
        ],
    }
}

fn coordinate_element(row: u8, column: u8, profile: Profile) -> ElementDescriptor {
    ElementDescriptor {
        id: format!("r{row}-c{column}"),
        name: Some(format!("Row {row}, column {column}")),
        kind: ElementKind::Zone,
        geometry: Some(ElementGeometry::Rect {
            x: f32::from(column) / f32::from(profile.columns),
            y: f32::from(row) / f32::from(profile.rows),
            w: 1.0 / f32::from(profile.columns),
            h: 1.0 / f32::from(profile.rows),
        }),
        physical_tags: Vec::new(),
        capabilities: element_capabilities(),
        notes: vec!["Physical identity and occupancy are unverified.".to_owned()],
        warnings: Vec::new(),
    }
}

pub(crate) fn coordinate_for(profile: Profile, id: &str) -> Option<(u8, u8)> {
    if profile == BLACKWIDOW_V4_PRO {
        return element_coordinate(id);
    }
    let (row, column) = id.strip_prefix('r')?.split_once("-c")?;
    let row = row.parse::<u8>().ok()?;
    let column = column.parse::<u8>().ok()?;
    (row < profile.rows && column < profile.columns).then_some((row, column))
}

fn element_descriptor(spec: &ElementSpec) -> ElementDescriptor {
    let notes = match spec.evidence {
        CoordinateEvidence::LiveValidated => Vec::new(),
        CoordinateEvidence::Corroborated => vec![
            "Coordinate is derived from OpenRGB and has not been individually hardware-validated on this unit.".to_owned(),
        ],
        CoordinateEvidence::Presumed => vec![
            "Coordinate is presumed from OpenRGB; the wrist rest was unavailable for direct validation.".to_owned(),
        ],
    };

    ElementDescriptor {
        id: spec.id.to_owned(),
        name: Some(spec.name.to_owned()),
        kind: spec.kind.clone(),
        geometry: Some(ElementGeometry::Rect {
            x: f32::from(spec.column) / 23.0,
            y: f32::from(spec.row) / 8.0,
            w: 1.0 / 23.0,
            h: 1.0 / 8.0,
        }),
        physical_tags: Vec::new(),
        capabilities: element_capabilities(),
        notes,
        warnings: Vec::new(),
    }
}

fn element_capabilities() -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        emission: true,
        off_is_wear_safe: true,
        ..CapabilitySet::default()
    }
}

pub(crate) fn element_coordinate(id: &str) -> Option<(u8, u8)> {
    ELEMENTS
        .iter()
        .find(|element| element.id == id)
        .map(|element| (element.row, element.column))
}

fn capabilities(
    scope: CapabilityScope,
    persistence: bool,
    supports_wheel: bool,
    supports_starlight: bool,
) -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: 255,
            scope,
        },
        hardware_effects: Some(HardwareEffectsCapability {
            effects: hardware_effects(supports_wheel, supports_starlight),
            scope,
            concurrent_with_streaming: false,
        }),
        appearance_slots: None,
        persistence: if persistence {
            PersistenceCapability::CurrentState {
                requirement: PersistenceRequirement::Optional,
                explicit_commit: true,
                readback: false,
            }
        } else {
            PersistenceCapability::None
        },
        state_readback: StateReadbackCapability::None,
        frame_upload: (scope == CapabilityScope::Surface).then_some(FrameUploadCapability {
            scope,
            update_mode: FrameUpdateMode::FullFrameOnly,
            max_rate_hz: Some(60),
            atomic: false,
            buffering: BufferingMode::Immediate,
            shm: None,
        }),
        emission: true,
        off_is_wear_safe: true,
        physical_power: None,
        power_domain: None,
    }
}

fn hardware_effects(
    supports_wheel: bool,
    supports_starlight: bool,
) -> Vec<HardwareEffectDescriptor> {
    let mut effects = vec![
        HardwareEffectDescriptor {
            id: HardwareEffectId::new(SPECTRUM_EFFECT_ID),
            name: "Spectrum".to_owned(),
            parameters: Vec::new(),
        },
        directional_effect(
            WAVE_EFFECT_ID,
            "Wave",
            vec![EffectDirection::Forward, EffectDirection::Reverse],
        ),
        HardwareEffectDescriptor {
            id: HardwareEffectId::new(REACTIVE_EFFECT_ID),
            name: "Reactive".to_owned(),
            parameters: vec![
                EffectParameter::Colour {
                    minimum_colours: 1,
                    maximum_colours: 1,
                },
                EffectParameter::Speed {
                    range: DiscreteRange::new(1, 4, 1),
                },
            ],
        },
        HardwareEffectDescriptor {
            id: HardwareEffectId::new(BREATHING_EFFECT_ID),
            name: "Breathing".to_owned(),
            parameters: vec![EffectParameter::Colour {
                minimum_colours: 1,
                maximum_colours: 2,
            }],
        },
    ];
    if supports_wheel {
        effects.insert(
            2,
            directional_effect(
                WHEEL_EFFECT_ID,
                "Wheel",
                vec![
                    EffectDirection::Clockwise,
                    EffectDirection::CounterClockwise,
                ],
            ),
        );
    }
    if supports_starlight {
        effects.push(HardwareEffectDescriptor {
            id: HardwareEffectId::new(STARLIGHT_EFFECT_ID),
            name: "Starlight".to_owned(),
            parameters: vec![
                EffectParameter::Colour {
                    minimum_colours: 1,
                    maximum_colours: 2,
                },
                EffectParameter::Speed {
                    range: DiscreteRange::new(1, 3, 1),
                },
            ],
        });
    }
    effects
}

fn directional_effect(
    id: &str,
    name: &str,
    values: Vec<EffectDirection>,
) -> HardwareEffectDescriptor {
    HardwareEffectDescriptor {
        id: HardwareEffectId::new(id),
        name: name.to_owned(),
        parameters: vec![EffectParameter::Direction { values }],
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn element_map_has_unique_bounded_identities_and_coordinates() {
        let mut ids = HashSet::new();
        let mut coordinates = HashSet::new();

        for element in ELEMENTS {
            assert!(
                ids.insert(element.id),
                "duplicate element ID {}",
                element.id
            );
            assert!(
                coordinates.insert((element.row, element.column)),
                "duplicate coordinate r{}-c{}",
                element.row,
                element.column
            );
            assert!(element.row < 8);
            assert!(element.column < 23);
        }
    }

    #[test]
    fn known_empty_coordinates_remain_unpublished() {
        for coordinate in [(0, 0), (6, 18), (6, 22), (7, 20), (7, 22)] {
            assert!(
                ELEMENTS
                    .iter()
                    .all(|element| (element.row, element.column) != coordinate)
            );
        }
    }

    #[test]
    fn untested_matrix_topology_is_bounded_and_disables_persistence() {
        let profile = PROFILES[3];
        let descriptor = untested_matrix_keyboard(profile);
        let surface = descriptor.surfaces.first().expect("matrix surface");

        assert_eq!(descriptor.id, "razer-blackwidow-v4-mini-hyperspeed-wired");
        assert_eq!(
            descriptor.physical_tags,
            ["shape:keyboard", "form:standalone"]
        );
        assert_eq!(surface.elements.len(), 5 * 14);
        assert_eq!(coordinate_for(profile, "r4-c13"), Some((4, 13)));
        assert_eq!(coordinate_for(profile, "r5-c0"), None);
        assert_eq!(
            descriptor.capabilities.persistence,
            PersistenceCapability::None
        );
        assert!(
            descriptor
                .warnings
                .iter()
                .any(|warning| warning.contains("experimental"))
        );
    }

    #[test]
    fn mapped_blackwidow_publishes_its_validated_physical_form() {
        let descriptor = blackwidow_v4_pro(BLACKWIDOW_V4_PRO);

        assert_eq!(
            descriptor.physical_tags,
            ["shape:keyboard", "layout:us-ansi", "form:standalone"]
        );
    }

    #[test]
    fn profile_specific_effects_are_not_advertised() {
        let profile = PROFILES
            .iter()
            .copied()
            .find(|profile| profile.slug == "huntsman-mini-analog")
            .expect("imported Huntsman Mini Analog");
        let descriptor = untested_matrix_keyboard(profile);
        let effects = descriptor
            .capabilities
            .hardware_effects
            .expect("hardware effects");

        assert!(
            effects
                .effects
                .iter()
                .all(|effect| effect.id.as_str() != STARLIGHT_EFFECT_ID)
        );
    }

    #[test]
    fn huntsman_v2_analog_uses_the_combined_upstream_frame_bounds() {
        let profile = PROFILES
            .iter()
            .copied()
            .find(|profile| profile.slug == "huntsman-v2-analog")
            .expect("imported Huntsman V2 Analog");
        let descriptor = untested_matrix_keyboard(profile);
        let surface = descriptor.surfaces.first().expect("matrix surface");

        assert_eq!(surface.elements.len(), 9 * 22);
        assert_eq!(coordinate_for(profile, "r8-c21"), Some((8, 21)));
        assert_eq!(coordinate_for(profile, "r9-c0"), None);
    }
}
