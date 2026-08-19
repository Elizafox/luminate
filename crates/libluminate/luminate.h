/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford */

#ifndef LUMINATE_H
#define LUMINATE_H

#if defined(_WIN32) && defined(LUMINATE_SHARED)
#  if defined(LUMINATE_BUILDING_LIBRARY)
#    define LUMINATE_API __declspec(dllexport)
#  else
#    define LUMINATE_API __declspec(dllimport)
#  endif
#elif defined(__GNUC__) || defined(__clang__)
#  define LUMINATE_API __attribute__((visibility("default")))
#else
#  define LUMINATE_API
#endif

#if defined(__GNUC__) || defined(__clang__)
/* Prefer the GNU spelling even where a standard `[[nodiscard]]` is also
 * available: GCC rejects a `[[nodiscard]]` attribute-specifier following the
 * GNU `__attribute__((visibility(...)))` spelling `LUMINATE_API` expands to
 * on the preceding line, in strict C23/C++17 mode. Both attributes use the
 * GNU spelling here, which composes fine. */
#  define LUMINATE_NODISCARD __attribute__((warn_unused_result))
#elif defined(__cplusplus) && __cplusplus >= 201703L
#  define LUMINATE_NODISCARD [[nodiscard]]
#elif defined(__STDC_VERSION__) && __STDC_VERSION__ >= 202311L
#  define LUMINATE_NODISCARD [[nodiscard]]
#else
#  define LUMINATE_NODISCARD
#endif

#if defined(__cplusplus) && __cplusplus >= 201402L
#  define LUMINATE_DEPRECATED(message) [[deprecated(message)]]
#elif defined(__STDC_VERSION__) && __STDC_VERSION__ >= 202311L
#  define LUMINATE_DEPRECATED(message) [[deprecated(message)]]
#elif defined(__GNUC__) || defined(__clang__)
#  define LUMINATE_DEPRECATED(message) __attribute__((deprecated(message)))
#else
#  define LUMINATE_DEPRECATED(message)
#endif

#ifdef __cplusplus
#  define LUMINATE_BEGIN_DECLS extern "C" {
#  define LUMINATE_END_DECLS }
#  define LUMINATE_STATIC_ASSERT(condition, message) static_assert(condition, message)
#else
#  define LUMINATE_BEGIN_DECLS
#  define LUMINATE_END_DECLS
#  define LUMINATE_STATIC_ASSERT(condition, message) _Static_assert(condition, message)
#endif

#define LUMINATE_REQUIRE_C_ABI(version) \
    LUMINATE_STATIC_ASSERT((version) == LUMINATE_C_ABI_VERSION, \
                           "incompatible libluminate C ABI")

/* Generated with cbindgen:0.29.4 */

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
LUMINATE_BEGIN_DECLS

/**
 * Version of the C ABI implemented by this library.
 *
 * Increment this when the C surface changes incompatibly. The build script
 * uses the same constant for the ELF SONAME, and cbindgen publishes it as
 * `LUMINATE_C_ABI_VERSION` in the generated header.
 *
 * Bumped 4 → 5 when `luminate_device_location` was removed: collections
 * replace `DeviceLocation`-based fan-out entirely, and `LuminateDevice` no
 * longer carries a location to expose. The same version 5 additionally
 * gained the collection CRUD C bindings (`luminate_client_create_collection`
 * and friends, plus
 * `LuminateCollection`/`LuminateCollectionList`/`LuminateCollectionSnapshot`/
 * `LuminateCollectionMember`/`LuminateCollectionMemberInput`): since v5 had
 * not shipped yet, the location removal and the collection C API addition
 * are folded into one coordinated pre-release bump rather than two.
 *
 * Bumped 5 → 6 when the client → daemon shared-memory frame-streaming fast
 * path was added:
 * `LuminateShmFrameStream`/`LuminateShmFrameAck` and
 * `luminate_client_begin_shm_frame_stream`/`luminate_client_shm_upload_frame_full`/
 * `luminate_client_end_shm_frame_stream`, plus the new
 * `LUMINATE_EVENT_SHM_STREAM_ENDED` event kind and its
 * `luminate_event_shm_stream_target`/`luminate_event_shm_stream_generation`
 * accessors.
 *
 * Bumped 6 → 7 when `Collection.owner_uid` became cross-platform: added
 * `LuminateOwnerKind` and
 * `luminate_collection_owner_kind`/`luminate_collection_owner_sid`;
 * `luminate_collection_owner_uid`'s signature is unchanged but its
 * documented validity narrows to `LuminateOwnerKindUid`.
 *
 * Bumped 7 → 8 for the multi-principal authorization façade's C surface:
 * new opaque handles `LuminatePolicyDocument`, `LuminatePolicyDocumentBuilder`,
 * `LuminateMultiUserClient`, `LuminateAuthorizedSession`,
 * `LuminateAuthorizationEvaluation`, `LuminateAuthorizedFrameStream`,
 * `LuminateAuthorizedShmFrameStream`, `LuminateAuthorizedEventSubscription`,
 * `LuminatePolicyProvider`, `LuminatePolicyCompletion`, and
 * `LuminatePolicyCancellation`, plus their bindings. See
 * `docs/development/c-api.md`'s "Authentication, scope, and administration"
 * section for the design.
 *
 * Bumped 8 → 9 for pluggable `OwnershipStore` C bindings: new opaque handle
 * `LuminateOwnershipStore` (`luminate_ownership_store_in_memory`/`_new_sync`/
 * `_new_async`/`_free`), reusing `LuminatePolicyCompletion`/
 * `LuminatePolicyCancellation` and gaining
 * `luminate_policy_completion_complete_ownership_get`/`_put`/`_remove` and
 * the generic `luminate_policy_completion_fail`. The same version 9 adds an
 * `ownership_store` parameter to
 * `luminate_multi_user_client_connect_with_policy_provider` and
 * `luminate_multi_user_client_connect_path_with_policy_provider`, an
 * in-place signature change rather than new function variants since neither
 * had shipped outside this repository yet.
 *
 * Bumped 9 → 10 for completion-based (non-blocking) session operation
 * calls: a `_async` sibling of every `luminate_authorized_session_*`
 * operation, each taking a `completion_context`/`completion_context_free`/
 * `on_complete` triple instead of blocking the caller. Ten new
 * `Luminate*CompletionFn` typedefs, one per distinct payload shape (mirrors
 * the blocking API's one-out-parameter-shape-per-payload-kind convention);
 * no new opaque handles.
 *
 * Bumped 10 → 11 for the typed management API and the accompanying
 * Rust/C feature-parity additions: generic colour inputs, ordinary-client
 * state refresh/save-current and selector mutations, management snapshot
 * views and patch/value builders, and blocking/non-blocking ordinary and
 * authorized management calls.
 *
 * Bumped 11 → 12 for runtime policy administration: policy-document
 * read-back accessors, synchronous and asynchronous `LuminatePolicyStore`
 * providers, managed multi-user client constructors, and blocking and
 * completion-based policy reads and replacements. The same unreleased ABI 12
 * development surface later gained additive parity accessors for client path
 * metadata, multi-user metadata and principal groups, device and shared-memory
 * capability fields, and configuration-event change records.
 *
 * Bumped 12 → 13 when appearance discriminants gained `Mixed` and collection
 * aggregate state gained its snapshot and accessors.
 *
 * Bumped 13 → 14 when ordinary and authorized colour setters were removed,
 * the Static effect constructor began accepting `LuminateColourInput`, and
 * Static gained a borrowed generic-colour accessor.
 *
 * Bumped 14 → 15 when capability accessors gained
 * `luminate_capability_set_off_is_wear_safe`.
 *
 * Bumped 17 → 18 when the ordinary client gained blocking and asynchronous
 * metadata-free ping endpoints.
 *
 * Bumped 18 → 19 for new front-end policy operations and their public C
 * constants.
 *
 * Bumped 19 → 20 for the unified single-session client builder,
 * authentication and session-scope configuration, and authenticated-session
 * metadata surface. Before the first release, ABI 20 was completed by
 * removing the superseded multi-user, policy-provider, policy-store, and
 * ownership-store families and adding actor-bound attestation administration.
 *
 * Bumped 21 → 22 for typed appearance-slot capabilities, values, state,
 * scene authoring, and mutation entry points.
 *
 * Bumped 22 → 23 for the pre-release C API consistency pass: complete colour
 * enumeration, authenticated event subscriptions, opaque server information,
 * borrowed item and effect views, consolidated variant accessors, semantic
 * discriminant typedefs, and removal of the partial asynchronous surface.
 *
 * Bumped 23 → 24 for the coherent asynchronous operation surface: every
 * external-I/O operation gained an asynchronous sibling, operation handles
 * gained cancellation and durable diagnostics, client operations became
 * concurrent, and retained client domains corrected dependent-handle
 * lifetimes. `LUMINATE_STATUS_CANCELLED` was added without renumbering the
 * existing status values.
 *
 * Bumped 24 → 25 when devices gained borrowed physical-tag count and indexed
 * accessors.
 *
 * Bumped 25 → 26 when surface and element physical-tag accessors and
 * structured permission and protocol-incompatibility diagnostic accessors
 * were added. Existing function signatures and discriminant values are
 * unchanged.
 *
 * Bumped 26 → 27 for lossless GUI editing round trips: collection aggregate
 * appearance payloads, owned effect and setting-value copies, policy builder
 * seeding and mutations, and seeded scene builders with blocking and
 * asynchronous replacement submission.
 *
 * Bumped 27 → 28 when unenforced role quotas were removed from the policy
 * model, including their C input type, builder mutation, and role accessor.
 */
#define LUMINATE_C_ABI_VERSION 28

/**
 * Status returned by C API operations.
 *
 * On `Ok`, all documented output pointers have been initialized. On any
 * other status, retrieve the calling thread's diagnostic before issuing
 * another libluminate call on that thread, as it may overwrite the
 * previous one.
 */
enum LuminateStatus
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint32_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  LUMINATE_STATUS_OK = 0,
  LUMINATE_STATUS_NULL_POINTER = 1,
  LUMINATE_STATUS_INVALID_UTF8 = 2,
  LUMINATE_STATUS_DAEMON_UNAVAILABLE = 3,
  LUMINATE_STATUS_INCOMPATIBLE_DAEMON = 4,
  LUMINATE_STATUS_PERMISSION_DENIED = 5,
  LUMINATE_STATUS_NOT_FOUND = 6,
  LUMINATE_STATUS_UNSUPPORTED = 7,
  LUMINATE_STATUS_INVALID_ARGUMENT = 8,
  LUMINATE_STATUS_INTERNAL = 9,
  LUMINATE_STATUS_IO = 10,
  LUMINATE_STATUS_PROTOCOL = 11,
  LUMINATE_STATUS_CONNECTION_POISONED = 12,
  LUMINATE_STATUS_UNKNOWN_STATE = 13,
  LUMINATE_STATUS_TIMEOUT = 14,
  LUMINATE_STATUS_INCOMPATIBLE_EVENT_SOCKET = 15,
  LUMINATE_STATUS_UNAVAILABLE = 16,
  LUMINATE_STATUS_RATE_LIMITED = 17,
  LUMINATE_STATUS_PARTIAL_MUTATION = 18,
  LUMINATE_STATUS_CONFLICT = 19,
  LUMINATE_STATUS_TRANSITION_IMPOSSIBLE = 20,
  LUMINATE_STATUS_AUTHENTICATION_FAILED = 21,
  LUMINATE_STATUS_CANCELLED = 22,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum LuminateStatus LuminateStatus;
#else
typedef uint32_t LuminateStatus;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * Result of trying to cancel a local asynchronous operation.
 */
enum LuminateAsyncCancelResult
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint32_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  LUMINATE_ASYNC_CANCEL_ACCEPTED = 0,
  LUMINATE_ASYNC_CANCEL_ALREADY_CANCELLED = 1,
  LUMINATE_ASYNC_CANCEL_ALREADY_COMPLETED = 2,
  LUMINATE_ASYNC_CANCEL_INVALID = UINT32_MAX,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum LuminateAsyncCancelResult LuminateAsyncCancelResult;
#else
typedef uint32_t LuminateAsyncCancelResult;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * Sanitized authentication source for a connected client.
 */
enum LuminateAuthenticationSource
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint32_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  LUMINATE_AUTHENTICATION_SOURCE_PEER = 0,
  LUMINATE_AUTHENTICATION_SOURCE_BEARER = 1,
  LUMINATE_AUTHENTICATION_SOURCE_ATTESTATION = 2,
  LUMINATE_AUTHENTICATION_SOURCE_EXTERNAL = 3,
  LUMINATE_AUTHENTICATION_SOURCE_UNKNOWN = UINT32_MAX,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum LuminateAuthenticationSource LuminateAuthenticationSource;
#else
typedef uint32_t LuminateAuthenticationSource;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * Kind of owner identity.
 */
enum LuminateOwnerKind
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint32_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  LUMINATE_OWNER_KIND_UID = 0,
  LUMINATE_OWNER_KIND_SID = 1,
  LUMINATE_OWNER_KIND_PRINCIPAL = 2,
  LUMINATE_OWNER_KIND_INVALID = UINT32_MAX,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum LuminateOwnerKind LuminateOwnerKind;
#else
typedef uint32_t LuminateOwnerKind;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * Borrowed view of one facet adoption record within an owning state.
 */
typedef struct LuminateAdoption LuminateAdoption;

/**
 * Owned wear-safe all-off plan; free with `luminate_all_off_plan_free`.
 */
typedef struct LuminateAllOffPlan LuminateAllOffPlan;

/**
 * Borrowed view of appearance operations supported by a slot.
 */
typedef struct LuminateAppearanceCapability LuminateAppearanceCapability;

/**
 * Borrowed view of one appearance-slot descriptor.
 */
typedef struct LuminateAppearanceSlotDescriptor LuminateAppearanceSlotDescriptor;

/**
 * Borrowed view of one appearance-slot value.
 */
typedef struct LuminateAppearanceSlotValue LuminateAppearanceSlotValue;

/**
 * Borrowed view of appearance slots advertised by a surface.
 */
typedef struct LuminateAppearanceSlotsCapability LuminateAppearanceSlotsCapability;

/**
 * Opaque, reference-counted handle for one asynchronous C operation.
 */
typedef struct LuminateAsyncOperation LuminateAsyncOperation;

/**
 * Borrowed attestation metadata within an owning attestation list.
 */
typedef struct LuminateAttestation LuminateAttestation;

typedef struct LuminateAttestationList LuminateAttestationList;

/**
 * An owned authorization decision, returned by
 * `luminate_policy_document_evaluate`.
 */
typedef struct LuminateAuthorizationEvaluation LuminateAuthorizationEvaluation;

/**
 * Borrowed view of the capabilities advertised by a device, surface,
 * element, or group.
 */
typedef struct LuminateCapabilitySet LuminateCapabilitySet;

/**
 * Opaque libluminate client handle.
 *
 * Operations on one `LuminateClient` may be called concurrently. The caller
 * must still ensure `luminate_client_free` does not overlap another use of
 * that exact handle.
 */
typedef struct LuminateClient LuminateClient;

/**
 * Opaque, deep-copying configuration for one client connection.
 */
typedef struct LuminateClientBuilder LuminateClientBuilder;

/**
 * Borrowed view of a collection within an owning
 * `LuminateCollectionList`/`LuminateCollectionSnapshot`.
 */
typedef struct LuminateCollection LuminateCollection;

/**
 * Owned list of every registered collection, returned by
 * `luminate_client_list_collections`; free with
 * `luminate_collection_list_free`.
 */
typedef struct LuminateCollectionList LuminateCollectionList;

/**
 * Borrowed view of one member entry within an owning collection: either a
 * concrete target or a nested collection reference.
 */
typedef struct LuminateCollectionMember LuminateCollectionMember;

typedef struct LuminateCollectionOutcome LuminateCollectionOutcome;

/**
 * Owned single collection, returned by `luminate_client_get_collection`;
 * free with `luminate_collection_snapshot_free`.
 */
typedef struct LuminateCollectionSnapshot LuminateCollectionSnapshot;

/**
 * Owned collection appearance state, returned by
 * `luminate_client_get_collection_state`; free with
 * `luminate_collection_state_snapshot_free`.
 */
typedef struct LuminateCollectionStateSnapshot LuminateCollectionStateSnapshot;

/**
 * Borrowed view of a colour value within an owning facet value.
 */
typedef struct LuminateColour LuminateColour;

/**
 * Borrowed view of one colour capability within an owning capability set.
 */
typedef struct LuminateColourCapability LuminateColourCapability;

/**
 * Borrowed view of one channel within an owning colour capability.
 */
typedef struct LuminateColourChannelCapability LuminateColourChannelCapability;

typedef struct LuminateCreatedAttestation LuminateCreatedAttestation;

typedef struct LuminateCreatedToken LuminateCreatedToken;

/**
 * Borrowed daemon-preferences view.
 */
typedef struct LuminateDaemonPreferences LuminateDaemonPreferences;

/**
 * Borrowed view of a device within an owning topology or device snapshot.
 */
typedef struct LuminateDevice LuminateDevice;

/**
 * Borrowed per-device reconciliation preference.
 */
typedef struct LuminateDeviceReconciliationPreference LuminateDeviceReconciliationPreference;

/**
 * Owned single device, returned by `luminate_client_get_device`; free with
 * `luminate_device_snapshot_free`.
 */
typedef struct LuminateDeviceSnapshot LuminateDeviceSnapshot;

/**
 * Owned effect created by one of the `luminate_effect_create_*` constructors.
 * Free with `luminate_effect_free`.
 */
typedef struct LuminateEffect LuminateEffect;

/**
 * Borrowed view of one choice option within an owning choice-kind effect
 * parameter.
 */
typedef struct LuminateEffectChoice LuminateEffectChoice;

/**
 * Borrowed view of one configurable parameter of a hardware effect
 * descriptor.
 */
typedef struct LuminateEffectParameter LuminateEffectParameter;

/**
 * Borrowed effect within an owning snapshot or list root.
 */
typedef struct LuminateEffectView LuminateEffectView;

/**
 * Borrowed view of an element within an owning surface.
 */
typedef struct LuminateElement LuminateElement;

/**
 * Owned topology- or state-changed event, returned by
 * `luminate_event_subscription_next`; free with `luminate_event_free`.
 */
typedef struct LuminateEvent LuminateEvent;

/**
 * Opaque event subscription handle. Calls on one handle must not overlap.
 */
typedef struct LuminateEventSubscription LuminateEventSubscription;

/**
 * Borrowed view of one observed facet value within an owning state.
 */
typedef struct LuminateFacetObservation LuminateFacetObservation;

/**
 * Borrowed view of an observed or adopted facet's value.
 */
typedef struct LuminateFacetValue LuminateFacetValue;

/**
 * Borrowed view of the frame-upload capability within an owning capability
 * set.
 */
typedef struct LuminateFrameUploadCapability LuminateFrameUploadCapability;

/**
 * Borrowed view of a group within an owning device.
 */
typedef struct LuminateGroup LuminateGroup;

/**
 * Borrowed view of one member entry within an owning group.
 */
typedef struct LuminateGroupMember LuminateGroupMember;

/**
 * Borrowed view of the hardware-effects capability within an owning
 * capability set.
 */
typedef struct LuminateHardwareEffectsCapability LuminateHardwareEffectsCapability;

/**
 * Borrowed managed-plugin view.
 */
typedef struct LuminateManagedPlugin LuminateManagedPlugin;

/**
 * Borrowed management change.
 */
typedef struct LuminateManagementChange LuminateManagementChange;

/**
 * Owned redacted result of an applied management patch.
 */
typedef struct LuminateManagementChangeSet LuminateManagementChangeSet;

/**
 * Borrowed redacted management changes carried by an event.
 */
typedef struct LuminateManagementChangeSetView LuminateManagementChangeSetView;

/**
 * Mutable, reusable atomic management patch builder.
 */
typedef struct LuminateManagementPatchBuilder LuminateManagementPatchBuilder;

/**
 * Owned authoritative management snapshot.
 */
typedef struct LuminateManagementSnapshot LuminateManagementSnapshot;

/**
 * Borrowed collection or scene owner identity.
 */
typedef struct LuminateOwnerIdentity LuminateOwnerIdentity;

/**
 * Borrowed view of the physical-power capability within an owning
 * capability set.
 */
typedef struct LuminatePhysicalPowerCapability LuminatePhysicalPowerCapability;

/**
 * Borrowed plugin-setting schema.
 */
typedef struct LuminatePluginSettingSchema LuminatePluginSettingSchema;

/**
 * Borrowed choice within an owning plugin setup session.
 */
typedef struct LuminatePluginSetupChoice LuminatePluginSetupChoice;

/**
 * Owned snapshot of one plugin setup session.
 */
typedef struct LuminatePluginSetupSession LuminatePluginSetupSession;

/**
 * Borrowed setup workflow view.
 */
typedef struct LuminatePluginSetupWorkflow LuminatePluginSetupWorkflow;

/**
 * Owned list of setup workflows advertised by one installed plugin.
 */
typedef struct LuminatePluginSetupWorkflowList LuminatePluginSetupWorkflowList;

/**
 * Borrowed binding within an owning policy document.
 */
typedef struct LuminatePolicyBinding LuminatePolicyBinding;

/**
 * A validated, immutable access policy document, built by
 * `luminate_policy_document_build` and evaluated by
 * `luminate_policy_document_evaluate`.
 */
typedef struct LuminatePolicyDocument LuminatePolicyDocument;

/**
 * A mutable, deep-copying builder for a [`LuminatePolicyDocument`].
 *
 * Every setter/adder validates and copies its input immediately; the
 * builder never borrows caller memory past the call that filled it.
 * `luminate_policy_document_build` does not consume the builder, so it may
 * keep being edited and rebuilt (for example after bumping its revision).
 */
typedef struct LuminatePolicyDocumentBuilder LuminatePolicyDocumentBuilder;

/**
 * Borrowed role within an owning policy document.
 */
typedef struct LuminatePolicyRole LuminatePolicyRole;

/**
 * Borrowed rule within an owning policy role.
 */
typedef struct LuminatePolicyRule LuminatePolicyRule;

/**
 * Borrowed view of the power domain reference within an owning capability
 * set.
 */
typedef struct LuminatePowerDomainRef LuminatePowerDomainRef;

/**
 * Borrowed view of one readable facet within an owning capability set's
 * state-readback capability.
 */
typedef struct LuminateReadableFacet LuminateReadableFacet;

/**
 * An owned remote principal: exact opaque authority, subject, and verified
 * group strings, with no credentials or arbitrary claims.
 */
typedef struct LuminateRemotePrincipal LuminateRemotePrincipal;

/**
 * Borrowed reported setting value.
 */
typedef struct LuminateReportedSettingValue LuminateReportedSettingValue;

/**
 * Borrowed view of a scene.
 */
typedef struct LuminateScene LuminateScene;

/**
 * Borrowed view of one scene binding.
 */
typedef struct LuminateSceneBinding LuminateSceneBinding;

/**
 * Mutable deep-copying editor seeded from an existing scene.
 */
typedef struct LuminateSceneBuilder LuminateSceneBuilder;

/**
 * Owned list of observable scenes.
 */
typedef struct LuminateSceneList LuminateSceneList;

/**
 * Owned single scene snapshot.
 */
typedef struct LuminateSceneSnapshot LuminateSceneSnapshot;

/**
 * Owned server metadata returned by `luminate_client_server_info`.
 */
typedef struct LuminateServerInfo LuminateServerInfo;

/**
 * Owned snapshot of sanitized authenticated-session metadata.
 */
typedef struct LuminateSessionMetadata LuminateSessionMetadata;

/**
 * Opaque, deep-copying builder for an allow-only session scope.
 */
typedef struct LuminateSessionScopeBuilder LuminateSessionScopeBuilder;

/**
 * Owned recursive plugin setting value.
 */
typedef struct LuminateSettingValue LuminateSettingValue;

/**
 * Borrowed recursive setting value.
 */
typedef struct LuminateSettingValueView LuminateSettingValueView;

/**
 * Borrowed view of the shared-memory fast-path capability within an owning
 * frame-upload capability.
 */
typedef struct LuminateShmFrameCapability LuminateShmFrameCapability;

/**
 * Owned client-published shared-memory frame stream, returned by
 * `luminate_client_begin_shm_frame_stream`; ended (and freed) with
 * `luminate_client_end_shm_frame_stream`.
 */
typedef struct LuminateShmFrameStream LuminateShmFrameStream;

/**
 * Borrowed view of a device's state within an owning state snapshot.
 */
typedef struct LuminateState LuminateState;

/**
 * Owned device state, returned by `luminate_client_get_state`; free with
 * `luminate_state_snapshot_free`.
 */
typedef struct LuminateStateSnapshot LuminateStateSnapshot;

/**
 * Borrowed view of a surface within an owning device.
 */
typedef struct LuminateSurface LuminateSurface;

/**
 * Borrowed view of the target an observation or adoption record describes.
 */
typedef struct LuminateTargetView LuminateTargetView;

/**
 * Borrowed token metadata within an owning token list.
 */
typedef struct LuminateToken LuminateToken;

typedef struct LuminateTokenList LuminateTokenList;

/**
 * Owned list of all devices, returned by `luminate_client_list_devices` or a
 * baseline subscribe call; free with `luminate_topology_snapshot_free`.
 */
typedef struct LuminateTopologySnapshot LuminateTopologySnapshot;

/**
 * Owned transition status snapshot.
 */
typedef struct LuminateTransitionSnapshot LuminateTransitionSnapshot;

/**
 * Owned list of withdrawn device identifiers, returned by
 * `luminate_client_list_withdrawn_devices`.
 */
typedef struct LuminateWithdrawnDeviceList LuminateWithdrawnDeviceList;

/**
 * A length-suffixed string.
 */
typedef struct LuminateStringView {
  const char *data;
  uintptr_t len;
} LuminateStringView;

/**
 * A luminate target.
 */
typedef struct LuminateTarget {
  const char *device_id;
  const char *surface_id;
  const char *element_id;
  const char *group_id;
} LuminateTarget;

/**
 * Releases a consumer-owned asynchronous completion context.
 */
typedef void (*LuminateCompletionContextFreeFn)(void *context);

/**
 * Completes an asynchronous server-information request.
 */
typedef void (*LuminateAsyncServerInfoCompletionFn)(void *context,
                                                    const struct LuminateAsyncOperation *operation,
                                                    LuminateStatus status,
                                                    struct LuminateServerInfo *payload);

/**
 * Completes an asynchronous operation with no payload.
 */
typedef void (*LuminateAsyncStatusCompletionFn)(void *context,
                                                const struct LuminateAsyncOperation *operation,
                                                LuminateStatus status);

/**
 * Completes an asynchronous topology request.
 */
typedef void (*LuminateAsyncTopologySnapshotCompletionFn)(void *context,
                                                          const struct LuminateAsyncOperation *operation,
                                                          LuminateStatus status,
                                                          struct LuminateTopologySnapshot *payload);

/**
 * Completes an asynchronous withdrawn-device request.
 */
typedef void (*LuminateAsyncWithdrawnDeviceListCompletionFn)(void *context,
                                                             const struct LuminateAsyncOperation *operation,
                                                             LuminateStatus status,
                                                             struct LuminateWithdrawnDeviceList *payload);

/**
 * Completes an asynchronous attestation creation.
 */
typedef void (*LuminateAsyncCreatedAttestationCompletionFn)(void *context,
                                                            const struct LuminateAsyncOperation *operation,
                                                            LuminateStatus status,
                                                            struct LuminateCreatedAttestation *payload);

/**
 * Completes an asynchronous attestation-list request.
 */
typedef void (*LuminateAsyncAttestationListCompletionFn)(void *context,
                                                         const struct LuminateAsyncOperation *operation,
                                                         LuminateStatus status,
                                                         struct LuminateAttestationList *payload);

/**
 * Completes an asynchronous policy-document request.
 */
typedef void (*LuminateAsyncPolicyDocumentCompletionFn)(void *context,
                                                        const struct LuminateAsyncOperation *operation,
                                                        LuminateStatus status,
                                                        struct LuminatePolicyDocument *payload);

/**
 * Completes an asynchronous token creation or rotation.
 */
typedef void (*LuminateAsyncCreatedTokenCompletionFn)(void *context,
                                                      const struct LuminateAsyncOperation *operation,
                                                      LuminateStatus status,
                                                      struct LuminateCreatedToken *payload);

/**
 * Completes an asynchronous token-list request.
 */
typedef void (*LuminateAsyncTokenListCompletionFn)(void *context,
                                                   const struct LuminateAsyncOperation *operation,
                                                   LuminateStatus status,
                                                   struct LuminateTokenList *payload);

/**
 * One input member for `luminate_client_create_collection`,
 * `luminate_client_add_collection_member`, or
 * `luminate_client_remove_collection_member`: either a concrete target or a
 * nested collection reference.
 */
typedef struct LuminateCollectionMemberInput {
  /**
   * `true` if `collection_id` is the populated field (a nested collection
   * reference); `false` if `target` is the populated field (a concrete
   * target).
   */
  bool is_collection;
  /**
   * A concrete target; read only when `is_collection` is `false`.
   */
  struct LuminateTarget target;
  /**
   * A nested collection's id; read only when `is_collection` is `true`.
   */
  const char *collection_id;
} LuminateCollectionMemberInput;

/**
 * Completes an asynchronous operation returning an owned C string.
 */
typedef void (*LuminateAsyncStringCompletionFn)(void *context,
                                                const struct LuminateAsyncOperation *operation,
                                                LuminateStatus status,
                                                char *payload);

/**
 * Completes an asynchronous collection-list request.
 */
typedef void (*LuminateAsyncCollectionListCompletionFn)(void *context,
                                                        const struct LuminateAsyncOperation *operation,
                                                        LuminateStatus status,
                                                        struct LuminateCollectionList *payload);

/**
 * Completes an asynchronous collection request.
 */
typedef void (*LuminateAsyncCollectionSnapshotCompletionFn)(void *context,
                                                            const struct LuminateAsyncOperation *operation,
                                                            LuminateStatus status,
                                                            struct LuminateCollectionSnapshot *payload);

/**
 * One slot assignment supplied to a mutation or scene definition.
 */
typedef struct LuminateAppearanceSlotInput {
  const char *slot_id;
  const struct LuminateEffect *effect;
} LuminateAppearanceSlotInput;

/**
 * Whether a target is emitting light.
 */
typedef uint32_t LuminateEmissionState;

/**
 * Sparse state input for one scene binding.
 */
typedef struct LuminateSceneTargetStateInput {
  /**
   * Optional owned/borrowed effect handle. `Effect::Off` is rejected.
   */
  const struct LuminateEffect *appearance;
  bool has_brightness;
  uint32_t brightness;
  bool has_emission;
  /**
   * One of the `LUMINATE_EMISSION_*` values.
   */
  LuminateEmissionState emission;
  const struct LuminateAppearanceSlotInput *appearance_slots;
  uintptr_t appearance_slot_count;
} LuminateSceneTargetStateInput;

/**
 * One explicitly-authored scene binding.
 */
typedef struct LuminateSceneBindingInput {
  /**
   * Null for frozen binding; otherwise the gating collection id.
   */
  const char *dynamic_collection_id;
  struct LuminateTarget target;
  struct LuminateSceneTargetStateInput state;
} LuminateSceneBindingInput;

/**
 * Completes an asynchronous scene request.
 */
typedef void (*LuminateAsyncSceneSnapshotCompletionFn)(void *context,
                                                       const struct LuminateAsyncOperation *operation,
                                                       LuminateStatus status,
                                                       struct LuminateSceneSnapshot *payload);

/**
 * Completes an asynchronous scene-list request.
 */
typedef void (*LuminateAsyncSceneListCompletionFn)(void *context,
                                                   const struct LuminateAsyncOperation *operation,
                                                   LuminateStatus status,
                                                   struct LuminateSceneList *payload);

/**
 * One target, collection, or explicit target list. `kind` selects `target`
 * (0), `collection_id` (1), or `targets`/`target_count` (2).
 */
typedef struct LuminateSelectorInput {
  uint32_t kind;
  struct LuminateTarget target;
  const char *collection_id;
  const struct LuminateTarget *targets;
  uintptr_t target_count;
} LuminateSelectorInput;

/**
 * Completes an asynchronous selector mutation.
 */
typedef void (*LuminateAsyncCollectionOutcomeCompletionFn)(void *context,
                                                           const struct LuminateAsyncOperation *operation,
                                                           LuminateStatus status,
                                                           struct LuminateCollectionOutcome *payload);

/**
 * Completes an asynchronous operation returning a generation number.
 */
typedef void (*LuminateAsyncU32CompletionFn)(void *context,
                                             const struct LuminateAsyncOperation *operation,
                                             LuminateStatus status,
                                             uint32_t value);

/**
 * An RGB triplet.
 */
typedef struct LuminateRgb {
  uint8_t r;
  uint8_t g;
  uint8_t b;
} LuminateRgb;

/**
 * The daemon's response to one uploaded frame, mirroring [`crate::FrameAck`].
 */
typedef struct LuminateFrameAck {
  /**
   * The sequence number the daemon accepted (echoes the uploaded frame's).
   */
  uint64_t sequence;
  /**
   * Nonzero if the frame was accepted but rate-limited, not forwarded to
   * the plugin.
   */
  uint8_t dropped;
} LuminateFrameAck;

/**
 * Completes an asynchronous frame upload.
 */
typedef void (*LuminateAsyncFrameAckCompletionFn)(void *context,
                                                  const struct LuminateAsyncOperation *operation,
                                                  LuminateStatus status,
                                                  struct LuminateFrameAck value);

/**
 * Completes asynchronous shared-memory stream negotiation.
 */
typedef void (*LuminateAsyncShmFrameStreamCompletionFn)(void *context,
                                                        const struct LuminateAsyncOperation *operation,
                                                        LuminateStatus status,
                                                        struct LuminateShmFrameStream *stream);

/**
 * Completes an asynchronous management snapshot request.
 */
typedef void (*LuminateAsyncManagementSnapshotCompletionFn)(void *context,
                                                            const struct LuminateAsyncOperation *operation,
                                                            LuminateStatus status,
                                                            struct LuminateManagementSnapshot *payload);

/**
 * Completes an asynchronous management patch.
 */
typedef void (*LuminateAsyncManagementChangeSetCompletionFn)(void *context,
                                                             const struct LuminateAsyncOperation *operation,
                                                             LuminateStatus status,
                                                             struct LuminateManagementChangeSet *payload);

/**
 * Completes an asynchronous plugin setup session operation.
 */
typedef void (*LuminateAsyncPluginSetupSessionCompletionFn)(void *context,
                                                            const struct LuminateAsyncOperation *operation,
                                                            LuminateStatus status,
                                                            struct LuminatePluginSetupSession *payload);

/**
 * Completes an asynchronous plugin setup workflow request.
 */
typedef void (*LuminateAsyncPluginSetupWorkflowListCompletionFn)(void *context,
                                                                 const struct LuminateAsyncOperation *operation,
                                                                 LuminateStatus status,
                                                                 struct LuminatePluginSetupWorkflowList *payload);

/**
 * Transition configuration. Zero `step_interval_ms` selects the default.
 */
typedef struct LuminateTransitionOptions {
  uint64_t duration_ms;
  uint64_t step_interval_ms;
  /**
   * One of the `LUMINATE_TRANSITION_FUNCTION_*` values.
   */
  uint32_t function;
  /**
   * One of the `LUMINATE_TRANSITION_COLOUR_*` values.
   */
  uint32_t colour_interpolation;
  /**
   * One of the `LUMINATE_HUE_DIRECTION_*` values. Must be shortest for
   * `OKLab` interpolation.
   */
  uint32_t hue_direction;
} LuminateTransitionOptions;

/**
 * Completes an asynchronous transition operation.
 */
typedef void (*LuminateAsyncTransitionSnapshotCompletionFn)(void *context,
                                                            const struct LuminateAsyncOperation *operation,
                                                            LuminateStatus status,
                                                            struct LuminateTransitionSnapshot *payload);

/**
 * One concrete ephemeral transition destination.
 */
typedef struct LuminateTransitionTargetStateInput {
  struct LuminateTarget target;
  struct LuminateSceneTargetStateInput state;
} LuminateTransitionTargetStateInput;

typedef void (*LuminateAsyncClientCompletionFn)(void *context,
                                                const struct LuminateAsyncOperation *operation,
                                                LuminateStatus status,
                                                struct LuminateClient *client);

/**
 * Completes an asynchronous event-subscription operation.
 */
typedef void (*LuminateAsyncEventSubscriptionCompletionFn)(void *context,
                                                           const struct LuminateAsyncOperation *operation,
                                                           LuminateStatus status,
                                                           struct LuminateEventSubscription *subscription);

/**
 * Completes an asynchronous subscription-and-baseline operation.
 */
typedef void (*LuminateAsyncSubscriptionBaselineCompletionFn)(void *context,
                                                              const struct LuminateAsyncOperation *operation,
                                                              LuminateStatus status,
                                                              struct LuminateEventSubscription *subscription,
                                                              struct LuminateTopologySnapshot *topology);

/**
 * Completes an asynchronous event wait.
 */
typedef void (*LuminateAsyncEventCompletionFn)(void *context,
                                               const struct LuminateAsyncOperation *operation,
                                               LuminateStatus status,
                                               struct LuminateEvent *event);

/**
 * A semantic operation evaluated by an access policy.
 */
typedef uint32_t LuminatePolicyOperation;

/**
 * Optional constraints applied to every resource a rule is checked against.
 * An empty array field (`*_count == 0`) means "no constraint on this
 * dimension"; `values` need not be non-null in that case.
 */
typedef struct LuminateResourceConstraintsInput {
  const char *const *device_ids;
  uintptr_t device_id_count;
  const char *const *provider_instances;
  uintptr_t provider_instance_count;
  bool has_host_attached;
  bool host_attached;
  const char *const *collections;
  uintptr_t collection_count;
} LuminateResourceConstraintsInput;

/**
 * Whether appearance-slot mutations may omit values.
 */
typedef uint32_t LuminateAppearanceSlotUpdatePolicy;

typedef uint32_t LuminateCctEmulation;

/**
 * Granularity at which a capability applies.
 */
typedef uint32_t LuminateCapabilityScope;

/**
 * What persistence a device offers.
 */
typedef uint32_t LuminatePersistenceKind;

/**
 * Whether persistence is optional or mandatory.
 */
typedef uint32_t LuminatePersistenceRequirement;

/**
 * Whether device state can be read back.
 */
typedef uint32_t LuminateStateReadbackKind;

/**
 * How colour channels are encoded.
 */
typedef uint32_t LuminateColourEncoding;

/**
 * Identity of one colour channel.
 */
typedef uint32_t LuminateColourChannel;

/**
 * Which frame update styles a target accepts.
 */
typedef uint32_t LuminateFrameUpdateMode;

/**
 * How uploaded frames are committed.
 */
typedef uint32_t LuminateBufferingMode;

/**
 * Packed shared-memory pixel encoding.
 */
typedef uint32_t LuminateShmPixelFormat;

/**
 * Shared-memory pixel-buffer layout.
 */
typedef uint32_t LuminateShmFrameShapeKind;

/**
 * Which effect-parameter payload is populated.
 */
typedef uint32_t LuminateEffectParameterKind;

/**
 * An unsigned 16-bit range.
 */
typedef struct LuminateU16Range {
  uint16_t minimum;
  uint16_t maximum;
  uint16_t step;
} LuminateU16Range;

/**
 * A supported effect motion direction.
 */
typedef uint32_t LuminateEffectDirection;

/**
 * An unsigned 32-bit range.
 */
typedef struct LuminateU32Range {
  uint32_t minimum;
  uint32_t maximum;
  uint32_t step;
} LuminateU32Range;

/**
 * A readable or observable state facet.
 */
typedef uint32_t LuminateStateFacetKind;

/**
 * How trustworthy a readback is.
 */
typedef uint32_t LuminateReadbackFidelity;

/**
 * What a power-domain reference points at.
 */
typedef uint32_t LuminatePowerDomainKind;

/**
 * What a collection member refers to.
 */
typedef uint32_t LuminateCollectionMemberKind;

typedef struct LuminateColourChannelInput {
  LuminateColourChannel channel;
  uint32_t value;
} LuminateColourChannelInput;

typedef struct LuminateColourInput {
  LuminateColourEncoding encoding;
  const struct LuminateColourChannelInput *channels;
  uintptr_t channel_count;
} LuminateColourInput;

/**
 * Which payload an event carries.
 */
typedef uint32_t LuminateEventKind;

typedef uint32_t LuminateUnsupportedPolicy;

typedef uint32_t LuminateReconciliationPolicy;

/**
 * One per-device reconciliation preference used by a patch mutation.
 */
typedef struct LuminateDeviceReconciliationPreferenceInput {
  const char *device_id;
  LuminateReconciliationPolicy policy;
} LuminateDeviceReconciliationPreferenceInput;

/**
 * Optional daemon preferences used by a patch mutation.
 */
typedef struct LuminateDaemonPreferencesInput {
  bool has_default_unsupported_policy;
  LuminateUnsupportedPolicy default_unsupported_policy;
  bool has_reconciliation_policy;
  LuminateReconciliationPolicy reconciliation_policy;
  const struct LuminateDeviceReconciliationPreferenceInput *device_reconciliation;
  uintptr_t device_reconciliation_count;
  bool has_cct_emulation;
  LuminateCctEmulation cct_emulation;
  bool has_prefer_shm;
  bool prefer_shm;
  bool has_prefer_client_shm;
  bool prefer_client_shm;
} LuminateDaemonPreferencesInput;

typedef uint32_t LuminatePluginRuntimeStateKind;

typedef uint32_t LuminatePluginSettingKind;

typedef uint32_t LuminateReportedSettingKind;

typedef uint32_t LuminateManagementSettingKind;

typedef uint32_t LuminateManagementChangeKind;

/**
 * Surface shape.
 */
typedef uint32_t LuminateSurfaceKind;

/**
 * A single point.
 */
typedef struct LuminatePoint {
  float x;
  float y;
} LuminatePoint;

/**
 * A matrix cell.
 */
typedef struct LuminateMatrixCell {
  uint16_t row;
  uint16_t column;
} LuminateMatrixCell;

/**
 * Element role within a surface.
 */
typedef uint32_t LuminateElementKind;

/**
 * Which field of an element's geometry union is populated.
 */
typedef uint32_t LuminateGeometryKind;

/**
 * A rectangle.
 */
typedef struct LuminateRect {
  float x;
  float y;
  float width;
  float height;
} LuminateRect;

/**
 * How a group was created and is managed.
 */
typedef uint32_t LuminateGroupKind;

/**
 * What a group member refers to.
 */
typedef uint32_t LuminateGroupMemberKind;

/**
 * Whether a matching policy rule allows or denies.
 */
typedef uint32_t LuminateRuleEffect;

/**
 * One input rule for `luminate_policy_document_builder_role_add_rule`.
 * `effect` is one of the `LUMINATE_RULE_EFFECT_*` values and each entry of
 * `operations` is one of the `LUMINATE_POLICY_OP_*` values. `reason` may be
 * null. `cache_hint_ms` is read only when `has_cache_hint_ms` is `true`, and
 * is clamped to `luminate_core::policy::MAX_CACHE_HINT` by validation, not
 * by this input struct.
 */
typedef struct LuminateRuleInput {
  const char *id;
  LuminateRuleEffect effect;
  const LuminatePolicyOperation *operations;
  uintptr_t operation_count;
  struct LuminateResourceConstraintsInput resources;
  const char *reason;
  bool has_cache_hint_ms;
  uint64_t cache_hint_ms;
} LuminateRuleInput;

/**
 * One resource an evaluated request would affect. `provider_instance` may
 * be null. `collections` may be null when `collection_count` is `0`.
 */
typedef struct LuminateResourceInput {
  const char *device_id;
  const char *provider_instance;
  bool host_attached;
  const char *const *collections;
  uintptr_t collection_count;
} LuminateResourceInput;

typedef uint32_t LuminatePluginSetupSessionState;

typedef uint32_t LuminatePluginSetupWorkflowKind;

/**
 * The daemon's response to one published sample, mirroring
 * [`super::frame::LuminateFrameAck`]. There is no `dropped` field: the fast
 * path is fire-and-forget, so this only ever confirms the local publish
 * succeeded, never that the daemon (let alone the plugin) has applied it.
 */
typedef struct LuminateShmFrameAck {
  /**
   * The sequence number this client assigned and just published.
   */
  uint64_t sequence;
} LuminateShmFrameAck;

/**
 * Whether a device is currently reachable.
 */
typedef uint32_t LuminateReachability;

/**
 * A device's reconciliation lifecycle status.
 */
typedef uint32_t LuminateReconciliationStatus;

/**
 * How trustworthy an observation is.
 */
typedef uint32_t LuminateObservationConfidence;

/**
 * Where an observation came from.
 */
typedef uint32_t LuminateObservationSource;

/**
 * Whether an adopted baseline was accepted.
 */
typedef uint32_t LuminateAdoptionStatus;

/**
 * What a target view addresses.
 */
typedef uint32_t LuminateTargetKind;

/**
 * Which appearance payload is populated.
 */
typedef uint32_t LuminateAppearanceKind;

/**
 * Which effective-appearance payload is populated.
 */
typedef uint32_t LuminateEffectiveAppearanceKind;

/**
 * Whether a target's physical power is on.
 */
typedef uint32_t LuminatePhysicalPowerState;

#define LUMINATE_MANAGEMENT_SETTING_BOOLEAN 0

#define LUMINATE_MANAGEMENT_SETTING_INTEGER 1

#define LUMINATE_MANAGEMENT_SETTING_NUMBER 2

#define LUMINATE_MANAGEMENT_SETTING_STRING 3

#define LUMINATE_MANAGEMENT_SETTING_ARRAY 4

#define LUMINATE_MANAGEMENT_SETTING_TABLE 5

#define LUMINATE_REPORTED_SETTING_UNSET 0

#define LUMINATE_REPORTED_SETTING_VISIBLE 1

#define LUMINATE_REPORTED_SETTING_REDACTED 2

#define LUMINATE_PLUGIN_SETTING_BOOLEAN 0

#define LUMINATE_PLUGIN_SETTING_INTEGER 1

#define LUMINATE_PLUGIN_SETTING_NUMBER 2

#define LUMINATE_PLUGIN_SETTING_STRING 3

#define LUMINATE_PLUGIN_SETTING_ENUMERATION 4

#define LUMINATE_PLUGIN_SETTING_ARRAY 5

#define LUMINATE_PLUGIN_RUNTIME_INACTIVE 0

#define LUMINATE_PLUGIN_RUNTIME_LOADING 1

#define LUMINATE_PLUGIN_RUNTIME_LOADED 2

#define LUMINATE_PLUGIN_RUNTIME_FAILED 3

#define LUMINATE_MANAGEMENT_CHANGE_DAEMON_PREFERENCES 0

#define LUMINATE_MANAGEMENT_CHANGE_PLUGIN_ACTIVATION 1

#define LUMINATE_MANAGEMENT_CHANGE_PLUGIN_RECONCILIATION 2

#define LUMINATE_MANAGEMENT_CHANGE_PLUGIN_SETTING 3

#define LUMINATE_RECONCILIATION_POLICY_RESTORE 0

#define LUMINATE_RECONCILIATION_POLICY_ADOPT 1

#define LUMINATE_RECONCILIATION_POLICY_LEAVE 2

#define LUMINATE_UNSUPPORTED_POLICY_SKIP 0

#define LUMINATE_UNSUPPORTED_POLICY_REJECT 1

#define LUMINATE_CCT_EMULATION_AUTO 0

#define LUMINATE_CCT_EMULATION_DISABLED 1

#define LUMINATE_PLUGIN_SETUP_PROVISION 0

#define LUMINATE_PLUGIN_SETUP_REPAIR 1

#define LUMINATE_PLUGIN_SETUP_DISCOVER 2

#define LUMINATE_PLUGIN_SETUP_IMPORT 3

#define LUMINATE_PLUGIN_SETUP_FACTORY_PROVISION 4

#define LUMINATE_PLUGIN_SETUP_CHOICE 0

#define LUMINATE_PLUGIN_SETUP_PHYSICAL_ACTION 1

#define LUMINATE_PLUGIN_SETUP_COMPLETED 2

#define LUMINATE_PLUGIN_SETUP_FAILED 3

#define LUMINATE_PLUGIN_SETUP_CANCELLED 4

#define LUMINATE_PLUGIN_SETUP_APPLYING 5

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

/**
 * Connects to the default daemon socket.
 *
 * # Safety
 *
 * `out_client` must be a valid non-null out-pointer. The returned handle must
 * later be released with `luminate_client_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_connect(struct LuminateClient **out_client);

/**
 * Connects to a daemon socket at a specific path.
 *
 * # Safety
 *
 * `path` must point to a valid NUL-terminated UTF-8 string. `out_client` must
 * be a valid non-null out-pointer. The returned handle must later be released
 * with `luminate_client_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_connect_path(const char *path,
                                            struct LuminateClient **out_client);

/**
 * Releases a client handle returned by `luminate_client_connect*`.
 *
 * # Safety
 *
 * `client` must either be null or a valid handle previously returned by
 * `luminate_client_connect` or `luminate_client_connect_path`. It must not be
 * freed more than once.
 */
LUMINATE_API void luminate_client_free(struct LuminateClient *client);

/**
 * Releases string memory allocated by libluminate.
 *
 * # Safety
 *
 * `value` must either be null or a pointer returned by libluminate that was
 * documented as requiring `luminate_string_free`.
 */
LUMINATE_API void luminate_string_free(char *value);

/**
 * Releases an owned `LuminateServerInfo`. Null is a no-op.
 *
 * # Safety
 *
 * `info` must be null or a pointer returned by `luminate_client_server_info`.
 */
LUMINATE_API void luminate_server_info_free(struct LuminateServerInfo *info);

LUMINATE_API
struct LuminateStringView luminate_server_info_daemon_name(const struct LuminateServerInfo *info);

LUMINATE_API
struct LuminateStringView luminate_server_info_daemon_version(const struct LuminateServerInfo *info);

LUMINATE_API
uint32_t luminate_server_info_protocol_abi_version(const struct LuminateServerInfo *info);

/**
 * Returns the libluminate version string.
 *
 * The returned pointer is a static NUL-terminated string valid for the
 * lifetime of the process; it must not be freed.
 */
LUMINATE_API const char *luminate_version(void);

/**
 * Returns a borrowed pointer to the last error message for the current thread,
 * or null if none has been recorded.
 *
 * # Lifetime and invalidation
 *
 * The returned pointer **borrows** thread-local storage and stays valid only
 * until the next libluminate call *on this same thread*. Any such call may
 * overwrite or clear the message and free the buffer this pointer refers to.
 * Do not retain it, share it across threads, or use it after another
 * libluminate call; copy the string out first if you need to keep it. For a
 * caller that cannot uphold that, use [`luminate_copy_last_error_message`],
 * which copies into a buffer you own and has no lifetime hazard.
 */
LUMINATE_API const char *luminate_last_error_message(void);

/**
 * Retrieves retry guidance for the current thread's last error.
 *
 * Returns `true` and writes milliseconds to `out_retry_after_ms` when the
 * provider supplied guidance. Returns `false` for other errors or a null
 * output pointer. Like the message, this metadata is replaced by the next
 * libluminate call on this thread.
 */
LUMINATE_API bool luminate_last_error_retry_after_ms(uint64_t *out_retry_after_ms);

/**
 * Returns the number of targets applied before the current thread's last
 * partial mutation error.
 */
LUMINATE_API uintptr_t luminate_last_error_applied_target_count(void);

/**
 * Retrieves one borrowed target from the current thread's last partial
 * mutation error.
 *
 * Returns `false` for an out-of-range index or null output pointer. Component
 * strings follow the same nullability rules as mutation input targets and
 * remain valid only until the next libluminate call on this thread.
 */
LUMINATE_API
bool luminate_last_error_applied_target(uintptr_t index,
                                        struct LuminateTarget *out_target);

/**
 * Returns the safe policy reason from the current permission-denied error.
 */
LUMINATE_API struct LuminateStringView luminate_last_error_permission_denied_reason(void);

/**
 * Returns the daemon version from the current incompatibility error.
 */
LUMINATE_API struct LuminateStringView luminate_last_error_incompatible_daemon_version(void);

/**
 * Returns the optional reason from the current incompatibility error.
 */
LUMINATE_API struct LuminateStringView luminate_last_error_incompatibility_reason(void);

/**
 * Retrieves the supported primary protocol ABI from an incompatibility error.
 *
 * Returns `false` and leaves the output unchanged for other errors or a null
 * output pointer.
 */
LUMINATE_API bool luminate_last_error_supported_protocol_abi_version(uint32_t *out_version);

/**
 * Retrieves the supported event protocol version from an incompatibility error.
 *
 * Returns `false` and leaves the output unchanged for other errors or a null
 * output pointer.
 */
LUMINATE_API bool luminate_last_error_supported_event_protocol_version(uint32_t *out_version);

/**
 * Copies the current thread's last error message into a caller-owned
 * buffer, always NUL-terminating it, and returns the total number of
 * bytes required (including the terminator).
 *
 * This is the lifetime-safe alternative to
 * [`luminate_last_error_message`]. Because the caller owns the
 * destination buffer, no pointer into libluminate-managed storage is
 * exposed. If no error is recorded, the message is empty (a single NUL),
 * so the return value is 1.
 *
 * If `buf` is null or `buf_len` is 0, nothing is written and the
 * required length is still returned. This allows callers to query the
 * required buffer size before allocating.
 *
 * If the message (including its terminator) does not fit, it is
 * truncated to `buf_len` bytes and still NUL-terminated. A return value
 * greater than `buf_len` indicates that truncation occurred.
 *
 * # Safety
 *
 * If `buf` is non-null, it must point to at least `buf_len` writable
 * bytes.
 */
LUMINATE_API uintptr_t luminate_copy_last_error_message(char *buf, uintptr_t buf_len);

/**
 * Asynchronously fetches server information.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_server_info_async(struct LuminateClient *client,
                                                 void *completion_context,
                                                 LuminateCompletionContextFreeFn completion_context_free,
                                                 LuminateAsyncServerInfoCompletionFn on_complete,
                                                 struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously purges one withdrawn device.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_purge_withdrawn_device_async(struct LuminateClient *client,
                                                            const char *device_id,
                                                            void *completion_context,
                                                            LuminateCompletionContextFreeFn completion_context_free,
                                                            LuminateAsyncStatusCompletionFn on_complete,
                                                            struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously lists current devices.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_list_devices_async(struct LuminateClient *client,
                                                  void *completion_context,
                                                  LuminateCompletionContextFreeFn completion_context_free,
                                                  LuminateAsyncTopologySnapshotCompletionFn on_complete,
                                                  struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously lists withdrawn device identifiers.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_list_withdrawn_devices_async(struct LuminateClient *client,
                                                            void *completion_context,
                                                            LuminateCompletionContextFreeFn completion_context_free,
                                                            LuminateAsyncWithdrawnDeviceListCompletionFn on_complete,
                                                            struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously refreshes one device's state.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_refresh_state_async(struct LuminateClient *client,
                                                   const char *device_id,
                                                   void *completion_context,
                                                   LuminateCompletionContextFreeFn completion_context_free,
                                                   LuminateAsyncStatusCompletionFn on_complete,
                                                   struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously creates an identity-only attestation.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_create_attestation_async(const struct LuminateClient *client,
                                                        const char *name,
                                                        const char *authority,
                                                        const char *subject,
                                                        bool has_expiry,
                                                        uint64_t expires_at_unix_ms,
                                                        void *completion_context,
                                                        LuminateCompletionContextFreeFn completion_context_free,
                                                        LuminateAsyncCreatedAttestationCompletionFn on_complete,
                                                        struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously creates a principal attestation with verified groups.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_create_principal_attestation_async(const struct LuminateClient *client,
                                                                  const char *name,
                                                                  const char *authority,
                                                                  const char *subject,
                                                                  const char *const *verified_groups,
                                                                  uintptr_t verified_group_count,
                                                                  bool has_expiry,
                                                                  uint64_t expires_at_unix_ms,
                                                                  void *completion_context,
                                                                  LuminateCompletionContextFreeFn completion_context_free,
                                                                  LuminateAsyncCreatedAttestationCompletionFn on_complete,
                                                                  struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously lists actor-bound attestations.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_list_attestations_async(const struct LuminateClient *client,
                                                       void *completion_context,
                                                       LuminateCompletionContextFreeFn completion_context_free,
                                                       LuminateAsyncAttestationListCompletionFn on_complete,
                                                       struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously revokes an actor-bound attestation.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_revoke_attestation_async(const struct LuminateClient *client,
                                                        const char *name,
                                                        void *completion_context,
                                                        LuminateCompletionContextFreeFn completion_context_free,
                                                        LuminateAsyncStatusCompletionFn on_complete,
                                                        struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously reads the active access policy.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_get_access_policy_async(const struct LuminateClient *client,
                                                       void *completion_context,
                                                       LuminateCompletionContextFreeFn completion_context_free,
                                                       LuminateAsyncPolicyDocumentCompletionFn on_complete,
                                                       struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously revision-replaces the active access policy.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_replace_access_policy_async(const struct LuminateClient *client,
                                                           uint64_t expected_revision,
                                                           const struct LuminatePolicyDocument *replacement,
                                                           void *completion_context,
                                                           LuminateCompletionContextFreeFn completion_context_free,
                                                           LuminateAsyncPolicyDocumentCompletionFn on_complete,
                                                           struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously creates a display-once bearer token.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_create_token_async(const struct LuminateClient *client,
                                                  const char *id,
                                                  const char *authority,
                                                  const char *subject,
                                                  bool has_expiry,
                                                  uint64_t expires_at_unix_ms,
                                                  void *completion_context,
                                                  LuminateCompletionContextFreeFn completion_context_free,
                                                  LuminateAsyncCreatedTokenCompletionFn on_complete,
                                                  struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously lists bearer-token metadata.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_list_tokens_async(const struct LuminateClient *client,
                                                 void *completion_context,
                                                 LuminateCompletionContextFreeFn completion_context_free,
                                                 LuminateAsyncTokenListCompletionFn on_complete,
                                                 struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously revokes a bearer token.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_revoke_token_async(const struct LuminateClient *client,
                                                  const char *id,
                                                  void *completion_context,
                                                  LuminateCompletionContextFreeFn completion_context_free,
                                                  LuminateAsyncStatusCompletionFn on_complete,
                                                  struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously rotates a bearer token and returns its replacement secret.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_rotate_token_async(const struct LuminateClient *client,
                                                  const char *id,
                                                  bool has_expiry,
                                                  uint64_t expires_at_unix_ms,
                                                  void *completion_context,
                                                  LuminateCompletionContextFreeFn completion_context_free,
                                                  LuminateAsyncCreatedTokenCompletionFn on_complete,
                                                  struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously creates a collection.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_create_collection_async(struct LuminateClient *client,
                                                       const char *name,
                                                       const char *description,
                                                       const char *kind,
                                                       const struct LuminateCollectionMemberInput *members,
                                                       uintptr_t member_count,
                                                       void *completion_context,
                                                       LuminateCompletionContextFreeFn completion_context_free,
                                                       LuminateAsyncStringCompletionFn on_complete,
                                                       struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously destroys a collection.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_destroy_collection_async(struct LuminateClient *client,
                                                        const char *id,
                                                        void *completion_context,
                                                        LuminateCompletionContextFreeFn completion_context_free,
                                                        LuminateAsyncStatusCompletionFn on_complete,
                                                        struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously adds one explicit collection member.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_add_collection_member_async(struct LuminateClient *client,
                                                           const char *id,
                                                           const struct LuminateCollectionMemberInput *member,
                                                           void *completion_context,
                                                           LuminateCompletionContextFreeFn completion_context_free,
                                                           LuminateAsyncStatusCompletionFn on_complete,
                                                           struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously removes one explicit collection member.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_remove_collection_member_async(struct LuminateClient *client,
                                                              const char *id,
                                                              const struct LuminateCollectionMemberInput *member,
                                                              void *completion_context,
                                                              LuminateCompletionContextFreeFn completion_context_free,
                                                              LuminateAsyncStatusCompletionFn on_complete,
                                                              struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously lists collections.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_list_collections_async(struct LuminateClient *client,
                                                      void *completion_context,
                                                      LuminateCompletionContextFreeFn completion_context_free,
                                                      LuminateAsyncCollectionListCompletionFn on_complete,
                                                      struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously fetches one collection.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_get_collection_async(struct LuminateClient *client,
                                                    const char *id,
                                                    void *completion_context,
                                                    LuminateCompletionContextFreeFn completion_context_free,
                                                    LuminateAsyncCollectionSnapshotCompletionFn on_complete,
                                                    struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously applies logical appearance slots to a concrete surface.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_set_appearance_slots_async(struct LuminateClient *client,
                                                          const struct LuminateTarget *target,
                                                          const struct LuminateAppearanceSlotInput *values,
                                                          uintptr_t value_count,
                                                          void *completion_context,
                                                          LuminateCompletionContextFreeFn completion_context_free,
                                                          LuminateAsyncStatusCompletionFn on_complete,
                                                          struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously creates an explicitly authored scene.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_create_scene_async(struct LuminateClient *client,
                                                  const char *name,
                                                  const char *description,
                                                  const struct LuminateSceneBindingInput *bindings,
                                                  uintptr_t binding_count,
                                                  void *completion_context,
                                                  LuminateCompletionContextFreeFn completion_context_free,
                                                  LuminateAsyncSceneSnapshotCompletionFn on_complete,
                                                  struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously captures intended state into a new scene.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_capture_scene_async(struct LuminateClient *client,
                                                   const char *name,
                                                   const char *description,
                                                   const char *dynamic_collection_id,
                                                   const struct LuminateTarget *targets,
                                                   uintptr_t target_count,
                                                   void *completion_context,
                                                   LuminateCompletionContextFreeFn completion_context_free,
                                                   LuminateAsyncSceneSnapshotCompletionFn on_complete,
                                                   struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously replaces an explicitly authored scene.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_replace_scene_async(struct LuminateClient *client,
                                                   const char *id,
                                                   uint64_t expected_revision,
                                                   const char *name,
                                                   const char *description,
                                                   const struct LuminateSceneBindingInput *bindings,
                                                   uintptr_t binding_count,
                                                   void *completion_context,
                                                   LuminateCompletionContextFreeFn completion_context_free,
                                                   LuminateAsyncSceneSnapshotCompletionFn on_complete,
                                                   struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously replaces a scene from an independent seeded builder. The
 * builder is copied during submission and may be mutated or freed after this
 * function returns.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_replace_scene_from_builder_async(struct LuminateClient *client,
                                                                const struct LuminateSceneBuilder *builder,
                                                                void *completion_context,
                                                                LuminateCompletionContextFreeFn completion_context_free,
                                                                LuminateAsyncSceneSnapshotCompletionFn on_complete,
                                                                struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously recaptures intended state for an existing scene.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_recapture_scene_async(struct LuminateClient *client,
                                                     const char *id,
                                                     uint64_t expected_revision,
                                                     const char *dynamic_collection_id,
                                                     const struct LuminateTarget *targets,
                                                     uintptr_t target_count,
                                                     void *completion_context,
                                                     LuminateCompletionContextFreeFn completion_context_free,
                                                     LuminateAsyncSceneSnapshotCompletionFn on_complete,
                                                     struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously deletes a scene at an expected revision.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_delete_scene_async(struct LuminateClient *client,
                                                  const char *id,
                                                  uint64_t expected_revision,
                                                  void *completion_context,
                                                  LuminateCompletionContextFreeFn completion_context_free,
                                                  LuminateAsyncStatusCompletionFn on_complete,
                                                  struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously lists observable scenes.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_list_scenes_async(struct LuminateClient *client,
                                                 void *completion_context,
                                                 LuminateCompletionContextFreeFn completion_context_free,
                                                 LuminateAsyncSceneListCompletionFn on_complete,
                                                 struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously fetches one observable scene.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_get_scene_async(struct LuminateClient *client,
                                               const char *id,
                                               void *completion_context,
                                               LuminateCompletionContextFreeFn completion_context_free,
                                               LuminateAsyncSceneSnapshotCompletionFn on_complete,
                                               struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously applies a scene immediately.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_apply_scene_async(struct LuminateClient *client,
                                                 const char *id,
                                                 void *completion_context,
                                                 LuminateCompletionContextFreeFn completion_context_free,
                                                 LuminateAsyncStatusCompletionFn on_complete,
                                                 struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously applies an effect to one target.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_set_effect_async(struct LuminateClient *client,
                                                const struct LuminateTarget *target,
                                                const struct LuminateEffect *effect,
                                                void *completion_context,
                                                LuminateCompletionContextFreeFn completion_context_free,
                                                LuminateAsyncStatusCompletionFn on_complete,
                                                struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously changes one target's emission state.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_set_emission_async(struct LuminateClient *client,
                                                  const struct LuminateTarget *target,
                                                  uint32_t state,
                                                  void *completion_context,
                                                  LuminateCompletionContextFreeFn completion_context_free,
                                                  LuminateAsyncStatusCompletionFn on_complete,
                                                  struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously sets brightness through a selector.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_set_brightness_selector_async(struct LuminateClient *client,
                                                             const struct LuminateSelectorInput *selector,
                                                             uint32_t value,
                                                             bool has_policy,
                                                             uint32_t policy,
                                                             void *completion_context,
                                                             LuminateCompletionContextFreeFn completion_context_free,
                                                             LuminateAsyncCollectionOutcomeCompletionFn on_complete,
                                                             struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously applies an effect through a selector.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_set_effect_selector_async(struct LuminateClient *client,
                                                         const struct LuminateSelectorInput *selector,
                                                         const struct LuminateEffect *effect,
                                                         bool has_policy,
                                                         uint32_t policy,
                                                         void *completion_context,
                                                         LuminateCompletionContextFreeFn completion_context_free,
                                                         LuminateAsyncCollectionOutcomeCompletionFn on_complete,
                                                         struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously changes emission state through a selector.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_set_emission_selector_async(struct LuminateClient *client,
                                                           const struct LuminateSelectorInput *selector,
                                                           uint32_t state,
                                                           void *completion_context,
                                                           LuminateCompletionContextFreeFn completion_context_free,
                                                           LuminateAsyncCollectionOutcomeCompletionFn on_complete,
                                                           struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously begins an ordinary frame stream.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_begin_frame_stream_async(struct LuminateClient *client,
                                                        const struct LuminateTarget *target,
                                                        void *completion_context,
                                                        LuminateCompletionContextFreeFn completion_context_free,
                                                        LuminateAsyncU32CompletionFn on_complete,
                                                        struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously uploads a full ordinary frame.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_upload_frame_full_async(struct LuminateClient *client,
                                                       const struct LuminateTarget *target,
                                                       uint32_t generation,
                                                       uint64_t sequence,
                                                       const struct LuminateRgb *colours,
                                                       uintptr_t count,
                                                       uint8_t commit,
                                                       void *completion_context,
                                                       LuminateCompletionContextFreeFn completion_context_free,
                                                       LuminateAsyncFrameAckCompletionFn on_complete,
                                                       struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously uploads a sparse ordinary frame.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_upload_frame_partial_async(struct LuminateClient *client,
                                                          const struct LuminateTarget *target,
                                                          uint32_t generation,
                                                          uint64_t sequence,
                                                          const uint32_t *indices,
                                                          const struct LuminateRgb *colours,
                                                          uintptr_t count,
                                                          uint8_t commit,
                                                          void *completion_context,
                                                          LuminateCompletionContextFreeFn completion_context_free,
                                                          LuminateAsyncFrameAckCompletionFn on_complete,
                                                          struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously ends an ordinary frame stream.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_end_frame_stream_async(struct LuminateClient *client,
                                                      const struct LuminateTarget *target,
                                                      uint32_t generation,
                                                      void *completion_context,
                                                      LuminateCompletionContextFreeFn completion_context_free,
                                                      LuminateAsyncStatusCompletionFn on_complete,
                                                      struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously negotiates a client-published shared-memory frame stream.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_begin_shm_frame_stream_async(struct LuminateClient *client,
                                                            const struct LuminateTarget *target,
                                                            void *completion_context,
                                                            LuminateCompletionContextFreeFn completion_context_free,
                                                            LuminateAsyncShmFrameStreamCompletionFn on_complete,
                                                            struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously ends and consumes a shared-memory frame stream.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_end_shm_frame_stream_async(struct LuminateShmFrameStream *stream,
                                                          void *completion_context,
                                                          LuminateCompletionContextFreeFn completion_context_free,
                                                          LuminateAsyncStatusCompletionFn on_complete,
                                                          struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously reads authoritative daemon management state.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_get_management_async(struct LuminateClient *client,
                                                    void *completion_context,
                                                    LuminateCompletionContextFreeFn completion_context_free,
                                                    LuminateAsyncManagementSnapshotCompletionFn on_complete,
                                                    struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously applies an atomic management patch.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_patch_management_async(struct LuminateClient *client,
                                                      const struct LuminateManagementPatchBuilder *patch,
                                                      void *completion_context,
                                                      LuminateCompletionContextFreeFn completion_context_free,
                                                      LuminateAsyncManagementChangeSetCompletionFn on_complete,
                                                      struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously starts one plugin setup workflow.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_plugin_setup_start_async(struct LuminateClient *client,
                                                        const char *plugin,
                                                        const char *workflow,
                                                        void *completion_context,
                                                        LuminateCompletionContextFreeFn completion_context_free,
                                                        LuminateAsyncPluginSetupSessionCompletionFn on_complete,
                                                        struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously selects one plugin setup choice.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_plugin_setup_choose_async(struct LuminateClient *client,
                                                         const char *session_id,
                                                         uint64_t generation,
                                                         const char *choice,
                                                         void *completion_context,
                                                         LuminateCompletionContextFreeFn completion_context_free,
                                                         LuminateAsyncPluginSetupSessionCompletionFn on_complete,
                                                         struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously confirms a plugin setup physical action.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_plugin_setup_confirm_async(struct LuminateClient *client,
                                                          const char *session_id,
                                                          uint64_t generation,
                                                          void *completion_context,
                                                          LuminateCompletionContextFreeFn completion_context_free,
                                                          LuminateAsyncPluginSetupSessionCompletionFn on_complete,
                                                          struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously reads one plugin setup session.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_plugin_setup_get_async(struct LuminateClient *client,
                                                      const char *session_id,
                                                      void *completion_context,
                                                      LuminateCompletionContextFreeFn completion_context_free,
                                                      LuminateAsyncPluginSetupSessionCompletionFn on_complete,
                                                      struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously cancels one plugin setup session.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_plugin_setup_cancel_async(struct LuminateClient *client,
                                                         const char *session_id,
                                                         void *completion_context,
                                                         LuminateCompletionContextFreeFn completion_context_free,
                                                         LuminateAsyncPluginSetupSessionCompletionFn on_complete,
                                                         struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously lists one plugin's setup workflows.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_plugin_setup_workflows_async(struct LuminateClient *client,
                                                            const char *plugin,
                                                            void *completion_context,
                                                            LuminateCompletionContextFreeFn completion_context_free,
                                                            LuminateAsyncPluginSetupWorkflowListCompletionFn on_complete,
                                                            struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously starts a scene-to-scene transition.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_transition_scene_to_scene_async(struct LuminateClient *client,
                                                               const char *source_scene_id,
                                                               const char *destination_scene_id,
                                                               struct LuminateTransitionOptions timing,
                                                               void *completion_context,
                                                               LuminateCompletionContextFreeFn completion_context_free,
                                                               LuminateAsyncTransitionSnapshotCompletionFn on_complete,
                                                               struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously starts a current-to-scene transition.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_transition_current_to_scene_async(struct LuminateClient *client,
                                                                 const char *destination_scene_id,
                                                                 struct LuminateTransitionOptions timing,
                                                                 void *completion_context,
                                                                 LuminateCompletionContextFreeFn completion_context_free,
                                                                 LuminateAsyncTransitionSnapshotCompletionFn on_complete,
                                                                 struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously starts a scene-to-ephemeral-state transition.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_transition_scene_to_states_async(struct LuminateClient *client,
                                                                const char *source_scene_id,
                                                                const struct LuminateTransitionTargetStateInput *states,
                                                                uintptr_t state_count,
                                                                struct LuminateTransitionOptions timing,
                                                                void *completion_context,
                                                                LuminateCompletionContextFreeFn completion_context_free,
                                                                LuminateAsyncTransitionSnapshotCompletionFn on_complete,
                                                                struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously starts a current-to-ephemeral-state transition.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_transition_current_to_states_async(struct LuminateClient *client,
                                                                  const struct LuminateTransitionTargetStateInput *states,
                                                                  uintptr_t state_count,
                                                                  struct LuminateTransitionOptions timing,
                                                                  void *completion_context,
                                                                  LuminateCompletionContextFreeFn completion_context_free,
                                                                  LuminateAsyncTransitionSnapshotCompletionFn on_complete,
                                                                  struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously connects to the default daemon socket.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_connect_async(void *completion_context,
                                             LuminateCompletionContextFreeFn completion_context_free,
                                             LuminateAsyncClientCompletionFn on_complete,
                                             struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously connects to an explicit daemon socket path.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_connect_path_async(const char *path,
                                                  void *completion_context,
                                                  LuminateCompletionContextFreeFn completion_context_free,
                                                  LuminateAsyncClientCompletionFn on_complete,
                                                  struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously connects using a deep copy of a client builder.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_builder_connect_async(const struct LuminateClientBuilder *builder,
                                                     void *completion_context,
                                                     LuminateCompletionContextFreeFn completion_context_free,
                                                     LuminateAsyncClientCompletionFn on_complete,
                                                     struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously subscribes using the client's derived event path.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_subscribe_async(struct LuminateClient *client,
                                               void *completion_context,
                                               LuminateCompletionContextFreeFn completion_context_free,
                                               LuminateAsyncEventSubscriptionCompletionFn on_complete,
                                               struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously subscribes using an explicit event socket path.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_subscribe_path_async(struct LuminateClient *client,
                                                    const char *path,
                                                    void *completion_context,
                                                    LuminateCompletionContextFreeFn completion_context_free,
                                                    LuminateAsyncEventSubscriptionCompletionFn on_complete,
                                                    struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously subscribes and retrieves a topology baseline.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_subscribe_with_baseline_async(struct LuminateClient *client,
                                                             void *completion_context,
                                                             LuminateCompletionContextFreeFn completion_context_free,
                                                             LuminateAsyncSubscriptionBaselineCompletionFn on_complete,
                                                             struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously subscribes on an explicit path and retrieves a baseline.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_subscribe_with_baseline_path_async(struct LuminateClient *client,
                                                                  const char *path,
                                                                  void *completion_context,
                                                                  LuminateCompletionContextFreeFn completion_context_free,
                                                                  LuminateAsyncSubscriptionBaselineCompletionFn on_complete,
                                                                  struct LuminateAsyncOperation **out_operation);

/**
 * Asynchronously waits for the next event on a subscription.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_event_subscription_next_async(struct LuminateEventSubscription *subscription,
                                                      void *completion_context,
                                                      LuminateCompletionContextFreeFn completion_context_free,
                                                      LuminateAsyncEventCompletionFn on_complete,
                                                      struct LuminateAsyncOperation **out_operation);

/**
 * Retains an asynchronous operation handle. Null returns null.
 *
 * # Safety
 *
 * `operation` must be null or a live operation handle.
 */
LUMINATE_API
struct LuminateAsyncOperation *luminate_async_operation_retain(const struct LuminateAsyncOperation *operation);

/**
 * Releases an asynchronous operation reference. Null is a no-op.
 *
 * # Safety
 *
 * `operation` must be null or one owned reference to a live operation handle.
 */
LUMINATE_API void luminate_async_operation_release(struct LuminateAsyncOperation *operation);

/**
 * Requests deterministic local cancellation of an asynchronous operation.
 *
 * # Safety
 *
 * `operation` must be null or a live operation handle.
 */
LUMINATE_API
LuminateAsyncCancelResult luminate_async_operation_cancel(struct LuminateAsyncOperation *operation);

/**
 * Retrieves the terminal status, leaving `out_status` unchanged while pending.
 *
 * # Safety
 *
 * Both pointers must be live for the duration of the call when non-null.
 */
LUMINATE_API
bool luminate_async_operation_status(const struct LuminateAsyncOperation *operation,
                                     LuminateStatus *out_status);

/**
 * Returns the durable error message borrowed from a failed operation.
 *
 * # Safety
 *
 * `operation` must be null or a live operation handle.
 */
LUMINATE_API
struct LuminateStringView luminate_async_operation_error_message(const struct LuminateAsyncOperation *operation);

/**
 * Returns the durable safe policy reason from a permission-denied operation.
 *
 * # Safety
 *
 * `operation` must be null or a live operation handle.
 */
LUMINATE_API
struct LuminateStringView luminate_async_operation_error_permission_denied_reason(const struct LuminateAsyncOperation *operation);

/**
 * Returns the durable daemon version from an incompatible operation.
 *
 * # Safety
 *
 * `operation` must be null or a live operation handle.
 */
LUMINATE_API
struct LuminateStringView luminate_async_operation_error_incompatible_daemon_version(const struct LuminateAsyncOperation *operation);

/**
 * Returns the durable optional reason from an incompatible operation.
 *
 * # Safety
 *
 * `operation` must be null or a live operation handle.
 */
LUMINATE_API
struct LuminateStringView luminate_async_operation_error_incompatibility_reason(const struct LuminateAsyncOperation *operation);

/**
 * Retrieves the durable supported primary protocol ABI version.
 *
 * Returns `false` without changing the output for other errors or nulls.
 *
 * # Safety
 *
 * Both pointers must be live for the duration of the call when non-null.
 */
LUMINATE_API
bool luminate_async_operation_error_supported_protocol_abi_version(const struct LuminateAsyncOperation *operation,
                                                                   uint32_t *out_version);

/**
 * Retrieves the durable supported event protocol version.
 *
 * Returns `false` without changing the output for other errors or nulls.
 *
 * # Safety
 *
 * Both pointers must be live for the duration of the call when non-null.
 */
LUMINATE_API
bool luminate_async_operation_error_supported_event_protocol_version(const struct LuminateAsyncOperation *operation,
                                                                     uint32_t *out_version);

/**
 * Retrieves durable retry guidance from a failed operation.
 *
 * # Safety
 *
 * Both pointers must be live for the duration of the call when non-null.
 */
LUMINATE_API
bool luminate_async_operation_error_retry_after_ms(const struct LuminateAsyncOperation *operation,
                                                   uint64_t *out_retry_after_ms);

/**
 * Returns the number of durable applied targets on a failed operation.
 *
 * # Safety
 *
 * `operation` must be null or a live operation handle.
 */
LUMINATE_API
uintptr_t luminate_async_operation_error_applied_target_count(const struct LuminateAsyncOperation *operation);

/**
 * Retrieves one durable applied target borrowed from a failed operation.
 *
 * # Safety
 *
 * Both pointers must be live for the duration of the call when non-null.
 */
LUMINATE_API
bool luminate_async_operation_error_applied_target_at(const struct LuminateAsyncOperation *operation,
                                                      uintptr_t index,
                                                      struct LuminateTarget *out_target);

/**
 * Creates a client builder using peer authentication and the default socket.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_builder_new(struct LuminateClientBuilder **out_builder);

/**
 * Releases a client builder. Null is a no-op.
 */
LUMINATE_API void luminate_client_builder_free(struct LuminateClientBuilder *builder);

/**
 * Selects peer authentication.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_builder_authenticate_peer(struct LuminateClientBuilder *builder);

/**
 * Selects daemon bearer-token authentication and deep-copies the credential.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_builder_authenticate_bearer(struct LuminateClientBuilder *builder,
                                                           const uint8_t *credential,
                                                           uintptr_t credential_len);

/**
 * Selects a socket path and deep-copies it.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_builder_set_path(struct LuminateClientBuilder *builder,
                                                const char *path);

/**
 * Selects actor-bound attestation authentication and deep-copies its inputs.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_builder_authenticate_attestation(struct LuminateClientBuilder *builder,
                                                                const char *name,
                                                                const uint8_t *credential,
                                                                uintptr_t credential_len);

/**
 * Selects an external authentication provider and deep-copies its inputs.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_builder_authenticate_external(struct LuminateClientBuilder *builder,
                                                             const char *provider,
                                                             const uint8_t *credential,
                                                             uintptr_t credential_len);

/**
 * Adds one unconstrained, allow-only operation grant to the session scope.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_builder_add_scope_operation(struct LuminateClientBuilder *builder,
                                                           LuminatePolicyOperation operation);

/**
 * Creates an empty session-scope builder.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_session_scope_builder_new(struct LuminateSessionScopeBuilder **out_scope);

/**
 * Releases a session-scope builder. Null is a no-op.
 */
LUMINATE_API void luminate_session_scope_builder_free(struct LuminateSessionScopeBuilder *scope);

/**
 * Adds one unconstrained multi-operation grant, deep-copying the operation array.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_session_scope_builder_add_grant(struct LuminateSessionScopeBuilder *scope,
                                                        const LuminatePolicyOperation *operations,
                                                        uintptr_t operation_count);

/**
 * Adds one constrained multi-operation grant, deep-copying every input.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_session_scope_builder_add_constrained_grant(struct LuminateSessionScopeBuilder *scope,
                                                                    const LuminatePolicyOperation *operations,
                                                                    uintptr_t operation_count,
                                                                    const struct LuminateResourceConstraintsInput *resources);

/**
 * Deep-copies a validated scope into the client builder.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_builder_set_scope(struct LuminateClientBuilder *builder,
                                                 const struct LuminateSessionScopeBuilder *scope);

/**
 * Connects without consuming the builder.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_builder_connect(const struct LuminateClientBuilder *builder,
                                               struct LuminateClient **out_client);

/**
 * Copies this client's sanitized session metadata into an owned snapshot.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_get_session_metadata(const struct LuminateClient *client,
                                                    struct LuminateSessionMetadata **out_metadata);

/**
 * Releases a session metadata snapshot. Null is a no-op.
 */
LUMINATE_API void luminate_session_metadata_free(struct LuminateSessionMetadata *metadata);

/**
 * Returns the authenticated authority from a metadata snapshot.
 */
LUMINATE_API
struct LuminateStringView luminate_session_metadata_authority(const struct LuminateSessionMetadata *metadata);

/**
 * Returns the authenticated subject from a metadata snapshot.
 */
LUMINATE_API
struct LuminateStringView luminate_session_metadata_subject(const struct LuminateSessionMetadata *metadata);

/**
 * Returns the number of verified authentication groups.
 */
LUMINATE_API
uintptr_t luminate_session_metadata_group_count(const struct LuminateSessionMetadata *metadata);

/**
 * Returns one verified authentication group, or an empty view out of range.
 */
LUMINATE_API
struct LuminateStringView luminate_session_metadata_group_at(const struct LuminateSessionMetadata *metadata,
                                                             uintptr_t index);

/**
 * Returns the authentication-source discriminant.
 */
LUMINATE_API
LuminateAuthenticationSource luminate_session_metadata_source(const struct LuminateSessionMetadata *metadata);

/**
 * Returns the attestation/provider name when the source has one.
 */
LUMINATE_API
struct LuminateStringView luminate_session_metadata_source_name(const struct LuminateSessionMetadata *metadata);

/**
 * Returns the non-secret credential ID when one exists.
 */
LUMINATE_API
struct LuminateStringView luminate_session_metadata_credential_id(const struct LuminateSessionMetadata *metadata);

/**
 * Writes the expiry in Unix milliseconds and returns whether one exists.
 */
LUMINATE_API
bool luminate_session_metadata_expires_at_unix_ms(const struct LuminateSessionMetadata *metadata,
                                                  uint64_t *out_expiry);

/**
 * Subscribes to daemon events using the event path derived from this client's
 * primary socket.
 *
 * # Safety
 *
 * `client` must be valid and `out_subscription` must be a valid non-null
 * out-pointer. Release the returned handle with
 * `luminate_event_subscription_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_subscribe(struct LuminateClient *client,
                                         struct LuminateEventSubscription **out_subscription);

/**
 * Subscribes to an explicit daemon event socket using this client's
 * authenticated, single-use event ticket.
 *
 * # Safety
 *
 * `path` must be a valid NUL-terminated UTF-8 string and
 * `out_subscription` a valid non-null out-pointer. Release the returned
 * handle with `luminate_event_subscription_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_subscribe_path(struct LuminateClient *client,
                                              const char *path,
                                              struct LuminateEventSubscription **out_subscription);

/**
 * Releases an event subscription handle.
 *
 * # Safety
 *
 * `subscription` must be null or a uniquely owned valid handle and must not
 * be freed more than once.
 */
LUMINATE_API void luminate_event_subscription_free(struct LuminateEventSubscription *subscription);

/**
 * Retrieves the connected daemon version string as a newly allocated C string.
 *
 * # Safety
 *
 * `client` must be a valid client handle. `out_version` must be a valid
 * non-null out-pointer. The returned string must be released with
 * `luminate_string_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_daemon_version(struct LuminateClient *client,
                                              char **out_version);

/**
 * Retrieves the primary daemon socket path as a newly allocated C string.
 *
 * # Safety
 *
 * `client` must be a valid client handle. `out_path` must be a valid non-null
 * out-pointer. The returned string must be released with
 * `luminate_string_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_socket_path(struct LuminateClient *client,
                                           char **out_path);

/**
 * Retrieves the conventional event socket path as a newly allocated C string.
 *
 * # Safety
 *
 * `client` must be a valid client handle. `out_path` must be a valid non-null
 * out-pointer. The returned string must be released with
 * `luminate_string_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_event_socket_path(struct LuminateClient *client,
                                                 char **out_path);

/**
 * Fetches server information as a newly allocated owned object.
 *
 * # Safety
 *
 * `client` must be a valid client handle. `out_info` must be a valid non-null
 * out-pointer. The returned object must be released with
 * `luminate_server_info_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_server_info(struct LuminateClient *client,
                                           struct LuminateServerInfo **out_info);

/**
 * Checks that the client's existing daemon connection is responsive.
 *
 * The daemon returns no metadata and performs no authorization. Calls are
 * subject to a small per-connection rate limit.
 *
 * # Safety
 *
 * `client` must be a valid client handle.
 */
LUMINATE_API LUMINATE_NODISCARD LuminateStatus luminate_client_ping(struct LuminateClient *client);

/**
 * Permanently removes retained state for a withdrawn device.
 *
 * The daemon rejects active devices. This operation changes daemon persistence
 * only and never sends a hardware mutation. The presence check and purge are
 * atomic with respect to topology changes, so an identifier that reappears
 * after enumeration is not purged.
 *
 * # Safety
 *
 * `client` must be a valid client handle and `device_id` must point to a valid
 * NUL-terminated UTF-8 string.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_purge_withdrawn_device(struct LuminateClient *client,
                                                      const char *device_id);

/**
 * Asks the daemon to re-enumerate every plugin's hardware and reconcile
 * whatever changed, as it does on resume from suspend.
 *
 * Returns once the rescan is scheduled, not once it completes. Watch for
 * topology and state events to learn what actually changed.
 *
 * # Safety
 *
 * `client` must be a valid client handle.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_rescan(struct LuminateClient *client);

LUMINATE_API
LuminateOwnerKind luminate_owner_identity_kind(const struct LuminateOwnerIdentity *owner);

LUMINATE_API
bool luminate_owner_identity_uid(const struct LuminateOwnerIdentity *owner,
                                 uint32_t *out_uid);

LUMINATE_API
struct LuminateStringView luminate_owner_identity_sid(const struct LuminateOwnerIdentity *owner);

LUMINATE_API
struct LuminateStringView luminate_owner_identity_principal_authority(const struct LuminateOwnerIdentity *owner);

LUMINATE_API
struct LuminateStringView luminate_owner_identity_principal_subject(const struct LuminateOwnerIdentity *owner);

/**
 * Returns the typed C ABI version implemented by this build; bump when the
 * accessor surface changes incompatibly.
 *
 * The same constant supplies the ELF SONAME and generated C header. Packaging
 * metadata is checked against it by the ABI-version integration test.
 */
LUMINATE_API uint32_t luminate_c_abi_version(void);

/**
 * Releases a `LuminateEffect` created by one of the
 * `luminate_effect_create_*` constructors, or returned by
 * `luminate_facet_value_effect`. Null is a no-op.
 */
LUMINATE_API void luminate_effect_free(struct LuminateEffect *value);

/**
 * Fetches the full device topology from the daemon and writes an owned
 * `LuminateTopologySnapshot` to `out`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_list_devices(struct LuminateClient *client,
                                            struct LuminateTopologySnapshot **out);

/**
 * Fetches the identifiers of absent devices with retained state and writes an
 * owned `LuminateWithdrawnDeviceList` to `out`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_list_withdrawn_devices(struct LuminateClient *client,
                                                      struct LuminateWithdrawnDeviceList **out);

/**
 * Fetches a single device by id and writes an owned `LuminateDeviceSnapshot`
 * to `out`; returns not-found if no such device exists.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_get_device(struct LuminateClient *client,
                                          const char *id,
                                          struct LuminateDeviceSnapshot **out);

/**
 * Fetches a single device's state by id and writes an owned
 * `LuminateStateSnapshot` to `out`; returns not-found if no such device
 * exists.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_get_state(struct LuminateClient *client,
                                         const char *id,
                                         struct LuminateStateSnapshot **out);

/**
 * Fetches one collection's aggregate appearance state by id and writes an
 * owned `LuminateCollectionStateSnapshot` to `out`; returns not-found if no
 * such collection exists.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_get_collection_state(struct LuminateClient *client,
                                                    const char *id,
                                                    struct LuminateCollectionStateSnapshot **out);

/**
 * Blocks until the next event arrives on the subscription and writes an
 * owned `LuminateEvent` to `out`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_event_subscription_next(struct LuminateEventSubscription *subscription,
                                                struct LuminateEvent **out);

/**
 * Subscribes to events using the client's default event socket path and
 * writes the current topology as a baseline `LuminateTopologySnapshot` to
 * `out`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_subscribe_with_baseline(struct LuminateClient *client,
                                                       struct LuminateEventSubscription **sub,
                                                       struct LuminateTopologySnapshot **out);

/**
 * Subscribes to events using an explicit event socket `path` and writes the
 * current topology as a baseline `LuminateTopologySnapshot` to `out`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_subscribe_with_baseline_path(struct LuminateClient *client,
                                                            const char *path,
                                                            struct LuminateEventSubscription **sub,
                                                            struct LuminateTopologySnapshot **out);

/**
 * Number of devices in the topology snapshot.
 */
LUMINATE_API
uintptr_t luminate_topology_snapshot_device_count(const struct LuminateTopologySnapshot *v);

/**
 * Number of identifiers in a withdrawn device list.
 */
LUMINATE_API
uintptr_t luminate_withdrawn_device_list_count(const struct LuminateWithdrawnDeviceList *value);

/**
 * Borrowed identifier at `index`, or an empty view if out of range.
 */
LUMINATE_API
struct LuminateStringView luminate_withdrawn_device_list_at(const struct LuminateWithdrawnDeviceList *value,
                                                            uintptr_t index);

/**
 * Borrowed device at `index`, or null if out of range.
 */
LUMINATE_API
const struct LuminateDevice *luminate_topology_snapshot_device_at(const struct LuminateTopologySnapshot *v,
                                                                  uintptr_t i);

/**
 * The borrowed device held by this snapshot.
 */
LUMINATE_API
const struct LuminateDevice *luminate_device_snapshot_device(const struct LuminateDeviceSnapshot *v);

/**
 * The borrowed state held by this snapshot.
 */
LUMINATE_API
const struct LuminateState *luminate_state_snapshot_state(const struct LuminateStateSnapshot *v);

/**
 * The device's vendor name, or an absent view if not set.
 */
LUMINATE_API struct LuminateStringView luminate_device_vendor(const struct LuminateDevice *v);

/**
 * The device's model name, or an absent view if not set.
 */
LUMINATE_API struct LuminateStringView luminate_device_model(const struct LuminateDevice *v);

/**
 * The daemon-configured provider instance responsible for the device, or an
 * absent view when ownership is unknown.
 */
LUMINATE_API
struct LuminateStringView luminate_device_provider_instance(const struct LuminateDevice *v);

/**
 * The device's category, or an absent view if not set.
 */
LUMINATE_API struct LuminateStringView luminate_device_category(const struct LuminateDevice *v);

/**
 * Whether the device is physically attached to this host.
 */
LUMINATE_API bool luminate_device_host_attached(const struct LuminateDevice *v);

/**
 * Creates an identity-only actor-bound attestation.
 *
 * On success, ownership of `*out_attestation` transfers to the caller; free
 * it with `luminate_created_attestation_free`. The secret is displayed once.
 */
LUMINATE_API
LuminateStatus luminate_client_create_attestation(const struct LuminateClient *client,
                                                  const char *name,
                                                  const char *authority,
                                                  const char *subject,
                                                  bool has_expiry,
                                                  uint64_t expires_at_unix_ms,
                                                  struct LuminateCreatedAttestation **out_attestation);

/**
 * Creates a display-once attestation carrying verified principal groups.
 *
 * Every string is deep-copied. On success, ownership of `*out_attestation`
 * transfers to the caller; free it with
 * `luminate_created_attestation_free`.
 */
LUMINATE_API
LuminateStatus luminate_client_create_principal_attestation(const struct LuminateClient *client,
                                                            const char *name,
                                                            const char *authority,
                                                            const char *subject,
                                                            const char *const *verified_groups,
                                                            uintptr_t verified_group_count,
                                                            bool has_expiry,
                                                            uint64_t expires_at_unix_ms,
                                                            struct LuminateCreatedAttestation **out_attestation);

/**
 * Lists attestations bound to the client's kernel actor.
 *
 * On success, ownership of `*out_attestations` transfers to the caller; free
 * it with `luminate_attestation_list_free`.
 */
LUMINATE_API
LuminateStatus luminate_client_list_attestations(const struct LuminateClient *client,
                                                 struct LuminateAttestationList **out_attestations);

/**
 * Revokes an actor-bound attestation by name.
 */
LUMINATE_API
LuminateStatus luminate_client_revoke_attestation(const struct LuminateClient *client,
                                                  const char *name);

/**
 * Reads the active daemon access policy.
 *
 * On success, ownership of `*out_policy` transfers to the caller; free it
 * with `luminate_policy_document_free`.
 */
LUMINATE_API
LuminateStatus luminate_client_get_access_policy(const struct LuminateClient *client,
                                                 struct LuminatePolicyDocument **out_policy);

/**
 * Revision-replaces the daemon access policy.
 *
 * `replacement` remains caller-owned. On success, ownership of `*out_policy`
 * transfers to the caller; free it with `luminate_policy_document_free`.
 */
LUMINATE_API
LuminateStatus luminate_client_replace_access_policy(const struct LuminateClient *client,
                                                     uint64_t expected_revision,
                                                     const struct LuminatePolicyDocument *replacement,
                                                     struct LuminatePolicyDocument **out_policy);

/**
 * Creates a daemon bearer token and returns its display-once secret.
 *
 * On success, ownership of `*out_token` transfers to the caller; free it with
 * `luminate_created_token_free`.
 */
LUMINATE_API
LuminateStatus luminate_client_create_token(const struct LuminateClient *client,
                                            const char *id,
                                            const char *authority,
                                            const char *subject,
                                            bool has_expiry,
                                            uint64_t expires_at_unix_ms,
                                            struct LuminateCreatedToken **out_token);

/**
 * Lists sanitized daemon bearer-token metadata.
 *
 * On success, ownership of `*out_tokens` transfers to the caller; free it
 * with `luminate_token_list_free`.
 */
LUMINATE_API
LuminateStatus luminate_client_list_tokens(const struct LuminateClient *client,
                                           struct LuminateTokenList **out_tokens);

/**
 * Revokes a daemon bearer token by identifier.
 */
LUMINATE_API
LuminateStatus luminate_client_revoke_token(const struct LuminateClient *client,
                                            const char *id);

/**
 * Rotates a daemon bearer token and returns its replacement secret once.
 *
 * On success, ownership of `*out_token` transfers to the caller; free it with
 * `luminate_created_token_free`.
 */
LUMINATE_API
LuminateStatus luminate_client_rotate_token(const struct LuminateClient *client,
                                            const char *id,
                                            bool has_expiry,
                                            uint64_t expires_at_unix_ms,
                                            struct LuminateCreatedToken **out_token);

LUMINATE_API
uintptr_t luminate_created_token_secret(const struct LuminateCreatedToken *token,
                                        uint8_t *buffer,
                                        uintptr_t buffer_len);

LUMINATE_API
struct LuminateStringView luminate_created_token_id(const struct LuminateCreatedToken *token);

LUMINATE_API
struct LuminateStringView luminate_created_token_authority(const struct LuminateCreatedToken *token);

LUMINATE_API
struct LuminateStringView luminate_created_token_subject(const struct LuminateCreatedToken *token);

LUMINATE_API uintptr_t luminate_token_list_count(const struct LuminateTokenList *tokens);

LUMINATE_API
const struct LuminateToken *luminate_token_list_at(const struct LuminateTokenList *tokens,
                                                   uintptr_t index);

LUMINATE_API struct LuminateStringView luminate_token_id(const struct LuminateToken *token);

LUMINATE_API struct LuminateStringView luminate_token_authority(const struct LuminateToken *token);

LUMINATE_API struct LuminateStringView luminate_token_subject(const struct LuminateToken *token);

LUMINATE_API bool luminate_token_revoked(const struct LuminateToken *token);

LUMINATE_API
bool luminate_created_token_expires_at_unix_ms(const struct LuminateCreatedToken *token,
                                               uint64_t *out_expiry);

LUMINATE_API
bool luminate_token_expires_at_unix_ms(const struct LuminateToken *token,
                                       uint64_t *out_expiry);

LUMINATE_API
uintptr_t luminate_created_attestation_secret(const struct LuminateCreatedAttestation *attestation,
                                              uint8_t *buffer,
                                              uintptr_t buffer_len);

LUMINATE_API
struct LuminateStringView luminate_created_attestation_name(const struct LuminateCreatedAttestation *attestation);

LUMINATE_API
struct LuminateStringView luminate_created_attestation_authority(const struct LuminateCreatedAttestation *attestation);

LUMINATE_API
struct LuminateStringView luminate_created_attestation_subject(const struct LuminateCreatedAttestation *attestation);

LUMINATE_API
uintptr_t luminate_created_attestation_group_count(const struct LuminateCreatedAttestation *attestation);

LUMINATE_API
struct LuminateStringView luminate_created_attestation_group_at(const struct LuminateCreatedAttestation *attestation,
                                                                uintptr_t index);

LUMINATE_API
struct LuminateStringView luminate_created_attestation_credential_id(const struct LuminateCreatedAttestation *attestation);

LUMINATE_API
bool luminate_created_attestation_expires_at_unix_ms(const struct LuminateCreatedAttestation *attestation,
                                                     uint64_t *out_expiry);

LUMINATE_API
uintptr_t luminate_attestation_list_count(const struct LuminateAttestationList *attestations);

LUMINATE_API
const struct LuminateAttestation *luminate_attestation_list_at(const struct LuminateAttestationList *attestations,
                                                               uintptr_t index);

LUMINATE_API
struct LuminateStringView luminate_attestation_name(const struct LuminateAttestation *attestation);

LUMINATE_API
struct LuminateStringView luminate_attestation_authority(const struct LuminateAttestation *attestation);

LUMINATE_API
struct LuminateStringView luminate_attestation_subject(const struct LuminateAttestation *attestation);

LUMINATE_API
uintptr_t luminate_attestation_group_count(const struct LuminateAttestation *attestation);

LUMINATE_API
struct LuminateStringView luminate_attestation_group_at(const struct LuminateAttestation *attestation,
                                                        uintptr_t index);

LUMINATE_API
struct LuminateStringView luminate_attestation_credential_id(const struct LuminateAttestation *attestation);

LUMINATE_API
bool luminate_attestation_expires_at_unix_ms(const struct LuminateAttestation *attestation,
                                             uint64_t *out_expiry);

/**
 * Builds a wear-safe all-off plan for a borrowed device.
 *
 * # Safety
 *
 * `device` must be a valid borrowed device pointer and `out` a valid non-null
 * out-pointer.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_all_off_plan(const struct LuminateDevice *device,
                                     struct LuminateAllOffPlan **out);

/**
 * Number of wear-safe targets in the plan.
 */
LUMINATE_API uintptr_t luminate_all_off_plan_target_count(const struct LuminateAllOffPlan *plan);

/**
 * Borrowed wear-safe target at `index`, or null if out of range.
 */
LUMINATE_API
const struct LuminateTargetView *luminate_all_off_plan_target_at(const struct LuminateAllOffPlan *plan,
                                                                 uintptr_t index);

/**
 * Number of off-capable wear-limited targets skipped by the plan.
 */
LUMINATE_API
uintptr_t luminate_all_off_plan_skipped_persistent_count(const struct LuminateAllOffPlan *plan);

/**
 * Borrowed skipped wear-limited target at `index`, or null if out of range.
 */
LUMINATE_API
const struct LuminateTargetView *luminate_all_off_plan_skipped_persistent_at(const struct LuminateAllOffPlan *plan,
                                                                             uintptr_t index);

/**
 * Applies one logical appearance-slot mutation to a concrete surface.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_set_appearance_slots(struct LuminateClient *client,
                                                    const struct LuminateTarget *target,
                                                    const struct LuminateAppearanceSlotInput *values,
                                                    uintptr_t value_count);

/**
 * Borrowed appearance-slots capability, or null when absent.
 */
LUMINATE_API
const struct LuminateAppearanceSlotsCapability *luminate_capability_set_appearance_slots(const struct LuminateCapabilitySet *value);

/**
 * Completeness policy; one of `LUMINATE_APPEARANCE_SLOT_UPDATE_*`.
 */
LUMINATE_API
LuminateAppearanceSlotUpdatePolicy luminate_appearance_slots_update_policy(const struct LuminateAppearanceSlotsCapability *value);

/**
 * Appearance operations accepted by this slot.
 */
LUMINATE_API
const struct LuminateAppearanceCapability *luminate_appearance_slot_appearance(const struct LuminateAppearanceSlotDescriptor *value);

/**
 * Persistence kind for this slot; one of `LUMINATE_PERSISTENCE_*`.
 */
LUMINATE_API
uint32_t luminate_appearance_slot_persistence_kind(const struct LuminateAppearanceSlotDescriptor *value);

/**
 * Slot CCT emulation policy; one of `LUMINATE_CCT_EMULATION_*`.
 */
LUMINATE_API
uint32_t luminate_appearance_capability_cct_emulation(const struct LuminateAppearanceCapability *value);

/**
 * Borrowed hardware-effects capability accepted by the slot, or null.
 */
LUMINATE_API
const struct LuminateHardwareEffectsCapability *luminate_appearance_capability_hardware_effects(const struct LuminateAppearanceCapability *value);

/**
 * Slot identifier in one observed or scene value.
 */
LUMINATE_API
struct LuminateStringView luminate_appearance_slot_value_id(const struct LuminateAppearanceSlotValue *value);

/**
 * Borrowed view of a slot value's effect.
 */
LUMINATE_API
const struct LuminateEffectView *luminate_appearance_slot_value_effect(const struct LuminateAppearanceSlotValue *value);

/**
 * Whether the device/surface/element can be dark or emitting.
 */
LUMINATE_API bool luminate_capability_set_emission(const struct LuminateCapabilitySet *v);

/**
 * Whether turning the target off is safe even when ordinary appearance
 * updates are written through to non-volatile storage.
 */
LUMINATE_API bool luminate_capability_set_off_is_wear_safe(const struct LuminateCapabilitySet *v);

/**
 * How CCT requests are emulated when no native CCT channels are advertised;
 * one of the `LUMINATE_CCT_EMULATION_*` values.
 */
LUMINATE_API
LuminateCctEmulation luminate_capability_set_cct_emulation(const struct LuminateCapabilitySet *v);

/**
 * Whether brightness is independently controllable; one of the
 * `LUMINATE_CAPABILITY_*` values.
 */
LUMINATE_API
uint32_t luminate_capability_set_brightness_kind(const struct LuminateCapabilitySet *v);

/**
 * Writes the bit depth of independent brightness control.
 *
 * Returns false and leaves `out_bits` unchanged if brightness is not
 * independent or either pointer is null.
 */
LUMINATE_API
bool luminate_capability_set_brightness_bits(const struct LuminateCapabilitySet *v,
                                             uint8_t *out_bits);

/**
 * Writes the maximum independent brightness value.
 *
 * Returns false and leaves `out_maximum` unchanged if brightness is not
 * independent or either pointer is null.
 */
LUMINATE_API
bool luminate_capability_set_brightness_maximum(const struct LuminateCapabilitySet *v,
                                                uint32_t *out_maximum);

/**
 * Writes the scope at which independent brightness applies.
 *
 * Returns false and leaves `out_scope` unchanged if brightness is not
 * independent or either pointer is null.
 */
LUMINATE_API
bool luminate_capability_set_brightness_scope(const struct LuminateCapabilitySet *v,
                                              LuminateCapabilityScope *out_scope);

/**
 * Borrowed frame-upload capability, or null if frame upload is not
 * supported.
 */
LUMINATE_API
const struct LuminateFrameUploadCapability *luminate_capability_set_frame_upload(const struct LuminateCapabilitySet *v);

/**
 * Borrowed hardware-effects capability, or null if hardware effects are not
 * supported.
 */
LUMINATE_API
const struct LuminateHardwareEffectsCapability *luminate_capability_set_hardware_effects(const struct LuminateCapabilitySet *v);

/**
 * What the device retains across power cycles; one of the
 * `LUMINATE_PERSISTENCE_*` values.
 */
LUMINATE_API
LuminatePersistenceKind luminate_capability_set_persistence_kind(const struct LuminateCapabilitySet *v);

/**
 * Writes the persistence requirement.
 *
 * Returns false and leaves `out_requirement` unchanged if nothing is
 * persisted or either pointer is null.
 */
LUMINATE_API
bool luminate_capability_set_persistence_requirement(const struct LuminateCapabilitySet *v,
                                                     LuminatePersistenceRequirement *out_requirement);

/**
 * Writes the number of named profile slots.
 *
 * Returns false and leaves `out_slots` unchanged unless persistence kind is
 * profiles or either pointer is null.
 */
LUMINATE_API
bool luminate_capability_set_persistence_slots(const struct LuminateCapabilitySet *v,
                                               uint16_t *out_slots);

/**
 * Writes whether persistence requires an explicit commit call.
 *
 * Returns false and leaves `out_explicit_commit` unchanged if nothing is
 * persisted or either pointer is null.
 */
LUMINATE_API
bool luminate_capability_set_persistence_explicit_commit(const struct LuminateCapabilitySet *v,
                                                         bool *out_explicit_commit);

/**
 * Writes whether persisted state can be read back.
 *
 * Returns false and leaves `out_readback` unchanged if nothing is persisted
 * or either pointer is null.
 */
LUMINATE_API
bool luminate_capability_set_persistence_readback(const struct LuminateCapabilitySet *v,
                                                  bool *out_readback);

/**
 * Whether device state can be read back; one of the
 * `LUMINATE_STATE_READBACK_*` values.
 */
LUMINATE_API
LuminateStateReadbackKind luminate_capability_set_state_readback_kind(const struct LuminateCapabilitySet *v);

/**
 * Number of readable facets, or 0 if state is not readable.
 */
LUMINATE_API
uintptr_t luminate_capability_set_readable_facet_count(const struct LuminateCapabilitySet *v);

/**
 * Borrowed readable facet at `index`, or null if out of range or state is
 * not readable.
 */
LUMINATE_API
const struct LuminateReadableFacet *luminate_capability_set_readable_facet_at(const struct LuminateCapabilitySet *v,
                                                                              uintptr_t i);

/**
 * Whether reading state disturbs the device's visible output.
 */
LUMINATE_API
bool luminate_capability_set_read_disturbs_output(const struct LuminateCapabilitySet *v);

/**
 * Whether the device notifies of state changes made outside luminate.
 */
LUMINATE_API
bool luminate_capability_set_notifies_external_changes(const struct LuminateCapabilitySet *v);

/**
 * Borrowed physical-power capability, or null if not supported.
 */
LUMINATE_API
const struct LuminatePhysicalPowerCapability *luminate_capability_set_physical_power(const struct LuminateCapabilitySet *v);

/**
 * Borrowed power domain reference, or null if not set.
 */
LUMINATE_API
const struct LuminatePowerDomainRef *luminate_capability_set_power_domain(const struct LuminateCapabilitySet *v);

/**
 * How this colour capability's channels are interpreted; one of the
 * `LUMINATE_COLOUR_ENCODING_*` values.
 */
LUMINATE_API
LuminateColourEncoding luminate_colour_capability_encoding(const struct LuminateColourCapability *v);

LUMINATE_API
uintptr_t luminate_colour_capability_channel_count(const struct LuminateColourCapability *v);

LUMINATE_API
const struct LuminateColourChannelCapability *luminate_colour_capability_channel_at(const struct LuminateColourCapability *v,
                                                                                    uintptr_t index);

/**
 * Looks up a channel bit width by `LuminateColourChannel`.
 *
 * Returns false for a null capability, invalid or absent channel, or null
 * output.
 */
LUMINATE_API
bool luminate_colour_capability_bits(const struct LuminateColourCapability *v,
                                     LuminateColourChannel channel,
                                     uint8_t *out_bits);

/**
 * Which channel this capability describes; one of the
 * `LUMINATE_COLOUR_CHANNEL_*` values.
 */
LUMINATE_API
LuminateColourChannel luminate_colour_channel_capability_channel(const struct LuminateColourChannelCapability *v);

/**
 * Bit depth of this channel.
 */
LUMINATE_API
uint8_t luminate_colour_channel_capability_bits(const struct LuminateColourChannelCapability *v);

/**
 * Granularity at which frame upload applies; one of the `LUMINATE_SCOPE_*`
 * values.
 */
LUMINATE_API
LuminateCapabilityScope luminate_frame_upload_scope(const struct LuminateFrameUploadCapability *v);

/**
 * Which frame upload styles are accepted; one of the
 * `LUMINATE_FRAME_UPDATE_*` values.
 */
LUMINATE_API
LuminateFrameUpdateMode luminate_frame_upload_update_mode(const struct LuminateFrameUploadCapability *v);

/**
 * Whether a maximum upload rate is specified.
 */
LUMINATE_API
bool luminate_frame_upload_has_max_rate_hz(const struct LuminateFrameUploadCapability *v);

/**
 * Maximum upload rate in Hz, or 0 if unspecified.
 */
LUMINATE_API
uint16_t luminate_frame_upload_max_rate_hz(const struct LuminateFrameUploadCapability *v);

/**
 * Whether uploads are applied atomically.
 */
LUMINATE_API bool luminate_frame_upload_atomic(const struct LuminateFrameUploadCapability *v);

/**
 * How uploaded frames are committed to output; one of the
 * `LUMINATE_BUFFERING_*` values.
 */
LUMINATE_API
LuminateBufferingMode luminate_frame_upload_buffering(const struct LuminateFrameUploadCapability *v);

/**
 * Borrowed shared-memory fast-path capability, or null when only ordinary
 * request/response frame uploads are supported.
 */
LUMINATE_API
const struct LuminateShmFrameCapability *luminate_frame_upload_shm(const struct LuminateFrameUploadCapability *v);

/**
 * Number of packed pixel formats accepted by this shared-memory capability.
 */
LUMINATE_API
uintptr_t luminate_shm_frame_pixel_format_count(const struct LuminateShmFrameCapability *v);

/**
 * Packed pixel format at `index`, or `LUMINATE_DISCRIMINANT_INVALID` if out
 * of range.
 */
LUMINATE_API
LuminateShmPixelFormat luminate_shm_frame_pixel_format_at(const struct LuminateShmFrameCapability *v,
                                                          uintptr_t index);

/**
 * Number of bytes occupied by one pixel in `format`, or `0` for an unknown
 * discriminant.
 */
LUMINATE_API uintptr_t luminate_shm_pixel_format_bytes_per_pixel(LuminateShmPixelFormat format);

/**
 * The buffer layout; one of the `LUMINATE_SHM_FRAME_SHAPE_*` values.
 */
LUMINATE_API
LuminateShmFrameShapeKind luminate_shm_frame_shape_kind(const struct LuminateShmFrameCapability *v);

/**
 * Writes the total number of pixels in the shared-memory buffer.
 *
 * Returns false and leaves `out_pixel_count` unchanged if either pointer is
 * null.
 */
LUMINATE_API
bool luminate_shm_frame_pixel_count(const struct LuminateShmFrameCapability *v,
                                    uint32_t *out_pixel_count);

/**
 * Writes the matrix width.
 *
 * Returns false and leaves `out_width` unchanged unless the buffer shape is
 * matrix or either pointer is null.
 */
LUMINATE_API
bool luminate_shm_frame_matrix_width(const struct LuminateShmFrameCapability *v,
                                     uint32_t *out_width);

/**
 * Writes the matrix height.
 *
 * Returns false and leaves `out_height` unchanged unless the buffer shape is
 * matrix or either pointer is null.
 */
LUMINATE_API
bool luminate_shm_frame_matrix_height(const struct LuminateShmFrameCapability *v,
                                      uint32_t *out_height);

/**
 * Whether a shared-memory-specific maximum upload rate is specified.
 */
LUMINATE_API bool luminate_shm_frame_has_max_rate_hz(const struct LuminateShmFrameCapability *v);

/**
 * Shared-memory-specific maximum upload rate in Hz, or `0` when the ordinary
 * frame-upload limit applies.
 */
LUMINATE_API uint16_t luminate_shm_frame_max_rate_hz(const struct LuminateShmFrameCapability *v);

/**
 * Granularity at which hardware effects apply; one of the
 * `LUMINATE_SCOPE_*` values.
 */
LUMINATE_API
uint32_t luminate_hardware_effects_scope(const struct LuminateHardwareEffectsCapability *v);

/**
 * Whether hardware effects can run concurrently with frame streaming.
 */
LUMINATE_API
bool luminate_hardware_effects_concurrent_with_streaming(const struct LuminateHardwareEffectsCapability *v);

/**
 * Which variant of the effect parameter union is populated; one of the
 * `LUMINATE_EFFECT_PARAMETER_*` values.
 */
LUMINATE_API
LuminateEffectParameterKind luminate_effect_parameter_kind(const struct LuminateEffectParameter *v);

/**
 * Writes the inclusive colour-count range for a colour parameter.
 */
LUMINATE_API
bool luminate_effect_parameter_colour_count_range(const struct LuminateEffectParameter *v,
                                                  uint8_t *out_minimum,
                                                  uint8_t *out_maximum);

/**
 * The allowed speed range, or a zeroed range unless this is a speed
 * parameter.
 */
LUMINATE_API
bool luminate_effect_parameter_speed_range(const struct LuminateEffectParameter *v,
                                           struct LuminateU16Range *out_range);

/**
 * Number of supported directions, or 0 unless this is a direction
 * parameter.
 */
LUMINATE_API
uintptr_t luminate_effect_parameter_direction_count(const struct LuminateEffectParameter *v);

/**
 * Direction at `index`; one of the `LUMINATE_DIRECTION_*` values, or
 * `LUMINATE_DISCRIMINANT_INVALID` if out of range.
 */
LUMINATE_API
LuminateEffectDirection luminate_effect_parameter_direction_at(const struct LuminateEffectParameter *v,
                                                               uintptr_t i);

/**
 * The allowed duration-in-milliseconds range, or a zeroed range unless this
 * is a duration parameter.
 */
LUMINATE_API
bool luminate_effect_parameter_duration_range(const struct LuminateEffectParameter *v,
                                              struct LuminateU32Range *out_range);

/**
 * Bit depth of the brightness parameter, or 0 unless this is a brightness
 * parameter.
 */
LUMINATE_API
bool luminate_effect_parameter_brightness_bits(const struct LuminateEffectParameter *v,
                                               uint8_t *out_bits);

/**
 * Number of choice options, or 0 unless this is a choice parameter.
 */
LUMINATE_API
uintptr_t luminate_effect_parameter_choice_count(const struct LuminateEffectParameter *v);

/**
 * Borrowed choice option at `index`, or null if out of range or not a
 * choice parameter.
 */
LUMINATE_API
const struct LuminateEffectChoice *luminate_effect_parameter_choice_at(const struct LuminateEffectParameter *v,
                                                                       uintptr_t i);

/**
 * Which facet is readable; one of the `LUMINATE_FACET_*` values.
 */
LUMINATE_API
LuminateStateFacetKind luminate_readable_facet_kind(const struct LuminateReadableFacet *v);

/**
 * How trustworthy readback of this facet is; one of the
 * `LUMINATE_READBACK_*` values.
 */
LUMINATE_API
LuminateReadbackFidelity luminate_readable_facet_fidelity(const struct LuminateReadableFacet *v);

/**
 * Granularity at which physical power control applies; one of the
 * `LUMINATE_SCOPE_*` values.
 */
LUMINATE_API
LuminateCapabilityScope luminate_physical_power_scope(const struct LuminatePhysicalPowerCapability *v);

/**
 * What this power domain reference addresses; one of the
 * `LUMINATE_POWER_DOMAIN_*` values.
 */
LUMINATE_API
LuminatePowerDomainKind luminate_power_domain_kind(const struct LuminatePowerDomainRef *v);

/**
 * The referenced surface's id, or an absent view unless this domain refers
 * to a surface.
 */
LUMINATE_API
struct LuminateStringView luminate_power_domain_surface_id(const struct LuminatePowerDomainRef *v);

/**
 * Creates a collection owned by the calling principal and writes its
 * server-generated id, as a newly allocated C string, to `out_id`.
 * `description` and `kind` may be null. `members` may be null only if
 * `member_count` is 0.
 *
 * # Safety
 *
 * `client` must be a valid client handle, `name` must point to a valid
 * NUL-terminated UTF-8 string, `description`/`kind` must each be either null
 * or a valid NUL-terminated UTF-8 string, `members` must point to
 * `member_count` valid `LuminateCollectionMemberInput`s (each with a valid
 * `target` or `collection_id` per its `is_collection` flag), and `out_id`
 * must be a valid non-null out-pointer. The returned string must be released
 * with `luminate_string_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_create_collection(struct LuminateClient *client,
                                                 const char *name,
                                                 const char *description,
                                                 const char *kind,
                                                 const struct LuminateCollectionMemberInput *members,
                                                 uintptr_t member_count,
                                                 char **out_id);

/**
 * Destroys a collection. Refused if another collection still references it,
 * or if the caller doesn't own it.
 *
 * # Safety
 *
 * `client` must be a valid client handle and `id` must point to a valid
 * NUL-terminated UTF-8 string.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_destroy_collection(struct LuminateClient *client,
                                                  const char *id);

/**
 * Fetches every registered collection and writes an owned
 * `LuminateCollectionList` to `out`.
 *
 * # Safety
 *
 * `client` must be a valid client handle and `out` must be a valid non-null
 * out-pointer. The returned list must be released with
 * `luminate_collection_list_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_list_collections(struct LuminateClient *client,
                                                struct LuminateCollectionList **out);

/**
 * Fetches a single collection by id and writes an owned
 * `LuminateCollectionSnapshot` to `out`; returns not-found if no such
 * collection exists.
 *
 * # Safety
 *
 * `client` must be a valid client handle, `id` must point to a valid
 * NUL-terminated UTF-8 string, and `out` must be a valid non-null
 * out-pointer. The returned snapshot must be released with
 * `luminate_collection_snapshot_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_get_collection(struct LuminateClient *client,
                                              const char *id,
                                              struct LuminateCollectionSnapshot **out);

/**
 * Number of collections in the list.
 */
LUMINATE_API uintptr_t luminate_collection_list_count(const struct LuminateCollectionList *v);

/**
 * Borrowed collection at `index`, or null if out of range.
 */
LUMINATE_API
const struct LuminateCollection *luminate_collection_list_at(const struct LuminateCollectionList *v,
                                                             uintptr_t i);

/**
 * The borrowed collection held by this snapshot.
 */
LUMINATE_API
const struct LuminateCollection *luminate_collection_snapshot_collection(const struct LuminateCollectionSnapshot *v);

/**
 * The collection's description, or an absent view if not set.
 */
LUMINATE_API
struct LuminateStringView luminate_collection_description(const struct LuminateCollection *v);

/**
 * Borrowed identity which owns this collection.
 */
LUMINATE_API
const struct LuminateOwnerIdentity *luminate_collection_owner(const struct LuminateCollection *v);

/**
 * The collection's presentation-hint category, or an absent view if not
 * set.
 */
LUMINATE_API struct LuminateStringView luminate_collection_kind(const struct LuminateCollection *v);

/**
 * What this member refers to; one of the `LUMINATE_COLLECTION_MEMBER_*`
 * values.
 */
LUMINATE_API
LuminateCollectionMemberKind luminate_collection_member_kind(const struct LuminateCollectionMember *v);

/**
 * The referenced target, or null unless this member is a concrete target.
 */
LUMINATE_API
const struct LuminateTargetView *luminate_collection_member_target(const struct LuminateCollectionMember *v);

/**
 * The referenced collection's id, or an absent view unless this member is a
 * nested collection.
 */
LUMINATE_API
struct LuminateStringView luminate_collection_member_collection_id(const struct LuminateCollectionMember *v);

/**
 * Deep-copies a borrowed effect view into an owned effect. Release the result
 * with `luminate_effect_free`.
 *
 * Returns `LUMINATE_STATUS_NULL_POINTER` when either pointer is null.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_view_clone(const struct LuminateEffectView *view,
                                          struct LuminateEffect **out_effect);

/**
 * Deep-copies a borrowed generic colour into an owned static effect. Release
 * the result with `luminate_effect_free`.
 *
 * Returns `LUMINATE_STATUS_NULL_POINTER` when either pointer is null.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_create_static_from_colour(const struct LuminateColour *colour,
                                                         struct LuminateEffect **out_effect);

/**
 * Creates an owned off effect. Release with `luminate_effect_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_create_off(struct LuminateEffect **out);

/**
 * Creates an owned static-colour effect. Release with `luminate_effect_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_create_static(const struct LuminateColourInput *colour,
                                             struct LuminateEffect **out);

/**
 * Creates an owned spectrum effect. Release with `luminate_effect_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_create_spectrum(uint32_t period,
                                               struct LuminateEffect **out);

/**
 * Creates an owned rainbow effect. Release with `luminate_effect_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_create_rainbow(uint32_t period,
                                              struct LuminateEffect **out);

/**
 * Creates an owned morph effect over the given colours. Release with
 * `luminate_effect_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_create_morph(const struct LuminateRgb *colours,
                                            uintptr_t count,
                                            uint32_t period,
                                            struct LuminateEffect **out);

/**
 * Creates an owned hardware effect selected by `id`, with no arguments set.
 * Release with `luminate_effect_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_create_hardware(const char *id,
                                               struct LuminateEffect **out);

/**
 * Appends a colour to a hardware effect's colour list.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_hardware_add_colour(struct LuminateEffect *v,
                                                   struct LuminateRgb c);

/**
 * Sets the direction parameter on a hardware effect created by
 * `luminate_effect_create_hardware`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_hardware_set_direction(struct LuminateEffect *v,
                                                      uint32_t x);

/**
 * Sets the choice parameter on a hardware effect created by
 * `luminate_effect_create_hardware`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_hardware_set_choice(struct LuminateEffect *v,
                                                   const char *x);

/**
 * Applies the given effect to the target.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_set_effect(struct LuminateClient *client,
                                          const struct LuminateTarget *target,
                                          const struct LuminateEffect *effect);

/**
 * Which payload this event carries; one of the `LUMINATE_EVENT_*` values.
 */
LUMINATE_API LuminateEventKind luminate_event_kind(const struct LuminateEvent *v);

/**
 * Number of devices in a topology-changed event, or 0 unless this is a
 * topology event.
 */
LUMINATE_API uintptr_t luminate_event_topology_device_count(const struct LuminateEvent *v);

/**
 * Device id at `index` in a topology-changed event, or an absent view if
 * out of range or not a topology event.
 */
LUMINATE_API
struct LuminateStringView luminate_event_topology_device_at(const struct LuminateEvent *v,
                                                            uintptr_t i);

/**
 * Number of devices in a state-changed event, or 0 unless this is a state
 * event.
 */
LUMINATE_API uintptr_t luminate_event_state_device_count(const struct LuminateEvent *v);

/**
 * Device id at `index` in a state-changed event, or an absent view if out
 * of range or not a state event.
 */
LUMINATE_API
struct LuminateStringView luminate_event_state_device_at(const struct LuminateEvent *v,
                                                         uintptr_t i);

/**
 * Borrowed target of a shared-memory-stream-ended event, or null unless
 * this is one.
 */
LUMINATE_API
const struct LuminateTargetView *luminate_event_shm_stream_target(const struct LuminateEvent *v);

/**
 * Writes the generation the ended stream was negotiated under.
 *
 * Returns false and leaves `out_generation` unchanged unless this is a
 * shared-memory-stream-ended event or either pointer is null.
 */
LUMINATE_API
bool luminate_event_shm_stream_generation(const struct LuminateEvent *v,
                                          uint32_t *out_generation);

/**
 * Number of dirty transition identifiers, or 0 unless this is a transition
 * event. Zero in a transition event requests a full refresh.
 */
LUMINATE_API uintptr_t luminate_event_transition_count(const struct LuminateEvent *v);

/**
 * Borrowed dirty transition identifier at `index`, or an absent view.
 */
LUMINATE_API
struct LuminateStringView luminate_event_transition_id_at(const struct LuminateEvent *v,
                                                          uintptr_t index);

/**
 * Borrowed redacted change set carried by a configuration-changed event, or
 * null unless this is one.
 */
LUMINATE_API
const struct LuminateManagementChangeSetView *luminate_event_configuration_changes(const struct LuminateEvent *v);

/**
 * Begins a frame streaming session on the given target, writing the
 * session generation to `out_generation`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_begin_frame_stream(struct LuminateClient *client,
                                                  const struct LuminateTarget *target,
                                                  uint32_t *out_generation);

/**
 * Uploads a full frame of colours to the given target's active streaming
 * session.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_upload_frame_full(struct LuminateClient *client,
                                                 const struct LuminateTarget *target,
                                                 uint32_t generation,
                                                 uint64_t sequence,
                                                 const struct LuminateRgb *colours,
                                                 uintptr_t count,
                                                 uint8_t commit,
                                                 struct LuminateFrameAck *out_ack);

/**
 * Uploads a partial frame (sparse indices and colours) to the given
 * target's active streaming session.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_upload_frame_partial(struct LuminateClient *client,
                                                    const struct LuminateTarget *target,
                                                    uint32_t generation,
                                                    uint64_t sequence,
                                                    const uint32_t *indices,
                                                    const struct LuminateRgb *colours,
                                                    uintptr_t count,
                                                    uint8_t commit,
                                                    struct LuminateFrameAck *out_ack);

/**
 * Ends the frame streaming session with the given generation on the
 * target.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_end_frame_stream(struct LuminateClient *client,
                                                const struct LuminateTarget *target,
                                                uint32_t generation);

LUMINATE_API
LuminateStatus luminate_management_patch_builder_new(uint64_t expected_revision,
                                                     struct LuminateManagementPatchBuilder **out_builder);

LUMINATE_API
LuminateStatus luminate_management_patch_builder_set_expected_revision(struct LuminateManagementPatchBuilder *builder,
                                                                       uint64_t expected_revision);

LUMINATE_API
LuminateStatus luminate_management_patch_builder_set_daemon_preferences(struct LuminateManagementPatchBuilder *builder,
                                                                        const struct LuminateDaemonPreferencesInput *preferences);

LUMINATE_API
LuminateStatus luminate_management_patch_builder_set_plugin_enabled(struct LuminateManagementPatchBuilder *builder,
                                                                    const char *plugin,
                                                                    bool has_enabled,
                                                                    bool enabled);

LUMINATE_API
LuminateStatus luminate_management_patch_builder_set_plugin_reconciliation(struct LuminateManagementPatchBuilder *builder,
                                                                           const char *plugin,
                                                                           bool has_reconciliation,
                                                                           LuminateReconciliationPolicy reconciliation_value);

LUMINATE_API
LuminateStatus luminate_management_patch_builder_set_plugin_setting(struct LuminateManagementPatchBuilder *builder,
                                                                    const char *plugin,
                                                                    const char *key,
                                                                    const struct LuminateSettingValue *value);

LUMINATE_API
LuminateStatus luminate_management_patch_builder_clear_plugin_setting(struct LuminateManagementPatchBuilder *builder,
                                                                      const char *plugin,
                                                                      const char *key);

LUMINATE_API
LuminateStatus luminate_setting_value_new_number(double value,
                                                 struct LuminateSettingValue **out_value);

LUMINATE_API
LuminateStatus luminate_setting_value_new_string(const char *value,
                                                 struct LuminateSettingValue **out_value);

LUMINATE_API
LuminateStatus luminate_setting_value_new_array(struct LuminateSettingValue **out_value);

LUMINATE_API
LuminateStatus luminate_setting_value_new_table(struct LuminateSettingValue **out_value);

LUMINATE_API
LuminateStatus luminate_setting_value_array_push(struct LuminateSettingValue *array,
                                                 const struct LuminateSettingValue *value);

LUMINATE_API
LuminateStatus luminate_setting_value_table_insert(struct LuminateSettingValue *table,
                                                   const char *key,
                                                   const struct LuminateSettingValue *value);

LUMINATE_API
uint64_t luminate_management_snapshot_revision(const struct LuminateManagementSnapshot *snapshot);

LUMINATE_API
const struct LuminateDaemonPreferences *luminate_management_snapshot_desired_daemon(const struct LuminateManagementSnapshot *snapshot);

LUMINATE_API
const struct LuminateDaemonPreferences *luminate_management_snapshot_effective_daemon(const struct LuminateManagementSnapshot *snapshot);

LUMINATE_API
uintptr_t luminate_management_snapshot_locked_daemon_setting_count(const struct LuminateManagementSnapshot *snapshot);

LUMINATE_API
struct LuminateStringView luminate_management_snapshot_locked_daemon_setting_at(const struct LuminateManagementSnapshot *snapshot,
                                                                                uintptr_t index);

LUMINATE_API
uintptr_t luminate_management_snapshot_plugin_count(const struct LuminateManagementSnapshot *snapshot);

LUMINATE_API
const struct LuminateManagedPlugin *luminate_management_snapshot_plugin_at(const struct LuminateManagementSnapshot *snapshot,
                                                                           uintptr_t index);

LUMINATE_API
uintptr_t luminate_daemon_preferences_device_reconciliation_count(const struct LuminateDaemonPreferences *value);

LUMINATE_API
const struct LuminateDeviceReconciliationPreference *luminate_daemon_preferences_device_reconciliation_at(const struct LuminateDaemonPreferences *value,
                                                                                                          uintptr_t index);

LUMINATE_API
struct LuminateStringView luminate_device_reconciliation_preference_device_id(const struct LuminateDeviceReconciliationPreference *value);

LUMINATE_API
LuminateReconciliationPolicy luminate_device_reconciliation_preference_policy(const struct LuminateDeviceReconciliationPreference *value);

LUMINATE_API
bool luminate_managed_plugin_has_desired_enabled(const struct LuminateManagedPlugin *value);

LUMINATE_API
bool luminate_managed_plugin_desired_enabled(const struct LuminateManagedPlugin *value);

LUMINATE_API
bool luminate_managed_plugin_has_desired_reconciliation(const struct LuminateManagedPlugin *value);

LUMINATE_API
LuminateReconciliationPolicy luminate_managed_plugin_desired_reconciliation(const struct LuminateManagedPlugin *value);

LUMINATE_API
bool luminate_managed_plugin_has_effective_reconciliation(const struct LuminateManagedPlugin *value);

LUMINATE_API
LuminateReconciliationPolicy luminate_managed_plugin_effective_reconciliation(const struct LuminateManagedPlugin *value);

LUMINATE_API
LuminatePluginRuntimeStateKind luminate_managed_plugin_runtime_kind(const struct LuminateManagedPlugin *value);

LUMINATE_API
struct LuminateStringView luminate_managed_plugin_runtime_diagnostic(const struct LuminateManagedPlugin *value);

LUMINATE_API
uintptr_t luminate_managed_plugin_schema_count(const struct LuminateManagedPlugin *value);

LUMINATE_API
const struct LuminatePluginSettingSchema *luminate_managed_plugin_schema_at(const struct LuminateManagedPlugin *value,
                                                                            uintptr_t index);

LUMINATE_API
uintptr_t luminate_managed_plugin_locked_setting_count(const struct LuminateManagedPlugin *plugin);

LUMINATE_API
struct LuminateStringView luminate_managed_plugin_locked_setting_at(const struct LuminateManagedPlugin *plugin,
                                                                    uintptr_t index);

LUMINATE_API
LuminatePluginSettingKind luminate_plugin_setting_schema_kind(const struct LuminatePluginSettingSchema *value);

LUMINATE_API
const struct LuminateReportedSettingValue *luminate_plugin_setting_schema_default(const struct LuminatePluginSettingSchema *value);

LUMINATE_API
bool luminate_plugin_setting_schema_required(const struct LuminatePluginSettingSchema *value);

LUMINATE_API
bool luminate_plugin_setting_schema_sensitive(const struct LuminatePluginSettingSchema *value);

LUMINATE_API
bool luminate_plugin_setting_schema_restart_required(const struct LuminatePluginSettingSchema *value);

LUMINATE_API
bool luminate_plugin_setting_schema_has_minimum(const struct LuminatePluginSettingSchema *value);

LUMINATE_API
double luminate_plugin_setting_schema_minimum(const struct LuminatePluginSettingSchema *value);

LUMINATE_API
bool luminate_plugin_setting_schema_has_maximum(const struct LuminatePluginSettingSchema *value);

LUMINATE_API
double luminate_plugin_setting_schema_maximum(const struct LuminatePluginSettingSchema *value);

LUMINATE_API
const struct LuminateReportedSettingValue *luminate_plugin_setting_schema_constraints(const struct LuminatePluginSettingSchema *value);

LUMINATE_API
LuminateReportedSettingKind luminate_reported_setting_value_kind(const struct LuminateReportedSettingValue *value);

LUMINATE_API
const struct LuminateSettingValueView *luminate_reported_setting_value_visible(const struct LuminateReportedSettingValue *value);

/**
 * Deep-copies a borrowed visible setting value into an owned value. Nested
 * arrays, tables, strings, and keys are copied recursively. Release the
 * result with `luminate_setting_value_free`.
 *
 * Only visible reported values yield a view that can be passed here; callers
 * must reject unset and redacted values using
 * `luminate_reported_setting_value_kind` first.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_setting_value_view_clone(const struct LuminateSettingValueView *view,
                                                 struct LuminateSettingValue **out_value);

LUMINATE_API
LuminateManagementSettingKind luminate_setting_value_view_kind(const struct LuminateSettingValueView *value);

LUMINATE_API bool luminate_setting_value_view_boolean(const struct LuminateSettingValueView *value);

LUMINATE_API
int64_t luminate_setting_value_view_integer(const struct LuminateSettingValueView *value);

LUMINATE_API
double luminate_setting_value_view_number(const struct LuminateSettingValueView *value);

LUMINATE_API
struct LuminateStringView luminate_setting_value_view_string(const struct LuminateSettingValueView *value);

LUMINATE_API
uintptr_t luminate_setting_value_view_count(const struct LuminateSettingValueView *value);

LUMINATE_API
const struct LuminateSettingValueView *luminate_setting_value_view_array_at(const struct LuminateSettingValueView *value,
                                                                            uintptr_t index);

LUMINATE_API
struct LuminateStringView luminate_setting_value_view_table_key_at(const struct LuminateSettingValueView *value,
                                                                   uintptr_t index);

LUMINATE_API
const struct LuminateSettingValueView *luminate_setting_value_view_table_value_at(const struct LuminateSettingValueView *value,
                                                                                  uintptr_t index);

LUMINATE_API
uint64_t luminate_management_change_set_revision(const struct LuminateManagementChangeSet *value);

LUMINATE_API
uintptr_t luminate_management_change_set_count(const struct LuminateManagementChangeSet *value);

LUMINATE_API
const struct LuminateManagementChange *luminate_management_change_set_at(const struct LuminateManagementChangeSet *value,
                                                                         uintptr_t index);

/**
 * Revision committed by this borrowed event change set.
 */
LUMINATE_API
uint64_t luminate_management_change_set_view_revision(const struct LuminateManagementChangeSetView *value);

/**
 * Number of redacted changes in this borrowed event change set.
 */
LUMINATE_API
uintptr_t luminate_management_change_set_view_count(const struct LuminateManagementChangeSetView *value);

/**
 * Borrowed change at `index`, or null if out of range.
 */
LUMINATE_API
const struct LuminateManagementChange *luminate_management_change_set_view_at(const struct LuminateManagementChangeSetView *value,
                                                                              uintptr_t index);

LUMINATE_API
LuminateManagementChangeKind luminate_management_change_kind(const struct LuminateManagementChange *value);

LUMINATE_API
uintptr_t luminate_management_change_key_count(const struct LuminateManagementChange *value);

LUMINATE_API
struct LuminateStringView luminate_management_change_key_at(const struct LuminateManagementChange *value,
                                                            uintptr_t index);

LUMINATE_API
struct LuminateStringView luminate_management_change_plugin(const struct LuminateManagementChange *value);

LUMINATE_API
bool luminate_management_change_setting_sensitive(const struct LuminateManagementChange *value);

LUMINATE_API
LuminateStatus luminate_client_get_management(struct LuminateClient *client,
                                              struct LuminateManagementSnapshot **out_snapshot);

LUMINATE_API
LuminateStatus luminate_client_patch_management(struct LuminateClient *client,
                                                const struct LuminateManagementPatchBuilder *patch,
                                                struct LuminateManagementChangeSet **out_changes);

/**
 * Borrowed capability set for the device.
 */
LUMINATE_API
const struct LuminateCapabilitySet *luminate_device_capabilities(const struct LuminateDevice *v);

/**
 * Borrowed capability set for the surface.
 */
LUMINATE_API
const struct LuminateCapabilitySet *luminate_surface_capabilities(const struct LuminateSurface *v);

/**
 * The surface's shape; one of the `LUMINATE_SURFACE_*` values.
 */
LUMINATE_API LuminateSurfaceKind luminate_surface_kind(const struct LuminateSurface *v);

/**
 * Length of a linear surface, or 0 unless the surface kind is linear.
 */
LUMINATE_API
bool luminate_surface_linear_length(const struct LuminateSurface *v,
                                    float *out_length);

/**
 * Width/height of a sparse-2D surface, or zeroed unless the surface kind is
 * sparse-2d.
 */
LUMINATE_API
bool luminate_surface_sparse_size(const struct LuminateSurface *v,
                                  struct LuminatePoint *out_size);

/**
 * Row/column dimensions of a matrix surface, or zeroed unless the surface
 * kind is matrix.
 */
LUMINATE_API
bool luminate_surface_matrix_size(const struct LuminateSurface *v,
                                  struct LuminateMatrixCell *out_size);

/**
 * The element's human-readable display name, or an absent view if not set.
 */
LUMINATE_API struct LuminateStringView luminate_element_name(const struct LuminateElement *v);

/**
 * The element's role within its surface; one of the `LUMINATE_ELEMENT_*`
 * values.
 */
LUMINATE_API LuminateElementKind luminate_element_kind(const struct LuminateElement *v);

/**
 * Which geometry field is populated; one of the `LUMINATE_GEOMETRY_*`
 * values.
 */
LUMINATE_API LuminateGeometryKind luminate_element_geometry_kind(const struct LuminateElement *v);

/**
 * The element's rectangle geometry, or zeroed unless geometry kind is rect.
 */
LUMINATE_API
bool luminate_element_geometry_rect(const struct LuminateElement *v,
                                    struct LuminateRect *out_rect);

/**
 * The element's point geometry, or zeroed unless geometry kind is point.
 */
LUMINATE_API
bool luminate_element_geometry_point(const struct LuminateElement *v,
                                     struct LuminatePoint *out_point);

/**
 * The element's linear position, or 0 unless geometry kind is linear.
 */
LUMINATE_API
bool luminate_element_geometry_linear(const struct LuminateElement *v,
                                      float *out_position);

/**
 * The element's matrix cell, or zeroed unless geometry kind is matrix cell.
 */
LUMINATE_API
bool luminate_element_geometry_matrix_cell(const struct LuminateElement *v,
                                           struct LuminateMatrixCell *out_cell);

/**
 * Borrowed capability set for the element.
 */
LUMINATE_API
const struct LuminateCapabilitySet *luminate_element_capabilities(const struct LuminateElement *v);

/**
 * The group's description, or an absent view if not set.
 */
LUMINATE_API struct LuminateStringView luminate_group_description(const struct LuminateGroup *v);

/**
 * How the group was created and is managed; one of the `LUMINATE_GROUP_*`
 * values.
 */
LUMINATE_API LuminateGroupKind luminate_group_kind(const struct LuminateGroup *v);

/**
 * Borrowed capability set for the group.
 */
LUMINATE_API
const struct LuminateCapabilitySet *luminate_group_capabilities(const struct LuminateGroup *v);

/**
 * What this member refers to; one of the `LUMINATE_GROUP_MEMBER_*` values.
 */
LUMINATE_API
LuminateGroupMemberKind luminate_group_member_kind(const struct LuminateGroupMember *v);

/**
 * The referenced surface's id, or an absent view unless this member is a
 * surface or element.
 */
LUMINATE_API
struct LuminateStringView luminate_group_member_surface_id(const struct LuminateGroupMember *v);

/**
 * The referenced element's id, or an absent view unless this member is an
 * element.
 */
LUMINATE_API
struct LuminateStringView luminate_group_member_element_id(const struct LuminateGroupMember *v);

/**
 * The referenced group's id, or an absent view unless this member is a
 * group.
 */
LUMINATE_API
struct LuminateStringView luminate_group_member_group_id(const struct LuminateGroupMember *v);

/**
 * Creates an empty policy document builder at the given revision. Release
 * with `luminate_policy_document_builder_free`.
 *
 * # Safety
 *
 * `out_builder` must be a valid non-null out-pointer.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_policy_document_builder_new(uint64_t revision,
                                                    struct LuminatePolicyDocumentBuilder **out_builder);

/**
 * Seeds a mutable builder with a complete, independent copy of a validated
 * policy document. Building the result without mutation preserves the
 * document's observable contents and authorization behaviour. Release the
 * builder with `luminate_policy_document_builder_free`.
 *
 * # Safety
 *
 * `document` must be a valid non-null document pointer. `out_builder` must be
 * a valid non-null out-pointer.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_policy_document_builder_from_document(const struct LuminatePolicyDocument *document,
                                                              struct LuminatePolicyDocumentBuilder **out_builder);

/**
 * Sets the builder's revision, overwriting any previous value.
 *
 * # Safety
 *
 * `builder` must be a valid, exclusively used builder pointer.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_policy_document_builder_set_revision(struct LuminatePolicyDocumentBuilder *builder,
                                                             uint64_t revision);

/**
 * Adds one parent role to a role, creating both the role and its parent
 * entry if either is not yet present. A missing referenced parent is a
 * build-time validation error, not a builder-time one.
 *
 * # Safety
 *
 * `builder` must be a valid, exclusively used builder pointer. `role` and
 * `parent` must point to valid NUL-terminated UTF-8 strings.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_policy_document_builder_role_add_parent(struct LuminatePolicyDocumentBuilder *builder,
                                                                const char *role,
                                                                const char *parent);

/**
 * Removes a role and its contents by name. References to that role in parent
 * sets and bindings are not rewritten; the caller must update them before
 * building the document.
 *
 * # Safety
 *
 * `builder` must be a valid, exclusively used builder pointer. `role` must
 * point to a valid NUL-terminated UTF-8 string.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_policy_document_builder_remove_role(struct LuminatePolicyDocumentBuilder *builder,
                                                            const char *role);

/**
 * Removes one parent from a role. Both the role and parent relationship must
 * already exist.
 *
 * # Safety
 *
 * All pointers must be valid and non-null. `role` and `parent` must point to
 * NUL-terminated UTF-8 strings.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_policy_document_builder_role_remove_parent(struct LuminatePolicyDocumentBuilder *builder,
                                                                   const char *role,
                                                                   const char *parent);

/**
 * Appends one rule to a role, creating the role if not yet present.
 *
 * # Safety
 *
 * `builder` must be a valid, exclusively used builder pointer. `role` must
 * point to a valid NUL-terminated UTF-8 string. `rule` must be a valid,
 * non-null pointer to a fully populated `LuminateRuleInput`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_policy_document_builder_role_add_rule(struct LuminatePolicyDocumentBuilder *builder,
                                                              const char *role_name,
                                                              const struct LuminateRuleInput *rule);

/**
 * Replaces one rule at `index`, preserving its position.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_policy_document_builder_role_replace_rule(struct LuminatePolicyDocumentBuilder *builder,
                                                                  const char *role_name,
                                                                  uintptr_t index,
                                                                  const struct LuminateRuleInput *rule);

/**
 * Removes one rule at `index` from a role.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_policy_document_builder_role_remove_rule(struct LuminatePolicyDocumentBuilder *builder,
                                                                 const char *role_name,
                                                                 uintptr_t index);

/**
 * Appends one binding.
 *
 * # Safety
 *
 * `builder` must be a valid, exclusively used builder pointer. `authority`
 * must point to a valid NUL-terminated UTF-8 string. `subjects`, `groups`,
 * and `roles` must each point to their respective `*_count` valid string
 * pointers, or may be null when their count is `0`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_policy_document_builder_add_binding(struct LuminatePolicyDocumentBuilder *builder,
                                                            const char *authority,
                                                            const char *const *subjects,
                                                            uintptr_t subject_count,
                                                            const char *const *groups,
                                                            uintptr_t group_count,
                                                            const char *const *roles,
                                                            uintptr_t role_count);

/**
 * Replaces one binding at `index`, preserving its position.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_policy_document_builder_replace_binding(struct LuminatePolicyDocumentBuilder *builder,
                                                                uintptr_t index,
                                                                const char *authority,
                                                                const char *const *subjects,
                                                                uintptr_t subject_count,
                                                                const char *const *groups,
                                                                uintptr_t group_count,
                                                                const char *const *roles,
                                                                uintptr_t role_count);

/**
 * Removes one binding at `index`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_policy_document_builder_remove_binding(struct LuminatePolicyDocumentBuilder *builder,
                                                               uintptr_t index);

/**
 * Validates the builder's current contents and writes a new, immutable
 * `LuminatePolicyDocument`. Does not consume or clear the builder, which may
 * keep being edited and rebuilt afterwards. Release the returned document
 * with `luminate_policy_document_free`.
 *
 * # Safety
 *
 * `builder` must be a valid builder pointer. `out_document` must be a valid
 * non-null out-pointer.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_policy_document_build(const struct LuminatePolicyDocumentBuilder *builder,
                                              struct LuminatePolicyDocument **out_document);

/**
 * Returns a built document's revision, or `0` if `document` is null.
 */
LUMINATE_API
uint64_t luminate_policy_document_revision(const struct LuminatePolicyDocument *document);

/**
 * Number of roles in a built document.
 */
LUMINATE_API
uintptr_t luminate_policy_document_role_count(const struct LuminatePolicyDocument *document);

/**
 * Role name at `role_index`, in canonical order.
 */
LUMINATE_API
struct LuminateStringView luminate_policy_document_role_name_at(const struct LuminatePolicyDocument *document,
                                                                uintptr_t role_index);

/**
 * Borrowed role at `role_index`, in canonical order.
 */
LUMINATE_API
const struct LuminatePolicyRole *luminate_policy_document_role_at(const struct LuminatePolicyDocument *document,
                                                                  uintptr_t role_index);

/**
 * Number of parent roles for a role.
 */
LUMINATE_API uintptr_t luminate_policy_role_parent_count(const struct LuminatePolicyRole *role);

/**
 * Parent role name at `index`, in canonical order.
 */
LUMINATE_API
struct LuminateStringView luminate_policy_role_parent_at(const struct LuminatePolicyRole *role,
                                                         uintptr_t index);

/**
 * Number of rules on a role.
 */
LUMINATE_API uintptr_t luminate_policy_role_rule_count(const struct LuminatePolicyRole *role);

/**
 * Borrowed rule at `index`.
 */
LUMINATE_API
const struct LuminatePolicyRule *luminate_policy_role_rule_at(const struct LuminatePolicyRole *role,
                                                              uintptr_t index);

/**
 * Stable ID of a rule.
 */
LUMINATE_API
struct LuminateStringView luminate_policy_rule_id(const struct LuminatePolicyRule *rule);

/**
 * Effect of a rule, or `LUMINATE_DISCRIMINANT_INVALID`.
 */
LUMINATE_API LuminateRuleEffect luminate_policy_rule_effect(const struct LuminatePolicyRule *rule);

/**
 * Number of semantic operations matched by a rule.
 */
LUMINATE_API uintptr_t luminate_policy_rule_operation_count(const struct LuminatePolicyRule *rule);

/**
 * Semantic operation at `operation_index`, in canonical order.
 */
LUMINATE_API
uint32_t luminate_policy_rule_operation_at(const struct LuminatePolicyRule *rule,
                                           uintptr_t index);

/**
 * Safe reason attached to a rule, or an absent view.
 */
LUMINATE_API
struct LuminateStringView luminate_policy_rule_reason(const struct LuminatePolicyRule *rule);

/**
 * Writes a rule's cache hint in milliseconds when present.
 */
LUMINATE_API
bool luminate_policy_rule_cache_hint_ms(const struct LuminatePolicyRule *rule,
                                        uint64_t *out_cache_hint_ms);

/**
 * Writes a rule's host-attachment constraint when present.
 */
LUMINATE_API
bool luminate_policy_rule_host_attached(const struct LuminatePolicyRule *rule,
                                        bool *out_host_attached);

/**
 * Number of bindings in a built document.
 */
LUMINATE_API
uintptr_t luminate_policy_document_binding_count(const struct LuminatePolicyDocument *document);

/**
 * Borrowed binding at `index`.
 */
LUMINATE_API
const struct LuminatePolicyBinding *luminate_policy_document_binding_at(const struct LuminatePolicyDocument *document,
                                                                        uintptr_t index);

/**
 * Authority matched by a binding.
 */
LUMINATE_API
struct LuminateStringView luminate_policy_binding_authority(const struct LuminatePolicyBinding *binding);

/**
 * Creates an owned remote principal from exact, opaque identity strings.
 * Release with `luminate_remote_principal_free`.
 *
 * # Safety
 *
 * `authority` and `subject` must point to valid NUL-terminated UTF-8
 * strings. `groups` must point to `group_count` valid string pointers, or
 * may be null when `group_count` is `0`. `out_principal` must be a valid
 * non-null out-pointer.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_remote_principal_new(const char *authority,
                                             const char *subject,
                                             const char *const *groups,
                                             uintptr_t group_count,
                                             struct LuminateRemotePrincipal **out_principal);

/**
 * The principal's identity provider or trust domain.
 */
LUMINATE_API
struct LuminateStringView luminate_remote_principal_authority(const struct LuminateRemotePrincipal *v);

/**
 * The principal's authority-local subject identifier.
 */
LUMINATE_API
struct LuminateStringView luminate_remote_principal_subject(const struct LuminateRemotePrincipal *v);

/**
 * Number of verified groups on this principal.
 */
LUMINATE_API
uintptr_t luminate_remote_principal_group_count(const struct LuminateRemotePrincipal *v);

/**
 * Group at `index` in ascending sorted order, or an absent view if out of
 * range.
 */
LUMINATE_API
struct LuminateStringView luminate_remote_principal_group_at(const struct LuminateRemotePrincipal *v,
                                                             uintptr_t index);

/**
 * Evaluates one authorization request against a built document and writes
 * an owned `LuminateAuthorizationEvaluation`. `operation` is one of the
 * `LUMINATE_POLICY_OP_*` values. `resources` may be null when
 * `resource_count` is `0`, meaning the request has not yet been resolved to
 * concrete resources. Release the result with
 * `luminate_authorization_evaluation_free`.
 *
 * # Safety
 *
 * `document` and `principal` must be valid pointers. `resources` must point
 * to `resource_count` valid `LuminateResourceInput`s. `out_evaluation` must
 * be a valid non-null out-pointer.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_policy_document_evaluate(const struct LuminatePolicyDocument *document,
                                                 const struct LuminateRemotePrincipal *principal,
                                                 LuminatePolicyOperation operation,
                                                 const struct LuminateResourceInput *resources,
                                                 uintptr_t resource_count,
                                                 struct LuminateAuthorizationEvaluation **out_evaluation);

/**
 * Whether the evaluated request is allowed. Returns `false` if `v` is null.
 */
LUMINATE_API
bool luminate_authorization_evaluation_is_allowed(const struct LuminateAuthorizationEvaluation *v);

/**
 * The decision's validated caller-safe diagnostic, or an absent view.
 */
LUMINATE_API
struct LuminateStringView luminate_authorization_evaluation_reason(const struct LuminateAuthorizationEvaluation *v);

/**
 * The stable identifier of the matching rule, or an absent view for a
 * default denial.
 */
LUMINATE_API
struct LuminateStringView luminate_authorization_evaluation_audit_rule(const struct LuminateAuthorizationEvaluation *v);

/**
 * Whether the decision carries a cache lifetime hint.
 */
LUMINATE_API
bool luminate_authorization_evaluation_has_cache_hint_ms(const struct LuminateAuthorizationEvaluation *v);

/**
 * The decision's cache lifetime hint in milliseconds, clamped to
 * `luminate_core::policy::MAX_CACHE_HINT`; `0` if absent or `v` is null.
 */
LUMINATE_API
uint64_t luminate_authorization_evaluation_cache_hint_ms(const struct LuminateAuthorizationEvaluation *v);

/**
 * The policy revision under which the decision was made.
 */
LUMINATE_API
uint64_t luminate_authorization_evaluation_revision(const struct LuminateAuthorizationEvaluation *v);

/**
 * Creates an independent scene builder seeded from every editable field in
 * `scene`. Release it with `luminate_scene_builder_free`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_scene_builder_from_scene(const struct LuminateScene *scene,
                                                 struct LuminateSceneBuilder **out_builder);

/**
 * Replaces the builder's required UTF-8 scene name.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_scene_builder_set_name(struct LuminateSceneBuilder *builder,
                                               const char *name);

/**
 * Replaces the builder's UTF-8 description, or clears it when `description`
 * is null.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_scene_builder_set_description(struct LuminateSceneBuilder *builder,
                                                      const char *description);

/**
 * Number of bindings currently held by a scene builder.
 */
LUMINATE_API
uintptr_t luminate_scene_builder_binding_count(const struct LuminateSceneBuilder *builder);

/**
 * Borrowed binding at `index`, or null when the builder is null or the index
 * is out of range. The view is invalidated by the next builder mutation.
 */
LUMINATE_API
const struct LuminateSceneBinding *luminate_scene_builder_binding_at(const struct LuminateSceneBuilder *builder,
                                                                     uintptr_t index);

/**
 * Appends a deep copy of one validated binding input.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_scene_builder_add_binding(struct LuminateSceneBuilder *builder,
                                                  const struct LuminateSceneBindingInput *binding);

/**
 * Replaces the binding at `index` with a deep copy of `binding`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_scene_builder_replace_binding(struct LuminateSceneBuilder *builder,
                                                      uintptr_t index,
                                                      const struct LuminateSceneBindingInput *binding);

/**
 * Removes the binding at `index`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_scene_builder_remove_binding(struct LuminateSceneBuilder *builder,
                                                     uintptr_t index);

/**
 * Replaces the source scene using the builder's stored identifier and
 * expected revision. The builder is neither consumed nor mutated.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_replace_scene_from_builder(struct LuminateClient *client,
                                                          const struct LuminateSceneBuilder *builder,
                                                          struct LuminateSceneSnapshot **out);

/**
 * Creates an explicitly-authored scene.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_create_scene(struct LuminateClient *client,
                                            const char *name,
                                            const char *description,
                                            const struct LuminateSceneBindingInput *bindings,
                                            uintptr_t binding_count,
                                            struct LuminateSceneSnapshot **out);

/**
 * Captures intended state. A non-null `dynamic_collection_id` captures its
 * current leaves when `target_count` is zero.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_capture_scene(struct LuminateClient *client,
                                             const char *name,
                                             const char *description,
                                             const char *dynamic_collection_id,
                                             const struct LuminateTarget *targets,
                                             uintptr_t target_count,
                                             struct LuminateSceneSnapshot **out);

/**
 * Replaces an explicitly-authored scene.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_replace_scene(struct LuminateClient *client,
                                             const char *id,
                                             uint64_t expected_revision,
                                             const char *name,
                                             const char *description,
                                             const struct LuminateSceneBindingInput *bindings,
                                             uintptr_t binding_count,
                                             struct LuminateSceneSnapshot **out);

/**
 * Recaptures intended state at an expected revision.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_recapture_scene(struct LuminateClient *client,
                                               const char *id,
                                               uint64_t expected_revision,
                                               const char *dynamic_collection_id,
                                               const struct LuminateTarget *targets,
                                               uintptr_t target_count,
                                               struct LuminateSceneSnapshot **out);

/**
 * Deletes a scene at an expected revision.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_delete_scene(struct LuminateClient *client,
                                            const char *id,
                                            uint64_t expected_revision);

/**
 * Lists observable scenes.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_list_scenes(struct LuminateClient *client,
                                           struct LuminateSceneList **out);

/**
 * Gets one observable scene.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_get_scene(struct LuminateClient *client,
                                         const char *id,
                                         struct LuminateSceneSnapshot **out);

/**
 * Applies a scene immediately. Authorization exclusions are available
 * through ordinary Rust APIs; this convenience call reports successful
 * completion only.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_apply_scene(struct LuminateClient *client,
                                           const char *id);

LUMINATE_API uintptr_t luminate_scene_list_count(const struct LuminateSceneList *v);

LUMINATE_API
const struct LuminateScene *luminate_scene_list_at(const struct LuminateSceneList *v,
                                                   uintptr_t index);

LUMINATE_API
const struct LuminateScene *luminate_scene_snapshot_scene(const struct LuminateSceneSnapshot *v);

LUMINATE_API uint64_t luminate_scene_revision(const struct LuminateScene *v);

LUMINATE_API struct LuminateStringView luminate_scene_description(const struct LuminateScene *v);

LUMINATE_API
const struct LuminateTargetView *luminate_scene_binding_target(const struct LuminateSceneBinding *v);

LUMINATE_API
struct LuminateStringView luminate_scene_binding_collection_id(const struct LuminateSceneBinding *v);

LUMINATE_API
const struct LuminateEffectView *luminate_scene_binding_appearance(const struct LuminateSceneBinding *v);

/**
 * Number of appearance-slot values stored by this binding.
 */
LUMINATE_API
uintptr_t luminate_scene_binding_appearance_slot_count(const struct LuminateSceneBinding *v);

/**
 * Borrowed appearance-slot value at `index`, or null.
 */
LUMINATE_API
const struct LuminateAppearanceSlotValue *luminate_scene_binding_appearance_slot_at(const struct LuminateSceneBinding *v,
                                                                                    uintptr_t index);

LUMINATE_API bool luminate_scene_binding_has_brightness(const struct LuminateSceneBinding *v);

LUMINATE_API uint32_t luminate_scene_binding_brightness(const struct LuminateSceneBinding *v);

LUMINATE_API bool luminate_scene_binding_has_emission(const struct LuminateSceneBinding *v);

LUMINATE_API
LuminateEmissionState luminate_scene_binding_emission(const struct LuminateSceneBinding *v);

LUMINATE_API
const struct LuminateOwnerIdentity *luminate_scene_owner(const struct LuminateScene *v);

LUMINATE_API
LuminateStatus luminate_client_refresh_state(struct LuminateClient *client,
                                             const char *device_id);

LUMINATE_API
LuminateStatus luminate_client_save_current(struct LuminateClient *client,
                                            const struct LuminateTarget *target);

LUMINATE_API
LuminateStatus luminate_client_set_brightness_selector(struct LuminateClient *client,
                                                       const struct LuminateSelectorInput *selector,
                                                       uint32_t value,
                                                       bool has_policy,
                                                       LuminateUnsupportedPolicy policy,
                                                       struct LuminateCollectionOutcome **output);

LUMINATE_API
LuminateStatus luminate_client_set_effect_selector(struct LuminateClient *client,
                                                   const struct LuminateSelectorInput *selector,
                                                   const struct LuminateEffect *effect,
                                                   bool has_policy,
                                                   LuminateUnsupportedPolicy policy,
                                                   struct LuminateCollectionOutcome **output);

LUMINATE_API
LuminateStatus luminate_client_set_emission_selector(struct LuminateClient *client,
                                                     const struct LuminateSelectorInput *selector,
                                                     uint32_t state,
                                                     struct LuminateCollectionOutcome **output);

LUMINATE_API
uintptr_t luminate_collection_outcome_applied_count(const struct LuminateCollectionOutcome *value);

LUMINATE_API
const struct LuminateTargetView *luminate_collection_outcome_applied_at(const struct LuminateCollectionOutcome *value,
                                                                        uintptr_t index);

LUMINATE_API
uintptr_t luminate_collection_outcome_denied_count(const struct LuminateCollectionOutcome *value);

LUMINATE_API
const struct LuminateTargetView *luminate_collection_outcome_denied_at(const struct LuminateCollectionOutcome *value,
                                                                       uintptr_t index);

/**
 * Releases an owned plugin setup session. Null is a no-op.
 */
LUMINATE_API void luminate_plugin_setup_session_free(struct LuminatePluginSetupSession *value);

/**
 * Returns the opaque canonical session identifier.
 */
LUMINATE_API
struct LuminateStringView luminate_plugin_setup_session_id(const struct LuminatePluginSetupSession *value);

/**
 * Returns the canonical plugin name.
 */
LUMINATE_API
struct LuminateStringView luminate_plugin_setup_session_plugin(const struct LuminatePluginSetupSession *value);

/**
 * Returns the stable plugin-local workflow identifier.
 */
LUMINATE_API
struct LuminateStringView luminate_plugin_setup_session_workflow(const struct LuminatePluginSetupSession *value);

/**
 * Returns the interaction generation required by the next response.
 */
LUMINATE_API
uint64_t luminate_plugin_setup_session_generation(const struct LuminatePluginSetupSession *value);

/**
 * Returns the `LuminatePluginSetupSessionState` discriminant.
 */
LUMINATE_API
LuminatePluginSetupSessionState luminate_plugin_setup_session_state(const struct LuminatePluginSetupSession *value);

/**
 * Returns the choice prompt, physical instruction, completion summary, or
 * failure diagnostic for the current state.
 */
LUMINATE_API
struct LuminateStringView luminate_plugin_setup_session_message(const struct LuminatePluginSetupSession *value);

/**
 * Writes the committed management revision for a completed session.
 *
 * Returns false and leaves `out_revision` unchanged unless the session is
 * completed or either pointer is null.
 */
LUMINATE_API
bool luminate_plugin_setup_session_revision(const struct LuminatePluginSetupSession *value,
                                            uint64_t *out_revision);

/**
 * Returns the number of choices in the current choice interaction.
 */
LUMINATE_API
uintptr_t luminate_plugin_setup_session_choice_count(const struct LuminatePluginSetupSession *value);

/**
 * Returns the borrowed choice at `index`, or null when unavailable.
 */
LUMINATE_API
const struct LuminatePluginSetupChoice *luminate_plugin_setup_session_choice_at(const struct LuminatePluginSetupSession *value,
                                                                                uintptr_t index);

/**
 * Returns the choice's stable identifier, or an absent view.
 */
LUMINATE_API
struct LuminateStringView luminate_plugin_setup_choice_id(const struct LuminatePluginSetupChoice *value);

/**
 * Returns the choice's label, or an absent view.
 */
LUMINATE_API
struct LuminateStringView luminate_plugin_setup_choice_label(const struct LuminatePluginSetupChoice *value);

/**
 * Returns the optional choice description, or an absent view.
 */
LUMINATE_API
struct LuminateStringView luminate_plugin_setup_choice_description(const struct LuminatePluginSetupChoice *value);

/**
 * Releases an owned plugin setup workflow list. Null is a no-op.
 */
LUMINATE_API
void luminate_plugin_setup_workflow_list_free(struct LuminatePluginSetupWorkflowList *value);

/**
 * Returns the number of setup workflows in `value`, or zero for null.
 */
LUMINATE_API
uintptr_t luminate_plugin_setup_workflow_list_count(const struct LuminatePluginSetupWorkflowList *value);

/**
 * Returns the borrowed workflow at `index`, or null when out of range.
 */
LUMINATE_API
const struct LuminatePluginSetupWorkflow *luminate_plugin_setup_workflow_list_at(const struct LuminatePluginSetupWorkflowList *value,
                                                                                 uintptr_t index);

/**
 * Returns the canonical plugin name which owns `value`.
 */
LUMINATE_API
struct LuminateStringView luminate_plugin_setup_workflow_plugin(const struct LuminatePluginSetupWorkflow *value);

/**
 * Returns the stable plugin-local workflow identifier.
 */
LUMINATE_API
struct LuminateStringView luminate_plugin_setup_workflow_id(const struct LuminatePluginSetupWorkflow *value);

/**
 * Returns the short human-readable workflow name.
 */
LUMINATE_API
struct LuminateStringView luminate_plugin_setup_workflow_label(const struct LuminatePluginSetupWorkflow *value);

/**
 * Returns the human-readable workflow description.
 */
LUMINATE_API
struct LuminateStringView luminate_plugin_setup_workflow_description(const struct LuminatePluginSetupWorkflow *value);

/**
 * Returns the workflow's `LuminatePluginSetupWorkflowKind` value.
 */
LUMINATE_API
LuminatePluginSetupWorkflowKind luminate_plugin_setup_workflow_kind(const struct LuminatePluginSetupWorkflow *value);

/**
 * Starts one advertised plugin setup workflow.
 */
LUMINATE_API
LuminateStatus luminate_client_plugin_setup_start(struct LuminateClient *client,
                                                  const char *plugin,
                                                  const char *workflow,
                                                  struct LuminatePluginSetupSession **out_session);

/**
 * Selects one choice in the current plugin setup interaction.
 */
LUMINATE_API
LuminateStatus luminate_client_plugin_setup_choose(struct LuminateClient *client,
                                                   const char *session_id,
                                                   uint64_t generation,
                                                   const char *choice,
                                                   struct LuminatePluginSetupSession **out_session);

/**
 * Confirms the current plugin setup physical action.
 */
LUMINATE_API
LuminateStatus luminate_client_plugin_setup_confirm(struct LuminateClient *client,
                                                    const char *session_id,
                                                    uint64_t generation,
                                                    struct LuminatePluginSetupSession **out_session);

/**
 * Reads the current state of an actor-owned plugin setup session.
 */
LUMINATE_API
LuminateStatus luminate_client_plugin_setup_get(struct LuminateClient *client,
                                                const char *session_id,
                                                struct LuminatePluginSetupSession **out_session);

/**
 * Cancels an actor-owned plugin setup session.
 */
LUMINATE_API
LuminateStatus luminate_client_plugin_setup_cancel(struct LuminateClient *client,
                                                   const char *session_id,
                                                   struct LuminatePluginSetupSession **out_session);

/**
 * Lists setup workflows advertised by one installed plugin.
 */
LUMINATE_API
LuminateStatus luminate_client_plugin_setup_workflows(struct LuminateClient *client,
                                                      const char *plugin,
                                                      struct LuminatePluginSetupWorkflowList **out_workflows);

/**
 * Negotiates and begins a client-published shared-memory frame stream on
 * the given target, writing the ready handle to `out_stream`.
 * [`crate::LuminateStatusUnsupported`]-equivalent
 * (`LUMINATE_STATUS_UNSUPPORTED`) means the fast path isn't offered to
 * this connection or target; the caller's fallback is
 * `luminate_client_begin_frame_stream`. The returned stream retains its
 * originating connection and may outlive the public client handle.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_begin_shm_frame_stream(struct LuminateClient *client,
                                                      const struct LuminateTarget *target,
                                                      struct LuminateShmFrameStream **out_stream);

/**
 * Publishes one full frame of colours to an active client-published
 * shared-memory stream. `colours` must have exactly the pixel count the
 * stream's target advertised; a mismatched count is reported as
 * [`crate::LuminateStatusInvalidArgument`]-equivalent
 * (`LUMINATE_STATUS_INVALID_ARGUMENT`), not silently truncated or padded.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_shm_upload_frame_full(struct LuminateShmFrameStream *stream,
                                                     const struct LuminateRgb *colours,
                                                     uintptr_t count,
                                                     uint8_t commit,
                                                     struct LuminateShmFrameAck *out_ack);

/**
 * Ends the client-published shared-memory stream, telling the daemon over
 * its original connection, then frees the handle. Idempotent-safe to call
 * with null (a no-op), but not safe to call twice on the same non-null
 * pointer: like every other owned root in this crate, the handle is
 * consumed and invalidated by this call.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_end_shm_frame_stream(struct LuminateShmFrameStream *stream);

/**
 * Collection id described by this aggregate state snapshot.
 */
LUMINATE_API
struct LuminateStringView luminate_collection_state_id(const struct LuminateCollectionStateSnapshot *value);

/**
 * Whether configured appearance is known for every collection constituent.
 */
LUMINATE_API
bool luminate_collection_state_has_appearance(const struct LuminateCollectionStateSnapshot *value);

/**
 * Configured appearance kind, including `LUMINATE_APPEARANCE_MIXED`, or
 * `LUMINATE_DISCRIMINANT_INVALID` when unknown.
 */
LUMINATE_API
uint32_t luminate_collection_state_appearance_kind(const struct LuminateCollectionStateSnapshot *value);

/**
 * Borrowed configured static colour, or null when configured appearance is
 * unknown or is not static. The view remains valid while `value` remains
 * alive.
 */
LUMINATE_API
const struct LuminateColour *luminate_collection_state_appearance_colour(const struct LuminateCollectionStateSnapshot *value);

/**
 * Borrowed configured effect, or null when configured appearance is unknown
 * or is not an effect. The view remains valid while `value` remains alive.
 */
LUMINATE_API
const struct LuminateEffectView *luminate_collection_state_appearance_effect(const struct LuminateCollectionStateSnapshot *value);

/**
 * Whether effective appearance is known for every collection constituent.
 */
LUMINATE_API
bool luminate_collection_state_has_effective_appearance(const struct LuminateCollectionStateSnapshot *value);

/**
 * Effective appearance kind, including
 * `LUMINATE_EFFECTIVE_APPEARANCE_MIXED`, or
 * `LUMINATE_DISCRIMINANT_INVALID` when unknown.
 */
LUMINATE_API
uint32_t luminate_collection_state_effective_appearance_kind(const struct LuminateCollectionStateSnapshot *value);

/**
 * Borrowed effective static colour, or null when effective appearance is
 * unknown, off, streaming, mixed, or is not static. The view remains valid
 * while `value` remains alive.
 */
LUMINATE_API
const struct LuminateColour *luminate_collection_state_effective_appearance_colour(const struct LuminateCollectionStateSnapshot *value);

/**
 * Borrowed effective effect, or null when effective appearance is unknown,
 * off, streaming, mixed, or is not an effect. The view remains valid while
 * `value` remains alive.
 */
LUMINATE_API
const struct LuminateEffectView *luminate_collection_state_effective_appearance_effect(const struct LuminateCollectionStateSnapshot *value);

/**
 * Finds the observation for a specific target and facet kind, or null if
 * none matches. `kind` is one of the `LUMINATE_FACET_*` values. `target` is
 * a borrowed target view from an observation or adoption record of this
 * same state; this is the one supported way to look up a specific facet.
 * `observations` carries no ordering contract, so a caller assuming a fixed
 * position (for example that `Appearance` is always first) sees whatever a
 * given daemon build happens to sort by, not a guarantee.
 */
LUMINATE_API
const struct LuminateFacetObservation *luminate_state_find_observation(const struct LuminateState *state,
                                                                       const struct LuminateTargetView *target,
                                                                       uint32_t kind);

/**
 * Whether the device is currently reachable; one of the
 * `LUMINATE_REACHABILITY_*` values.
 */
LUMINATE_API LuminateReachability luminate_state_reachability(const struct LuminateState *v);

/**
 * Progress of reconciling desired state with hardware; one of the
 * `LUMINATE_RECONCILIATION_*` values.
 */
LUMINATE_API
LuminateReconciliationStatus luminate_state_reconciliation(const struct LuminateState *v);

/**
 * The most recent reconciliation error message, or an absent view if there
 * was none.
 */
LUMINATE_API struct LuminateStringView luminate_state_latest_error(const struct LuminateState *v);

/**
 * Whether a most-recent reconciliation attempt timestamp is recorded.
 */
LUMINATE_API bool luminate_state_has_latest_attempt_ms(const struct LuminateState *v);

/**
 * Timestamp of the most recent reconciliation attempt in milliseconds, or 0
 * if none is recorded.
 */
LUMINATE_API uint64_t luminate_state_latest_attempt_ms(const struct LuminateState *v);

/**
 * Borrowed target this observation describes.
 */
LUMINATE_API
const struct LuminateTargetView *luminate_observation_target(const struct LuminateFacetObservation *v);

/**
 * Borrowed observed facet value.
 */
LUMINATE_API
const struct LuminateFacetValue *luminate_observation_value(const struct LuminateFacetObservation *v);

/**
 * How this observation was obtained; one of the `LUMINATE_CONFIDENCE_*`
 * values.
 */
LUMINATE_API
LuminateObservationConfidence luminate_observation_confidence(const struct LuminateFacetObservation *v);

/**
 * Where this observation's value came from; one of the
 * `LUMINATE_SOURCE_*` values.
 */
LUMINATE_API
LuminateObservationSource luminate_observation_source(const struct LuminateFacetObservation *v);

/**
 * Timestamp this observation was made, in milliseconds.
 */
LUMINATE_API uint64_t luminate_observation_observed_at_ms(const struct LuminateFacetObservation *v);

/**
 * Whether this observation is considered stale.
 */
LUMINATE_API bool luminate_observation_stale(const struct LuminateFacetObservation *v);

/**
 * Borrowed target this adoption record describes.
 */
LUMINATE_API
const struct LuminateTargetView *luminate_adoption_target(const struct LuminateAdoption *v);

/**
 * Which facet this adoption record covers; one of the `LUMINATE_FACET_*`
 * values.
 */
LUMINATE_API LuminateStateFacetKind luminate_adoption_facet(const struct LuminateAdoption *v);

/**
 * Whether the facet's pre-existing hardware state was adopted; one of the
 * `LUMINATE_ADOPTION_*` values.
 */
LUMINATE_API LuminateAdoptionStatus luminate_adoption_status(const struct LuminateAdoption *v);

/**
 * Which addressing level this target names; one of the `LUMINATE_TARGET_*`
 * values.
 */
LUMINATE_API LuminateTargetKind luminate_target_view_kind(const struct LuminateTargetView *v);

/**
 * The target's device id.
 */
LUMINATE_API
struct LuminateStringView luminate_target_view_device_id(const struct LuminateTargetView *v);

/**
 * The target's surface id, or an absent view unless the target addresses a
 * surface or element.
 */
LUMINATE_API
struct LuminateStringView luminate_target_view_surface_id(const struct LuminateTargetView *v);

/**
 * The target's element id, or an absent view unless the target addresses an
 * element.
 */
LUMINATE_API
struct LuminateStringView luminate_target_view_element_id(const struct LuminateTargetView *v);

/**
 * The target's group id, or an absent view unless the target addresses a
 * group.
 */
LUMINATE_API
struct LuminateStringView luminate_target_view_group_id(const struct LuminateTargetView *v);

/**
 * Which state facet this value represents; one of the `LUMINATE_FACET_*`
 * values.
 */
LUMINATE_API LuminateStateFacetKind luminate_facet_value_kind(const struct LuminateFacetValue *v);

/**
 * Whether the appearance value is a static colour or a running effect; one
 * of the `LUMINATE_APPEARANCE_*` values, or `LUMINATE_DISCRIMINANT_INVALID`
 * unless this is an appearance facet.
 */
LUMINATE_API
LuminateAppearanceKind luminate_facet_value_appearance_kind(const struct LuminateFacetValue *v);

/**
 * Whether the effective-appearance value is off, a static colour, a running
 * effect, or an active frame stream; one of the
 * `LUMINATE_EFFECTIVE_APPEARANCE_*` values, or `LUMINATE_DISCRIMINANT_INVALID`
 * unless this is an effective-appearance facet.
 */
LUMINATE_API
LuminateEffectiveAppearanceKind luminate_facet_value_effective_appearance_kind(const struct LuminateFacetValue *v);

/**
 * Borrowed static colour, or null unless this is a static appearance or
 * effective-appearance facet.
 */
LUMINATE_API
const struct LuminateColour *luminate_facet_value_colour(const struct LuminateFacetValue *v);

/**
 * Borrowed running effect, or null unless this is an effect appearance or
 * effective-appearance facet.
 */
LUMINATE_API
const struct LuminateEffectView *luminate_facet_value_effect(const struct LuminateFacetValue *v);

/**
 * Writes whether an appearance-slots facet contains every advertised slot.
 *
 * Returns false and leaves `out_complete` unchanged unless this is an
 * appearance-slots facet or either pointer is null.
 */
LUMINATE_API
bool luminate_facet_value_appearance_slots_complete(const struct LuminateFacetValue *v,
                                                    bool *out_complete);

/**
 * Number of known values in an appearance-slots facet.
 */
LUMINATE_API
uintptr_t luminate_facet_value_appearance_slot_count(const struct LuminateFacetValue *v);

/**
 * Borrowed appearance-slot value at `index`, or null.
 */
LUMINATE_API
const struct LuminateAppearanceSlotValue *luminate_facet_value_appearance_slot_at(const struct LuminateFacetValue *v,
                                                                                  uintptr_t index);

/**
 * Writes the brightness value.
 *
 * Returns false and leaves `out_brightness` unchanged unless this is a
 * brightness facet or either pointer is null.
 */
LUMINATE_API
bool luminate_facet_value_brightness(const struct LuminateFacetValue *v,
                                     uint32_t *out_brightness);

/**
 * Whether the target is emitting light; one of the `LUMINATE_EMISSION_*`
 * values, or `LUMINATE_DISCRIMINANT_INVALID` unless this is an emission
 * facet.
 */
LUMINATE_API
LuminateEmissionState luminate_facet_value_emission(const struct LuminateFacetValue *v);

/**
 * Whether the target's physical power is on; one of the
 * `LUMINATE_PHYSICAL_POWER_*` values, or `LUMINATE_DISCRIMINANT_INVALID`
 * unless this is a physical-power facet.
 */
LUMINATE_API
LuminatePhysicalPowerState luminate_facet_value_physical_power(const struct LuminateFacetValue *v);

/**
 * How this colour's channels are interpreted; one of the
 * `LUMINATE_COLOUR_ENCODING_*` values.
 */
LUMINATE_API LuminateColourEncoding luminate_colour_encoding(const struct LuminateColour *v);

/**
 * Number of channel values in this colour.
 */
LUMINATE_API uintptr_t luminate_colour_channel_count(const struct LuminateColour *v);

/**
 * Copies the channel and raw value at `index` into caller-owned outputs.
 *
 * Returns false for a null colour, null output, or out-of-range index. Both
 * outputs remain unchanged on failure.
 */
LUMINATE_API
bool luminate_colour_channel_at(const struct LuminateColour *v,
                                uintptr_t index,
                                LuminateColourChannel *out_channel,
                                uint32_t *out_value);

/**
 * Looks up a colour component by `LuminateColourChannel`.
 *
 * Returns false for a null colour, invalid or absent channel, or null output.
 */
LUMINATE_API
bool luminate_colour_value(const struct LuminateColour *v,
                           LuminateColourChannel channel,
                           uint32_t *out_value);

/**
 * Starts a scene-to-scene transition.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_transition_scene_to_scene(struct LuminateClient *client,
                                                         const char *source_scene_id,
                                                         const char *destination_scene_id,
                                                         struct LuminateTransitionOptions timing,
                                                         struct LuminateTransitionSnapshot **out);

/**
 * Starts a current-to-scene transition.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_transition_current_to_scene(struct LuminateClient *client,
                                                           const char *destination_scene_id,
                                                           struct LuminateTransitionOptions timing,
                                                           struct LuminateTransitionSnapshot **out);

/**
 * Starts a scene-to-ephemeral-state transition.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_transition_scene_to_states(struct LuminateClient *client,
                                                          const char *source_scene_id,
                                                          const struct LuminateTransitionTargetStateInput *states,
                                                          uintptr_t state_count,
                                                          struct LuminateTransitionOptions timing,
                                                          struct LuminateTransitionSnapshot **out);

/**
 * Starts a current-to-ephemeral-state transition.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_transition_current_to_states(struct LuminateClient *client,
                                                            const struct LuminateTransitionTargetStateInput *states,
                                                            uintptr_t state_count,
                                                            struct LuminateTransitionOptions timing,
                                                            struct LuminateTransitionSnapshot **out);

/**
 * Gets a transition snapshot.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_transition_get(struct LuminateClient *client,
                                              const char *id,
                                              struct LuminateTransitionSnapshot **out);

/**
 * Aborts a transition and waits until no later step can write.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_transition_abort(struct LuminateClient *client,
                                                const char *id,
                                                struct LuminateTransitionSnapshot **out);

/**
 * Waits for a terminal transition.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_transition_wait(struct LuminateClient *client,
                                               const char *id,
                                               struct LuminateTransitionSnapshot **out);

/**
 * Frees an owned transition snapshot.
 */
LUMINATE_API void luminate_transition_snapshot_free(struct LuminateTransitionSnapshot *value);

/**
 * Borrowed transition identifier.
 */
LUMINATE_API
struct LuminateStringView luminate_transition_id(const struct LuminateTransitionSnapshot *value);

/**
 * Elapsed milliseconds.
 */
LUMINATE_API
uint64_t luminate_transition_elapsed_ms(const struct LuminateTransitionSnapshot *value);

/**
 * Requested duration in milliseconds.
 */
LUMINATE_API
uint64_t luminate_transition_duration_ms(const struct LuminateTransitionSnapshot *value);

/**
 * Status kind: 0 active, 1 completed, 2 cancelled, or 3 failed.
 */
LUMINATE_API
uint32_t luminate_transition_status_kind(const struct LuminateTransitionSnapshot *value);

/**
 * Cancellation reason: 0 aborted, 1 replaced, 2 conflicting mutation, or 3
 * authorization expired. Returns `UINT32_MAX` unless the transition was
 * cancelled.
 */
LUMINATE_API
uint32_t luminate_transition_cancellation_reason(const struct LuminateTransitionSnapshot *value);

/**
 * Borrowed runtime-failure diagnostic, or an empty view for other statuses.
 */
LUMINATE_API
struct LuminateStringView luminate_transition_failure_diagnostic(const struct LuminateTransitionSnapshot *value);

/**
 * Number of targets updated by the failed step.
 */
LUMINATE_API
uintptr_t luminate_transition_failed_target_count(const struct LuminateTransitionSnapshot *value);

/**
 * Borrowed target updated by the failed step at `index`, or null.
 */
LUMINATE_API
const struct LuminateTargetView *luminate_transition_failed_target_at(const struct LuminateTransitionSnapshot *value,
                                                                      uintptr_t index);

/**
 * Number of concrete controlled targets.
 */
LUMINATE_API
uintptr_t luminate_transition_target_count(const struct LuminateTransitionSnapshot *value);

/**
 * Borrowed controlled target at `index`, or null.
 */
LUMINATE_API
const struct LuminateTargetView *luminate_transition_target_at(const struct LuminateTransitionSnapshot *value,
                                                               uintptr_t index);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus


/*
 * Typed C11 accessor API.
 *
 * Every pointer below (other than the four `*_snapshot`/`LuminateEvent`
 * roots and `LuminateEffect`) is a *borrowed view*: it points into memory
 * owned by the snapshot, state, or event it came from, remains valid only as
 * long as that owning value has not been freed, and must never be freed on
 * its own. `LuminateStringView` values borrow the same way. `count`/`at`
 * pairs follow the same convention throughout: `at` returns null for an
 * out-of-range index, and accessors on a null input return a zeroed/null/
 * `UINT32_MAX` sentinel rather than trapping. Passing a non-null pointer that
 * does not point to the expected type is undefined behaviour.
 */

/**
 * One hardware-driven effect a device advertises, borrowed from the owning
 * `LuminateHardwareEffectsCapability`.
 */
typedef struct LuminateHardwareEffectDescriptor LuminateHardwareEffectDescriptor;

/** Which effect variant `LuminateEffect` holds; one of the `LUMINATE_EFFECT_*` values. */
typedef uint32_t LuminateEffectKind;

/** Sentinel returned by kind/enum accessors when the input pointer is null or otherwise unreadable. */
#define LUMINATE_DISCRIMINANT_INVALID UINT32_MAX

/* LuminateSurfaceKind: shape of a surface's addressable elements. */
#define LUMINATE_SURFACE_OPAQUE UINT32_C(0) /**< No addressable internal structure. */
#define LUMINATE_SURFACE_ZONE UINT32_C(1) /**< A single uniformly-lit zone. */
#define LUMINATE_SURFACE_LINEAR UINT32_C(2) /**< A 1D strip; see `luminate_surface_linear_length`. */
#define LUMINATE_SURFACE_SPARSE_2D UINT32_C(3) /**< Irregularly placed 2D points; see `luminate_surface_sparse_size`. */
#define LUMINATE_SURFACE_MATRIX UINT32_C(4) /**< A dense row/column grid; see `luminate_surface_matrix_size`. */

/* LuminateElementKind: role of one element within a surface. */
#define LUMINATE_ELEMENT_KEY UINT32_C(0) /**< A keyboard key. */
#define LUMINATE_ELEMENT_LED UINT32_C(1) /**< A single discrete LED. */
#define LUMINATE_ELEMENT_ZONE UINT32_C(2) /**< A sub-zone of a surface. */
#define LUMINATE_ELEMENT_LOGO UINT32_C(3) /**< A logo or badge light. */
#define LUMINATE_ELEMENT_RING_SEGMENT UINT32_C(4) /**< One segment of a ring light. */

/* LuminateGeometryKind: which field of an element's optional geometry union is populated. */
#define LUMINATE_GEOMETRY_NONE UINT32_C(0) /**< The element has no geometry. */
#define LUMINATE_GEOMETRY_RECT UINT32_C(1) /**< See `luminate_element_geometry_rect`. */
#define LUMINATE_GEOMETRY_POINT UINT32_C(2) /**< See `luminate_element_geometry_point`. */
#define LUMINATE_GEOMETRY_LINEAR UINT32_C(3) /**< See `luminate_element_geometry_linear`. */
#define LUMINATE_GEOMETRY_MATRIX_CELL UINT32_C(4) /**< See `luminate_element_geometry_matrix_cell`. */

/* LuminateGroupKind: how a group was created and is managed. */
#define LUMINATE_GROUP_BUILT_IN UINT32_C(0) /**< Implicit group defined by the daemon. */
#define LUMINATE_GROUP_TOPOLOGY UINT32_C(1) /**< Derived from device topology. */
#define LUMINATE_GROUP_DRIVER UINT32_C(2) /**< Declared by a device driver/plugin. */
#define LUMINATE_GROUP_USER UINT32_C(3) /**< Created by a user. */
#define LUMINATE_GROUP_APPLICATION UINT32_C(4) /**< Created by a client application. */

/* LuminateGroupMemberKind: what a group member entry refers to. */
#define LUMINATE_GROUP_MEMBER_SURFACE UINT32_C(0) /**< See `luminate_group_member_surface_id`. */
#define LUMINATE_GROUP_MEMBER_ELEMENT UINT32_C(1) /**< See `luminate_group_member_surface_id` and `luminate_group_member_element_id`. */
#define LUMINATE_GROUP_MEMBER_GROUP UINT32_C(2) /**< See `luminate_group_member_group_id`. */

/* LuminateCollectionMemberKind: what a collection member entry refers to. */
#define LUMINATE_COLLECTION_MEMBER_TARGET UINT32_C(0) /**< See `luminate_collection_member_target`. */
#define LUMINATE_COLLECTION_MEMBER_COLLECTION UINT32_C(1) /**< See `luminate_collection_member_collection_id`. */

/* LuminateCapabilityScope (brightness): whether brightness can be set independently of colour. */
#define LUMINATE_CAPABILITY_NONE UINT32_C(0) /**< Brightness is not independently controllable. */
#define LUMINATE_CAPABILITY_INDEPENDENT UINT32_C(1) /**< Brightness has its own control; see the `luminate_capability_set_brightness_*` accessors. */

/* LuminateCapabilityScope (general): the granularity at which a capability applies. */
#define LUMINATE_SCOPE_ELEMENT UINT32_C(0) /**< Applies per element. */
#define LUMINATE_SCOPE_SURFACE UINT32_C(1) /**< Applies per surface. */
#define LUMINATE_SCOPE_DEVICE UINT32_C(2) /**< Applies per device. */
#define LUMINATE_SCOPE_CONTROLLER UINT32_C(3) /**< Applies per physical controller (may span devices). */

/* LuminateColourEncoding: how a colour value's channels are interpreted. */
#define LUMINATE_COLOUR_ENCODING_ADDITIVE UINT32_C(0) /**< Additive RGB-family channels. */
#define LUMINATE_COLOUR_ENCODING_HSV UINT32_C(1) /**< Hue/saturation/value. */
#define LUMINATE_COLOUR_ENCODING_HSL UINT32_C(2) /**< Hue/saturation/lightness. */
#define LUMINATE_COLOUR_ENCODING_CCT UINT32_C(3) /**< Correlated colour temperature. */
#define LUMINATE_COLOUR_ENCODING_MONOCHROME UINT32_C(4) /**< A single intensity channel. */

/* LuminateColourChannel: identity of one channel within a colour value. */
#define LUMINATE_COLOUR_CHANNEL_RED UINT32_C(0)
#define LUMINATE_COLOUR_CHANNEL_GREEN UINT32_C(1)
#define LUMINATE_COLOUR_CHANNEL_BLUE UINT32_C(2)
#define LUMINATE_COLOUR_CHANNEL_WHITE UINT32_C(3)
#define LUMINATE_COLOUR_CHANNEL_WARM_WHITE UINT32_C(4)
#define LUMINATE_COLOUR_CHANNEL_COOL_WHITE UINT32_C(5)
#define LUMINATE_COLOUR_CHANNEL_AMBER UINT32_C(6)
#define LUMINATE_COLOUR_CHANNEL_ULTRAVIOLET UINT32_C(7)
#define LUMINATE_COLOUR_CHANNEL_HUE UINT32_C(8) /**< HSV/HSL hue. */
#define LUMINATE_COLOUR_CHANNEL_SATURATION UINT32_C(9) /**< HSV/HSL saturation. */
#define LUMINATE_COLOUR_CHANNEL_VALUE UINT32_C(10) /**< HSV value. */
#define LUMINATE_COLOUR_CHANNEL_LIGHTNESS UINT32_C(11) /**< HSL lightness. */
#define LUMINATE_COLOUR_CHANNEL_TEMPERATURE UINT32_C(12) /**< CCT temperature. */
#define LUMINATE_COLOUR_CHANNEL_INTENSITY UINT32_C(13) /**< Monochrome intensity. */

/* LuminateFrameUpdateMode: which frame upload styles a target accepts. */
#define LUMINATE_FRAME_UPDATE_FULL_ONLY UINT32_C(0) /**< Only `luminate_client_upload_frame_full` is accepted. */
#define LUMINATE_FRAME_UPDATE_PARTIAL UINT32_C(1) /**< Only `luminate_client_upload_frame_partial` is accepted. */
#define LUMINATE_FRAME_UPDATE_BOTH UINT32_C(2) /**< Both full and partial uploads are accepted. */

/* LuminateBufferingMode: how uploaded frames reach the output. */
#define LUMINATE_BUFFERING_IMMEDIATE UINT32_C(0) /**< Each upload takes effect immediately. */
#define LUMINATE_BUFFERING_EXPLICIT_COMMIT UINT32_C(1) /**< Uploads stage until `commit` is set on a frame call. */
#define LUMINATE_BUFFERING_DOUBLE UINT32_C(2) /**< The daemon double-buffers uploads internally. */

/* LuminateShmPixelFormat: fixed-stride pixel encodings for shared-memory frames. */
#define LUMINATE_SHM_PIXEL_FORMAT_RGB8 UINT32_C(0) /**< Red, green, blue; 3 bytes per pixel. */
#define LUMINATE_SHM_PIXEL_FORMAT_RGBW8 UINT32_C(1) /**< Red, green, blue, white; 4 bytes per pixel. */
#define LUMINATE_SHM_PIXEL_FORMAT_MONO8 UINT32_C(2) /**< Monochrome intensity; 1 byte per pixel. */
#define LUMINATE_SHM_PIXEL_FORMAT_RGBX8 UINT32_C(3) /**< Red, green, blue, padding; 4 bytes per pixel. */

/* LuminateShmFrameShapeKind: shared-memory pixel-buffer layout. */
#define LUMINATE_SHM_FRAME_SHAPE_LINEAR UINT32_C(0) /**< A flat plugin-defined pixel order. */
#define LUMINATE_SHM_FRAME_SHAPE_MATRIX UINT32_C(1) /**< A row-major width-by-height matrix. */

/* LuminatePersistenceKind: what a device retains across power cycles. */
#define LUMINATE_PERSISTENCE_NONE UINT32_C(0) /**< Nothing is retained. */
#define LUMINATE_PERSISTENCE_CURRENT_STATE UINT32_C(1) /**< The last-applied state is retained. */
#define LUMINATE_PERSISTENCE_PROFILES UINT32_C(2) /**< Named profile slots are retained; see `luminate_capability_set_persistence_slots`. */

/* LuminatePersistenceRequirement: whether ordinary operation must write persistent storage. */
#define LUMINATE_PERSISTENCE_OPTIONAL UINT32_C(0) /**< The device works fully without ever persisting; an explicit save is opt-in convenience. */
#define LUMINATE_PERSISTENCE_REQUIRED UINT32_C(1) /**< The device always writes state through to non-volatile storage; no explicit save is needed. */

/* LuminateStateReadbackKind: whether the daemon can read state back from the device. */
#define LUMINATE_STATE_READBACK_NONE UINT32_C(0) /**< State cannot be read back. */
#define LUMINATE_STATE_READBACK_READABLE UINT32_C(1) /**< One or more facets can be read back; see `luminate_capability_set_readable_facet_at`. */

/* LuminateReadbackFidelity: how much to trust a readback value. */
#define LUMINATE_READBACK_BEST_EFFORT UINT32_C(0) /**< The value may be approximate or stale. */
#define LUMINATE_READBACK_EXACT UINT32_C(1) /**< The value exactly reflects hardware state. */

/* LuminateEffectParameterKind: which variant of `LuminateEffectParameter` is populated. */
#define LUMINATE_EFFECT_PARAMETER_COLOUR UINT32_C(0) /**< See `luminate_effect_parameter_colour_count_range`. */
#define LUMINATE_EFFECT_PARAMETER_SPEED UINT32_C(1) /**< See `luminate_effect_parameter_speed_range`. */
#define LUMINATE_EFFECT_PARAMETER_DIRECTION UINT32_C(2) /**< See `luminate_effect_parameter_direction_count`/`_at`. */
#define LUMINATE_EFFECT_PARAMETER_DURATION UINT32_C(3) /**< See `luminate_effect_parameter_duration_range`. */
#define LUMINATE_EFFECT_PARAMETER_BRIGHTNESS UINT32_C(4) /**< See `luminate_effect_parameter_brightness_bits`. */
#define LUMINATE_EFFECT_PARAMETER_CHOICE UINT32_C(5) /**< See `luminate_effect_parameter_choice_count`/`_at`. */

/* LuminateEffectDirection: a motion direction a hardware effect can run in. */
#define LUMINATE_DIRECTION_FORWARD UINT32_C(0)
#define LUMINATE_DIRECTION_REVERSE UINT32_C(1)
#define LUMINATE_DIRECTION_CLOCKWISE UINT32_C(2)
#define LUMINATE_DIRECTION_COUNTER_CLOCKWISE UINT32_C(3)
#define LUMINATE_DIRECTION_INWARD UINT32_C(4)
#define LUMINATE_DIRECTION_OUTWARD UINT32_C(5)
#define LUMINATE_DIRECTION_RANDOM UINT32_C(6)

/* LuminatePowerDomainKind: what a power domain reference addresses. */
#define LUMINATE_POWER_DOMAIN_DEVICE UINT32_C(0) /**< The whole device shares one power domain. */
#define LUMINATE_POWER_DOMAIN_SURFACE UINT32_C(1) /**< A specific surface has its own power domain; see `luminate_power_domain_surface_id`. */

/* LuminateStateFacetKind: an observable/adoptable facet of device state. */
#define LUMINATE_FACET_APPEARANCE UINT32_C(0) /**< Colour or running effect. */
#define LUMINATE_FACET_BRIGHTNESS UINT32_C(1)
#define LUMINATE_FACET_EMISSION UINT32_C(2) /**< Whether light is currently being emitted. */
#define LUMINATE_FACET_PHYSICAL_POWER UINT32_C(3)
#define LUMINATE_FACET_EFFECTIVE_APPEARANCE UINT32_C(4) /**< Daemon-synthesized; see `LUMINATE_EFFECTIVE_APPEARANCE_*`. */
#define LUMINATE_FACET_APPEARANCE_SLOTS UINT32_C(5) /**< Known firmware-stored appearance programs. */

/* LuminateAppearanceSlotUpdatePolicy: completeness rule for one logical mutation. */
#define LUMINATE_APPEARANCE_SLOT_UPDATE_INDEPENDENT UINT32_C(0)
#define LUMINATE_APPEARANCE_SLOT_UPDATE_PARTIAL_IF_KNOWN UINT32_C(1)
#define LUMINATE_APPEARANCE_SLOT_UPDATE_COMPLETE_SET UINT32_C(2)

/* LuminateReachability: whether a device currently responds. */
#define LUMINATE_REACHABILITY_UNKNOWN UINT32_C(0)
#define LUMINATE_REACHABILITY_REACHABLE UINT32_C(1)
#define LUMINATE_REACHABILITY_UNAVAILABLE UINT32_C(2)

/* LuminateReconciliationStatus: progress of reconciling desired state with hardware. */
#define LUMINATE_RECONCILIATION_IDLE UINT32_C(0) /**< No reconciliation in progress or needed. */
#define LUMINATE_RECONCILIATION_RECONCILING UINT32_C(1) /**< Actively applying state to hardware. */
#define LUMINATE_RECONCILIATION_COMPLETE UINT32_C(2) /**< Hardware matches desired state. */
#define LUMINATE_RECONCILIATION_DRIFTED UINT32_C(3) /**< Hardware state has diverged from what was last applied. */
#define LUMINATE_RECONCILIATION_FAILED UINT32_C(4) /**< The last reconciliation attempt failed. */

/* LuminateObservationConfidence: how strongly an observation is supported. */
#define LUMINATE_CONFIDENCE_ASSUMED UINT32_C(0) /**< Projected from a successful write but not read back. */
#define LUMINATE_CONFIDENCE_BEST_EFFORT UINT32_C(1) /**< Readback or derivation whose fidelity is insufficient to treat as exact. */
#define LUMINATE_CONFIDENCE_CONFIRMED UINT32_C(2) /**< Exact hardware readback, or a sound derivation from confirmed inputs. */

/* LuminateObservationSource: where an observation's value came from. */
#define LUMINATE_SOURCE_SUCCESSFUL_APPLY UINT32_C(0) /**< Recorded after a mutation succeeded. */
#define LUMINATE_SOURCE_READBACK UINT32_C(1) /**< Read directly from hardware. */
#define LUMINATE_SOURCE_DERIVED UINT32_C(2) /**< Computed from other known state. */
#define LUMINATE_SOURCE_ADOPTED_BASELINE UINT32_C(3) /**< Taken from an adopted pre-existing baseline. */

/* LuminateAdoptionStatus: whether a facet's pre-existing hardware state was adopted as the baseline. */
#define LUMINATE_ADOPTION_NOT_APPLICABLE UINT32_C(0) /**< Adoption does not apply to this facet. */
#define LUMINATE_ADOPTION_PENDING UINT32_C(1) /**< Eligible adoption has not yet become durable. */
#define LUMINATE_ADOPTION_DURABLE UINT32_C(2) /**< Adopted and persisted. */
#define LUMINATE_ADOPTION_INELIGIBLE_FIDELITY UINT32_C(3) /**< Readback fidelity was too low to adopt. */
#define LUMINATE_ADOPTION_PERSISTENCE_FAILED UINT32_C(4) /**< Adoption was attempted but persisting it failed. */

/* LuminateTargetKind: which addressing level a `LuminateTargetView` names. */
#define LUMINATE_TARGET_DEVICE UINT32_C(0)
#define LUMINATE_TARGET_SURFACE UINT32_C(1)
#define LUMINATE_TARGET_ELEMENT UINT32_C(2)
#define LUMINATE_TARGET_GROUP UINT32_C(3)

/* LuminateAppearanceKind: which variant of a facet's appearance value is populated. */
#define LUMINATE_APPEARANCE_STATIC UINT32_C(0) /**< See `luminate_facet_value_colour`. */
#define LUMINATE_APPEARANCE_EFFECT UINT32_C(1) /**< See `luminate_facet_value_effect`. */
#define LUMINATE_APPEARANCE_MIXED UINT32_C(2) /**< Constituents have different configured appearances. */

/* LuminateEffectiveAppearanceKind: which variant of a facet's effective-appearance value is populated. */
#define LUMINATE_EFFECTIVE_APPEARANCE_OFF UINT32_C(0) /**< The target is not currently emitting light. */
#define LUMINATE_EFFECTIVE_APPEARANCE_STATIC UINT32_C(1) /**< See `luminate_facet_value_colour`. */
#define LUMINATE_EFFECTIVE_APPEARANCE_EFFECT UINT32_C(2) /**< See `luminate_facet_value_effect`. */
#define LUMINATE_EFFECTIVE_APPEARANCE_STREAMING UINT32_C(3) /**< A client is actively pushing raw frames to this target; no colour or effect is populated. */
#define LUMINATE_EFFECTIVE_APPEARANCE_MIXED UINT32_C(4) /**< Constituents have different effective appearances. */

/* LuminateEmissionState: whether a target is currently emitting light. */
#define LUMINATE_EMISSION_DARK UINT32_C(0)
#define LUMINATE_EMISSION_EMITTING UINT32_C(1)

/* LuminatePhysicalPowerState: whether a target's physical power is on. */
#define LUMINATE_PHYSICAL_POWER_OFF UINT32_C(0)
#define LUMINATE_PHYSICAL_POWER_ON UINT32_C(1)

/* LuminateEffectKind: which effect variant a `LuminateEffect` holds. */
#define LUMINATE_EFFECT_OFF UINT32_C(0) /**< No effect; target is dark. */
#define LUMINATE_EFFECT_STATIC UINT32_C(1) /**< A fixed colour, no animation. */
#define LUMINATE_EFFECT_BREATHE UINT32_C(2) /**< Smooth fade in/out of one colour. */
#define LUMINATE_EFFECT_PULSE UINT32_C(3) /**< Sharp pulse of one colour. */
#define LUMINATE_EFFECT_SCANNER UINT32_C(4) /**< A moving point of light (Larson scanner style). */
#define LUMINATE_EFFECT_MORPH UINT32_C(5) /**< Cycles/blends through a list of colours. */
#define LUMINATE_EFFECT_SPECTRUM UINT32_C(6) /**< Cycles through the full colour spectrum. */
#define LUMINATE_EFFECT_RAINBOW UINT32_C(7) /**< A static or scrolling rainbow gradient. */
#define LUMINATE_EFFECT_HARDWARE UINT32_C(8) /**< A device-native effect selected by id; see `luminate_effect_create_hardware`. */
#define LUMINATE_EFFECT_STROBE UINT32_C(9) /**< Rapid on/off flashing of one colour. */

/* LuminateEventKind: which payload a `LuminateEvent` carries. */
#define LUMINATE_EVENT_TOPOLOGY_CHANGED UINT32_C(0) /**< Devices were added/removed/changed; see `luminate_event_topology_device_at`. */
#define LUMINATE_EVENT_STATE_CHANGED UINT32_C(1) /**< One or more devices' state changed; see `luminate_event_state_device_at`. */
#define LUMINATE_EVENT_SHM_STREAM_ENDED UINT32_C(2) /**< A client-published shared-memory frame stream ended on the daemon side; see `luminate_event_shm_stream_target`. */
#define LUMINATE_EVENT_CONFIGURATION_CHANGED UINT32_C(3) /**< Managed configuration changed; see `luminate_event_configuration_changes`. */
#define LUMINATE_EVENT_SCENES_CHANGED UINT32_C(4) /**< The persistent scene registry changed. */
#define LUMINATE_EVENT_TRANSITIONS_CHANGED UINT32_C(5) /**< One or more daemon-managed transition statuses changed. */
#define LUMINATE_EVENT_RESYNC_REQUIRED UINT32_C(6) /**< Events were lost; fetch every authoritative baseline again. */

#define LUMINATE_TRANSITION_FUNCTION_LINEAR UINT32_C(0)
#define LUMINATE_TRANSITION_FUNCTION_EASE_IN UINT32_C(1)
#define LUMINATE_TRANSITION_FUNCTION_EASE_OUT UINT32_C(2)
#define LUMINATE_TRANSITION_FUNCTION_EASE_IN_OUT UINT32_C(3)

#define LUMINATE_TRANSITION_COLOUR_ENCODED UINT32_C(0)
#define LUMINATE_TRANSITION_COLOUR_OKLAB UINT32_C(1)

#define LUMINATE_HUE_DIRECTION_SHORTEST UINT32_C(0)
#define LUMINATE_HUE_DIRECTION_INCREASING UINT32_C(1)
#define LUMINATE_HUE_DIRECTION_DECREASING UINT32_C(2)

/* LuminatePolicyOperation: a semantic operation evaluated by an access policy. */
#define LUMINATE_POLICY_OP_OBSERVE UINT32_C(0) /**< Observe topology, state, or events. */
#define LUMINATE_POLICY_OP_REFRESH UINT32_C(1) /**< Refresh state from hardware. */
#define LUMINATE_POLICY_OP_CONTROL UINT32_C(2) /**< Control device state. */
#define LUMINATE_POLICY_OP_HARDWARE_ADMINISTRATION UINT32_C(3) /**< Change hardware-level configuration. */
#define LUMINATE_POLICY_OP_DAEMON_ADMINISTRATION UINT32_C(4) /**< Administer the daemon. */
#define LUMINATE_POLICY_OP_MANAGE_PLUGINS UINT32_C(5) /**< Manage plugins and their configuration. */
#define LUMINATE_POLICY_OP_CREATE_COLLECTION UINT32_C(6) /**< Create a collection. */
#define LUMINATE_POLICY_OP_DESTROY_COLLECTION UINT32_C(7) /**< Destroy a collection. */
#define LUMINATE_POLICY_OP_MODIFY_COLLECTION UINT32_C(8) /**< Change collection membership. */
#define LUMINATE_POLICY_OP_ADMINISTER_COLLECTIONS UINT32_C(9) /**< Administer a collection without owning it. */
#define LUMINATE_POLICY_OP_MANAGE_POLICY UINT32_C(10) /**< Read or replace the remote access policy. */
#define LUMINATE_POLICY_OP_MANAGE_AUTHENTICATION UINT32_C(11) /**< Manage front-end authentication records. */
#define LUMINATE_POLICY_OP_ADMINISTER_FRONTEND UINT32_C(12) /**< Administer front-end operations for non-daemon policy checks. */
#define LUMINATE_POLICY_OP_CREATE_SCENE UINT32_C(13) /**< Create a persistent scene. */
#define LUMINATE_POLICY_OP_MODIFY_SCENE UINT32_C(14) /**< Replace or recapture a scene. */
#define LUMINATE_POLICY_OP_DESTROY_SCENE UINT32_C(15) /**< Delete a scene. */
#define LUMINATE_POLICY_OP_ADMINISTER_SCENES UINT32_C(16) /**< Administer a scene without owning it. */

/* LuminateRuleEffect: whether a matching policy rule allows or denies. */
#define LUMINATE_RULE_EFFECT_ALLOW UINT32_C(0)
#define LUMINATE_RULE_EFFECT_DENY UINT32_C(1)

/* LuminatePolicyCallMode: whether a policy provider's callbacks return synchronously or complete later via a completion token. */
#define LUMINATE_POLICY_CALL_MODE_SYNC UINT32_C(0)
#define LUMINATE_POLICY_CALL_MODE_ASYNC UINT32_C(1)

/** Releases a `LuminateTopologySnapshot` returned by `luminate_client_list_devices` or a baseline subscribe call. Null is a no-op. */
LUMINATE_API void luminate_topology_snapshot_free(LuminateTopologySnapshot *value);
/** Releases a `LuminateWithdrawnDeviceList` returned by `luminate_client_list_withdrawn_devices`. Null is a no-op. */
LUMINATE_API void luminate_withdrawn_device_list_free(LuminateWithdrawnDeviceList *value);
/** Releases a `LuminateDeviceSnapshot` returned by `luminate_client_get_device`. Null is a no-op. */
LUMINATE_API void luminate_device_snapshot_free(LuminateDeviceSnapshot *value);
/** Releases a `LuminateStateSnapshot` returned by `luminate_client_get_state`. Null is a no-op. */
LUMINATE_API void luminate_state_snapshot_free(LuminateStateSnapshot *value);
LUMINATE_API void luminate_collection_state_snapshot_free(LuminateCollectionStateSnapshot *value);
/** Releases a `LuminateEvent` returned by `luminate_event_subscription_next`. Null is a no-op. */
LUMINATE_API void luminate_event_free(LuminateEvent *value);
/** Releases a `LuminateCollectionList` returned by `luminate_client_list_collections`. Null is a no-op. */
LUMINATE_API void luminate_collection_list_free(LuminateCollectionList *value);
/** Releases a `LuminateCollectionSnapshot` returned by `luminate_client_get_collection`. Null is a no-op. */
LUMINATE_API void luminate_collection_snapshot_free(LuminateCollectionSnapshot *value);
/** Releases a scene list. Null is a no-op. */
LUMINATE_API void luminate_scene_list_free(LuminateSceneList *value);
/** Releases a scene snapshot. Null is a no-op. */
LUMINATE_API void luminate_scene_snapshot_free(LuminateSceneSnapshot *value);
/** Releases a `LuminateAllOffPlan` returned by `luminate_all_off_plan`. Null is a no-op. */
LUMINATE_API void luminate_all_off_plan_free(LuminateAllOffPlan *value);
/** The collection's stable, server-generated identifier. */
LUMINATE_API LuminateStringView luminate_collection_id(const LuminateCollection *value);
/** The collection's human-readable display name. */
LUMINATE_API LuminateStringView luminate_collection_name(const LuminateCollection *value);
/** Number of members explicitly listed in the collection. */
LUMINATE_API uintptr_t luminate_collection_member_count(const LuminateCollection *value);
/** Borrowed member at `index`, or null if out of range. */
LUMINATE_API const LuminateCollectionMember *luminate_collection_member_at(const LuminateCollection *value, uintptr_t index);
/** The scene's stable identifier. */
LUMINATE_API LuminateStringView luminate_scene_id(const LuminateScene *value);
/** The scene's human-readable name. */
LUMINATE_API LuminateStringView luminate_scene_name(const LuminateScene *value);
/** Number of ordered bindings in the scene. */
LUMINATE_API uintptr_t luminate_scene_binding_count(const LuminateScene *value);
/** Borrowed binding at `index`, or null if out of range. */
LUMINATE_API const LuminateSceneBinding *luminate_scene_binding_at(const LuminateScene *value, uintptr_t index);

/** Number of surfaces on a device. */
LUMINATE_API uintptr_t luminate_device_surface_count(const LuminateDevice *value);
/** Borrowed surface at `index`, or null if out of range. */
LUMINATE_API const LuminateSurface *luminate_device_surface_at(const LuminateDevice *value, uintptr_t index);
/** Number of groups a device belongs to. */
LUMINATE_API uintptr_t luminate_device_group_count(const LuminateDevice *value);
/** Borrowed group at `index`, or null if out of range. */
LUMINATE_API const LuminateGroup *luminate_device_group_at(const LuminateDevice *value, uintptr_t index);
/** The device's stable identifier. */
LUMINATE_API LuminateStringView luminate_device_id(const LuminateDevice *value);
/** The device's human-readable display name. */
LUMINATE_API LuminateStringView luminate_device_name(const LuminateDevice *value);
/** Number of physical-form tags attached to the device. */
LUMINATE_API uintptr_t luminate_device_physical_tag_count(const LuminateDevice *value);
/** Physical-form tag at `index`, or an absent view if out of range. */
LUMINATE_API LuminateStringView luminate_device_physical_tag_at(const LuminateDevice *value, uintptr_t index);
/** Number of informational notes attached to the device. */
LUMINATE_API uintptr_t luminate_device_note_count(const LuminateDevice *value);
/** Note text at `index`, or an empty view if out of range. */
LUMINATE_API LuminateStringView luminate_device_note_at(const LuminateDevice *value, uintptr_t index);
/** Number of warnings attached to the device. */
LUMINATE_API uintptr_t luminate_device_warning_count(const LuminateDevice *value);
/** Warning text at `index`, or an empty view if out of range. */
LUMINATE_API LuminateStringView luminate_device_warning_at(const LuminateDevice *value, uintptr_t index);

/** The surface's identifier, unique within its owning device. */
LUMINATE_API LuminateStringView luminate_surface_id(const LuminateSurface *value);
/** The surface's human-readable display name. */
LUMINATE_API LuminateStringView luminate_surface_name(const LuminateSurface *value);
/** Number of elements on this surface. */
LUMINATE_API uintptr_t luminate_surface_element_count(const LuminateSurface *value);
/** Borrowed element at `index`, or null if out of range. */
LUMINATE_API const LuminateElement *luminate_surface_element_at(const LuminateSurface *value, uintptr_t index);
/** Number of physical-form tags attached to the surface. */
LUMINATE_API uintptr_t luminate_surface_physical_tag_count(const LuminateSurface *value);
/** Physical-form tag at `index`, or an absent view if out of range. */
LUMINATE_API LuminateStringView luminate_surface_physical_tag_at(const LuminateSurface *value, uintptr_t index);
/** Number of informational notes attached to the surface. */
LUMINATE_API uintptr_t luminate_surface_note_count(const LuminateSurface *value);
/** Note text at `index`, or an empty view if out of range. */
LUMINATE_API LuminateStringView luminate_surface_note_at(const LuminateSurface *value, uintptr_t index);
/** Number of warnings attached to the surface. */
LUMINATE_API uintptr_t luminate_surface_warning_count(const LuminateSurface *value);
/** Warning text at `index`, or an empty view if out of range. */
LUMINATE_API LuminateStringView luminate_surface_warning_at(const LuminateSurface *value, uintptr_t index);

/** The element's identifier, unique within its owning surface. */
LUMINATE_API LuminateStringView luminate_element_id(const LuminateElement *value);
/** Number of physical-form tags attached to the element. */
LUMINATE_API uintptr_t luminate_element_physical_tag_count(const LuminateElement *value);
/** Physical-form tag at `index`, or an absent view if out of range. */
LUMINATE_API LuminateStringView luminate_element_physical_tag_at(const LuminateElement *value, uintptr_t index);
/** Number of informational notes attached to the element. */
LUMINATE_API uintptr_t luminate_element_note_count(const LuminateElement *value);
/** Note text at `index`, or an empty view if out of range. */
LUMINATE_API LuminateStringView luminate_element_note_at(const LuminateElement *value, uintptr_t index);
/** Number of warnings attached to the element. */
LUMINATE_API uintptr_t luminate_element_warning_count(const LuminateElement *value);
/** Warning text at `index`, or an empty view if out of range. */
LUMINATE_API LuminateStringView luminate_element_warning_at(const LuminateElement *value, uintptr_t index);

/** The group's identifier, unique within its owning device. */
LUMINATE_API LuminateStringView luminate_group_id(const LuminateGroup *value);
/** The group's human-readable display name. */
LUMINATE_API LuminateStringView luminate_group_name(const LuminateGroup *value);
/** Number of members in the group. */
LUMINATE_API uintptr_t luminate_group_member_count(const LuminateGroup *value);
/** Borrowed member at `index`, or null if out of range. */
LUMINATE_API const LuminateGroupMember *luminate_group_member_at(const LuminateGroup *value, uintptr_t index);
/** Number of informational notes attached to the group. */
LUMINATE_API uintptr_t luminate_group_note_count(const LuminateGroup *value);
/** Note text at `index`, or an empty view if out of range. */
LUMINATE_API LuminateStringView luminate_group_note_at(const LuminateGroup *value, uintptr_t index);
/** Number of warnings attached to the group. */
LUMINATE_API uintptr_t luminate_group_warning_count(const LuminateGroup *value);
/** Warning text at `index`, or an empty view if out of range. */
LUMINATE_API LuminateStringView luminate_group_warning_at(const LuminateGroup *value, uintptr_t index);

/** Number of colour capabilities advertised. */
LUMINATE_API uintptr_t luminate_capability_set_colour_count(const LuminateCapabilitySet *value);
/** Borrowed colour capability at `index`, or null if out of range. */
LUMINATE_API const LuminateColourCapability *luminate_capability_set_colour_at(const LuminateCapabilitySet *value, uintptr_t index);
/** Number of channels within a colour capability. */
LUMINATE_API uintptr_t luminate_colour_capability_channel_count(const LuminateColourCapability *value);
/** Borrowed channel capability at `index`, or null if out of range. */
LUMINATE_API const LuminateColourChannelCapability *luminate_colour_capability_channel_at(const LuminateColourCapability *value, uintptr_t index);
/** Number of hardware effects advertised by a hardware-effects capability. */
LUMINATE_API uintptr_t luminate_hardware_effects_effect_count(const LuminateHardwareEffectsCapability *value);
/** Borrowed hardware effect descriptor at `index`, or null if out of range. */
LUMINATE_API const LuminateHardwareEffectDescriptor *luminate_hardware_effects_effect_at(const LuminateHardwareEffectsCapability *value, uintptr_t index);

/** Number of appearance slots advertised. */
LUMINATE_API uintptr_t luminate_appearance_slot_count(const LuminateAppearanceSlotsCapability *value);
/** Borrowed appearance-slot descriptor at `index`, or null. */
LUMINATE_API const LuminateAppearanceSlotDescriptor *luminate_appearance_slot_at(const LuminateAppearanceSlotsCapability *value, uintptr_t index);
/** Stable identifier of an appearance slot. */
LUMINATE_API LuminateStringView luminate_appearance_slot_id(const LuminateAppearanceSlotDescriptor *value);
/** Human-readable name of an appearance slot. */
LUMINATE_API LuminateStringView luminate_appearance_slot_name(const LuminateAppearanceSlotDescriptor *value);
/** Number of informational notes attached to an appearance slot. */
LUMINATE_API uintptr_t luminate_appearance_slot_note_count(const LuminateAppearanceSlotDescriptor *value);
/** Informational note at `index`, or an empty view. */
LUMINATE_API LuminateStringView luminate_appearance_slot_note_at(const LuminateAppearanceSlotDescriptor *value, uintptr_t index);
/** Number of warnings attached to an appearance slot. */
LUMINATE_API uintptr_t luminate_appearance_slot_warning_count(const LuminateAppearanceSlotDescriptor *value);
/** Warning at `index`, or an empty view. */
LUMINATE_API LuminateStringView luminate_appearance_slot_warning_at(const LuminateAppearanceSlotDescriptor *value, uintptr_t index);
/** Number of colour models accepted by a slot appearance. */
LUMINATE_API uintptr_t luminate_appearance_capability_colour_count(const LuminateAppearanceCapability *value);
/** Borrowed colour capability at `index`, or null. */
LUMINATE_API const LuminateColourCapability *luminate_appearance_capability_colour_at(const LuminateAppearanceCapability *value, uintptr_t index);
/** The hardware effect's stable identifier, passed to `luminate_effect_create_hardware`. */
LUMINATE_API LuminateStringView luminate_hardware_effect_descriptor_id(const LuminateHardwareEffectDescriptor *value);
/** The hardware effect's human-readable display name. */
LUMINATE_API LuminateStringView luminate_hardware_effect_descriptor_name(const LuminateHardwareEffectDescriptor *value);
/** Number of configurable parameters the hardware effect exposes. */
LUMINATE_API uintptr_t luminate_hardware_effect_descriptor_parameter_count(const LuminateHardwareEffectDescriptor *value);
/** Borrowed parameter descriptor at `index`, or null if out of range. */
LUMINATE_API const LuminateEffectParameter *luminate_hardware_effect_descriptor_parameter_at(const LuminateHardwareEffectDescriptor *value, uintptr_t index);

/** The device id this state snapshot belongs to. */
LUMINATE_API LuminateStringView luminate_state_device_id(const LuminateState *value);
/** Number of facet observations recorded for the device. */
LUMINATE_API uintptr_t luminate_state_observation_count(const LuminateState *value);
/** Borrowed observation at `index`, or null if out of range. */
LUMINATE_API const LuminateFacetObservation *luminate_state_observation_at(const LuminateState *value, uintptr_t index);
/** Number of facet adoption records for the device. */
LUMINATE_API uintptr_t luminate_state_adoption_count(const LuminateState *value);
/** Borrowed adoption record at `index`, or null if out of range. */
LUMINATE_API const LuminateAdoption *luminate_state_adoption_at(const LuminateState *value, uintptr_t index);
/** Number of channel values in a colour. */
LUMINATE_API uintptr_t luminate_colour_channel_count(const LuminateColour *value);
/**
 * Copies the channel and raw value at `index` into caller-owned outputs.
 *
 * Returns false for a null colour, null output, or out-of-range index. Both
 * outputs remain unchanged on failure.
 */
LUMINATE_API bool luminate_colour_channel_at(const LuminateColour *value, uintptr_t index, LuminateColourChannel *out_channel, uint32_t *out_value);

/** Which effect variant this owned effect holds, or `LUMINATE_DISCRIMINANT_INVALID`. */
LUMINATE_API LuminateEffectKind luminate_effect_kind(const LuminateEffect *effect);
/** Copies an RGB-only animated effect's colour into `out_colour`. */
LUMINATE_API bool luminate_effect_rgb(const LuminateEffect *effect, LuminateRgb *out_colour);
/** Borrowed generic colour, or null unless this is a static effect. */
LUMINATE_API const LuminateColour *luminate_effect_static_colour(const LuminateEffect *effect);
/** Copies an animated effect's period into `out_period_ms`. */
LUMINATE_API bool luminate_effect_period_ms(const LuminateEffect *effect, uint32_t *out_period_ms);
/** Number of RGB colours in a morph or hardware effect. */
LUMINATE_API uintptr_t luminate_effect_rgb_count(const LuminateEffect *effect);
/** Copies the RGB colour at `index` into `out_colour`. */
LUMINATE_API bool luminate_effect_rgb_at(const LuminateEffect *effect, uintptr_t index, LuminateRgb *out_colour);
/** Hardware effect identifier, or an absent view. */
LUMINATE_API LuminateStringView luminate_effect_hardware_id(const LuminateEffect *effect);
/** Copies the optional hardware speed into `out_speed`. */
LUMINATE_API bool luminate_effect_speed(const LuminateEffect *effect, uint16_t *out_speed);
/** Copies the optional hardware duration into `out_duration_ms`. */
LUMINATE_API bool luminate_effect_duration_ms(const LuminateEffect *effect, uint32_t *out_duration_ms);
/** Copies the optional hardware brightness into `out_brightness`. */
LUMINATE_API bool luminate_effect_brightness(const LuminateEffect *effect, uint32_t *out_brightness);
/** Optional hardware direction, or `LUMINATE_DISCRIMINANT_INVALID`. */
LUMINATE_API LuminateEffectDirection luminate_effect_direction(const LuminateEffect *effect);
/** Optional hardware choice, or an absent view. */
LUMINATE_API LuminateStringView luminate_effect_choice(const LuminateEffect *effect);

/** Which effect variant this borrowed view holds, or `LUMINATE_DISCRIMINANT_INVALID`. */
LUMINATE_API LuminateEffectKind luminate_effect_view_kind(const LuminateEffectView *effect);
/** Copies an RGB-only animated effect view's colour into `out_colour`. */
LUMINATE_API bool luminate_effect_view_rgb(const LuminateEffectView *effect, LuminateRgb *out_colour);
/** Borrowed generic colour, or null unless this is a static effect view. */
LUMINATE_API const LuminateColour *luminate_effect_view_static_colour(const LuminateEffectView *effect);
/** Copies an animated effect view's period into `out_period_ms`. */
LUMINATE_API bool luminate_effect_view_period_ms(const LuminateEffectView *effect, uint32_t *out_period_ms);
/** Number of RGB colours in a morph or hardware effect view. */
LUMINATE_API uintptr_t luminate_effect_view_rgb_count(const LuminateEffectView *effect);
/** Copies the RGB colour at `index` from an effect view into `out_colour`. */
LUMINATE_API bool luminate_effect_view_rgb_at(const LuminateEffectView *effect, uintptr_t index, LuminateRgb *out_colour);
/** Hardware effect-view identifier, or an absent view. */
LUMINATE_API LuminateStringView luminate_effect_view_hardware_id(const LuminateEffectView *effect);
/** Copies the optional hardware effect-view speed into `out_speed`. */
LUMINATE_API bool luminate_effect_view_speed(const LuminateEffectView *effect, uint16_t *out_speed);
/** Copies the optional hardware effect-view duration into `out_duration_ms`. */
LUMINATE_API bool luminate_effect_view_duration_ms(const LuminateEffectView *effect, uint32_t *out_duration_ms);
/** Copies the optional hardware effect-view brightness into `out_brightness`. */
LUMINATE_API bool luminate_effect_view_brightness(const LuminateEffectView *effect, uint32_t *out_brightness);
/** Optional hardware effect-view direction, or `LUMINATE_DISCRIMINANT_INVALID`. */
LUMINATE_API LuminateEffectDirection luminate_effect_view_direction(const LuminateEffectView *effect);
/** Optional hardware effect-view choice, or an absent view. */
LUMINATE_API LuminateStringView luminate_effect_view_choice(const LuminateEffectView *effect);
/** Creates an owned breathe effect. Release with `luminate_effect_free`. */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_create_breathe(LuminateRgb colour, uint32_t period_ms, LuminateEffect **out_effect);
/** Creates an owned pulse effect. Release with `luminate_effect_free`. */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_create_pulse(LuminateRgb colour, uint32_t period_ms, LuminateEffect **out_effect);
/** Creates an owned strobe effect. Release with `luminate_effect_free`. */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_create_strobe(LuminateRgb colour, uint32_t period_ms, LuminateEffect **out_effect);
/** Creates an owned scanner effect. Release with `luminate_effect_free`. */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_create_scanner(LuminateRgb colour, uint32_t period_ms, LuminateEffect **out_effect);
/** Sets the speed parameter on a hardware effect created by `luminate_effect_create_hardware`. */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_hardware_set_speed(LuminateEffect *effect, uint16_t speed);
/** Sets the duration-in-milliseconds parameter on a hardware effect created by `luminate_effect_create_hardware`. */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_hardware_set_duration_ms(LuminateEffect *effect, uint32_t duration_ms);
/** Sets the brightness parameter on a hardware effect created by `luminate_effect_create_hardware`. */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_effect_hardware_set_brightness(LuminateEffect *effect, uint32_t brightness);
/** The choice option's stable identifier. */
LUMINATE_API LuminateStringView luminate_effect_choice_id(const LuminateEffectChoice *value);
/** The choice option's human-readable display name. */
LUMINATE_API LuminateStringView luminate_effect_choice_name(const LuminateEffectChoice *value);

/**
 * Sets brightness on the given target, in the raw units of its advertised
 * brightness capability (0 to `luminate_capability_set_brightness_maximum`,
 * not a 0-100 percentage).
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_set_brightness(LuminateClient *client, const LuminateTarget *target, uint32_t value);
/** Clears any desired-state override for the given target, returning it to its default. */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_clear_target(LuminateClient *client, const LuminateTarget *target);
/** Turns off (dark) the given target. */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_set_off(LuminateClient *client, const LuminateTarget *target);
/** Reapplies the given target's last-known configured appearance. */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_restore_appearance(LuminateClient *client, const LuminateTarget *target);
/** Sets whether the given target emits light. */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_set_emission(LuminateClient *client, const LuminateTarget *target, LuminateEmissionState state);

/** Adds one member to a collection's explicit membership. A no-op if `member` is already present. */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_add_collection_member(LuminateClient *client, const char *id, const LuminateCollectionMemberInput *member);
/** Removes one member from a collection's explicit membership. A no-op if `member` is absent. */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_remove_collection_member(LuminateClient *client, const char *id, const LuminateCollectionMemberInput *member);

/*
 * `null_safe_free!`-generated release functions for the multi-principal
 * authorization façade's opaque handles (`policy.rs`, `multi_user/`,
 * `streams.rs`, `policy_provider.rs`, `ownership_store.rs`): invisible to
 * cbindgen for the same reason as the `luminate_topology_snapshot_free` and
 * friends above, hand-declared the same way.
 */
LUMINATE_API void luminate_policy_document_builder_free(LuminatePolicyDocumentBuilder *value);
LUMINATE_API void luminate_scene_builder_free(LuminateSceneBuilder *value);
LUMINATE_API void luminate_policy_document_free(LuminatePolicyDocument *value);
LUMINATE_API void luminate_remote_principal_free(LuminateRemotePrincipal *value);
LUMINATE_API void luminate_authorization_evaluation_free(LuminateAuthorizationEvaluation *value);
LUMINATE_API void luminate_collection_outcome_free(LuminateCollectionOutcome *value);
LUMINATE_API void luminate_attestation_list_free(LuminateAttestationList *value);
LUMINATE_API void luminate_created_attestation_free(LuminateCreatedAttestation *value);
LUMINATE_API void luminate_token_list_free(LuminateTokenList *value);
LUMINATE_API void luminate_created_token_free(LuminateCreatedToken *value);
LUMINATE_API void luminate_management_snapshot_free(LuminateManagementSnapshot *value);
LUMINATE_API void luminate_management_change_set_free(LuminateManagementChangeSet *value);
LUMINATE_API void luminate_management_patch_builder_free(LuminateManagementPatchBuilder *value);
LUMINATE_API void luminate_setting_value_free(LuminateSettingValue *value);
LUMINATE_API LUMINATE_NODISCARD LuminateStatus luminate_setting_value_new_boolean(bool value, LuminateSettingValue **out_value);
LUMINATE_API LUMINATE_NODISCARD LuminateStatus luminate_setting_value_new_integer(int64_t value, LuminateSettingValue **out_value);

/*
 * Management accessors generated by local accessor macros.
 */
LUMINATE_API bool luminate_daemon_preferences_has_default_unsupported_policy(const LuminateDaemonPreferences *value);
LUMINATE_API LuminateUnsupportedPolicy luminate_daemon_preferences_default_unsupported_policy(const LuminateDaemonPreferences *value);
LUMINATE_API bool luminate_daemon_preferences_has_reconciliation_policy(const LuminateDaemonPreferences *value);
LUMINATE_API LuminateReconciliationPolicy luminate_daemon_preferences_reconciliation_policy(const LuminateDaemonPreferences *value);
LUMINATE_API bool luminate_daemon_preferences_has_cct_emulation(const LuminateDaemonPreferences *value);
LUMINATE_API LuminateCctEmulation luminate_daemon_preferences_cct_emulation(const LuminateDaemonPreferences *value);
LUMINATE_API bool luminate_daemon_preferences_has_prefer_shm(const LuminateDaemonPreferences *value);
LUMINATE_API bool luminate_daemon_preferences_prefer_shm(const LuminateDaemonPreferences *value);
LUMINATE_API bool luminate_daemon_preferences_has_prefer_client_shm(const LuminateDaemonPreferences *value);
LUMINATE_API bool luminate_daemon_preferences_prefer_client_shm(const LuminateDaemonPreferences *value);
LUMINATE_API LuminateStringView luminate_managed_plugin_name(const LuminateManagedPlugin *value);
LUMINATE_API LuminateStringView luminate_managed_plugin_version(const LuminateManagedPlugin *value);
LUMINATE_API bool luminate_managed_plugin_required(const LuminateManagedPlugin *value);
LUMINATE_API bool luminate_managed_plugin_effective_enabled(const LuminateManagedPlugin *value);
LUMINATE_API bool luminate_managed_plugin_activation_locked(const LuminateManagedPlugin *value);
LUMINATE_API uintptr_t luminate_managed_plugin_desired_setting_count(const LuminateManagedPlugin *plugin);
LUMINATE_API LuminateStringView luminate_managed_plugin_desired_setting_key_at(const LuminateManagedPlugin *plugin, uintptr_t index);
LUMINATE_API const LuminateReportedSettingValue *luminate_managed_plugin_desired_setting_value_at(const LuminateManagedPlugin *plugin, uintptr_t index);
LUMINATE_API uintptr_t luminate_managed_plugin_effective_setting_count(const LuminateManagedPlugin *plugin);
LUMINATE_API LuminateStringView luminate_managed_plugin_effective_setting_key_at(const LuminateManagedPlugin *plugin, uintptr_t index);
LUMINATE_API const LuminateReportedSettingValue *luminate_managed_plugin_effective_setting_value_at(const LuminateManagedPlugin *plugin, uintptr_t index);
LUMINATE_API LuminateStringView luminate_plugin_setting_schema_key(const LuminatePluginSettingSchema *value);
LUMINATE_API LuminateStringView luminate_plugin_setting_schema_label(const LuminatePluginSettingSchema *value);
LUMINATE_API LuminateStringView luminate_plugin_setting_schema_description(const LuminatePluginSettingSchema *value);

/*
 * Ordinary-client selector operations generated by
 * `client_selector_status_operation!`.
 */
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_clear_target_selector(LuminateClient *client, const LuminateSelectorInput *selector, LuminateCollectionOutcome **out_outcome);
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_save_current_selector(LuminateClient *client, const LuminateSelectorInput *selector, LuminateCollectionOutcome **out_outcome);
LUMINATE_API
LUMINATE_NODISCARD
LuminateStatus luminate_client_restore_appearance_selector(LuminateClient *client, const LuminateSelectorInput *selector, LuminateCollectionOutcome **out_outcome);

/*
 * Asynchronous request operations generated by local implementation macros.
 */
typedef void (*LuminateAsyncDeviceSnapshotCompletionFn)(void *context, const LuminateAsyncOperation *operation, LuminateStatus status, LuminateDeviceSnapshot *payload);
typedef void (*LuminateAsyncStateSnapshotCompletionFn)(void *context, const LuminateAsyncOperation *operation, LuminateStatus status, LuminateStateSnapshot *payload);
typedef void (*LuminateAsyncCollectionStateSnapshotCompletionFn)(void *context, const LuminateAsyncOperation *operation, LuminateStatus status, LuminateCollectionStateSnapshot *payload);
LUMINATE_API LUMINATE_NODISCARD
LuminateStatus luminate_client_ping_async(LuminateClient *client, void *completion_context, LuminateCompletionContextFreeFn completion_context_free, LuminateAsyncStatusCompletionFn on_complete, LuminateAsyncOperation **out_operation);
LUMINATE_API LUMINATE_NODISCARD
LuminateStatus luminate_client_rescan_async(LuminateClient *client, void *completion_context, LuminateCompletionContextFreeFn completion_context_free, LuminateAsyncStatusCompletionFn on_complete, LuminateAsyncOperation **out_operation);
LUMINATE_API LUMINATE_NODISCARD
LuminateStatus luminate_client_get_device_async(LuminateClient *client, const char *id, void *completion_context, LuminateCompletionContextFreeFn completion_context_free, LuminateAsyncDeviceSnapshotCompletionFn on_complete, LuminateAsyncOperation **out_operation);
LUMINATE_API LUMINATE_NODISCARD
LuminateStatus luminate_client_get_state_async(LuminateClient *client, const char *id, void *completion_context, LuminateCompletionContextFreeFn completion_context_free, LuminateAsyncStateSnapshotCompletionFn on_complete, LuminateAsyncOperation **out_operation);
LUMINATE_API LUMINATE_NODISCARD
LuminateStatus luminate_client_get_collection_state_async(LuminateClient *client, const char *id, void *completion_context, LuminateCompletionContextFreeFn completion_context_free, LuminateAsyncCollectionStateSnapshotCompletionFn on_complete, LuminateAsyncOperation **out_operation);
LUMINATE_API LUMINATE_NODISCARD
LuminateStatus luminate_client_set_brightness_async(LuminateClient *client, const LuminateTarget *target, uint32_t value, void *completion_context, LuminateCompletionContextFreeFn completion_context_free, LuminateAsyncStatusCompletionFn on_complete, LuminateAsyncOperation **out_operation);
LUMINATE_API LUMINATE_NODISCARD
LuminateStatus luminate_client_clear_target_async(LuminateClient *client, const LuminateTarget *target, void *completion_context, LuminateCompletionContextFreeFn completion_context_free, LuminateAsyncStatusCompletionFn on_complete, LuminateAsyncOperation **out_operation);
LUMINATE_API LUMINATE_NODISCARD
LuminateStatus luminate_client_set_off_async(LuminateClient *client, const LuminateTarget *target, void *completion_context, LuminateCompletionContextFreeFn completion_context_free, LuminateAsyncStatusCompletionFn on_complete, LuminateAsyncOperation **out_operation);
LUMINATE_API LUMINATE_NODISCARD
LuminateStatus luminate_client_restore_appearance_async(LuminateClient *client, const LuminateTarget *target, void *completion_context, LuminateCompletionContextFreeFn completion_context_free, LuminateAsyncStatusCompletionFn on_complete, LuminateAsyncOperation **out_operation);
LUMINATE_API LUMINATE_NODISCARD
LuminateStatus luminate_client_save_current_async(LuminateClient *client, const LuminateTarget *target, void *completion_context, LuminateCompletionContextFreeFn completion_context_free, LuminateAsyncStatusCompletionFn on_complete, LuminateAsyncOperation **out_operation);
LUMINATE_API LUMINATE_NODISCARD
LuminateStatus luminate_client_clear_target_selector_async(LuminateClient *client, const LuminateSelectorInput *selector, void *completion_context, LuminateCompletionContextFreeFn completion_context_free, LuminateAsyncCollectionOutcomeCompletionFn on_complete, LuminateAsyncOperation **out_operation);
LUMINATE_API LUMINATE_NODISCARD
LuminateStatus luminate_client_save_current_selector_async(LuminateClient *client, const LuminateSelectorInput *selector, void *completion_context, LuminateCompletionContextFreeFn completion_context_free, LuminateAsyncCollectionOutcomeCompletionFn on_complete, LuminateAsyncOperation **out_operation);
LUMINATE_API LUMINATE_NODISCARD
LuminateStatus luminate_client_restore_appearance_selector_async(LuminateClient *client, const LuminateSelectorInput *selector, void *completion_context, LuminateCompletionContextFreeFn completion_context_free, LuminateAsyncCollectionOutcomeCompletionFn on_complete, LuminateAsyncOperation **out_operation);
LUMINATE_API LUMINATE_NODISCARD
LuminateStatus luminate_client_transition_get_async(LuminateClient *client, const char *id, void *completion_context, LuminateCompletionContextFreeFn completion_context_free, LuminateAsyncTransitionSnapshotCompletionFn on_complete, LuminateAsyncOperation **out_operation);
LUMINATE_API LUMINATE_NODISCARD
LuminateStatus luminate_client_transition_abort_async(LuminateClient *client, const char *id, void *completion_context, LuminateCompletionContextFreeFn completion_context_free, LuminateAsyncTransitionSnapshotCompletionFn on_complete, LuminateAsyncOperation **out_operation);
LUMINATE_API LUMINATE_NODISCARD
LuminateStatus luminate_client_transition_wait_async(LuminateClient *client, const char *id, void *completion_context, LuminateCompletionContextFreeFn completion_context_free, LuminateAsyncTransitionSnapshotCompletionFn on_complete, LuminateAsyncOperation **out_operation);

/*
 * Policy constraint accessors generated by
 * `policy.rs`'s `rule_constraint_string_accessors!` macro.
 */
LUMINATE_API
size_t luminate_policy_rule_device_id_count(const LuminatePolicyRule *rule);
LUMINATE_API
LuminateStringView luminate_policy_rule_device_id_at(const LuminatePolicyRule *rule, size_t index);
LUMINATE_API
size_t luminate_policy_rule_provider_instance_count(const LuminatePolicyRule *rule);
LUMINATE_API
LuminateStringView luminate_policy_rule_provider_instance_at(const LuminatePolicyRule *rule, size_t index);
LUMINATE_API
size_t luminate_policy_rule_collection_count(const LuminatePolicyRule *rule);
LUMINATE_API
LuminateStringView luminate_policy_rule_collection_at(const LuminatePolicyRule *rule, size_t index);

/*
 * Policy binding accessors generated by
 * `policy.rs`'s `binding_string_accessors!` macro.
 */
LUMINATE_API
size_t luminate_policy_binding_subject_count(const LuminatePolicyBinding *binding);
LUMINATE_API
LuminateStringView luminate_policy_binding_subject_at(const LuminatePolicyBinding *binding, size_t index);
LUMINATE_API
size_t luminate_policy_binding_group_count(const LuminatePolicyBinding *binding);
LUMINATE_API
LuminateStringView luminate_policy_binding_group_at(const LuminatePolicyBinding *binding, size_t index);
LUMINATE_API
size_t luminate_policy_binding_role_count(const LuminatePolicyBinding *binding);
LUMINATE_API
LuminateStringView luminate_policy_binding_role_at(const LuminatePolicyBinding *binding, size_t index);

static inline bool luminate_status_is_ok(LuminateStatus status) {
  return status == LUMINATE_STATUS_OK;
}

static inline bool luminate_status_is_error(LuminateStatus status) {
  return status != LUMINATE_STATUS_OK;
}

static inline bool luminate_string_view_is_present(LuminateStringView value) {
  return value.data != NULL;
}

static inline bool luminate_string_view_is_empty(LuminateStringView value) {
  return value.data != NULL && value.len == 0;
}

static inline bool luminate_string_view_equal(LuminateStringView left,
                                               LuminateStringView right) {
  return left.data != NULL && right.data != NULL && left.len == right.len &&
         (left.len == 0 || memcmp(left.data, right.data, left.len) == 0);
}

static inline LuminateRgb luminate_rgb(uint8_t r, uint8_t g, uint8_t b) {
  LuminateRgb value = {r, g, b};
  return value;
}

static inline LuminateTarget luminate_target_device(const char *device_id) {
  LuminateTarget target = {device_id, NULL, NULL, NULL};
  return target;
}

static inline LuminateTarget luminate_target_surface(const char *device_id,
                                                      const char *surface_id) {
  LuminateTarget target = {device_id, surface_id, NULL, NULL};
  return target;
}

static inline LuminateTarget luminate_target_element(const char *device_id,
                                                      const char *surface_id,
                                                      const char *element_id) {
  LuminateTarget target = {device_id, surface_id, element_id, NULL};
  return target;
}

static inline LuminateTarget luminate_target_group(const char *device_id,
                                                    const char *group_id) {
  LuminateTarget target = {device_id, NULL, NULL, group_id};
  return target;
}

LUMINATE_END_DECLS

#endif /* LUMINATE_H */
