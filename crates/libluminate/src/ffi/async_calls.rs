// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Asynchronous daemon request operations.

use super::async_common::{
    LuminateCompletionContextFreeFn, submit_client_async, validate_submission,
};
use super::async_operation::LuminateAsyncOperation;
use super::{
    Error, LuminateClient, LuminateServerInfo, LuminateStatus, c_char, c_void, client_ref,
    ffi_guard, read_required_str, sanitize_cstring, server_info_to_ffi,
};
use crate::ffi_typed::access_administration::{
    LuminateAttestationList, LuminateCreatedAttestation, LuminateCreatedToken, LuminateTokenList,
    optional_expiry,
};
use crate::ffi_typed::appearance_slots::{
    LuminateAppearanceSlotInput, read_appearance_slot_inputs,
};
use crate::ffi_typed::collections::{
    LuminateCollectionMemberInput, read_collection_member, read_members,
};
use crate::ffi_typed::effects::read_target;
use crate::ffi_typed::frame::LuminateFrameAck;
use crate::ffi_typed::management::{
    LuminateManagementChangeSet, LuminateManagementPatchBuilder, LuminateManagementSnapshot,
};
use crate::ffi_typed::policy::{LuminatePolicyDocument, read_str_array};
use crate::ffi_typed::scenes::{
    LuminateSceneBindingInput, LuminateSceneBuilder, clone_scene_builder, optional_string,
    read_bindings, read_targets,
};
use crate::ffi_typed::selector::{
    LuminateCollectionOutcome, LuminateSelectorInput, selector_input, unsupported,
};
use crate::ffi_typed::setup::{
    LuminatePluginSetupSession, LuminatePluginSetupWorkflowList, read_session_id,
};
use crate::ffi_typed::shm::{LuminateShmFrameStream, create_ffi_shm_stream};
use crate::ffi_typed::transitions::{
    LuminateTransitionOptions, LuminateTransitionTargetStateInput, options, target_states,
};
use crate::ffi_typed::{
    LuminateCollectionList, LuminateCollectionSnapshot, LuminateCollectionStateSnapshot,
    LuminateDeviceSnapshot, LuminateEffect, LuminateRgb, LuminateSceneList, LuminateSceneSnapshot,
    LuminateStateSnapshot, LuminateTarget, LuminateTopologySnapshot, LuminateTransitionSnapshot,
    LuminateWithdrawnDeviceList,
};
use crate::{
    Colour, CreatedAttestation, CreatedToken, Effect, EmissionState, FrameAck,
    PluginSetupInteractionResponse, Rgb, SceneCaptureMode, SceneId, TransitionId,
};
use luminate_core::collection::{CollectionCategory, CollectionId};
use luminate_core::device::DeviceId;
use luminate_core::frame::{FrameEnvelope, FramePayload};
use luminate_core::policy::{PolicyRevision, PrincipalId};
use std::future::Future;
use std::ptr;
use std::slice;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

/// Completes an asynchronous operation with no payload.
pub type LuminateAsyncStatusCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
    ),
>;

/// Completes an asynchronous server-information request.
pub type LuminateAsyncServerInfoCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateServerInfo,
    ),
>;

/// Completes an asynchronous topology request.
pub type LuminateAsyncTopologySnapshotCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateTopologySnapshot,
    ),
>;

/// Completes an asynchronous withdrawn-device request.
pub type LuminateAsyncWithdrawnDeviceListCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateWithdrawnDeviceList,
    ),
>;

/// Completes an asynchronous device request.
pub type LuminateAsyncDeviceSnapshotCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateDeviceSnapshot,
    ),
>;

/// Completes an asynchronous device-state request.
pub type LuminateAsyncStateSnapshotCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateStateSnapshot,
    ),
>;

/// Completes an asynchronous collection-state request.
pub type LuminateAsyncCollectionStateSnapshotCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateCollectionStateSnapshot,
    ),
>;

macro_rules! validate_client_submission {
    ($client:expr, $on_complete:expr, $out_operation:expr) => {{
        let client = match unsafe { client_ref($client) } {
            Ok(client) => client.clone(),
            Err(status) => return status,
        };
        let on_complete = match validate_submission($on_complete, $out_operation) {
            Ok(callback) => callback,
            Err(status) => return status,
        };
        (client, on_complete)
    }};
}

macro_rules! submit_pointer {
    ($client:expr, $context:expr, $free:expr, $callback:expr, $out:expr, $future:expr, $map:expr) => {{
        let context = $context as usize;
        #[allow(
            clippy::multiple_unsafe_ops_per_block,
            reason = "submission and its typed consumer callback have the same validated FFI contract"
        )]
        unsafe {
            submit_client_async(
                &$client,
                context as *mut c_void,
                $free,
                $out,
                $future,
                move |operation, status, payload| {
                    let payload = payload.map_or(ptr::null_mut(), |value| {
                        Box::into_raw(Box::new(($map)(value)))
                    });
                    $callback(context as *mut c_void, operation, status, payload);
                },
            )
        }
    }};
}

unsafe fn submit_status<F, Fut>(
    client: &super::FfiClient,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: unsafe extern "C" fn(*mut c_void, *const LuminateAsyncOperation, LuminateStatus),
    out_operation: *mut *mut LuminateAsyncOperation,
    future: F,
) -> LuminateStatus
where
    F: FnOnce(Arc<super::Client>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), Error>> + Send + 'static,
{
    let context = completion_context as usize;
    let deliver = move |operation, status, _payload| {
        // SAFETY: the callback was validated and transferred by the accepted
        // submission.
        unsafe { on_complete(context as *mut c_void, operation, status) };
    };
    unsafe {
        submit_client_async(
            client,
            context as *mut c_void,
            completion_context_free,
            out_operation,
            future,
            deliver,
        )
    }
}

/// Asynchronously fetches server information.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_server_info_async(
    client: *mut LuminateClient,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncServerInfoCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            |client| async move { client.server_info().await },
            server_info_to_ffi
        )
    })
}

macro_rules! status_operation {
    ($name:ident, $call:ident) => {
        #[doc = concat!("Asynchronously performs `", stringify!($call), "`.")]
        #[must_use]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(
            client: *mut LuminateClient,
            completion_context: *mut c_void,
            completion_context_free: LuminateCompletionContextFreeFn,
            on_complete: LuminateAsyncStatusCompletionFn,
            out_operation: *mut *mut LuminateAsyncOperation,
        ) -> LuminateStatus {
            ffi_guard(|| {
                let (client, on_complete) =
                    validate_client_submission!(client, on_complete, out_operation);
                unsafe {
                    submit_status(
                        &client,
                        completion_context,
                        completion_context_free,
                        on_complete,
                        out_operation,
                        |client| async move { client.$call().await },
                    )
                }
            })
        }
    };
}

status_operation!(luminate_client_ping_async, ping);
status_operation!(luminate_client_rescan_async, rescan);

/// Asynchronously purges one withdrawn device.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_purge_withdrawn_device_async(
    client: *mut LuminateClient,
    device_id: *const c_char,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncStatusCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let device_id = match unsafe { read_required_str(device_id, "device_id") } {
            Ok(value) => DeviceId::new(value.to_owned()),
            Err(status) => return status,
        };
        unsafe {
            submit_status(
                &client,
                completion_context,
                completion_context_free,
                on_complete,
                out_operation,
                move |client| async move { client.purge_withdrawn_device(device_id).await },
            )
        }
    })
}

/// Asynchronously lists current devices.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_list_devices_async(
    client: *mut LuminateClient,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncTopologySnapshotCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            |client| async move { client.list_devices().await },
            LuminateTopologySnapshot
        )
    })
}

/// Asynchronously lists withdrawn device identifiers.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_list_withdrawn_devices_async(
    client: *mut LuminateClient,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncWithdrawnDeviceListCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            |client| async move { client.list_withdrawn_devices().await },
            LuminateWithdrawnDeviceList
        )
    })
}

macro_rules! optional_snapshot_operation {
    ($name:ident, $callback:ty, $snapshot:ident, $id_kind:literal, $id_type:ident, $call:ident) => {
        #[doc = concat!("Asynchronously fetches one ", $id_kind, " snapshot.")]
        #[must_use]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(
            client: *mut LuminateClient,
            id: *const c_char,
            completion_context: *mut c_void,
            completion_context_free: LuminateCompletionContextFreeFn,
            on_complete: $callback,
            out_operation: *mut *mut LuminateAsyncOperation,
        ) -> LuminateStatus {
            ffi_guard(|| {
                let (client, on_complete) =
                    validate_client_submission!(client, on_complete, out_operation);
                let id = match unsafe { read_required_str(id, concat!($id_kind, "_id")) } {
                    Ok(value) => value.to_owned(),
                    Err(status) => return status,
                };
                let requested = id.clone();
                submit_pointer!(
                    client,
                    completion_context,
                    completion_context_free,
                    on_complete,
                    out_operation,
                    move |client| async move {
                        client
                            .$call($id_type::new(requested))
                            .await?
                            .ok_or_else(|| Error::NotFound(format!(concat!($id_kind, " '{}'"), id)))
                    },
                    $snapshot
                )
            })
        }
    };
}

optional_snapshot_operation!(
    luminate_client_get_device_async,
    LuminateAsyncDeviceSnapshotCompletionFn,
    LuminateDeviceSnapshot,
    "device",
    DeviceId,
    get_device
);
optional_snapshot_operation!(
    luminate_client_get_state_async,
    LuminateAsyncStateSnapshotCompletionFn,
    LuminateStateSnapshot,
    "device",
    DeviceId,
    get_state
);
optional_snapshot_operation!(
    luminate_client_get_collection_state_async,
    LuminateAsyncCollectionStateSnapshotCompletionFn,
    LuminateCollectionStateSnapshot,
    "collection",
    CollectionId,
    get_collection_state
);

/// Asynchronously refreshes one device's state.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_refresh_state_async(
    client: *mut LuminateClient,
    device_id: *const c_char,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncStatusCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let device_id = match unsafe { read_required_str(device_id, "device_id") } {
            Ok(value) => DeviceId::new(value.to_owned()),
            Err(status) => return status,
        };
        unsafe {
            submit_status(
                &client,
                completion_context,
                completion_context_free,
                on_complete,
                out_operation,
                move |client| async move { client.refresh_state(device_id).await },
            )
        }
    })
}

/// Completes an asynchronous attestation creation.
pub type LuminateAsyncCreatedAttestationCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateCreatedAttestation,
    ),
>;

/// Completes an asynchronous attestation-list request.
pub type LuminateAsyncAttestationListCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateAttestationList,
    ),
>;

/// Completes an asynchronous policy-document request.
pub type LuminateAsyncPolicyDocumentCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminatePolicyDocument,
    ),
>;

/// Completes an asynchronous token creation or rotation.
pub type LuminateAsyncCreatedTokenCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateCreatedToken,
    ),
>;

/// Completes an asynchronous token-list request.
pub type LuminateAsyncTokenListCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateTokenList,
    ),
>;

fn principal(
    authority: *const c_char,
    subject: *const c_char,
    authority_what: &'static str,
    subject_what: &'static str,
) -> Result<PrincipalId, LuminateStatus> {
    let authority = unsafe { read_required_str(authority, authority_what) }?.to_owned();
    let subject = unsafe { read_required_str(subject, subject_what) }?.to_owned();
    PrincipalId::new(authority, subject).map_err(|error| {
        super::set_last_error(error.to_string());
        LuminateStatus::InvalidArgument
    })
}

/// Asynchronously creates an identity-only attestation.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_create_attestation_async(
    client: *const LuminateClient,
    name: *const c_char,
    authority: *const c_char,
    subject: *const c_char,
    has_expiry: bool,
    expires_at_unix_ms: u64,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncCreatedAttestationCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    unsafe {
        luminate_client_create_principal_attestation_async(
            client,
            name,
            authority,
            subject,
            ptr::null(),
            0,
            has_expiry,
            expires_at_unix_ms,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
        )
    }
}

/// Asynchronously creates a principal attestation with verified groups.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_create_principal_attestation_async(
    client: *const LuminateClient,
    name: *const c_char,
    authority: *const c_char,
    subject: *const c_char,
    verified_groups: *const *const c_char,
    verified_group_count: usize,
    has_expiry: bool,
    expires_at_unix_ms: u64,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncCreatedAttestationCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) =
            validate_client_submission!(client.cast_mut(), on_complete, out_operation);
        let name = match unsafe { read_required_str(name, "attestation name") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let subject = match principal(
            authority,
            subject,
            "attestation authority",
            "attestation subject",
        ) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let groups = match unsafe {
            read_str_array(
                verified_groups,
                verified_group_count,
                "attestation verified groups",
            )
        } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let expires_at = if has_expiry {
            let Some(value) = UNIX_EPOCH.checked_add(Duration::from_millis(expires_at_unix_ms))
            else {
                super::set_last_error("attestation expiry is out of range");
                return LuminateStatus::InvalidArgument;
            };
            Some(value)
        } else {
            None
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                client
                    .authentication_administration()
                    .create_principal_attestation(name, subject, groups, expires_at)
                    .await
            },
            |created: CreatedAttestation| LuminateCreatedAttestation {
                metadata: created.metadata,
                secret: created.secret,
            }
        )
    })
}

/// Asynchronously lists actor-bound attestations.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_list_attestations_async(
    client: *const LuminateClient,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncAttestationListCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) =
            validate_client_submission!(client.cast_mut(), on_complete, out_operation);
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            |client| async move {
                client
                    .authentication_administration()
                    .list_attestations()
                    .await
            },
            LuminateAttestationList
        )
    })
}

/// Asynchronously revokes an actor-bound attestation.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_revoke_attestation_async(
    client: *const LuminateClient,
    name: *const c_char,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncStatusCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) =
            validate_client_submission!(client.cast_mut(), on_complete, out_operation);
        let name = match unsafe { read_required_str(name, "attestation name") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        unsafe {
            submit_status(
                &client,
                completion_context,
                completion_context_free,
                on_complete,
                out_operation,
                move |client| async move {
                    client
                        .authentication_administration()
                        .revoke_attestation(name)
                        .await
                },
            )
        }
    })
}

/// Asynchronously reads the active access policy.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_get_access_policy_async(
    client: *const LuminateClient,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncPolicyDocumentCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) =
            validate_client_submission!(client.cast_mut(), on_complete, out_operation);
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            |client| async move { client.policy_administration().get().await },
            LuminatePolicyDocument::from_document
        )
    })
}

/// Asynchronously revision-replaces the active access policy.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_replace_access_policy_async(
    client: *const LuminateClient,
    expected_revision: u64,
    replacement: *const LuminatePolicyDocument,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncPolicyDocumentCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) =
            validate_client_submission!(client.cast_mut(), on_complete, out_operation);
        let Some(replacement) = (unsafe { replacement.as_ref() }) else {
            super::set_last_error("replacement access policy pointer is null");
            return LuminateStatus::NullPointer;
        };
        let replacement = replacement.as_document().clone();
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                client
                    .policy_administration()
                    .replace(PolicyRevision(expected_revision), replacement)
                    .await
            },
            LuminatePolicyDocument::from_document
        )
    })
}

/// Asynchronously creates a display-once bearer token.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_create_token_async(
    client: *const LuminateClient,
    id: *const c_char,
    authority: *const c_char,
    subject: *const c_char,
    has_expiry: bool,
    expires_at_unix_ms: u64,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncCreatedTokenCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) =
            validate_client_submission!(client.cast_mut(), on_complete, out_operation);
        let id = match unsafe { read_required_str(id, "token ID") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let subject = match principal(authority, subject, "token authority", "token subject") {
            Ok(value) => value,
            Err(status) => return status,
        };
        let expires_at = match optional_expiry(has_expiry, expires_at_unix_ms) {
            Ok(value) => value,
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                client
                    .authentication_administration()
                    .create_token(id, subject, expires_at)
                    .await
            },
            |created: CreatedToken| LuminateCreatedToken {
                metadata: created.metadata,
                secret: created.secret,
            }
        )
    })
}

/// Asynchronously lists bearer-token metadata.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_list_tokens_async(
    client: *const LuminateClient,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncTokenListCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) =
            validate_client_submission!(client.cast_mut(), on_complete, out_operation);
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            |client| async move { client.authentication_administration().list_tokens().await },
            LuminateTokenList
        )
    })
}

/// Asynchronously revokes a bearer token.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_revoke_token_async(
    client: *const LuminateClient,
    id: *const c_char,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncStatusCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) =
            validate_client_submission!(client.cast_mut(), on_complete, out_operation);
        let id = match unsafe { read_required_str(id, "token ID") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        unsafe {
            submit_status(
                &client,
                completion_context,
                completion_context_free,
                on_complete,
                out_operation,
                move |client| async move {
                    client
                        .authentication_administration()
                        .revoke_token(id)
                        .await
                },
            )
        }
    })
}

/// Asynchronously rotates a bearer token and returns its replacement secret.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_rotate_token_async(
    client: *const LuminateClient,
    id: *const c_char,
    has_expiry: bool,
    expires_at_unix_ms: u64,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncCreatedTokenCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) =
            validate_client_submission!(client.cast_mut(), on_complete, out_operation);
        let id = match unsafe { read_required_str(id, "token ID") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let expires_at = match optional_expiry(has_expiry, expires_at_unix_ms) {
            Ok(value) => value,
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                client
                    .authentication_administration()
                    .rotate_token(id, expires_at)
                    .await
            },
            |created: CreatedToken| LuminateCreatedToken {
                metadata: created.metadata,
                secret: created.secret,
            }
        )
    })
}

/// Completes an asynchronous operation returning an owned C string.
pub type LuminateAsyncStringCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut c_char,
    ),
>;

/// Completes an asynchronous collection-list request.
pub type LuminateAsyncCollectionListCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateCollectionList,
    ),
>;

/// Completes an asynchronous collection request.
pub type LuminateAsyncCollectionSnapshotCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateCollectionSnapshot,
    ),
>;

fn optional_owned_string(
    value: *const c_char,
    field: &'static str,
) -> Result<Option<String>, LuminateStatus> {
    if value.is_null() {
        Ok(None)
    } else {
        unsafe { read_required_str(value, field) }.map(|value| Some(value.to_owned()))
    }
}

/// Asynchronously creates a collection.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_create_collection_async(
    client: *mut LuminateClient,
    name: *const c_char,
    description: *const c_char,
    kind: *const c_char,
    members: *const LuminateCollectionMemberInput,
    member_count: usize,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncStringCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let name = match unsafe { read_required_str(name, "name") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let description = match optional_owned_string(description, "description") {
            Ok(value) => value,
            Err(status) => return status,
        };
        let kind = match optional_owned_string(kind, "kind") {
            Ok(value) => value.map(CollectionCategory::new),
            Err(status) => return status,
        };
        let members = match unsafe { read_members(members, member_count) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let context = completion_context as usize;
        let deliver = move |operation, status, payload: Option<CollectionId>| {
            let payload = payload.map_or(ptr::null_mut(), |id| {
                sanitize_cstring(id.as_str().to_owned()).into_raw()
            });
            unsafe { on_complete(context as *mut c_void, operation, status, payload) };
        };
        unsafe {
            submit_client_async(
                &client,
                completion_context,
                completion_context_free,
                out_operation,
                move |client| async move {
                    client
                        .create_collection(name, description, kind, members)
                        .await
                },
                deliver,
            )
        }
    })
}

/// Asynchronously destroys a collection.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_destroy_collection_async(
    client: *mut LuminateClient,
    id: *const c_char,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncStatusCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let id = match unsafe { read_required_str(id, "collection_id") } {
            Ok(value) => CollectionId::new(value.to_owned()),
            Err(status) => return status,
        };
        unsafe {
            submit_status(
                &client,
                completion_context,
                completion_context_free,
                on_complete,
                out_operation,
                move |client| async move { client.destroy_collection(id).await },
            )
        }
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "the shared implementation retains the public asynchronous ABI parameters"
)]
unsafe fn collection_member_async(
    client: *mut LuminateClient,
    id: *const c_char,
    member: *const LuminateCollectionMemberInput,
    add: bool,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncStatusCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let id = match unsafe { read_required_str(id, "collection_id") } {
            Ok(value) => CollectionId::new(value.to_owned()),
            Err(status) => return status,
        };
        let Some(member) = (unsafe { member.as_ref() }) else {
            super::set_last_error("member pointer is null");
            return LuminateStatus::NullPointer;
        };
        let member = match unsafe { read_collection_member(member) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        unsafe {
            submit_status(
                &client,
                completion_context,
                completion_context_free,
                on_complete,
                out_operation,
                move |client| async move {
                    if add {
                        client.add_collection_member(id, member).await
                    } else {
                        client.remove_collection_member(id, member).await
                    }
                },
            )
        }
    })
}

/// Asynchronously adds one explicit collection member.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_add_collection_member_async(
    client: *mut LuminateClient,
    id: *const c_char,
    member: *const LuminateCollectionMemberInput,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncStatusCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    unsafe {
        collection_member_async(
            client,
            id,
            member,
            true,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
        )
    }
}

/// Asynchronously removes one explicit collection member.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_remove_collection_member_async(
    client: *mut LuminateClient,
    id: *const c_char,
    member: *const LuminateCollectionMemberInput,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncStatusCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    unsafe {
        collection_member_async(
            client,
            id,
            member,
            false,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
        )
    }
}

/// Asynchronously lists collections.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_list_collections_async(
    client: *mut LuminateClient,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncCollectionListCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            |client| async move { client.list_collections().await },
            LuminateCollectionList
        )
    })
}

/// Asynchronously fetches one collection.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_get_collection_async(
    client: *mut LuminateClient,
    id: *const c_char,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncCollectionSnapshotCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let id = match unsafe { read_required_str(id, "collection_id") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let requested = id.clone();
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                client
                    .get_collection(CollectionId::new(requested))
                    .await?
                    .ok_or_else(|| Error::NotFound(format!("collection '{id}'")))
            },
            LuminateCollectionSnapshot
        )
    })
}

/// Asynchronously applies logical appearance slots to a concrete surface.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_set_appearance_slots_async(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    values: *const LuminateAppearanceSlotInput,
    value_count: usize,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncStatusCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let target = match unsafe { read_target(target) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let values = match unsafe { read_appearance_slot_inputs(values, value_count) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        unsafe {
            submit_status(
                &client,
                completion_context,
                completion_context_free,
                on_complete,
                out_operation,
                move |client| async move { client.set_appearance_slots(target, values).await },
            )
        }
    })
}

/// Completes an asynchronous scene request.
pub type LuminateAsyncSceneSnapshotCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateSceneSnapshot,
    ),
>;

/// Completes an asynchronous scene-list request.
pub type LuminateAsyncSceneListCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateSceneList,
    ),
>;

fn capture_mode(value: *const c_char) -> Result<SceneCaptureMode, LuminateStatus> {
    if value.is_null() {
        Ok(SceneCaptureMode::Frozen)
    } else {
        let id = unsafe { read_required_str(value, "dynamic_collection_id") }?;
        Ok(SceneCaptureMode::DynamicCollectionMembers {
            collection: CollectionId::new(id.to_owned()),
        })
    }
}

/// Asynchronously creates an explicitly authored scene.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_create_scene_async(
    client: *mut LuminateClient,
    name: *const c_char,
    description: *const c_char,
    bindings: *const LuminateSceneBindingInput,
    binding_count: usize,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncSceneSnapshotCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let name = match unsafe { read_required_str(name, "name") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let description = match unsafe { optional_string(description, "description") } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let bindings = match unsafe { read_bindings(bindings, binding_count) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move { client.create_scene(name, description, bindings).await },
            LuminateSceneSnapshot
        )
    })
}

/// Asynchronously captures intended state into a new scene.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_capture_scene_async(
    client: *mut LuminateClient,
    name: *const c_char,
    description: *const c_char,
    dynamic_collection_id: *const c_char,
    targets: *const LuminateTarget,
    target_count: usize,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncSceneSnapshotCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let name = match unsafe { read_required_str(name, "name") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let description = match unsafe { optional_string(description, "description") } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let targets = match unsafe { read_targets(targets, target_count) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let mode = match capture_mode(dynamic_collection_id) {
            Ok(value) => value,
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move { client.capture_scene(name, description, mode, targets).await },
            LuminateSceneSnapshot
        )
    })
}

/// Asynchronously replaces an explicitly authored scene.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_replace_scene_async(
    client: *mut LuminateClient,
    id: *const c_char,
    expected_revision: u64,
    name: *const c_char,
    description: *const c_char,
    bindings: *const LuminateSceneBindingInput,
    binding_count: usize,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncSceneSnapshotCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let id = match unsafe { read_required_str(id, "scene_id") } {
            Ok(value) => SceneId::new(value.to_owned()),
            Err(status) => return status,
        };
        let name = match unsafe { read_required_str(name, "name") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let description = match unsafe { optional_string(description, "description") } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let bindings = match unsafe { read_bindings(bindings, binding_count) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                client
                    .replace_scene(id, expected_revision, name, description, bindings)
                    .await
            },
            LuminateSceneSnapshot
        )
    })
}

/// Asynchronously replaces a scene from an independent seeded builder. The
/// builder is copied during submission and may be mutated or freed after this
/// function returns.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_replace_scene_from_builder_async(
    client: *mut LuminateClient,
    builder: *const LuminateSceneBuilder,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncSceneSnapshotCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let builder = match clone_scene_builder(builder) {
            Ok(value) => value,
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                client
                    .replace_scene(
                        builder.id,
                        builder.expected_revision,
                        builder.name,
                        builder.description,
                        builder.bindings,
                    )
                    .await
            },
            LuminateSceneSnapshot
        )
    })
}

/// Asynchronously recaptures intended state for an existing scene.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_recapture_scene_async(
    client: *mut LuminateClient,
    id: *const c_char,
    expected_revision: u64,
    dynamic_collection_id: *const c_char,
    targets: *const LuminateTarget,
    target_count: usize,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncSceneSnapshotCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let id = match unsafe { read_required_str(id, "scene_id") } {
            Ok(value) => SceneId::new(value.to_owned()),
            Err(status) => return status,
        };
        let targets = match unsafe { read_targets(targets, target_count) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let mode = match capture_mode(dynamic_collection_id) {
            Ok(value) => value,
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                client
                    .recapture_scene(id, expected_revision, mode, targets)
                    .await
            },
            LuminateSceneSnapshot
        )
    })
}

/// Asynchronously deletes a scene at an expected revision.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_delete_scene_async(
    client: *mut LuminateClient,
    id: *const c_char,
    expected_revision: u64,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncStatusCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let id = match unsafe { read_required_str(id, "scene_id") } {
            Ok(value) => SceneId::new(value.to_owned()),
            Err(status) => return status,
        };
        unsafe {
            submit_status(
                &client,
                completion_context,
                completion_context_free,
                on_complete,
                out_operation,
                move |client| async move { client.delete_scene(id, expected_revision).await },
            )
        }
    })
}

/// Asynchronously lists observable scenes.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_list_scenes_async(
    client: *mut LuminateClient,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncSceneListCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            |client| async move { client.list_scenes().await },
            LuminateSceneList
        )
    })
}

/// Asynchronously fetches one observable scene.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_get_scene_async(
    client: *mut LuminateClient,
    id: *const c_char,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncSceneSnapshotCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let id = match unsafe { read_required_str(id, "scene_id") } {
            Ok(value) => SceneId::new(value.to_owned()),
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                client
                    .get_scene(id)
                    .await?
                    .ok_or_else(|| Error::NotFound("scene".to_owned()))
            },
            LuminateSceneSnapshot
        )
    })
}

/// Asynchronously applies a scene immediately.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_apply_scene_async(
    client: *mut LuminateClient,
    id: *const c_char,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncStatusCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let id = match unsafe { read_required_str(id, "scene_id") } {
            Ok(value) => SceneId::new(value.to_owned()),
            Err(status) => return status,
        };
        unsafe {
            submit_status(
                &client,
                completion_context,
                completion_context_free,
                on_complete,
                out_operation,
                move |client| async move { client.apply_scene(id).await.map(|_| ()) },
            )
        }
    })
}

/// Completes an asynchronous selector mutation.
pub type LuminateAsyncCollectionOutcomeCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateCollectionOutcome,
    ),
>;

fn emission_state(value: u32) -> Result<EmissionState, LuminateStatus> {
    match value {
        value if value == EmissionState::Dark as u32 => Ok(EmissionState::Dark),
        value if value == EmissionState::Emitting as u32 => Ok(EmissionState::Emitting),
        _ => {
            super::set_last_error("invalid LuminateEmissionState");
            Err(LuminateStatus::InvalidArgument)
        }
    }
}

fn effect_input(value: *const LuminateEffect) -> Result<Effect, LuminateStatus> {
    let Some(value) = (unsafe { value.cast::<Effect>().as_ref() }) else {
        super::set_last_error("effect pointer is null");
        return Err(LuminateStatus::NullPointer);
    };
    Ok(value.clone())
}

macro_rules! direct_target_operation_async {
    ($name:ident, $method:ident $(, $argument:ident: $type:ty)*) => {
        #[doc = concat!("Asynchronously performs `", stringify!($method), "` on one target.")]
        #[must_use]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(
            client: *mut LuminateClient,
            target: *const LuminateTarget,
            $($argument: $type,)*
            completion_context: *mut c_void,
            completion_context_free: LuminateCompletionContextFreeFn,
            on_complete: LuminateAsyncStatusCompletionFn,
            out_operation: *mut *mut LuminateAsyncOperation,
        ) -> LuminateStatus {
            ffi_guard(|| {
                let (client, on_complete) =
                    validate_client_submission!(client, on_complete, out_operation);
                let target = match unsafe { read_target(target) } {
                    Ok(value) => value,
                    Err(status) => return status,
                };
                unsafe {
                    submit_status(
                        &client,
                        completion_context,
                        completion_context_free,
                        on_complete,
                        out_operation,
                        move |client| async move { client.$method(target $(, $argument)*).await },
                    )
                }
            })
        }
    };
}

direct_target_operation_async!(luminate_client_set_brightness_async, set_brightness, value: u32);
direct_target_operation_async!(luminate_client_clear_target_async, clear_target);
direct_target_operation_async!(luminate_client_set_off_async, set_off);
direct_target_operation_async!(luminate_client_restore_appearance_async, restore_appearance);
direct_target_operation_async!(luminate_client_save_current_async, save_current);

/// Asynchronously applies an effect to one target.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_set_effect_async(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    effect: *const LuminateEffect,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncStatusCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let target = match unsafe { read_target(target) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let effect = match effect_input(effect) {
            Ok(value) => value,
            Err(status) => return status,
        };
        unsafe {
            submit_status(
                &client,
                completion_context,
                completion_context_free,
                on_complete,
                out_operation,
                move |client| async move { client.set_effect(target, effect).await },
            )
        }
    })
}

/// Asynchronously changes one target's emission state.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_set_emission_async(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    state: u32,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncStatusCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let state = match emission_state(state) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let target = match unsafe { read_target(target) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        unsafe {
            submit_status(
                &client,
                completion_context,
                completion_context_free,
                on_complete,
                out_operation,
                move |client| async move { client.set_emission(target, state).await },
            )
        }
    })
}

macro_rules! basic_selector_operation_async {
    ($name:ident, $method:ident) => {
        #[doc = concat!("Asynchronously performs `", stringify!($method), "` for a selector.")]
        #[must_use]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(
            client: *mut LuminateClient,
            selector: *const LuminateSelectorInput,
            completion_context: *mut c_void,
            completion_context_free: LuminateCompletionContextFreeFn,
            on_complete: LuminateAsyncCollectionOutcomeCompletionFn,
            out_operation: *mut *mut LuminateAsyncOperation,
        ) -> LuminateStatus {
            ffi_guard(|| {
                let (client, on_complete) =
                    validate_client_submission!(client, on_complete, out_operation);
                let selector = match unsafe { selector_input(selector) } {
                    Ok(value) => value,
                    Err(status) => return status,
                };
                submit_pointer!(
                    client,
                    completion_context,
                    completion_context_free,
                    on_complete,
                    out_operation,
                    move |client| async move { client.$method(selector).await },
                    LuminateCollectionOutcome
                )
            })
        }
    };
}

basic_selector_operation_async!(
    luminate_client_clear_target_selector_async,
    clear_target_selector
);
basic_selector_operation_async!(
    luminate_client_save_current_selector_async,
    save_current_selector
);
basic_selector_operation_async!(
    luminate_client_restore_appearance_selector_async,
    restore_appearance_selector
);

/// Asynchronously sets brightness through a selector.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_set_brightness_selector_async(
    client: *mut LuminateClient,
    selector: *const LuminateSelectorInput,
    value: u32,
    has_policy: bool,
    policy: u32,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncCollectionOutcomeCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let selector = match unsafe { selector_input(selector) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let policy = match unsupported(has_policy, policy) {
            Ok(value) => value,
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                client
                    .set_brightness_selector(selector, value, policy)
                    .await
            },
            LuminateCollectionOutcome
        )
    })
}

/// Asynchronously applies an effect through a selector.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_set_effect_selector_async(
    client: *mut LuminateClient,
    selector: *const LuminateSelectorInput,
    effect: *const LuminateEffect,
    has_policy: bool,
    policy: u32,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncCollectionOutcomeCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let selector = match unsafe { selector_input(selector) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let effect = match effect_input(effect) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let policy = match unsupported(has_policy, policy) {
            Ok(value) => value,
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move { client.set_effect_selector(selector, effect, policy).await },
            LuminateCollectionOutcome
        )
    })
}

/// Asynchronously changes emission state through a selector.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_set_emission_selector_async(
    client: *mut LuminateClient,
    selector: *const LuminateSelectorInput,
    state: u32,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncCollectionOutcomeCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let selector = match unsafe { selector_input(selector) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let state = match emission_state(state) {
            Ok(value) => value,
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move { client.set_emission_selector(selector, state).await },
            LuminateCollectionOutcome
        )
    })
}

/// Completes an asynchronous operation returning a generation number.
pub type LuminateAsyncU32CompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        value: u32,
    ),
>;

/// Completes an asynchronous frame upload.
pub type LuminateAsyncFrameAckCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        value: LuminateFrameAck,
    ),
>;

/// Asynchronously begins an ordinary frame stream.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_begin_frame_stream_async(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncU32CompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let target = match unsafe { read_target(target) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let context = completion_context as usize;
        let deliver = move |operation, status, value: Option<u32>| {
            unsafe {
                on_complete(
                    context as *mut c_void,
                    operation,
                    status,
                    value.unwrap_or_default(),
                );
            };
        };
        unsafe {
            submit_client_async(
                &client,
                completion_context,
                completion_context_free,
                out_operation,
                move |client| async move { client.begin_frame_stream(target).await },
                deliver,
            )
        }
    })
}

fn read_frame_colours(
    colours: *const LuminateRgb,
    count: usize,
) -> Result<Vec<Colour>, LuminateStatus> {
    if colours.is_null() && count != 0 {
        super::set_last_error("frame colours pointer is null");
        return Err(LuminateStatus::NullPointer);
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    Ok(unsafe { slice::from_raw_parts(colours, count) }
        .iter()
        .map(|value| Colour::rgb(Rgb::new(value.r, value.g, value.b)))
        .collect())
}

#[allow(
    clippy::too_many_arguments,
    reason = "the shared uploader retains the public asynchronous ABI parameters"
)]
unsafe fn upload_frame_async(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    envelope: FrameEnvelope,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncFrameAckCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let target = match unsafe { read_target(target) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let context = completion_context as usize;
        let deliver = move |operation, status, value: Option<FrameAck>| {
            let value = value.map_or(
                LuminateFrameAck {
                    sequence: 0,
                    dropped: 0,
                },
                |value| LuminateFrameAck {
                    sequence: value.sequence,
                    dropped: u8::from(value.dropped),
                },
            );
            unsafe { on_complete(context as *mut c_void, operation, status, value) };
        };
        unsafe {
            submit_client_async(
                &client,
                completion_context,
                completion_context_free,
                out_operation,
                move |client| async move { client.upload_frame(target, envelope).await },
                deliver,
            )
        }
    })
}

/// Asynchronously uploads a full ordinary frame.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_upload_frame_full_async(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    generation: u32,
    sequence: u64,
    colours: *const LuminateRgb,
    count: usize,
    commit: u8,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncFrameAckCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    let colours = match read_frame_colours(colours, count) {
        Ok(value) => value,
        Err(status) => return status,
    };
    let envelope = FrameEnvelope {
        generation,
        sequence,
        payload: FramePayload::Full(colours),
        commit: commit != 0,
    };
    unsafe {
        upload_frame_async(
            client,
            target,
            envelope,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
        )
    }
}

/// Asynchronously uploads a sparse ordinary frame.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_upload_frame_partial_async(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    generation: u32,
    sequence: u64,
    indices: *const u32,
    colours: *const LuminateRgb,
    count: usize,
    commit: u8,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncFrameAckCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    if count != 0 && indices.is_null() {
        super::set_last_error("frame indices or colours pointer is null");
        return LuminateStatus::NullPointer;
    }
    let colours = match read_frame_colours(colours, count) {
        Ok(value) => value,
        Err(status) => return status,
    };
    let indices = if count == 0 {
        Vec::new()
    } else {
        unsafe { slice::from_raw_parts(indices, count) }.to_vec()
    };
    let envelope = FrameEnvelope {
        generation,
        sequence,
        payload: FramePayload::Partial(indices.into_iter().zip(colours).collect()),
        commit: commit != 0,
    };
    unsafe {
        upload_frame_async(
            client,
            target,
            envelope,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
        )
    }
}

/// Asynchronously ends an ordinary frame stream.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_end_frame_stream_async(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    generation: u32,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncStatusCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let target = match unsafe { read_target(target) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        unsafe {
            submit_status(
                &client,
                completion_context,
                completion_context_free,
                on_complete,
                out_operation,
                move |client| async move { client.end_frame_stream(target, generation).await },
            )
        }
    })
}

/// Completes asynchronous shared-memory stream negotiation.
pub type LuminateAsyncShmFrameStreamCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        stream: *mut LuminateShmFrameStream,
    ),
>;

/// Asynchronously negotiates a client-published shared-memory frame stream.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_begin_shm_frame_stream_async(
    client: *mut LuminateClient,
    target: *const LuminateTarget,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncShmFrameStreamCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let target = match unsafe { read_target(target) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let stream_client = client.clone();
        let negotiated_target = target.clone();
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                let ready = client.begin_shm_frame_stream(negotiated_target).await?;
                let generation = ready.generation;
                match create_ffi_shm_stream(stream_client, target.clone(), &ready) {
                    Ok(stream) => Ok(stream),
                    Err(error) => {
                        // Cleanup is deliberately detached from local request
                        // cancellation once the daemon accepted negotiation.
                        tokio::spawn(async move {
                            let _ = client.end_shm_frame_stream(target, generation).await;
                        });
                        Err(error)
                    }
                }
            },
            |stream| stream
        )
    })
}

/// Asynchronously ends and consumes a shared-memory frame stream.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_end_shm_frame_stream_async(
    stream: *mut LuminateShmFrameStream,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncStatusCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        if stream.is_null() {
            super::set_last_error("shm frame stream handle is null");
            return LuminateStatus::NullPointer;
        }
        let on_complete = match validate_submission(on_complete, out_operation) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let stream_ref = unsafe { &*stream };
        let client = stream_ref.0.client().clone();
        // SAFETY: validation succeeded, so this accepted submission consumes
        // the caller's uniquely owned stream handle.
        let stream = unsafe { Box::from_raw(stream) }.0;
        let (_retained_client, target, generation) = stream.into_teardown();
        unsafe {
            submit_status(
                &client,
                completion_context,
                completion_context_free,
                on_complete,
                out_operation,
                move |client| async move {
                    // This inner task is a shielded resource cleanup. Aborting
                    // the public operation stops delivery but not teardown.
                    tokio::spawn(
                        async move { client.end_shm_frame_stream(target, generation).await },
                    )
                    .await
                    .map_err(|error| {
                        Error::Internal(format!("shared-memory teardown task failed: {error}"))
                    })?
                },
            )
        }
    })
}

/// Completes an asynchronous management snapshot request.
pub type LuminateAsyncManagementSnapshotCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateManagementSnapshot,
    ),
>;

/// Completes an asynchronous management patch.
pub type LuminateAsyncManagementChangeSetCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateManagementChangeSet,
    ),
>;

/// Completes an asynchronous plugin setup session operation.
pub type LuminateAsyncPluginSetupSessionCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminatePluginSetupSession,
    ),
>;

/// Completes an asynchronous plugin setup workflow request.
pub type LuminateAsyncPluginSetupWorkflowListCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminatePluginSetupWorkflowList,
    ),
>;

/// Asynchronously reads authoritative daemon management state.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_get_management_async(
    client: *mut LuminateClient,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncManagementSnapshotCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            |client| async move { client.get_management().await },
            LuminateManagementSnapshot
        )
    })
}

/// Asynchronously applies an atomic management patch.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_patch_management_async(
    client: *mut LuminateClient,
    patch: *const LuminateManagementPatchBuilder,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncManagementChangeSetCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let Some(patch) = (unsafe { patch.as_ref() }) else {
            super::set_last_error("management patch builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        let patch = patch.0.clone();
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move { client.patch_management(patch).await },
            LuminateManagementChangeSet
        )
    })
}

/// Asynchronously starts one plugin setup workflow.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_plugin_setup_start_async(
    client: *mut LuminateClient,
    plugin: *const c_char,
    workflow: *const c_char,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncPluginSetupSessionCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let plugin = match unsafe { read_required_str(plugin, "plugin") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let workflow = match unsafe { read_required_str(workflow, "workflow") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move { client.start_plugin_setup(plugin, workflow).await },
            LuminatePluginSetupSession
        )
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "the shared responder retains the public asynchronous ABI parameters"
)]
unsafe fn plugin_setup_respond_async(
    client: *mut LuminateClient,
    session_id: *const c_char,
    generation: u64,
    response: PluginSetupInteractionResponse,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncPluginSetupSessionCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let session = match unsafe { read_session_id(session_id) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                client
                    .respond_plugin_setup(session, generation, response)
                    .await
            },
            LuminatePluginSetupSession
        )
    })
}

/// Asynchronously selects one plugin setup choice.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_plugin_setup_choose_async(
    client: *mut LuminateClient,
    session_id: *const c_char,
    generation: u64,
    choice: *const c_char,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncPluginSetupSessionCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    let choice = match unsafe { read_required_str(choice, "choice") } {
        Ok(value) => value.to_owned(),
        Err(status) => return status,
    };
    unsafe {
        plugin_setup_respond_async(
            client,
            session_id,
            generation,
            PluginSetupInteractionResponse::Choice(choice),
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
        )
    }
}

/// Asynchronously confirms a plugin setup physical action.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_plugin_setup_confirm_async(
    client: *mut LuminateClient,
    session_id: *const c_char,
    generation: u64,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncPluginSetupSessionCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    unsafe {
        plugin_setup_respond_async(
            client,
            session_id,
            generation,
            PluginSetupInteractionResponse::Confirmed,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
        )
    }
}

unsafe fn plugin_setup_session_async(
    client: *mut LuminateClient,
    session_id: *const c_char,
    cancel: bool,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncPluginSetupSessionCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let session = match unsafe { read_session_id(session_id) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                if cancel {
                    client.cancel_plugin_setup(session).await
                } else {
                    client.plugin_setup_session(session).await
                }
            },
            LuminatePluginSetupSession
        )
    })
}

/// Asynchronously reads one plugin setup session.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_plugin_setup_get_async(
    client: *mut LuminateClient,
    session_id: *const c_char,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncPluginSetupSessionCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    unsafe {
        plugin_setup_session_async(
            client,
            session_id,
            false,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
        )
    }
}

/// Asynchronously cancels one plugin setup session.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_plugin_setup_cancel_async(
    client: *mut LuminateClient,
    session_id: *const c_char,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncPluginSetupSessionCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    unsafe {
        plugin_setup_session_async(
            client,
            session_id,
            true,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
        )
    }
}

/// Asynchronously lists one plugin's setup workflows.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_plugin_setup_workflows_async(
    client: *mut LuminateClient,
    plugin: *const c_char,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncPluginSetupWorkflowListCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let plugin = match unsafe { read_required_str(plugin, "plugin") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move { client.plugin_setup_workflows(plugin).await },
            LuminatePluginSetupWorkflowList
        )
    })
}

/// Completes an asynchronous transition operation.
pub type LuminateAsyncTransitionSnapshotCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        operation: *const LuminateAsyncOperation,
        status: LuminateStatus,
        payload: *mut LuminateTransitionSnapshot,
    ),
>;

/// Asynchronously starts a scene-to-scene transition.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_transition_scene_to_scene_async(
    client: *mut LuminateClient,
    source_scene_id: *const c_char,
    destination_scene_id: *const c_char,
    timing: LuminateTransitionOptions,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncTransitionSnapshotCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let source = match unsafe { read_required_str(source_scene_id, "source_scene_id") } {
            Ok(value) => SceneId::new(value.to_owned()),
            Err(status) => return status,
        };
        let destination =
            match unsafe { read_required_str(destination_scene_id, "destination_scene_id") } {
                Ok(value) => SceneId::new(value.to_owned()),
                Err(status) => return status,
            };
        let timing = match options(timing) {
            Ok(value) => value,
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                client
                    .transitions()
                    .scene_to_scene(source, destination, timing)
                    .await
            },
            LuminateTransitionSnapshot
        )
    })
}

/// Asynchronously starts a current-to-scene transition.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_transition_current_to_scene_async(
    client: *mut LuminateClient,
    destination_scene_id: *const c_char,
    timing: LuminateTransitionOptions,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncTransitionSnapshotCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let destination =
            match unsafe { read_required_str(destination_scene_id, "destination_scene_id") } {
                Ok(value) => SceneId::new(value.to_owned()),
                Err(status) => return status,
            };
        let timing = match options(timing) {
            Ok(value) => value,
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                client
                    .transitions()
                    .current_to_scene(destination, timing)
                    .await
            },
            LuminateTransitionSnapshot
        )
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "the shared state-transition implementation retains the public asynchronous ABI parameters"
)]
unsafe fn transition_to_states_async(
    client: *mut LuminateClient,
    source: Option<SceneId>,
    states: *const LuminateTransitionTargetStateInput,
    state_count: usize,
    timing: LuminateTransitionOptions,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncTransitionSnapshotCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let states = match unsafe { target_states(states, state_count) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        let timing = match options(timing) {
            Ok(value) => value,
            Err(error) => return error,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                match source {
                    Some(source) => {
                        client
                            .transitions()
                            .scene_to_states(source, states, timing)
                            .await
                    }
                    None => client.transitions().current_to_states(states, timing).await,
                }
            },
            LuminateTransitionSnapshot
        )
    })
}

/// Asynchronously starts a scene-to-ephemeral-state transition.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_transition_scene_to_states_async(
    client: *mut LuminateClient,
    source_scene_id: *const c_char,
    states: *const LuminateTransitionTargetStateInput,
    state_count: usize,
    timing: LuminateTransitionOptions,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncTransitionSnapshotCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    let source = match unsafe { read_required_str(source_scene_id, "source_scene_id") } {
        Ok(value) => SceneId::new(value.to_owned()),
        Err(error) => return error,
    };
    unsafe {
        transition_to_states_async(
            client,
            Some(source),
            states,
            state_count,
            timing,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
        )
    }
}

/// Asynchronously starts a current-to-ephemeral-state transition.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_transition_current_to_states_async(
    client: *mut LuminateClient,
    states: *const LuminateTransitionTargetStateInput,
    state_count: usize,
    timing: LuminateTransitionOptions,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncTransitionSnapshotCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    unsafe {
        transition_to_states_async(
            client,
            None,
            states,
            state_count,
            timing,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
        )
    }
}

unsafe fn transition_by_id_async(
    client: *mut LuminateClient,
    id: *const c_char,
    operation: u8,
    completion_context: *mut c_void,
    completion_context_free: LuminateCompletionContextFreeFn,
    on_complete: LuminateAsyncTransitionSnapshotCompletionFn,
    out_operation: *mut *mut LuminateAsyncOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let (client, on_complete) = validate_client_submission!(client, on_complete, out_operation);
        let id = match unsafe { read_required_str(id, "transition_id") } {
            Ok(value) => TransitionId::new(value.to_owned()),
            Err(status) => return status,
        };
        submit_pointer!(
            client,
            completion_context,
            completion_context_free,
            on_complete,
            out_operation,
            move |client| async move {
                match operation {
                    0 => client.transitions().get(id).await,
                    1 => client.transitions().abort(id).await,
                    _ => client.transitions().wait(id).await,
                }
            },
            LuminateTransitionSnapshot
        )
    })
}

macro_rules! transition_by_id_operation_async {
    ($name:ident, $operation:literal, $description:literal) => {
        #[doc = $description]
        #[must_use]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(
            client: *mut LuminateClient,
            id: *const c_char,
            completion_context: *mut c_void,
            completion_context_free: LuminateCompletionContextFreeFn,
            on_complete: LuminateAsyncTransitionSnapshotCompletionFn,
            out_operation: *mut *mut LuminateAsyncOperation,
        ) -> LuminateStatus {
            unsafe {
                transition_by_id_async(
                    client,
                    id,
                    $operation,
                    completion_context,
                    completion_context_free,
                    on_complete,
                    out_operation,
                )
            }
        }
    };
}

transition_by_id_operation_async!(
    luminate_client_transition_get_async,
    0,
    "Asynchronously gets a transition snapshot."
);
transition_by_id_operation_async!(
    luminate_client_transition_abort_async,
    1,
    "Asynchronously aborts a transition and waits for quiescence."
);
transition_by_id_operation_async!(
    luminate_client_transition_wait_async,
    2,
    "Asynchronously waits for a terminal transition without aborting it on local cancellation."
);
