// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::io::Write as _;
use std::ops::Deref;
#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::PathBuf;

use luminate_platform::secure_storage::create_private_file;
use luminate_platform::test_support::TestDir;

use luminate_core::appearance_slot::{AppearanceSlotId, AppearanceSlotValue};
use luminate_core::collection::{Collection, CollectionId, CollectionMember, OwnerIdentity};
use luminate_core::colour::Colour;
use luminate_core::device::DeviceId;
use luminate_core::effect::Effect;
use luminate_core::policy::PrincipalId;
use luminate_core::rgb::Rgb;
use luminate_core::scene::{SceneBinding, SceneTargetState};
use luminate_core::state::{AdoptedFacet, FacetValue};
use luminate_core::target::TargetId;

use crate::state::target_state::{TargetState, TargetStateEntry};

use super::*;

fn save(path: &Path, target_state: &[TargetStateEntry]) -> Result<()> {
    save_complete(path, target_state, &HashMap::new(), &HashMap::new(), &[])
}

/// Keeps each state file in a private per-test directory.
///
/// This matches the production directory permissions and satisfies `save`'s
/// parent-directory checks.
struct TestStatePath {
    _directory: TestDir,
    path: PathBuf,
}

impl Deref for TestStatePath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<Path> for TestStatePath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

fn unique_state_path(name: &str) -> TestStatePath {
    let directory = TestDir::new(name);
    let path = directory.join("state.json");
    TestStatePath {
        _directory: directory,
        path,
    }
}

fn write_private_fixture(path: &Path, contents: impl AsRef<[u8]>) {
    let mut file = create_private_file(path).expect("create private state fixture");
    file.write_all(contents.as_ref())
        .expect("write private state fixture");
}

fn sample_state() -> Vec<TargetStateEntry> {
    vec![
        TargetStateEntry {
            target: TargetId::Device(DeviceId::new("dev0")),
            state: TargetState::Effect(Effect::Static {
                colour: Colour::rgb(Rgb::new(10, 20, 30)),
            }),
        },
        TargetStateEntry {
            target: TargetId::Device(DeviceId::new("dev1")),
            state: TargetState::Brightness(42),
        },
    ]
}

fn retained_state_path(path: &Path) -> PathBuf {
    let parent = path.parent().expect("state path should have a parent");
    let prefix = format!(
        "{}.",
        path.file_name()
            .and_then(|name| name.to_str())
            .expect("state file name should be UTF-8")
    );
    let retained = fs::read_dir(parent)
        .expect("read state directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".corrupt"))
        })
        .collect::<Vec<_>>();
    assert_eq!(retained.len(), 1, "exactly one sidecar should be retained");
    retained.into_iter().next().expect("retained sidecar")
}

#[test]
fn missing_file_loads_as_empty() {
    let path = unique_state_path("missing");
    let loaded = load(&path).expect("load should not fail for a missing file");
    assert!(loaded.entries.is_empty());
}

#[test]
fn version_eight_colour_migration_covers_every_legacy_encoding() {
    let colour = |encoding: &str, channels: &[(&str, u64)]| {
        serde_json::json!({
            "encoding": encoding,
            "channels": channels.iter().map(|(channel, value)| {
                serde_json::json!({"channel": channel, "value": value})
            }).collect::<Vec<_>>()
        })
    };

    let cases = [
        (colour("Additive", &[("Red", 1)]), "Additive"),
        (
            colour("Hsv", &[("Hue", 1), ("Saturation", 2), ("Value", 3)]),
            "Hsv",
        ),
        (
            colour("Hsl", &[("Hue", 1), ("Saturation", 2), ("Lightness", 3)]),
            "Hsl",
        ),
        (colour("Cct", &[("Temperature", 2_700)]), "Cct"),
        (colour("Monochrome", &[("Intensity", 42)]), "Monochrome"),
    ];
    for (legacy, expected_key) in cases {
        let migrated = migrate_v8_colour(legacy).expect("known legacy colour should migrate");
        assert!(migrated.get(expected_key).is_some());
    }

    let already_current = serde_json::json!({"Cct": {"kelvin": 3000}});
    assert_eq!(
        migrate_v8_colour(already_current.clone()).expect("current colour is unchanged"),
        already_current
    );
    for malformed in [
        serde_json::json!({}),
        serde_json::json!({"encoding": "Hsv"}),
        colour("Additive", &[]),
        colour("Hsv", &[("Hue", 1)]),
        colour("FutureColour", &[("Value", 1)]),
    ] {
        assert!(migrate_v8_colour(malformed).is_err());
    }
}

#[test]
fn version_eight_target_migration_handles_colour_and_static_rgb_shapes() {
    let mut document = serde_json::json!({
        "targets": [
            {
                "state": {
                    "Colour": {
                        "encoding": "Cct",
                        "channels": [{"channel": "Temperature", "value": 3200}]
                    }
                }
            },
            {"state": {"Effect": {"Static": {"colour": {"r": 1, "g": 2, "b": 3}}}}}
        ]
    });
    migrate_v8_colours(&mut document).expect("legacy target colours should migrate");

    assert_eq!(
        document["targets"][0]["state"]["Effect"]["Static"]["colour"]["Cct"]["kelvin"],
        3200
    );
    assert_eq!(
        document["targets"][1]["state"]["Effect"]["Static"]["colour"]["Additive"][2]["value"],
        3
    );

    for mut malformed in [
        serde_json::json!({}),
        serde_json::json!({"targets": [null]}),
        serde_json::json!({"targets": [{"state": {"Effect": {"Static": {"colour": {"r": "bad", "g": 2, "b": 3}}}}}]}),
        serde_json::json!({"targets": [{"state": {"Effect": {"Static": {"colour": {"r": 1, "g": "bad", "b": 3}}}}}]}),
        serde_json::json!({"targets": [{"state": {"Effect": {"Static": {"colour": {"r": 1, "g": 2, "b": "bad"}}}}}]}),
    ] {
        assert!(migrate_v8_colours(&mut malformed).is_err());
    }
}

#[test]
fn save_then_load_round_trips() {
    let path = unique_state_path("roundtrip");

    save(&path, &sample_state()).expect("save should succeed");
    let loaded = load(&path).expect("load should succeed");

    assert!(loaded.preserve_order);
    assert_eq!(loaded.entries.len(), 2);
    let TargetState::Brightness(value) = &loaded.entries[1].state else {
        panic!("unexpected state for dev1: {:?}", loaded.entries[1].state);
    };
    assert_eq!(*value, 42);

    let _ = fs::remove_file(&path);
}

#[test]
fn adopted_baseline_round_trips_separately_from_desired_state() {
    let path = unique_state_path("adopted");
    let adopted = vec![AdoptedFacet {
        target: TargetId::device("dev0"),
        value: FacetValue::Brightness(77),
        confirmed_at_ms: 1234,
    }];
    save_complete(
        &path,
        &sample_state(),
        &HashMap::new(),
        &HashMap::new(),
        &adopted,
    )
    .expect("save adopted baseline");
    let loaded = load(&path).expect("load adopted baseline");
    assert_eq!(loaded.entries.len(), 2);
    assert_eq!(loaded.adopted_baseline.len(), 1);
    assert_eq!(loaded.adopted_baseline[0].value, FacetValue::Brightness(77));
    assert_eq!(loaded.adopted_baseline[0].confirmed_at_ms, 1234);
    let _ = fs::remove_file(&path);
}

#[test]
fn collections_round_trip() {
    use luminate_core::collection::{
        Collection, CollectionCategory, CollectionId, CollectionMember, OwnerIdentity,
        collection_category,
    };
    use luminate_core::target::TargetId;

    let path = unique_state_path("collections");
    let id = CollectionId::new("living-room");
    let mut collections = HashMap::new();
    collections.insert(
        id.clone(),
        Collection {
            id: id.clone(),
            name: "Living room".to_owned(),
            description: None,
            owner: OwnerIdentity::Principal(
                PrincipalId::new("unix", "1000").expect("valid principal"),
            ),
            kind: Some(CollectionCategory::new(collection_category::LOCATION)),
            members: vec![CollectionMember::Target(TargetId::device("lamp"))],
        },
    );

    save_complete(&path, &sample_state(), &collections, &HashMap::new(), &[])
        .expect("save should succeed");
    let loaded = load(&path).expect("load should succeed");

    assert_eq!(loaded.collections.len(), 1);
    let loaded_collection = loaded
        .collections
        .get(&id)
        .expect("the living-room collection should round-trip");
    assert_eq!(loaded_collection.name, "Living room");
    assert_eq!(
        loaded_collection.owner,
        OwnerIdentity::Principal(PrincipalId::new("unix", "1000").expect("valid principal"))
    );
    assert_eq!(loaded_collection.members.len(), 1);

    let _ = fs::remove_file(&path);
}

fn persisted_collection(id: &str, members: Vec<CollectionMember>) -> (CollectionId, Collection) {
    let id = CollectionId::new(id);
    (
        id.clone(),
        Collection {
            id,
            name: "Collection".to_owned(),
            description: None,
            owner: OwnerIdentity::Principal(
                PrincipalId::new("unix", "1000").expect("valid principal"),
            ),
            kind: None,
            members,
        },
    )
}

#[test]
fn snapshot_validation_rejects_invalid_collection_graphs() {
    let (key, mut mismatched) = persisted_collection("key", Vec::new());
    mismatched.id = CollectionId::new("value");
    let cases = [
        HashMap::from([(key, mismatched)]),
        HashMap::from([persisted_collection(
            "missing",
            vec![CollectionMember::Collection(CollectionId::new("absent"))],
        )]),
        HashMap::from([persisted_collection(
            "self",
            vec![CollectionMember::Collection(CollectionId::new("self"))],
        )]),
        HashMap::from([
            persisted_collection(
                "first",
                vec![CollectionMember::Collection(CollectionId::new("second"))],
            ),
            persisted_collection(
                "second",
                vec![CollectionMember::Collection(CollectionId::new("first"))],
            ),
        ]),
        HashMap::from([persisted_collection(
            "duplicate",
            vec![
                CollectionMember::Target(TargetId::device("lamp")),
                CollectionMember::Target(TargetId::device("lamp")),
            ],
        )]),
    ];

    for collections in cases {
        let state = PersistedState {
            version: PERSISTENCE_VERSION,
            collections,
            ..PersistedState::default()
        };
        assert!(ValidatedPersistedSnapshot::new(state).is_err());
    }
}

#[test]
fn snapshot_validation_rejects_a_scene_reference_to_a_missing_collection() {
    let id = SceneId::generate();
    let state = PersistedState {
        version: PERSISTENCE_VERSION,
        scenes: HashMap::from([(
            id.clone(),
            Scene {
                id,
                revision: 1,
                name: "Missing collection".to_owned(),
                description: None,
                owner: OwnerIdentity::Principal(
                    PrincipalId::new("unix", "1000").expect("valid principal"),
                ),
                bindings: vec![SceneBinding::DynamicCollectionMember {
                    collection: CollectionId::new("absent"),
                    target: TargetId::device("lamp"),
                    state: SceneTargetState {
                        appearance: None,
                        brightness: Some(50),
                        emission: None,
                        appearance_slots: None,
                    },
                }],
            },
        )]),
        ..PersistedState::default()
    };

    let error = ValidatedPersistedSnapshot::new(state)
        .err()
        .expect("missing collection reference must fail");
    assert!(error.to_string().contains("missing collection"));
}

#[test]
fn save_rejects_oversized_candidate_without_replacing_previous_snapshot() {
    let path = unique_state_path("rejected-save-is-transactional");
    save(&path, &sample_state()).expect("save original snapshot");
    let original = fs::read(&path).expect("read original snapshot");

    let (id, mut collection) = persisted_collection("oversized", Vec::new());
    collection.name = "x".repeat(MAX_TEXT_BYTES + 1);
    let error = save_complete(
        &path,
        &sample_state(),
        &HashMap::from([(id, collection)]),
        &HashMap::new(),
        &[],
    )
    .expect_err("oversized collection name must be rejected");

    assert!(error.to_string().contains("collection name"));
    assert_eq!(fs::read(&path).expect("read retained snapshot"), original);
}

#[test]
fn scenes_round_trip_in_current_snapshot() {
    use luminate_core::collection::OwnerIdentity;
    use luminate_core::scene::{Scene, SceneBinding, SceneId, SceneTargetState};
    use luminate_core::state::EmissionState;

    let path = unique_state_path("scenes");
    let id = SceneId::generate();
    let mut scenes = HashMap::new();
    scenes.insert(
        id.clone(),
        Scene {
            id: id.clone(),
            revision: 1,
            name: "Evening".to_owned(),
            description: Some("Warm and low".to_owned()),
            owner: OwnerIdentity::Principal(
                PrincipalId::new("unix", "1000").expect("valid principal"),
            ),
            bindings: vec![SceneBinding::Frozen {
                target: TargetId::device("lamp"),
                state: SceneTargetState {
                    appearance: Some(Effect::Static {
                        colour: Colour::rgb(Rgb::new(30, 15, 5)),
                    }),
                    brightness: Some(20),
                    emission: Some(EmissionState::Emitting),
                    appearance_slots: None,
                },
            }],
        },
    );

    save_complete(&path, &sample_state(), &HashMap::new(), &scenes, &[]).expect("save scenes");
    let loaded = load(&path).expect("load scenes");

    assert_eq!(loaded.scenes, scenes);
    let contents = fs::read_to_string(&path).expect("read state");
    assert!(contents.contains("\"version\": 13"));
    let _ = fs::remove_file(&path);
}

#[test]
fn version_twelve_round_trips_desired_and_scene_slot_values() {
    let path = unique_state_path("v12-appearance-slots");
    let value = AppearanceSlotValue {
        slot: AppearanceSlotId::new("ac"),
        effect: Effect::Off,
    };
    let target = TargetId::surface("alienware-aw-elc", "power-button");
    let entries = vec![TargetStateEntry {
        target: target.clone(),
        state: TargetState::AppearanceSlots(vec![value.clone()]),
    }];
    let id = SceneId::new("00000000-0000-0000-0000-000000000012");
    let scenes = HashMap::from([(
        id.clone(),
        Scene {
            id,
            revision: 1,
            name: "Power".to_owned(),
            description: None,
            owner: OwnerIdentity::Principal(
                PrincipalId::new("unix", "1000").expect("valid principal"),
            ),
            bindings: vec![SceneBinding::Frozen {
                target,
                state: SceneTargetState {
                    appearance: None,
                    brightness: None,
                    emission: None,
                    appearance_slots: Some(vec![value]),
                },
            }],
        },
    )]);

    save_complete(&path, &entries, &HashMap::new(), &scenes, &[]).expect("save slots");
    let loaded = load(&path).expect("load slots");

    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.scenes, scenes);
}

#[test]
fn legacy_alienware_power_button_state_and_scene_pair_migrate_to_slots() {
    let scene_id = SceneId::generate();
    let ac_effect = Effect::Static {
        colour: Colour::rgb(Rgb::new(1, 2, 3)),
    };
    let battery_effect = Effect::Static {
        colour: Colour::rgb(Rgb::new(4, 5, 6)),
    };
    let owner =
        OwnerIdentity::Principal(PrincipalId::new("unix", "1000").expect("valid principal"));
    let binding = |surface: &str, effect: Effect| SceneBinding::Frozen {
        target: TargetId::surface("alienware-aw-elc", surface),
        state: SceneTargetState {
            appearance: Some(effect),
            brightness: None,
            emission: None,
            appearance_slots: None,
        },
    };
    let mut persisted = PersistedState {
        version: 11,
        targets: vec![
            TargetStateEntry {
                target: TargetId::surface("alienware-aw-elc", "power-button-ac"),
                state: TargetState::Effect(ac_effect.clone()),
            },
            TargetStateEntry {
                target: TargetId::surface("alienware-aw-elc", "power-button-battery"),
                state: TargetState::Effect(battery_effect.clone()),
            },
        ],
        scenes: HashMap::from([(
            scene_id.clone(),
            Scene {
                id: scene_id.clone(),
                revision: 1,
                name: "Power button".to_owned(),
                description: None,
                owner,
                bindings: vec![
                    binding("power-button-ac", ac_effect),
                    binding("power-button-battery", battery_effect),
                ],
            },
        )]),
        adopted_baseline: vec![AdoptedFacet {
            target: TargetId::surface("alienware-aw-elc", "power-button-ac"),
            value: FacetValue::Appearance(AppearanceState::Static(Colour::rgb(Rgb::new(1, 2, 3)))),
            confirmed_at_ms: 123,
        }],
        ..PersistedState::default()
    };

    assert!(migrate_alienware_power_button(&mut persisted).expect("migration succeeds"));
    assert!(persisted.targets.iter().all(|entry| {
        entry.target == TargetId::surface("alienware-aw-elc", "power-button")
            && matches!(entry.state, TargetState::AppearanceSlots(_))
    }));
    let scene = persisted.scenes.get(&scene_id).expect("migrated scene");
    assert_eq!(scene.bindings.len(), 1);
    let values = scene.bindings[0]
        .state()
        .appearance_slots
        .as_ref()
        .expect("migrated slots");
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].slot.as_str(), "ac");
    assert_eq!(values[1].slot.as_str(), "battery");
    assert_eq!(
        persisted.adopted_baseline[0].target,
        TargetId::surface("alienware-aw-elc", "power-button")
    );
    assert!(matches!(
        persisted.adopted_baseline[0].value,
        FacetValue::AppearanceSlots(_)
    ));
}

#[test]
fn duplicate_legacy_alienware_scene_binding_is_rejected() {
    let id = SceneId::generate();
    let state = SceneTargetState {
        appearance: Some(Effect::Off),
        brightness: None,
        emission: None,
        appearance_slots: None,
    };
    let mut scene = Scene {
        id,
        revision: 1,
        name: "Duplicate".to_owned(),
        description: None,
        owner: OwnerIdentity::Principal(PrincipalId::new("unix", "1000").expect("valid principal")),
        bindings: vec![
            SceneBinding::Frozen {
                target: TargetId::surface("alienware-aw-elc", "power-button-ac"),
                state: state.clone(),
            },
            SceneBinding::Frozen {
                target: TargetId::surface("alienware-aw-elc", "power-button-ac"),
                state,
            },
        ],
    };

    let error = migrate_alienware_scene(&mut scene).expect_err("duplicate must be rejected");
    assert!(error.contains("duplicate legacy Alienware ac bindings"));
}

#[test]
fn version_nine_snapshot_defaults_to_no_scenes() {
    let path = unique_state_path("v9-no-scenes");
    write_private_fixture(
        &path,
        r#"{
  "version": 9,
  "targets": [],
  "collections": {},
  "adopted_baseline": []
}"#,
    );

    let loaded = load(&path).expect("load version nine state");

    assert!(loaded.scenes.is_empty());
    let _ = fs::remove_file(&path);
}

#[test]
fn a_v7_file_with_bare_owner_uid_is_refused_with_reset_guidance() {
    // No pre-v8 daemon could have produced any owner other than a Unix
    // uid, so a legacy `owner_uid` integer is read straight into
    // `OwnerIdentity::Uid`.
    let path = unique_state_path("legacy-owner-uid");
    write_private_fixture(
        &path,
        r#"{
  "version": 7,
  "targets": [],
  "collections": {
    "living-room": {
      "id": "living-room",
      "name": "Living room",
      "description": null,
      "owner_uid": 1000,
      "kind": null,
      "members": []
    }
  }
}"#,
    );

    let error = load(&path).expect_err("legacy ownership must be refused");
    assert!(error.to_string().contains("reset-owned-objects --confirm"));

    let _ = fs::remove_file(&path);
}

#[test]
fn reset_owned_objects_backs_up_and_preserves_target_state_and_baseline() {
    let path = unique_state_path("reset-owned-objects");
    let adopted = vec![AdoptedFacet {
        target: TargetId::device("dev0"),
        value: FacetValue::Brightness(77),
        confirmed_at_ms: 1234,
    }];
    let mut collections = HashMap::new();
    collections.insert(
        CollectionId::new("legacy"),
        Collection {
            id: CollectionId::new("legacy"),
            name: "Legacy".to_owned(),
            description: None,
            owner: OwnerIdentity::Uid(1000),
            kind: None,
            members: Vec::new(),
        },
    );
    save_complete(
        &path,
        &sample_state(),
        &collections,
        &HashMap::new(),
        &adopted,
    )
    .expect("save legacy state");

    let backup = reset_owned_objects(&path, true).expect("reset should succeed");
    let loaded = load(&path).expect("reset state should be loadable");
    assert!(loaded.collections.is_empty());
    assert!(loaded.scenes.is_empty());
    assert_eq!(loaded.entries.len(), 2);
    assert_eq!(loaded.entries[0].target, sample_state()[0].target);
    assert_eq!(loaded.entries[1].target, sample_state()[1].target);
    assert_eq!(loaded.adopted_baseline, adopted);
    assert!(backup.exists());

    let _ = fs::remove_file(&backup);
    let _ = fs::remove_file(&path);
}

#[test]
fn legacy_state_without_collections_loads_with_empty_collections() {
    let path = unique_state_path("no-collections");
    save(&path, &sample_state()).expect("save should succeed");
    let loaded = load(&path).expect("load should succeed");
    assert!(loaded.collections.is_empty());
    let _ = fs::remove_file(&path);
}

#[test]
fn a_v6_files_location_fields_are_silently_dropped_on_load() {
    let path = unique_state_path("legacy-locations");
    write_private_fixture(
        &path,
        r#"{
  "version": 6,
  "targets": [],
  "location_overrides": { "dev0": "office" },
  "location_defaults": { "office": { "state": { "Brightness": 1 }, "on_unsupported": "Skip" } },
  "collections": {}
}"#,
    );

    let loaded = load(&path).expect("a v6 file with location fields should still load");
    assert!(loaded.entries.is_empty());
    assert!(loaded.collections.is_empty());

    let _ = fs::remove_file(&path);
}

#[test]
fn corrupt_file_loads_as_empty() {
    let path = unique_state_path("corrupt");
    let corrupt_contents = b"not json";
    write_private_fixture(&path, corrupt_contents);

    let loaded = load(&path).expect("load should recover from corrupt state");
    assert!(loaded.entries.is_empty());
    assert!(!path.exists(), "rejected state should be moved aside");

    let retained = retained_state_path(&path);
    assert_eq!(
        fs::read(&retained).expect("read retained corrupt state"),
        corrupt_contents
    );

    let _ = fs::remove_file(&retained);
}

#[test]
fn versionless_file_is_loaded_as_legacy_unordered_state() {
    let path = unique_state_path("legacy");
    write_private_fixture(
        &path,
        r#"{
  "targets": [
    {
      "target": { "Device": "dev0" },
      "state": { "Brightness": 42 }
    }
  ]
}"#,
    );

    let loaded = load(&path).expect("load legacy state");
    assert!(!loaded.preserve_order);
    assert_eq!(loaded.entries.len(), 1);

    let _ = fs::remove_file(path);
}

#[cfg(unix)]
#[test]
fn saved_state_file_is_not_group_or_world_accessible() {
    let path = unique_state_path("mode");

    save(&path, &sample_state()).expect("save should succeed");
    let mode = fs::metadata(&path)
        .expect("state file should exist")
        .permissions()
        .mode();
    // Owner-only: no group or other bits, regardless of the process umask.
    assert_eq!(
        mode & 0o077,
        0,
        "state file should not be group/other accessible (mode {:04o})",
        mode & 0o7777
    );

    let _ = fs::remove_file(&path);
}

#[cfg(unix)]
#[test]
fn load_rejects_a_symlinked_state_file() {
    let target = unique_state_path("symlink-target");
    let link = unique_state_path("symlink-link");
    save(&target, &sample_state()).expect("save target should succeed");
    symlink(&target, &link).expect("create symlink");

    // O_NOFOLLOW makes opening the symlink itself fail rather than reading
    // through to an attacker-chosen target.
    let error = load(&link).expect_err("loading a symlinked state path must fail");
    assert!(
        error.to_string().contains("failed to open state file"),
        "unexpected error: {error}"
    );

    let _ = fs::remove_file(&link);
    let _ = fs::remove_file(&target);
}

#[cfg(unix)]
#[test]
fn load_rejects_a_group_readable_state_file() {
    let path = unique_state_path("permissive-state");
    save(&path, &sample_state()).expect("save state fixture");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640))
        .expect("make state group-readable");

    let error = load(&path).expect_err("loading a group-readable authority file must fail");
    assert!(
        error.to_string().contains("failed to open state file"),
        "unexpected error: {error}"
    );
}

#[cfg(unix)]
#[test]
fn save_does_not_write_through_a_preexisting_symlink() {
    let decoy = unique_state_path("decoy-target");
    let path = unique_state_path("decoy-link");
    fs::write(&decoy, b"sentinel").expect("write decoy");
    symlink(&decoy, &path).expect("create symlink at state path");

    save(&path, &sample_state()).expect("save over a symlink should succeed");

    // The rename replaces the symlink name with our regular file; the decoy
    // the symlink pointed at is untouched.
    assert_eq!(
        fs::read(&decoy).expect("decoy still readable"),
        b"sentinel",
        "save must not write through the symlink to its target"
    );
    assert!(
        !fs::symlink_metadata(&path)
            .expect("state path exists")
            .file_type()
            .is_symlink(),
        "state path should be a regular file after save"
    );
    let loaded = load(&path).expect("load should succeed");
    assert_eq!(loaded.entries.len(), 2);

    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&decoy);
}

#[test]
fn oversized_file_loads_as_empty() {
    let path = unique_state_path("oversized");
    // Valid JSON prefix followed by padding pushes the file just past the
    // byte cap; load must bail before deserializing rather than pulling it
    // all into memory.
    let mut bytes = vec![b' '; MAX_STATE_FILE_BYTES + 1];
    bytes[0] = b'{';
    write_private_fixture(&path, &bytes);

    let loaded = load(&path).expect("oversized file should load as empty, not error");
    assert!(loaded.entries.is_empty());
    assert!(!path.exists(), "rejected state should be moved aside");

    let retained = retained_state_path(&path);
    assert_eq!(
        fs::read(&retained).expect("read retained oversized state"),
        bytes
    );

    let _ = fs::remove_file(&retained);
}
