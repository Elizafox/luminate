// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Daemon and plugin administration requests.
//!
//! These change the shape of the daemon rather than the appearance of
//! hardware: re-reading a device's live state, forgetting a withdrawn device,
//! rescanning, and unloading or reloading a plugin. The three plugin-level
//! operations run on a blocking thread, since a plugin host round trip can
//! outlast a reactor tick, and commit any resulting topology change before
//! acknowledging.

use super::super::authz::RequestAuthorizer;
use super::super::{
    Arc, DaemonError, Event, RescanReason, Response, ResponseStatus, TargetId, device, slice, task,
};
use super::{DispatchContext, mutate};

pub(super) async fn refresh_state(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    device: device::DeviceId,
) -> Response {
    let topology_generation = match auth.authorize(slice::from_ref(&device)).await {
        Ok(generation) => generation,
        Err(response) => return response,
    };

    let manager = Arc::clone(ctx.plugin_manager);
    mutate(
        ctx.mutations,
        vec![device.clone()],
        topology_generation,
        move |state| {
            let (request, generation) = {
                let state = state.blocking_lock();
                (state.read_request(&device), state.generation(&device))
            };

            if request.targets.is_empty() {
                return Err(DaemonError::UnsupportedCapability {
                    target: TargetId::Device(device),
                    reason: "device advertises no live state readback".to_owned(),
                });
            }

            state.blocking_lock().begin_reconciliation(&device);

            let snapshot = manager.read_state(&device, request)?;

            state
                .blocking_lock()
                .accept_snapshot(&device, generation, snapshot, false)
                .map(|_| ())
        },
    )
    .await
}

pub(super) async fn purge_withdrawn_device(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    device: device::DeviceId,
) -> Response {
    let topology_generation = match auth.authorize(slice::from_ref(&device)).await {
        Ok(generation) => generation,
        Err(response) => return response,
    };

    let logged_device = device.clone();

    mutate(
        ctx.mutations,
        vec![device.clone()],
        topology_generation,
        move |state| {
            let removed = state.blocking_lock().purge_withdrawn_device(&device)?;
            tracing::info!(device = %logged_device, removed, "purged withdrawn-device state");
            Ok(())
        },
    )
    .await
}

pub(super) async fn rescan(ctx: &DispatchContext<'_>, auth: &RequestAuthorizer<'_>) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }

    // `Ack` means the rescan was queued, not completed. Re-enumerating every
    // plugin may outlive a request timeout, so the topology coordinator owns
    // the operation asynchronously from this point onward.
    //
    // Clients that need completion or change visibility should observe the
    // resulting topology and state events.
    if ctx.rescans.request(RescanReason::Operator) {
        tracing::info!("client requested a hardware rescan");
        Response {
            status: ResponseStatus::Ack,
        }
    } else {
        Response::from(DaemonError::Internal(
            "rescan could not be scheduled: the daemon is shutting down".to_owned(),
        ))
    }
}

pub(super) async fn unload_plugin(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    name: String,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }

    let manager = Arc::clone(ctx.plugin_manager);
    let unload_name = name.clone();
    let outcome = task::spawn_blocking(move || manager.unload_plugin(&unload_name)).await;
    match outcome {
        Ok(Ok(reconciled)) => {
            if !reconciled.changed_devices.is_empty() {
                ctx.state
                    .lock()
                    .await
                    .replace_devices_preserving_withdrawn_state(reconciled.devices);
                let _ = ctx.mutations.events.send(Event::TopologyChanged {
                    devices: reconciled.changed_devices,
                });
            }
            tracing::info!(plugin = %name, "client unloaded a plugin");
            Response {
                status: ResponseStatus::Ack,
            }
        }
        Ok(Err(error)) => Response::from(error),
        Err(join_error) => Response::from(DaemonError::Internal(format!(
            "plugin unload task panicked: {join_error}"
        ))),
    }
}

pub(super) async fn reload_plugin(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    name: String,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }

    let manager = Arc::clone(ctx.plugin_manager);
    let reload_name = name.clone();
    let outcome = task::spawn_blocking(move || manager.reload_plugin(&reload_name)).await;
    match outcome {
        Ok(Ok(reconciled)) => {
            if !reconciled.changed_devices.is_empty() {
                ctx.state
                    .lock()
                    .await
                    .replace_devices_preserving_withdrawn_state(reconciled.devices);
                let _ = ctx.mutations.events.send(Event::TopologyChanged {
                    devices: reconciled.changed_devices,
                });
            }
            // The topology commit above makes the reloaded plugin's
            // devices visible immediately. A full rescan additionally
            // drives reappearance reconciliation (hardware state
            // restore) the same way resume-from-suspend does, since a
            // reload can hand back identical topology while still
            // needing its hardware state re-applied.
            if !ctx.rescans.request(RescanReason::Operator) {
                tracing::warn!(
                    plugin = %name,
                    "plugin reloaded but the follow-up rescan could not be scheduled: the daemon is shutting down"
                );
            }
            tracing::info!(plugin = %name, "client reloaded a plugin");
            Response {
                status: ResponseStatus::Ack,
            }
        }
        Ok(Err(error)) => Response::from(error),
        Err(join_error) => Response::from(DaemonError::Internal(format!(
            "plugin reload task panicked: {join_error}"
        ))),
    }
}
