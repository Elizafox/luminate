// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::{
    Client, ManagementChangeSet, ManagementPatch, ManagementSnapshot, Request, ResponseStatus,
    Result, unexpected_response,
};

impl Client {
    /// Reads the authoritative managed configuration and resolved plugin
    /// runtime view.
    ///
    /// # Errors
    ///
    /// Returns an authorization, transport, daemon, or protocol error if the
    /// snapshot cannot be fetched.
    pub async fn get_management(&self) -> Result<ManagementSnapshot> {
        match self.request(Request::GetManagement).await?.status {
            ResponseStatus::ManagementSnapshot(snapshot) => Ok(*snapshot),
            other @ (ResponseStatus::ServerInfo(_)
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
            | ResponseStatus::PluginSetupWorkflows(_)
            | ResponseStatus::ManagementPatched(_)
            | ResponseStatus::AccessPolicy(_)
            | ResponseStatus::TokenCreated { .. }
            | ResponseStatus::Tokens(_)
            | ResponseStatus::AttestationCreated { .. }
            | ResponseStatus::Attestations(_)
            | ResponseStatus::CollectionApplied { .. }
            | ResponseStatus::ShmFrameStreamReady { .. }
            | ResponseStatus::Scene(_)
            | ResponseStatus::Scenes(_)
            | ResponseStatus::SceneInfo(_)
            | ResponseStatus::SceneApplied { .. }
            | ResponseStatus::Transition(_)
            | ResponseStatus::PluginSetupSession(_)
            | ResponseStatus::Error(_)) => Err(unexpected_response("management snapshot", &other)),
        }
    }

    /// Validates and applies one revision-checked management patch atomically.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Conflict`](crate::ErrorKind::Conflict) when
    /// `patch.expected_revision` is stale. Also returns an authorization,
    /// validation, transport, daemon, or protocol error if the patch cannot be
    /// committed.
    pub async fn patch_management(&self, patch: ManagementPatch) -> Result<ManagementChangeSet> {
        match self
            .request(Request::PatchManagement { patch })
            .await?
            .status
        {
            ResponseStatus::ManagementPatched(changes) => Ok(changes),
            other @ (ResponseStatus::ServerInfo(_)
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
            | ResponseStatus::PluginSetupWorkflows(_)
            | ResponseStatus::ManagementSnapshot(_)
            | ResponseStatus::AccessPolicy(_)
            | ResponseStatus::TokenCreated { .. }
            | ResponseStatus::Tokens(_)
            | ResponseStatus::AttestationCreated { .. }
            | ResponseStatus::Attestations(_)
            | ResponseStatus::CollectionApplied { .. }
            | ResponseStatus::ShmFrameStreamReady { .. }
            | ResponseStatus::Scene(_)
            | ResponseStatus::Scenes(_)
            | ResponseStatus::SceneInfo(_)
            | ResponseStatus::SceneApplied { .. }
            | ResponseStatus::Transition(_)
            | ResponseStatus::PluginSetupSession(_)
            | ResponseStatus::Error(_)) => {
                Err(unexpected_response("management change set", &other))
            }
        }
    }
}
