// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Authoritative topology, desired state, observations, and mutation validation.
//!
//! `DaemonState` is the sole crate-facing type and composition root; its
//! concerns are split across sibling modules under `state/`.
//! This file keeps the struct itself, topology/target-group algebra, and the
//! cross-cutting orchestration methods that don't belong to a single concern.

pub(crate) mod collection;
mod frame_stream;
mod observation;
mod reconciliation;
pub(crate) mod scene;
pub(crate) mod target_state;
#[cfg(test)]
#[path = "tests/support.rs"]
mod tests_support;
mod validation;

use std::collections::{HashMap, HashSet};
use std::mem;
use std::sync::{self, Arc};

use luminate_core::capability::CapabilitySet;
use luminate_core::capability::CctEmulation;
use luminate_core::capability::ReadbackFidelity;
use luminate_core::collection::{Collection, CollectionId};
use luminate_core::device::{Device, DeviceId};
use luminate_core::element::ElementId;
use luminate_core::group::{Group, GroupId, GroupMember};
use luminate_core::scene::{Scene, SceneId};
use luminate_core::state::{
    AdoptedFacet, AdoptionStatus, AppearanceState, DeviceStateStatus, FacetObservation, FacetValue,
    ObservationConfidence, ObservationSource, StateFacetKind,
};
use luminate_core::surface::Surface;
use luminate_core::surface::SurfaceId;
use luminate_core::target::TargetId;
use luminate_plugin_api::{
    DeviceDescriptor, PluginFacetObservation, PluginReadError, PluginStateSnapshot,
};

use crate::error::DaemonError;
use crate::normalize::normalize_devices;
use crate::persistence;

use observation::{canonical_target_from_plugin, readable_fidelity, set_adoption_status};
use reconciliation::now_ms;
use target_state::TargetStateEntry;

#[derive(Debug, Default)]
pub struct DaemonState {
    devices: HashMap<DeviceId, Device>,

    /// Advances whenever authorization-relevant device topology changes.
    topology_generation: Arc<sync::atomic::AtomicU64>,

    /// Desired state overlays; see `state/target_state.rs`.
    target_state: Vec<TargetStateEntry>,

    /// User-created, cross-device target aggregates, keyed by id. See
    /// `state/collection.rs`.
    collections: HashMap<CollectionId, Collection>,

    /// Persistent sparse snapshots of intended state, keyed by id.
    scenes: HashMap<SceneId, Scene>,

    /// Confirmed hardware state promoted by `adopt`. This is deliberately not
    /// target state: adoption must never create an individual desired override.
    /// See `state/observation.rs`.
    adopted_baseline: Vec<AdoptedFacet>,

    /// Adoption becomes visible only after the whole persistence snapshot
    /// commits. See `state/observation.rs`.
    pending_adoption: Vec<AdoptedFacet>,

    /// See `state/observation.rs`.
    observations: HashMap<(TargetId, StateFacetKind), FacetObservation>,

    /// Shared between adoption bookkeeping (`state/observation.rs`) and
    /// reconciliation status (`state/reconciliation.rs`), which must observe
    /// the same per-device state.
    device_status: HashMap<DeviceId, DeviceStateStatus>,

    /// Device-mutation staleness counters. See `state/observation.rs`. Not to
    /// be confused with `FrameStream::generation`, an unrelated per-stream
    /// sequence counter local to `state/frame_stream.rs`.
    generations: HashMap<DeviceId, u64>,

    /// Active frame streams, keyed by target. Deliberately not part of
    /// `target_state`/persistence: a frame is a whole pixel array, not a
    /// representable `TargetState`, and persisting on every uploaded frame
    /// would mean a full state-file write per frame at streaming rates.
    /// Streamed pixels are hardware-only; `GetState`/persistence continue to
    /// reflect whatever was last set through an ordinary mutation. See
    /// `state/frame_stream.rs`.
    frame_streams: HashMap<TargetId, frame_stream::FrameStream>,

    /// Daemon-wide override for `Cct` emulation, from `DaemonConfig`. `None`
    /// leaves each target's own `CapabilitySet::cct_emulation` in effect. A
    /// target's explicit `CctEmulation::Disabled` always wins over this. See
    /// `state/validation.rs`.
    cct_emulation_override: Option<CctEmulation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TargetAtom {
    Surface(SurfaceId),
    Element {
        surface: SurfaceId,
        element: ElementId,
    },
}

impl DaemonState {
    pub(crate) fn ensure_persistable(&self) -> Result<(), DaemonError> {
        let adopted = self.adopted_baseline_for_persistence();
        persistence::validate_complete(
            &self.target_state,
            &self.collections,
            &self.scenes,
            &adopted,
        )
        .map_err(|error| DaemonError::ResourceLimit(error.to_string()))
    }

    pub fn from_devices(devices: Vec<Device>) -> Self {
        let mut device_map = HashMap::new();
        for device in devices {
            device_map.insert(device.id.clone(), device);
        }

        Self {
            devices: device_map,
            topology_generation: Arc::new(sync::atomic::AtomicU64::new(0)),
            target_state: Vec::new(),
            collections: HashMap::new(),
            scenes: HashMap::new(),
            adopted_baseline: Vec::new(),
            pending_adoption: Vec::new(),
            observations: HashMap::new(),
            device_status: HashMap::new(),
            generations: HashMap::new(),
            frame_streams: HashMap::new(),
            cct_emulation_override: None,
        }
    }

    pub fn from_descriptors(descriptors: &[DeviceDescriptor]) -> anyhow::Result<Self> {
        let devices = normalize_devices(descriptors)?;
        Ok(Self::from_devices(devices))
    }

    pub fn devices(&self) -> Vec<Device> {
        self.devices.values().cloned().collect()
    }

    /// Replaces the active topology while retaining state for withdrawn
    /// devices so it can be replayed if they return later.
    ///
    /// Entries targeting a still-present device are retained only when their
    /// full target and value remain valid against the replacement topology.
    /// This drops stale surfaces/elements/groups after a device-shape change
    /// without treating a temporarily absent device as permanently deleted.
    pub fn replace_devices_preserving_withdrawn_state(&mut self, devices: Vec<Device>) -> usize {
        self.topology_generation
            .fetch_add(1, sync::atomic::Ordering::AcqRel);
        self.devices = devices
            .into_iter()
            .map(|device| (device.id.clone(), device))
            .collect();

        let previous = mem::take(&mut self.target_state);
        let total = previous.len();
        self.target_state = previous
            .into_iter()
            .filter(|entry| {
                !self.devices.contains_key(entry.target.device_id())
                    || self
                        .validate_target_state(&entry.target, &entry.state)
                        .is_ok()
            })
            .collect();
        total - self.target_state.len()
    }

    /// Returns the generation of the topology used to resolve authorization resources.
    #[must_use]
    pub fn topology_generation(&self) -> u64 {
        self.topology_generation
            .load(sync::atomic::Ordering::Acquire)
    }

    /// Returns whether `generation` still describes the current topology.
    #[must_use]
    pub fn topology_generation_is(&self, generation: u64) -> bool {
        self.topology_generation() == generation
    }

    pub(crate) fn topology_generation_counter(&self) -> Arc<sync::atomic::AtomicU64> {
        Arc::clone(&self.topology_generation)
    }

    pub fn device(&self, id: &DeviceId) -> Option<Device> {
        self.devices.get(id).cloned()
    }

    #[must_use]
    pub fn capabilities_for_target(&self, target: &TargetId) -> Option<&CapabilitySet> {
        match target {
            TargetId::Device(device) => self.devices.get(device).map(|device| &device.capabilities),
            TargetId::Surface { device, surface } => self
                .devices
                .get(device)
                .and_then(|device| {
                    device
                        .surfaces
                        .iter()
                        .find(|candidate| &candidate.id == surface)
                })
                .map(|surface| &surface.capabilities),
            TargetId::Element {
                device,
                surface,
                element,
            } => self
                .devices
                .get(device)
                .and_then(|device| {
                    device
                        .surfaces
                        .iter()
                        .find(|candidate| &candidate.id == surface)
                })
                .and_then(|surface| {
                    surface
                        .elements
                        .iter()
                        .find(|candidate| &candidate.id == element)
                })
                .map(|element| &element.capabilities),
            TargetId::Group { device, group } => self
                .devices
                .get(device)
                .and_then(|device| {
                    device
                        .groups
                        .iter()
                        .find(|candidate| &candidate.id == group)
                })
                .map(|group| &group.capabilities),
        }
    }

    fn capabilities_for_target_or_error(
        &self,
        target: &TargetId,
    ) -> Result<&CapabilitySet, DaemonError> {
        self.capabilities_for_target(target)
            .ok_or_else(|| DaemonError::TargetNotFound(target.clone()))
    }

    /// Resolves `target` against the current topology at whatever scope it
    /// addresses (device/surface/element/group). Used both to reject
    /// mutations against targets that don't exist and to validate persisted
    /// entries for devices present in the active topology.
    pub(crate) fn ensure_target_exists(&self, target: &TargetId) -> Result<(), DaemonError> {
        if self.capabilities_for_target(target).is_some() {
            Ok(())
        } else {
            Err(DaemonError::TargetNotFound(target.clone()))
        }
    }

    pub(crate) fn canonical_observation_targets(&self, target: &TargetId) -> Vec<TargetId> {
        let TargetId::Group { device, .. } = target else {
            return vec![target.clone()];
        };
        self.target_atoms(target)
            .unwrap_or_default()
            .into_iter()
            .map(|atom| match atom {
                TargetAtom::Surface(surface) => TargetId::Surface {
                    device: device.clone(),
                    surface,
                },
                TargetAtom::Element { surface, element } => TargetId::Element {
                    device: device.clone(),
                    surface,
                    element,
                },
            })
            .collect()
    }

    /// Returns the canonical scopes whose observed state is changed by a
    /// successful write to `target`, including every covered descendant.
    pub(crate) fn observation_targets_after_write(&self, target: &TargetId) -> Vec<TargetId> {
        let Some(device) = self.devices.get(target.device_id()) else {
            return Vec::new();
        };
        let mut targets = Vec::new();
        let push_surface = |targets: &mut Vec<TargetId>, surface: &Surface| {
            targets.push(TargetId::Surface {
                device: device.id.clone(),
                surface: surface.id.clone(),
            });
            targets.extend(surface.elements.iter().map(|element| TargetId::Element {
                device: device.id.clone(),
                surface: surface.id.clone(),
                element: element.id.clone(),
            }));
        };

        match target {
            TargetId::Device(_) => {
                targets.push(target.clone());
                for surface in &device.surfaces {
                    push_surface(&mut targets, surface);
                }
            }
            TargetId::Surface { surface, .. } => {
                if let Some(surface) = device
                    .surfaces
                    .iter()
                    .find(|candidate| &candidate.id == surface)
                {
                    push_surface(&mut targets, surface);
                }
            }
            TargetId::Element { .. } => targets.push(target.clone()),
            TargetId::Group { .. } => {
                for atom in self.target_atoms(target).unwrap_or_default() {
                    match atom {
                        TargetAtom::Surface(surface_id) => {
                            if let Some(surface) = device
                                .surfaces
                                .iter()
                                .find(|candidate| candidate.id == surface_id)
                            {
                                push_surface(&mut targets, surface);
                            }
                        }
                        TargetAtom::Element { surface, element } => {
                            targets.push(TargetId::Element {
                                device: device.id.clone(),
                                surface,
                                element,
                            });
                        }
                    }
                }
            }
        }

        let mut deduplicated = Vec::with_capacity(targets.len());
        for target in targets {
            if !deduplicated.contains(&target) {
                deduplicated.push(target);
            }
        }
        deduplicated
    }

    pub(crate) fn target_covers(&self, covering: &TargetId, covered: &TargetId) -> bool {
        if covering == covered {
            return true;
        }
        if covering.device_id() != covered.device_id() {
            return false;
        }

        if matches!(covering, TargetId::Device(_)) {
            return true;
        }
        if matches!(covered, TargetId::Device(_)) {
            return false;
        }

        match (self.target_atoms(covering), self.target_atoms(covered)) {
            (Some(covering_atoms), Some(covered_atoms)) => covered_atoms.iter().all(|covered| {
                covering_atoms
                    .iter()
                    .any(|covering| target_atom_covers(covering, covered))
            }),
            _ => false,
        }
    }

    fn target_atoms(&self, target: &TargetId) -> Option<Vec<TargetAtom>> {
        match target {
            TargetId::Device(_) => None,
            TargetId::Surface { surface, .. } => Some(vec![TargetAtom::Surface(surface.clone())]),
            TargetId::Element {
                surface, element, ..
            } => Some(vec![TargetAtom::Element {
                surface: surface.clone(),
                element: element.clone(),
            }]),
            TargetId::Group { device, group } => {
                let device = self.devices.get(device)?;
                let group = device
                    .groups
                    .iter()
                    .find(|candidate| &candidate.id == group)?;
                let mut visited = HashSet::new();
                Some(group_atoms(device, group, &mut visited))
            }
        }
    }

    /// Validates the complete plugin snapshot before publishing any item, so a
    /// foreign or capability-inconsistent facet cannot partially corrupt state.
    pub fn accept_snapshot(
        &mut self,
        device: &DeviceId,
        expected_generation: u64,
        snapshot: PluginStateSnapshot,
        adopt: bool,
    ) -> Result<Vec<AdoptedFacet>, DaemonError> {
        if self.generation(device) != expected_generation {
            return Err(DaemonError::Internal(
                "discarded stale state snapshot after a newer mutation".to_owned(),
            ));
        }
        let PluginStateSnapshot {
            observations,
            errors,
        } = snapshot;
        let had_observations = !observations.is_empty();
        let validated = self.validate_snapshot(device, observations, &errors)?;

        let timestamp = now_ms();
        let mut adopted = Vec::new();
        for (target, mut value, fidelity) in validated {
            let confidence = match fidelity {
                ReadbackFidelity::Exact => ObservationConfidence::Confirmed,
                ReadbackFidelity::BestEffort => ObservationConfidence::BestEffort,
            };
            if let FacetValue::AppearanceSlots(slots) = &mut value {
                if let Some(FacetObservation {
                    value: FacetValue::AppearanceSlots(previous),
                    ..
                }) = self
                    .observations
                    .get(&(target.clone(), StateFacetKind::AppearanceSlots))
                {
                    for previous_value in &previous.values {
                        if !slots
                            .values
                            .iter()
                            .any(|value| value.slot == previous_value.slot)
                        {
                            slots.values.push(previous_value.clone());
                        }
                    }
                }
                slots.complete = self
                    .capabilities_for_target(&target)
                    .and_then(|capabilities| capabilities.appearance_slots.as_ref())
                    .is_some_and(|capability| slots.values.len() == capability.slots.len());
            }
            let facet = value.kind();
            self.observations.insert(
                (target.clone(), facet),
                FacetObservation {
                    target: target.clone(),
                    value: value.clone(),
                    confidence,
                    source: ObservationSource::Readback,
                    observed_at_ms: timestamp,
                    stale: false,
                },
            );
            if adopt && confidence == ObservationConfidence::Confirmed {
                adopted.push(AdoptedFacet {
                    target: target.clone(),
                    value,
                    confirmed_at_ms: timestamp,
                });
                let status = self.device_status_mut(device);
                set_adoption_status(status, target, facet, AdoptionStatus::Pending);
            } else if adopt {
                let status = self.device_status_mut(device);
                set_adoption_status(status, target, facet, AdoptionStatus::IneligibleFidelity);
            }
        }
        if errors.is_empty() {
            self.complete_reconciliation(device);
        } else {
            let diagnostic = errors
                .into_iter()
                .map(|error| error.diagnostic)
                .collect::<Vec<_>>()
                .join("; ");
            if had_observations {
                self.partial_reconciliation_failure(device, diagnostic);
            } else {
                self.fail_reconciliation(device, diagnostic);
            }
        }
        Ok(adopted)
    }

    fn validate_snapshot(
        &self,
        device: &DeviceId,
        observations: Vec<PluginFacetObservation>,
        errors: &[PluginReadError],
    ) -> Result<Vec<(TargetId, FacetValue, ReadbackFidelity)>, DaemonError> {
        for error in errors {
            let target = canonical_target_from_plugin(error.target.clone())?;
            if target.device_id() != device || self.capabilities_for_target(&target).is_none() {
                return Err(DaemonError::Internal(
                    "plugin snapshot error referenced a foreign target".to_owned(),
                ));
            }
        }
        let mut validated = Vec::new();
        let mut seen = HashSet::new();
        for observation in observations {
            let target = canonical_target_from_plugin(observation.target)?;
            if target.device_id() != device {
                return Err(DaemonError::Internal(
                    "plugin snapshot contained a foreign device target".to_owned(),
                ));
            }
            let capabilities = self.capabilities_for_target_or_error(&target)?;
            let facet = observation.value.kind();
            if !seen.insert((target.clone(), facet)) {
                return Err(DaemonError::Internal(
                    "plugin snapshot contained a duplicate facet".to_owned(),
                ));
            }
            let fidelity = readable_fidelity(capabilities, facet).ok_or_else(|| {
                DaemonError::Internal(format!(
                    "plugin reported unadvertised {facet:?} readback for {target:?}"
                ))
            })?;
            match &observation.value {
                FacetValue::Appearance(AppearanceState::Static(colour)) => {
                    self.validate_colour(&target, colour)?;
                }
                FacetValue::Appearance(AppearanceState::Effect(effect)) => {
                    self.validate_effect(&target, effect)?;
                }
                FacetValue::Appearance(AppearanceState::Mixed) => {
                    return Err(DaemonError::Internal(format!(
                        "plugin reported daemon-synthesized mixed appearance for {target:?}"
                    )));
                }
                FacetValue::Brightness(value) => self.validate_brightness(&target, *value)?,
                FacetValue::AppearanceSlots(slots) => {
                    let complete =
                        self.validate_observed_appearance_slot_values(&target, &slots.values)?;
                    if slots.complete != complete {
                        return Err(DaemonError::Internal(format!(
                            "plugin reported inconsistent appearance-slot completeness for {target:?}"
                        )));
                    }
                }
                FacetValue::Emission(_) if !capabilities.emission => {
                    return Err(DaemonError::Internal(format!(
                        "plugin reported emission for unsupported target {target:?}"
                    )));
                }
                FacetValue::PhysicalPower(_) if capabilities.physical_power.is_none() => {
                    return Err(DaemonError::Internal(format!(
                        "plugin reported physical power for unsupported target {target:?}"
                    )));
                }
                // `EffectiveAppearance` is unreachable here in practice: no
                // plugin capability ever advertises it as readable, so the
                // `readable_fidelity` lookup above already rejects it.
                // Handled here only for exhaustiveness.
                FacetValue::Emission(_)
                | FacetValue::PhysicalPower(_)
                | FacetValue::EffectiveAppearance(_) => {}
            }
            validated.push((target, observation.value, fidelity));
        }
        Ok(validated)
    }

    /// Permanently removes every retained state record for an absent device.
    ///
    /// Active devices are rejected: clearing their desired state has hardware
    /// semantics and must continue to use the ordinary `Clear` mutation path.
    /// Returns the number of persisted records removed.
    pub fn purge_withdrawn_device(&mut self, device: &DeviceId) -> Result<usize, DaemonError> {
        if self.devices.contains_key(device) {
            return Err(DaemonError::InvalidArgument {
                target: TargetId::Device(device.clone()),
                reason: "device is present; only withdrawn-device state can be purged".to_owned(),
            });
        }

        let mut removed = 0;
        let target_count = self.target_state.len();
        self.target_state
            .retain(|entry| entry.target.device_id() != device);
        removed += target_count - self.target_state.len();

        let adopted_count = self.adopted_baseline.len();
        self.adopted_baseline
            .retain(|facet| facet.target.device_id() != device);
        removed += adopted_count - self.adopted_baseline.len();

        let pending_count = self.pending_adoption.len();
        self.pending_adoption
            .retain(|facet| facet.target.device_id() != device);
        removed += pending_count - self.pending_adoption.len();

        self.observations
            .retain(|(target, _), _| target.device_id() != device);
        self.device_status.remove(device);
        self.generations.remove(device);

        if removed == 0 {
            return Err(DaemonError::TargetNotFound(TargetId::Device(
                device.clone(),
            )));
        }
        Ok(removed)
    }

    /// Returns the absent device identifiers for which purgeable state is
    /// retained, in stable lexical order.
    #[must_use]
    pub fn withdrawn_device_ids(&self) -> Vec<DeviceId> {
        let mut devices: HashSet<_> = self
            .target_state
            .iter()
            .map(|entry| entry.target.device_id().clone())
            .chain(
                self.adopted_baseline
                    .iter()
                    .map(|facet| facet.target.device_id().clone()),
            )
            .chain(
                self.pending_adoption
                    .iter()
                    .map(|facet| facet.target.device_id().clone()),
            )
            .filter(|device| !self.devices.contains_key(device))
            .collect();
        let mut devices: Vec<_> = devices.drain().collect();
        devices.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        devices
    }
}

fn group_atoms(device: &Device, group: &Group, visited: &mut HashSet<GroupId>) -> Vec<TargetAtom> {
    if !visited.insert(group.id.clone()) {
        return Vec::new();
    }

    let mut atoms = Vec::new();
    for member in &group.members {
        match member {
            GroupMember::Surface(surface) => atoms.push(TargetAtom::Surface(surface.clone())),
            GroupMember::Element { surface, element } => atoms.push(TargetAtom::Element {
                surface: surface.clone(),
                element: element.clone(),
            }),
            GroupMember::Group(group) => {
                if let Some(group) = device
                    .groups
                    .iter()
                    .find(|candidate| &candidate.id == group)
                {
                    atoms.extend(group_atoms(device, group, visited));
                }
            }
        }
    }

    atoms
}

fn target_atom_covers(covering: &TargetAtom, covered: &TargetAtom) -> bool {
    match (covering, covered) {
        (TargetAtom::Surface(covering), TargetAtom::Surface(covered)) => covering == covered,
        (
            TargetAtom::Surface(covering_surface),
            TargetAtom::Element {
                surface: covered_surface,
                ..
            },
        ) => covering_surface == covered_surface,
        (
            TargetAtom::Element {
                surface: covering_surface,
                element: covering_element,
            },
            TargetAtom::Element {
                surface: covered_surface,
                element: covered_element,
            },
        ) => covering_surface == covered_surface && covering_element == covered_element,
        (TargetAtom::Element { .. }, TargetAtom::Surface(_)) => false,
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
