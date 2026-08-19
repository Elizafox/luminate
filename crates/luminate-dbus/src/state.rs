// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Structured D-Bus projection of observed target state and reconciliation metadata.

use luminate::{
    AdoptionStatus, AppearanceState, CollectionStateStatus, DeviceStateStatus,
    EffectiveAppearanceState, EmissionState, FacetObservation, FacetValue, ObservationConfidence,
    ObservationSource, PhysicalPowerState, Reachability, ReconciliationStatus, StateFacetKind,
    TargetId,
};

use crate::error::MethodError;
use crate::management::{Dictionary, dictionary, owned};
use crate::path::canonical_id;
use crate::scene::{effect, static_colour};

pub(crate) fn device_state(status: DeviceStateStatus) -> Result<Dictionary, MethodError> {
    let observations = status
        .observations
        .into_iter()
        .map(observation)
        .collect::<Result<Vec<_>, _>>()?;
    let adoption = status
        .adoption
        .into_iter()
        .map(|(target, facet, adoption)| {
            (
                canonical_id(&target),
                facet_name(facet).to_owned(),
                adoption_name(adoption).to_owned(),
            )
        })
        .collect::<Vec<_>>();
    let mut result = dictionary([
        ("Device", owned(status.device.as_str().to_owned())?),
        ("Observations", owned(observations)?),
        ("Reachability", owned(reachability(status.reachability))?),
        (
            "Reconciliation",
            owned(reconciliation(status.reconciliation))?,
        ),
        ("Adoption", owned(adoption)?),
        ("HasLatestError", owned(status.latest_error.is_some())?),
        (
            "HasLatestAttemptMs",
            owned(status.latest_attempt_ms.is_some())?,
        ),
    ]);
    if let Some(error) = status.latest_error {
        result.insert("LatestError".into(), owned(error)?);
    }
    if let Some(attempt) = status.latest_attempt_ms {
        result.insert("LatestAttemptMs".into(), owned(attempt)?);
    }
    Ok(result)
}

pub(crate) fn target_state(
    status: DeviceStateStatus,
    target: &TargetId,
) -> Result<Dictionary, MethodError> {
    let observations = status
        .observations
        .into_iter()
        .filter(|observation| &observation.target == target)
        .map(observation)
        .collect::<Result<Vec<_>, _>>()?;
    let adoption = status
        .adoption
        .into_iter()
        .filter(|(adopted_target, _, _)| adopted_target == target)
        .map(|(adopted_target, facet, adoption)| {
            (
                canonical_id(&adopted_target),
                facet_name(facet).to_owned(),
                adoption_name(adoption).to_owned(),
            )
        })
        .collect::<Vec<_>>();
    let mut result = dictionary([
        ("Device", owned(status.device.as_str().to_owned())?),
        ("Target", owned(canonical_id(target))?),
        ("Observations", owned(observations)?),
        ("Reachability", owned(reachability(status.reachability))?),
        (
            "Reconciliation",
            owned(reconciliation(status.reconciliation))?,
        ),
        ("Adoption", owned(adoption)?),
        ("HasLatestError", owned(status.latest_error.is_some())?),
        (
            "HasLatestAttemptMs",
            owned(status.latest_attempt_ms.is_some())?,
        ),
    ]);
    if let Some(error) = status.latest_error {
        result.insert("LatestError".into(), owned(error)?);
    }
    if let Some(attempt) = status.latest_attempt_ms {
        result.insert("LatestAttemptMs".into(), owned(attempt)?);
    }
    Ok(result)
}

pub(crate) fn collection_state(value: CollectionStateStatus) -> Result<Dictionary, MethodError> {
    let mut result = dictionary([
        ("Collection", owned(value.collection.as_str().to_owned())?),
        ("HasAppearance", owned(value.appearance.is_some())?),
        (
            "HasEffectiveAppearance",
            owned(value.effective_appearance.is_some())?,
        ),
    ]);
    if let Some(observation) = value.appearance {
        result.insert(
            "Appearance".into(),
            owned(dictionary([
                ("Value", owned(appearance(observation.value)?)?),
                ("Confidence", owned(confidence(observation.confidence))?),
                ("ObservedAtMs", owned(observation.observed_at_ms)?),
                ("Stale", owned(observation.stale)?),
            ]))?,
        );
    }
    if let Some(observation) = value.effective_appearance {
        result.insert(
            "EffectiveAppearance".into(),
            owned(dictionary([
                ("Value", owned(effective_appearance(observation.value)?)?),
                ("Confidence", owned(confidence(observation.confidence))?),
                ("ObservedAtMs", owned(observation.observed_at_ms)?),
                ("Stale", owned(observation.stale)?),
            ]))?,
        );
    }
    Ok(result)
}

fn observation(value: FacetObservation) -> Result<Dictionary, MethodError> {
    Ok(dictionary([
        ("Target", owned(canonical_id(&value.target))?),
        ("Facet", owned(facet_name(value.value.kind()))?),
        ("Value", owned(facet_value(value.value)?)?),
        ("Confidence", owned(confidence(value.confidence))?),
        ("Source", owned(source(value.source))?),
        ("ObservedAtMs", owned(value.observed_at_ms)?),
        ("Stale", owned(value.stale)?),
    ]))
}

fn facet_value(value: FacetValue) -> Result<Dictionary, MethodError> {
    match value {
        FacetValue::Appearance(value) => appearance(value),
        FacetValue::Brightness(value) => Ok(dictionary([
            ("Kind", owned("brightness")?),
            ("Value", owned(value)?),
        ])),
        FacetValue::Emission(value) => Ok(dictionary([
            ("Kind", owned("emission")?),
            (
                "Value",
                owned(match value {
                    EmissionState::Dark => "dark",
                    EmissionState::Emitting => "emitting",
                })?,
            ),
        ])),
        FacetValue::PhysicalPower(value) => Ok(dictionary([
            ("Kind", owned("physical-power")?),
            (
                "Value",
                owned(match value {
                    PhysicalPowerState::Off => "off",
                    PhysicalPowerState::On => "on",
                })?,
            ),
        ])),
        FacetValue::EffectiveAppearance(value) => effective_appearance(value),
        FacetValue::AppearanceSlots(value) => Ok(dictionary([
            ("Kind", owned("appearance-slots")?),
            ("Complete", owned(value.complete)?),
            (
                "Values",
                owned(
                    value
                        .values
                        .into_iter()
                        .map(|value| {
                            Ok(dictionary([
                                ("Slot", owned(value.slot.as_str().to_owned())?),
                                ("Effect", owned(effect(value.effect)?)?),
                            ]))
                        })
                        .collect::<Result<Vec<_>, MethodError>>()?,
                )?,
            ),
        ])),
    }
}

fn appearance(value: AppearanceState) -> Result<Dictionary, MethodError> {
    match value {
        AppearanceState::Static(colour) => Ok(dictionary([
            ("Kind", owned("appearance-static")?),
            ("Colour", owned(static_colour(colour)?)?),
        ])),
        AppearanceState::Effect(value) => Ok(dictionary([
            ("Kind", owned("appearance-effect")?),
            ("Effect", owned(effect(value)?)?),
        ])),
        AppearanceState::Mixed => Ok(dictionary([("Kind", owned("appearance-mixed")?)])),
    }
}

fn effective_appearance(value: EffectiveAppearanceState) -> Result<Dictionary, MethodError> {
    match value {
        EffectiveAppearanceState::Off => {
            Ok(dictionary([("Kind", owned("effective-appearance-off")?)]))
        }
        EffectiveAppearanceState::Static(colour) => Ok(dictionary([
            ("Kind", owned("effective-appearance-static")?),
            ("Colour", owned(static_colour(colour)?)?),
        ])),
        EffectiveAppearanceState::Effect(value) => Ok(dictionary([
            ("Kind", owned("effective-appearance-effect")?),
            ("Effect", owned(effect(value)?)?),
        ])),
        EffectiveAppearanceState::Streaming => Ok(dictionary([(
            "Kind",
            owned("effective-appearance-streaming")?,
        )])),
        EffectiveAppearanceState::Mixed => {
            Ok(dictionary([("Kind", owned("effective-appearance-mixed")?)]))
        }
    }
}

fn facet_name(value: StateFacetKind) -> &'static str {
    match value {
        StateFacetKind::Appearance => "appearance",
        StateFacetKind::Brightness => "brightness",
        StateFacetKind::Emission => "emission",
        StateFacetKind::PhysicalPower => "physical-power",
        StateFacetKind::EffectiveAppearance => "effective-appearance",
        StateFacetKind::AppearanceSlots => "appearance-slots",
    }
}

fn confidence(value: ObservationConfidence) -> &'static str {
    match value {
        ObservationConfidence::Assumed => "assumed",
        ObservationConfidence::BestEffort => "best-effort",
        ObservationConfidence::Confirmed => "confirmed",
    }
}

fn source(value: ObservationSource) -> &'static str {
    match value {
        ObservationSource::SuccessfulApply => "successful-apply",
        ObservationSource::Readback => "readback",
        ObservationSource::Derived => "derived",
        ObservationSource::AdoptedBaseline => "adopted-baseline",
    }
}

fn reachability(value: Reachability) -> &'static str {
    match value {
        Reachability::Unknown => "unknown",
        Reachability::Reachable => "reachable",
        Reachability::Unavailable => "unavailable",
    }
}

fn reconciliation(value: ReconciliationStatus) -> &'static str {
    match value {
        ReconciliationStatus::Idle => "idle",
        ReconciliationStatus::Reconciling => "reconciling",
        ReconciliationStatus::Complete => "complete",
        ReconciliationStatus::Drifted => "drifted",
        ReconciliationStatus::Failed => "failed",
    }
}

fn adoption_name(value: AdoptionStatus) -> &'static str {
    match value {
        AdoptionStatus::NotApplicable => "not-applicable",
        AdoptionStatus::Pending => "pending",
        AdoptionStatus::Durable => "durable",
        AdoptionStatus::IneligibleFidelity => "ineligible-fidelity",
        AdoptionStatus::PersistenceFailed => "persistence-failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use luminate::{
        AggregateAppearanceObservation, AggregateEffectiveAppearanceObservation,
        AppearanceSlotsState, CollectionId, Colour, DeviceId, Rgb,
    };

    fn observation(value: FacetValue, target: &TargetId) -> FacetObservation {
        FacetObservation {
            target: target.clone(),
            value,
            confidence: ObservationConfidence::Confirmed,
            source: ObservationSource::Readback,
            observed_at_ms: 42,
            stale: false,
        }
    }

    #[test]
    fn target_state_preserves_every_facet_and_filters_other_targets() {
        let target = TargetId::device("keyboard");
        let other = TargetId::surface("keyboard", "logo");
        let status = DeviceStateStatus {
            device: DeviceId::new("keyboard"),
            observations: vec![
                observation(FacetValue::Appearance(AppearanceState::Mixed), &target),
                observation(FacetValue::Brightness(128), &target),
                observation(FacetValue::Emission(EmissionState::Emitting), &target),
                observation(FacetValue::PhysicalPower(PhysicalPowerState::On), &target),
                observation(
                    FacetValue::EffectiveAppearance(EffectiveAppearanceState::Streaming),
                    &target,
                ),
                observation(
                    FacetValue::AppearanceSlots(AppearanceSlotsState {
                        values: Vec::new(),
                        complete: true,
                    }),
                    &target,
                ),
                observation(FacetValue::Brightness(1), &other),
            ],
            reachability: Reachability::Reachable,
            reconciliation: ReconciliationStatus::Complete,
            adoption: vec![
                (
                    target.clone(),
                    StateFacetKind::Brightness,
                    AdoptionStatus::Durable,
                ),
                (other, StateFacetKind::Brightness, AdoptionStatus::Pending),
            ],
            latest_error: Some("previous failure".into()),
            latest_attempt_ms: Some(99),
        };

        let encoded = target_state(status, &target).expect("encode state");
        let observations = Vec::<Dictionary>::try_from(
            encoded["Observations"]
                .try_clone()
                .expect("clone observations"),
        )
        .expect("observations should be dictionaries");
        let adoption = Vec::<(String, String, String)>::try_from(
            encoded["Adoption"].try_clone().expect("clone adoption"),
        )
        .expect("adoption should be typed records");

        assert_eq!(observations.len(), 6);
        assert_eq!(adoption.len(), 1);
        assert_eq!(adoption[0].1, "brightness");
        assert!(
            bool::try_from(encoded["HasLatestError"].try_clone().expect("clone flag"))
                .expect("error presence should be boolean")
        );
    }

    #[test]
    fn device_state_preserves_observations_and_adoption_for_every_target() {
        let device_target = TargetId::device("keyboard");
        let surface_target = TargetId::surface("keyboard", "logo");
        let status = DeviceStateStatus {
            device: DeviceId::new("keyboard"),
            observations: vec![
                observation(FacetValue::Brightness(128), &device_target),
                observation(FacetValue::Emission(EmissionState::Dark), &surface_target),
            ],
            reachability: Reachability::Unknown,
            reconciliation: ReconciliationStatus::Idle,
            adoption: vec![
                (
                    device_target,
                    StateFacetKind::Brightness,
                    AdoptionStatus::Durable,
                ),
                (
                    surface_target,
                    StateFacetKind::Emission,
                    AdoptionStatus::Pending,
                ),
            ],
            latest_error: None,
            latest_attempt_ms: None,
        };

        let encoded = device_state(status).expect("encode device state");
        let observations = Vec::<Dictionary>::try_from(
            encoded["Observations"]
                .try_clone()
                .expect("clone observations"),
        )
        .expect("observations should be dictionaries");
        let adoption = Vec::<(String, String, String)>::try_from(
            encoded["Adoption"].try_clone().expect("clone adoption"),
        )
        .expect("adoption should be typed records");

        assert_eq!(observations.len(), 2);
        assert_eq!(adoption.len(), 2);
    }

    #[test]
    fn collection_state_preserves_configured_and_effective_observations() {
        let encoded = collection_state(CollectionStateStatus {
            collection: CollectionId::new("desk"),
            appearance: Some(AggregateAppearanceObservation {
                value: AppearanceState::Static(Colour::rgb(Rgb::new(1, 2, 3))),
                confidence: ObservationConfidence::BestEffort,
                observed_at_ms: 10,
                stale: true,
            }),
            effective_appearance: Some(AggregateEffectiveAppearanceObservation {
                value: EffectiveAppearanceState::Mixed,
                confidence: ObservationConfidence::Assumed,
                observed_at_ms: 9,
                stale: false,
            }),
        })
        .expect("encode collection state");

        assert!(
            bool::try_from(
                encoded["HasAppearance"]
                    .try_clone()
                    .expect("clone presence")
            )
            .expect("appearance presence should be boolean")
        );
        assert!(encoded.contains_key("Appearance"));
        assert!(encoded.contains_key("EffectiveAppearance"));
    }
}
