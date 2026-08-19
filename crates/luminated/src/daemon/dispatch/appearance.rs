// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Appearance and persistence mutations.
//!
//! Every request here accepts a [`Selector`], so each handler has two shapes:
//! a single target, which authorizes once and mutates directly, and a
//! collection, which resolves to leaves and applies best-effort under
//! per-leaf authorization. The collection branches converge on
//! [`dispatch_collection_mutation`] so that resolve-authorize-apply logic
//! exists once.
//!
//! Restoration is the exception: it resolves every target's retained
//! appearance up front, before the first hardware write, so an unusable
//! member fails the whole request instead of leaving hardware half-updated.

use super::super::authz::RequestAuthorizer;
use super::super::executor::apply_target_state;
use super::super::reconciliation::persistence_requirement_for_target;
use super::super::{
    AppearanceState, Arc, CollectionId, DaemonError, DaemonState, Effect, Mutex,
    PersistenceRequirement, PluginManager, Response, Selector, TargetId, TargetState,
    UnsupportedPolicy, slice,
};
use super::{DispatchContext, mutate, mutate_collection};

pub(super) async fn set_appearance_slots(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    request: luminate_protocol::SetAppearanceSlotsRequest,
) -> Response {
    let target = request.target;
    let devices = vec![target.device_id().clone()];
    let topology_generation = match auth.authorize(&devices).await {
        Ok(generation) => generation,
        Err(response) => return response,
    };
    ctx.cancel_transition_targets(slice::from_ref(&target))
        .await;
    let plugin_manager = Arc::clone(ctx.plugin_manager);

    mutate(ctx.mutations, devices, topology_generation, move |state| {
        let values = state
            .blocking_lock()
            .validate_appearance_slot_values(&target, &request.values)?;
        plugin_manager.apply_appearance_slots(&target, &values)?;
        state.blocking_lock().set_appearance_slots(target, values)
    })
    .await
}

pub(super) async fn set_effect(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    request: luminate_protocol::SetEffectRequest,
) -> Response {
    match request.selector {
        Selector::Collection(id) => {
            let policy = request
                .on_unsupported
                .unwrap_or(ctx.default_unsupported_policy);

            dispatch_collection_mutation(ctx, auth, id, TargetState::Effect(request.effect), policy)
                .await
        }
        Selector::Targets(targets) => {
            let policy = request
                .on_unsupported
                .unwrap_or(ctx.default_unsupported_policy);
            dispatch_resolved_mutation(
                ctx,
                auth,
                targets,
                TargetState::Effect(request.effect),
                policy,
            )
            .await
        }
        Selector::Target(target) => {
            let devices = vec![target.device_id().clone()];
            let topology_generation = match auth.authorize(&devices).await {
                Ok(generation) => generation,
                Err(response) => return response,
            };
            ctx.cancel_transition_targets(slice::from_ref(&target))
                .await;
            let plugin_manager = Arc::clone(ctx.plugin_manager);

            mutate(ctx.mutations, devices, topology_generation, move |state| {
                let applied_effect = match &request.effect {
                    Effect::Static { colour } => Effect::Static {
                        colour: state.blocking_lock().resolve_colour(&target, colour)?,
                    },
                    effect @ (Effect::Off
                    | Effect::Breathe { .. }
                    | Effect::Pulse { .. }
                    | Effect::Strobe { .. }
                    | Effect::Scanner { .. }
                    | Effect::Morph { .. }
                    | Effect::Spectrum { .. }
                    | Effect::Rainbow { .. }
                    | Effect::Hardware { .. }) => {
                        state.blocking_lock().validate_effect(&target, effect)?;
                        effect.clone()
                    }
                };
                plugin_manager.apply_effect(&target, &applied_effect)?;
                state.blocking_lock().set_effect(target, request.effect)
            })
            .await
        }
    }
}

pub(super) async fn set_brightness(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    request: luminate_protocol::SetBrightnessRequest,
) -> Response {
    match request.target {
        Selector::Collection(id) => {
            let policy = request
                .on_unsupported
                .unwrap_or(ctx.default_unsupported_policy);

            dispatch_collection_mutation(
                ctx,
                auth,
                id,
                TargetState::Brightness(request.value),
                policy,
            )
            .await
        }
        Selector::Targets(targets) => {
            let policy = request
                .on_unsupported
                .unwrap_or(ctx.default_unsupported_policy);
            dispatch_resolved_mutation(
                ctx,
                auth,
                targets,
                TargetState::Brightness(request.value),
                policy,
            )
            .await
        }
        Selector::Target(target) => {
            let devices = vec![target.device_id().clone()];
            let topology_generation = match auth.authorize(&devices).await {
                Ok(generation) => generation,
                Err(response) => return response,
            };
            ctx.cancel_transition_targets(slice::from_ref(&target))
                .await;
            let plugin_manager = Arc::clone(ctx.plugin_manager);

            mutate(ctx.mutations, devices, topology_generation, move |state| {
                state
                    .blocking_lock()
                    .validate_brightness(&target, request.value)?;
                plugin_manager.apply_brightness(&target, request.value)?;
                state.blocking_lock().set_brightness(target, request.value)
            })
            .await
        }
    }
}

pub(super) async fn restore_appearance_request(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    target: Selector,
) -> Response {
    match target {
        Selector::Collection(id) => dispatch_collection_appearance_restoration(ctx, auth, id).await,
        Selector::Targets(targets) => {
            dispatch_resolved_appearance_restoration(ctx, auth, targets).await
        }
        Selector::Target(target) => {
            let devices = vec![target.device_id().clone()];
            let topology_generation = match auth.authorize(&devices).await {
                Ok(generation) => generation,
                Err(response) => return response,
            };
            ctx.cancel_transition_targets(slice::from_ref(&target))
                .await;
            let plugin_manager = Arc::clone(ctx.plugin_manager);

            mutate(ctx.mutations, devices, topology_generation, move |state| {
                restore_appearance(state, &plugin_manager, &target)
            })
            .await
        }
    }
}

pub(super) async fn clear_target(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    target: Selector,
) -> Response {
    match target {
        Selector::Collection(id) => {
            dispatch_collection_mutation(ctx, auth, id, TargetState::Clear, UnsupportedPolicy::Skip)
                .await
        }
        Selector::Targets(targets) => {
            dispatch_resolved_mutation(
                ctx,
                auth,
                targets,
                TargetState::Clear,
                UnsupportedPolicy::Skip,
            )
            .await
        }
        Selector::Target(target) => {
            let devices = vec![target.device_id().clone()];
            let topology_generation = match auth.authorize(&devices).await {
                Ok(generation) => generation,
                Err(response) => return response,
            };
            ctx.cancel_transition_targets(slice::from_ref(&target))
                .await;
            let plugin_manager = Arc::clone(ctx.plugin_manager);

            mutate(ctx.mutations, devices, topology_generation, move |state| {
                state.blocking_lock().ensure_target_exists(&target)?;
                plugin_manager.apply_clear(&target)?;
                state.blocking_lock().clear_target(&target)
            })
            .await
        }
    }
}

pub(super) async fn save_current(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    target: Selector,
) -> Response {
    match target {
        Selector::Collection(id) => {
            let leaves = match ctx.state.lock().await.resolve_collection_leaves(&id) {
                Ok(leaves) => leaves,
                Err(error) => return error.into(),
            };
            let (applied, denied, topology_generation) = auth.authorize_partial(&leaves).await;
            let devices: Vec<_> = applied
                .iter()
                .map(|target| target.device_id().clone())
                .collect();
            let plugin_manager = Arc::clone(ctx.plugin_manager);
            let applied_for_job = applied.clone();

            mutate_collection(
                ctx.mutations,
                devices,
                topology_generation,
                applied,
                denied,
                move |state| save_current_many(state, &plugin_manager, &applied_for_job),
            )
            .await
        }
        Selector::Targets(targets) => dispatch_resolved_save(ctx, auth, targets).await,
        Selector::Target(target) => {
            let devices = vec![target.device_id().clone()];
            let topology_generation = match auth.authorize(&devices).await {
                Ok(generation) => generation,
                Err(response) => return response,
            };
            let plugin_manager = Arc::clone(ctx.plugin_manager);
            mutate(ctx.mutations, devices, topology_generation, move |state| {
                save_current_target(state, &plugin_manager, &target)
            })
            .await
        }
    }
}

fn appearance_target_state(appearance: AppearanceState) -> Option<TargetState> {
    match appearance {
        AppearanceState::Static(colour) => Some(TargetState::Effect(Effect::Static { colour })),
        AppearanceState::Effect(Effect::Off) | AppearanceState::Mixed => None,
        AppearanceState::Effect(effect) => Some(TargetState::Effect(effect)),
    }
}

/// Resolves every retained appearance before applying any hardware mutation.
///
/// A group may be addressed once when all canonical members agree and the
/// group capability can faithfully express that appearance. Otherwise each
/// canonical member is restored independently.
fn restore_appearance(
    state: &Arc<Mutex<DaemonState>>,
    plugin_manager: &PluginManager,
    target: &TargetId,
) -> Result<(), DaemonError> {
    restore_appearances(state, plugin_manager, slice::from_ref(target))
}

/// Restores each target's own retained appearance after preflighting the
/// complete set.
///
/// Resolving every operation before the first hardware write keeps a missing
/// or unusable collection member from causing an avoidable partial mutation.
pub(super) fn restore_appearances(
    state: &Arc<Mutex<DaemonState>>,
    plugin_manager: &PluginManager,
    targets: &[TargetId],
) -> Result<(), DaemonError> {
    let operations = {
        let daemon_state = state.blocking_lock();
        let mut operations = Vec::new();
        for target in targets {
            daemon_state.ensure_target_exists(target)?;

            let members = daemon_state.canonical_observation_targets(target);
            let mut target_operations = Vec::with_capacity(members.len());
            for member in &members {
                daemon_state.ensure_not_frame_streaming(member)?;
                let appearance = daemon_state
                    .retained_appearance(member)
                    .and_then(appearance_target_state)
                    .ok_or_else(|| DaemonError::UnknownState {
                        target: member.clone(),
                    })?;
                daemon_state.validate_target_state(member, &appearance)?;
                target_operations.push((member.clone(), appearance));
            }

            if matches!(target, TargetId::Group { .. }) {
                let Some((_, first)) = target_operations.first() else {
                    return Err(DaemonError::UnknownState {
                        target: target.clone(),
                    });
                };
                let uniform = target_operations
                    .iter()
                    .all(|(_, appearance)| target_states_equal(appearance, first));
                if uniform && daemon_state.validate_target_state(target, first).is_ok() {
                    operations.push((target.clone(), first.clone()));
                } else {
                    operations.extend(target_operations);
                }
            } else {
                operations.extend(target_operations);
            }
        }

        operations
    };

    let mut applied = Vec::new();
    for (operation_target, appearance) in operations {
        if let Err(error) =
            apply_target_state(state, plugin_manager, &operation_target, &appearance)
        {
            if applied.is_empty() {
                return Err(error);
            }
            return Err(DaemonError::PartialMutation {
                diagnostic: format!(
                    "restored {} target(s) before {operation_target:?} failed: {error}",
                    applied.len()
                ),
                applied_targets: applied,
            });
        }

        let mut daemon_state = state.blocking_lock();
        match appearance {
            TargetState::Effect(effect) => {
                daemon_state.set_effect(operation_target.clone(), effect)?;
            }
            TargetState::Brightness(_) | TargetState::AppearanceSlots(_) | TargetState::Clear => {
                return Err(DaemonError::Internal(
                    "appearance restoration resolved a non-appearance state".to_owned(),
                ));
            }
        }
        drop(daemon_state);
        applied.push(operation_target);
    }

    Ok(())
}

/// Authorizes and restores the retained appearance of every authorized leaf
/// in a collection.
async fn dispatch_collection_appearance_restoration(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: CollectionId,
) -> Response {
    let leaves = match ctx.state.lock().await.resolve_collection_leaves(&id) {
        Ok(leaves) => leaves,
        Err(error) => return error.into(),
    };
    dispatch_resolved_appearance_restoration(ctx, auth, leaves).await
}

async fn dispatch_resolved_appearance_restoration(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    leaves: Vec<TargetId>,
) -> Response {
    let (applied, denied, topology_generation) = auth.authorize_partial(&leaves).await;
    ctx.cancel_transition_targets(&applied).await;
    let devices = applied
        .iter()
        .map(|target| target.device_id().clone())
        .collect();
    let plugin_manager = Arc::clone(ctx.plugin_manager);
    let applied_for_job = applied.clone();

    mutate_collection(
        ctx.mutations,
        devices,
        topology_generation,
        applied,
        denied,
        move |state| restore_appearances(state, &plugin_manager, &applied_for_job),
    )
    .await
}

fn target_states_equal(left: &TargetState, right: &TargetState) -> bool {
    match (left, right) {
        (TargetState::Effect(left), TargetState::Effect(right)) => left == right,
        _ => false,
    }
}

/// Applies a `TargetState`-shaped mutation to the already ACL-authorized
/// leaf targets returned by `daemon::authz::authorize_partial`.
///
/// Collection members may themselves be sub-device targets (surfaces,
/// elements, or device-scoped groups), so each leaf target is updated via
/// [`apply_target_state`] rather than coercing the collection to whole
/// devices.
///
/// This helper performs no standing-default bookkeeping. Collections do
/// not yet have an equivalent to a location's former standing default.
/// `id` is used only for diagnostics.
fn apply_collection_mutation(
    state: &Arc<Mutex<DaemonState>>,
    plugin_manager: &PluginManager,
    id: Option<&CollectionId>,
    authorized_leaves: &[TargetId],
    target_state: &TargetState,
    policy: UnsupportedPolicy,
) -> Result<(), DaemonError> {
    let members = state.blocking_lock().prepare_collection_mutation(
        authorized_leaves,
        target_state,
        policy,
    )?;
    let mut applied = Vec::new();
    for member in &members {
        if let Err(error) = apply_target_state(state, plugin_manager, member, target_state) {
            if applied.is_empty() {
                return Err(error);
            }

            if let Err(persistence_error) = state
                .blocking_lock()
                .record_collection_partial_success(&applied, target_state)
            {
                return Err(DaemonError::PartialMutation {
                    diagnostic: format!(
                        "{} applied to {} member(s), but their intended state was rejected: {persistence_error}",
                        id.map_or("resolved target set", CollectionId::as_str),
                        applied.len()
                    ),
                    applied_targets: applied,
                });
            }

            return Err(DaemonError::PartialMutation {
                diagnostic: format!(
                    "{} applied to {} member(s) before {member:?} failed: {error}",
                    id.map_or("resolved target set", CollectionId::as_str),
                    applied.len()
                ),
                applied_targets: applied.clone(),
            });
        }
        applied.push(member.clone());
    }
    Ok(())
}

/// Authorizes and applies a `TargetState`-shaped mutation to a collection
/// selector using best-effort per-leaf authorization
/// (`daemon::authz::authorize_partial`).
///
/// Shared by the collection branches of `SetEffect`, `SetBrightness`, and
/// `ClearTarget`. Each resolves its selector to a
/// collection ID and `TargetState` before delegating here, avoiding
/// duplicated resolve-authorize-apply logic.
async fn dispatch_collection_mutation(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: CollectionId,
    target_state: TargetState,
    policy: UnsupportedPolicy,
) -> Response {
    let leaves = match ctx.state.lock().await.resolve_collection_leaves(&id) {
        Ok(leaves) => leaves,
        Err(error) => return error.into(),
    };
    dispatch_resolved_mutation_with_collection(ctx, auth, leaves, Some(id), target_state, policy)
        .await
}

async fn dispatch_resolved_mutation(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    leaves: Vec<TargetId>,
    target_state: TargetState,
    policy: UnsupportedPolicy,
) -> Response {
    dispatch_resolved_mutation_with_collection(ctx, auth, leaves, None, target_state, policy).await
}

async fn dispatch_resolved_mutation_with_collection(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    leaves: Vec<TargetId>,
    collection: Option<CollectionId>,
    target_state: TargetState,
    policy: UnsupportedPolicy,
) -> Response {
    let (applied, denied, topology_generation) = auth.authorize_partial(&leaves).await;
    ctx.cancel_transition_targets(&applied).await;
    let devices: Vec<_> = applied
        .iter()
        .map(|target| target.device_id().clone())
        .collect();
    let plugin_manager = Arc::clone(ctx.plugin_manager);
    let applied_for_job = applied.clone();

    mutate_collection(
        ctx.mutations,
        devices,
        topology_generation,
        applied,
        denied,
        move |state| {
            apply_collection_mutation(
                state,
                &plugin_manager,
                collection.as_ref(),
                &applied_for_job,
                &target_state,
                policy,
            )
        },
    )
    .await
}

async fn dispatch_resolved_save(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    leaves: Vec<TargetId>,
) -> Response {
    let (applied, denied, topology_generation) = auth.authorize_partial(&leaves).await;
    let devices = applied
        .iter()
        .map(|target| target.device_id().clone())
        .collect();
    let plugin_manager = Arc::clone(ctx.plugin_manager);
    let applied_for_job = applied.clone();

    mutate_collection(
        ctx.mutations,
        devices,
        topology_generation,
        applied,
        denied,
        move |state| save_current_many(state, &plugin_manager, &applied_for_job),
    )
    .await
}

/// Saves a target's current hardware state, matching `SaveCurrent` for a
/// single target.
///
/// Targets with write-through persistence are already durable, so this is
/// a no-op. All other targets dispatch a hardware persistence operation.
fn save_current_target(
    state: &Arc<Mutex<DaemonState>>,
    plugin_manager: &PluginManager,
    target: &TargetId,
) -> Result<(), DaemonError> {
    let persistence_required = {
        let state = state.blocking_lock();
        state.ensure_target_state_known(target)?;
        persistence_requirement_for_target(&state, target) == Some(PersistenceRequirement::Required)
    };

    // Write-through persistence is already durable: there is no pending state
    // to save. Avoid an unnecessary hardware persistence call (for example, a
    // flash or EEPROM write).
    if persistence_required {
        return Ok(());
    }

    plugin_manager.apply_save_current(target)
}

/// Fans out `SaveCurrent` across every target in a collection.
///
/// `SaveCurrent` affects only hardware persistence; it does not mutate the
/// daemon's desired state. Because there is no in-memory state to commit
/// incrementally, a failure stops the fan-out and returns the first error.
fn save_current_many(
    state: &Arc<Mutex<DaemonState>>,
    plugin_manager: &PluginManager,
    targets: &[TargetId],
) -> Result<(), DaemonError> {
    for target in targets {
        save_current_target(state, plugin_manager, target)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "appearance_tests.rs"]
mod tests;
