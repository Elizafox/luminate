// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! OpenAPI-only descriptions of the canonical JSON projected by REST.

#![allow(
    dead_code,
    reason = "these DTOs exist solely for compile-time OpenAPI schema generation"
)]

use serde_json::Value;
use utoipa::ToSchema;

#[derive(ToSchema)]
pub(super) struct DeviceSchema {
    pub id: String,
    pub name: String,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub provider_instance: Option<String>,
    /// Canonical surface objects. Their capability fields are open to protocol additions.
    #[schema(value_type = Vec<Object>)]
    pub surfaces: Vec<Value>,
    /// Canonical device-local group objects.
    #[schema(value_type = Vec<Object>)]
    pub groups: Vec<Value>,
    /// Canonical capability set. Unknown capability fields must be preserved.
    #[schema(value_type = Object)]
    pub capabilities: Value,
    pub category: Option<String>,
    pub physical_tags: Vec<String>,
    pub host_attached: bool,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(ToSchema)]
pub(super) struct CollectionSchema {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// Unix UID, Windows SID, or `{authority, subject}` principal.
    #[schema(value_type = Object)]
    pub owner: Value,
    pub kind: Option<String>,
    /// Externally tagged `Target` or `Collection` members.
    #[schema(value_type = Vec<Object>)]
    pub members: Vec<Value>,
}

#[derive(ToSchema)]
pub(super) struct DeviceStateSchema {
    pub device: String,
    #[schema(value_type = Vec<Object>)]
    pub observations: Vec<Value>,
    #[schema(value_type = Object)]
    pub reachability: Value,
    #[schema(value_type = Object)]
    pub reconciliation: Value,
    #[schema(value_type = Vec<Object>)]
    pub adoption: Vec<Value>,
    pub latest_error: Option<String>,
    pub latest_attempt_ms: Option<u64>,
}

#[derive(ToSchema)]
pub(super) struct CollectionStateSchema {
    pub collection: String,
    #[schema(value_type = Object)]
    pub appearance: Option<Value>,
    #[schema(value_type = Object)]
    pub effective_appearance: Option<Value>,
}

#[derive(ToSchema)]
pub(super) struct SceneSchema {
    pub id: String,
    pub revision: u64,
    pub name: String,
    pub description: Option<String>,
    #[schema(value_type = Object)]
    pub owner: Value,
    /// Externally tagged frozen or dynamic collection bindings.
    #[schema(value_type = Vec<Object>)]
    pub bindings: Vec<Value>,
}

#[derive(ToSchema)]
pub(super) struct ManagementSnapshotSchema {
    pub revision: u64,
    #[schema(value_type = Object)]
    pub desired_daemon: Value,
    #[schema(value_type = Object)]
    pub effective_daemon: Value,
    pub locked_daemon_settings: Vec<String>,
    #[schema(value_type = Vec<Object>)]
    pub plugins: Vec<Value>,
}

#[derive(ToSchema)]
pub(super) struct ManagementChangeSetSchema {
    pub revision: u64,
    #[schema(value_type = Vec<Object>)]
    pub changes: Vec<Value>,
}

#[derive(ToSchema)]
pub(super) struct PolicyDocumentSchema {
    pub revision: u64,
    /// Roles keyed by stable role name.
    #[schema(value_type = Object)]
    pub roles: Value,
    #[schema(value_type = Vec<Object>)]
    pub bindings: Vec<Value>,
}

#[derive(ToSchema)]
pub(super) struct TransitionStatusSchema {
    pub id: String,
    /// Externally tagged canonical target identifiers.
    #[schema(value_type = Vec<Object>)]
    pub targets: Vec<Value>,
    pub elapsed_ms: u64,
    pub duration_ms: u64,
    /// `Completed`, `Cancelled`, or `Failed`; absent while active.
    #[schema(value_type = Object)]
    pub outcome: Option<Value>,
}

#[derive(ToSchema)]
pub(super) struct IdSchema {
    pub id: String,
}

#[derive(ToSchema)]
pub(super) struct OutcomeSchema {
    #[schema(value_type = Vec<Object>)]
    pub applied: Vec<Value>,
    #[schema(value_type = Vec<Object>)]
    pub denied: Vec<Value>,
}

#[derive(ToSchema)]
pub(super) struct ReplacePolicySchema {
    pub expected_revision: u64,
    #[schema(value_type = Object)]
    pub document: Value,
}

#[derive(ToSchema)]
pub(super) struct ManagementPatchSchema {
    pub expected_revision: u64,
    #[schema(value_type = Vec<Object>)]
    pub mutations: Vec<Value>,
}
