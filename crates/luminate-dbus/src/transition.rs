// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Native D-Bus conversion for daemon-managed transitions.

use std::time::Duration;

use luminate::{
    AppearanceSlotId, AppearanceSlotValue, HueDirection, SceneTargetState, TransitionCancellation,
    TransitionColourInterpolation, TransitionFunction, TransitionOptions, TransitionOutcome,
    TransitionStatus, TransitionTargetState,
};
use zbus::zvariant::{DeserializeDict, Type};

use crate::control;
use crate::effect_request::EffectRequest;
use crate::error::MethodError;
use crate::management::{Dictionary, dictionary, owned};
use crate::path::{canonical_id, parse_canonical_id};

#[derive(Debug, DeserializeDict, Type)]
#[zvariant(signature = "a{sv}", rename_all = "PascalCase", deny_unknown_fields)]
pub(crate) struct OptionsRequest {
    duration_ms: u64,
    step_interval_ms: Option<u64>,
    function: String,
    colour_interpolation: String,
    hue_direction: Option<String>,
}

impl OptionsRequest {
    pub(crate) fn into_options(self) -> Result<TransitionOptions, MethodError> {
        let function = match self.function.as_str() {
            "linear" => TransitionFunction::Linear,
            "ease-in" => TransitionFunction::EaseIn,
            "ease-out" => TransitionFunction::EaseOut,
            "ease-in-out" => TransitionFunction::EaseInOut,
            value => return Err(invalid(format!("unknown transition function {value:?}"))),
        };
        let colour_interpolation = match self.colour_interpolation.as_str() {
            "encoded" => TransitionColourInterpolation::Encoded {
                hue_direction: match self.hue_direction.as_deref().unwrap_or("shortest") {
                    "shortest" => HueDirection::Shortest,
                    "increasing" => HueDirection::Increasing,
                    "decreasing" => HueDirection::Decreasing,
                    value => return Err(invalid(format!("unknown hue direction {value:?}"))),
                },
            },
            "oklab" => {
                if self.hue_direction.is_some() {
                    return Err(invalid("OKLab interpolation does not accept HueDirection"));
                }
                TransitionColourInterpolation::Oklab
            }
            value => {
                return Err(invalid(format!(
                    "unknown transition colour interpolation {value:?}"
                )));
            }
        };
        TransitionOptions::new(
            Duration::from_millis(self.duration_ms),
            self.step_interval_ms.map(Duration::from_millis),
        )
        .map(|options| {
            options
                .with_function(function)
                .with_colour_interpolation(colour_interpolation)
        })
        .map_err(|error| invalid(error.to_string()))
    }
}

#[derive(Debug, DeserializeDict, Type)]
#[zvariant(signature = "a{sv}", rename_all = "PascalCase", deny_unknown_fields)]
pub(crate) struct TargetStateRequest {
    target: String,
    appearance: Option<EffectRequest>,
    brightness: Option<u32>,
    emission: Option<String>,
    appearance_slots: Option<Vec<(String, EffectRequest)>>,
}

impl TargetStateRequest {
    pub(crate) fn into_state(self) -> Result<TransitionTargetState, MethodError> {
        let state = SceneTargetState {
            appearance: self
                .appearance
                .map(EffectRequest::into_effect)
                .transpose()?,
            brightness: self.brightness,
            emission: self
                .emission
                .as_deref()
                .map(control::emission_state)
                .transpose()?,
            appearance_slots: self
                .appearance_slots
                .map(|values| {
                    values
                        .into_iter()
                        .map(|(slot, effect)| {
                            Ok(AppearanceSlotValue {
                                slot: AppearanceSlotId::new(slot),
                                effect: effect.into_effect()?,
                            })
                        })
                        .collect::<Result<_, MethodError>>()
                })
                .transpose()?,
        };
        state
            .validate()
            .map_err(|error| invalid(error.to_string()))?;
        Ok(TransitionTargetState {
            target: parse_canonical_id(&self.target)?,
            state,
        })
    }
}

pub(crate) fn states(
    values: Vec<TargetStateRequest>,
) -> Result<Vec<TransitionTargetState>, MethodError> {
    values
        .into_iter()
        .map(TargetStateRequest::into_state)
        .collect()
}

pub(crate) fn status(value: TransitionStatus) -> Result<Dictionary, MethodError> {
    let mut result = dictionary([
        ("Id", owned(value.id.as_str().to_owned())?),
        (
            "Targets",
            owned(value.targets.iter().map(canonical_id).collect::<Vec<_>>())?,
        ),
        ("ElapsedMs", owned(value.elapsed_ms)?),
        ("DurationMs", owned(value.duration_ms)?),
        ("HasOutcome", owned(value.outcome.is_some())?),
    ]);
    if let Some(value) = value.outcome {
        result.insert("Outcome".into(), owned(outcome(value)?)?);
    }
    Ok(result)
}

fn outcome(value: TransitionOutcome) -> Result<Dictionary, MethodError> {
    match value {
        TransitionOutcome::Completed => Ok(dictionary([("Kind", owned("completed")?)])),
        TransitionOutcome::Cancelled(reason) => Ok(dictionary([
            ("Kind", owned("cancelled")?),
            (
                "Reason",
                owned(match reason {
                    TransitionCancellation::Aborted => "aborted",
                    TransitionCancellation::Replaced => "replaced",
                    TransitionCancellation::ConflictingMutation => "conflicting-mutation",
                    TransitionCancellation::AuthorizationExpired => "authorization-expired",
                })?,
            ),
        ])),
        TransitionOutcome::Failed {
            diagnostic,
            applied_targets,
        } => Ok(dictionary([
            ("Kind", owned("failed")?),
            ("Diagnostic", owned(diagnostic)?),
            (
                "AppliedTargets",
                owned(applied_targets.iter().map(canonical_id).collect::<Vec<_>>())?,
            ),
        ])),
    }
}

fn invalid(message: impl Into<String>) -> MethodError {
    MethodError::InvalidArgument(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use luminate::{TargetId, TransitionId};

    fn options(colour_interpolation: &str, hue_direction: Option<&str>) -> OptionsRequest {
        OptionsRequest {
            duration_ms: 1_000,
            step_interval_ms: Some(20),
            function: "ease-in-out".into(),
            colour_interpolation: colour_interpolation.into(),
            hue_direction: hue_direction.map(str::to_owned),
        }
    }

    #[test]
    fn options_cover_interpolation_variants_and_validation() {
        let encoded = options("encoded", Some("decreasing"))
            .into_options()
            .expect("encoded options");
        assert_eq!(encoded.function, TransitionFunction::EaseInOut);
        assert_eq!(
            encoded.colour_interpolation,
            TransitionColourInterpolation::Encoded {
                hue_direction: HueDirection::Decreasing
            }
        );
        assert_eq!(
            options("oklab", None)
                .into_options()
                .expect("OKLab options")
                .colour_interpolation,
            TransitionColourInterpolation::Oklab
        );
        assert!(options("oklab", Some("shortest")).into_options().is_err());
        let mut invalid = options("encoded", None);
        invalid.duration_ms = 0;
        assert!(invalid.into_options().is_err());
    }

    #[test]
    fn status_preserves_active_completed_cancelled_and_failed_shapes() {
        let target = TargetId::device("lamp");
        let encode = |outcome| {
            status(TransitionStatus {
                id: TransitionId::new("transition"),
                targets: vec![target.clone()],
                elapsed_ms: 20,
                duration_ms: 100,
                outcome,
            })
            .expect("encode transition status")
        };

        assert!(
            !bool::try_from(encode(None)["HasOutcome"].try_clone().expect("clone flag"))
                .expect("boolean flag")
        );
        for outcome in [
            TransitionOutcome::Completed,
            TransitionOutcome::Cancelled(TransitionCancellation::Aborted),
            TransitionOutcome::Cancelled(TransitionCancellation::Replaced),
            TransitionOutcome::Cancelled(TransitionCancellation::ConflictingMutation),
            TransitionOutcome::Cancelled(TransitionCancellation::AuthorizationExpired),
            TransitionOutcome::Failed {
                diagnostic: "hardware failure".into(),
                applied_targets: vec![target.clone()],
            },
        ] {
            let encoded = encode(Some(outcome));
            assert!(encoded.contains_key("Outcome"));
        }
    }
}
