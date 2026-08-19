// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use luminate_core::device::DeviceId;
use luminate_core::effect::Effect;
use luminate_core::element::ElementId;
use luminate_core::surface::SurfaceId;

use super::super::tests_support::*;
use super::*;

fn empty_state() -> DaemonState {
    DaemonState::from_devices(Vec::new())
}

fn target_member(device: &str) -> CollectionMember {
    CollectionMember::Target(TargetId::device(device))
}

#[test]
fn create_collection_assigns_a_fresh_id_and_stores_members() {
    let mut state = empty_state();
    let id = state
        .create_collection(
            "Living room".to_owned(),
            Some("Wall strip and two lamps".to_owned()),
            OwnerIdentity::Uid(1000),
            None,
            vec![target_member("lamp-a"), target_member("lamp-b")],
        )
        .expect("create collection");

    let collection = state.collection(&id).expect("collection should exist");
    assert_eq!(collection.name, "Living room");
    assert_eq!(collection.owner, OwnerIdentity::Uid(1000));
    assert_eq!(collection.members.len(), 2);
}

#[test]
fn create_collection_deduplicates_repeated_members() {
    let mut state = empty_state();
    let id = state
        .create_collection(
            "Duplicates".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![target_member("lamp-a"), target_member("lamp-a")],
        )
        .expect("create collection");

    assert_eq!(state.collection(&id).expect("exists").members.len(), 1);
}

#[test]
fn create_collection_rejects_a_nonexistent_nested_collection() {
    let mut state = empty_state();
    let error = state
        .create_collection(
            "Ghost".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![CollectionMember::Collection(CollectionId::new("ghost"))],
        )
        .expect_err("nested reference to an unknown collection must fail");

    assert!(matches!(error, DaemonError::CollectionNotFound(_)));
}

#[test]
fn generated_ids_differ_across_creations() {
    let mut state = empty_state();
    let first = state
        .create_collection(
            "A".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            Vec::new(),
        )
        .expect("create first");
    let second = state
        .create_collection(
            "B".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            Vec::new(),
        )
        .expect("create second");
    assert_ne!(first, second);
}

#[test]
fn destroy_collection_removes_it() {
    let mut state = empty_state();
    let id = state
        .create_collection(
            "Temp".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            Vec::new(),
        )
        .expect("create collection");

    state.destroy_collection(&id).expect("destroy collection");
    assert!(state.collection(&id).is_none());
}

#[test]
fn destroy_collection_reports_not_found_for_an_unknown_id() {
    let mut state = empty_state();
    let error = state
        .destroy_collection(&CollectionId::new("missing"))
        .expect_err("destroying an unknown collection must fail");
    assert!(matches!(error, DaemonError::CollectionNotFound(_)));
}

#[test]
fn destroy_collection_is_restricted_while_referenced() {
    let mut state = empty_state();
    let inner = state
        .create_collection(
            "Nook".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            Vec::new(),
        )
        .expect("create inner collection");
    let outer = state
        .create_collection(
            "Living room".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![CollectionMember::Collection(inner.clone())],
        )
        .expect("create outer collection");

    let error = state
        .destroy_collection(&inner)
        .expect_err("a referenced collection must not be destroyable");
    let DaemonError::CollectionInUse { referenced_by, .. } = error else {
        panic!("expected CollectionInUse, got {error:?}");
    };
    assert_eq!(referenced_by, vec![outer.clone()]);

    // Removing the reference first must then allow the destroy.
    state
        .remove_collection_member(&outer, &CollectionMember::Collection(inner.clone()))
        .expect("remove the reference");
    state
        .destroy_collection(&inner)
        .expect("now-unreferenced collection should be destroyable");
}

#[test]
fn add_collection_member_rejects_direct_self_reference() {
    let mut state = empty_state();
    let id = state
        .create_collection(
            "Self".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            Vec::new(),
        )
        .expect("create collection");

    let error = state
        .add_collection_member(&id, CollectionMember::Collection(id.clone()))
        .expect_err("self-reference must be rejected");
    assert!(matches!(error, DaemonError::CollectionSelfReference(_)));
}

#[test]
fn add_collection_member_rejects_an_indirect_cycle() {
    let mut state = empty_state();
    let a = state
        .create_collection(
            "A".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            Vec::new(),
        )
        .expect("create a");
    let b = state
        .create_collection(
            "B".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![CollectionMember::Collection(a.clone())],
        )
        .expect("create b nested in a");

    // a -> b would close the cycle a -> b -> a.
    let error = state
        .add_collection_member(&a, CollectionMember::Collection(b))
        .expect_err("indirect cycle must be rejected");
    assert!(matches!(error, DaemonError::CollectionCycle(_)));
}

#[test]
fn add_collection_member_allows_a_diamond_reference() {
    let mut state = empty_state();
    let shared = state
        .create_collection(
            "Hallway".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![target_member("hall-light")],
        )
        .expect("create shared collection");
    let left = state
        .create_collection(
            "Left wing".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![CollectionMember::Collection(shared.clone())],
        )
        .expect("create left wing");
    let right = state
        .create_collection(
            "Right wing".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![CollectionMember::Collection(shared.clone())],
        )
        .expect("create right wing");

    assert!(state.collection(&left).is_some());
    assert!(state.collection(&right).is_some());
}

#[test]
fn add_collection_member_rejects_a_nonexistent_nested_collection() {
    let mut state = empty_state();
    let id = state
        .create_collection(
            "Room".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            Vec::new(),
        )
        .expect("create room");

    let error = state
        .add_collection_member(
            &id,
            CollectionMember::Collection(CollectionId::new("ghost")),
        )
        .expect_err("nested reference to an unknown collection must fail");
    assert!(matches!(error, DaemonError::CollectionNotFound(_)));
}

#[test]
fn add_collection_member_on_unknown_collection_fails() {
    let mut state = empty_state();
    let error = state
        .add_collection_member(&CollectionId::new("missing"), target_member("lamp"))
        .expect_err("adding to an unknown collection must fail");
    assert!(matches!(error, DaemonError::CollectionNotFound(_)));
}

#[test]
fn add_collection_member_is_idempotent_for_a_duplicate() {
    let mut state = empty_state();
    let id = state
        .create_collection(
            "Room".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![target_member("lamp")],
        )
        .expect("create room");

    state
        .add_collection_member(&id, target_member("lamp"))
        .expect("adding a duplicate member should be a no-op, not an error");
    assert_eq!(state.collection(&id).expect("exists").members.len(), 1);
}

#[test]
fn remove_collection_member_drops_it() {
    let mut state = empty_state();
    let id = state
        .create_collection(
            "Room".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![target_member("lamp")],
        )
        .expect("create room");

    state
        .remove_collection_member(&id, &target_member("lamp"))
        .expect("remove member");
    assert!(state.collection(&id).expect("exists").members.is_empty());
}

#[test]
fn remove_collection_member_on_unknown_collection_fails() {
    let mut state = empty_state();
    let error = state
        .remove_collection_member(&CollectionId::new("missing"), &target_member("lamp"))
        .expect_err("removing from an unknown collection must fail");
    assert!(matches!(error, DaemonError::CollectionNotFound(_)));
}

#[test]
fn remove_collection_member_on_absent_member_is_a_noop() {
    let mut state = empty_state();
    let id = state
        .create_collection(
            "Room".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            Vec::new(),
        )
        .expect("create room");

    state
        .remove_collection_member(&id, &target_member("lamp"))
        .expect("removing an absent member should be a no-op, not an error");
}

#[test]
fn resolve_collection_leaves_flattens_nested_collections() {
    let mut state = empty_state();
    let inner = state
        .create_collection(
            "Nook".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![target_member("nook-lamp")],
        )
        .expect("create inner");
    let outer = state
        .create_collection(
            "Living room".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![
                target_member("main-lamp"),
                CollectionMember::Collection(inner),
            ],
        )
        .expect("create outer");

    let mut leaves = state
        .resolve_collection_leaves(&outer)
        .expect("resolve leaves");
    leaves.sort_by(|left, right| left.device_id().as_str().cmp(right.device_id().as_str()));

    assert_eq!(
        leaves,
        vec![
            TargetId::Device(DeviceId::new("main-lamp")),
            TargetId::Device(DeviceId::new("nook-lamp")),
        ]
    );
}

#[test]
fn resolve_collection_leaves_deduplicates_a_diamond_reference() {
    let mut state = empty_state();
    let shared = state
        .create_collection(
            "Hallway".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![target_member("hall-light")],
        )
        .expect("create shared collection");
    let left = state
        .create_collection(
            "Left wing".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![
                target_member("left-lamp"),
                CollectionMember::Collection(shared.clone()),
            ],
        )
        .expect("create left wing");
    let combined = state
        .create_collection(
            "Whole floor".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            vec![
                CollectionMember::Collection(left),
                CollectionMember::Collection(shared),
            ],
        )
        .expect("create combined collection");

    let leaves = state
        .resolve_collection_leaves(&combined)
        .expect("resolve leaves");

    assert_eq!(
        leaves.len(),
        2,
        "the shared hallway light must be reported once, not twice: {leaves:?}"
    );
}

#[test]
fn resolve_collection_leaves_on_unknown_collection_fails() {
    let state = empty_state();
    let error = state
        .resolve_collection_leaves(&CollectionId::new("missing"))
        .expect_err("resolving an unknown collection must fail");
    assert!(matches!(error, DaemonError::CollectionNotFound(_)));
}

#[test]
fn skipped_effect_fails_when_every_collection_member_is_unsupported() {
    let state = demo_state();
    let members = vec![TargetId::Element {
        device: DeviceId::new("demo-kbd"),
        surface: SurfaceId::new("main"),
        element: ElementId::new("topology-only"),
    }];

    let error = state
        .prepare_collection_mutation(
            &members,
            &TargetState::Effect(Effect::Off),
            UnsupportedPolicy::Skip,
        )
        .expect_err("an effect unsupported by every member must fail");

    assert!(matches!(error, DaemonError::UnsupportedCapability { .. }));
}

#[test]
fn skipped_effect_succeeds_when_any_collection_member_is_supported() {
    let state = demo_state();
    let supported = TargetId::device("demo-kbd");
    let members = vec![
        supported.clone(),
        TargetId::Element {
            device: DeviceId::new("demo-kbd"),
            surface: SurfaceId::new("main"),
            element: ElementId::new("topology-only"),
        },
    ];

    let prepared = state
        .prepare_collection_mutation(
            &members,
            &TargetState::Effect(Effect::Off),
            UnsupportedPolicy::Skip,
        )
        .expect("one supported member is enough for best-effort application");

    assert_eq!(prepared, vec![supported]);
}

#[test]
fn restore_collections_replaces_the_registry() {
    let mut state = empty_state();
    state
        .create_collection(
            "Stale".to_owned(),
            None,
            OwnerIdentity::Uid(1000),
            None,
            Vec::new(),
        )
        .expect("create stale collection");

    let mut restored = HashMap::new();
    let id = CollectionId::new("restored");
    restored.insert(
        id.clone(),
        Collection {
            id: id.clone(),
            name: "Restored".to_owned(),
            description: None,
            owner: OwnerIdentity::Uid(1000),
            kind: None,
            members: Vec::new(),
        },
    );
    state.restore_collections(restored);

    assert_eq!(state.collections().len(), 1);
    assert!(state.collection(&id).is_some());
}

#[test]
fn demo_state_fixture_starts_with_no_collections() {
    let state = demo_state();
    assert!(state.collections().is_empty());
}
