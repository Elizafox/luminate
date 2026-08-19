// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Creating, destroying, and re-shaping collections.
//!
//! Collections are owned rather than device-scoped, so these requests pass an
//! empty target list to the ordinary ACL check and then apply a second,
//! ownership-based check via [`authorize_collection_owner`]. Reading and
//! applying appearance to a collection is not here; those live in
//! `inspection` and `appearance` respectively.

use std::slice;

use super::super::authz::RequestAuthorizer;
use luminate_core::collection::{CollectionCategory, CollectionMember};

use super::super::{
    Arc, AuthorizationPolicy, CollectionId, DaemonError, DaemonState, Decision, Mutex, Operation,
    OwnerIdentity, Principal, Response, ResponseStatus, device,
};
use super::{DispatchContext, mutate};

fn member_devices(
    state: &DaemonState,
    members: &[CollectionMember],
) -> Result<Vec<device::DeviceId>, DaemonError> {
    let mut devices = Vec::new();
    for member in members {
        match member {
            CollectionMember::Target(target) => devices.push(target.device_id().clone()),
            CollectionMember::Collection(id) => devices.extend(
                state
                    .resolve_collection_leaves(id)?
                    .into_iter()
                    .map(|target| target.device_id().clone()),
            ),
        }
    }
    Ok(devices)
}

fn collection_devices(
    state: &DaemonState,
    id: &CollectionId,
) -> Result<Vec<device::DeviceId>, DaemonError> {
    Ok(state
        .resolve_collection_leaves(id)?
        .into_iter()
        .map(|target| target.device_id().clone())
        .collect())
}

pub(super) async fn create_collection(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    name: String,
    description: Option<String>,
    kind: Option<CollectionCategory>,
    members: Vec<CollectionMember>,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }

    let owner = OwnerIdentity::from(ctx.principal);
    let (devices, topology_generation) = {
        let state = ctx.state.lock().await;
        let devices = match member_devices(&state, &members) {
            Ok(devices) => devices,
            Err(error) => return error.into(),
        };
        (devices, state.topology_generation())
    };
    let outcome = ctx
        .mutations
        .execute_authorized(
            devices,
            topology_generation,
            Box::new(move |state| {
                state
                    .blocking_lock()
                    .create_collection(name, description, owner, kind, members)
            }),
        )
        .await;

    match outcome {
        Ok(id) => Response {
            status: ResponseStatus::CollectionCreated { id },
        },
        Err(error) => error.into(),
    }
}

pub(super) async fn destroy_collection(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: CollectionId,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }

    if let Err(error) = authorize_collection_owner(ctx.state, &id, ctx.principal, ctx.policy).await
    {
        return error.into();
    }

    let (devices, topology_generation) = {
        let state = ctx.state.lock().await;
        let devices = match collection_devices(&state, &id) {
            Ok(devices) => devices,
            Err(error) => return error.into(),
        };
        (devices, state.topology_generation())
    };
    mutate(ctx.mutations, devices, topology_generation, move |state| {
        state
            .blocking_lock()
            .destroy_collection(&id)
            .map(|_collection| ())
    })
    .await
}

pub(super) async fn add_collection_member(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: CollectionId,
    member: CollectionMember,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }

    if let Err(error) = authorize_collection_owner(ctx.state, &id, ctx.principal, ctx.policy).await
    {
        return error.into();
    }

    let (devices, topology_generation) = {
        let state = ctx.state.lock().await;
        let mut devices = match collection_devices(&state, &id) {
            Ok(devices) => devices,
            Err(error) => return error.into(),
        };
        let added_devices = match member_devices(&state, slice::from_ref(&member)) {
            Ok(devices) => devices,
            Err(error) => return error.into(),
        };
        devices.extend(added_devices);
        (devices, state.topology_generation())
    };
    mutate(ctx.mutations, devices, topology_generation, move |state| {
        state.blocking_lock().add_collection_member(&id, member)
    })
    .await
}

pub(super) async fn remove_collection_member(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: CollectionId,
    member: CollectionMember,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }

    if let Err(error) = authorize_collection_owner(ctx.state, &id, ctx.principal, ctx.policy).await
    {
        return error.into();
    }

    let (devices, topology_generation) = {
        let state = ctx.state.lock().await;
        let devices = match collection_devices(&state, &id) {
            Ok(devices) => devices,
            Err(error) => return error.into(),
        };
        (devices, state.topology_generation())
    };

    mutate(ctx.mutations, devices, topology_generation, move |state| {
        state.blocking_lock().remove_collection_member(&id, &member)
    })
    .await
}

/// Authorizes `principal` to destroy or structurally modify collection
/// `id`.
///
/// The collection owner is always authorized. For any other principal,
/// `policy` is consulted for [`Operation::AdministerCollections`], which
/// grants administrative authority over collections the principal does not
/// own.
///
/// # Errors
///
/// Returns [`DaemonError::CollectionNotFound`] if `id` does not exist.
///
/// Returns [`DaemonError::CollectionNotOwned`] if `principal` is neither
/// the collection owner nor authorized to administer collections.
pub(super) async fn authorize_collection_owner(
    state: &Arc<Mutex<DaemonState>>,
    id: &CollectionId,
    principal: &Principal,
    policy: &dyn AuthorizationPolicy,
) -> Result<(), DaemonError> {
    let owner = state
        .lock()
        .await
        .collection(id)
        .map(|collection| collection.owner.clone())
        .ok_or_else(|| DaemonError::CollectionNotFound(id.clone()))?;
    if owner == OwnerIdentity::from(principal) {
        return Ok(());
    }

    match policy
        .authorize(principal, Operation::AdministerCollections, &[])
        .await
    {
        Decision::Allow => Ok(()),
        Decision::Deny { .. } => Err(DaemonError::CollectionNotOwned(id.clone())),
    }
}
