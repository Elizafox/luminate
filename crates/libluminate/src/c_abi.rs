// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

/// Version of the C ABI implemented by this library.
///
/// Increment this when the C surface changes incompatibly. The build script
/// uses the same constant for the ELF SONAME, and cbindgen publishes it as
/// `LUMINATE_C_ABI_VERSION` in the generated header.
///
/// Bumped 4 → 5 when `luminate_device_location` was removed: collections
/// replace `DeviceLocation`-based fan-out entirely, and `LuminateDevice` no
/// longer carries a location to expose. The same version 5 additionally
/// gained the collection CRUD C bindings (`luminate_client_create_collection`
/// and friends, plus
/// `LuminateCollection`/`LuminateCollectionList`/`LuminateCollectionSnapshot`/
/// `LuminateCollectionMember`/`LuminateCollectionMemberInput`): since v5 had
/// not shipped yet, the location removal and the collection C API addition
/// are folded into one coordinated pre-release bump rather than two.
///
/// Bumped 5 → 6 when the client → daemon shared-memory frame-streaming fast
/// path was added:
/// `LuminateShmFrameStream`/`LuminateShmFrameAck` and
/// `luminate_client_begin_shm_frame_stream`/`luminate_client_shm_upload_frame_full`/
/// `luminate_client_end_shm_frame_stream`, plus the new
/// `LUMINATE_EVENT_SHM_STREAM_ENDED` event kind and its
/// `luminate_event_shm_stream_target`/`luminate_event_shm_stream_generation`
/// accessors.
///
/// Bumped 6 → 7 when `Collection.owner_uid` became cross-platform: added
/// `LuminateOwnerKind` and
/// `luminate_collection_owner_kind`/`luminate_collection_owner_sid`;
/// `luminate_collection_owner_uid`'s signature is unchanged but its
/// documented validity narrows to `LuminateOwnerKindUid`.
///
/// Bumped 7 → 8 for the multi-principal authorization façade's C surface:
/// new opaque handles `LuminatePolicyDocument`, `LuminatePolicyDocumentBuilder`,
/// `LuminateMultiUserClient`, `LuminateAuthorizedSession`,
/// `LuminateAuthorizationEvaluation`, `LuminateAuthorizedFrameStream`,
/// `LuminateAuthorizedShmFrameStream`, `LuminateAuthorizedEventSubscription`,
/// `LuminatePolicyProvider`, `LuminatePolicyCompletion`, and
/// `LuminatePolicyCancellation`, plus their bindings. See
/// `docs/development/c-api.md`'s "Authentication, scope, and administration"
/// section for the design.
///
/// Bumped 8 → 9 for pluggable `OwnershipStore` C bindings: new opaque handle
/// `LuminateOwnershipStore` (`luminate_ownership_store_in_memory`/`_new_sync`/
/// `_new_async`/`_free`), reusing `LuminatePolicyCompletion`/
/// `LuminatePolicyCancellation` and gaining
/// `luminate_policy_completion_complete_ownership_get`/`_put`/`_remove` and
/// the generic `luminate_policy_completion_fail`. The same version 9 adds an
/// `ownership_store` parameter to
/// `luminate_multi_user_client_connect_with_policy_provider` and
/// `luminate_multi_user_client_connect_path_with_policy_provider`, an
/// in-place signature change rather than new function variants since neither
/// had shipped outside this repository yet.
///
/// Bumped 9 → 10 for completion-based (non-blocking) session operation
/// calls: a `_async` sibling of every `luminate_authorized_session_*`
/// operation, each taking a `completion_context`/`completion_context_free`/
/// `on_complete` triple instead of blocking the caller. Ten new
/// `Luminate*CompletionFn` typedefs, one per distinct payload shape (mirrors
/// the blocking API's one-out-parameter-shape-per-payload-kind convention);
/// no new opaque handles.
///
/// Bumped 10 → 11 for the typed management API and the accompanying
/// Rust/C feature-parity additions: generic colour inputs, ordinary-client
/// state refresh/save-current and selector mutations, management snapshot
/// views and patch/value builders, and blocking/non-blocking ordinary and
/// authorized management calls.
///
/// Bumped 11 → 12 for runtime policy administration: policy-document
/// read-back accessors, synchronous and asynchronous `LuminatePolicyStore`
/// providers, managed multi-user client constructors, and blocking and
/// completion-based policy reads and replacements. The same unreleased ABI 12
/// development surface later gained additive parity accessors for client path
/// metadata, multi-user metadata and principal groups, device and shared-memory
/// capability fields, and configuration-event change records.
///
/// Bumped 12 → 13 when appearance discriminants gained `Mixed` and collection
/// aggregate state gained its snapshot and accessors.
///
/// Bumped 13 → 14 when ordinary and authorized colour setters were removed,
/// the Static effect constructor began accepting `LuminateColourInput`, and
/// Static gained a borrowed generic-colour accessor.
///
/// Bumped 14 → 15 when capability accessors gained
/// `luminate_capability_set_off_is_wear_safe`.
///
/// Bumped 17 → 18 when the ordinary client gained blocking and asynchronous
/// metadata-free ping endpoints.
///
/// Bumped 18 → 19 for new front-end policy operations and their public C
/// constants.
///
/// Bumped 19 → 20 for the unified single-session client builder,
/// authentication and session-scope configuration, and authenticated-session
/// metadata surface. Before the first release, ABI 20 was completed by
/// removing the superseded multi-user, policy-provider, policy-store, and
/// ownership-store families and adding actor-bound attestation administration.
///
/// Bumped 21 → 22 for typed appearance-slot capabilities, values, state,
/// scene authoring, and mutation entry points.
///
/// Bumped 22 → 23 for the pre-release C API consistency pass: complete colour
/// enumeration, authenticated event subscriptions, opaque server information,
/// borrowed item and effect views, consolidated variant accessors, semantic
/// discriminant typedefs, and removal of the partial asynchronous surface.
///
/// Bumped 23 → 24 for the coherent asynchronous operation surface: every
/// external-I/O operation gained an asynchronous sibling, operation handles
/// gained cancellation and durable diagnostics, client operations became
/// concurrent, and retained client domains corrected dependent-handle
/// lifetimes. `LUMINATE_STATUS_CANCELLED` was added without renumbering the
/// existing status values.
///
/// Bumped 24 → 25 when devices gained borrowed physical-tag count and indexed
/// accessors.
///
/// Bumped 25 → 26 when surface and element physical-tag accessors and
/// structured permission and protocol-incompatibility diagnostic accessors
/// were added. Existing function signatures and discriminant values are
/// unchanged.
///
/// Bumped 26 → 27 for lossless GUI editing round trips: collection aggregate
/// appearance payloads, owned effect and setting-value copies, policy builder
/// seeding and mutations, and seeded scene builders with blocking and
/// asynchronous replacement submission.
///
/// Bumped 27 → 28 when unenforced role quotas were removed from the policy
/// model, including their C input type, builder mutation, and role accessor.
pub const LUMINATE_C_ABI_VERSION: u32 = 28;
