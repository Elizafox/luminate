// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Plugin setup workflow discovery and daemon-owned interactive sessions.

use std::collections::BTreeMap;

use luminate_plugin_api::{PluginSetupSettingValue, PluginSetupStep};
use luminate_protocol::{
    ErrorCode, ManagementMutation, ManagementPatch, PluginSetupInteractionResponse,
    PluginSetupSessionId, PluginSetupSessionState, Response, ResponseStatus, SettingValue,
    WriteOnly,
};

use super::super::authz::RequestAuthorizer;
use super::{DispatchContext, OperationError};
use crate::plugin_host::execute_setup_step;
use crate::plugins::setup::PreparedSetupStep;
use crate::plugins::setup::valid_public_text;
use tokio::task;

pub(super) async fn list_workflows(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    plugin: &str,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    match ctx.plugin_manager.setup_workflows(plugin) {
        Some(workflows) => Response {
            status: ResponseStatus::PluginSetupWorkflows(workflows),
        },
        None => setup_error(ErrorCode::NotFound, format!("plugin not found: {plugin}")),
    }
}

pub(super) async fn start(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    plugin: String,
    workflow: String,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    let Some(workflows) = ctx.plugin_manager.setup_workflows(&plugin) else {
        return setup_error(ErrorCode::NotFound, "plugin not found".to_owned());
    };
    if !workflows.iter().any(|candidate| candidate.id == workflow) {
        return setup_error(
            ErrorCode::InvalidArgument,
            format!("plugin {plugin} does not advertise setup workflow {workflow}"),
        );
    }
    let baseline_settings = {
        let managed = ctx.management.managed.lock().await;
        managed
            .plugins
            .iter()
            .find(|entry| entry.name == plugin)
            .map_or_else(toml::Table::new, |entry| entry.settings.clone())
    };
    let prepared = match ctx.plugin_manager.prepare_setup_start(
        &plugin,
        &workflow,
        ctx.principal.rate_limit_key(),
        baseline_settings,
    ) {
        Ok(prepared) => prepared,
        Err(error) => return setup_error(ErrorCode::Internal, error.to_string()),
    };
    execute_step(ctx, auth, prepared).await
}

pub(super) async fn respond(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    session: PluginSetupSessionId,
    generation: u64,
    response: PluginSetupInteractionResponse,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    let prepared = match ctx.plugin_manager.prepare_setup_response(
        &session,
        generation,
        response,
        &ctx.principal.rate_limit_key(),
    ) {
        Ok(prepared) => prepared,
        Err(error) => {
            let diagnostic = error.to_string();
            if diagnostic.contains("not found") || diagnostic.contains("another actor") {
                return setup_error(ErrorCode::NotFound, "setup session not found".to_owned());
            }
            let code = if diagnostic.contains("stale") || diagnostic.contains("in progress") {
                ErrorCode::Conflict
            } else {
                ErrorCode::InvalidArgument
            };
            return setup_error(code, diagnostic);
        }
    };
    execute_step(ctx, auth, prepared).await
}

pub(super) async fn get(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    session: &PluginSetupSessionId,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    match ctx
        .plugin_manager
        .setup_session(session, &ctx.principal.rate_limit_key())
    {
        Ok(snapshot) => session_response(snapshot),
        Err(_) => setup_error(ErrorCode::NotFound, "setup session not found".to_owned()),
    }
}

pub(super) async fn cancel(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    session: &PluginSetupSessionId,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    match ctx
        .plugin_manager
        .cancel_setup(session, &ctx.principal.rate_limit_key())
    {
        Ok(snapshot) => session_response(snapshot),
        Err(_) => setup_error(ErrorCode::NotFound, "setup session not found".to_owned()),
    }
}

async fn execute_step(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    prepared: PreparedSetupStep,
) -> Response {
    let session = prepared.session.clone();
    let outcome =
        task::spawn_blocking(move || execute_setup_step(&prepared.path, &prepared.request)).await;
    let step = match outcome {
        Ok(Ok(step)) => step,
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "plugin setup step failed");
            return finish_failed(ctx, &session, "plugin setup failed".to_owned());
        }
        Err(error) => {
            return finish_failed(
                ctx,
                &session,
                format!("plugin setup host panicked: {error}"),
            );
        }
    };
    match step {
        PluginSetupStep::Interaction {
            continuation,
            interaction,
        } => match ctx
            .plugin_manager
            .finish_setup_interaction(&session, continuation, interaction)
        {
            Ok(snapshot) => session_response(snapshot),
            Err(error) => finish_failed(ctx, &session, error.to_string()),
        },
        PluginSetupStep::Complete { settings, summary } => {
            commit_setup(ctx, auth, &session, settings, summary).await
        }
    }
}

async fn commit_setup(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    session: &PluginSetupSessionId,
    settings: BTreeMap<String, PluginSetupSettingValue>,
    summary: String,
) -> Response {
    if !valid_public_text(&summary, 2_048) {
        return finish_failed(
            ctx,
            session,
            "plugin setup returned an invalid summary".to_owned(),
        );
    }
    let (snapshot, baseline) = match ctx.plugin_manager.begin_setup_commit(session) {
        Ok(Some(commit)) => commit,
        Ok(None) => {
            return match ctx
                .plugin_manager
                .setup_session(session, &ctx.principal.rate_limit_key())
            {
                Ok(snapshot) => session_response(snapshot),
                Err(error) => setup_error(ErrorCode::NotFound, error.to_string()),
            };
        }
        Err(error) => return setup_error(ErrorCode::Internal, error.to_string()),
    };
    let (revision, conflict) = {
        let managed = ctx.management.managed.lock().await;
        let current = managed
            .plugins
            .iter()
            .find(|entry| entry.name == snapshot.plugin)
            .map(|entry| &entry.settings);
        let conflict = settings
            .keys()
            .any(|key| dotted_value(current, key) != dotted_value(Some(&baseline), key));
        (managed.revision, conflict)
    };
    if conflict {
        return finish_failed(
            ctx,
            session,
            "plugin settings changed while setup was in progress; start setup again".to_owned(),
        );
    }
    let mutations = settings
        .into_iter()
        .map(|(key, value)| {
            Ok(ManagementMutation::SetPluginSetting {
                plugin: snapshot.plugin.clone(),
                key,
                value: WriteOnly::new(setting_value(value)?),
            })
        })
        .collect::<Result<Vec<_>, &'static str>>();
    let mutations = match mutations {
        Ok(mutations) if !mutations.is_empty() => mutations,
        Ok(_) => return finish_failed(ctx, session, "setup returned no settings".to_owned()),
        Err(error) => return finish_failed(ctx, session, error.to_owned()),
    };
    let response = super::management::patch_management(
        ctx,
        auth,
        ManagementPatch {
            expected_revision: revision,
            mutations,
        },
    )
    .await;
    let revision = match committed_revision(&response) {
        Ok(revision) => revision,
        Err(error) => return finish_failed(ctx, session, error),
    };
    match ctx.plugin_manager.finish_setup_terminal(
        session,
        PluginSetupSessionState::Completed { summary, revision },
    ) {
        Ok(snapshot) => session_response(snapshot),
        Err(error) => setup_error(ErrorCode::Internal, error.to_string()),
    }
}

fn committed_revision(response: &Response) -> Result<u64, String> {
    match &response.status {
        ResponseStatus::ManagementPatched(changes) => Ok(changes.revision),
        ResponseStatus::Error(error) => Err(error.message.clone()),
        ResponseStatus::ServerInfo(_)
        | ResponseStatus::Devices(_)
        | ResponseStatus::WithdrawnDevices(_)
        | ResponseStatus::Device(_)
        | ResponseStatus::State(_)
        | ResponseStatus::CollectionState(_)
        | ResponseStatus::EventTicket(_)
        | ResponseStatus::Ack
        | ResponseStatus::FrameStreamStarted { .. }
        | ResponseStatus::FrameAck { .. }
        | ResponseStatus::CollectionCreated { .. }
        | ResponseStatus::Collections(_)
        | ResponseStatus::CollectionInfo(_)
        | ResponseStatus::Scene(_)
        | ResponseStatus::Scenes(_)
        | ResponseStatus::SceneInfo(_)
        | ResponseStatus::SceneApplied { .. }
        | ResponseStatus::Transition(_)
        | ResponseStatus::ManagementSnapshot(_)
        | ResponseStatus::PluginSetupWorkflows(_)
        | ResponseStatus::PluginSetupSession(_)
        | ResponseStatus::AccessPolicy(_)
        | ResponseStatus::TokenCreated { .. }
        | ResponseStatus::Tokens(_)
        | ResponseStatus::AttestationCreated { .. }
        | ResponseStatus::Attestations(_)
        | ResponseStatus::CollectionApplied { .. }
        | ResponseStatus::ShmFrameStreamReady { .. } => {
            Err("unexpected configuration result".to_owned())
        }
    }
}

fn setting_value(value: PluginSetupSettingValue) -> Result<SettingValue, &'static str> {
    match value {
        PluginSetupSettingValue::Boolean(value) => Ok(SettingValue::Boolean(value)),
        PluginSetupSettingValue::Integer(value) => Ok(SettingValue::Integer(value)),
        PluginSetupSettingValue::Number(value) if value.is_finite() => {
            Ok(SettingValue::Number(value))
        }
        PluginSetupSettingValue::Number(_) => Err("setup returned a non-finite number"),
        PluginSetupSettingValue::String(value) => Ok(SettingValue::String(value)),
        PluginSetupSettingValue::Array(values) => values
            .into_iter()
            .map(setting_value)
            .collect::<Result<Vec<_>, _>>()
            .map(SettingValue::Array),
        PluginSetupSettingValue::Table(values) => values
            .into_iter()
            .map(|(key, value)| Ok((key, setting_value(value)?)))
            .collect::<Result<_, _>>()
            .map(SettingValue::Table),
    }
}

fn dotted_value<'a>(table: Option<&'a toml::Table>, key: &str) -> Option<&'a toml::Value> {
    let mut parts = key.split('.');
    let first = parts.next()?;
    let mut value = table?.get(first)?;
    for part in parts {
        value = value.as_table()?.get(part)?;
    }
    Some(value)
}

fn finish_failed(
    ctx: &DispatchContext<'_>,
    session: &PluginSetupSessionId,
    diagnostic: String,
) -> Response {
    match ctx
        .plugin_manager
        .finish_setup_terminal(session, PluginSetupSessionState::Failed { diagnostic })
    {
        Ok(snapshot) => session_response(snapshot),
        Err(error) => setup_error(ErrorCode::Internal, error.to_string()),
    }
}

fn session_response(session: luminate_protocol::PluginSetupSession) -> Response {
    Response {
        status: ResponseStatus::PluginSetupSession(Box::new(session)),
    }
}

fn setup_error(code: ErrorCode, message: String) -> Response {
    Response {
        status: ResponseStatus::Error(OperationError {
            code,
            message,
            retry_after_ms: None,
            applied_targets: Vec::new(),
        }),
    }
}
