// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Topology, capability, and terminal-safety output tests.

use super::*;
use luminate_core::capability::{
    EffectChoice, EffectDirection, FrameUploadCapability, HardwareEffectDescriptor,
    HardwareEffectId, HardwareEffectsCapability, ReadableFacet, ReadbackFidelity,
};
use luminate_core::state::StateFacetKind;
use luminate_core::util::DiscreteRange;

#[test]
fn formats_all_topology_variants() {
    let surface_kinds = [
        (SurfaceKind::Opaque, "opaque"),
        (SurfaceKind::Zone, "zone"),
        (SurfaceKind::Linear { length: 2.5 }, "linear length=2.5"),
        (
            SurfaceKind::Sparse2d {
                width: 3.0,
                height: 4.0,
            },
            "sparse2d 3x4",
        ),
        (SurfaceKind::Matrix { rows: 6, cols: 22 }, "matrix 6x22"),
    ];
    for (kind, expected) in surface_kinds {
        assert_eq!(format_surface_kind(&kind), expected);
    }

    for (kind, expected) in [
        (ElementKind::Key, "key"),
        (ElementKind::Led, "led"),
        (ElementKind::Zone, "zone"),
        (ElementKind::Logo, "logo"),
        (ElementKind::RingSegment, "ring-segment"),
    ] {
        assert_eq!(format_element_kind(&kind), expected);
    }

    assert_eq!(
        format_geometry(&ElementGeometry::Rect {
            x: 0.1,
            y: 0.2,
            w: 0.3,
            h: 0.4,
        }),
        "rect x=0.1 y=0.2 w=0.3 h=0.4"
    );
    assert_eq!(
        format_geometry(&ElementGeometry::Point { x: 0.5, y: 0.6 }),
        "point x=0.5 y=0.6"
    );
    assert_eq!(
        format_geometry(&ElementGeometry::Linear { position: 0.7 }),
        "linear position=0.7"
    );
    assert_eq!(
        format_geometry(&ElementGeometry::MatrixCell { row: 2, col: 3 }),
        "matrix-cell row=2 col=3"
    );

    for (kind, expected) in [
        (GroupKind::BuiltIn, "built-in"),
        (GroupKind::Topology, "topology"),
        (GroupKind::Driver, "driver"),
        (GroupKind::User, "user"),
        (GroupKind::Application, "application"),
    ] {
        assert_eq!(format_group_kind(kind), expected);
    }
}

#[test]
fn formats_capability_variants() {
    let parameters = vec![
        EffectParameter::Colour {
            minimum_colours: 1,
            maximum_colours: 3,
        },
        EffectParameter::Speed {
            range: DiscreteRange::new(1, 5, 2),
        },
        EffectParameter::Direction {
            values: vec![EffectDirection::Forward, EffectDirection::Random],
        },
        EffectParameter::Duration {
            milliseconds: DiscreteRange::new(100, 500, 100),
        },
        EffectParameter::Brightness { bits: 8 },
        EffectParameter::Choice {
            options: vec![EffectChoice {
                id: "scene\nnight".to_owned(),
                name: "Night".to_owned(),
            }],
        },
    ];
    let capabilities = CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: 255,
            scope: CapabilityScope::Device,
        },
        frame_upload: Some(FrameUploadCapability {
            scope: CapabilityScope::Surface,
            update_mode: FrameUpdateMode::Both,
            max_rate_hz: Some(60),
            atomic: true,
            buffering: BufferingMode::DoubleBuffered,
            shm: None,
        }),
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![HardwareEffectDescriptor {
                id: HardwareEffectId::new("wave"),
                name: "Wave".to_owned(),
                parameters,
            }],
            scope: CapabilityScope::Controller,
            concurrent_with_streaming: true,
        }),
        persistence: PersistenceCapability::Profiles {
            requirement: PersistenceRequirement::Required,
            slots: 4,
            explicit_commit: true,
            readback: false,
        },
        ..CapabilitySet::default()
    };

    let formatted = format_capabilities(&capabilities);
    for expected in [
        "additive[red:8,green:8,blue:8]",
        "independent:8bit:0-255@device",
        "emission=false",
        "profiles(required, slots=4, commit=true, readback=false)",
        "frame-upload=surface full-or-partial double-buffered 60Hz",
        "speed:1-5 step 2",
        "direction:forward|random",
        "choice:scene\\nnight",
        "hw-effects=controller",
        ", concurrent",
    ] {
        assert!(
            formatted.contains(expected),
            "missing {expected:?}: {formatted}"
        );
    }
}

#[test]
fn small_formatters_cover_remaining_enum_values() {
    for (scope, expected) in [
        (CapabilityScope::Element, "element"),
        (CapabilityScope::Surface, "surface"),
        (CapabilityScope::Device, "device"),
        (CapabilityScope::Controller, "controller"),
    ] {
        assert_eq!(format_scope(scope), expected);
    }
    for (mode, expected) in [
        (FrameUpdateMode::FullFrameOnly, "full-frame"),
        (FrameUpdateMode::Partial, "partial"),
        (FrameUpdateMode::Both, "full-or-partial"),
    ] {
        assert_eq!(format_frame_update_mode(mode), expected);
    }
    for (mode, expected) in [
        (BufferingMode::Immediate, "immediate"),
        (BufferingMode::ExplicitCommit, "explicit-commit"),
        (BufferingMode::DoubleBuffered, "double-buffered"),
    ] {
        assert_eq!(format_buffering_mode(mode), expected);
    }
}

#[test]
fn format_persistence_capability_covers_current_state() {
    let current_state = PersistenceCapability::CurrentState {
        requirement: PersistenceRequirement::Optional,
        explicit_commit: false,
        readback: true,
    };
    assert_eq!(
        format_persistence_capability(&current_state),
        "state(optional, commit=false, readback=true)"
    );
    assert_eq!(
        format_persistence_capability(&PersistenceCapability::None),
        "none"
    );
}

#[test]
fn format_capabilities_reports_emission() {
    let capabilities = CapabilitySet {
        emission: true,
        ..CapabilitySet::default()
    };

    assert!(format_capabilities(&capabilities).contains("emission=true"));
}

#[test]
fn format_state_readback_capability_covers_readable_and_none() {
    assert_eq!(
        format_state_readback_capability(&StateReadbackCapability::None),
        "none"
    );

    let readable = StateReadbackCapability::Readable {
        facets: vec![ReadableFacet {
            facet: StateFacetKind::Brightness,
            fidelity: ReadbackFidelity::Exact,
        }],
        read_disturbs_output: true,
        notifies_external_changes: false,
    };
    let formatted = format_state_readback_capability(&readable);
    assert!(formatted.contains("Brightness"));
    assert!(formatted.contains("disturbs=true"));
    assert!(formatted.contains("notifies=false"));
}

#[test]
fn format_group_member_covers_every_variant() {
    use luminate_core::element::ElementId;
    use luminate_core::group::GroupId;
    use luminate_core::surface::SurfaceId;

    assert_eq!(
        format_group_member(&GroupMember::Surface(SurfaceId::new("zones"))),
        "surface:zones"
    );
    assert_eq!(
        format_group_member(&GroupMember::Element {
            surface: SurfaceId::new("zones"),
            element: ElementId::new("zone-1"),
        }),
        "element:zones/zone-1"
    );
    assert_eq!(
        format_group_member(&GroupMember::Group(GroupId::new("wasd"))),
        "group:wasd"
    );
}

#[test]
fn terminal_safe_passes_ordinary_text_through() {
    // Printable ASCII and non-ASCII UTF-8 must be untouched.
    assert_eq!(terminal_safe("Keyboard 1"), "Keyboard 1");
    assert_eq!(terminal_safe("Ünïcödé ✨"), "Ünïcödé ✨");
}

#[test]
fn terminal_safe_escapes_control_and_escape_sequences() {
    // A hostile device name embedding an ESC-based colour sequence must
    // not reach the terminal as live control bytes.
    let hostile = "name\u{1b}[31mRED\u{1b}[0m";
    let escaped = terminal_safe(hostile);
    assert!(
        !escaped.contains('\u{1b}'),
        "ESC must be escaped: {escaped:?}"
    );
    assert!(escaped.contains("\\u{1b}"));

    // C0 (newline/tab), DEL, and C1 controls are all escaped.
    assert_eq!(terminal_safe("a\nb\tc"), "a\\nb\\tc");
    assert_eq!(terminal_safe("x\u{7f}y"), "x\\u{7f}y");
    assert_eq!(terminal_safe("x\u{9b}y"), "x\\u{9b}y");
}

#[test]
fn terminal_json_escapes_controls_without_changing_values() {
    let value = serde_json::json!({
        "name": "hostile\n\u{1b}]0;title\u{7}\u{9b}31m\u{7f}",
    });
    let rendered = terminal_json_pretty(&value).expect("serialize terminal-safe JSON");

    assert!(!rendered.contains('\u{1b}'));
    assert!(!rendered.contains('\u{9b}'));
    assert!(!rendered.contains('\u{7f}'));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&rendered).expect("parse terminal-safe JSON"),
        value
    );
}

#[test]
fn capability_strings_escape_plugin_control_characters() {
    let capabilities = CapabilitySet {
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![HardwareEffectDescriptor {
                id: HardwareEffectId::new("off\u{1b}]0;pwned\u{7}"),
                name: "Hostile effect".to_owned(),
                parameters: vec![EffectParameter::Choice {
                    options: vec![EffectChoice {
                        id: "scene\u{1b}[2J".to_owned(),
                        name: "Hostile scene".to_owned(),
                    }],
                }],
            }],
            scope: CapabilityScope::Device,
            concurrent_with_streaming: false,
        }),
        ..CapabilitySet::default()
    };

    let formatted = format_capabilities(&capabilities);

    assert!(!formatted.chars().any(char::is_control));
    assert!(formatted.contains("off\\u{1b}]0;pwned\\u{7}"));
    assert!(formatted.contains("scene\\u{1b}[2J"));
}
