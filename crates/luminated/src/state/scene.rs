// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Daemon-owned scene registry and persistence restoration.

use std::collections::HashMap;

use luminate_core::appearance_slot::AppearanceSlotUpdatePolicy;
use luminate_core::collection::OwnerIdentity;
use luminate_core::effect::Effect;
use luminate_core::scene::{Scene, SceneBinding, SceneCaptureMode, SceneId, SceneTargetState};
use luminate_core::state::{
    AppearanceState, EffectiveAppearanceState, EmissionState, FacetValue, StateFacetKind,
};
use luminate_core::target::TargetId;

use super::DaemonState;
use crate::error::DaemonError;

impl DaemonState {
    /// Every scene currently registered, for listing and snapshotting.
    #[must_use]
    pub fn scenes(&self) -> &HashMap<SceneId, Scene> {
        &self.scenes
    }

    /// Looks up one scene by id.
    #[must_use]
    pub fn scene(&self, id: &SceneId) -> Option<&Scene> {
        self.scenes.get(id)
    }

    /// Restores already-validated persisted scenes.
    pub fn restore_scenes(&mut self, scenes: HashMap<SceneId, Scene>) {
        self.scenes = scenes;
    }

    /// Captures composed intended state for concrete targets.
    ///
    /// # Errors
    ///
    /// Returns an error when a target does not exist, is outside the selected
    /// dynamic collection, or has no representable intended facets.
    pub fn capture_bindings(
        &self,
        mode: &SceneCaptureMode,
        targets: &[TargetId],
    ) -> Result<Vec<SceneBinding>, DaemonError> {
        let dynamic_leaves = match mode {
            SceneCaptureMode::Frozen => None,
            SceneCaptureMode::DynamicCollectionMembers { collection } => {
                Some((collection, self.resolve_collection_leaves(collection)?))
            }
        };
        let selected = if targets.is_empty() {
            dynamic_leaves
                .as_ref()
                .map_or_else(Vec::new, |(_, leaves)| leaves.clone())
        } else {
            targets.to_vec()
        };
        let mut bindings = Vec::with_capacity(selected.len());
        for target in &selected {
            self.ensure_target_exists(target)?;
            if let Some((collection, leaves)) = &dynamic_leaves
                && !leaves.contains(target)
            {
                return Err(DaemonError::InvalidArgument {
                    target: target.clone(),
                    reason: format!(
                        "target is not a current member of dynamic collection {}",
                        collection.as_str()
                    ),
                });
            }
            let state = self.capture_target_state(target)?;
            bindings.push(match mode {
                SceneCaptureMode::Frozen => SceneBinding::Frozen {
                    target: target.clone(),
                    state,
                },
                SceneCaptureMode::DynamicCollectionMembers { collection } => {
                    SceneBinding::DynamicCollectionMember {
                        collection: collection.clone(),
                        target: target.clone(),
                        state,
                    }
                }
            });
        }
        Ok(bindings)
    }

    /// Creates a validated scene at revision one.
    ///
    /// # Errors
    ///
    /// Returns an error when a binding is invalid against current topology or
    /// collection membership.
    pub fn create_scene(
        &mut self,
        name: String,
        description: Option<String>,
        owner: OwnerIdentity,
        bindings: Vec<SceneBinding>,
    ) -> Result<Scene, DaemonError> {
        let scene = Scene {
            id: SceneId::generate(),
            revision: 1,
            name,
            description,
            owner,
            bindings,
        };
        self.validate_scene_bindings(&scene)?;
        self.scenes.insert(scene.id.clone(), scene.clone());
        if let Err(error) = self.ensure_persistable() {
            self.scenes.remove(&scene.id);
            return Err(error);
        }
        Ok(scene)
    }

    /// Replaces a scene definition if its expected revision is current.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown scene, stale revision, or invalid
    /// replacement.
    pub fn replace_scene(
        &mut self,
        id: &SceneId,
        expected_revision: u64,
        name: String,
        description: Option<String>,
        bindings: Vec<SceneBinding>,
    ) -> Result<Scene, DaemonError> {
        let current = self
            .scenes
            .get(id)
            .ok_or_else(|| DaemonError::SceneNotFound(id.clone()))?;
        if current.revision != expected_revision {
            return Err(DaemonError::SceneRevisionConflict {
                id: id.clone(),
                expected: expected_revision,
                actual: current.revision,
            });
        }
        let replacement = Scene {
            id: id.clone(),
            revision: current.revision.saturating_add(1),
            name,
            description,
            owner: current.owner.clone(),
            bindings,
        };
        self.validate_scene_bindings(&replacement)?;
        let previous = self.scenes.insert(id.clone(), replacement.clone());
        if let Err(error) = self.ensure_persistable() {
            if let Some(previous) = previous {
                self.scenes.insert(id.clone(), previous);
            }
            return Err(error);
        }
        Ok(replacement)
    }

    /// Deletes a scene if its expected revision is current.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown scene or stale revision.
    pub fn delete_scene(
        &mut self,
        id: &SceneId,
        expected_revision: u64,
    ) -> Result<Scene, DaemonError> {
        let current = self
            .scenes
            .get(id)
            .ok_or_else(|| DaemonError::SceneNotFound(id.clone()))?;
        if current.revision != expected_revision {
            return Err(DaemonError::SceneRevisionConflict {
                id: id.clone(),
                expected: expected_revision,
                actual: current.revision,
            });
        }
        self.scenes
            .remove(id)
            .ok_or_else(|| DaemonError::SceneNotFound(id.clone()))
    }

    fn validate_scene_bindings(&self, scene: &Scene) -> Result<(), DaemonError> {
        scene.validate()?;
        for binding in &scene.bindings {
            let target = binding.target();
            let state = binding.state();
            self.ensure_target_exists(target)?;
            if let Some(appearance) = &state.appearance {
                self.validate_target_state(
                    target,
                    &super::target_state::TargetState::Effect(appearance.clone()),
                )?;
            }
            if let Some(brightness) = state.brightness {
                self.validate_target_state(
                    target,
                    &super::target_state::TargetState::Brightness(brightness),
                )?;
            }
            if state.emission.is_some() {
                self.validate_target_state(
                    target,
                    &super::target_state::TargetState::Effect(Effect::Off),
                )?;
            }
            if let Some(values) = &state.appearance_slots {
                self.validate_appearance_slot_values(target, values)?;
            }
            if let Some(collection) = binding.dynamic_collection() {
                let leaves = self.resolve_collection_leaves(collection)?;
                if !leaves.contains(target) {
                    return Err(DaemonError::InvalidArgument {
                        target: target.clone(),
                        reason: format!(
                            "target is not a current member of dynamic collection {}",
                            collection.as_str()
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    pub(crate) fn capture_target_state(
        &self,
        target: &TargetId,
    ) -> Result<SceneTargetState, DaemonError> {
        let mut captured = SceneTargetState {
            appearance: None,
            brightness: None,
            emission: None,
            appearance_slots: None,
        };
        for facet in &self.adopted_baseline {
            if self.target_covers(&facet.target, target) {
                apply_facet(&mut captured, &facet.value);
            }
        }
        for entry in &self.target_state {
            if !self.target_covers(&entry.target, target) {
                continue;
            }
            match &entry.state {
                super::target_state::TargetState::Effect(Effect::Off) => {
                    captured.emission = Some(EmissionState::Dark);
                }
                super::target_state::TargetState::Effect(effect) => {
                    captured.appearance = Some(effect.clone());
                    captured.emission = Some(EmissionState::Emitting);
                }
                super::target_state::TargetState::Brightness(value) => {
                    captured.brightness = Some(*value);
                    if *value == 0 {
                        captured.emission = Some(EmissionState::Dark);
                    }
                }
                super::target_state::TargetState::AppearanceSlots(values) => {
                    captured.appearance_slots = Some(values.clone());
                }
                super::target_state::TargetState::Clear => {
                    captured = SceneTargetState {
                        appearance: None,
                        brightness: None,
                        emission: None,
                        appearance_slots: None,
                    };
                }
            }
        }
        captured.validate().map_err(|_| DaemonError::UnknownState {
            target: target.clone(),
        })?;
        if let Some(capability) = self
            .capabilities_for_target(target)
            .and_then(|capabilities| capabilities.appearance_slots.as_ref())
            && capability.update_policy == AppearanceSlotUpdatePolicy::CompleteSet
            && captured.appearance_slots.as_ref().map_or(0, Vec::len) != capability.slots.len()
        {
            return Err(DaemonError::UnknownState {
                target: target.clone(),
            });
        }
        Ok(captured)
    }

    /// Captures a current endpoint only from fresh, exact, non-streaming
    /// observations.
    pub(crate) fn fresh_current_target_state(
        &self,
        target: &TargetId,
    ) -> Result<SceneTargetState, DaemonError> {
        self.ensure_not_frame_streaming(target)?;
        let status = self
            .device_state_status(target.device_id())
            .ok_or_else(|| DaemonError::UnknownState {
                target: target.clone(),
            })?;
        let appearance = status
            .observation(target, StateFacetKind::Appearance)
            .filter(|observation| !observation.stale)
            .and_then(|observation| match &observation.value {
                FacetValue::Appearance(AppearanceState::Static(colour)) => Some(Effect::Static {
                    colour: colour.clone(),
                }),
                FacetValue::Appearance(AppearanceState::Effect(effect)) => Some(effect.clone()),
                FacetValue::Appearance(AppearanceState::Mixed)
                | FacetValue::Brightness(_)
                | FacetValue::Emission(_)
                | FacetValue::PhysicalPower(_)
                | FacetValue::EffectiveAppearance(_)
                | FacetValue::AppearanceSlots(_) => None,
            })
            .ok_or_else(|| DaemonError::UnknownState {
                target: target.clone(),
            })?;
        let emission = status
            .observation(target, StateFacetKind::Emission)
            .filter(|observation| !observation.stale)
            .and_then(|observation| match observation.value {
                FacetValue::Emission(emission) => Some(emission),
                FacetValue::Appearance(_)
                | FacetValue::Brightness(_)
                | FacetValue::PhysicalPower(_)
                | FacetValue::EffectiveAppearance(_)
                | FacetValue::AppearanceSlots(_) => None,
            })
            .ok_or_else(|| DaemonError::UnknownState {
                target: target.clone(),
            })?;
        let effective = status
            .observation(target, StateFacetKind::EffectiveAppearance)
            .filter(|observation| !observation.stale)
            .is_some_and(|observation| {
                matches!(
                    observation.value,
                    FacetValue::EffectiveAppearance(
                        EffectiveAppearanceState::Off
                            | EffectiveAppearanceState::Static(_)
                            | EffectiveAppearanceState::Effect(_)
                    )
                )
            });
        if !effective {
            return Err(DaemonError::UnknownState {
                target: target.clone(),
            });
        }
        let brightness = status
            .observation(target, StateFacetKind::Brightness)
            .filter(|observation| !observation.stale)
            .and_then(|observation| match observation.value {
                FacetValue::Brightness(brightness) => Some(brightness),
                FacetValue::Appearance(_)
                | FacetValue::Emission(_)
                | FacetValue::PhysicalPower(_)
                | FacetValue::EffectiveAppearance(_)
                | FacetValue::AppearanceSlots(_) => None,
            });
        Ok(SceneTargetState {
            appearance: Some(appearance),
            brightness,
            emission: Some(emission),
            appearance_slots: None,
        })
    }
}

fn apply_facet(captured: &mut SceneTargetState, value: &FacetValue) {
    match value {
        FacetValue::Appearance(AppearanceState::Static(colour)) => {
            captured.appearance = Some(Effect::Static {
                colour: colour.clone(),
            });
        }
        FacetValue::Appearance(AppearanceState::Effect(effect)) => {
            captured.appearance = Some(effect.clone());
        }
        FacetValue::Appearance(AppearanceState::Mixed)
        | FacetValue::PhysicalPower(_)
        | FacetValue::EffectiveAppearance(_) => {}
        FacetValue::Brightness(value) => captured.brightness = Some(*value),
        FacetValue::Emission(value) => captured.emission = Some(*value),
        FacetValue::AppearanceSlots(slots) => {
            captured.appearance_slots = Some(slots.values.clone());
        }
    }
}

#[cfg(test)]
#[path = "scene_tests.rs"]
mod tests;
