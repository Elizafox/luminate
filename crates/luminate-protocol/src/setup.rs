// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Typed plugin setup workflow metadata shared by the daemon and clients.

use serde::{Deserialize, Serialize};

/// Opaque daemon-generated setup session identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PluginSetupSessionId(String);

impl PluginSetupSessionId {
    /// Constructs an identifier after validating its canonical representation.
    ///
    /// # Errors
    ///
    /// Returns an error unless `value` is exactly 32 lowercase hexadecimal
    /// characters.
    pub fn parse(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.len() == 32
            && value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            Ok(Self(value))
        } else {
            Err("invalid plugin setup session identifier")
        }
    }

    /// Returns the canonical identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Broad purpose of a plugin-defined setup workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PluginSetupWorkflowKind {
    /// Connect hardware or a service which is not configured yet.
    Provision,

    /// Repair credentials or connectivity for an existing configuration.
    Repair,

    /// Discover resources without committing a configuration.
    Discover,

    /// Import configuration obtained outside Luminate.
    Import,

    /// Provision hardware which is in its factory setup state.
    FactoryProvision,
}

/// One setup workflow advertised by an installed plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PluginSetupWorkflow {
    /// Canonical name of the plugin which owns the workflow.
    pub plugin: String,

    /// Stable plugin-local workflow identifier.
    pub id: String,

    /// Short human-readable name.
    pub label: String,

    /// Human-readable explanation of what the workflow configures.
    pub description: String,

    /// Broad purpose of the workflow.
    pub kind: PluginSetupWorkflowKind,
}

impl PluginSetupWorkflow {
    /// Constructs one plugin-owned setup workflow descriptor.
    #[must_use]
    pub fn new(
        plugin: impl Into<String>,
        id: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
        kind: PluginSetupWorkflowKind,
    ) -> Self {
        Self {
            plugin: plugin.into(),
            id: id.into(),
            label: label.into(),
            description: description.into(),
            kind,
        }
    }
}

/// One user-selectable setup choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSetupChoice {
    /// Stable choice identifier.
    pub id: String,
    /// Short user-facing name.
    pub label: String,
    /// Optional user-facing detail.
    pub description: Option<String>,
}

/// Client-visible state of an interactive setup session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginSetupSessionState {
    /// The workflow needs the user to select one choice.
    Choice {
        /// User-facing prompt.
        prompt: String,

        /// Choices in deterministic presentation order.
        choices: Vec<PluginSetupChoice>,
    },

    /// The workflow needs an action on physical hardware.
    PhysicalAction {
        /// User-facing instruction.
        instruction: String,
    },

    /// The workflow finished and its settings are being committed.
    Applying,

    /// Settings were committed and the plugin was reconciled.
    Completed {
        /// Sanitized completion summary.
        summary: String,

        /// Managed-configuration revision containing the result.
        revision: u64,
    },

    /// The workflow failed without exposing sensitive intermediate state.
    Failed {
        /// Operator-safe diagnostic.
        diagnostic: String,
    },

    /// The initiating actor cancelled the workflow.
    Cancelled,
}

/// Snapshot of one daemon-owned setup session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSetupSession {
    /// Opaque session identifier.
    pub id: PluginSetupSessionId,

    /// Canonical owning plugin name.
    pub plugin: String,

    /// Stable plugin-local workflow ID.
    pub workflow: String,

    /// Interaction generation required by the next response.
    pub generation: u64,

    /// Current client-visible state.
    pub state: PluginSetupSessionState,
}

/// Response to the current setup interaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginSetupInteractionResponse {
    /// Selects one advertised choice by stable ID.
    Choice(String),

    /// Confirms completion of a requested physical action.
    Confirmed,
}
