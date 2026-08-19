// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Collection projections and portable fake-daemon round trips.

#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "Safety is documented before each C-boundary assertion; Clippy cannot associate comments outside assertion macro expansions."
)]

use super::*;
use crate::ffi::{luminate_client_connect_path, luminate_client_free};
use crate::ffi_typed::state::luminate_target_view_device_id;
use crate::ffi_typed::test_support::{FakeDaemon, respond};
use luminate_core::policy::PrincipalId;
use luminate_core::target::TargetId;
use luminate_protocol::{Request, ResponseStatus};
use std::ffi::CString;

fn bytes(view: LuminateStringView) -> Option<&'static [u8]> {
    if view.data.is_null() {
        None
    } else {
        // SAFETY: test inputs remain alive while their borrowed views are inspected.
        Some(unsafe { std::slice::from_raw_parts(view.data.cast(), view.len) })
    }
}

fn collection(members: Vec<CollectionMember>) -> Collection {
    Collection {
        id: CollectionId::new("living-room"),
        name: "Living Room".to_owned(),
        description: Some("Downstairs lighting".to_owned()),
        owner: OwnerIdentity::Uid(501),
        kind: Some(CollectionCategory::new("location")),
        members,
    }
}

fn sid_owned_collection() -> Collection {
    Collection {
        owner: OwnerIdentity::Sid("S-1-5-21-1-2-3-1000".to_owned()),
        ..collection(Vec::new())
    }
}

#[test]
fn collection_accessors_cover_member_variants() {
    let collection = collection(vec![
        CollectionMember::Target(TargetId::device("lamp")),
        CollectionMember::Collection(CollectionId::new("nook")),
    ]);
    let ptr: *const LuminateCollection = ptr::from_ref(&collection).cast();

    // SAFETY: `ptr` and each returned member pointer borrow the live `collection`.
    unsafe {
        assert_eq!(
            bytes(luminate_collection_id(ptr)),
            Some(b"living-room".as_slice())
        );
        assert_eq!(
            bytes(luminate_collection_name(ptr)),
            Some(b"Living Room".as_slice())
        );
        assert_eq!(
            bytes(luminate_collection_description(ptr)),
            Some(b"Downstairs lighting".as_slice())
        );
        let owner = luminate_collection_owner(ptr);
        assert_eq!(
            luminate_owner_identity_kind(owner),
            LuminateOwnerKind::LuminateOwnerKindUid
        );
        let mut uid = 0;
        assert!(luminate_owner_identity_uid(owner, &raw mut uid));
        assert_eq!(uid, 501);
        assert_eq!(bytes(luminate_owner_identity_sid(owner)), None);
        assert_eq!(
            bytes(luminate_collection_kind(ptr)),
            Some(b"location".as_slice())
        );
        assert_eq!(luminate_collection_member_count(ptr), 2);
        assert!(luminate_collection_member_at(ptr, 2).is_null());

        let target = luminate_collection_member_at(ptr, 0);
        assert_eq!(luminate_collection_member_kind(target), 0);
        assert!(!luminate_collection_member_target(target).is_null());
        assert_eq!(
            bytes(luminate_collection_member_collection_id(target)),
            None
        );
        assert_eq!(
            bytes(luminate_target_view_device_id(
                luminate_collection_member_target(target)
            )),
            Some(b"lamp".as_slice())
        );

        let nested = luminate_collection_member_at(ptr, 1);
        assert_eq!(luminate_collection_member_kind(nested), 1);
        assert!(luminate_collection_member_target(nested).is_null());
        assert_eq!(
            bytes(luminate_collection_member_collection_id(nested)),
            Some(b"nook".as_slice())
        );
    }
}

#[test]
fn sid_owned_collection_exposes_owner_kind_and_sid() {
    let collection = sid_owned_collection();
    let ptr: *const LuminateCollection = ptr::from_ref(&collection).cast();

    // SAFETY: `ptr` borrows the live `collection`.
    unsafe {
        let owner = luminate_collection_owner(ptr);
        assert_eq!(
            luminate_owner_identity_kind(owner),
            LuminateOwnerKind::LuminateOwnerKindSid
        );
        let mut uid = 7;
        assert!(!luminate_owner_identity_uid(owner, &raw mut uid));
        assert_eq!(uid, 7);
        assert_eq!(
            bytes(luminate_owner_identity_sid(owner)),
            Some(b"S-1-5-21-1-2-3-1000".as_slice())
        );
    }
}

#[test]
fn principal_owned_collection_exposes_complete_identity() {
    let collection = Collection {
        owner: OwnerIdentity::Principal(
            PrincipalId::new("oidc.example", "alice").expect("valid principal"),
        ),
        ..collection(Vec::new())
    };
    let ptr: *const LuminateCollection = ptr::from_ref(&collection).cast();

    // SAFETY: `ptr` borrows the live `collection`.
    unsafe {
        let owner = luminate_collection_owner(ptr);
        assert_eq!(
            luminate_owner_identity_kind(owner),
            LuminateOwnerKind::LuminateOwnerKindPrincipal
        );
        assert_eq!(
            bytes(luminate_owner_identity_principal_authority(owner)),
            Some(b"oidc.example".as_slice())
        );
        assert_eq!(
            bytes(luminate_owner_identity_principal_subject(owner)),
            Some(b"alice".as_slice())
        );
        assert_eq!(bytes(luminate_owner_identity_sid(owner)), None);
    }
}

#[test]
fn collection_list_and_snapshot_expose_their_members() {
    let list = Box::new(LuminateCollectionList(vec![
        collection(Vec::new()),
        collection(Vec::new()),
    ]));
    let list_ptr = Box::into_raw(list);
    // SAFETY: `list_ptr` was just allocated and is freed exactly once below.
    unsafe {
        assert_eq!(luminate_collection_list_count(list_ptr), 2);
        assert!(!luminate_collection_list_at(list_ptr, 0).is_null());
        assert!(luminate_collection_list_at(list_ptr, 2).is_null());
        luminate_collection_list_free(list_ptr);
    }

    let snapshot = Box::new(LuminateCollectionSnapshot(collection(Vec::new())));
    let snapshot_ptr = Box::into_raw(snapshot);
    // SAFETY: `snapshot_ptr` was just allocated and is freed exactly once below.
    unsafe {
        assert!(!luminate_collection_snapshot_collection(snapshot_ptr).is_null());
        luminate_collection_snapshot_free(snapshot_ptr);
    }

    // Null is a documented no-op for both free functions.
    // SAFETY: both functions explicitly accept null.
    unsafe {
        luminate_collection_list_free(ptr::null_mut());
        luminate_collection_snapshot_free(ptr::null_mut());
    }
}

#[test]
fn null_and_absent_values_return_documented_sentinels() {
    let list = ptr::null::<LuminateCollectionList>();
    let snapshot = ptr::null::<LuminateCollectionSnapshot>();
    let collection = ptr::null::<LuminateCollection>();
    let member = ptr::null::<LuminateCollectionMember>();
    // SAFETY: every accessor explicitly accepts null and returns a sentinel.
    unsafe {
        assert_eq!(luminate_collection_list_count(list), 0);
        assert!(luminate_collection_list_at(list, 0).is_null());
        assert!(luminate_collection_snapshot_collection(snapshot).is_null());
        assert_eq!(bytes(luminate_collection_id(collection)), None);
        assert_eq!(bytes(luminate_collection_name(collection)), None);
        assert_eq!(bytes(luminate_collection_description(collection)), None);
        assert!(luminate_collection_owner(collection).is_null());
        assert_eq!(
            luminate_owner_identity_kind(ptr::null()),
            LuminateOwnerKind::LuminateOwnerKindInvalid
        );
        assert_eq!(bytes(luminate_owner_identity_sid(ptr::null())), None);
        assert_eq!(bytes(luminate_collection_kind(collection)), None);
        assert_eq!(luminate_collection_member_count(collection), 0);
        assert!(luminate_collection_member_at(collection, 0).is_null());
        assert_eq!(luminate_collection_member_kind(member), u32::MAX);
        assert!(luminate_collection_member_target(member).is_null());
        assert_eq!(
            bytes(luminate_collection_member_collection_id(member)),
            None
        );
    }

    let mut undescribed = collection_fixture();
    undescribed.description = None;
    undescribed.kind = None;
    let undescribed: *const LuminateCollection = ptr::from_ref(&undescribed).cast();
    // SAFETY: `undescribed` refers to a live `Collection`.
    unsafe {
        assert_eq!(bytes(luminate_collection_description(undescribed)), None);
        assert_eq!(bytes(luminate_collection_kind(undescribed)), None);
    }
}

fn collection_fixture() -> Collection {
    collection(Vec::new())
}

async fn serve_collection_crud_sequence(mut daemon: FakeDaemon) {
    let mut stream = daemon.accept("collections-test-daemon").await;
    assert!(matches!(
        respond(
            &mut stream,
            ResponseStatus::CollectionCreated {
                id: CollectionId::new("living-room"),
            },
        )
        .await,
        Request::CreateCollection { .. }
    ));
    assert!(matches!(
        respond(&mut stream, ResponseStatus::Ack).await,
        Request::AddCollectionMember { .. }
    ));
    assert!(matches!(
        respond(&mut stream, ResponseStatus::Ack).await,
        Request::RemoveCollectionMember { .. }
    ));
    assert!(matches!(
        respond(
            &mut stream,
            ResponseStatus::Collections(vec![collection_fixture()]),
        )
        .await,
        Request::ListCollections
    ));
    assert!(matches!(
        respond(
            &mut stream,
            ResponseStatus::CollectionInfo(Some(Box::new(collection_fixture()))),
        )
        .await,
        Request::GetCollection { .. }
    ));
    assert!(matches!(
        respond(&mut stream, ResponseStatus::CollectionInfo(None)).await,
        Request::GetCollection { .. }
    ));
    assert!(matches!(
        respond(&mut stream, ResponseStatus::Ack).await,
        Request::DestroyCollection { .. }
    ));
}

/// Runs the client half of the CRUD sequence `serve_collection_crud_sequence`
/// expects, against an already-connected `client`.
///
/// # Safety
///
/// `client` must be a valid, connected `LuminateClient` handle not used
/// concurrently by another thread.
#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "The test exercises several independent client operations across one live connection."
)]
unsafe fn exercise_collection_crud(client: *mut LuminateClient) {
    let name = CString::new("Living Room").expect("valid name");
    let collection_id = CString::new("living-room").expect("valid id");
    let nested_id = CString::new("nook").expect("valid nested id");
    let device = CString::new("lamp").expect("valid device id");
    let target_member = LuminateCollectionMemberInput {
        is_collection: false,
        target: LuminateTarget {
            device_id: device.as_ptr(),
            surface_id: ptr::null(),
            element_id: ptr::null(),
            group_id: ptr::null(),
        },
        collection_id: ptr::null(),
    };
    let nested_member = LuminateCollectionMemberInput {
        is_collection: true,
        target: LuminateTarget {
            device_id: ptr::null(),
            surface_id: ptr::null(),
            element_id: ptr::null(),
            group_id: ptr::null(),
        },
        collection_id: nested_id.as_ptr(),
    };

    // SAFETY: every pointer refers to live local storage or a NUL-terminated
    // string kept alive for the duration of this function, and every owned
    // resource is freed exactly once.
    unsafe {
        let mut out_id = ptr::null_mut();
        assert_eq!(
            luminate_client_create_collection(
                client,
                name.as_ptr(),
                ptr::null(),
                ptr::null(),
                &raw const target_member,
                1,
                &raw mut out_id,
            ),
            LuminateStatus::Ok
        );
        assert!(!out_id.is_null());
        luminate_string_free_for_test(out_id);

        assert_eq!(
            luminate_client_add_collection_member(
                client,
                collection_id.as_ptr(),
                &raw const nested_member
            ),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_client_remove_collection_member(
                client,
                collection_id.as_ptr(),
                &raw const nested_member
            ),
            LuminateStatus::Ok
        );

        let mut list = ptr::null_mut();
        assert_eq!(
            luminate_client_list_collections(client, &raw mut list),
            LuminateStatus::Ok
        );
        assert_eq!(luminate_collection_list_count(list), 1);
        luminate_collection_list_free(list);

        let mut snapshot = ptr::null_mut();
        assert_eq!(
            luminate_client_get_collection(client, collection_id.as_ptr(), &raw mut snapshot),
            LuminateStatus::Ok
        );
        assert!(!snapshot.is_null());
        luminate_collection_snapshot_free(snapshot);

        let mut missing = ptr::null_mut();
        assert_eq!(
            luminate_client_get_collection(client, collection_id.as_ptr(), &raw mut missing),
            LuminateStatus::NotFound
        );
        assert!(missing.is_null());

        assert_eq!(
            luminate_client_destroy_collection(client, collection_id.as_ptr()),
            LuminateStatus::Ok
        );
    }
}

/// Exercises the successful dispatch branch of every collection CRUD client
/// operation against a live mock daemon. A stopped or null client can only
/// ever reach the `Internal`/`NullPointer` early-return branches, so this is
/// the only way to cover the `Ok` match arms.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn collection_operations_round_trip_through_a_live_daemon() {
    let daemon = FakeDaemon::bind("collections-ffi-crud");
    let path_c = daemon.c_path();
    let server = tokio::spawn(serve_collection_crud_sequence(daemon));
    let mut client = ptr::null_mut();
    // SAFETY: `path_c` is a live NUL-terminated string, `client` is a
    // writable local out-pointer, and the connected handle is freed exactly
    // once after `exercise_collection_crud` finishes with it.
    unsafe {
        assert_eq!(
            luminate_client_connect_path(path_c.as_ptr(), &raw mut client),
            LuminateStatus::Ok
        );
        exercise_collection_crud(client);
        luminate_client_free(client);
    }

    server.await.expect("collections server task");
}

/// `luminate_string_free` lives in the untyped `ffi` module; this local
/// wrapper keeps the round-trip test above self-contained without a second
/// import block.
unsafe fn luminate_string_free_for_test(value: *mut c_char) {
    // SAFETY: `value` was returned by `luminate_client_create_collection` and
    // has not been freed yet.
    unsafe { crate::ffi::luminate_string_free(value) };
}

#[test]
fn member_decoding_covers_both_shapes_and_invalid_combinations() {
    let device = CString::new("lamp").expect("valid device id");
    let nested = CString::new("nook").expect("valid nested id");

    let target_input = LuminateCollectionMemberInput {
        is_collection: false,
        target: LuminateTarget {
            device_id: device.as_ptr(),
            surface_id: ptr::null(),
            element_id: ptr::null(),
            group_id: ptr::null(),
        },
        collection_id: ptr::null(),
    };
    // SAFETY: `target_input`'s live component is a valid NUL-terminated string.
    assert_eq!(
        unsafe { read_collection_member(&target_input) },
        Ok(CollectionMember::Target(TargetId::device("lamp")))
    );

    let nested_input = LuminateCollectionMemberInput {
        is_collection: true,
        target: LuminateTarget {
            device_id: ptr::null(),
            surface_id: ptr::null(),
            element_id: ptr::null(),
            group_id: ptr::null(),
        },
        collection_id: nested.as_ptr(),
    };
    // SAFETY: `nested_input`'s live component is a valid NUL-terminated string.
    assert_eq!(
        unsafe { read_collection_member(&nested_input) },
        Ok(CollectionMember::Collection(CollectionId::new("nook")))
    );

    let missing_collection_id = LuminateCollectionMemberInput {
        is_collection: true,
        target: nested_input.target,
        collection_id: ptr::null(),
    };
    // SAFETY: null `collection_id` is rejected before dereferencing.
    assert_eq!(
        unsafe { read_collection_member(&missing_collection_id) },
        Err(LuminateStatus::NullPointer)
    );

    let missing_device_id = LuminateCollectionMemberInput {
        is_collection: false,
        target: LuminateTarget {
            device_id: ptr::null(),
            surface_id: ptr::null(),
            element_id: ptr::null(),
            group_id: ptr::null(),
        },
        collection_id: ptr::null(),
    };
    // SAFETY: null `device_id` is rejected before dereferencing.
    assert_eq!(
        unsafe { read_collection_member(&missing_device_id) },
        Err(LuminateStatus::NullPointer)
    );

    // SAFETY: a zero count never dereferences `members`, even when null.
    assert_eq!(unsafe { read_members(ptr::null(), 0) }, Ok(Vec::new()));
    // SAFETY: null is rejected before dereferencing when `count` is nonzero.
    assert_eq!(
        unsafe { read_members(ptr::null(), 1) },
        Err(LuminateStatus::NullPointer)
    );
    let inputs = [target_input, nested_input];
    // SAFETY: `inputs` holds `count` valid, readable members.
    assert_eq!(
        unsafe { read_members(inputs.as_ptr(), inputs.len()) },
        Ok(vec![
            CollectionMember::Target(TargetId::device("lamp")),
            CollectionMember::Collection(CollectionId::new("nook")),
        ])
    );
}

#[test]
fn member_operations_reject_null_member_and_null_client() {
    let name = CString::new("Living Room").expect("valid name");
    let id = CString::new("living-room").expect("valid id");
    // A null client still reaches the argument-validation branches before
    // the (absent) worker thread would be used.
    let client: *mut LuminateClient = ptr::null_mut();
    // SAFETY: every call below is documented to validate its arguments
    // before dereferencing the (null) client handle.
    unsafe {
        let mut out_id = ptr::null_mut();
        assert_eq!(
            luminate_client_create_collection(
                client,
                name.as_ptr(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                1,
                &raw mut out_id,
            ),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_add_collection_member(client, id.as_ptr(), ptr::null()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_remove_collection_member(client, id.as_ptr(), ptr::null()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_destroy_collection(client, id.as_ptr()),
            LuminateStatus::NullPointer
        );
        let mut list = ptr::null_mut();
        assert_eq!(
            luminate_client_list_collections(client, &raw mut list),
            LuminateStatus::NullPointer
        );
        let mut snapshot = ptr::null_mut();
        assert_eq!(
            luminate_client_get_collection(client, id.as_ptr(), &raw mut snapshot),
            LuminateStatus::NullPointer
        );
    }
}
