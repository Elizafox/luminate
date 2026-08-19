// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::effects::read_target;
use super::*;

use crate::ffi::sanitize_cstring;

/// One input member for `luminate_client_create_collection`,
/// `luminate_client_add_collection_member`, or
/// `luminate_client_remove_collection_member`: either a concrete target or a
/// nested collection reference.
#[repr(C)]
pub struct LuminateCollectionMemberInput {
    /// `true` if `collection_id` is the populated field (a nested collection
    /// reference); `false` if `target` is the populated field (a concrete
    /// target).
    pub is_collection: bool,
    /// A concrete target; read only when `is_collection` is `false`.
    pub target: LuminateTarget,
    /// A nested collection's id; read only when `is_collection` is `true`.
    pub collection_id: *const c_char,
}

pub(crate) unsafe fn read_collection_member(
    v: &LuminateCollectionMemberInput,
) -> Result<CollectionMember, LuminateStatus> {
    if v.is_collection {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let id = unsafe { read_required_str(v.collection_id, "collection_id") }?;
        Ok(CollectionMember::Collection(CollectionId::new(
            id.to_owned(),
        )))
    } else {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let target = unsafe { read_target(ptr::from_ref(&v.target)) }?;
        Ok(CollectionMember::Target(target))
    }
}

pub(crate) unsafe fn read_members(
    members: *const LuminateCollectionMemberInput,
    count: usize,
) -> Result<Vec<CollectionMember>, LuminateStatus> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if members.is_null() {
        crate::ffi::set_last_error("members pointer is null");
        return Err(LuminateStatus::NullPointer);
    }
    // SAFETY: upheld by the enclosing function's documented C pointer contract:
    // `members` points to `count` valid, readable `LuminateCollectionMemberInput`s.
    let slice = unsafe { std::slice::from_raw_parts(members, count) };
    slice
        .iter()
        // SAFETY: every element of `slice` is a valid `LuminateCollectionMemberInput`.
        .map(|m| unsafe { read_collection_member(m) })
        .collect()
}

/// Creates a collection owned by the calling principal and writes its
/// server-generated id, as a newly allocated C string, to `out_id`.
/// `description` and `kind` may be null. `members` may be null only if
/// `member_count` is 0.
///
/// # Safety
///
/// `client` must be a valid client handle, `name` must point to a valid
/// NUL-terminated UTF-8 string, `description`/`kind` must each be either null
/// or a valid NUL-terminated UTF-8 string, `members` must point to
/// `member_count` valid `LuminateCollectionMemberInput`s (each with a valid
/// `target` or `collection_id` per its `is_collection` flag), and `out_id`
/// must be a valid non-null out-pointer. The returned string must be released
/// with `luminate_string_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_create_collection(
    client: *mut LuminateClient,
    name: *const c_char,
    description: *const c_char,
    kind: *const c_char,
    members: *const LuminateCollectionMemberInput,
    member_count: usize,
    out_id: *mut *mut c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let name = match unsafe { read_required_str(name, "name") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        let description = if description.is_null() {
            None
        } else {
            // SAFETY: upheld by the enclosing function's documented C pointer contract.
            match unsafe { read_required_str(description, "description") } {
                Ok(v) => Some(v.to_owned()),
                Err(e) => return e,
            }
        };
        let kind = if kind.is_null() {
            None
        } else {
            // SAFETY: upheld by the enclosing function's documented C pointer contract.
            match unsafe { read_required_str(kind, "kind") } {
                Ok(v) => Some(CollectionCategory::new(v.to_owned())),
                Err(e) => return e,
            }
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let members = match unsafe { read_members(members, member_count) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let out_id = match unsafe { out_ptr_mut(out_id, "collection id") } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let result = match call_client(client, move |c| async move {
            c.create_collection(name, description, kind, members).await
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(id) => {
                clear_last_error();
                *out_id = sanitize_cstring(id.as_str().to_owned()).into_raw();
                LuminateStatus::Ok
            }
            Err(e) => store_error(&e),
        }
    })
}

/// Destroys a collection. Refused if another collection still references it,
/// or if the caller doesn't own it.
///
/// # Safety
///
/// `client` must be a valid client handle and `id` must point to a valid
/// NUL-terminated UTF-8 string.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_destroy_collection(
    client: *mut LuminateClient,
    id: *const c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let id = match unsafe { read_required_str(id, "collection_id") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        let result = match call_client(client, move |c| async move {
            c.destroy_collection(CollectionId::new(id)).await
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(()) => {
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(e) => store_error(&e),
        }
    })
}

/// Defines a collection-membership client operation over one
/// `LuminateCollectionMemberInput`. `$doc` becomes the generated function's
/// doc comment.
///
// cbindgen can't see through this macro, which is why its expansions
// (`luminate_client_add_collection_member`, `_remove_collection_member`) are
// hand-declared again in the `trailer` of `cbindgen.toml`. Keep the two in
// sync if this macro's signature changes.
macro_rules! collection_member_operation {
    ($doc:literal, $fn:ident, $method:ident) => {
        #[doc = $doc]
        #[must_use]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn(
            client: *mut LuminateClient,
            id: *const c_char,
            member: *const LuminateCollectionMemberInput,
        ) -> LuminateStatus {
            ffi_guard(|| {
                // SAFETY: upheld by the enclosing function's documented C pointer contract.
                let client = match unsafe { client_ref(client) } {
                    Ok(v) => v,
                    Err(e) => return e,
                };
                // SAFETY: upheld by the enclosing function's documented C pointer contract.
                let id = match unsafe { read_required_str(id, "collection_id") } {
                    Ok(v) => v.to_owned(),
                    Err(e) => return e,
                };
                if member.is_null() {
                    crate::ffi::set_last_error("member pointer is null");
                    return LuminateStatus::NullPointer;
                }
                // SAFETY: checked for null above; upheld by the enclosing function's
                // documented C pointer contract.
                let member = match unsafe { read_collection_member(&*member) } {
                    Ok(v) => v,
                    Err(e) => return e,
                };
                let result = match call_client(client, move |c| async move {
                    c.$method(CollectionId::new(id), member).await
                }) {
                    Ok(v) => v,
                    Err(e) => return e,
                };
                match result {
                    Ok(()) => {
                        clear_last_error();
                        LuminateStatus::Ok
                    }
                    Err(e) => store_error(&e),
                }
            })
        }
    };
}
collection_member_operation!(
    "Adds one member to a collection's explicit membership. A no-op if \
     `member` is already present.",
    luminate_client_add_collection_member,
    add_collection_member
);
collection_member_operation!(
    "Removes one member from a collection's explicit membership. A no-op if \
     `member` is absent.",
    luminate_client_remove_collection_member,
    remove_collection_member
);

/// Fetches every registered collection and writes an owned
/// `LuminateCollectionList` to `out`.
///
/// # Safety
///
/// `client` must be a valid client handle and `out` must be a valid non-null
/// out-pointer. The returned list must be released with
/// `luminate_collection_list_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_list_collections(
    client: *mut LuminateClient,
    out: *mut *mut LuminateCollectionList,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let out = match unsafe { out_ptr_mut(out, "collection list") } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let result = match call_client(client, |c| async move { c.list_collections().await }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(v) => {
                *out = Box::into_raw(Box::new(LuminateCollectionList(v)));
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(e) => store_error(&e),
        }
    })
}

/// Fetches a single collection by id and writes an owned
/// `LuminateCollectionSnapshot` to `out`; returns not-found if no such
/// collection exists.
///
/// # Safety
///
/// `client` must be a valid client handle, `id` must point to a valid
/// NUL-terminated UTF-8 string, and `out` must be a valid non-null
/// out-pointer. The returned snapshot must be released with
/// `luminate_collection_snapshot_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_get_collection(
    client: *mut LuminateClient,
    id: *const c_char,
    out: *mut *mut LuminateCollectionSnapshot,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let id = match unsafe { read_required_str(id, "collection_id") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let out = match unsafe { out_ptr_mut(out, "collection snapshot") } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let requested = id.clone();
        let result = match call_client(client, move |c| async move {
            c.get_collection(CollectionId::new(requested)).await
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(Some(v)) => {
                *out = Box::into_raw(Box::new(LuminateCollectionSnapshot(v)));
                clear_last_error();
                LuminateStatus::Ok
            }
            Ok(None) => store_error(&Error::NotFound(format!("collection '{id}'"))),
            Err(e) => store_error(&e),
        }
    })
}

/// Number of collections in the list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_list_count(v: *const LuminateCollectionList) -> usize {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(0, |v| v.0.len())
}

/// Borrowed collection at `index`, or null if out of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_list_at(
    v: *const LuminateCollectionList,
    i: usize,
) -> *const LuminateCollection {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }
        .and_then(|v| v.0.get(i))
        .map_or(ptr::null(), cast_ref)
}

/// The borrowed collection held by this snapshot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_snapshot_collection(
    v: *const LuminateCollectionSnapshot,
) -> *const LuminateCollection {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(ptr::null(), |v| cast_ref(&v.0))
}

str_accessor!(
    "The collection's stable, server-generated identifier.",
    luminate_collection_id,
    LuminateCollection,
    Collection,
    |v: &Collection| v.id.as_str()
);
str_accessor!(
    "The collection's human-readable name.",
    luminate_collection_name,
    LuminateCollection,
    Collection,
    |v: &Collection| v.name.as_str()
);
/// The collection's description, or an absent view if not set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_description(
    v: *const LuminateCollection,
) -> LuminateStringView {
    native_ref!(v, Collection).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| optional_sv(v.description.as_deref()),
    )
}

/// Borrowed identity which owns this collection.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_owner(
    v: *const LuminateCollection,
) -> *const LuminateOwnerIdentity {
    native_ref!(v, Collection).map_or(ptr::null(), |v| cast_ref(&v.owner))
}

/// The collection's presentation-hint category, or an absent view if not
/// set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_kind(
    v: *const LuminateCollection,
) -> LuminateStringView {
    native_ref!(v, Collection).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| optional_sv(v.kind.as_ref().map(CollectionCategory::as_str)),
    )
}
vec_accessors!(
    "Number of members explicitly listed in the collection.",
    "Borrowed member at `index`, or null if out of range.",
    luminate_collection_member_count,
    luminate_collection_member_at,
    LuminateCollection,
    Collection,
    LuminateCollectionMember,
    members
);

/// What this member refers to; one of the `LUMINATE_COLLECTION_MEMBER_*`
/// values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_member_kind(
    v: *const LuminateCollectionMember,
) -> LuminateCollectionMemberKind {
    native_ref!(v, CollectionMember).map_or(u32::MAX, |v| match v {
        CollectionMember::Target(_) => 0,
        CollectionMember::Collection(_) => 1,
    })
}

/// The referenced target, or null unless this member is a concrete target.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_member_target(
    v: *const LuminateCollectionMember,
) -> *const LuminateTargetView {
    native_ref!(v, CollectionMember).map_or(ptr::null(), |v| match v {
        CollectionMember::Target(target) => cast_ref(target),
        CollectionMember::Collection(_) => ptr::null(),
    })
}

/// The referenced collection's id, or an absent view unless this member is a
/// nested collection.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_collection_member_collection_id(
    v: *const LuminateCollectionMember,
) -> LuminateStringView {
    native_ref!(v, CollectionMember).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| match v {
            CollectionMember::Collection(id) => sv(id.as_str()),
            CollectionMember::Target(_) => optional_sv(None),
        },
    )
}

#[cfg(test)]
#[path = "collections_tests.rs"]
mod tests;
