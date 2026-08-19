// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

fn collection_id() -> CollectionId {
    CollectionId::new("living-room")
}

#[test]
fn generate_produces_distinct_identifiers() {
    assert_ne!(CollectionId::generate(), CollectionId::generate());
}

#[test]
fn references_self_detects_direct_self_reference() {
    let id = collection_id();
    let members = vec![CollectionMember::Collection(id.clone())];

    assert!(Collection::references_self(&id, &members));
}

#[test]
fn references_self_ignores_other_members() {
    let id = collection_id();
    let members = vec![
        CollectionMember::Target(TargetId::device("lamp")),
        CollectionMember::Collection(CollectionId::new("kitchen")),
    ];

    assert!(!Collection::references_self(&id, &members));
}

#[test]
fn references_self_on_empty_membership_is_false() {
    assert!(!Collection::references_self(&collection_id(), &[]));
}

#[test]
fn collection_member_target_round_trips_through_json() {
    let member = CollectionMember::Target(TargetId::device("lamp"));
    let encoded = serde_json::to_string(&member).expect("encode member");
    let decoded: CollectionMember = serde_json::from_str(&encoded).expect("decode member");

    assert_eq!(decoded, member);
}

#[test]
fn collection_round_trips_through_json() {
    let collection = Collection {
        id: collection_id(),
        name: "Living room".to_owned(),
        description: Some("Wall-mounted strip and two lamps".to_owned()),
        owner: OwnerIdentity::Uid(1000),
        kind: Some(CollectionCategory::new(collection_category::LOCATION)),
        members: vec![
            CollectionMember::Target(TargetId::device("lamp")),
            CollectionMember::Target(TargetId::group("strip", "wall")),
            CollectionMember::Collection(CollectionId::new("nook")),
        ],
    };

    let encoded = serde_json::to_string(&collection).expect("encode collection");
    let decoded: Collection = serde_json::from_str(&encoded).expect("decode collection");

    assert_eq!(decoded, collection);
}

#[test]
fn collection_id_as_str_round_trips_new() {
    assert_eq!(CollectionId::new("kitchen").as_str(), "kitchen");
}

#[test]
fn collection_category_as_str_round_trips_new() {
    assert_eq!(
        CollectionCategory::new(collection_category::ZONE).as_str(),
        "zone"
    );
}

#[test]
fn collection_without_a_kind_round_trips_through_json() {
    let collection = Collection {
        id: collection_id(),
        name: "Untagged".to_owned(),
        description: None,
        owner: OwnerIdentity::Uid(1000),
        kind: None,
        members: Vec::new(),
    };

    let encoded = serde_json::to_string(&collection).expect("encode collection");
    let decoded: Collection = serde_json::from_str(&encoded).expect("decode collection");

    assert_eq!(decoded, collection);
}

#[test]
fn owner_identity_uid_encodes_as_a_bare_integer_and_sid_as_a_bare_string() {
    let uid = serde_json::to_string(&OwnerIdentity::Uid(1000)).expect("encode uid");
    assert_eq!(uid, "1000");
    let sid =
        serde_json::to_string(&OwnerIdentity::Sid("S-1-5-21-1".to_owned())).expect("encode sid");
    assert_eq!(sid, "\"S-1-5-21-1\"");
}

#[test]
fn a_legacy_owner_uid_field_deserializes_as_owner_uid() {
    // Every collection persisted before `OwnerIdentity` existed used a
    // bare `owner_uid: u32` field; no daemon could have written anything
    // else, so this must keep loading exactly as `OwnerIdentity::Uid`.
    let legacy = serde_json::json!({
        "id": collection_id(),
        "name": "Legacy",
        "description": null,
        "owner_uid": 1000,
        "kind": null,
        "members": [],
    });
    let collection: Collection =
        serde_json::from_value(legacy).expect("decode legacy owner_uid field");
    assert_eq!(collection.owner, OwnerIdentity::Uid(1000));
}

fn collection(id: &str, members: Vec<CollectionMember>) -> Collection {
    Collection {
        id: CollectionId::new(id),
        name: id.to_owned(),
        description: None,
        owner: OwnerIdentity::Uid(1000),
        kind: None,
        members,
    }
}

#[test]
fn graph_resolution_is_ordered_and_deduplicates_diamonds() {
    let shared = collection(
        "shared",
        vec![CollectionMember::Target(TargetId::device("shared-lamp"))],
    );
    let left = collection(
        "left",
        vec![
            CollectionMember::Target(TargetId::device("left-lamp")),
            CollectionMember::Collection(shared.id.clone()),
        ],
    );
    let root = collection(
        "root",
        vec![
            CollectionMember::Collection(left.id.clone()),
            CollectionMember::Collection(shared.id.clone()),
        ],
    );
    let collections = [root, shared, left];
    let graph = CollectionGraph::new(&collections);

    assert_eq!(
        graph.resolve_leaves(&CollectionId::new("root")),
        Ok(vec![
            TargetId::device("left-lamp"),
            TargetId::device("shared-lamp"),
        ])
    );
}

#[test]
fn graph_resolution_rejects_missing_references_and_cycles() {
    let missing = collection(
        "missing-root",
        vec![CollectionMember::Collection(CollectionId::new("absent"))],
    );
    let left = collection(
        "left",
        vec![CollectionMember::Collection(CollectionId::new("right"))],
    );
    let right = collection(
        "right",
        vec![CollectionMember::Collection(CollectionId::new("left"))],
    );
    let collections = [missing, left, right];
    let graph = CollectionGraph::new(&collections);

    assert_eq!(
        graph.resolve_leaves(&CollectionId::new("missing-root")),
        Err(CollectionGraphError::CollectionNotFound(CollectionId::new(
            "absent"
        )))
    );
    assert!(matches!(
        graph.resolve_leaves(&CollectionId::new("left")),
        Err(CollectionGraphError::Cycle(_))
    ));
}

#[test]
fn graph_resolution_rejects_duplicate_identifiers() {
    let first = collection("duplicate", Vec::new());
    let second = collection(
        "duplicate",
        vec![CollectionMember::Target(TargetId::device("lamp"))],
    );
    let collections = [first, second];
    let graph = CollectionGraph::new(&collections);

    assert_eq!(
        graph.resolve_leaves(&CollectionId::new("duplicate")),
        Err(CollectionGraphError::DuplicateCollection(
            CollectionId::new("duplicate")
        ))
    );
}

#[test]
fn graph_projection_removes_hidden_targets_and_links() {
    let hidden = collection(
        "hidden",
        vec![CollectionMember::Target(TargetId::device("hidden-lamp"))],
    );
    let mixed = collection(
        "mixed",
        vec![
            CollectionMember::Target(TargetId::device("visible-lamp")),
            CollectionMember::Target(TargetId::device("hidden-lamp")),
        ],
    );
    let root = collection(
        "root",
        vec![
            CollectionMember::Collection(hidden.id.clone()),
            CollectionMember::Collection(mixed.id.clone()),
        ],
    );
    let collections = [root, mixed, hidden];
    let graph = CollectionGraph::new(&collections);

    let projected = graph
        .project(
            |target| target.device_id().as_str() == "visible-lamp",
            |_| false,
        )
        .expect("project valid graph");

    assert_eq!(
        projected
            .iter()
            .map(|collection| collection.id.as_str())
            .collect::<Vec<_>>(),
        vec!["mixed", "root"]
    );
    assert_eq!(
        projected[0].members,
        vec![CollectionMember::Target(TargetId::device("visible-lamp"))]
    );
    assert_eq!(
        projected[1].members,
        vec![CollectionMember::Collection(CollectionId::new("mixed"))]
    );
}

#[test]
fn graph_projection_requires_explicit_visibility_for_empty_collections() {
    let empty = collection("empty", Vec::new());
    let collections = [empty];
    let graph = CollectionGraph::new(&collections);

    assert!(
        graph
            .project(|_| false, |_| false)
            .expect("project valid graph")
            .is_empty()
    );
    assert_eq!(
        graph
            .project(|_| false, |id| id.as_str() == "empty")
            .expect("project valid graph")
            .len(),
        1
    );
}
