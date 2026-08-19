// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Blocking ordinary-client C transition operations and snapshots.

#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "each FFI entry point documents one pointer contract covering its repeated field reads"
)]

use std::time::Duration;

use luminate_core::transition::TransitionCancellation;

use super::effects::read_target;
use super::scenes::{LuminateSceneTargetStateInput, read_scene_target_state};
use super::*;

/// Transition configuration. Zero `step_interval_ms` selects the default.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LuminateTransitionOptions {
    pub duration_ms: u64,
    pub step_interval_ms: u64,
    /// One of the `LUMINATE_TRANSITION_FUNCTION_*` values.
    pub function: u32,
    /// One of the `LUMINATE_TRANSITION_COLOUR_*` values.
    pub colour_interpolation: u32,
    /// One of the `LUMINATE_HUE_DIRECTION_*` values. Must be shortest for
    /// `OKLab` interpolation.
    pub hue_direction: u32,
}

/// One concrete ephemeral transition destination.
#[repr(C)]
pub struct LuminateTransitionTargetStateInput {
    pub target: LuminateTarget,
    pub state: LuminateSceneTargetStateInput,
}

pub(crate) fn options(
    value: LuminateTransitionOptions,
) -> Result<TransitionOptions, LuminateStatus> {
    let function = match value.function {
        0 => TransitionFunction::Linear,
        1 => TransitionFunction::EaseIn,
        2 => TransitionFunction::EaseOut,
        3 => TransitionFunction::EaseInOut,
        _ => return invalid_transition_option("unknown transition function"),
    };
    let hue_direction = match value.hue_direction {
        0 => HueDirection::Shortest,
        1 => HueDirection::Increasing,
        2 => HueDirection::Decreasing,
        _ => return invalid_transition_option("unknown hue direction"),
    };
    let colour_interpolation = match value.colour_interpolation {
        0 => TransitionColourInterpolation::Encoded { hue_direction },
        1 if hue_direction == HueDirection::Shortest => TransitionColourInterpolation::Oklab,
        1 => {
            return invalid_transition_option(
                "hue direction does not apply to OKLab interpolation",
            );
        }
        _ => return invalid_transition_option("unknown transition colour interpolation"),
    };
    TransitionOptions::new(
        Duration::from_millis(value.duration_ms),
        (value.step_interval_ms != 0).then(|| Duration::from_millis(value.step_interval_ms)),
    )
    .map(|options| {
        options
            .with_function(function)
            .with_colour_interpolation(colour_interpolation)
    })
    .map_err(|error| {
        crate::ffi::set_last_error(error.to_string());
        LuminateStatus::InvalidArgument
    })
}

fn invalid_transition_option<T>(message: &str) -> Result<T, LuminateStatus> {
    crate::ffi::set_last_error(message);
    Err(LuminateStatus::InvalidArgument)
}

pub(crate) unsafe fn target_states(
    values: *const LuminateTransitionTargetStateInput,
    count: usize,
) -> Result<Vec<TransitionTargetState>, LuminateStatus> {
    if count == 0 {
        crate::ffi::set_last_error("transition target states must not be empty");
        return Err(LuminateStatus::InvalidArgument);
    }
    if values.is_null() {
        crate::ffi::set_last_error("transition target states pointer is null");
        return Err(LuminateStatus::NullPointer);
    }
    // SAFETY: caller guarantees a readable array of `count` inputs.
    unsafe { std::slice::from_raw_parts(values, count) }
        .iter()
        .map(|value| {
            Ok(TransitionTargetState {
                // SAFETY: each embedded target satisfies the public pointer contract.
                target: unsafe { read_target(ptr::from_ref(&value.target)) }?,
                // SAFETY: each embedded state satisfies the public pointer contract.
                state: unsafe { read_scene_target_state(&value.state) }?,
            })
        })
        .collect()
}

pub(in crate::ffi_typed) fn write_status(
    result: crate::Result<TransitionStatus>,
    out: *mut *mut LuminateTransitionSnapshot,
) -> LuminateStatus {
    match result {
        Ok(status) => {
            // SAFETY: public entry points validate `out`.
            unsafe { *out = Box::into_raw(Box::new(LuminateTransitionSnapshot(status))) };
            clear_last_error();
            LuminateStatus::Ok
        }
        Err(error) => store_error(&error),
    }
}

unsafe fn transition_out(
    out: *mut *mut LuminateTransitionSnapshot,
) -> Result<*mut *mut LuminateTransitionSnapshot, LuminateStatus> {
    if out.is_null() {
        crate::ffi::set_last_error("transition snapshot output pointer is null");
        Err(LuminateStatus::NullPointer)
    } else {
        Ok(out)
    }
}

/// Starts a scene-to-scene transition.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_transition_scene_to_scene(
    client: *mut LuminateClient,
    source_scene_id: *const c_char,
    destination_scene_id: *const c_char,
    timing: LuminateTransitionOptions,
    out: *mut *mut LuminateTransitionSnapshot,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        let source = match unsafe { read_required_str(source_scene_id, "source_scene_id") } {
            Ok(value) => SceneId::new(value),
            Err(error) => return error,
        };
        let destination =
            match unsafe { read_required_str(destination_scene_id, "destination_scene_id") } {
                Ok(value) => SceneId::new(value),
                Err(error) => return error,
            };
        let timing = match options(timing) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let out = match unsafe { transition_out(out) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        let result = match call_client(client, move |client| async move {
            client
                .transitions()
                .scene_to_scene(source, destination, timing)
                .await
        }) {
            Ok(value) => value,
            Err(error) => return error,
        };
        write_status(result, out)
    })
}

/// Starts a current-to-scene transition.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_transition_current_to_scene(
    client: *mut LuminateClient,
    destination_scene_id: *const c_char,
    timing: LuminateTransitionOptions,
    out: *mut *mut LuminateTransitionSnapshot,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        let destination =
            match unsafe { read_required_str(destination_scene_id, "destination_scene_id") } {
                Ok(value) => SceneId::new(value),
                Err(error) => return error,
            };
        let timing = match options(timing) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let out = match unsafe { transition_out(out) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        let result = match call_client(client, move |client| async move {
            client
                .transitions()
                .current_to_scene(destination, timing)
                .await
        }) {
            Ok(value) => value,
            Err(error) => return error,
        };
        write_status(result, out)
    })
}

unsafe fn transition_to_states(
    client: *mut LuminateClient,
    source: Option<SceneId>,
    states: *const LuminateTransitionTargetStateInput,
    state_count: usize,
    timing: LuminateTransitionOptions,
    out: *mut *mut LuminateTransitionSnapshot,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        let states = match unsafe { target_states(states, state_count) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        let timing = match options(timing) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let out = match unsafe { transition_out(out) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        let result = match call_client(client, move |client| async move {
            match source {
                Some(source) => {
                    client
                        .transitions()
                        .scene_to_states(source, states, timing)
                        .await
                }
                None => client.transitions().current_to_states(states, timing).await,
            }
        }) {
            Ok(value) => value,
            Err(error) => return error,
        };
        write_status(result, out)
    })
}

/// Starts a scene-to-ephemeral-state transition.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_transition_scene_to_states(
    client: *mut LuminateClient,
    source_scene_id: *const c_char,
    states: *const LuminateTransitionTargetStateInput,
    state_count: usize,
    timing: LuminateTransitionOptions,
    out: *mut *mut LuminateTransitionSnapshot,
) -> LuminateStatus {
    let source = match unsafe { read_required_str(source_scene_id, "source_scene_id") } {
        Ok(value) => SceneId::new(value),
        Err(error) => return error,
    };
    unsafe { transition_to_states(client, Some(source), states, state_count, timing, out) }
}

/// Starts a current-to-ephemeral-state transition.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_transition_current_to_states(
    client: *mut LuminateClient,
    states: *const LuminateTransitionTargetStateInput,
    state_count: usize,
    timing: LuminateTransitionOptions,
    out: *mut *mut LuminateTransitionSnapshot,
) -> LuminateStatus {
    unsafe { transition_to_states(client, None, states, state_count, timing, out) }
}

unsafe fn transition_by_id(
    client: *mut LuminateClient,
    id: *const c_char,
    out: *mut *mut LuminateTransitionSnapshot,
    operation: u32,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        let id = match unsafe { read_required_str(id, "transition_id") } {
            Ok(value) => TransitionId::new(value),
            Err(error) => return error,
        };
        let out = match unsafe { transition_out(out) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        let result = match call_client(client, move |client| async move {
            match operation {
                0 => client.transitions().get(id).await,
                1 => client.transitions().abort(id).await,
                _ => client.transitions().wait(id).await,
            }
        }) {
            Ok(value) => value,
            Err(error) => return error,
        };
        write_status(result, out)
    })
}

/// Gets a transition snapshot.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_transition_get(
    client: *mut LuminateClient,
    id: *const c_char,
    out: *mut *mut LuminateTransitionSnapshot,
) -> LuminateStatus {
    unsafe { transition_by_id(client, id, out, 0) }
}

/// Aborts a transition and waits until no later step can write.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_transition_abort(
    client: *mut LuminateClient,
    id: *const c_char,
    out: *mut *mut LuminateTransitionSnapshot,
) -> LuminateStatus {
    unsafe { transition_by_id(client, id, out, 1) }
}

/// Waits for a terminal transition.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_transition_wait(
    client: *mut LuminateClient,
    id: *const c_char,
    out: *mut *mut LuminateTransitionSnapshot,
) -> LuminateStatus {
    unsafe { transition_by_id(client, id, out, 2) }
}

/// Frees an owned transition snapshot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_transition_snapshot_free(value: *mut LuminateTransitionSnapshot) {
    if !value.is_null() {
        // SAFETY: pointer came from this library and is freed at most once.
        drop(unsafe { Box::from_raw(value) });
    }
}

/// Borrowed transition identifier.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_transition_id(
    value: *const LuminateTransitionSnapshot,
) -> LuminateStringView {
    unsafe { value.as_ref() }.map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |value| sv(value.0.id.as_str()),
    )
}

/// Elapsed milliseconds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_transition_elapsed_ms(
    value: *const LuminateTransitionSnapshot,
) -> u64 {
    unsafe { value.as_ref() }.map_or(0, |value| value.0.elapsed_ms)
}

/// Requested duration in milliseconds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_transition_duration_ms(
    value: *const LuminateTransitionSnapshot,
) -> u64 {
    unsafe { value.as_ref() }.map_or(0, |value| value.0.duration_ms)
}

/// Status kind: 0 active, 1 completed, 2 cancelled, or 3 failed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_transition_status_kind(
    value: *const LuminateTransitionSnapshot,
) -> u32 {
    unsafe { value.as_ref() }.map_or(u32::MAX, |value| match value.0.outcome {
        None => 0,
        Some(TransitionOutcome::Completed) => 1,
        Some(TransitionOutcome::Cancelled(_)) => 2,
        Some(TransitionOutcome::Failed { .. }) => 3,
    })
}

/// Cancellation reason: 0 aborted, 1 replaced, 2 conflicting mutation, or 3
/// authorization expired. Returns `UINT32_MAX` unless the transition was
/// cancelled.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_transition_cancellation_reason(
    value: *const LuminateTransitionSnapshot,
) -> u32 {
    unsafe { value.as_ref() }.map_or(u32::MAX, |value| match value.0.outcome {
        Some(TransitionOutcome::Cancelled(TransitionCancellation::Aborted)) => 0,
        Some(TransitionOutcome::Cancelled(TransitionCancellation::Replaced)) => 1,
        Some(TransitionOutcome::Cancelled(TransitionCancellation::ConflictingMutation)) => 2,
        Some(TransitionOutcome::Cancelled(TransitionCancellation::AuthorizationExpired)) => 3,
        None | Some(TransitionOutcome::Completed | TransitionOutcome::Failed { .. }) => u32::MAX,
    })
}

/// Borrowed runtime-failure diagnostic, or an empty view for other statuses.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_transition_failure_diagnostic(
    value: *const LuminateTransitionSnapshot,
) -> LuminateStringView {
    let diagnostic = unsafe { value.as_ref() }.and_then(|value| match &value.0.outcome {
        Some(TransitionOutcome::Failed { diagnostic, .. }) => Some(diagnostic.as_str()),
        None | Some(TransitionOutcome::Completed | TransitionOutcome::Cancelled(_)) => None,
    });
    diagnostic.map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        sv,
    )
}

/// Number of targets updated by the failed step.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_transition_failed_target_count(
    value: *const LuminateTransitionSnapshot,
) -> usize {
    unsafe { value.as_ref() }.map_or(0, |value| match &value.0.outcome {
        Some(TransitionOutcome::Failed {
            applied_targets, ..
        }) => applied_targets.len(),
        None | Some(TransitionOutcome::Completed | TransitionOutcome::Cancelled(_)) => 0,
    })
}

/// Borrowed target updated by the failed step at `index`, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_transition_failed_target_at(
    value: *const LuminateTransitionSnapshot,
    index: usize,
) -> *const LuminateTargetView {
    unsafe { value.as_ref() }
        .and_then(|value| match &value.0.outcome {
            Some(TransitionOutcome::Failed {
                applied_targets, ..
            }) => applied_targets.get(index),
            None | Some(TransitionOutcome::Completed | TransitionOutcome::Cancelled(_)) => None,
        })
        .map_or(ptr::null(), cast_ref)
}

/// Number of concrete controlled targets.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_transition_target_count(
    value: *const LuminateTransitionSnapshot,
) -> usize {
    unsafe { value.as_ref() }.map_or(0, |value| value.0.targets.len())
}

/// Borrowed controlled target at `index`, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_transition_target_at(
    value: *const LuminateTransitionSnapshot,
    index: usize,
) -> *const LuminateTargetView {
    unsafe { value.as_ref() }
        .and_then(|value| value.0.targets.get(index))
        .map_or(ptr::null(), cast_ref)
}

#[cfg(test)]
#[path = "transitions_tests.rs"]
mod tests;
