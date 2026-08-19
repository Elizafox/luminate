// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Reading and patching managed configuration.
//!
//! `PatchManagement` is the one request that persists before it acts. Once the
//! new revision is on disk the patch has succeeded, so every later step
//! (refreshing the effective view, re-merging plugin settings, reconciling
//! plugin hosts) reports failure without rolling the revision back. The
//! errors below therefore describe how far the commit got, rather than
//! implying the configuration was rejected.

use super::super::authz::RequestAuthorizer;
use super::super::{
    Arc, ErrorCode, Event, OperationError, RescanReason, Response, ResponseStatus, task,
};
use super::DispatchContext;
use crate::managed_config::{self, PrepareManagementPatchError};

pub(super) async fn get_management(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }

    let managed = ctx.management.managed.lock().await;
    Response {
        status: ResponseStatus::ManagementSnapshot(Box::new(managed.snapshot(
            &ctx.management.global,
            ctx.plugin_manager.management_plugins(),
        ))),
    }
}

pub(super) async fn patch_management(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    patch: luminate_protocol::ManagementPatch,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }

    let mut managed = ctx.management.managed.lock().await;
    let prepared = match managed.prepare_patch(patch, &ctx.plugin_manager.manageable_plugins()) {
        Ok(prepared) => prepared,
        Err(PrepareManagementPatchError::Conflict {
            expected_revision,
            current_revision,
        }) => {
            return management_error(
                ErrorCode::Conflict,
                format!(
                    "expected revision {expected_revision}, current revision is {current_revision}"
                ),
            );
        }
        Err(PrepareManagementPatchError::Invalid(error)) => {
            return management_error(ErrorCode::InvalidArgument, error.to_string());
        }
    };
    let changes = match prepared.commit(&ctx.management.managed_path, &mut managed) {
        Ok(changes) => changes,
        Err(error) => {
            return management_error(
                ErrorCode::Internal,
                format!("failed to persist managed configuration: {error:#}"),
            );
        }
    };
    let effective =
        managed_config::merge_daemon_preferences(&ctx.management.global, &managed.daemon);
    ctx.state
        .lock()
        .await
        .set_cct_emulation_override(effective.cct_emulation);
    ctx.management.replace_effective(effective);
    let configuration_changes = match ctx
        .plugin_manager
        .apply_managed_config(&ctx.management.global, &managed)
    {
        Ok(configuration_changes) => configuration_changes,
        Err(error) => {
            return management_error(
                ErrorCode::Internal,
                format!(
                    "managed configuration was persisted but its runtime view could not be updated: {error:#}"
                ),
            );
        }
    };
    let plugins = Arc::clone(ctx.plugin_manager);
    let reconciled = match task::spawn_blocking(move || {
        plugins.reconcile_managed_plugins(&configuration_changes)
    })
    .await
    {
        Ok(reconciled) => reconciled,
        Err(join_error) => {
            return management_error(
                ErrorCode::Internal,
                format!(
                    "managed configuration was persisted but activation reconciliation panicked: {join_error}"
                ),
            );
        }
    };
    if let Some(topology) = reconciled.topology
        && !topology.changed_devices.is_empty()
    {
        ctx.state
            .lock()
            .await
            .replace_devices_preserving_withdrawn_state(topology.devices);
        let _ = ctx.mutations.events.send(Event::TopologyChanged {
            devices: topology.changed_devices,
        });
    }
    if reconciled.activated && !ctx.rescans.request(RescanReason::Operator) {
        tracing::warn!(
            "managed plugins were activated but the follow-up rescan could not be scheduled: the daemon is shutting down"
        );
    }
    let _ = ctx.mutations.events.send(Event::ConfigurationChanged {
        changes: changes.clone(),
    });

    Response {
        status: ResponseStatus::ManagementPatched(changes),
    }
}

fn management_error(code: ErrorCode, message: String) -> Response {
    Response {
        status: ResponseStatus::Error(OperationError {
            code,
            message,
            retry_after_ms: None,
            applied_targets: Vec::new(),
        }),
    }
}
