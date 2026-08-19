// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Frame streaming, including both shared-memory fast paths.
//!
//! Frame uploads are hardware-only mutations. Streamed pixels are neither
//! persisted nor reflected in `GetState`'s `Appearance` or `Brightness`
//! facets, and individual uploads do not emit state-change events.
//!
//! `DaemonState` tracks only the stream's operational bookkeeping:
//! generation, sequence, and rate limiting. The begin and end transitions
//! emit `StateChanged` so subscribers can observe
//! `EffectiveAppearance::Streaming` becoming active or inactive.
//!
//! Ending a stream is deliberately not re-authorized: only the connection
//! that created a stream can end it, and a client must always be able to
//! relinquish its own target. This mirrors the unconditional cleanup that
//! runs when a connection drops.

use luminate_core::frame::FrameEnvelope;

use super::super::authz::RequestAuthorizer;
use super::super::{Arc, DaemonError, Event, Response, ResponseStatus, TargetId, slice, sync};
use super::DispatchContext;

pub(super) async fn begin_frame_stream(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    target: TargetId,
) -> Response {
    let topology_generation = match auth.authorize(slice::from_ref(target.device_id())).await {
        Ok(generation) => generation,
        Err(response) => return response,
    };
    ctx.cancel_transition_targets(slice::from_ref(&target))
        .await;
    let mut daemon_state = ctx.state.lock().await;
    if !daemon_state.topology_generation_is(topology_generation) {
        return DaemonError::AuthorizationConflict {
            reason: "device topology changed after authorization; retry the request".to_owned(),
        }
        .into();
    }
    match daemon_state.begin_frame_stream(&target) {
        Ok(generation) => {
            // Negotiate the shared-memory transport before announcing the stream as
            // active. This is best-effort: unsupported hardware, `prefer_shm` being
            // disabled, or any negotiation failure leaves the stream on the ordinary
            // pipe, which is already the unconditional fallback for frame uploads.
            let shm_capability = ctx
                .prefer_shm
                .then(|| daemon_state.capabilities_for_target(&target))
                .flatten()
                .and_then(|capabilities| capabilities.frame_upload.as_ref())
                .and_then(|frame_upload| frame_upload.shm.clone());

            ctx.owned_streams
                .lock()
                .expect("lock poisoned")
                .push(target.clone());

            drop(daemon_state);

            if let Some(capability) = shm_capability {
                let plugin_manager = Arc::clone(ctx.plugin_manager);
                let shm_target = target.clone();
                let device = target.device_id().clone();
                let _ = ctx
                    .mutations
                    .execute_hardware_only_authorized(
                        vec![device],
                        topology_generation,
                        Box::new(move || {
                            let _ = plugin_manager.begin_shm_stream(
                                &shm_target,
                                &capability,
                                generation,
                            );
                            Ok(())
                        }),
                    )
                    .await;
            }

            let _ = ctx.mutations.events.send(Event::StateChanged {
                devices: vec![target.device_id().clone()],
            });

            Response {
                status: ResponseStatus::FrameStreamStarted { generation },
            }
        }
        Err(error) => error.into(),
    }
}

pub(super) async fn upload_frame(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    target: TargetId,
    envelope: FrameEnvelope,
) -> Response {
    let authorized_generation = match auth.authorize(slice::from_ref(target.device_id())).await {
        Ok(generation) => generation,
        Err(response) => return response,
    };
    let topology_generation = match ctx
        .state
        .lock()
        .await
        .frame_stream_topology_generation(&target, envelope.generation)
    {
        Ok(generation) if generation == authorized_generation => generation,
        Ok(_) => {
            return DaemonError::AuthorizationConflict {
                reason: "frame stream authorization no longer matches the device topology"
                    .to_owned(),
            }
            .into();
        }
        Err(error) => return error.into(),
    };
    let sequence = envelope.sequence;
    let dropped = Arc::new(sync::atomic::AtomicBool::new(false));
    let dropped_in_job = Arc::clone(&dropped);
    let state = Arc::clone(ctx.state);
    let plugin_manager = Arc::clone(ctx.plugin_manager);
    let device = target.device_id().clone();
    let outcome = ctx
        .mutations
        .execute_hardware_only_authorized(
            vec![device],
            topology_generation,
            Box::new(move || {
                let forwarded = plugin_manager.apply_one_frame(&state, &target, &envelope)?;
                if !forwarded {
                    dropped_in_job.store(true, sync::atomic::Ordering::Relaxed);
                }
                Ok(())
            }),
        )
        .await;
    match outcome {
        Ok(()) => Response {
            status: ResponseStatus::FrameAck {
                sequence,
                dropped: dropped.load(sync::atomic::Ordering::Relaxed),
            },
        },
        Err(error) => error.into(),
    }
}

pub(super) async fn end_frame_stream(
    ctx: &DispatchContext<'_>,
    target: TargetId,
    generation: u32,
) -> Response {
    ctx.state.lock().await.end_frame_stream(&target, generation);

    ctx.owned_streams
        .lock()
        .expect("lock poisoned")
        .retain(|owned| owned != &target);

    if ctx.plugin_manager.has_active_shm_stream(&target) {
        let plugin_manager = Arc::clone(ctx.plugin_manager);
        let shm_target = target.clone();
        let device = target.device_id().clone();
        let _ = ctx
            .mutations
            .execute_hardware_only(
                vec![device],
                Box::new(move || {
                    plugin_manager.end_shm_stream(&shm_target, generation);
                    Ok(())
                }),
            )
            .await;
    }

    let _ = ctx.mutations.events.send(Event::StateChanged {
        devices: vec![target.device_id().clone()],
    });

    Response {
        status: ResponseStatus::Ack,
    }
}

/// Explicit opt-in fast path, a new client call rather than a transparent
/// upgrade to `BeginFrameStream`: unlike `BeginFrameStream`'s best-effort SHM
/// negotiation, every failure reason here collapses to the same `Unsupported`
/// outcome rather than a distinct error, since the caller's uniform response
/// is always "fall back to `BeginFrameStream`" regardless of which
/// precondition failed.
pub(super) async fn begin_shm_frame_stream(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    target: TargetId,
) -> Response {
    let topology_generation = match auth.authorize(slice::from_ref(target.device_id())).await {
        Ok(generation) => generation,
        Err(response) => return response,
    };
    ctx.cancel_transition_targets(slice::from_ref(&target))
        .await;

    if !ctx.prefer_client_shm || !ctx.principal.is_same_user_as_daemon() {
        return unsupported_shm_response(&target);
    }

    let mut daemon_state = ctx.state.lock().await;
    if !daemon_state.topology_generation_is(topology_generation) {
        return DaemonError::AuthorizationConflict {
            reason: "device topology changed after authorization; retry the request".to_owned(),
        }
        .into();
    }

    let Some(capability) = daemon_state
        .capabilities_for_target(&target)
        .and_then(|capabilities| capabilities.frame_upload.as_ref())
        .and_then(|frame_upload| frame_upload.shm.clone())
    else {
        return unsupported_shm_response(&target);
    };

    // A conflicting stream is reported as `Unsupported` regardless of whether
    // it was created by `BeginShmFrameStream` or `BeginFrameStream`. Both
    // paths share the same per-target stream bookkeeping, so ownership is
    // deliberately treated as an implementation detail rather than something
    // exposed through this API.
    let Ok(generation) = daemon_state.begin_frame_stream(&target) else {
        drop(daemon_state);
        return unsupported_shm_response(&target);
    };

    drop(daemon_state);

    let outcome = ctx.plugin_manager.begin_shm_client_stream(
        ctx.plugin_manager,
        ctx.state,
        &target,
        &capability,
        generation,
    );

    let ready = match outcome {
        Ok(Some(ready)) => ready,
        Ok(None) => {
            ctx.state.lock().await.end_frame_stream(&target, generation);
            return unsupported_shm_response(&target);
        }
        Err(error) => {
            tracing::warn!(
                target = ?target,
                %error,
                "negotiating a client-published shared-memory stream failed"
            );
            ctx.state.lock().await.end_frame_stream(&target, generation);
            return unsupported_shm_response(&target);
        }
    };

    ctx.owned_streams
        .lock()
        .expect("lock poisoned")
        .push(target.clone());

    // Best-effort, matching `BeginFrameStream`: the client → daemon shared-
    // memory leg is already established; this attempts to add the
    // daemon → plugin leg so the relay remains zero-copy end to end.
    //
    // If negotiation fails, only the second hop falls back to the ordinary
    // pipe. `apply_one_frame` already handles that path transparently.
    if ctx.prefer_shm {
        let plugin_manager = Arc::clone(ctx.plugin_manager);
        let shm_target = target.clone();
        let device = target.device_id().clone();
        let _ = ctx
            .mutations
            .execute_hardware_only_authorized(
                vec![device],
                topology_generation,
                Box::new(move || {
                    let _ = plugin_manager.begin_shm_stream(&shm_target, &capability, generation);
                    Ok(())
                }),
            )
            .await;
    }

    let _ = ctx.mutations.events.send(Event::StateChanged {
        devices: vec![target.device_id().clone()],
    });
    Response {
        status: ResponseStatus::ShmFrameStreamReady {
            generation,
            service_name: ready.service_name,
            event_service_name: ready.event_service_name,
            pixel_format: ready.pixel_format,
            stream_nonce: ready.stream_nonce,
            segment_bytes: ready.segment_bytes,
        },
    }
}

pub(super) async fn end_shm_frame_stream(
    ctx: &DispatchContext<'_>,
    target: TargetId,
    generation: u32,
) -> Response {
    ctx.state.lock().await.end_frame_stream(&target, generation);
    ctx.owned_streams
        .lock()
        .expect("lock poisoned")
        .retain(|owned| owned != &target);
    if ctx.plugin_manager.has_active_shm_stream(&target) {
        let plugin_manager = Arc::clone(ctx.plugin_manager);
        let shm_target = target.clone();
        let device = target.device_id().clone();
        let _ = ctx
            .mutations
            .execute_hardware_only(
                vec![device],
                Box::new(move || {
                    plugin_manager.end_shm_stream(&shm_target, generation);
                    Ok(())
                }),
            )
            .await;
    }
    if ctx
        .plugin_manager
        .end_shm_client_stream(&target, generation)
    {
        let _ = ctx.mutations.events.send(Event::StateChanged {
            devices: vec![target.device_id().clone()],
        });
    }
    Response {
        status: ResponseStatus::Ack,
    }
}

/// Builds the uniform `Unsupported` response every `BeginShmFrameStream`
/// rejection reason collapses to.
fn unsupported_shm_response(target: &TargetId) -> Response {
    Response::from(DaemonError::UnsupportedCapability {
        target: target.clone(),
        reason: "client-published shared-memory frame streaming is not available for this \
                 connection or target"
            .to_owned(),
    })
}
