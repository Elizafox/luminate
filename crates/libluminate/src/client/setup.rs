// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Plugin setup workflow discovery.

use super::{Client, Request, ResponseStatus, Result, unexpected_response};
use crate::{
    PluginSetupInteractionResponse, PluginSetupSession, PluginSetupSessionId, PluginSetupWorkflow,
};

impl Client {
    /// Lists setup workflows advertised by one installed plugin.
    ///
    /// An empty list means the plugin does not provide setup through Luminate.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::NotFound`](crate::ErrorKind::NotFound) when
    /// `plugin` is not installed. Also returns an authorization, transport,
    /// daemon, or protocol error if the workflows cannot be fetched.
    pub async fn plugin_setup_workflows(
        &self,
        plugin: impl Into<String>,
    ) -> Result<Vec<PluginSetupWorkflow>> {
        match self
            .request(Request::ListPluginSetupWorkflows {
                plugin: plugin.into(),
            })
            .await?
            .status
        {
            ResponseStatus::PluginSetupWorkflows(workflows) => Ok(workflows),
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
            | ResponseStatus::Scene(_)
            | ResponseStatus::Scenes(_)
            | ResponseStatus::SceneInfo(_)
            | ResponseStatus::SceneApplied { .. }
            | ResponseStatus::Transition(_)
            | ResponseStatus::ManagementSnapshot(_)
            | ResponseStatus::ManagementPatched(_)
            | ResponseStatus::AccessPolicy(_)
            | ResponseStatus::TokenCreated { .. }
            | ResponseStatus::Tokens(_)
            | ResponseStatus::AttestationCreated { .. }
            | ResponseStatus::Attestations(_)
            | ResponseStatus::CollectionApplied { .. }
            | ResponseStatus::ShmFrameStreamReady { .. }
            | ResponseStatus::PluginSetupSession(_)
            | ResponseStatus::Error(_)) => {
                Err(unexpected_response("plugin setup workflows", &other))
            }
        }
    }

    /// Starts one advertised plugin setup workflow.
    ///
    /// # Errors
    ///
    /// Returns an authorization, validation, transport, daemon, or protocol
    /// error if the session cannot be started.
    pub async fn start_plugin_setup(
        &self,
        plugin: impl Into<String>,
        workflow: impl Into<String>,
    ) -> Result<PluginSetupSession> {
        self.plugin_setup_request(Request::StartPluginSetup {
            plugin: plugin.into(),
            workflow: workflow.into(),
        })
        .await
    }

    /// Responds to the current interaction in a plugin setup session.
    ///
    /// # Errors
    ///
    /// Returns an error when the session is absent, belongs to another actor,
    /// the generation is stale, the response has the wrong type, or setup or
    /// configuration persistence fails.
    pub async fn respond_plugin_setup(
        &self,
        session: PluginSetupSessionId,
        generation: u64,
        response: PluginSetupInteractionResponse,
    ) -> Result<PluginSetupSession> {
        self.plugin_setup_request(Request::RespondPluginSetup {
            session,
            generation,
            response,
        })
        .await
    }

    /// Reads the current state of an actor-owned plugin setup session.
    ///
    /// # Errors
    ///
    /// Returns an authorization, not-found, transport, daemon, or protocol
    /// error if the session cannot be read.
    pub async fn plugin_setup_session(
        &self,
        session: PluginSetupSessionId,
    ) -> Result<PluginSetupSession> {
        self.plugin_setup_request(Request::GetPluginSetup { session })
            .await
    }

    /// Cancels an actor-owned plugin setup session.
    ///
    /// # Errors
    ///
    /// Returns an authorization, not-found, transport, daemon, or protocol
    /// error if the session cannot be cancelled.
    pub async fn cancel_plugin_setup(
        &self,
        session: PluginSetupSessionId,
    ) -> Result<PluginSetupSession> {
        self.plugin_setup_request(Request::CancelPluginSetup { session })
            .await
    }

    async fn plugin_setup_request(&self, request: Request) -> Result<PluginSetupSession> {
        match self.request(request).await?.status {
            ResponseStatus::PluginSetupSession(session) => Ok(*session),
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
            | ResponseStatus::Scene(_)
            | ResponseStatus::Scenes(_)
            | ResponseStatus::SceneInfo(_)
            | ResponseStatus::SceneApplied { .. }
            | ResponseStatus::Transition(_)
            | ResponseStatus::ManagementSnapshot(_)
            | ResponseStatus::ManagementPatched(_)
            | ResponseStatus::PluginSetupWorkflows(_)
            | ResponseStatus::AccessPolicy(_)
            | ResponseStatus::TokenCreated { .. }
            | ResponseStatus::Tokens(_)
            | ResponseStatus::AttestationCreated { .. }
            | ResponseStatus::Attestations(_)
            | ResponseStatus::CollectionApplied { .. }
            | ResponseStatus::ShmFrameStreamReady { .. }
            | ResponseStatus::Error(_)) => Err(unexpected_response("plugin setup session", &other)),
        }
    }
}
