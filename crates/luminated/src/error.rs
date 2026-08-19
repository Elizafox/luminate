// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Domain errors returned through the daemon protocol.

use std::ffi;

use luminate_core::collection::CollectionId;
use luminate_core::scene::{SceneId, SceneValidationError};
use luminate_core::target::TargetId;
use luminate_core::transition::TransitionId;
use luminate_protocol::{ErrorCode, OperationError};
use luminate_protocol::{Response, ResponseStatus};

/// Errors that can occur while resolving or applying a client mutation,
/// carrying enough structure to map to a client-visible `ErrorCode` via
/// [`DaemonError::error_code`] without string-matching a formatted message.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("target not found: {0:?}")]
    TargetNotFound(TargetId),

    /// A target resolved against the current topology, but no loaded
    /// plugin claims ownership of its device. Reachable only as an
    /// internal inconsistency: every device that reaches normalization has
    /// a corresponding owner by construction of
    /// `PluginManager::device_descriptors`'s ownership filtering.
    #[error("no plugin owns device: {0}")]
    DeviceUnowned(String),

    #[error("plugin {plugin} does not support the update for {device}: {diagnostic}")]
    PluginUnsupportedUpdate {
        plugin: String,
        device: String,
        diagnostic: String,
    },

    #[error("plugin {plugin} rejected invalid update for {device}: {diagnostic}")]
    PluginInvalidUpdate {
        plugin: String,
        device: String,
        diagnostic: String,
    },

    #[error("plugin {plugin} encountered an I/O failure for {device}: {diagnostic}")]
    PluginIo {
        plugin: String,
        device: String,
        diagnostic: String,
    },

    #[error("plugin {plugin} cannot currently reach {device}: {diagnostic}")]
    PluginUnavailable {
        plugin: String,
        device: String,
        diagnostic: String,
    },

    #[error("plugin {plugin} rate-limited the update for {device}: {diagnostic}")]
    PluginRateLimited {
        plugin: String,
        device: String,
        diagnostic: String,
        retry_after_ms: Option<u64>,
    },

    #[error("plugin {plugin} encountered an internal failure for {device}: {diagnostic}")]
    PluginInternal {
        plugin: String,
        device: String,
        diagnostic: String,
    },

    #[error("no loaded plugin named {0:?}")]
    PluginNotFound(String),

    /// The plugin's host process was terminated as part of the reload, but
    /// the replacement host failed to start (or reported different identity
    /// metadata). The plugin is left unloaded, not left in its old state.
    #[error("failed to reload plugin {plugin}: {diagnostic}")]
    PluginReloadFailed { plugin: String, diagnostic: String },

    #[error("operation is not supported for target {target:?}: {reason}")]
    UnsupportedCapability { target: TargetId, reason: String },

    #[error("invalid operation for target {target:?}: {reason}")]
    InvalidArgument { target: TargetId, reason: String },

    #[error("cannot save current state for {target:?}: state is unknown")]
    UnknownState { target: TargetId },

    #[error("transition is impossible: {0}")]
    TransitionImpossible(String),

    #[error("transition not found: {0:?}")]
    TransitionNotFound(TransitionId),

    /// A frame stream or hardware effect already owns `target` in a way
    /// incompatible with the requested operation.
    #[error("conflict for target {target:?}: {reason}")]
    Conflict { target: TargetId, reason: String },

    /// The topology used for authorization changed before the mutation began.
    #[error("authorization conflict: {reason}")]
    AuthorizationConflict { reason: String },

    #[error("failed to serialize plugin update: {0}")]
    Serialize(#[from] serde_json::Error),

    #[error("plugin update payload contained an interior NUL byte: {0}")]
    InteriorNul(#[from] ffi::NulError),

    #[error("internal daemon error: {0}")]
    Internal(String),

    #[error("persistence resource limit exceeded: {0}")]
    ResourceLimit(String),

    /// A fan-out mutation changed some hardware before a later member failed.
    /// The successful subset has been committed and must be persisted even
    /// though the client receives an error describing the partial result.
    #[error("partial mutation: {diagnostic}")]
    PartialMutation {
        diagnostic: String,
        applied_targets: Vec<TargetId>,
    },

    #[error("collection not found: {0:?}")]
    CollectionNotFound(CollectionId),

    /// A collection mutation would have made it list itself as a direct
    /// member.
    #[error("collection {0:?} cannot list itself as a member")]
    CollectionSelfReference(CollectionId),

    /// A collection mutation would have closed an indirect cycle through the
    /// named nested collection.
    #[error("adding {0:?} would create a collection membership cycle")]
    CollectionCycle(CollectionId),

    /// `id` cannot be destroyed while `referenced_by` still lists it as a
    /// nested member.
    #[error("collection {id:?} is still referenced by {referenced_by:?}")]
    CollectionInUse {
        id: CollectionId,
        referenced_by: Vec<CollectionId>,
    },

    /// The caller doesn't own the collection it tried to destroy or
    /// structurally modify.
    #[error("collection {0:?} is owned by a different principal")]
    CollectionNotOwned(CollectionId),

    #[error("scene not found: {0:?}")]
    SceneNotFound(SceneId),

    #[error("scene {id:?} revision conflict: expected {expected}, current {actual}")]
    SceneRevisionConflict {
        id: SceneId,
        expected: u64,
        actual: u64,
    },

    #[error("scene {0:?} is owned by a different principal")]
    SceneNotOwned(SceneId),

    #[error("invalid scene: {0}")]
    InvalidScene(#[from] SceneValidationError),

    #[error("collection {collection:?} is referenced by scene {scene:?}")]
    CollectionReferencedByScene {
        collection: CollectionId,
        scene: SceneId,
    },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl DaemonError {
    #[must_use]
    pub fn error_code(&self) -> ErrorCode {
        match self {
            Self::TargetNotFound(_)
            | Self::CollectionNotFound(_)
            | Self::SceneNotFound(_)
            | Self::TransitionNotFound(_)
            | Self::PluginNotFound(_) => ErrorCode::NotFound,
            Self::PluginUnsupportedUpdate { .. } | Self::UnsupportedCapability { .. } => {
                ErrorCode::Unsupported
            }
            Self::PluginInvalidUpdate { .. }
            | Self::InvalidArgument { .. }
            | Self::InvalidScene(_)
            | Self::CollectionSelfReference(_)
            | Self::CollectionCycle(_)
            | Self::ResourceLimit(_) => ErrorCode::InvalidArgument,
            Self::PluginIo { .. } => ErrorCode::Io,
            Self::PluginUnavailable { .. } | Self::PluginReloadFailed { .. } => {
                ErrorCode::Unavailable
            }
            Self::PluginRateLimited { .. } => ErrorCode::RateLimited,
            Self::UnknownState { .. } => ErrorCode::UnknownState,
            Self::TransitionImpossible(_) => ErrorCode::TransitionImpossible,
            Self::Conflict { .. }
            | Self::AuthorizationConflict { .. }
            | Self::CollectionInUse { .. } => ErrorCode::Conflict,
            Self::SceneRevisionConflict { .. } | Self::CollectionReferencedByScene { .. } => {
                ErrorCode::Conflict
            }
            Self::DeviceUnowned(_)
            | Self::Serialize(_)
            | Self::InteriorNul(_)
            | Self::Internal(_)
            | Self::PluginInternal { .. }
            | Self::Other(_) => ErrorCode::Internal,
            Self::PartialMutation { .. } => ErrorCode::PartialMutation,
            Self::CollectionNotOwned(_) | Self::SceneNotOwned(_) => ErrorCode::PermissionDenied,
        }
    }

    #[must_use]
    pub fn operation_error(&self) -> OperationError {
        let retry_after_ms = if let Self::PluginRateLimited { retry_after_ms, .. } = self {
            *retry_after_ms
        } else {
            None
        };
        let applied_targets = if let Self::PartialMutation {
            applied_targets, ..
        } = self
        {
            applied_targets.clone()
        } else {
            Vec::new()
        };
        OperationError {
            code: self.error_code(),
            message: self.to_string(),
            retry_after_ms,
            applied_targets,
        }
    }
}

/// Convert `DaemonError`s into `Response`s for clients.
impl From<DaemonError> for Response {
    fn from(error: DaemonError) -> Response {
        Response {
            status: ResponseStatus::Error(error.operation_error()),
        }
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
