// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Typed C bindings for plugin setup workflow discovery.

use super::policy::null_view;
use super::*;

use std::ffi::c_char;

use crate::ffi::read_required_str;
use crate::{
    PluginSetupInteractionResponse, PluginSetupSession, PluginSetupSessionId,
    PluginSetupSessionState, PluginSetupWorkflow, PluginSetupWorkflowKind,
};

pub type LuminatePluginSetupWorkflowKind = u32;
pub const LUMINATE_PLUGIN_SETUP_PROVISION: LuminatePluginSetupWorkflowKind = 0;
pub const LUMINATE_PLUGIN_SETUP_REPAIR: LuminatePluginSetupWorkflowKind = 1;
pub const LUMINATE_PLUGIN_SETUP_DISCOVER: LuminatePluginSetupWorkflowKind = 2;
pub const LUMINATE_PLUGIN_SETUP_IMPORT: LuminatePluginSetupWorkflowKind = 3;
pub const LUMINATE_PLUGIN_SETUP_FACTORY_PROVISION: LuminatePluginSetupWorkflowKind = 4;

pub type LuminatePluginSetupSessionState = u32;
pub const LUMINATE_PLUGIN_SETUP_CHOICE: LuminatePluginSetupSessionState = 0;
pub const LUMINATE_PLUGIN_SETUP_PHYSICAL_ACTION: LuminatePluginSetupSessionState = 1;
pub const LUMINATE_PLUGIN_SETUP_COMPLETED: LuminatePluginSetupSessionState = 2;
pub const LUMINATE_PLUGIN_SETUP_FAILED: LuminatePluginSetupSessionState = 3;
pub const LUMINATE_PLUGIN_SETUP_CANCELLED: LuminatePluginSetupSessionState = 4;
pub const LUMINATE_PLUGIN_SETUP_APPLYING: LuminatePluginSetupSessionState = 5;

/// Owned snapshot of one plugin setup session.
pub struct LuminatePluginSetupSession(pub(crate) PluginSetupSession);

/// Borrowed choice within an owning plugin setup session.
pub struct LuminatePluginSetupChoice;

/// Releases an owned plugin setup session. Null is a no-op.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_session_free(
    value: *mut LuminatePluginSetupSession,
) {
    if !value.is_null() {
        // SAFETY: the pointer contract requires a value returned by this API.
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            drop(Box::from_raw(value));
        }));
    }
}

fn map_session<T>(
    value: *const LuminatePluginSetupSession,
    map: impl FnOnce(&PluginSetupSession) -> T,
) -> Option<T> {
    // SAFETY: the C contract requires a live API-owned session pointer.
    unsafe { value.as_ref() }.map(|value| map(&value.0))
}

/// Returns the opaque canonical session identifier.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_session_id(
    value: *const LuminatePluginSetupSession,
) -> LuminateStringView {
    map_session(value, |value| sv(value.id.as_str())).unwrap_or_else(null_view)
}

/// Returns the canonical plugin name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_session_plugin(
    value: *const LuminatePluginSetupSession,
) -> LuminateStringView {
    map_session(value, |value| sv(&value.plugin)).unwrap_or_else(null_view)
}

/// Returns the stable plugin-local workflow identifier.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_session_workflow(
    value: *const LuminatePluginSetupSession,
) -> LuminateStringView {
    map_session(value, |value| sv(&value.workflow)).unwrap_or_else(null_view)
}

/// Returns the interaction generation required by the next response.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_session_generation(
    value: *const LuminatePluginSetupSession,
) -> u64 {
    map_session(value, |value| value.generation).unwrap_or(0)
}

/// Returns the `LuminatePluginSetupSessionState` discriminant.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_session_state(
    value: *const LuminatePluginSetupSession,
) -> LuminatePluginSetupSessionState {
    map_session(value, |value| match value.state {
        PluginSetupSessionState::Choice { .. } => LUMINATE_PLUGIN_SETUP_CHOICE,
        PluginSetupSessionState::PhysicalAction { .. } => LUMINATE_PLUGIN_SETUP_PHYSICAL_ACTION,
        PluginSetupSessionState::Applying => LUMINATE_PLUGIN_SETUP_APPLYING,
        PluginSetupSessionState::Completed { .. } => LUMINATE_PLUGIN_SETUP_COMPLETED,
        PluginSetupSessionState::Failed { .. } => LUMINATE_PLUGIN_SETUP_FAILED,
        PluginSetupSessionState::Cancelled => LUMINATE_PLUGIN_SETUP_CANCELLED,
    })
    .unwrap_or(u32::MAX)
}

/// Returns the choice prompt, physical instruction, completion summary, or
/// failure diagnostic for the current state.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_session_message(
    value: *const LuminatePluginSetupSession,
) -> LuminateStringView {
    map_session(value, |value| match &value.state {
        PluginSetupSessionState::Choice { prompt, .. } => sv(prompt),
        PluginSetupSessionState::PhysicalAction { instruction } => sv(instruction),
        PluginSetupSessionState::Completed { summary, .. } => sv(summary),
        PluginSetupSessionState::Failed { diagnostic } => sv(diagnostic),
        PluginSetupSessionState::Applying | PluginSetupSessionState::Cancelled => null_view(),
    })
    .unwrap_or_else(null_view)
}

/// Writes the committed management revision for a completed session.
///
/// Returns false and leaves `out_revision` unchanged unless the session is
/// completed or either pointer is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_session_revision(
    value: *const LuminatePluginSetupSession,
    out_revision: *mut u64,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(revision), Some(out_revision)) = (
        map_session(value, |value| match value.state {
            PluginSetupSessionState::Completed { revision, .. } => Some(revision),
            PluginSetupSessionState::Choice { .. }
            | PluginSetupSessionState::PhysicalAction { .. }
            | PluginSetupSessionState::Applying
            | PluginSetupSessionState::Failed { .. }
            | PluginSetupSessionState::Cancelled => None,
        })
        .flatten(),
        unsafe { out_revision.as_mut() },
    ) else {
        return false;
    };
    *out_revision = revision;
    true
}

/// Returns the number of choices in the current choice interaction.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_session_choice_count(
    value: *const LuminatePluginSetupSession,
) -> usize {
    map_session(value, |value| match &value.state {
        PluginSetupSessionState::Choice { choices, .. } => choices.len(),
        _ => 0,
    })
    .unwrap_or(0)
}

fn choice_ref(
    value: *const LuminatePluginSetupChoice,
) -> Option<&'static crate::PluginSetupChoice> {
    // SAFETY: the C contract requires a view returned by
    // `luminate_plugin_setup_session_choice_at`.
    unsafe { value.cast::<crate::PluginSetupChoice>().as_ref() }
}

/// Returns the borrowed choice at `index`, or null when unavailable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_session_choice_at(
    value: *const LuminatePluginSetupSession,
    index: usize,
) -> *const LuminatePluginSetupChoice {
    map_session(value, |value| match &value.state {
        PluginSetupSessionState::Choice { choices, .. } => {
            choices.get(index).map_or(ptr::null(), cast_ref)
        }
        _ => ptr::null(),
    })
    .unwrap_or(ptr::null())
}

/// Returns the choice's stable identifier, or an absent view.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_choice_id(
    value: *const LuminatePluginSetupChoice,
) -> LuminateStringView {
    choice_ref(value).map_or_else(null_view, |choice| sv(&choice.id))
}

/// Returns the choice's label, or an absent view.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_choice_label(
    value: *const LuminatePluginSetupChoice,
) -> LuminateStringView {
    choice_ref(value).map_or_else(null_view, |choice| sv(&choice.label))
}

/// Returns the optional choice description, or an absent view.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_choice_description(
    value: *const LuminatePluginSetupChoice,
) -> LuminateStringView {
    choice_ref(value)
        .and_then(|choice| choice.description.as_deref())
        .map_or_else(null_view, sv)
}

/// Owned list of setup workflows advertised by one installed plugin.
pub struct LuminatePluginSetupWorkflowList(pub(crate) Vec<PluginSetupWorkflow>);

/// Borrowed setup workflow view.
pub struct LuminatePluginSetupWorkflow;

fn map_workflow<T>(
    value: *const LuminatePluginSetupWorkflow,
    map: impl FnOnce(&PluginSetupWorkflow) -> T,
) -> Option<T> {
    // SAFETY: the C contract requires a pointer returned by
    // `luminate_plugin_setup_workflow_list_at`, whose pointee remains owned by
    // the live list. The borrow cannot escape this helper.
    unsafe { value.cast::<PluginSetupWorkflow>().as_ref() }.map(map)
}

/// Releases an owned plugin setup workflow list. Null is a no-op.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_workflow_list_free(
    value: *mut LuminatePluginSetupWorkflowList,
) {
    if !value.is_null() {
        // SAFETY: the pointer contract requires a value returned by this API.
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            drop(Box::from_raw(value));
        }));
    }
}

/// Returns the number of setup workflows in `value`, or zero for null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_workflow_list_count(
    value: *const LuminatePluginSetupWorkflowList,
) -> usize {
    // SAFETY: required by the public pointer contract.
    unsafe { value.as_ref() }.map_or(0, |value| value.0.len())
}

/// Returns the borrowed workflow at `index`, or null when out of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_workflow_list_at(
    value: *const LuminatePluginSetupWorkflowList,
    index: usize,
) -> *const LuminatePluginSetupWorkflow {
    // SAFETY: required by the public pointer contract.
    unsafe { value.as_ref() }
        .and_then(|value| value.0.get(index))
        .map_or(ptr::null(), |workflow| {
            ptr::from_ref(workflow).cast::<LuminatePluginSetupWorkflow>()
        })
}

/// Returns the canonical plugin name which owns `value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_workflow_plugin(
    value: *const LuminatePluginSetupWorkflow,
) -> LuminateStringView {
    map_workflow(value, |value| sv(&value.plugin)).unwrap_or_else(null_view)
}

/// Returns the stable plugin-local workflow identifier.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_workflow_id(
    value: *const LuminatePluginSetupWorkflow,
) -> LuminateStringView {
    map_workflow(value, |value| sv(&value.id)).unwrap_or_else(null_view)
}

/// Returns the short human-readable workflow name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_workflow_label(
    value: *const LuminatePluginSetupWorkflow,
) -> LuminateStringView {
    map_workflow(value, |value| sv(&value.label)).unwrap_or_else(null_view)
}

/// Returns the human-readable workflow description.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_workflow_description(
    value: *const LuminatePluginSetupWorkflow,
) -> LuminateStringView {
    map_workflow(value, |value| sv(&value.description)).unwrap_or_else(null_view)
}

/// Returns the workflow's `LuminatePluginSetupWorkflowKind` value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_plugin_setup_workflow_kind(
    value: *const LuminatePluginSetupWorkflow,
) -> LuminatePluginSetupWorkflowKind {
    map_workflow(value, |value| match value.kind {
        PluginSetupWorkflowKind::Provision => LUMINATE_PLUGIN_SETUP_PROVISION,
        PluginSetupWorkflowKind::Repair => LUMINATE_PLUGIN_SETUP_REPAIR,
        PluginSetupWorkflowKind::Discover => LUMINATE_PLUGIN_SETUP_DISCOVER,
        PluginSetupWorkflowKind::Import => LUMINATE_PLUGIN_SETUP_IMPORT,
        PluginSetupWorkflowKind::FactoryProvision => LUMINATE_PLUGIN_SETUP_FACTORY_PROVISION,
        _ => u32::MAX,
    })
    .unwrap_or(u32::MAX)
}

fn finish_workflows(
    result: crate::Result<Vec<PluginSetupWorkflow>>,
) -> (LuminateStatus, *mut LuminatePluginSetupWorkflowList) {
    match result {
        Ok(workflows) => (
            LuminateStatus::Ok,
            Box::into_raw(Box::new(LuminatePluginSetupWorkflowList(workflows))),
        ),
        Err(error) => (store_error(&error), ptr::null_mut()),
    }
}

fn finish_session(
    result: crate::Result<PluginSetupSession>,
) -> (LuminateStatus, *mut LuminatePluginSetupSession) {
    match result {
        Ok(session) => (
            LuminateStatus::Ok,
            Box::into_raw(Box::new(LuminatePluginSetupSession(session))),
        ),
        Err(error) => (store_error(&error), ptr::null_mut()),
    }
}

pub(crate) unsafe fn read_session_id(
    value: *const c_char,
) -> Result<PluginSetupSessionId, LuminateStatus> {
    // SAFETY: forwarded from each public C string contract.
    let value = unsafe { read_required_str(value, "session_id") }?;
    PluginSetupSessionId::parse(value.to_owned()).map_err(|error| {
        crate::ffi::set_last_error(error);
        LuminateStatus::InvalidArgument
    })
}

fn write_session_result(
    result: crate::Result<PluginSetupSession>,
    out_session: *mut *mut LuminatePluginSetupSession,
) -> LuminateStatus {
    let (status, session) = finish_session(result);
    if status == LuminateStatus::Ok {
        // SAFETY: each caller validates this pointer before invoking the helper.
        unsafe { out_session.write(session) };
        clear_last_error();
    }
    status
}

/// Starts one advertised plugin setup workflow.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_plugin_setup_start(
    client: *mut LuminateClient,
    plugin: *const c_char,
    workflow: *const c_char,
    out_session: *mut *mut LuminatePluginSetupSession,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        // SAFETY: required by the public string contracts.
        let plugin = match unsafe { read_required_str(plugin, "plugin") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        // SAFETY: required by the public string contracts.
        let workflow = match unsafe { read_required_str(workflow, "workflow") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        if out_session.is_null() {
            crate::ffi::set_last_error("plugin setup session output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let result = match call_client(client, move |client| async move {
            client.start_plugin_setup(plugin, workflow).await
        }) {
            Ok(value) => value,
            Err(status) => return status,
        };
        write_session_result(result, out_session)
    })
}

/// Selects one choice in the current plugin setup interaction.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_plugin_setup_choose(
    client: *mut LuminateClient,
    session_id: *const c_char,
    generation: u64,
    choice: *const c_char,
    out_session: *mut *mut LuminatePluginSetupSession,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        // SAFETY: required by the public string contracts.
        let session = match unsafe { read_session_id(session_id) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        // SAFETY: required by the public string contracts.
        let choice = match unsafe { read_required_str(choice, "choice") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        if out_session.is_null() {
            crate::ffi::set_last_error("plugin setup session output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let result = match call_client(client, move |client| async move {
            client
                .respond_plugin_setup(
                    session,
                    generation,
                    PluginSetupInteractionResponse::Choice(choice),
                )
                .await
        }) {
            Ok(value) => value,
            Err(status) => return status,
        };
        write_session_result(result, out_session)
    })
}

/// Confirms the current plugin setup physical action.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_plugin_setup_confirm(
    client: *mut LuminateClient,
    session_id: *const c_char,
    generation: u64,
    out_session: *mut *mut LuminatePluginSetupSession,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        // SAFETY: required by the public string contract.
        let session = match unsafe { read_session_id(session_id) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        if out_session.is_null() {
            crate::ffi::set_last_error("plugin setup session output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let result = match call_client(client, move |client| async move {
            client
                .respond_plugin_setup(
                    session,
                    generation,
                    PluginSetupInteractionResponse::Confirmed,
                )
                .await
        }) {
            Ok(value) => value,
            Err(status) => return status,
        };
        write_session_result(result, out_session)
    })
}

/// Reads the current state of an actor-owned plugin setup session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_plugin_setup_get(
    client: *mut LuminateClient,
    session_id: *const c_char,
    out_session: *mut *mut LuminatePluginSetupSession,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        // SAFETY: required by the public string contract.
        let session = match unsafe { read_session_id(session_id) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        if out_session.is_null() {
            crate::ffi::set_last_error("plugin setup session output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let result = match call_client(client, move |client| async move {
            client.plugin_setup_session(session).await
        }) {
            Ok(value) => value,
            Err(status) => return status,
        };
        write_session_result(result, out_session)
    })
}

/// Cancels an actor-owned plugin setup session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_plugin_setup_cancel(
    client: *mut LuminateClient,
    session_id: *const c_char,
    out_session: *mut *mut LuminatePluginSetupSession,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        // SAFETY: required by the public string contract.
        let session = match unsafe { read_session_id(session_id) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        if out_session.is_null() {
            crate::ffi::set_last_error("plugin setup session output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let result = match call_client(client, move |client| async move {
            client.cancel_plugin_setup(session).await
        }) {
            Ok(value) => value,
            Err(status) => return status,
        };
        write_session_result(result, out_session)
    })
}

/// Lists setup workflows advertised by one installed plugin.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_plugin_setup_workflows(
    client: *mut LuminateClient,
    plugin: *const c_char,
    out_workflows: *mut *mut LuminatePluginSetupWorkflowList,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: required by the public pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        // SAFETY: required by the public pointer contract.
        let plugin = match unsafe { read_required_str(plugin, "plugin") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        if out_workflows.is_null() {
            crate::ffi::set_last_error("plugin setup workflow output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let result = match call_client(client, move |client| async move {
            client.plugin_setup_workflows(plugin).await
        }) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let (status, workflows) = finish_workflows(result);
        if status == LuminateStatus::Ok {
            // SAFETY: checked for null above.
            unsafe { *out_workflows = workflows };
            clear_last_error();
        }
        status
    })
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
