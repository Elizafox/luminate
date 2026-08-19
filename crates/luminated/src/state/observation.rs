// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Hardware observations, adoption bookkeeping, per-device generations, and
//! the assumptions the daemon projects into `observations` after a
//! successful mutation apply.

use std::collections::{HashMap, HashSet};
use std::mem;

use luminate_core::capability::{
    CapabilitySet, PowerDomainRef, ReadbackFidelity, StateReadbackCapability,
};
use luminate_core::collection::{CollectionId, CollectionMember};
use luminate_core::device::{Device, DeviceId};
use luminate_core::effect::Effect;
use luminate_core::group::{Group, GroupId, GroupMember};
use luminate_core::state::{
    AdoptedFacet, AdoptionStatus, AggregateAppearanceObservation,
    AggregateEffectiveAppearanceObservation, AppearanceSlotsState, AppearanceState,
    CollectionStateStatus, DeviceStateStatus, EffectiveAppearanceState, EmissionState,
    FacetObservation, FacetValue, ObservationConfidence, ObservationSource, PhysicalPowerState,
    Reachability, ReconciliationStatus, StateFacetKind,
};
use luminate_core::target::TargetId;
use luminate_plugin_api::{
    PluginReadRequest, PluginReadTarget, PluginTarget, PluginUpdateOperation,
};

use crate::error::DaemonError;

use super::DaemonState;
use super::reconciliation::now_ms;
use super::target_state::TargetState;

impl DaemonState {
    #[must_use]
    pub fn adopted_baseline(&self) -> &[AdoptedFacet] {
        &self.adopted_baseline
    }

    pub fn restore_adopted_baseline(&mut self, adopted: Vec<AdoptedFacet>) {
        self.adopted_baseline = adopted
            .into_iter()
            .filter(|facet| {
                !self.devices.contains_key(facet.target.device_id())
                    || self.capabilities_for_target(&facet.target).is_some()
            })
            .collect();

        for facet in &self.adopted_baseline {
            self.observations.insert(
                (facet.target.clone(), facet.value.kind()),
                FacetObservation {
                    target: facet.target.clone(),
                    value: facet.value.clone(),
                    confidence: ObservationConfidence::Confirmed,
                    source: ObservationSource::AdoptedBaseline,
                    observed_at_ms: facet.confirmed_at_ms,
                    stale: true,
                },
            );
        }
    }

    /// Marks every observation stale, returning the number that were
    /// previously fresh.
    ///
    /// Called when the daemon loses its basis for believing its observations
    /// still reflect the current hardware state; at present, immediately
    /// before system suspend. Hardware may be power-cycled, reset, or changed
    /// by another controller while the machine is asleep, so continuing to
    /// present pre-suspend observations as current would mislead the user.
    ///
    /// Observations are deliberately marked stale rather than erased,
    /// mirroring [`Self::restore_persisted_retaining_withdrawn`] after a
    /// process restart. A stale observation is still the best available
    /// knowledge until a fresh readback replaces it.
    ///
    /// Desired state and the adopted baseline are left untouched. This affects
    /// only what the daemon can truthfully claim to know, not what the user
    /// wants the hardware to be doing.
    pub fn mark_observations_stale(&mut self) -> usize {
        let mut marked = 0;
        // Order is irrelevant; every entry gets the same treatment and the
        // only output is a count, so the map's arbitrary iteration order
        // cannot affect the result.
        #[allow(
            clippy::iter_over_hash_type,
            reason = "sets one flag on every entry; iteration order cannot affect the outcome"
        )]
        for observation in self.observations.values_mut() {
            if !observation.stale {
                observation.stale = true;
                marked += 1;
            }
        }
        marked
    }

    #[must_use]
    pub fn merged_adopted_baseline(&self, delta: &[AdoptedFacet]) -> Vec<AdoptedFacet> {
        let mut merged = self.adopted_baseline.clone();
        for facet in delta {
            replace_adopted_facet(&mut merged, facet.clone());
        }
        merged
    }

    pub fn commit_adopted_baseline(&mut self, delta: &[AdoptedFacet]) {
        self.adopted_baseline = self.merged_adopted_baseline(delta);
        for facet in delta {
            let device = facet.target.device_id().clone();
            let status = self.device_status_mut(&device);
            set_adoption_status(
                status,
                facet.target.clone(),
                facet.value.kind(),
                AdoptionStatus::Durable,
            );
        }
    }

    pub fn stage_adoption(&mut self, delta: Vec<AdoptedFacet>) -> Result<(), DaemonError> {
        let previous = self.pending_adoption.clone();
        for facet in delta {
            replace_adopted_facet(&mut self.pending_adoption, facet);
        }
        if let Err(error) = self.ensure_persistable() {
            self.pending_adoption = previous;
            return Err(error);
        }
        Ok(())
    }

    #[must_use]
    pub fn adopted_baseline_for_persistence(&self) -> Vec<AdoptedFacet> {
        self.merged_adopted_baseline(&self.pending_adoption)
    }

    pub fn finish_pending_adoption(&mut self, persisted: bool) {
        let pending = mem::take(&mut self.pending_adoption);
        if persisted {
            self.commit_adopted_baseline(&pending);
            return;
        }
        for facet in pending {
            let device = facet.target.device_id().clone();
            let status = self.device_status_mut(&device);
            let kind = facet.value.kind();
            set_adoption_status(
                status,
                facet.target,
                kind,
                AdoptionStatus::PersistenceFailed,
            );
        }
    }

    #[must_use]
    pub fn generation(&self, device: &DeviceId) -> u64 {
        self.generations.get(device).copied().unwrap_or(0)
    }

    /// Returns the retained configured appearance, including a stale
    /// observation. Restoration deliberately treats stale as last-known
    /// configuration rather than as current hardware truth.
    #[must_use]
    pub(crate) fn retained_appearance(&self, target: &TargetId) -> Option<AppearanceState> {
        self.observations
            .get(&(target.clone(), StateFacetKind::Appearance))
            .and_then(|observation| match &observation.value {
                FacetValue::Appearance(appearance) => Some(appearance.clone()),
                FacetValue::Brightness(_)
                | FacetValue::Emission(_)
                | FacetValue::PhysicalPower(_)
                | FacetValue::EffectiveAppearance(_)
                | FacetValue::AppearanceSlots(_) => None,
            })
    }

    #[must_use]
    pub fn device_state_status(&self, device: &DeviceId) -> Option<DeviceStateStatus> {
        let topology = self.devices.get(device)?;
        let mut status = self
            .device_status
            .get(device)
            .cloned()
            .unwrap_or_else(|| empty_device_status(device.clone()));
        status.observations = self
            .observations
            .values()
            .filter(|observation| observation.target.device_id() == device)
            .cloned()
            .collect();

        let mut streaming_targets_covered = HashSet::new();
        let mut synthesized: Vec<FacetObservation> = status
            .observations
            .iter()
            .filter_map(|observation| {
                let FacetValue::Appearance(appearance) = &observation.value else {
                    return None;
                };
                if self.is_frame_streaming(&observation.target) {
                    streaming_targets_covered.insert(observation.target.clone());
                    return Some(streaming_effective_appearance_observation(
                        &observation.target,
                    ));
                }
                let emission = self
                    .observations
                    .get(&(observation.target.clone(), StateFacetKind::Emission))?;

                let FacetValue::Emission(emission_state) = &emission.value else {
                    return None;
                };

                Some(effective_appearance_observation(
                    appearance,
                    *emission_state,
                    observation,
                    emission,
                ))
            })
            .collect();

        // A stream's own target may never have had an `Appearance` observation
        // at all (for example a write-only plugin with no readback capability),
        // in which case the loop above never visits it. Report it directly so
        // "is this target streaming" doesn't depend on incidental readback.
        synthesized.extend(
            self.frame_streams
                .keys()
                .filter(|target| {
                    target.device_id() == device && !streaming_targets_covered.contains(*target)
                })
                .map(streaming_effective_appearance_observation),
        );

        status.observations.extend(synthesized);
        synthesize_device_aggregates(topology, &mut status.observations);

        // Stable ordering for display/snapshot purposes only. Consumers must
        // look up a specific facet with `DeviceStateStatus::observation`,
        // never by position, as observation order is not total.
        status.observations.sort_by(|left, right| {
            target_sort_key(&left.target)
                .cmp(&target_sort_key(&right.target))
                .then_with(|| (left.value.kind() as u8).cmp(&(right.value.kind() as u8)))
        });

        Some(status)
    }

    /// Returns configured and effective appearance synthesized across one
    /// collection's transitive membership.
    #[must_use]
    pub fn collection_state_status(
        &self,
        collection: &CollectionId,
    ) -> Option<CollectionStateStatus> {
        self.collections.get(collection)?;

        let statuses: HashMap<_, _> = self
            .devices
            .keys()
            .filter_map(|device| {
                self.device_state_status(device)
                    .map(|status| (device.clone(), status))
            })
            .collect();
        let mut configured = HashMap::new();
        let mut effective = HashMap::new();
        let mut visiting = HashSet::new();
        let appearance = self.collection_aggregate(
            collection,
            StateFacetKind::Appearance,
            &statuses,
            &mut configured,
            &mut visiting,
        );
        visiting.clear();
        let effective_appearance = self.collection_aggregate(
            collection,
            StateFacetKind::EffectiveAppearance,
            &statuses,
            &mut effective,
            &mut visiting,
        );

        Some(CollectionStateStatus {
            collection: collection.clone(),
            appearance: appearance.and_then(collection_appearance_observation),
            effective_appearance: effective_appearance
                .and_then(collection_effective_appearance_observation),
        })
    }

    fn collection_aggregate(
        &self,
        id: &CollectionId,
        facet: StateFacetKind,
        statuses: &HashMap<DeviceId, DeviceStateStatus>,
        memo: &mut HashMap<CollectionId, Option<FacetObservation>>,
        visiting: &mut HashSet<CollectionId>,
    ) -> Option<FacetObservation> {
        if let Some(cached) = memo.get(id) {
            return cached.clone();
        }
        if !visiting.insert(id.clone()) {
            return None;
        }
        let collection = self.collections.get(id)?;
        let constituents: Vec<_> = collection
            .members
            .iter()
            .map(|member| match member {
                CollectionMember::Target(target) => statuses
                    .get(target.device_id())
                    .and_then(|status| status.observation(target, facet))
                    .cloned(),
                CollectionMember::Collection(nested) => {
                    self.collection_aggregate(nested, facet, statuses, memo, visiting)
                }
            })
            .collect();
        visiting.remove(id);
        let aggregate = aggregate_observations(&constituents, None);
        memo.insert(id.clone(), aggregate.clone());
        aggregate
    }

    #[must_use]
    pub fn confirmed_observations_match(
        &self,
        operations: &[(TargetId, PluginUpdateOperation)],
    ) -> bool {
        operations.iter().all(|(target, operation)| {
            let expected = match operation {
                PluginUpdateOperation::SetEffect {
                    effect: Effect::Off,
                } => Some(FacetValue::Emission(EmissionState::Dark)),
                PluginUpdateOperation::SetEffect {
                    effect: Effect::Static { colour },
                } => Some(FacetValue::Appearance(AppearanceState::Static(
                    colour.clone(),
                ))),
                PluginUpdateOperation::SetEffect { effect } => Some(FacetValue::Appearance(
                    AppearanceState::Effect(effect.clone()),
                )),
                PluginUpdateOperation::SetBrightness { value } => {
                    Some(FacetValue::Brightness(*value))
                }
                PluginUpdateOperation::SetAppearanceSlots { values } => {
                    Some(FacetValue::AppearanceSlots(AppearanceSlotsState {
                        values: values.clone(),
                        complete: self
                            .capabilities_for_target(target)
                            .and_then(|capabilities| capabilities.appearance_slots.as_ref())
                            .is_some_and(|capability| values.len() == capability.slots.len()),
                    }))
                }
                PluginUpdateOperation::Clear | PluginUpdateOperation::SaveCurrent => None,
            };

            let Some(expected) = expected else {
                return true;
            };

            self.observations
                .get(&(target.clone(), expected.kind()))
                .filter(|observation| observation.confidence == ObservationConfidence::Confirmed)
                .is_none_or(|observation| observation.value == expected)
        })
    }

    #[must_use]
    pub fn read_request(&self, device: &DeviceId) -> PluginReadRequest {
        let Some(device_state) = self.devices.get(device) else {
            return PluginReadRequest {
                targets: Vec::new(),
            };
        };
        let mut targets = Vec::new();
        push_read_target(
            &mut targets,
            PluginTarget::Device {
                device: device.as_str().to_owned(),
            },
            &device_state.capabilities,
        );

        for surface in &device_state.surfaces {
            push_read_target(
                &mut targets,
                PluginTarget::Surface {
                    device: device.as_str().to_owned(),
                    surface: surface.id.as_str().to_owned(),
                },
                &surface.capabilities,
            );

            for element in &surface.elements {
                push_read_target(
                    &mut targets,
                    PluginTarget::Element {
                        device: device.as_str().to_owned(),
                        surface: surface.id.as_str().to_owned(),
                        element: element.id.as_str().to_owned(),
                    },
                    &element.capabilities,
                );
            }
        }
        PluginReadRequest { targets }
    }

    pub fn reserve_generation(&mut self, device: DeviceId) {
        let generation = self.generations.entry(device).or_default();
        *generation = generation.wrapping_add(1);
    }

    pub fn note_successful_operation(
        &mut self,
        target: &TargetId,
        operation: &PluginUpdateOperation,
    ) {
        let state = match operation {
            PluginUpdateOperation::SetEffect { effect } => TargetState::Effect(effect.clone()),
            PluginUpdateOperation::SetBrightness { value } => TargetState::Brightness(*value),
            PluginUpdateOperation::Clear => TargetState::Clear,
            PluginUpdateOperation::SetAppearanceSlots { values } => {
                TargetState::AppearanceSlots(values.clone())
            }
            PluginUpdateOperation::SaveCurrent => return,
        };
        self.note_successful_apply(target, &state);
    }

    pub(crate) fn note_successful_apply(&mut self, target: &TargetId, state: &TargetState) {
        if !matches!(target, TargetId::Element { .. }) {
            for canonical in self.observation_targets_after_write(target) {
                self.note_successful_apply_at_canonical_target(&canonical, state);
            }
            return;
        }
        self.note_successful_apply_at_canonical_target(target, state);
    }

    fn note_successful_apply_at_canonical_target(
        &mut self,
        target: &TargetId,
        state: &TargetState,
    ) {
        if matches!(state, TargetState::Clear) {
            let invalidated = self
                .observations
                .keys()
                .filter(|(observed, _)| self.target_covers(target, observed))
                .cloned()
                .collect::<Vec<_>>();
            for key in invalidated {
                self.observations.remove(&key);
            }
            return;
        }
        let timestamp = now_ms();
        let capabilities = self.capabilities_for_target(target);
        let power_domain = capabilities
            .and_then(|capabilities| capabilities.power_domain.clone())
            .map(|domain| match domain {
                PowerDomainRef::Device => TargetId::Device(target.device_id().clone()),
                PowerDomainRef::Surface { surface } => {
                    TargetId::surface(target.device_id().as_str(), surface)
                }
            });
        let owns_power_domain =
            capabilities.is_some_and(|capabilities| capabilities.physical_power.is_some());
        let appearance_slot_count = capabilities
            .and_then(|capabilities| capabilities.appearance_slots.as_ref())
            .map(|capability| capability.slots.len());
        for mut value in projected_observations(state, owns_power_domain) {
            if let FacetValue::AppearanceSlots(slots) = &mut value {
                slots.complete = appearance_slot_count == Some(slots.values.len());
            }
            self.observations.insert(
                (target.clone(), value.kind()),
                FacetObservation {
                    target: target.clone(),
                    value,
                    confidence: ObservationConfidence::Assumed,
                    source: ObservationSource::SuccessfulApply,
                    observed_at_ms: timestamp,
                    stale: false,
                },
            );
        }
        if let Some(domain) = power_domain {
            let domain_is_target = &domain == target;
            let turns_domain_off =
                domain_is_target && matches!(state, TargetState::Effect(Effect::Off));
            if !turns_domain_off {
                self.observations.insert(
                    (domain.clone(), StateFacetKind::PhysicalPower),
                    FacetObservation {
                        target: domain,
                        value: FacetValue::PhysicalPower(PhysicalPowerState::On),
                        confidence: ObservationConfidence::Assumed,
                        source: ObservationSource::Derived,
                        observed_at_ms: timestamp,
                        stale: false,
                    },
                );
            }
        }
    }
}

fn projected_observations(state: &TargetState, owns_power_domain: bool) -> Vec<FacetValue> {
    match state {
        TargetState::Effect(Effect::Off) => {
            let mut values = vec![FacetValue::Emission(EmissionState::Dark)];
            if owns_power_domain {
                values.push(FacetValue::PhysicalPower(PhysicalPowerState::Off));
            }
            values
        }
        TargetState::Effect(effect) => {
            let emission = match effect {
                Effect::Off => EmissionState::Dark,
                Effect::Static { colour } if colour.is_dark() => EmissionState::Dark,
                Effect::Static { .. }
                | Effect::Breathe { .. }
                | Effect::Pulse { .. }
                | Effect::Strobe { .. }
                | Effect::Scanner { .. }
                | Effect::Morph { .. }
                | Effect::Spectrum { .. }
                | Effect::Rainbow { .. }
                | Effect::Hardware { .. } => EmissionState::Emitting,
            };
            vec![
                FacetValue::Appearance(match effect {
                    Effect::Static { colour } => AppearanceState::Static(colour.clone()),
                    Effect::Off
                    | Effect::Breathe { .. }
                    | Effect::Pulse { .. }
                    | Effect::Strobe { .. }
                    | Effect::Scanner { .. }
                    | Effect::Morph { .. }
                    | Effect::Spectrum { .. }
                    | Effect::Rainbow { .. }
                    | Effect::Hardware { .. } => AppearanceState::Effect(effect.clone()),
                }),
                FacetValue::Emission(emission),
            ]
        }
        TargetState::Brightness(value) if *value == 0 => vec![
            FacetValue::Brightness(*value),
            FacetValue::Emission(EmissionState::Dark),
        ],
        TargetState::Brightness(value) => vec![FacetValue::Brightness(*value)],
        TargetState::AppearanceSlots(values) => {
            vec![FacetValue::AppearanceSlots(AppearanceSlotsState {
                values: values.clone(),
                complete: false,
            })]
        }
        TargetState::Clear => Vec::new(),
    }
}

/// Combines a target's `Appearance` and `Emission` observations into a
/// derived `EffectiveAppearance` observation. Confidence and freshness follow
/// the weaker of the two inputs, since the combined fact is only as sound as
/// its least-supported half.
fn effective_appearance_observation(
    appearance: &AppearanceState,
    emission: EmissionState,
    appearance_observation: &FacetObservation,
    emission_observation: &FacetObservation,
) -> FacetObservation {
    let effective = match emission {
        EmissionState::Dark => EffectiveAppearanceState::Off,
        EmissionState::Emitting => match appearance {
            AppearanceState::Static(colour) => EffectiveAppearanceState::Static(colour.clone()),
            AppearanceState::Effect(effect) => EffectiveAppearanceState::Effect(effect.clone()),
            AppearanceState::Mixed => EffectiveAppearanceState::Mixed,
        },
    };
    FacetObservation {
        target: appearance_observation.target.clone(),
        value: FacetValue::EffectiveAppearance(effective),
        confidence: appearance_observation
            .confidence
            .min(emission_observation.confidence),
        source: ObservationSource::Derived,
        observed_at_ms: appearance_observation
            .observed_at_ms
            .min(emission_observation.observed_at_ms),
        stale: appearance_observation.stale || emission_observation.stale,
    }
}

/// Reports `EffectiveAppearance::Streaming` for a target with an active
/// frame stream, in place of the `Appearance`/`Emission`-derived value: an
/// active stream's pixel content is unknown to the daemon and unrelated to
/// the target's last-configured `Appearance`, regardless of `Emission`.
/// Confidence is `Confirmed` because stream occupancy is authoritative
/// in-memory daemon state, not a readback or derivation from one.
fn streaming_effective_appearance_observation(target: &TargetId) -> FacetObservation {
    FacetObservation {
        target: target.clone(),
        value: FacetValue::EffectiveAppearance(EffectiveAppearanceState::Streaming),
        confidence: ObservationConfidence::Confirmed,
        source: ObservationSource::Derived,
        observed_at_ms: now_ms(),
        stale: false,
    }
}

fn synthesize_device_aggregates(device: &Device, observations: &mut Vec<FacetObservation>) {
    let mut by_facet: HashMap<(TargetId, StateFacetKind), FacetObservation> = observations
        .drain(..)
        .map(|observation| {
            (
                (observation.target.clone(), observation.value.kind()),
                observation,
            )
        })
        .collect();

    for facet in [
        StateFacetKind::Appearance,
        StateFacetKind::EffectiveAppearance,
    ] {
        for surface in &device.surfaces {
            let target = TargetId::Surface {
                device: device.id.clone(),
                surface: surface.id.clone(),
            };
            let children: Vec<_> = surface
                .elements
                .iter()
                .filter(|element| appearance_capable(&element.capabilities))
                .map(|element| {
                    by_facet
                        .get(&(
                            TargetId::Element {
                                device: device.id.clone(),
                                surface: surface.id.clone(),
                                element: element.id.clone(),
                            },
                            facet,
                        ))
                        .cloned()
                })
                .collect();
            replace_with_aggregate(&mut by_facet, target, facet, &children);
        }

        let mut group_memo = HashMap::new();
        for group in &device.groups {
            synthesize_group(device, group, facet, &mut by_facet, &mut group_memo);
        }

        let target = TargetId::Device(device.id.clone());
        let children: Vec<_> = device
            .surfaces
            .iter()
            .filter(|surface| {
                appearance_capable(&surface.capabilities)
                    || surface
                        .elements
                        .iter()
                        .any(|element| appearance_capable(&element.capabilities))
            })
            .map(|surface| {
                by_facet
                    .get(&(
                        TargetId::Surface {
                            device: device.id.clone(),
                            surface: surface.id.clone(),
                        },
                        facet,
                    ))
                    .cloned()
            })
            .collect();
        replace_with_aggregate(&mut by_facet, target, facet, &children);
    }

    observations.extend(by_facet.into_values());
}

fn synthesize_group(
    device: &Device,
    group: &Group,
    facet: StateFacetKind,
    observations: &mut HashMap<(TargetId, StateFacetKind), FacetObservation>,
    memo: &mut HashMap<GroupId, Option<FacetObservation>>,
) -> Option<FacetObservation> {
    if let Some(cached) = memo.get(&group.id) {
        return cached.clone();
    }

    let target = TargetId::Group {
        device: device.id.clone(),
        group: group.id.clone(),
    };
    let children: Vec<_> = group
        .members
        .iter()
        .filter_map(|member| match member {
            GroupMember::Surface(surface) => Some(
                observations
                    .get(&(
                        TargetId::Surface {
                            device: device.id.clone(),
                            surface: surface.clone(),
                        },
                        facet,
                    ))
                    .cloned(),
            ),
            GroupMember::Element { surface, element } => Some(
                observations
                    .get(&(
                        TargetId::Element {
                            device: device.id.clone(),
                            surface: surface.clone(),
                            element: element.clone(),
                        },
                        facet,
                    ))
                    .cloned(),
            ),
            GroupMember::Group(nested) => device
                .groups
                .iter()
                .find(|candidate| &candidate.id == nested)
                .map(|nested| synthesize_group(device, nested, facet, observations, memo)),
        })
        .collect();
    replace_with_aggregate(observations, target.clone(), facet, &children);
    let aggregate = observations.get(&(target, facet)).cloned();
    memo.insert(group.id.clone(), aggregate.clone());
    aggregate
}

fn appearance_capable(capabilities: &CapabilitySet) -> bool {
    !capabilities.colour.is_empty() || capabilities.hardware_effects.is_some()
}

fn replace_with_aggregate(
    observations: &mut HashMap<(TargetId, StateFacetKind), FacetObservation>,
    target: TargetId,
    facet: StateFacetKind,
    children: &[Option<FacetObservation>],
) {
    if children.is_empty() {
        return;
    }

    let fallback = observations.get(&(target.clone(), facet)).cloned();
    if fallback.as_ref().is_some_and(|observation| {
        observation.value == FacetValue::EffectiveAppearance(EffectiveAppearanceState::Streaming)
    }) {
        return;
    }
    observations.remove(&(target.clone(), facet));
    if let Some(aggregate) = aggregate_observations(children, fallback.as_ref()) {
        observations.insert(
            (target.clone(), facet),
            FacetObservation {
                target,
                ..aggregate
            },
        );
    }
}

fn aggregate_observations(
    constituents: &[Option<FacetObservation>],
    fallback: Option<&FacetObservation>,
) -> Option<FacetObservation> {
    if constituents.is_empty() {
        return fallback.cloned();
    }

    let known: Vec<_> = constituents.iter().flatten().collect();
    let first = known.first()?;
    let facet = first.value.kind();
    let mixed = known.iter().any(|observation| {
        matches!(
            observation.value,
            FacetValue::Appearance(AppearanceState::Mixed)
                | FacetValue::EffectiveAppearance(EffectiveAppearanceState::Mixed)
        )
    }) || known
        .iter()
        .skip(1)
        .any(|observation| observation.value != first.value);
    if !mixed && known.len() != constituents.len() {
        return None;
    }

    let value = if mixed {
        match facet {
            StateFacetKind::Appearance => FacetValue::Appearance(AppearanceState::Mixed),
            StateFacetKind::EffectiveAppearance => {
                FacetValue::EffectiveAppearance(EffectiveAppearanceState::Mixed)
            }
            StateFacetKind::Brightness
            | StateFacetKind::Emission
            | StateFacetKind::PhysicalPower
            | StateFacetKind::AppearanceSlots => return None,
        }
    } else {
        first.value.clone()
    };
    Some(FacetObservation {
        target: first.target.clone(),
        value,
        confidence: known
            .iter()
            .map(|observation| observation.confidence)
            .min()?,
        source: ObservationSource::Derived,
        observed_at_ms: known
            .iter()
            .map(|observation| observation.observed_at_ms)
            .min()?,
        stale: known.iter().any(|observation| observation.stale),
    })
}

fn collection_appearance_observation(
    observation: FacetObservation,
) -> Option<AggregateAppearanceObservation> {
    let FacetValue::Appearance(value) = observation.value else {
        return None;
    };
    Some(AggregateAppearanceObservation {
        value,
        confidence: observation.confidence,
        observed_at_ms: observation.observed_at_ms,
        stale: observation.stale,
    })
}

fn collection_effective_appearance_observation(
    observation: FacetObservation,
) -> Option<AggregateEffectiveAppearanceObservation> {
    let FacetValue::EffectiveAppearance(value) = observation.value else {
        return None;
    };
    Some(AggregateEffectiveAppearanceObservation {
        value,
        confidence: observation.confidence,
        observed_at_ms: observation.observed_at_ms,
        stale: observation.stale,
    })
}

pub(crate) fn empty_device_status(device: DeviceId) -> DeviceStateStatus {
    DeviceStateStatus {
        device,
        observations: Vec::new(),
        reachability: Reachability::Unknown,
        reconciliation: ReconciliationStatus::Idle,
        adoption: Vec::new(),
        latest_error: None,
        latest_attempt_ms: None,
    }
}

impl DaemonState {
    pub(crate) fn device_status_mut(&mut self, device: &DeviceId) -> &mut DeviceStateStatus {
        self.device_status
            .entry(device.clone())
            .or_insert_with(|| empty_device_status(device.clone()))
    }
}

fn target_sort_key(target: &TargetId) -> (u8, &str, &str, &str) {
    match target {
        TargetId::Device(device) => (0, device.as_str(), "", ""),
        TargetId::Surface { device, surface } => (3, device.as_str(), surface.as_str(), ""),
        TargetId::Element {
            device,
            surface,
            element,
        } => (1, device.as_str(), surface.as_str(), element.as_str()),
        TargetId::Group { device, group } => (2, device.as_str(), group.as_str(), ""),
    }
}

fn replace_adopted_facet(facets: &mut Vec<AdoptedFacet>, mut replacement: AdoptedFacet) {
    if let FacetValue::AppearanceSlots(replacement_slots) = &mut replacement.value
        && let Some(AdoptedFacet {
            value: FacetValue::AppearanceSlots(current_slots),
            ..
        }) = facets.iter().find(|current| {
            current.target == replacement.target
                && current.value.kind() == StateFacetKind::AppearanceSlots
        })
    {
        for current_value in &current_slots.values {
            if !replacement_slots
                .values
                .iter()
                .any(|value| value.slot == current_value.slot)
            {
                replacement_slots.values.push(current_value.clone());
            }
        }
        replacement_slots.complete |= current_slots.complete;
    }
    facets.retain(|current| {
        current.target != replacement.target || current.value.kind() != replacement.value.kind()
    });
    facets.push(replacement);
}

pub(crate) fn set_adoption_status(
    status: &mut DeviceStateStatus,
    target: TargetId,
    facet: StateFacetKind,
    adoption: AdoptionStatus,
) {
    if let Some(existing) = status
        .adoption
        .iter_mut()
        .find(|(candidate, kind, _)| candidate == &target && *kind == facet)
    {
        existing.2 = adoption;
    } else {
        status.adoption.push((target, facet, adoption));
    }
}

fn push_read_target(
    targets: &mut Vec<PluginReadTarget>,
    target: PluginTarget,
    capabilities: &CapabilitySet,
) {
    let StateReadbackCapability::Readable { facets, .. } = &capabilities.state_readback else {
        return;
    };
    if !facets.is_empty() {
        targets.push(PluginReadTarget {
            target,
            facets: facets.iter().map(|facet| facet.facet).collect(),
        });
    }
}

pub(crate) fn readable_fidelity(
    capabilities: &CapabilitySet,
    facet: StateFacetKind,
) -> Option<ReadbackFidelity> {
    let StateReadbackCapability::Readable { facets, .. } = &capabilities.state_readback else {
        return None;
    };
    facets
        .iter()
        .find(|readable| readable.facet == facet)
        .map(|readable| readable.fidelity)
}

pub(crate) fn canonical_target_from_plugin(target: PluginTarget) -> Result<TargetId, DaemonError> {
    match target {
        PluginTarget::Device { device } => Ok(TargetId::device(device)),
        PluginTarget::Surface { device, surface } => Ok(TargetId::surface(device, surface)),
        PluginTarget::Element {
            device,
            surface,
            element,
        } => Ok(TargetId::element(device, surface, element)),
        PluginTarget::Group { .. } => Err(DaemonError::Internal(
            "plugin snapshot used a non-canonical group target".to_owned(),
        )),
    }
}

#[cfg(test)]
#[path = "observation_tests.rs"]
mod tests;
