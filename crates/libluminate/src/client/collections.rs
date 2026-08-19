// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::{
    Client, Collection, CollectionCategory, CollectionId, CollectionMember, Request,
    ResponseStatus, Result, unexpected_response,
};

impl Client {
    /// Creates a collection owned by the calling principal. `members` is
    /// validated as a self-reference- and cycle-free set by the daemon
    /// before the collection is created.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed,
    /// [`Error::NotFound`] if `members` references a nested collection that
    /// doesn't exist, [`Error::InvalidArgument`] if `members` would create a
    /// self-reference or a cycle, or another [`Error`] variant for an
    /// I/O/protocol failure.
    pub async fn create_collection(
        &self,
        name: String,
        description: Option<String>,
        kind: Option<CollectionCategory>,
        members: Vec<CollectionMember>,
    ) -> Result<CollectionId> {
        match self
            .request(Request::CreateCollection {
                name,
                description,
                kind,
                members,
            })
            .await?
            .status
        {
            ResponseStatus::CollectionCreated { id } => Ok(id),
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
            | ResponseStatus::Collections(_)
            | ResponseStatus::CollectionInfo(_)
            | ResponseStatus::CollectionApplied { .. }
            | ResponseStatus::ShmFrameStreamReady { .. }
            | ResponseStatus::PluginSetupWorkflows(_)
            | ResponseStatus::ManagementSnapshot(_)
            | ResponseStatus::ManagementPatched(_)
            | ResponseStatus::AccessPolicy(_)
            | ResponseStatus::TokenCreated { .. }
            | ResponseStatus::Tokens(_)
            | ResponseStatus::AttestationCreated { .. }
            | ResponseStatus::Attestations(_)
            | ResponseStatus::Scene(_)
            | ResponseStatus::Scenes(_)
            | ResponseStatus::SceneInfo(_)
            | ResponseStatus::SceneApplied { .. }
            | ResponseStatus::Transition(_)
            | ResponseStatus::PluginSetupSession(_)
            | ResponseStatus::Error { .. }) => {
                Err(unexpected_response("collection created", &other))
            }
        }
    }

    /// Destroys a collection. Refused if another collection still
    /// references it, or if the caller doesn't own it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed,
    /// [`Error::NotFound`] if no such collection exists,
    /// [`Error::PermissionDenied`] if the caller doesn't own it, an
    /// [`Error::Conflict`] if another collection still references it, or
    /// another [`Error`] variant for an I/O/protocol failure.
    pub async fn destroy_collection(&self, id: CollectionId) -> Result<()> {
        self.expect_ack(Request::DestroyCollection { id }).await
    }

    /// Adds one member to a collection's explicit membership. A no-op if
    /// `member` is already present.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed,
    /// [`Error::NotFound`] if `id` (or a nested collection `member`
    /// references) doesn't exist, [`Error::InvalidArgument`] if `member`
    /// would create a self-reference or a cycle, [`Error::PermissionDenied`]
    /// if the caller doesn't own `id`, or another [`Error`] variant for an
    /// I/O/protocol failure.
    pub async fn add_collection_member(
        &self,
        id: CollectionId,
        member: CollectionMember,
    ) -> Result<()> {
        self.expect_ack(Request::AddCollectionMember { id, member })
            .await
    }

    /// Removes one member from a collection's explicit membership. A no-op
    /// if `member` is absent.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed,
    /// [`Error::NotFound`] if `id` doesn't exist, [`Error::PermissionDenied`]
    /// if the caller doesn't own `id`, or another [`Error`] variant for an
    /// I/O/protocol failure.
    pub async fn remove_collection_member(
        &self,
        id: CollectionId,
        member: CollectionMember,
    ) -> Result<()> {
        self.expect_ack(Request::RemoveCollectionMember { id, member })
            .await
    }

    /// Lists every collection currently registered.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed, or
    /// another [`Error`] variant for an I/O/protocol failure.
    pub async fn list_collections(&self) -> Result<Vec<Collection>> {
        match self.request(Request::ListCollections).await?.status {
            ResponseStatus::Collections(collections) => Ok(collections),
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
            | ResponseStatus::CollectionInfo(_)
            | ResponseStatus::CollectionApplied { .. }
            | ResponseStatus::ShmFrameStreamReady { .. }
            | ResponseStatus::PluginSetupWorkflows(_)
            | ResponseStatus::ManagementSnapshot(_)
            | ResponseStatus::ManagementPatched(_)
            | ResponseStatus::AccessPolicy(_)
            | ResponseStatus::TokenCreated { .. }
            | ResponseStatus::Tokens(_)
            | ResponseStatus::AttestationCreated { .. }
            | ResponseStatus::Attestations(_)
            | ResponseStatus::Scene(_)
            | ResponseStatus::Scenes(_)
            | ResponseStatus::SceneInfo(_)
            | ResponseStatus::SceneApplied { .. }
            | ResponseStatus::Transition(_)
            | ResponseStatus::PluginSetupSession(_)
            | ResponseStatus::Error { .. }) => Err(unexpected_response("collection list", &other)),
        }
    }

    /// Looks up one collection by id; `Ok(None)` if no such collection
    /// exists.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed, or
    /// another [`Error`] variant for an I/O/protocol failure.
    pub async fn get_collection(&self, id: CollectionId) -> Result<Option<Collection>> {
        match self.request(Request::GetCollection { id }).await?.status {
            ResponseStatus::CollectionInfo(collection) => Ok(collection.map(|boxed| *boxed)),
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
            | ResponseStatus::CollectionApplied { .. }
            | ResponseStatus::ShmFrameStreamReady { .. }
            | ResponseStatus::PluginSetupWorkflows(_)
            | ResponseStatus::ManagementSnapshot(_)
            | ResponseStatus::ManagementPatched(_)
            | ResponseStatus::AccessPolicy(_)
            | ResponseStatus::TokenCreated { .. }
            | ResponseStatus::Tokens(_)
            | ResponseStatus::AttestationCreated { .. }
            | ResponseStatus::Attestations(_)
            | ResponseStatus::Scene(_)
            | ResponseStatus::Scenes(_)
            | ResponseStatus::SceneInfo(_)
            | ResponseStatus::SceneApplied { .. }
            | ResponseStatus::Transition(_)
            | ResponseStatus::PluginSetupSession(_)
            | ResponseStatus::Error { .. }) => {
                Err(unexpected_response("collection lookup", &other))
            }
        }
    }
}
