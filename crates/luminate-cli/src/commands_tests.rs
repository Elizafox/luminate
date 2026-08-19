// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Command selection, summaries, and all-off behaviour tests.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::slice;

use super::*;
use luminate_core::capability::{
    CapabilitySet, ColourCapability, PersistenceCapability, PersistenceRequirement,
};
use luminate_core::collection::OwnerIdentity;
use luminate_core::colour::Colour;
use luminate_core::device::DeviceCategory;
use luminate_core::element::{Element, ElementId, ElementKind};
use luminate_core::group::GroupId;
use luminate_core::group::{Group, GroupKind, GroupMember};
use luminate_core::rgb::Rgb;
use luminate_core::state::{
    AdoptionStatus, AppearanceState, DeviceStateStatus, FacetObservation, FacetValue,
    ObservationConfidence, ObservationSource, Reachability, ReconciliationStatus, StateFacetKind,
};
use luminate_core::surface::{Surface, SurfaceId, SurfaceKind};
use serde_json::json;

fn setup_session(state: PluginSetupSessionState) -> PluginSetupSession {
    PluginSetupSession {
        id: luminate::PluginSetupSessionId::parse("0123456789abcdef0123456789abcdef")
            .expect("valid session ID"),
        plugin: "example".to_owned(),
        workflow: "pair".to_owned(),
        generation: 3,
        state,
    }
}

#[test]
fn setup_workflow_json_has_stable_lowercase_vocabulary() {
    let workflow = PluginSetupWorkflow::new(
        "example",
        "pair",
        "Pair",
        "Pair a device.",
        PluginSetupWorkflowKind::FactoryProvision,
    );
    assert_eq!(
        setup_workflow_json(&workflow),
        json!({
            "plugin": "example",
            "id": "pair",
            "label": "Pair",
            "description": "Pair a device.",
            "kind": "factory_provision",
        })
    );
}

#[test]
fn setup_session_json_projects_interactions_and_terminal_state() {
    let choice = setup_session(PluginSetupSessionState::Choice {
        prompt: "Choose one".to_owned(),
        choices: vec![luminate::PluginSetupChoice {
            id: "first".to_owned(),
            label: "First".to_owned(),
            description: None,
        }],
    });
    assert_eq!(
        setup_session_json(&choice),
        json!({
            "session": "0123456789abcdef0123456789abcdef",
            "plugin": "example",
            "workflow": "pair",
            "generation": 3,
            "status": {
                "state": "choice",
                "prompt": "Choose one",
                "choices": [{"id": "first", "label": "First", "description": null}],
            },
        })
    );

    let completed = setup_session(PluginSetupSessionState::Completed {
        summary: "Paired".to_owned(),
        revision: 9,
    });
    assert_eq!(
        setup_session_json(&completed)["status"],
        json!({"state": "completed", "summary": "Paired", "revision": 9})
    );
}

#[test]
fn setup_input_accepts_numbers_ids_and_typed_json() {
    let choices = vec![luminate::PluginSetupChoice {
        id: "bridge-a".to_owned(),
        label: "Bridge A".to_owned(),
        description: None,
    }];
    assert_eq!(
        read_interactive_choice(&mut Cursor::new("1\n"), &choices).expect("numeric choice"),
        PluginSetupInteractionResponse::Choice("bridge-a".to_owned())
    );
    assert_eq!(
        read_interactive_choice(&mut Cursor::new("bridge-a\n"), &choices).expect("ID choice"),
        PluginSetupInteractionResponse::Choice("bridge-a".to_owned())
    );
    assert_eq!(
        read_json_setup_response(
            &mut Cursor::new("{\"response\":\"choice\",\"choice\":\"bridge-a\"}\n"),
            &PluginSetupSessionState::Choice {
                prompt: String::new(),
                choices,
            },
        )
        .expect("JSON choice"),
        PluginSetupInteractionResponse::Choice("bridge-a".to_owned())
    );
    assert_eq!(
        read_json_setup_response(
            &mut Cursor::new("{\"response\":\"confirmed\"}\n"),
            &PluginSetupSessionState::PhysicalAction {
                instruction: String::new(),
            },
        )
        .expect("JSON confirmation"),
        PluginSetupInteractionResponse::Confirmed
    );
}

#[test]
fn setup_input_rejects_eof_invalid_choices_and_mismatched_json() {
    let choices = vec![luminate::PluginSetupChoice {
        id: "bridge-a".to_owned(),
        label: "Bridge A".to_owned(),
        description: None,
    }];
    assert!(read_interactive_choice(&mut Cursor::new(""), &choices).is_err());
    assert!(read_interactive_choice(&mut Cursor::new("2\n"), &choices).is_err());
    assert!(
        read_json_setup_response(
            &mut Cursor::new("{\"response\":\"confirmed\"}\n"),
            &PluginSetupSessionState::Choice {
                prompt: String::new(),
                choices,
            },
        )
        .is_err()
    );
}

fn test_capabilities(required_persistence: bool) -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        persistence: if required_persistence {
            PersistenceCapability::CurrentState {
                requirement: PersistenceRequirement::Required,
                explicit_commit: false,
                readback: false,
            }
        } else {
            PersistenceCapability::None
        },
        ..CapabilitySet::default()
    }
}

fn test_surface(id: &str, capabilities: CapabilitySet, element_ids: &[&str]) -> Surface {
    Surface {
        id: SurfaceId::new(id),
        name: id.to_owned(),
        kind: SurfaceKind::Zone,
        physical_tags: Vec::new(),
        elements: element_ids
            .iter()
            .map(|element| Element {
                id: ElementId::new(*element),
                name: None,
                kind: ElementKind::Led,
                geometry: None,
                physical_tags: Vec::new(),
                capabilities: test_capabilities(false),
                notes: Vec::new(),
                warnings: Vec::new(),
            })
            .collect(),
        capabilities,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

fn test_device(surfaces: Vec<Surface>) -> Device {
    Device {
        id: DeviceId::new("controller"),
        name: "Controller".to_owned(),
        vendor: None,
        model: None,
        provider_instance: None,
        surfaces,
        groups: Vec::new(),
        capabilities: CapabilitySet::default(),
        category: None,
        physical_tags: Vec::new(),
        host_attached: false,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

#[test]
fn all_off_uses_safe_surfaces_and_skips_required_persistence() {
    let device = test_device(vec![
        test_surface("ring", test_capabilities(false), &["led"]),
        test_surface("logo", test_capabilities(false), &["led"]),
        test_surface("power-button", test_capabilities(true), &["led"]),
    ]);

    assert_eq!(
        all_off_plan(&device).targets(),
        [
            TargetId::surface("controller", "ring"),
            TargetId::surface("controller", "logo"),
        ]
    );
    assert_eq!(
        all_off_plan(&device).skipped_persistent(),
        [TargetId::surface("controller", "power-button")]
    );
}

#[test]
fn all_off_falls_back_to_elements_when_surface_is_not_drivable() {
    let device = test_device(vec![test_surface(
        "zones",
        CapabilitySet::default(),
        &["left", "right"],
    )]);

    assert_eq!(
        all_off_plan(&device).targets(),
        vec![
            TargetId::element("controller", "zones", "left"),
            TargetId::element("controller", "zones", "right"),
        ]
    );
}

#[test]
fn sorting_preserves_provider_element_order() {
    let mut devices = vec![test_device(vec![test_surface(
        "zones",
        CapabilitySet::default(),
        &["led-2", "led-10", "led-1"],
    )])];

    sort_devices(&mut devices);

    let ids = devices[0].surfaces[0]
        .elements
        .iter()
        .map(|element| element.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["led-2", "led-10", "led-1"]);
}

#[test]
fn typed_summaries_project_only_discovery_fields() {
    let mut device = test_device(vec![test_surface(
        "zones",
        CapabilitySet::default(),
        &["led"],
    )]);
    device.category = Some(DeviceCategory::new("controller"));
    device.host_attached = true;
    device.groups.push(Group {
        id: GroupId::new("all"),
        name: "All".to_owned(),
        description: Some("not projected".to_owned()),
        kind: GroupKind::Topology,
        members: Vec::new(),
        capabilities: CapabilitySet::default(),
        notes: Vec::new(),
        warnings: Vec::new(),
    });

    assert_eq!(
        device_summaries(slice::from_ref(&device))[0]
            .to_json()
            .expect("serialize device summary"),
        json!({
            "id": "controller",
            "name": "Controller",
            "category": "controller",
            "host_attached": true,
        })
    );
    assert_eq!(
        surface_summaries(slice::from_ref(&device))[0]
            .to_json()
            .expect("serialize surface summary"),
        json!({
            "device": "controller",
            "id": "zones",
            "name": "zones",
            "kind": "Zone",
        })
    );
    assert_eq!(
        element_summaries(slice::from_ref(&device))[0]
            .to_json()
            .expect("serialize element summary"),
        json!({
            "device": "controller",
            "surface": "zones",
            "id": "led",
            "name": null,
            "kind": "Led",
        })
    );
    assert_eq!(
        group_summaries(slice::from_ref(&device))[0]
            .to_json()
            .expect("serialize group summary"),
        json!({
            "device": "controller",
            "id": "all",
            "name": "All",
            "kind": "Topology",
        })
    );
}

#[test]
fn collection_summaries_preserve_optional_kind_as_null() {
    let collections = vec![Collection {
        id: CollectionId::new("living-room"),
        name: "Living Room".to_owned(),
        description: Some("not projected".to_owned()),
        owner: OwnerIdentity::Uid(1000),
        kind: None,
        members: Vec::new(),
    }];

    assert_eq!(
        collection_summaries(&collections)[0]
            .to_json()
            .expect("serialize collection summary"),
        json!({
            "id": "living-room",
            "name": "Living Room",
            "kind": null,
        })
    );
}

#[test]
fn collection_view_rejects_topology_filters() {
    for command in [
        ListCommand {
            view: Some(ListView::Collection),
            json: false,
            device: Some("keyboard".to_owned()),
            category: None,
        },
        ListCommand {
            view: Some(ListView::Collection),
            json: false,
            device: None,
            category: Some("keyboard".to_owned()),
        },
    ] {
        assert!(validate_list_command(&command).is_err());
    }
}

#[test]
fn selector_target_hint_extracts_target_selector() {
    let target = TargetId::surface("controller", "ring");
    let selector = Selector::Target(target.clone());

    assert_eq!(selector_target_hint(&selector), Some(target));
}

#[test]
fn selector_target_hint_is_none_for_collection_selector() {
    let selector = Selector::Collection(CollectionId::new("living-room"));

    assert_eq!(selector_target_hint(&selector), None);
}

#[test]
fn list_filter_matches_device_id_alone() {
    let mut devices = vec![
        Device {
            id: DeviceId::new("keyboard"),
            name: "Keyboard".to_owned(),
            vendor: None,
            model: None,
            provider_instance: None,
            surfaces: Vec::new(),
            groups: Vec::new(),
            capabilities: CapabilitySet::default(),
            category: Some(DeviceCategory::new("keyboard")),
            physical_tags: Vec::new(),
            host_attached: false,
            notes: Vec::new(),
            warnings: Vec::new(),
        },
        Device {
            id: DeviceId::new("mouse"),
            name: "Mouse".to_owned(),
            vendor: None,
            model: None,
            provider_instance: None,
            surfaces: Vec::new(),
            groups: Vec::new(),
            capabilities: CapabilitySet::default(),
            category: Some(DeviceCategory::new("mouse")),
            physical_tags: Vec::new(),
            host_attached: false,
            notes: Vec::new(),
            warnings: Vec::new(),
        },
    ];
    let command = ListCommand {
        view: None,
        json: false,
        device: Some("mouse".to_owned()),
        category: None,
    };

    filter_devices(&mut devices, &command);

    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].id.as_str(), "mouse");
}

#[test]
fn setting_values_parse_from_natural_json() {
    assert_eq!(
        read_setting_value(Cursor::new(
            br#"{"enabled":true,"ports":[80,443],"ratio":0.5}"#
        ))
        .expect("valid setting"),
        SettingValue::Table(BTreeMap::from([
            ("enabled".to_owned(), SettingValue::Boolean(true)),
            (
                "ports".to_owned(),
                SettingValue::Array(vec![SettingValue::Integer(80), SettingValue::Integer(443),]),
            ),
            ("ratio".to_owned(), SettingValue::Number(0.5)),
        ]))
    );
    assert_eq!(
        read_setting_value(Cursor::new(br#""secret""#)).expect("valid string"),
        SettingValue::String("secret".to_owned())
    );
}

#[test]
fn setting_values_reject_null_and_multiple_documents() {
    assert!(read_setting_value(Cursor::new(b"null")).is_err());
    assert!(read_setting_value(Cursor::new(b"true false")).is_err());
}

#[test]
fn setting_values_cover_scalars_and_nested_values() {
    assert_eq!(
        setting_value_from_json(json!(false)).expect("boolean"),
        SettingValue::Boolean(false)
    );
    assert_eq!(
        setting_value_from_json(json!(-4)).expect("integer"),
        SettingValue::Integer(-4)
    );
    assert_eq!(
        setting_value_from_json(json!(["one", {"two": 2}])).expect("nested value"),
        SettingValue::Array(vec![
            SettingValue::String("one".to_owned()),
            SettingValue::Table(BTreeMap::from([(
                "two".to_owned(),
                SettingValue::Integer(2),
            )])),
        ])
    );
    assert!(setting_value_from_json(json!(f64::INFINITY)).is_err());
}

#[test]
fn preference_parsers_accept_every_supported_value() {
    assert_eq!(
        parse_unsupported_policy("skip").expect("skip policy"),
        UnsupportedPolicy::Skip
    );
    assert_eq!(
        parse_unsupported_policy("reject").expect("reject policy"),
        UnsupportedPolicy::Reject
    );
    assert_eq!(
        parse_reconciliation_policy("restore").expect("restore policy"),
        ReconciliationPolicy::Restore
    );
    assert_eq!(
        parse_reconciliation_policy("adopt").expect("adopt policy"),
        ReconciliationPolicy::Adopt
    );
    assert_eq!(
        parse_reconciliation_policy("leave").expect("leave policy"),
        ReconciliationPolicy::Leave
    );
    assert_eq!(
        parse_cct_emulation("auto").expect("auto CCT"),
        CctEmulation::Auto
    );
    assert_eq!(
        parse_cct_emulation("disabled").expect("disabled CCT"),
        CctEmulation::Disabled
    );
    assert!(parse_boolean("true").expect("true"));
    assert!(!parse_boolean("false").expect("false"));
}

#[test]
fn daemon_preferences_set_and_clear_cover_each_preference() {
    let empty_preferences = || DaemonPreferences {
        default_unsupported_policy: None,
        reconciliation_policy: None,
        device_reconciliation: Vec::new(),
        cct_emulation: None,
        prefer_shm: None,
        prefer_client_shm: None,
    };
    let mut preferences = empty_preferences();
    for (preference, value, device) in [
        (DaemonPreference::DefaultUnsupportedPolicy, "reject", None),
        (DaemonPreference::ReconciliationPolicy, "adopt", None),
        (
            DaemonPreference::DeviceReconciliation,
            "leave",
            Some("desk"),
        ),
        (DaemonPreference::CctEmulation, "disabled", None),
        (DaemonPreference::PreferShm, "true", None),
        (DaemonPreference::PreferClientShm, "false", None),
    ] {
        preferences = set_daemon_preference(
            preferences,
            &DaemonPreferenceSetCommand {
                preference,
                value: value.to_owned(),
                device: device.map(str::to_owned),
                revision: 0,
                json: false,
            },
        )
        .expect("supported preference");
    }
    assert_eq!(
        preferences.default_unsupported_policy,
        Some(UnsupportedPolicy::Reject)
    );
    assert_eq!(
        preferences.reconciliation_policy,
        Some(ReconciliationPolicy::Adopt)
    );
    assert_eq!(preferences.device_reconciliation.len(), 1);
    assert_eq!(preferences.cct_emulation, Some(CctEmulation::Disabled));
    assert_eq!(preferences.prefer_shm, Some(true));
    assert_eq!(preferences.prefer_client_shm, Some(false));

    for (preference, device) in [
        (DaemonPreference::DefaultUnsupportedPolicy, None),
        (DaemonPreference::ReconciliationPolicy, None),
        (DaemonPreference::DeviceReconciliation, Some("desk")),
        (DaemonPreference::CctEmulation, None),
        (DaemonPreference::PreferShm, None),
        (DaemonPreference::PreferClientShm, None),
    ] {
        preferences = clear_daemon_preference(
            preferences,
            &DaemonPreferenceClearCommand {
                preference,
                device: device.map(str::to_owned),
                revision: 0,
                json: false,
            },
        )
        .expect("supported clear");
    }
    assert_eq!(preferences, empty_preferences());
}

#[test]
fn daemon_preference_set_preserves_unrelated_values_and_replaces_scoped_entry() {
    let preferences = DaemonPreferences {
        default_unsupported_policy: Some(UnsupportedPolicy::Reject),
        reconciliation_policy: Some(ReconciliationPolicy::Leave),
        device_reconciliation: vec![DeviceReconciliationPreference {
            device: DeviceId::new("desk"),
            policy: ReconciliationPolicy::Restore,
        }],
        cct_emulation: Some(CctEmulation::Disabled),
        prefer_shm: Some(true),
        prefer_client_shm: Some(false),
    };
    let command = DaemonPreferenceSetCommand {
        preference: DaemonPreference::DeviceReconciliation,
        value: "adopt".to_owned(),
        device: Some("desk".to_owned()),
        revision: 4,
        json: false,
    };

    let updated =
        set_daemon_preference(preferences.clone(), &command).expect("valid daemon preference");

    assert_eq!(
        updated.device_reconciliation,
        vec![DeviceReconciliationPreference {
            device: DeviceId::new("desk"),
            policy: ReconciliationPolicy::Adopt,
        }]
    );
    assert_eq!(
        updated.default_unsupported_policy,
        preferences.default_unsupported_policy
    );
    assert_eq!(updated.cct_emulation, preferences.cct_emulation);
    assert_eq!(updated.prefer_shm, preferences.prefer_shm);
    assert_eq!(updated.prefer_client_shm, preferences.prefer_client_shm);
}

#[test]
fn daemon_preferences_reject_invalid_values_and_scopes() {
    let preferences = DaemonPreferences {
        default_unsupported_policy: None,
        reconciliation_policy: None,
        device_reconciliation: Vec::new(),
        cct_emulation: None,
        prefer_shm: None,
        prefer_client_shm: None,
    };
    let invalid_value = DaemonPreferenceSetCommand {
        preference: DaemonPreference::PreferShm,
        value: "yes".to_owned(),
        device: None,
        revision: 0,
        json: false,
    };
    let invalid_scope = DaemonPreferenceClearCommand {
        preference: DaemonPreference::CctEmulation,
        device: Some("desk".to_owned()),
        revision: 0,
        json: false,
    };

    assert!(set_daemon_preference(preferences.clone(), &invalid_value).is_err());
    assert!(clear_daemon_preference(preferences, &invalid_scope).is_err());
}

#[test]
fn list_filter_combines_device_and_category_to_empty() {
    let mut devices = vec![Device {
        id: DeviceId::new("keyboard"),
        name: "Keyboard".to_owned(),
        vendor: None,
        model: None,
        provider_instance: None,
        surfaces: Vec::new(),
        groups: Vec::new(),
        capabilities: CapabilitySet::default(),
        category: Some(DeviceCategory::new("keyboard")),
        physical_tags: Vec::new(),
        host_attached: false,
        notes: Vec::new(),
        warnings: Vec::new(),
    }];
    let command = ListCommand {
        view: None,
        json: false,
        device: Some("keyboard".to_owned()),
        category: Some("mouse".to_owned()),
    };

    filter_devices(&mut devices, &command);

    assert!(devices.is_empty());
}

#[test]
fn filter_devices_drops_devices_without_a_category() {
    let mut devices = vec![Device {
        id: DeviceId::new("controller"),
        name: "Controller".to_owned(),
        vendor: None,
        model: None,
        provider_instance: None,
        surfaces: Vec::new(),
        groups: Vec::new(),
        capabilities: CapabilitySet::default(),
        category: None,
        physical_tags: Vec::new(),
        host_attached: false,
        notes: Vec::new(),
        warnings: Vec::new(),
    }];
    let command = ListCommand {
        view: None,
        json: false,
        device: None,
        category: Some("keyboard".to_owned()),
    };

    filter_devices(&mut devices, &command);

    assert!(devices.is_empty());
}

#[test]
fn sorting_orders_groups_and_group_members() {
    let mut device = test_device(vec![test_surface(
        "zones",
        CapabilitySet::default(),
        &["led"],
    )]);
    device.groups = vec![
        Group {
            id: GroupId::new("zebra"),
            name: "Zebra".to_owned(),
            description: None,
            kind: GroupKind::Topology,
            members: vec![
                GroupMember::Group(GroupId::new("nested-b")),
                GroupMember::Element {
                    surface: SurfaceId::new("zones"),
                    element: ElementId::new("led"),
                },
                GroupMember::Surface(SurfaceId::new("zones")),
            ],
            capabilities: CapabilitySet::default(),
            notes: Vec::new(),
            warnings: Vec::new(),
        },
        Group {
            id: GroupId::new("alpha"),
            name: "Alpha".to_owned(),
            description: None,
            kind: GroupKind::Topology,
            members: Vec::new(),
            capabilities: CapabilitySet::default(),
            notes: Vec::new(),
            warnings: Vec::new(),
        },
    ];
    let mut devices = vec![device];

    sort_devices(&mut devices);

    let group_ids = devices[0]
        .groups
        .iter()
        .map(|group| group.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(group_ids, vec!["alpha", "zebra"]);

    let member_labels = devices[0].groups[1]
        .members
        .iter()
        .map(format_group_member)
        .collect::<Vec<_>>();
    assert_eq!(
        member_labels,
        vec![
            "element:zones/led".to_owned(),
            "group:nested-b".to_owned(),
            "surface:zones".to_owned(),
        ]
    );
}

#[test]
fn list_filter_matches_device_and_category() {
    let mut devices = vec![
        Device {
            id: DeviceId::new("keyboard"),
            name: "Keyboard".to_owned(),
            vendor: None,
            model: None,
            provider_instance: None,
            surfaces: Vec::new(),
            groups: Vec::new(),
            capabilities: CapabilitySet::default(),
            category: Some(DeviceCategory::new("keyboard")),
            physical_tags: Vec::new(),
            host_attached: false,
            notes: Vec::new(),
            warnings: Vec::new(),
        },
        Device {
            id: DeviceId::new("mouse"),
            name: "Mouse".to_owned(),
            vendor: None,
            model: None,
            provider_instance: None,
            surfaces: Vec::new(),
            groups: Vec::new(),
            capabilities: CapabilitySet::default(),
            category: Some(DeviceCategory::new("mouse")),
            physical_tags: Vec::new(),
            host_attached: false,
            notes: Vec::new(),
            warnings: Vec::new(),
        },
    ];
    let command = ListCommand {
        view: None,
        json: false,
        device: None,
        category: Some("keyboard".to_owned()),
    };

    filter_devices(&mut devices, &command);

    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].id.as_str(), "keyboard");
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the exact full-device JSON contract is clearer as one fixture"
)]
fn list_json_schema_is_pinned() {
    let device = Device {
        id: DeviceId::new("controller"),
        name: "Controller".to_owned(),
        vendor: Some("Example".to_owned()),
        model: Some("One".to_owned()),
        provider_instance: Some("demo".to_owned()),
        surfaces: vec![test_surface("ring", test_capabilities(false), &["led"])],
        groups: vec![Group {
            id: GroupId::new("all"),
            name: "All".to_owned(),
            description: Some("Every light".to_owned()),
            kind: GroupKind::Topology,
            members: vec![GroupMember::Surface(SurfaceId::new("ring"))],
            capabilities: CapabilitySet::default(),
            notes: vec!["group note".to_owned()],
            warnings: vec!["group warning".to_owned()],
        }],
        capabilities: CapabilitySet::default(),
        category: Some(DeviceCategory::new("controller")),
        physical_tags: vec![
            "shape:modular-light-bar".to_owned(),
            "example:faceted".to_owned(),
        ],
        host_attached: false,
        notes: vec!["device note".to_owned()],
        warnings: vec!["device warning".to_owned()],
    };

    let actual = serde_json::to_value([device]).expect("serialize list output");
    let empty_capabilities = json!({
        "colour": [],
        "cct_emulation": "Auto",
        "brightness": "None",
        "frame_upload": null,
        "hardware_effects": null,
        "appearance_slots": null,
        "persistence": "None",
        "state_readback": "None",
        "emission": false,
        "off_is_wear_safe": false,
        "physical_power": null,
        "power_domain": null
    });
    let rgb_capabilities = json!({
        "colour": [{
            "Additive": [
                { "channel": "Red", "bits": 8 },
                { "channel": "Green", "bits": 8 },
                { "channel": "Blue", "bits": 8 }
            ]
        }],
        "cct_emulation": "Auto",
        "brightness": "None",
        "frame_upload": null,
        "hardware_effects": null,
        "appearance_slots": null,
        "persistence": "None",
        "state_readback": "None",
        "emission": false,
        "off_is_wear_safe": false,
        "physical_power": null,
        "power_domain": null
    });
    let expected = json!([{
        "id": "controller",
        "name": "Controller",
        "vendor": "Example",
        "model": "One",
        "provider_instance": "demo",
        "surfaces": [{
            "id": "ring",
            "name": "ring",
            "kind": "Zone",
            "physical_tags": [],
            "elements": [{
                "id": "led",
                "name": null,
                "kind": "Led",
                "geometry": null,
                "physical_tags": [],
                "capabilities": rgb_capabilities,
                "notes": [],
                "warnings": []
            }],
            "capabilities": rgb_capabilities,
            "notes": [],
            "warnings": []
        }],
        "groups": [{
            "id": "all",
            "name": "All",
            "description": "Every light",
            "kind": "Topology",
            "members": [{ "Surface": "ring" }],
            "capabilities": empty_capabilities,
            "notes": ["group note"],
            "warnings": ["group warning"]
        }],
        "capabilities": empty_capabilities,
        "category": "controller",
        "physical_tags": ["shape:modular-light-bar", "example:faceted"],
        "host_attached": false,
        "notes": ["device note"],
        "warnings": ["device warning"]
    }]);

    assert_eq!(actual, expected);
}

#[test]
fn state_json_schema_is_pinned() {
    let target = TargetId::surface("controller", "ring");
    let state = DeviceStateStatus {
        device: DeviceId::new("controller"),
        observations: vec![FacetObservation {
            target: target.clone(),
            value: FacetValue::Appearance(AppearanceState::Static(Colour::rgb(Rgb::new(1, 2, 3)))),
            confidence: ObservationConfidence::Confirmed,
            source: ObservationSource::Readback,
            observed_at_ms: 42,
            stale: false,
        }],
        reachability: Reachability::Reachable,
        reconciliation: ReconciliationStatus::Complete,
        adoption: vec![(target, StateFacetKind::Appearance, AdoptionStatus::Durable)],
        latest_error: Some("example error".to_owned()),
        latest_attempt_ms: Some(43),
    };

    let actual = serde_json::to_value(state).expect("serialize state output");
    let expected = json!({
        "device": "controller",
        "observations": [{
            "target": { "Surface": { "device": "controller", "surface": "ring" } },
            "value": { "Appearance": { "Static": {
                "Additive": [
                    { "channel": "Red", "value": 1 },
                    { "channel": "Green", "value": 2 },
                    { "channel": "Blue", "value": 3 }
                ]
            } } },
            "confidence": "Confirmed",
            "source": "Readback",
            "observed_at_ms": 42,
            "stale": false
        }],
        "reachability": "Reachable",
        "reconciliation": "Complete",
        "adoption": [[
            { "Surface": { "device": "controller", "surface": "ring" } },
            "Appearance",
            "Durable"
        ]],
        "latest_error": "example error",
        "latest_attempt_ms": 43
    });

    assert_eq!(actual, expected);
}
