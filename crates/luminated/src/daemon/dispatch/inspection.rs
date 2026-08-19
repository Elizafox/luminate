// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Read-only requests.
//!
//! Each handler authorizes, takes the state lock, and answers from the
//! in-memory snapshot. Nothing here reaches a plugin or mutates anything, so
//! none of it goes through the mutation executor.

use luminate_core::collection::{Collection, CollectionGraph, CollectionMember};

use super::super::authz::RequestAuthorizer;
use super::super::{
    CollectionId, DaemonError, HashSet, PROTOCOL_ABI_VERSION, Response, ResponseStatus, ServerInfo,
    device, env, slice,
};
use super::DispatchContext;

pub(super) async fn server_info(auth: &RequestAuthorizer<'_>) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }

    Response {
        status: ResponseStatus::ServerInfo(ServerInfo {
            daemon_name: env!("CARGO_PKG_NAME").to_owned(),
            daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol_abi_version: PROTOCOL_ABI_VERSION,
        }),
    }
}

pub(super) async fn list_devices(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
) -> Response {
    let (mut devices, topology_generation) = {
        let state = ctx.state.lock().await;
        (state.devices(), state.topology_generation())
    };
    for device in &mut devices {
        attach_provider_instance(ctx, device);
    }
    let ids: Vec<_> = devices.iter().map(|device| device.id.clone()).collect();
    let (visible, authorized_generation) = match auth.filter_devices(&ids).await {
        Ok(result) => result,
        Err(response) => return response,
    };
    if topology_generation != authorized_generation
        || !auth.topology_is_current(topology_generation).await
    {
        return authorization_conflict().into();
    }
    let visible: HashSet<_> = visible.into_iter().collect();
    Response {
        status: ResponseStatus::Devices(
            devices
                .into_iter()
                .filter(|device| visible.contains(&device.id))
                .collect(),
        ),
    }
}

pub(super) async fn list_withdrawn_devices(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
) -> Response {
    let topology_generation = match auth.authorize(&[]).await {
        Ok(generation) => generation,
        Err(response) => return response,
    };

    let state = ctx.state.lock().await;
    if !state.topology_generation_is(topology_generation) {
        return authorization_conflict().into();
    }
    Response {
        status: ResponseStatus::WithdrawnDevices(state.withdrawn_device_ids()),
    }
}

pub(super) async fn get_device(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: device::DeviceId,
) -> Response {
    let topology_generation = match auth.authorize(slice::from_ref(&id)).await {
        Ok(generation) => generation,
        Err(response) => return response,
    };

    let state = ctx.state.lock().await;
    if !state.topology_generation_is(topology_generation) {
        return authorization_conflict().into();
    }
    let device = state.device(&id).map(|mut device| {
        attach_provider_instance(ctx, &mut device);
        Box::new(device)
    });
    Response {
        status: ResponseStatus::Device(device),
    }
}

fn attach_provider_instance(ctx: &DispatchContext<'_>, device: &mut device::Device) {
    device.provider_instance = ctx.plugin_manager.owner_name(&device.id);
}

pub(super) async fn get_state(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    device: device::DeviceId,
) -> Response {
    let topology_generation = match auth.authorize(slice::from_ref(&device)).await {
        Ok(generation) => generation,
        Err(response) => return response,
    };

    let state = ctx.state.lock().await;
    if !state.topology_generation_is(topology_generation) {
        return authorization_conflict().into();
    }
    Response {
        status: ResponseStatus::State(state.device_state_status(&device).map(Box::new)),
    }
}

pub(super) async fn get_collection_state(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    collection: CollectionId,
) -> Response {
    let (leaves, resolved_generation) = {
        let state = ctx.state.lock().await;
        let leaves = match state.resolve_collection_leaves(&collection) {
            Ok(leaves) => leaves,
            Err(DaemonError::CollectionNotFound(_)) => Vec::new(),
            Err(error) => return error.into(),
        };
        (leaves, state.topology_generation())
    };
    let mut devices = Vec::new();
    for target in leaves {
        if !devices.contains(target.device_id()) {
            devices.push(target.device_id().clone());
        }
    }
    let topology_generation = match auth.authorize(&devices).await {
        Ok(generation) => generation,
        Err(response) => return response,
    };
    if topology_generation != resolved_generation {
        return authorization_conflict().into();
    }

    let state = ctx.state.lock().await;
    if !state.topology_generation_is(topology_generation) {
        return authorization_conflict().into();
    }
    Response {
        status: ResponseStatus::CollectionState(
            state.collection_state_status(&collection).map(Box::new),
        ),
    }
}

pub(super) async fn list_collections(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
) -> Response {
    match projected_collections(ctx, auth).await {
        Ok(collections) => Response {
            status: ResponseStatus::Collections(collections),
        },
        Err(response) => response,
    }
}

pub(super) async fn get_collection(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: CollectionId,
) -> Response {
    match projected_collections(ctx, auth).await {
        Ok(collections) => Response {
            status: ResponseStatus::CollectionInfo(
                collections
                    .into_iter()
                    .find(|collection| collection.id == id)
                    .map(Box::new),
            ),
        },
        Err(response) => response,
    }
}

async fn projected_collections(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
) -> Result<Vec<Collection>, Response> {
    let (collections, topology_generation): (Vec<_>, _) = {
        let state = ctx.state.lock().await;
        (
            state.collections().values().cloned().collect(),
            state.topology_generation(),
        )
    };
    let mut devices = Vec::new();
    for collection in &collections {
        for member in &collection.members {
            if let CollectionMember::Target(target) = member
                && !devices.contains(target.device_id())
            {
                devices.push(target.device_id().clone());
            }
        }
    }

    let (visible, filtered_generation) = auth.filter_devices(&devices).await?;
    let empty_generation = auth.authorize(&[]).await.ok();
    if filtered_generation != topology_generation
        || empty_generation.is_some_and(|generation| generation != topology_generation)
        || !auth.topology_is_current(topology_generation).await
    {
        return Err(authorization_conflict().into());
    }
    let visible: HashSet<_> = visible.into_iter().collect();
    let empty_visible = empty_generation.is_some();
    CollectionGraph::new(&collections)
        .project(
            |target| visible.contains(target.device_id()),
            |_| empty_visible,
        )
        .map_err(|error| DaemonError::Internal(error.to_string()).into())
}

fn authorization_conflict() -> DaemonError {
    DaemonError::AuthorizationConflict {
        reason: "device topology changed after authorization; retry the request".to_owned(),
    }
}
