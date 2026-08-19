// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Desired target state: the `TargetState`/`TargetStateEntry` types, and the
//! compaction logic that keeps `DaemonState::target_state` a minimal set of
//! non-overlapping overlays as new mutations and persisted restores arrive.

use std::mem;

use serde::{Deserialize, Serialize};

use luminate_core::appearance_slot::AppearanceSlotValue;
use luminate_core::capability::CapabilitySet;
use luminate_core::effect::Effect;
use luminate_core::state::FacetValue;
use luminate_core::target::TargetId;

use crate::error::DaemonError;

use super::DaemonState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TargetState {
    Effect(Effect),

    Brightness(u32),

    AppearanceSlots(Vec<AppearanceSlotValue>),

    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TargetStateFacets(u8);

impl TargetStateFacets {
    const APPEARANCE: u8 = 1 << 0;
    const BRIGHTNESS: u8 = 1 << 1;
    const EMISSION: u8 = 1 << 2;
    const PHYSICAL_POWER: u8 = 1 << 3;
    const APPEARANCE_SLOTS: u8 = 1 << 4;
    const ALL: Self = Self(
        Self::APPEARANCE
            | Self::BRIGHTNESS
            | Self::EMISSION
            | Self::PHYSICAL_POWER
            | Self::APPEARANCE_SLOTS,
    );

    const fn is_covered_by(self, newer: Self) -> bool {
        self.0 & !newer.0 == 0
    }
}

impl TargetState {
    fn facets(&self, capabilities: Option<&CapabilitySet>) -> TargetStateFacets {
        let has_power = capabilities.is_some_and(|capabilities| {
            capabilities.physical_power.is_some() || capabilities.power_domain.is_some()
        });
        let power = u8::from(has_power) * TargetStateFacets::PHYSICAL_POWER;
        match self {
            Self::Effect(Effect::Off) => TargetStateFacets(TargetStateFacets::EMISSION | power),
            Self::Effect(_) => TargetStateFacets(
                TargetStateFacets::APPEARANCE | TargetStateFacets::EMISSION | power,
            ),
            Self::Brightness(value) => TargetStateFacets(
                TargetStateFacets::BRIGHTNESS
                    | power
                    | if *value == 0 {
                        TargetStateFacets::EMISSION
                    } else {
                        0
                    },
            ),
            Self::AppearanceSlots(_) => TargetStateFacets(TargetStateFacets::APPEARANCE_SLOTS),
            Self::Clear => TargetStateFacets::ALL,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetStateEntry {
    pub target: TargetId,

    pub state: TargetState,
}

impl DaemonState {
    pub fn set_effect(&mut self, target: TargetId, effect: Effect) -> Result<(), DaemonError> {
        self.ensure_target_exists(&target)?;
        let state = TargetState::Effect(effect);
        self.note_successful_apply(&target, &state);
        self.insert_target_state_checked(target, state)
    }

    pub fn set_brightness(&mut self, target: TargetId, value: u32) -> Result<(), DaemonError> {
        self.ensure_target_exists(&target)?;
        let state = TargetState::Brightness(value);
        self.note_successful_apply(&target, &state);
        self.insert_target_state_checked(target, state)
    }

    pub fn set_appearance_slots(
        &mut self,
        target: TargetId,
        values: Vec<AppearanceSlotValue>,
    ) -> Result<(), DaemonError> {
        self.ensure_target_exists(&target)?;
        let mut merged = self.known_appearance_slot_values(&target);
        for value in values {
            if let Some(existing) = merged.iter_mut().find(|known| known.slot == value.slot) {
                *existing = value;
            } else {
                merged.push(value);
            }
        }
        let state = TargetState::AppearanceSlots(merged);
        self.note_successful_apply(&target, &state);
        self.insert_target_state_checked(target, state)
    }

    pub(crate) fn known_appearance_slot_values(
        &self,
        target: &TargetId,
    ) -> Vec<AppearanceSlotValue> {
        let mut known = self
            .adopted_baseline
            .iter()
            .filter(|facet| &facet.target == target)
            .filter_map(|facet| match &facet.value {
                FacetValue::AppearanceSlots(slots) => Some(slots.values.clone()),
                FacetValue::Appearance(_)
                | FacetValue::Brightness(_)
                | FacetValue::Emission(_)
                | FacetValue::PhysicalPower(_)
                | FacetValue::EffectiveAppearance(_) => None,
            })
            .flatten()
            .collect::<Vec<_>>();

        for entry in self
            .target_state
            .iter()
            .filter(|entry| &entry.target == target)
        {
            match &entry.state {
                TargetState::AppearanceSlots(values) => known.clone_from(values),
                TargetState::Clear => known.clear(),
                TargetState::Effect(_) | TargetState::Brightness(_) => {}
            }
        }
        known
    }

    pub fn clear_target(&mut self, target: &TargetId) -> Result<(), DaemonError> {
        self.ensure_target_exists(target)?;
        let state = TargetState::Clear;
        self.note_successful_apply(target, &state);
        self.insert_target_state_checked(target.clone(), state)
    }

    pub fn ensure_target_state_known(&self, target: &TargetId) -> Result<(), DaemonError> {
        self.ensure_target_exists(target)?;
        if self
            .target_state
            .iter()
            .rev()
            .find(|entry| &entry.target == target)
            .is_some_and(|entry| !matches!(entry.state, TargetState::Clear))
        {
            return Ok(());
        }

        Err(DaemonError::UnknownState {
            target: target.clone(),
        })
    }

    #[must_use]
    pub fn target_states(&self) -> &[TargetStateEntry] {
        &self.target_state
    }

    pub(crate) fn insert_target_state(&mut self, target: TargetId, target_state: TargetState) {
        // Appearance and independently advertised brightness compose in hardware,
        // so compaction is only valid within a single facet.
        //
        // `Clear` acts as an ordered barrier across all facets. A newer `Clear`
        // replaces every operation it covers, but an older one cannot be removed:
        // replay still needs it to reset facets that subsequent colour/effect or
        // brightness updates left unchanged.
        let new_facets = target_state.facets(self.capabilities_for_target(&target));
        let previous = mem::take(&mut self.target_state);
        let retained = previous
            .into_iter()
            .filter(|entry| {
                !self.target_covers(&target, &entry.target)
                    || (new_facets != TargetStateFacets::ALL
                        && !entry
                            .state
                            .facets(self.capabilities_for_target(&entry.target))
                            .is_covered_by(new_facets))
            })
            .collect();
        self.target_state = retained;
        self.target_state.push(TargetStateEntry {
            target,
            state: target_state,
        });
    }

    pub(crate) fn insert_target_state_checked(
        &mut self,
        target: TargetId,
        target_state: TargetState,
    ) -> Result<(), DaemonError> {
        let previous = self.target_state.clone();
        self.insert_target_state(target, target_state);
        if let Err(error) = self.ensure_persistable() {
            self.target_state = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Restores previously persisted target state.
    ///
    /// Discards entries whose targets no longer exist in the current
    /// topology (for example after a plugin or device is removed).
    ///
    /// Returns the number of entries restored and dropped, respectively.
    pub fn restore_persisted(
        &mut self,
        persisted: Vec<TargetStateEntry>,
        preserve_order: bool,
    ) -> (usize, usize) {
        let total = persisted.len();
        let mut restored = 0;

        let mut persisted = persisted;
        if !preserve_order {
            persisted.sort_by_key(|entry| target_restore_order(&entry.target));
        }

        for entry in persisted {
            if self
                .validate_target_state(&entry.target, &entry.state)
                .is_ok()
            {
                self.insert_target_state(entry.target, entry.state);
                restored += 1;
            }
        }

        (restored, total - restored)
    }

    /// Restores valid active state and preserves entries for device IDs that
    /// are absent from the startup topology.
    ///
    /// Those missing entries may correspond to temporarily offline dynamic
    /// devices, so they are retained to avoid losing their persisted state
    /// during unrelated future saves.
    pub fn restore_persisted_retaining_withdrawn(
        &mut self,
        persisted: Vec<TargetStateEntry>,
        preserve_order: bool,
    ) -> (usize, usize, usize) {
        let total = persisted.len();
        let mut active = Vec::new();
        let mut withdrawn = Vec::new();

        for entry in persisted {
            if self.devices.contains_key(entry.target.device_id()) {
                active.push(entry);
            } else {
                withdrawn.push(entry);
            }
        }

        let withdrawn_count = withdrawn.len();
        let (restored, dropped) = self.restore_persisted(active, preserve_order);
        self.target_state.extend(withdrawn);
        debug_assert_eq!(
            total,
            restored + dropped + withdrawn_count,
            "restore accounting must preserve the total number of persisted entries"
        );
        (restored, withdrawn_count, dropped)
    }
}

fn target_restore_order(target: &TargetId) -> u8 {
    match target {
        TargetId::Element { .. } => 0,
        TargetId::Surface { .. } => 1,
        TargetId::Group { .. } => 2,
        TargetId::Device(_) => 3,
    }
}

#[cfg(test)]
#[path = "target_state_tests.rs"]
mod tests;
