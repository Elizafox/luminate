// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Argument parsing and effect-construction tests.

use clap::CommandFactory as _;

use super::*;

fn build_colour(
    rgb: Option<&str>,
    intensity: Option<u32>,
    kelvin: Option<u32>,
) -> anyhow::Result<Colour> {
    let rgb = rgb.map(parse_css_colour).transpose()?;
    if usize::from(rgb.is_some()) + usize::from(intensity.is_some()) + usize::from(kelvin.is_some())
        != 1
    {
        bail!("choose exactly one static colour form");
    }
    parse_static_colour(rgb, None, None, kelvin, intensity, &[])
}

fn build_effect(
    kind: EffectKindArg,
    rgb: Option<&str>,
    extra_rgb: &[String],
    period_ms: Option<u32>,
) -> anyhow::Result<Effect> {
    build_effect_with_static(
        kind,
        EffectBuildInput {
            rgb,
            hsv: None,
            hsl: None,
            kelvin: None,
            intensity: None,
            additive_channels: &[],
            extra_rgb,
            period_ms,
        },
    )
}

#[test]
fn ping_is_a_parameter_free_command() {
    let cli = Cli::try_parse_from(["luminatectl", "ping"]).expect("parse ping");
    assert!(matches!(cli.command, Command::Ping));
}

#[test]
fn set_slots_parses_repeated_assignments_for_one_surface() {
    let cli = Cli::try_parse_from([
        "luminatectl",
        "set-slots",
        "--device",
        "alienware-aw-elc",
        "--surface",
        "power-button",
        "--slot",
        "ac=static:#0080ff",
        "--slot",
        "battery=off",
    ])
    .expect("parse set-slots command");
    let Command::SetSlots(command) = cli.command else {
        panic!("expected set-slots command");
    };
    let (target, values) = command.into_parts().expect("build slot mutation");
    assert_eq!(
        target,
        TargetId::surface("alienware-aw-elc", "power-button")
    );
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].slot.as_str(), "ac");
    assert_eq!(values[1].effect, Effect::Off);
}

#[test]
fn plugin_setup_discovery_and_execution_parse() {
    let discovery = Cli::try_parse_from([
        "luminatectl",
        "plugin",
        "setup",
        "luminate-plugin-philips-hue",
        "--json",
    ])
    .expect("parse setup discovery");
    assert!(matches!(
        discovery.command,
        Command::Plugin(PluginCommand {
            action: PluginAction::Setup {
                name,
                workflow: None,
                json: true,
            },
        }) if name == "luminate-plugin-philips-hue"
    ));

    let execution = Cli::try_parse_from([
        "luminatectl",
        "plugin",
        "setup",
        "luminate-plugin-philips-hue",
        "push-link",
    ])
    .expect("parse setup execution");
    assert!(matches!(
        execution.command,
        Command::Plugin(PluginCommand {
            action: PluginAction::Setup {
                name,
                workflow: Some(workflow),
                json: false,
            },
        }) if name == "luminate-plugin-philips-hue" && workflow == "push-link"
    ));
}

fn target() -> TargetSelector {
    TargetSelector {
        device: Some("device".to_owned()),
        collection: None,
        surface: None,
        element: None,
        key: None,
        group: None,
    }
}

#[test]
fn target_conversion_rejects_invalid_combinations() {
    let mut collection_only = target();
    collection_only.device = None;
    collection_only.collection = Some("living-room".to_owned());
    assert!(collection_only.into_target().is_err());

    let mut missing_device = target();
    missing_device.device = None;
    assert!(missing_device.into_target().is_err());

    let mut duplicate_element = target();
    duplicate_element.element = Some("led".to_owned());
    duplicate_element.key = Some("escape".to_owned());
    assert!(duplicate_element.into_target().is_err());

    let mut group_and_surface = target();
    group_and_surface.surface = Some("keys".to_owned());
    group_and_surface.group = Some("gaming".to_owned());
    assert!(group_and_surface.into_target().is_err());
}

#[test]
fn colour_builder_covers_supported_forms_and_errors() {
    assert_eq!(
        build_colour(Some("#123"), None, None).expect("valid CSS colour"),
        Colour::rgb(Rgb::new(17, 34, 51))
    );
    assert_eq!(
        build_colour(None, Some(42), None).expect("monochrome colour"),
        Colour::monochrome(42)
    );
    assert_eq!(
        build_colour(None, None, Some(2700)).expect("CCT colour"),
        Colour::cct(2700)
    );
    assert!(build_colour(None, None, None).is_err());
    assert!(build_colour(Some("red"), Some(1), None).is_err());
    assert!(parse_css_colour("definitely-not-a-colour").is_err());

    assert!(parse_static_colour(None, Some("120, 50, 75"), None, None, None, &[]).is_ok());
    assert!(parse_static_colour(None, None, Some("240, 60, 40"), None, None, &[]).is_ok());
    let additive = [
        "red=1",
        "green=2",
        "blue=3",
        "white=4",
        "warm-white=5",
        "cool-white=6",
        "amber=7",
        "uv=8",
    ]
    .map(str::to_owned);
    assert!(parse_static_colour(None, None, None, None, None, &additive).is_ok());
    assert!(parse_u32_triplet("--hsv", "1,2").is_err());
}

#[test]
fn effect_builder_covers_portable_variants() {
    let cases = [
        (EffectKindArg::Static, Some("red"), None),
        (EffectKindArg::Breathe, Some("red"), Some(100)),
        (EffectKindArg::Breathing, Some("red"), Some(100)),
        (EffectKindArg::Pulse, Some("red"), Some(100)),
        (EffectKindArg::Strobe, Some("red"), Some(100)),
        (EffectKindArg::Scanner, Some("red"), Some(100)),
        (EffectKindArg::Spectrum, None, Some(100)),
        (EffectKindArg::Rainbow, None, Some(100)),
    ];
    for (kind, rgb, period) in cases {
        assert!(build_effect(kind, rgb, &[], period).is_ok());
    }
    assert!(build_effect(EffectKindArg::Off, None, &[], None).is_ok());
    assert!(
        build_effect(
            EffectKindArg::Morph,
            Some("red"),
            &["blue".to_owned()],
            Some(100),
        )
        .is_ok()
    );
}

#[test]
fn effect_builder_rejects_missing_and_extraneous_arguments() {
    assert!(build_effect(EffectKindArg::Static, None, &[], None).is_err());
    assert!(build_effect(EffectKindArg::Breathe, Some("red"), &[], None).is_err());
    assert!(build_effect(EffectKindArg::Morph, Some("red"), &[], Some(100)).is_err());
    assert!(build_effect(EffectKindArg::Off, Some("red"), &[], None).is_err());
    assert!(
        build_effect(
            EffectKindArg::Spectrum,
            None,
            &["blue".to_owned()],
            Some(100),
        )
        .is_err()
    );
}

#[test]
fn long_help_documents_targeting_effects_and_persistent_all_off() {
    let mut help = Vec::new();
    Cli::command()
        .write_long_help(&mut help)
        .expect("render long help");
    let help = String::from_utf8(help).expect("help is UTF-8");

    for contract in [
        "--collection COLLECTION is exclusive",
        "--key KEY is element shorthand",
        "--effect names the effect",
        "otherwise it is treated",
        "required persistent targets are reported",
    ] {
        assert!(help.contains(contract), "help omitted contract: {contract}");
    }
}

#[test]
fn documented_target_and_effect_examples_parse() {
    for arguments in [
        vec![
            "luminate",
            "inspect",
            "--device",
            "demo-keyboard",
            "--surface",
            "keys",
            "--key",
            "escape",
        ],
        vec![
            "luminate",
            "set-effect",
            "--collection",
            "living-room",
            "--effect",
            "static",
            "--rgb",
            "rgb(255, 200, 120)",
        ],
        vec![
            "luminate",
            "set-effect",
            "--device",
            "demo-bulb",
            "--effect",
            "breathe",
            "--rgb",
            "#0050ff",
            "--period-ms",
            "1500",
        ],
        vec![
            "luminate",
            "set-effect",
            "--device",
            "demo-bulb",
            "--effect",
            "demo-scene-show",
            "--choice",
            "sunset",
        ],
        vec!["luminate", "purge-withdrawn", "--device", "retired-device"],
    ] {
        Cli::try_parse_from(arguments).expect("documented command should parse");
    }
}

#[test]
fn typed_list_views_and_bare_list_parse() {
    for arguments in [
        vec!["luminate", "list"],
        vec!["luminate", "list", "device"],
        vec!["luminate", "list", "surface"],
        vec!["luminate", "list", "element"],
        vec!["luminate", "list", "group"],
        vec!["luminate", "list", "collection"],
    ] {
        Cli::try_parse_from(arguments).expect("list command should parse");
    }
}

#[test]
fn typed_list_shared_flags_parse_before_and_after_the_view() {
    for arguments in [
        vec![
            "luminate",
            "list",
            "--json",
            "--device",
            "keyboard",
            "--category",
            "keyboard",
            "element",
        ],
        vec![
            "luminate",
            "list",
            "element",
            "--json",
            "--device",
            "keyboard",
            "--category",
            "keyboard",
        ],
    ] {
        Cli::try_parse_from(arguments).expect("global list flags should parse");
    }
}

#[test]
fn typed_list_plural_names_are_rejected() {
    for view in ["devices", "surfaces", "elements", "groups", "collections"] {
        assert!(Cli::try_parse_from(["luminate", "list", view]).is_err());
    }
}

#[test]
fn management_commands_parse() {
    for arguments in [
        vec!["luminate", "plugin", "list"],
        vec!["luminate", "plugin", "list", "--json"],
        vec!["luminate", "plugin", "show", "lifx"],
        vec!["luminate", "plugin", "show", "lifx", "--json"],
        vec!["luminate", "plugin", "enable", "lifx", "--revision", "4"],
        vec![
            "luminate",
            "plugin",
            "disable",
            "lifx",
            "--revision",
            "4",
            "--json",
        ],
        vec!["luminate", "plugin", "reset", "lifx", "--revision", "4"],
        vec![
            "luminate",
            "config",
            "set",
            "lifx",
            "discovery_address",
            "--revision",
            "4",
        ],
        vec![
            "luminate",
            "config",
            "clear",
            "lifx",
            "discovery_address",
            "--revision",
            "4",
            "--json",
        ],
        vec![
            "luminate",
            "config",
            "daemon",
            "set",
            "prefer-shm",
            "true",
            "--revision",
            "4",
        ],
        vec![
            "luminate",
            "config",
            "daemon",
            "set",
            "device-reconciliation",
            "adopt",
            "--device",
            "desk",
            "--revision",
            "4",
        ],
        vec![
            "luminate",
            "config",
            "set-reconciliation",
            "lifx",
            "restore",
            "--revision",
            "4",
            "--json",
        ],
        vec![
            "luminate",
            "config",
            "clear-reconciliation",
            "lifx",
            "--revision",
            "4",
        ],
    ] {
        Cli::try_parse_from(arguments).expect("management command should parse");
    }
}

#[test]
fn management_mutations_require_a_revision() {
    for arguments in [
        vec!["luminate", "plugin", "enable", "lifx"],
        vec!["luminate", "plugin", "disable", "lifx"],
        vec!["luminate", "plugin", "reset", "lifx"],
        vec!["luminate", "config", "set", "lifx", "discovery_address"],
        vec!["luminate", "config", "clear", "lifx", "discovery_address"],
        vec![
            "luminate",
            "config",
            "set-reconciliation",
            "lifx",
            "restore",
        ],
        vec!["luminate", "config", "clear-reconciliation", "lifx"],
        vec!["luminate", "config", "daemon", "set", "prefer-shm", "true"],
        vec!["luminate", "config", "daemon", "clear", "cct-emulation"],
    ] {
        assert!(Cli::try_parse_from(arguments).is_err());
    }
}

#[test]
fn scoped_daemon_preferences_require_their_scope() {
    let arguments = vec![
        "luminate",
        "config",
        "daemon",
        "set",
        "device-reconciliation",
        "adopt",
        "--revision",
        "4",
    ];
    assert!(Cli::try_parse_from(arguments).is_err());
}

#[test]
fn selectors_and_effect_families_are_mutually_exclusive() {
    for arguments in [
        vec![
            "luminate",
            "inspect",
            "--collection",
            "living-room",
            "--device",
            "demo",
        ],
        vec![
            "luminate",
            "set-effect",
            "--device",
            "demo",
            "--effect",
            "off",
            "--kind",
            "typed",
        ],
    ] {
        assert!(Cli::try_parse_from(arguments).is_err());
    }
}

#[test]
fn key_shorthand_defaults_to_keyboard_surface() {
    let target = TargetSelector {
        device: Some("demo".to_owned()),
        collection: None,
        surface: None,
        element: None,
        key: Some("escape".to_owned()),
        group: None,
    }
    .into_target()
    .expect("key shorthand should build a target");

    assert_eq!(target, TargetId::element("demo", "keyboard", "escape"));
}

#[test]
fn key_shorthand_allows_surface_override() {
    let target = TargetSelector {
        device: Some("demo".to_owned()),
        collection: None,
        surface: Some("keys".to_owned()),
        element: None,
        key: Some("escape".to_owned()),
        group: None,
    }
    .into_target()
    .expect("key shorthand should build a target");

    assert_eq!(target, TargetId::element("demo", "keys", "escape"));
}

#[test]
fn set_effect_builds_hardware_invocation_with_arguments() {
    let command = SetEffectCommand {
        target: TargetSelector {
            device: Some("alienware-keyboard".to_owned()),
            collection: None,
            surface: None,
            element: None,
            key: None,
            group: None,
        },
        effect: "aw-sweeper".to_owned(),
        kind: None,
        rgb: Some("#ff0000".to_owned()),
        hsv: None,
        hsl: None,
        kelvin: None,
        intensity: None,
        additive_channel: Vec::new(),
        extra_rgb: vec!["#0000ff".to_owned()],
        period_ms: None,
        speed: Some(4),
        direction: Some(DirectionArg::CounterClockwise),
        duration_ms: Some(1500),
        brightness: Some(200),
        choice: Some("aurora".to_owned()),
        reject: false,
    };

    let (selector, effect, policy) = command
        .into_parts()
        .expect("hardware invocation should build");

    assert_eq!(
        selector,
        Selector::Target(TargetId::device("alienware-keyboard"))
    );
    assert_eq!(policy, None);
    let Effect::Hardware { id, arguments } = effect else {
        panic!("expected a hardware effect, got {effect:?}");
    };
    assert_eq!(id.as_str(), "aw-sweeper");
    assert_eq!(
        arguments.colours,
        vec![Rgb::new(255, 0, 0), Rgb::new(0, 0, 255)]
    );
    assert_eq!(arguments.speed, Some(4));
    assert_eq!(arguments.direction, Some(EffectDirection::CounterClockwise));
    assert_eq!(arguments.duration_ms, Some(1500));
    assert_eq!(arguments.brightness, Some(200));
    assert_eq!(arguments.choice.as_deref(), Some("aurora"));
}

#[test]
fn static_effect_collection_builds_collection_selector() {
    let command = SetEffectCommand {
        target: TargetSelector {
            device: None,
            collection: Some("living-room".to_owned()),
            surface: None,
            element: None,
            key: None,
            group: None,
        },
        effect: "static".to_owned(),
        kind: None,
        rgb: Some("rgb(1, 2, 3)".to_owned()),
        hsv: None,
        hsl: None,
        intensity: None,
        kelvin: None,
        additive_channel: Vec::new(),
        extra_rgb: Vec::new(),
        period_ms: None,
        speed: None,
        direction: None,
        duration_ms: None,
        brightness: None,
        choice: None,
        reject: true,
    };

    let (selector, effect, policy) = command.into_parts().expect("collection selector");
    assert_eq!(
        selector,
        Selector::Collection(CollectionId::new("living-room"))
    );
    assert_eq!(
        effect,
        Effect::Static {
            colour: Colour::rgb(Rgb::new(1, 2, 3))
        }
    );
    assert_eq!(policy, Some(UnsupportedPolicy::Reject));
}

#[test]
fn set_effect_collection_builds_collection_selector() {
    let command = SetEffectCommand {
        target: TargetSelector {
            device: None,
            collection: Some("host-devices".to_owned()),
            surface: None,
            element: None,
            key: None,
            group: None,
        },
        effect: "off".to_owned(),
        kind: None,
        rgb: None,
        hsv: None,
        hsl: None,
        kelvin: None,
        intensity: None,
        additive_channel: Vec::new(),
        extra_rgb: Vec::new(),
        period_ms: None,
        speed: None,
        direction: None,
        duration_ms: None,
        brightness: None,
        choice: None,
        reject: false,
    };

    let (selector, effect, policy) = command.into_parts().expect("collection selector");
    assert_eq!(
        selector,
        Selector::Collection(CollectionId::new("host-devices"))
    );
    assert!(matches!(effect, Effect::Off));
    assert_eq!(policy, None);
}

#[test]
fn set_effect_rejects_hardware_arguments_on_typed_kind() {
    let command = SetEffectCommand {
        target: TargetSelector {
            device: Some("demo".to_owned()),
            collection: None,
            surface: None,
            element: None,
            key: None,
            group: None,
        },
        effect: "static".to_owned(),
        kind: None,
        rgb: Some("#ff0000".to_owned()),
        hsv: None,
        hsl: None,
        kelvin: None,
        intensity: None,
        additive_channel: Vec::new(),
        extra_rgb: Vec::new(),
        period_ms: None,
        speed: Some(3),
        direction: None,
        duration_ms: None,
        brightness: None,
        choice: None,
        reject: false,
    };

    let error = command
        .into_parts()
        .expect_err("--speed is meaningless on a typed effect");
    assert!(
        error.to_string().contains("speed"),
        "error should name the offending flag: {error}"
    );
}

#[test]
fn set_effect_defaults_to_builtin_when_name_matches() {
    let command = SetEffectCommand {
        target: TargetSelector {
            device: Some("demo".to_owned()),
            collection: None,
            surface: None,
            element: None,
            key: None,
            group: None,
        },
        effect: "off".to_owned(),
        kind: None,
        rgb: None,
        hsv: None,
        hsl: None,
        kelvin: None,
        intensity: None,
        additive_channel: Vec::new(),
        extra_rgb: Vec::new(),
        period_ms: None,
        speed: None,
        direction: None,
        duration_ms: None,
        brightness: None,
        choice: None,
        reject: false,
    };

    let (_, effect, _) = command
        .into_parts()
        .expect("known built-in name should resolve without --kind");
    assert!(matches!(effect, Effect::Off));
}

#[test]
fn set_effect_falls_back_to_hardware_when_name_is_unknown() {
    let command = SetEffectCommand {
        target: TargetSelector {
            device: Some("demo".to_owned()),
            collection: None,
            surface: None,
            element: None,
            key: None,
            group: None,
        },
        effect: "aw-sweeper".to_owned(),
        kind: None,
        rgb: None,
        hsv: None,
        hsl: None,
        kelvin: None,
        intensity: None,
        additive_channel: Vec::new(),
        extra_rgb: Vec::new(),
        period_ms: None,
        speed: None,
        direction: None,
        duration_ms: None,
        brightness: None,
        choice: None,
        reject: false,
    };

    let (_, effect, _) = command
        .into_parts()
        .expect("unrecognized name should fall back to a hardware effect");
    let Effect::Hardware { id, .. } = effect else {
        panic!("expected a hardware effect, got {effect:?}");
    };
    assert_eq!(id.as_str(), "aw-sweeper");
}

#[test]
fn set_effect_kind_builtin_rejects_unknown_name() {
    let command = SetEffectCommand {
        target: TargetSelector {
            device: Some("demo".to_owned()),
            collection: None,
            surface: None,
            element: None,
            key: None,
            group: None,
        },
        effect: "aw-sweeper".to_owned(),
        kind: Some(EffectKindSelector::Builtin),
        rgb: None,
        hsv: None,
        hsl: None,
        kelvin: None,
        intensity: None,
        additive_channel: Vec::new(),
        extra_rgb: Vec::new(),
        period_ms: None,
        speed: None,
        direction: None,
        duration_ms: None,
        brightness: None,
        choice: None,
        reject: false,
    };

    assert!(
        command.into_parts().is_err(),
        "--kind builtin with an unrecognized name should fail"
    );
}

#[test]
fn set_effect_kind_hardware_forces_hardware_even_for_builtin_name() {
    let command = SetEffectCommand {
        target: TargetSelector {
            device: Some("demo".to_owned()),
            collection: None,
            surface: None,
            element: None,
            key: None,
            group: None,
        },
        effect: "off".to_owned(),
        kind: Some(EffectKindSelector::Hardware),
        rgb: None,
        hsv: None,
        hsl: None,
        kelvin: None,
        intensity: None,
        additive_channel: Vec::new(),
        extra_rgb: Vec::new(),
        period_ms: None,
        speed: None,
        direction: None,
        duration_ms: None,
        brightness: None,
        choice: None,
        reject: false,
    };

    let (_, effect, _) = command
        .into_parts()
        .expect("--kind hardware should force the hardware interpretation");
    let Effect::Hardware { id, .. } = effect else {
        panic!("expected a hardware effect, got {effect:?}");
    };
    assert_eq!(id.as_str(), "off");
}
