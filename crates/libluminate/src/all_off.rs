// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Wear-safe planning for turning off every drivable part of a device.

use luminate_core::capability::{CapabilitySet, PersistenceCapability, PersistenceRequirement};

use crate::{Device, TargetId};

/// A wear-safe set of maximal, non-overlapping targets for an all-off action.
///
/// Targets whose off operation may require a persistent write are reported
/// separately so callers can avoid unnecessary writes to wear-limited storage.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AllOffPlan {
    targets: Vec<TargetId>,
    skipped_persistent: Vec<TargetId>,
}

impl AllOffPlan {
    /// Returns the targets that can safely be turned off.
    #[must_use]
    pub fn targets(&self) -> &[TargetId] {
        &self.targets
    }

    /// Returns off-capable targets skipped because turning them off may
    /// require a wear-sensitive persistent write.
    #[must_use]
    pub fn skipped_persistent(&self) -> &[TargetId] {
        &self.skipped_persistent
    }

    fn add(&mut self, target: TargetId, capabilities: &CapabilitySet) {
        if capabilities_are_wear_limited(capabilities) {
            self.skipped_persistent.push(target);
        } else {
            self.targets.push(target);
        }
    }
}

/// Selects maximal, non-overlapping off-capable targets for `device`.
///
/// Groups are excluded because their members can overlap arbitrary topology
/// branches. A target whose off operation is not wear-safe is placed in
/// [`AllOffPlan::skipped_persistent`] instead of the executable target set.
#[must_use]
pub fn all_off_plan(device: &Device) -> AllOffPlan {
    let device_target = TargetId::Device(device.id.clone());
    if capabilities_can_turn_off(&device.capabilities) {
        let mut plan = AllOffPlan::default();
        plan.add(device_target, &device.capabilities);
        return plan;
    }

    let mut plan = AllOffPlan::default();
    for surface in &device.surfaces {
        let surface_target = TargetId::surface(device.id.as_str(), surface.id.as_str());
        if capabilities_can_turn_off(&surface.capabilities) {
            plan.add(surface_target, &surface.capabilities);
            continue;
        }

        for element in &surface.elements {
            if capabilities_can_turn_off(&element.capabilities) {
                plan.add(
                    TargetId::element(device.id.as_str(), surface.id.as_str(), element.id.as_str()),
                    &element.capabilities,
                );
            }
        }
    }
    plan
}

fn capabilities_can_turn_off(capabilities: &CapabilitySet) -> bool {
    !capabilities.colour.is_empty()
        || capabilities
            .hardware_effects
            .as_ref()
            .is_some_and(|effects| {
                effects
                    .effects
                    .iter()
                    .any(|effect| effect.id.as_str() == "off")
            })
}

fn capabilities_are_wear_limited(capabilities: &CapabilitySet) -> bool {
    !capabilities.off_is_wear_safe
        && matches!(
            capabilities.persistence,
            PersistenceCapability::CurrentState {
                requirement: PersistenceRequirement::Required,
                ..
            } | PersistenceCapability::Profiles {
                requirement: PersistenceRequirement::Required,
                ..
            }
        )
}

#[cfg(test)]
#[path = "all_off_tests.rs"]
mod tests;
