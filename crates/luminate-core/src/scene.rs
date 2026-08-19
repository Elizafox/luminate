// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Persistent, owner-controlled snapshots of intended target state.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::appearance_slot::{AppearanceSlotId, AppearanceSlotValue};
use crate::collection::{CollectionId, OwnerIdentity};
use crate::effect::Effect;
use crate::state::EmissionState;
use crate::target::TargetId;
use crate::util::declare_opaque_id;

/// A persistent sparse snapshot of intended state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scene {
    /// Stable, server-generated identifier.
    pub id: SceneId,

    /// Optimistic-concurrency revision, beginning at one.
    pub revision: u64,

    /// Human-readable name. Scene names need not be unique.
    pub name: String,

    /// Optional human-readable description.
    pub description: Option<String>,

    /// Identity of the principal that created the scene.
    pub owner: OwnerIdentity,

    /// Ordered target bindings.
    pub bindings: Vec<SceneBinding>,
}

impl Scene {
    /// Validates the scene's sparse states and rejects duplicate concrete
    /// targets across all bindings.
    ///
    /// # Errors
    ///
    /// Returns a [`SceneValidationError`] when the scene has revision zero,
    /// has no bindings, contains an invalid target state, or binds the same
    /// captured target more than once.
    pub fn validate(&self) -> Result<(), SceneValidationError> {
        if Uuid::parse_str(self.id.as_str()).is_err() {
            return Err(SceneValidationError::InvalidId);
        }
        if self.revision == 0 {
            return Err(SceneValidationError::ZeroRevision);
        }
        if self.bindings.is_empty() {
            return Err(SceneValidationError::NoBindings);
        }

        let mut targets = HashSet::with_capacity(self.bindings.len());
        for binding in &self.bindings {
            binding.state().validate()?;
            if !targets.insert(binding.target()) {
                return Err(SceneValidationError::DuplicateTarget(
                    binding.target().clone(),
                ));
            }
        }

        Ok(())
    }
}

/// One captured target and the rule governing whether it remains applicable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneBinding {
    /// Always addresses one captured concrete target.
    Frozen {
        /// Captured target.
        target: TargetId,

        /// Sparse intended state to apply.
        state: SceneTargetState,
    },

    /// Applies only while the captured target remains a transitive member of
    /// the named collection.
    DynamicCollectionMember {
        /// Collection whose current membership gates application.
        collection: CollectionId,

        /// Concrete target captured from the collection.
        target: TargetId,

        /// Captured sparse intended state for this target.
        state: SceneTargetState,
    },
}

impl SceneBinding {
    /// Returns the captured concrete target.
    #[must_use]
    pub const fn target(&self) -> &TargetId {
        match self {
            Self::Frozen { target, .. } | Self::DynamicCollectionMember { target, .. } => target,
        }
    }

    /// Returns the captured sparse target state.
    #[must_use]
    pub const fn state(&self) -> &SceneTargetState {
        match self {
            Self::Frozen { state, .. } | Self::DynamicCollectionMember { state, .. } => state,
        }
    }

    /// Returns the dynamic collection, if this binding is membership-gated.
    #[must_use]
    pub const fn dynamic_collection(&self) -> Option<&CollectionId> {
        match self {
            Self::Frozen { .. } => None,
            Self::DynamicCollectionMember { collection, .. } => Some(collection),
        }
    }
}

/// Sparse intended facets captured for one target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneTargetState {
    /// Configured appearance, excluding [`Effect::Off`].
    pub appearance: Option<Effect>,

    /// Configured brightness.
    pub brightness: Option<u32>,

    /// Whether this target should emit light.
    pub emission: Option<EmissionState>,

    /// Ordered firmware-stored appearance programs.
    #[serde(default)]
    pub appearance_slots: Option<Vec<AppearanceSlotValue>>,
}

impl SceneTargetState {
    /// Validates that the state is non-empty and internally consistent.
    ///
    /// `Emitting` requires an appearance because the current mutation model
    /// resumes emission by reapplying that appearance. Brightness zero and
    /// `Emitting` are contradictory.
    ///
    /// # Errors
    ///
    /// Returns a [`SceneValidationError`] for an empty state,
    /// [`Effect::Off`] appearance, or a contradictory facet combination.
    pub fn validate(&self) -> Result<(), SceneValidationError> {
        if self.appearance.is_none()
            && self.brightness.is_none()
            && self.emission.is_none()
            && self.appearance_slots.is_none()
        {
            return Err(SceneValidationError::EmptyTargetState);
        }
        if matches!(self.appearance, Some(Effect::Off)) {
            return Err(SceneValidationError::OffAppearance);
        }
        if self.appearance_slots.as_ref().is_some_and(Vec::is_empty) {
            return Err(SceneValidationError::EmptyAppearanceSlots);
        }
        if let Some(values) = &self.appearance_slots {
            let mut slots = HashSet::with_capacity(values.len());
            for value in values {
                if !slots.insert(&value.slot) {
                    return Err(SceneValidationError::DuplicateAppearanceSlot(
                        value.slot.clone(),
                    ));
                }
            }
        }
        if self.emission == Some(EmissionState::Emitting) && self.appearance.is_none() {
            return Err(SceneValidationError::EmittingWithoutAppearance);
        }
        if self.emission == Some(EmissionState::Emitting) && self.brightness == Some(0) {
            return Err(SceneValidationError::EmittingAtZeroBrightness);
        }

        Ok(())
    }
}

/// How capture binds selected targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneCaptureMode {
    /// Capture each selected concrete target permanently.
    Frozen,

    /// Capture the collection's current leaf members and gate each binding on
    /// that collection's future membership.
    DynamicCollectionMembers {
        /// Collection to resolve at capture and application time.
        collection: CollectionId,
    },
}

/// Opaque, server-generated stable identifier for a scene.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SceneId(String);

declare_opaque_id!(
    SceneId,
    "Wraps an existing scene identifier. Prefer [`Self::generate`] when \
     minting a fresh scene identifier."
);

impl SceneId {
    /// Mints a fresh, randomly generated identifier.
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

/// Invalid scene definition.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SceneValidationError {
    /// Persisted scene identifiers are UUIDs minted by the daemon.
    #[error("scene identifier must be a UUID")]
    InvalidId,

    /// Revisions begin at one.
    #[error("scene revision must be at least one")]
    ZeroRevision,

    /// A scene must affect at least one target.
    #[error("scene must contain at least one binding")]
    NoBindings,

    /// A sparse target state must contain at least one facet.
    #[error("scene target state must contain at least one facet")]
    EmptyTargetState,

    /// Off controls emission and is not an appearance.
    #[error("Effect::Off cannot be stored as a scene appearance")]
    OffAppearance,

    /// Resuming emission needs a configured appearance to reapply.
    #[error("emitting scene state requires an appearance")]
    EmittingWithoutAppearance,

    /// Zero brightness cannot visibly emit.
    #[error("emitting scene state cannot have zero brightness")]
    EmittingAtZeroBrightness,

    /// A present slot collection must contain at least one value.
    #[error("scene appearance slots must contain at least one value")]
    EmptyAppearanceSlots,

    /// A slot may occur only once in a target state.
    #[error("scene appearance slot {0:?} occurs more than once")]
    DuplicateAppearanceSlot(AppearanceSlotId),

    /// One scene cannot bind the same captured target twice.
    #[error("scene contains a duplicate target binding: {0:?}")]
    DuplicateTarget(TargetId),
}

#[cfg(test)]
#[path = "scene_tests.rs"]
mod tests;
