// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Daemon-managed transition preflight, scheduling, status, and abort.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use luminate_core::capability::{ColourCapability, ColourChannel, EffectParameter};
use luminate_core::colour::{Colour, ColourChannelValue};
use luminate_core::effect::Effect;
use luminate_core::rgb::Rgb;
use luminate_core::scene::{SceneId, SceneTargetState};
use luminate_core::state::EmissionState;
use luminate_core::target::TargetId;
use luminate_core::transition::{
    TransitionCancellation, TransitionColourInterpolation, TransitionId, TransitionOptions,
    TransitionOutcome, TransitionStatus, TransitionTargetState, ease_progress, interpolate_effect,
    interpolate_u32, snap_to_discrete_range_u32,
};
use luminate_core::util::DiscreteRange;
use luminate_protocol::{
    Response, ResponseStatus, StartTransitionRequest, TransitionDestination, TransitionSource,
};
use tokio::sync::Mutex;
use tokio::time::{Instant, sleep_until};

use super::super::authz::RequestAuthorizer;
use super::super::executor::apply_target_state;
use super::super::{DaemonError, TargetState};
use super::DispatchContext;
use crate::plugins::PluginManager;

#[derive(Clone)]
struct Endpoint {
    target: TargetId,
    appearance: Effect,
    brightness: Option<u32>,
    emission: EmissionState,
}

#[derive(Clone)]
struct TargetPlan {
    start: Endpoint,
    end: Endpoint,
    hue_maximum: u32,
}

pub(super) async fn start(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    request: StartTransitionRequest,
) -> Response {
    let (plans, source_is_scene) = match preflight(ctx, &request).await {
        Ok(preflight) => preflight,
        Err(error) => return error.into(),
    };
    let targets = plans
        .iter()
        .map(|plan| plan.start.target.clone())
        .collect::<Vec<_>>();
    let devices = targets
        .iter()
        .map(|target| target.device_id().clone())
        .collect::<Vec<_>>();
    let generation = match auth.authorize(&devices).await {
        Ok(generation) => generation,
        Err(response) => return response,
    };

    ctx.cancel_transition_targets_with_reason(&targets, TransitionCancellation::Replaced)
        .await;

    if source_is_scene {
        // A scene source describes the hardware state at time zero, not only
        // an interpolation value. Apply it before the transition is visible so
        // the first scheduled step always starts from that declared state.
        let source = plans
            .iter()
            .map(|plan| plan.start.clone())
            .collect::<Vec<_>>();
        let state = Arc::clone(ctx.state);
        let plugin_manager = Arc::clone(ctx.plugin_manager);
        if let Err(error) = ctx
            .mutations
            .execute_authorized(
                devices.clone(),
                generation,
                Box::new(move |_| apply_endpoints(&state, &plugin_manager, &source, true)),
            )
            .await
        {
            return error.into();
        }
    }

    let id = TransitionId::generate();
    let status = TransitionStatus {
        id: id.clone(),
        targets,
        elapsed_ms: 0,
        duration_ms: u64::try_from(request.options.duration().as_millis()).unwrap_or(u64::MAX),
        outcome: None,
    };
    let entry = ctx
        .mutations
        .transitions
        .insert(status.clone(), request.renewable_lease_ms);
    let _ = ctx
        .mutations
        .events
        .send(luminate_protocol::Event::TransitionsChanged {
            transitions: vec![id.clone()],
        });

    let executor = ctx.mutations.clone();
    let plugin_manager = Arc::clone(ctx.plugin_manager);
    let options = request.options;
    tokio::spawn(async move {
        run_transition(executor, plugin_manager, entry, plans, options).await;
    });

    Response {
        status: ResponseStatus::Transition(Box::new(status)),
    }
}

pub(super) async fn get(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: TransitionId,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    transition_response(ctx, &id)
}

pub(super) async fn abort(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: TransitionId,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    let Some(entry) = ctx.mutations.transitions.get(&id) else {
        return DaemonError::TransitionNotFound(id).into();
    };
    entry.request_abort(TransitionCancellation::Aborted);
    entry.wait_finished().await;
    transition_response(ctx, &id)
}

pub(super) async fn renew(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: TransitionId,
    lease_ms: u64,
) -> Response {
    if lease_ms == 0 {
        return DaemonError::TransitionImpossible(
            "renewable transition lease must be greater than zero".to_owned(),
        )
        .into();
    }
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    let Some(entry) = ctx.mutations.transitions.get(&id) else {
        return DaemonError::TransitionNotFound(id).into();
    };
    entry.renew(lease_ms);
    let _ = ctx
        .mutations
        .events
        .send(luminate_protocol::Event::TransitionsChanged {
            transitions: vec![id.clone()],
        });
    transition_response(ctx, &id)
}

fn transition_response(ctx: &DispatchContext<'_>, id: &TransitionId) -> Response {
    match ctx.mutations.transitions.get(id) {
        Some(entry) => Response {
            status: ResponseStatus::Transition(Box::new(entry.snapshot())),
        },
        None => DaemonError::TransitionNotFound(id.clone()).into(),
    }
}

async fn preflight(
    ctx: &DispatchContext<'_>,
    request: &StartTransitionRequest,
) -> Result<(Vec<TargetPlan>, bool), DaemonError> {
    let state = ctx.state.lock().await;
    let destination_sparse = match &request.destination {
        TransitionDestination::Scene(id) => resolve_scene(&state, id)?,
        TransitionDestination::TargetStates(states) => {
            normalize_target_states(states, request.authorized_targets.as_deref())?
        }
    };
    if destination_sparse.is_empty() {
        return Err(impossible("transition has no concrete destination targets"));
    }
    let destination_targets = destination_sparse.keys().cloned().collect::<HashSet<_>>();
    let source_sparse = match &request.source {
        TransitionSource::Current => HashMap::new(),
        TransitionSource::Scene(id) => {
            let source = resolve_scene(&state, id)?;
            if source.keys().cloned().collect::<HashSet<_>>() != destination_targets {
                return Err(impossible(
                    "source and destination resolve to different target sets",
                ));
            }
            source
        }
    };

    let mut plans = Vec::with_capacity(destination_sparse.len());
    let mut destinations = destination_sparse.into_iter().collect::<Vec<_>>();
    destinations.sort_by(|(left, _), (right, _)| format!("{left:?}").cmp(&format!("{right:?}")));
    for (target, destination) in destinations {
        state.ensure_target_exists(&target)?;
        let source = match source_sparse.get(&target) {
            None => state
                .fresh_current_target_state(&target)
                .map_err(|_| impossible("a required current endpoint is unknown"))?,
            Some(scene) if scene.appearance.is_some() && scene.emission.is_some() => scene.clone(),
            Some(scene) => {
                let current = state
                    .fresh_current_target_state(&target)
                    .map_err(|_| impossible("a required current endpoint is unknown"))?;
                merge(scene, &current)
            }
        };
        let destination = merge(&destination, &source);
        let start = complete(target.clone(), source)?;
        let end = complete(target, destination)?;
        let hue_maximum = hue_maximum(&state, &start)?;
        let interpolation_start = logical_appearance(&start)?;
        let interpolation_end = logical_appearance(&end)?;
        validate_colour_interpolation(
            &state,
            &start.target,
            &interpolation_start,
            request.options.colour_interpolation,
        )?;
        // Exercise a real intermediate value during preflight. Some endpoint
        // pairs are individually valid but cannot be interpolated safely.
        interpolate_effect(
            &interpolation_start,
            &interpolation_end,
            1,
            2,
            hue_maximum,
            request.options.colour_interpolation,
        )
        .map_err(|error| impossible(&error.to_string()))?;
        validate_endpoint(&state, &start)?;
        validate_endpoint(&state, &end)?;
        plans.push(TargetPlan {
            start,
            end,
            hue_maximum,
        });
    }
    plans.sort_by(|left, right| {
        format!("{:?}", left.start.target).cmp(&format!("{:?}", right.start.target))
    });
    Ok((plans, matches!(request.source, TransitionSource::Scene(_))))
}

fn validate_colour_interpolation(
    state: &super::super::DaemonState,
    target: &TargetId,
    effect: &Effect,
    interpolation: TransitionColourInterpolation,
) -> Result<(), DaemonError> {
    if interpolation != TransitionColourInterpolation::Oklab {
        return Ok(());
    }
    let Effect::Static {
        colour: Colour::Additive(_),
    } = effect
    else {
        return Ok(());
    };
    let rgb8 = state
        .capabilities_for_target(target)
        .is_some_and(|capabilities| capabilities.colour.iter().any(is_rgb8_capability));
    if rgb8 {
        Ok(())
    } else {
        Err(impossible(
            "OKLab interpolation requires an advertised 8-bit RGB encoding",
        ))
    }
}

fn is_rgb8_capability(capability: &ColourCapability) -> bool {
    let ColourCapability::Additive(channels) = capability else {
        return false;
    };
    let [red, green, blue] = channels.as_slice() else {
        return false;
    };
    red.channel == ColourChannel::Red
        && red.bits == 8
        && green.channel == ColourChannel::Green
        && green.bits == 8
        && blue.channel == ColourChannel::Blue
        && blue.bits == 8
}

fn resolve_scene(
    state: &super::super::DaemonState,
    id: &SceneId,
) -> Result<HashMap<TargetId, SceneTargetState>, DaemonError> {
    let scene = state
        .scene(id)
        .ok_or_else(|| DaemonError::SceneNotFound(id.clone()))?;
    let mut resolved = HashMap::new();
    for binding in &scene.bindings {
        let applies = match binding.dynamic_collection() {
            None => true,
            Some(collection) => state
                .resolve_collection_leaves(collection)
                .is_ok_and(|leaves| leaves.contains(binding.target())),
        };
        if applies {
            resolved.insert(binding.target().clone(), binding.state().clone());
        }
    }
    Ok(resolved)
}

fn normalize_target_states(
    states: &[TransitionTargetState],
    authorized: Option<&[TargetId]>,
) -> Result<HashMap<TargetId, SceneTargetState>, DaemonError> {
    let mut normalized = HashMap::new();
    for target_state in states {
        if authorized.is_some_and(|targets| !targets.contains(&target_state.target)) {
            continue;
        }
        validate_sparse_transition_state(&target_state.state)?;
        if normalized
            .insert(target_state.target.clone(), target_state.state.clone())
            .is_some()
        {
            return Err(impossible("transition destination repeats a target"));
        }
    }
    Ok(normalized)
}

fn validate_sparse_transition_state(state: &SceneTargetState) -> Result<(), DaemonError> {
    if state.appearance_slots.is_some() {
        return Err(impossible(
            "appearance slots cannot be interpolated by a transition",
        ));
    }
    if state.appearance.is_none() && state.brightness.is_none() && state.emission.is_none() {
        return Err(impossible(
            "transition target state must contain at least one facet",
        ));
    }
    if matches!(state.appearance, Some(Effect::Off)) {
        return Err(impossible(
            "Effect::Off is an emission state, not a retained appearance",
        ));
    }
    if state.emission == Some(EmissionState::Emitting) && state.brightness == Some(0) {
        return Err(impossible(
            "an emitting transition endpoint cannot have zero brightness",
        ));
    }
    Ok(())
}

fn merge(sparse: &SceneTargetState, fallback: &SceneTargetState) -> SceneTargetState {
    SceneTargetState {
        appearance: sparse
            .appearance
            .clone()
            .or_else(|| fallback.appearance.clone()),
        brightness: sparse.brightness.or(fallback.brightness),
        emission: sparse.emission.or(fallback.emission),
        appearance_slots: None,
    }
}

fn complete(target: TargetId, state: SceneTargetState) -> Result<Endpoint, DaemonError> {
    Ok(Endpoint {
        target,
        appearance: state
            .appearance
            .ok_or_else(|| impossible("transition endpoint has no retained appearance"))?,
        brightness: state.brightness,
        emission: state
            .emission
            .ok_or_else(|| impossible("transition endpoint has unknown emission"))?,
    })
}

fn validate_endpoint(
    state: &super::super::DaemonState,
    endpoint: &Endpoint,
) -> Result<(), DaemonError> {
    state.validate_target_state(
        &endpoint.target,
        &TargetState::Effect(endpoint.appearance.clone()),
    )?;
    if let Some(brightness) = endpoint.brightness {
        state.validate_target_state(&endpoint.target, &TargetState::Brightness(brightness))?;
    }
    if endpoint.emission == EmissionState::Dark {
        state.validate_target_state(&endpoint.target, &TargetState::Effect(Effect::Off))?;
    }
    Ok(())
}

fn hue_maximum(state: &super::super::DaemonState, endpoint: &Endpoint) -> Result<u32, DaemonError> {
    let Effect::Static { colour } = &endpoint.appearance else {
        return Ok(u32::MAX);
    };
    let hue_bits = state
        .capabilities_for_target(&endpoint.target)
        .and_then(|capabilities| {
            capabilities
                .colour
                .iter()
                .find_map(|capability| match (colour, capability) {
                    (Colour::Hsv { .. }, ColourCapability::Hsv { hue_bits, .. })
                    | (Colour::Hsl { .. }, ColourCapability::Hsl { hue_bits, .. }) => {
                        Some(*hue_bits)
                    }
                    _ => None,
                })
        });
    let Some(bits) = hue_bits else {
        return Ok(u32::MAX);
    };
    match bits {
        1..=31 => Ok((1_u32 << bits) - 1),
        32 => Ok(u32::MAX),
        _ => Err(impossible("target advertises an invalid hue channel width")),
    }
}

async fn run_transition(
    executor: super::super::executor::MutationExecutor,
    plugin_manager: Arc<PluginManager>,
    entry: Arc<super::super::transition::TransitionEntry>,
    plans: Vec<TargetPlan>,
    options: TransitionOptions,
) {
    let id = entry.snapshot().id;
    let devices = plans
        .iter()
        .map(|plan| plan.start.target.device_id().clone())
        .collect::<Vec<_>>();
    let started = Instant::now();
    let deadline = started + options.duration();
    let mut next = started + options.step_interval();
    let outcome = loop {
        let wake = entry
            .lease_deadline()
            .map_or(next.min(deadline), |lease| next.min(deadline).min(lease));
        tokio::select! {
            () = sleep_until(wake) => {}
            () = entry.wait_for_change() => {
                if let Some(reason) = entry.cancellation() {
                    break TransitionOutcome::Cancelled(reason);
                }
                continue;
            }
        }
        if entry
            .lease_deadline()
            .is_some_and(|lease| Instant::now() >= lease)
        {
            break TransitionOutcome::Cancelled(TransitionCancellation::AuthorizationExpired);
        }
        if let Some(reason) = entry.cancellation() {
            break TransitionOutcome::Cancelled(reason);
        }
        let elapsed = Instant::now()
            .saturating_duration_since(started)
            .min(options.duration());
        let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        let duration_ms = u64::try_from(options.duration().as_millis()).unwrap_or(u64::MAX);
        let step = match interpolate_plans(&plans, elapsed_ms, duration_ms, options) {
            Ok(step) => step,
            Err(error) => {
                break TransitionOutcome::Failed {
                    diagnostic: error.to_string(),
                    applied_targets: Vec::new(),
                };
            }
        };
        let state = Arc::clone(&executor.state);
        let plugins = Arc::clone(&plugin_manager);
        let step_for_job = step.clone();
        if let Err(error) = executor
            .execute_hardware_only(
                devices.clone(),
                Box::new(move || apply_endpoints(&state, &plugins, &step_for_job, true)),
            )
            .await
        {
            let (diagnostic, applied_targets) = transition_failure(error);
            break TransitionOutcome::Failed {
                diagnostic,
                applied_targets,
            };
        }
        entry.update_elapsed(elapsed_ms);
        let _ = executor
            .events
            .send(luminate_protocol::Event::StateChanged {
                devices: devices.clone(),
            });
        let _ = executor
            .events
            .send(luminate_protocol::Event::TransitionsChanged {
                transitions: vec![id.clone()],
            });
        if elapsed >= options.duration() {
            break TransitionOutcome::Completed;
        }
        next += options.step_interval();
        if next < Instant::now() {
            next = Instant::now() + options.step_interval();
        }
    };

    // Hardware-only steps update in-memory state without persistence. Run an
    // empty mutation through the commit path before publishing the outcome so
    // the final state is durable.
    let _ = executor
        .execute_without_notification(devices, Box::new(|_| Ok(())))
        .await;
    executor.transitions.finish(&id, outcome);
    let _ = executor
        .events
        .send(luminate_protocol::Event::TransitionsChanged {
            transitions: vec![id],
        });
}

#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "all non-partial daemon failures deliberately share the same transition outcome"
)]
fn transition_failure(error: DaemonError) -> (String, Vec<TargetId>) {
    match error {
        DaemonError::PartialMutation {
            diagnostic,
            applied_targets,
        } => (diagnostic, applied_targets),
        error => (error.to_string(), Vec::new()),
    }
}

fn interpolate_plans(
    plans: &[TargetPlan],
    numerator: u64,
    denominator: u64,
    options: TransitionOptions,
) -> Result<Vec<Endpoint>, DaemonError> {
    let (numerator, denominator) = ease_progress(options.function, numerator, denominator)
        .map_err(|error| impossible(&error.to_string()))?;
    plans
        .iter()
        .map(|plan| {
            if numerator >= denominator {
                return Ok(plan.end.clone());
            }
            let start_appearance = logical_appearance(&plan.start)?;
            let end_appearance = logical_appearance(&plan.end)?;
            let start_brightness = logical_brightness(&plan.start);
            let end_brightness = logical_brightness(&plan.end);
            Ok(Endpoint {
                target: plan.start.target.clone(),
                appearance: interpolate_effect(
                    &start_appearance,
                    &end_appearance,
                    numerator,
                    denominator,
                    plan.hue_maximum,
                    options.colour_interpolation,
                )
                .map_err(|error| impossible(&error.to_string()))?,
                brightness: start_brightness
                    .zip(end_brightness)
                    .map(|(start, end)| interpolate_u32(start, end, numerator, denominator))
                    .transpose()
                    .map_err(|error| impossible(&error.to_string()))?,
                emission: if numerator >= denominator {
                    plan.end.emission
                } else if plan.start.emission == EmissionState::Emitting
                    || plan.end.emission == EmissionState::Emitting
                {
                    EmissionState::Emitting
                } else {
                    EmissionState::Dark
                },
            })
        })
        .collect()
}

fn logical_brightness(endpoint: &Endpoint) -> Option<u32> {
    endpoint.brightness.map(|brightness| {
        if endpoint.emission == EmissionState::Dark {
            0
        } else {
            brightness
        }
    })
}

fn logical_appearance(endpoint: &Endpoint) -> Result<Effect, DaemonError> {
    if endpoint.emission == EmissionState::Emitting || endpoint.brightness.is_some() {
        return Ok(endpoint.appearance.clone());
    }
    darken_effect(&endpoint.appearance)
}

fn darken_effect(effect: &Effect) -> Result<Effect, DaemonError> {
    let dark_rgb = Rgb::new(0, 0, 0);
    Ok(match effect {
        Effect::Static { colour } => Effect::Static {
            colour: darken_colour(colour)?,
        },
        Effect::Breathe { period_ms, .. } => Effect::Breathe {
            colour: dark_rgb,
            period_ms: *period_ms,
        },
        Effect::Pulse { period_ms, .. } => Effect::Pulse {
            colour: dark_rgb,
            period_ms: *period_ms,
        },
        Effect::Strobe { period_ms, .. } => Effect::Strobe {
            colour: dark_rgb,
            period_ms: *period_ms,
        },
        Effect::Scanner { period_ms, .. } => Effect::Scanner {
            colour: dark_rgb,
            period_ms: *period_ms,
        },
        Effect::Morph { colours, period_ms } => Effect::Morph {
            colours: vec![dark_rgb; colours.len()],
            period_ms: *period_ms,
        },
        Effect::Hardware { id, arguments } if arguments.brightness.is_some() => {
            let mut arguments = arguments.clone();
            arguments.brightness = Some(0);
            Effect::Hardware {
                id: id.clone(),
                arguments,
            }
        }
        Effect::Hardware { id, arguments } if !arguments.colours.is_empty() => {
            let mut arguments = arguments.clone();
            arguments.colours.fill(dark_rgb);
            Effect::Hardware {
                id: id.clone(),
                arguments,
            }
        }
        Effect::Off
        | Effect::Spectrum { .. }
        | Effect::Rainbow { .. }
        | Effect::Hardware { .. } => {
            return Err(impossible(
                "an endpoint without independent brightness cannot be faded safely",
            ));
        }
    })
}

fn darken_colour(colour: &Colour) -> Result<Colour, DaemonError> {
    Ok(match colour {
        Colour::Additive(channels) => Colour::additive(
            channels
                .iter()
                .map(|channel| ColourChannelValue::new(channel.channel, 0))
                .collect(),
        )
        .map_err(|error| impossible(&error.to_string()))?,
        Colour::Hsv {
            hue, saturation, ..
        } => Colour::hsv(*hue, *saturation, 0),
        Colour::Hsl {
            hue, saturation, ..
        } => Colour::hsl(*hue, *saturation, 0),
        Colour::Monochrome { .. } => Colour::monochrome(0),
        Colour::Cct { .. } => {
            return Err(impossible(
                "correlated colour temperature cannot represent zero visibility",
            ));
        }
    })
}

fn apply_endpoints(
    state: &Arc<Mutex<super::super::DaemonState>>,
    plugin_manager: &PluginManager,
    endpoints: &[Endpoint],
    include_final_dark: bool,
) -> Result<(), DaemonError> {
    let mut applied = Vec::new();
    for endpoint in endpoints {
        let appearance = {
            let daemon_state = state.blocking_lock();
            snap_hardware_effect(&daemon_state, &endpoint.target, &endpoint.appearance)?
        };
        if let Err(error) = apply_target_state(
            state,
            plugin_manager,
            &endpoint.target,
            &TargetState::Effect(appearance.clone()),
        ) {
            return Err(partial_error(error, applied));
        }
        applied.push(endpoint.target.clone());
        if let Err(error) = state
            .blocking_lock()
            .set_effect(endpoint.target.clone(), appearance)
        {
            return Err(partial_error(error, applied));
        }
        if let Some(brightness) = endpoint.brightness {
            if let Err(error) = apply_target_state(
                state,
                plugin_manager,
                &endpoint.target,
                &TargetState::Brightness(brightness),
            ) {
                return Err(partial_error(error, applied));
            }
            if let Err(error) = state
                .blocking_lock()
                .set_brightness(endpoint.target.clone(), brightness)
            {
                return Err(partial_error(error, applied));
            }
        }
        if include_final_dark && endpoint.emission == EmissionState::Dark {
            if let Err(error) = apply_target_state(
                state,
                plugin_manager,
                &endpoint.target,
                &TargetState::Effect(Effect::Off),
            ) {
                return Err(partial_error(error, applied));
            }
            if let Err(error) = state
                .blocking_lock()
                .set_effect(endpoint.target.clone(), Effect::Off)
            {
                return Err(partial_error(error, applied));
            }
        }
    }
    Ok(())
}

fn partial_error(error: DaemonError, mut applied_targets: Vec<TargetId>) -> DaemonError {
    applied_targets.sort_by(|left, right| format!("{left:?}").cmp(&format!("{right:?}")));
    applied_targets.dedup();
    if applied_targets.is_empty() {
        error
    } else {
        DaemonError::PartialMutation {
            diagnostic: error.to_string(),
            applied_targets,
        }
    }
}

fn snap_hardware_effect(
    state: &super::super::DaemonState,
    target: &TargetId,
    effect: &Effect,
) -> Result<Effect, DaemonError> {
    let Effect::Hardware { id, arguments } = effect else {
        return Ok(effect.clone());
    };
    let descriptor = state
        .capabilities_for_target(target)
        .and_then(|capabilities| capabilities.hardware_effects.as_ref())
        .and_then(|effects| effects.effects.iter().find(|effect| effect.id == *id))
        .ok_or_else(|| impossible("hardware effect descriptor disappeared during transition"))?;
    let mut arguments = arguments.clone();
    for parameter in &descriptor.parameters {
        match parameter {
            EffectParameter::Speed { range } => {
                if let Some(speed) = arguments.speed {
                    let snapped = snap_to_discrete_range_u32(
                        u32::from(speed),
                        DiscreteRange::new(
                            u32::from(range.min),
                            u32::from(range.max),
                            u32::from(range.step),
                        ),
                    )
                    .map_err(|error| impossible(&error.to_string()))?;
                    arguments.speed = Some(
                        u16::try_from(snapped)
                            .map_err(|_| impossible("snapped speed exceeds its wire range"))?,
                    );
                }
            }
            EffectParameter::Duration { milliseconds } => {
                if let Some(duration) = arguments.duration_ms {
                    arguments.duration_ms = Some(
                        snap_to_discrete_range_u32(duration, *milliseconds)
                            .map_err(|error| impossible(&error.to_string()))?,
                    );
                }
            }
            EffectParameter::Colour { .. }
            | EffectParameter::Direction { .. }
            | EffectParameter::Brightness { .. }
            | EffectParameter::Choice { .. } => {}
        }
    }
    Ok(Effect::Hardware {
        id: id.clone(),
        arguments,
    })
}

fn impossible(message: &str) -> DaemonError {
    let diagnostic = if message.trim().is_empty() {
        "It's just not."
    } else {
        message
    };
    DaemonError::TransitionImpossible(diagnostic.to_owned())
}

#[cfg(test)]
#[path = "transitions_tests.rs"]
mod tests;
