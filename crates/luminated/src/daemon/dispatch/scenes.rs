// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Persistent scene authoring, capture, inspection, and immediate application.

use std::collections::HashSet;

use luminate_core::collection::OwnerIdentity;
use luminate_core::device::DeviceId;
use luminate_core::effect::Effect;
use luminate_core::scene::{Scene, SceneBinding, SceneCaptureMode, SceneId, SceneTargetState};
use luminate_core::state::EmissionState;
use luminate_core::target::TargetId;
use luminate_plugin_api::PluginUpdateOperation;

use super::super::authz::RequestAuthorizer;
use super::super::{
    Arc, DaemonError, DaemonState, Decision, Mutex, Operation, PluginManager, Response,
    ResponseStatus,
};
use super::{DispatchContext, mutate};
use crate::state::target_state::TargetState;

fn binding_devices(bindings: &[SceneBinding]) -> Vec<DeviceId> {
    bindings
        .iter()
        .map(|binding| binding.target().device_id().clone())
        .collect()
}

pub(super) async fn list_scenes(
    ctx: &DispatchContext<'_>,
    _auth: &RequestAuthorizer<'_>,
) -> Response {
    let scenes = ctx
        .state
        .lock()
        .await
        .scenes()
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let mut visible = Vec::new();
    for scene in scenes {
        if scene_visible(ctx, &scene).await {
            visible.push(scene);
        }
    }
    visible.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    Response {
        status: ResponseStatus::Scenes(visible),
    }
}

pub(super) async fn get_scene(
    ctx: &DispatchContext<'_>,
    _auth: &RequestAuthorizer<'_>,
    id: SceneId,
) -> Response {
    let scene = ctx.state.lock().await.scene(&id).cloned();
    // Do not reveal whether a scene exists when its bound devices are not
    // visible to the caller.
    let scene = match scene {
        Some(scene) if scene_visible(ctx, &scene).await => Some(Box::new(scene)),
        Some(_) | None => None,
    };
    Response {
        status: ResponseStatus::SceneInfo(scene),
    }
}

async fn scene_visible(ctx: &DispatchContext<'_>, scene: &Scene) -> bool {
    let auth = RequestAuthorizer::new(
        ctx.state,
        ctx.plugin_manager.as_ref(),
        ctx.policy,
        ctx.principal,
        Operation::Observe,
    );
    auth.authorize(&binding_devices(&scene.bindings))
        .await
        .is_ok()
}

pub(super) async fn create_scene(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    name: String,
    description: Option<String>,
    bindings: Vec<SceneBinding>,
) -> Response {
    let devices = binding_devices(&bindings);
    let generation = match auth.authorize(&devices).await {
        Ok(generation) => generation,
        Err(response) => return response,
    };
    let owner = OwnerIdentity::from(ctx.principal);
    let outcome = ctx
        .mutations
        .execute_authorized(
            devices,
            generation,
            Box::new(move |state| {
                state
                    .blocking_lock()
                    .create_scene(name, description, owner, bindings)
            }),
        )
        .await;
    if outcome.is_ok() {
        let _ = ctx
            .mutations
            .events
            .send(luminate_protocol::Event::ScenesChanged);
    }
    scene_result(outcome)
}

pub(super) async fn capture_scene(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    name: String,
    description: Option<String>,
    mode: SceneCaptureMode,
    targets: Vec<TargetId>,
) -> Response {
    let selected = if targets.is_empty()
        && let SceneCaptureMode::DynamicCollectionMembers { collection } = &mode
    {
        match ctx.state.lock().await.resolve_collection_leaves(collection) {
            Ok(leaves) => leaves,
            Err(error) => return error.into(),
        }
    } else {
        targets.clone()
    };
    let devices = selected
        .iter()
        .map(|target| target.device_id().clone())
        .collect::<Vec<_>>();
    let generation = match auth.authorize(&devices).await {
        Ok(generation) => generation,
        Err(response) => return response,
    };
    let owner = OwnerIdentity::from(ctx.principal);
    let outcome = ctx
        .mutations
        .execute_authorized(
            devices,
            generation,
            Box::new(move |state| {
                let mut state = state.blocking_lock();
                let bindings = state.capture_bindings(&mode, &targets)?;
                state.create_scene(name, description, owner, bindings)
            }),
        )
        .await;
    if outcome.is_ok() {
        let _ = ctx
            .mutations
            .events
            .send(luminate_protocol::Event::ScenesChanged);
    }
    scene_result(outcome)
}

#[allow(
    clippy::too_many_arguments,
    reason = "the handler mirrors the revisioned wire request fields"
)]
pub(super) async fn replace_scene(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: SceneId,
    expected_revision: u64,
    name: String,
    description: Option<String>,
    bindings: Vec<SceneBinding>,
) -> Response {
    if let Err(error) = authorize_scene_owner(ctx, &id).await {
        return error.into();
    }
    let devices = binding_devices(&bindings);
    let generation = match auth.authorize(&devices).await {
        Ok(generation) => generation,
        Err(response) => return response,
    };
    let outcome = ctx
        .mutations
        .execute_authorized(
            devices,
            generation,
            Box::new(move |state| {
                state.blocking_lock().replace_scene(
                    &id,
                    expected_revision,
                    name,
                    description,
                    bindings,
                )
            }),
        )
        .await;
    if outcome.is_ok() {
        let _ = ctx
            .mutations
            .events
            .send(luminate_protocol::Event::ScenesChanged);
    }
    scene_result(outcome)
}

pub(super) async fn recapture_scene(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: SceneId,
    expected_revision: u64,
    mode: SceneCaptureMode,
    targets: Vec<TargetId>,
) -> Response {
    if let Err(error) = authorize_scene_owner(ctx, &id).await {
        return error.into();
    }
    let selected = if targets.is_empty()
        && let SceneCaptureMode::DynamicCollectionMembers { collection } = &mode
    {
        match ctx.state.lock().await.resolve_collection_leaves(collection) {
            Ok(leaves) => leaves,
            Err(error) => return error.into(),
        }
    } else {
        targets.clone()
    };
    let devices = selected
        .iter()
        .map(|target| target.device_id().clone())
        .collect::<Vec<_>>();
    let generation = match auth.authorize(&devices).await {
        Ok(generation) => generation,
        Err(response) => return response,
    };
    let outcome = ctx
        .mutations
        .execute_authorized(
            devices,
            generation,
            Box::new(move |state| {
                let mut state = state.blocking_lock();
                let bindings = state.capture_bindings(&mode, &targets)?;
                let scene = state
                    .scene(&id)
                    .cloned()
                    .ok_or_else(|| DaemonError::SceneNotFound(id.clone()))?;
                state.replace_scene(
                    &id,
                    expected_revision,
                    scene.name,
                    scene.description,
                    bindings,
                )
            }),
        )
        .await;
    if outcome.is_ok() {
        let _ = ctx
            .mutations
            .events
            .send(luminate_protocol::Event::ScenesChanged);
    }
    scene_result(outcome)
}

pub(super) async fn delete_scene(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: SceneId,
    expected_revision: u64,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    if let Err(error) = authorize_scene_owner(ctx, &id).await {
        return error.into();
    }
    let generation = ctx.state.lock().await.topology_generation();
    let response = mutate(ctx.mutations, Vec::new(), generation, move |state| {
        state
            .blocking_lock()
            .delete_scene(&id, expected_revision)
            .map(|_| ())
    })
    .await;
    if matches!(response.status, ResponseStatus::Ack) {
        let _ = ctx
            .mutations
            .events
            .send(luminate_protocol::Event::ScenesChanged);
    }
    response
}

async fn authorize_scene_owner(ctx: &DispatchContext<'_>, id: &SceneId) -> Result<(), DaemonError> {
    let owner = ctx
        .state
        .lock()
        .await
        .scene(id)
        .map(|scene| scene.owner.clone())
        .ok_or_else(|| DaemonError::SceneNotFound(id.clone()))?;
    if owner == OwnerIdentity::from(ctx.principal) {
        return Ok(());
    }
    match ctx
        .policy
        .authorize(ctx.principal, Operation::AdministerScenes, &[])
        .await
    {
        Decision::Allow => Ok(()),
        Decision::Deny { .. } => Err(DaemonError::SceneNotOwned(id.clone())),
    }
}

pub(super) async fn apply_scene(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: SceneId,
    authorized_targets: Option<Vec<TargetId>>,
) -> Response {
    let mut targets = {
        let state = ctx.state.lock().await;
        let Some(scene) = state.scene(&id) else {
            return DaemonError::SceneNotFound(id).into();
        };
        let mut targets = Vec::new();
        for binding in &scene.bindings {
            let applies = match binding.dynamic_collection() {
                None => true,
                Some(collection) => match state.resolve_collection_leaves(collection) {
                    Ok(leaves) => leaves.contains(binding.target()),
                    Err(_) => false,
                },
            };
            if applies && state.capabilities_for_target(binding.target()).is_some() {
                targets.push((binding.target().clone(), binding.state().clone()));
            }
        }
        targets
    };
    targets.sort_by(|(left, _), (right, _)| format!("{left:?}").cmp(&format!("{right:?}")));
    if let Some(authorized_targets) = &authorized_targets {
        targets.retain(|(target, _)| authorized_targets.contains(target));
    }
    let leaves = targets
        .iter()
        .map(|(target, _)| target.clone())
        .collect::<Vec<_>>();
    let (applied, denied, generation) = auth.authorize_partial(&leaves).await;
    ctx.cancel_transition_targets(&applied).await;
    let allowed = applied.iter().collect::<HashSet<_>>();
    targets.retain(|(target, _)| allowed.contains(target));

    let mut operations = build_operations(&targets);
    {
        let state = ctx.state.lock().await;
        for (target, operation) in &mut operations {
            if let TargetState::AppearanceSlots(values) = operation {
                match state.validate_appearance_slot_values(target, values) {
                    Ok(resolved) => *values = resolved,
                    Err(error) => return error.into(),
                }
            } else if let Err(error) = state.validate_target_state(target, operation) {
                return error.into();
            }
        }
    }
    let devices = applied
        .iter()
        .map(|target| target.device_id().clone())
        .collect();
    let plugin_manager = Arc::clone(ctx.plugin_manager);
    let applied_result = applied.clone();
    let outcome = ctx
        .mutations
        .execute_authorized(
            devices,
            generation,
            Box::new(move |state| apply_operations(state, &plugin_manager, &operations)),
        )
        .await;
    match outcome {
        Ok(()) => Response {
            status: ResponseStatus::SceneApplied {
                applied: applied_result,
                denied,
            },
        },
        Err(error) => error.into(),
    }
}

fn build_operations(targets: &[(TargetId, SceneTargetState)]) -> Vec<(TargetId, TargetState)> {
    // Establish appearance and brightness before applying final emission.
    // Turning emission on reapplies the retained appearance because the
    // target may currently be dark.
    let mut operations = Vec::new();
    for (target, state) in targets {
        if let Some(values) = &state.appearance_slots {
            operations.push((target.clone(), TargetState::AppearanceSlots(values.clone())));
        }
    }
    for (target, state) in targets {
        if let Some(appearance) = &state.appearance {
            operations.push((target.clone(), TargetState::Effect(appearance.clone())));
        }
    }
    for (target, state) in targets {
        if let Some(brightness) = state.brightness {
            operations.push((target.clone(), TargetState::Brightness(brightness)));
        }
    }
    for (target, state) in targets {
        match state.emission {
            Some(EmissionState::Dark) => {
                operations.push((target.clone(), TargetState::Effect(Effect::Off)));
            }
            Some(EmissionState::Emitting) => {
                if let Some(appearance) = &state.appearance {
                    operations.push((target.clone(), TargetState::Effect(appearance.clone())));
                }
            }
            None => {}
        }
    }
    operations
}

fn apply_operations(
    state: &Arc<Mutex<DaemonState>>,
    plugin_manager: &PluginManager,
    operations: &[(TargetId, TargetState)],
) -> Result<(), DaemonError> {
    let resolved = resolve_operations(state, operations)?;
    let outcomes = plugin_manager.apply_batch_parallel(&resolved);
    commit_operation_outcomes(state, operations, outcomes)
}

fn commit_operation_outcomes(
    state: &Arc<Mutex<DaemonState>>,
    operations: &[(TargetId, TargetState)],
    outcomes: Vec<Result<(), DaemonError>>,
) -> Result<(), DaemonError> {
    let mut applied_targets = Vec::new();
    let mut failures = Vec::new();
    let mut first_error = None;

    for ((target, operation), outcome) in operations.iter().zip(outcomes) {
        if let Err(error) = outcome {
            failures.push(format!("{target:?}: {error}"));
            if first_error.is_none() {
                first_error = Some(error);
            }
            continue;
        }

        match operation {
            TargetState::Effect(effect) => state
                .blocking_lock()
                .set_effect(target.clone(), effect.clone())?,
            TargetState::Brightness(value) => state
                .blocking_lock()
                .set_brightness(target.clone(), *value)?,
            TargetState::AppearanceSlots(values) => state
                .blocking_lock()
                .set_appearance_slots(target.clone(), values.clone())?,
            TargetState::Clear => state.blocking_lock().clear_target(target)?,
        }
        if !applied_targets.contains(target) {
            applied_targets.push(target.clone());
        }
    }

    if failures.is_empty() {
        return Ok(());
    }
    if applied_targets.is_empty() {
        return Err(first_error.unwrap_or_else(|| {
            DaemonError::Internal("scene application failed without a diagnostic".to_owned())
        }));
    }

    Err(DaemonError::PartialMutation {
        diagnostic: format!(
            "scene applied updates to {} target(s), but {} operation(s) failed: {}",
            applied_targets.len(),
            failures.len(),
            failures.join("; ")
        ),
        applied_targets,
    })
}

fn resolve_operations(
    state: &Arc<Mutex<DaemonState>>,
    operations: &[(TargetId, TargetState)],
) -> Result<Vec<(TargetId, PluginUpdateOperation)>, DaemonError> {
    let daemon_state = state.blocking_lock();
    operations
        .iter()
        .map(|(target, operation)| {
            let operation = match operation {
                TargetState::Effect(Effect::Static { colour }) => {
                    PluginUpdateOperation::SetEffect {
                        effect: Effect::Static {
                            colour: daemon_state.resolve_colour(target, colour)?,
                        },
                    }
                }
                TargetState::Effect(effect) => PluginUpdateOperation::SetEffect {
                    effect: effect.clone(),
                },
                TargetState::Brightness(value) => {
                    PluginUpdateOperation::SetBrightness { value: *value }
                }
                TargetState::AppearanceSlots(values) => PluginUpdateOperation::SetAppearanceSlots {
                    values: values.clone(),
                },
                TargetState::Clear => PluginUpdateOperation::Clear,
            };
            Ok((target.clone(), operation))
        })
        .collect()
}

#[cfg(test)]
#[path = "scenes_tests.rs"]
mod tests;

fn scene_result(outcome: Result<Scene, DaemonError>) -> Response {
    match outcome {
        Ok(scene) => Response {
            status: ResponseStatus::Scene(Box::new(scene)),
        },
        Err(error) => error.into(),
    }
}
