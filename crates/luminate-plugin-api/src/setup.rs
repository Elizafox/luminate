// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Static setup metadata and typed step payloads for isolated setup hosts.

use std::collections::BTreeMap;
use std::ffi::{CStr, c_char};
use std::fmt;

use serde::{Deserialize, Serialize};

/// Broad purpose of a plugin-defined setup workflow.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginSetupWorkflowKind {
    /// Connect hardware or a service which is not configured yet.
    Provision = 0,
    /// Repair credentials or connectivity for an existing configuration.
    Repair = 1,
    /// Discover resources without committing configuration.
    Discover = 2,
    /// Import configuration obtained outside Luminate.
    Import = 3,
    /// Provision hardware in its factory setup state.
    FactoryProvision = 4,
}

impl PluginSetupWorkflowKind {
    /// Decodes a raw plugin ABI value.
    #[must_use]
    pub const fn from_abi(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Provision),
            1 => Some(Self::Repair),
            2 => Some(Self::Discover),
            3 => Some(Self::Import),
            4 => Some(Self::FactoryProvision),
            _ => None,
        }
    }
}

/// One statically inspectable setup workflow.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PluginSetupWorkflowDescriptor {
    /// Stable plugin-local ASCII identifier.
    pub id: *const c_char,
    /// Short UTF-8 user-facing name.
    pub label: *const c_char,
    /// UTF-8 explanation of what the workflow configures.
    pub description: *const c_char,
    /// Raw [`PluginSetupWorkflowKind`] discriminant.
    pub kind: u32,
}

// SAFETY: workflow descriptors are immutable metadata whose pointers refer to
// static storage owned by the plugin image.
unsafe impl Sync for PluginSetupWorkflowDescriptor {}

impl PluginSetupWorkflowDescriptor {
    /// Constructs static workflow metadata.
    #[must_use]
    pub const fn new(
        id: &'static CStr,
        label: &'static CStr,
        description: &'static CStr,
        kind: PluginSetupWorkflowKind,
    ) -> Self {
        Self {
            id: id.as_ptr(),
            label: label.as_ptr(),
            description: description.as_ptr(),
            kind: kind as u32,
        }
    }
}

/// A response to the interaction requested by the previous setup step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginSetupResponse {
    /// Stable identifier of one advertised choice.
    Choice(String),
    /// Confirms that the requested physical action has been performed.
    Confirmed,
}

/// Input passed to one isolated plugin setup step.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSetupRequest {
    /// Stable workflow identifier from static metadata.
    pub workflow: String,
    /// Plugin-private continuation returned by the preceding step.
    pub continuation: Vec<u8>,
    /// Client response, absent for the initial step.
    pub response: Option<PluginSetupResponse>,
}

impl fmt::Debug for PluginSetupRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PluginSetupRequest")
            .field("workflow", &self.workflow)
            .field("continuation_len", &self.continuation.len())
            .field("response", &self.response)
            .finish()
    }
}

/// One choice presented by a setup workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSetupChoice {
    /// Stable identifier returned when this choice is selected.
    pub id: String,
    /// Short user-facing name.
    pub label: String,
    /// Optional user-facing detail.
    pub description: Option<String>,
}

/// Interaction required before a setup workflow can continue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginSetupInteraction {
    /// The user must select one of a bounded set of choices.
    Choice {
        /// User-facing prompt.
        prompt: String,
        /// Choices in deterministic presentation order.
        choices: Vec<PluginSetupChoice>,
    },
    /// The user must perform an action on the physical hardware.
    PhysicalAction {
        /// User-facing instruction.
        instruction: String,
    },
}

/// A setting value produced by a successful setup workflow.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum PluginSetupSettingValue {
    /// A Boolean value.
    Boolean(bool),
    /// A signed integer value.
    Integer(i64),
    /// A finite floating-point value.
    Number(f64),
    /// A UTF-8 string value.
    String(String),
    /// An ordered sequence.
    Array(Vec<Self>),
    /// A string-keyed table.
    Table(BTreeMap<String, Self>),
}

/// Result of one isolated plugin setup step.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum PluginSetupStep {
    /// More user interaction is required.
    Interaction {
        /// Opaque plugin-private continuation retained only by the daemon.
        continuation: Vec<u8>,
        /// Client-visible interaction.
        interaction: PluginSetupInteraction,
    },
    /// Setup completed and produced settings owned by the plugin.
    Complete {
        /// Settings to validate and commit atomically.
        settings: BTreeMap<String, PluginSetupSettingValue>,
        /// Sanitized user-facing completion summary.
        summary: String,
    },
}

impl fmt::Debug for PluginSetupStep {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Interaction {
                continuation,
                interaction,
            } => formatter
                .debug_struct("Interaction")
                .field("continuation_len", &continuation.len())
                .field("interaction", interaction)
                .finish(),
            Self::Complete { settings, summary } => formatter
                .debug_struct("Complete")
                .field("setting_keys", &settings.keys().collect::<Vec<_>>())
                .field("summary", summary)
                .finish(),
        }
    }
}

/// Maximum request or response accepted for one setup callback.
pub const PLUGIN_SETUP_CBOR_CAPACITY: usize = 256 * 1024;
