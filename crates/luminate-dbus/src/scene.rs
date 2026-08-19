// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Native D-Bus scene definitions and snapshots.

use std::collections::HashMap;

use luminate::capability::ColourChannel;
use luminate::collection::OwnerIdentity;
use luminate::{
    AppearanceSlotId, AppearanceSlotValue, Colour, Effect, EmissionState, Scene, SceneBinding,
    SceneTargetState,
};
use zbus::zvariant::{DeserializeDict, Type};

use crate::convert::{direction_id, invalid_argument};
use crate::effect_request::EffectRequest;
use crate::error::MethodError;
use crate::management::{Dictionary, dictionary, owned};
use crate::path::{canonical_id, parse_canonical_id};

pub(crate) type SceneBindingRecord = (String, String, Dictionary);
pub(crate) type SceneRecord = (
    String,
    u64,
    String,
    bool,
    String,
    String,
    String,
    Vec<SceneBindingRecord>,
);

#[derive(Debug, DeserializeDict, Type)]
#[zvariant(signature = "a{sv}", rename_all = "PascalCase", deny_unknown_fields)]
pub(crate) struct SceneBindingRequest {
    target: String,
    dynamic_collection: Option<String>,
    appearance: Option<EffectRequest>,
    brightness: Option<u32>,
    emission: Option<String>,
    appearance_slots: Option<Vec<(String, EffectRequest)>>,
}

impl SceneBindingRequest {
    pub(crate) fn into_binding(self) -> Result<SceneBinding, MethodError> {
        let target = parse_canonical_id(&self.target)?;
        let state = SceneTargetState {
            appearance: self
                .appearance
                .map(EffectRequest::into_effect)
                .transpose()?,
            brightness: self.brightness,
            emission: self
                .emission
                .map(|value| match value.as_str() {
                    "dark" => Ok(EmissionState::Dark),
                    "emitting" => Ok(EmissionState::Emitting),
                    _ => Err(invalid_argument(format!(
                        "unknown emission state {value:?}"
                    ))),
                })
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
                        .collect::<Result<Vec<_>, MethodError>>()
                })
                .transpose()?,
        };
        state
            .validate()
            .map_err(|error| invalid_argument(error.to_string()))?;
        Ok(match self.dynamic_collection {
            Some(collection) => SceneBinding::DynamicCollectionMember {
                collection: luminate::CollectionId::new(collection),
                target,
                state,
            },
            None => SceneBinding::Frozen { target, state },
        })
    }
}

pub(crate) fn record(scene: Scene) -> Result<SceneRecord, MethodError> {
    let (owner_kind, owner) = match scene.owner {
        OwnerIdentity::Uid(uid) => ("uid".to_owned(), uid.to_string()),
        OwnerIdentity::Sid(sid) => ("sid".to_owned(), sid),
        OwnerIdentity::Principal(principal) => (
            "principal".to_owned(),
            format!("{}:{}", principal.authority(), principal.subject()),
        ),
    };
    Ok((
        scene.id.as_str().to_owned(),
        scene.revision,
        scene.name,
        scene.description.is_some(),
        scene.description.unwrap_or_default(),
        owner_kind,
        owner,
        scene
            .bindings
            .into_iter()
            .map(binding_record)
            .collect::<Result<_, _>>()?,
    ))
}

fn binding_record(binding: SceneBinding) -> Result<SceneBindingRecord, MethodError> {
    let (collection, target, state) = match binding {
        SceneBinding::Frozen { target, state } => (String::new(), target, state),
        SceneBinding::DynamicCollectionMember {
            collection,
            target,
            state,
        } => (collection.as_str().to_owned(), target, state),
    };
    let mut fields = Dictionary::new();
    if let Some(appearance) = state.appearance {
        fields.insert("Appearance".to_owned(), owned(effect(appearance)?)?);
    }
    if let Some(brightness) = state.brightness {
        fields.insert("Brightness".to_owned(), owned(brightness)?);
    }
    if let Some(emission) = state.emission {
        fields.insert(
            "Emission".to_owned(),
            owned(match emission {
                EmissionState::Dark => "dark",
                EmissionState::Emitting => "emitting",
            })?,
        );
    }
    if let Some(values) = state.appearance_slots {
        fields.insert(
            "AppearanceSlots".to_owned(),
            owned(
                values
                    .into_iter()
                    .map(|value| Ok((value.slot.as_str().to_owned(), effect(value.effect)?)))
                    .collect::<Result<Vec<_>, MethodError>>()?,
            )?,
        );
    }
    Ok((collection, canonical_id(&target), fields))
}

pub(crate) fn effect(value: Effect) -> Result<Dictionary, MethodError> {
    let mut result = Dictionary::new();
    result.insert("Kind".to_owned(), owned(value.capability_id().to_owned())?);
    match value {
        Effect::Off => {}
        Effect::Static { colour } => {
            result.insert("StaticColour".to_owned(), owned(static_colour(colour)?)?);
        }
        Effect::Breathe { colour, period_ms }
        | Effect::Pulse { colour, period_ms }
        | Effect::Strobe { colour, period_ms }
        | Effect::Scanner { colour, period_ms } => {
            result.insert("Colours".to_owned(), owned(vec![rgb(colour)])?);
            result.insert("PeriodMs".to_owned(), owned(period_ms)?);
        }
        Effect::Morph { colours, period_ms } => {
            result.insert(
                "Colours".to_owned(),
                owned(colours.into_iter().map(rgb).collect::<Vec<_>>())?,
            );
            result.insert("PeriodMs".to_owned(), owned(period_ms)?);
        }
        Effect::Spectrum { period_ms } | Effect::Rainbow { period_ms } => {
            result.insert("PeriodMs".to_owned(), owned(period_ms)?);
        }
        Effect::Hardware { id, arguments } => {
            result.insert("HardwareId".to_owned(), owned(id.as_str().to_owned())?);
            if !arguments.colours.is_empty() {
                result.insert(
                    "Colours".to_owned(),
                    owned(arguments.colours.into_iter().map(rgb).collect::<Vec<_>>())?,
                );
            }
            if let Some(speed) = arguments.speed {
                result.insert("Speed".to_owned(), owned(speed)?);
            }
            if let Some(direction) = arguments.direction {
                result.insert("Direction".to_owned(), owned(direction_id(direction))?);
            }
            if let Some(duration) = arguments.duration_ms {
                result.insert("DurationMs".to_owned(), owned(duration)?);
            }
            if let Some(brightness) = arguments.brightness {
                result.insert("Brightness".to_owned(), owned(brightness)?);
            }
            if let Some(choice) = arguments.choice {
                result.insert("Choice".to_owned(), owned(choice)?);
            }
        }
    }
    Ok(result)
}

fn rgb(value: luminate::Rgb) -> (u8, u8, u8) {
    (value.r, value.g, value.b)
}

pub(crate) fn static_colour(value: Colour) -> Result<Dictionary, MethodError> {
    let (model, channels) = match value {
        Colour::Additive(values) => (
            "additive",
            values
                .into_iter()
                .map(|value| (channel(value.channel).to_owned(), value.value))
                .collect(),
        ),
        Colour::Hsv {
            hue,
            saturation,
            value,
        } => (
            "hsv",
            HashMap::from([
                ("hue".to_owned(), hue),
                ("saturation".to_owned(), saturation),
                ("value".to_owned(), value),
            ]),
        ),
        Colour::Hsl {
            hue,
            saturation,
            lightness,
        } => (
            "hsl",
            HashMap::from([
                ("hue".to_owned(), hue),
                ("saturation".to_owned(), saturation),
                ("lightness".to_owned(), lightness),
            ]),
        ),
        Colour::Cct { kelvin } => ("cct", HashMap::from([("temperature".to_owned(), kelvin)])),
        Colour::Monochrome { intensity } => (
            "monochrome",
            HashMap::from([("intensity".to_owned(), intensity)]),
        ),
    };
    Ok(dictionary([
        ("Model", owned(model)?),
        ("Channels", owned(channels)?),
    ]))
}

fn channel(value: ColourChannel) -> &'static str {
    match value {
        ColourChannel::Red => "red",
        ColourChannel::Green => "green",
        ColourChannel::Blue => "blue",
        ColourChannel::White => "white",
        ColourChannel::WarmWhite => "warm-white",
        ColourChannel::CoolWhite => "cool-white",
        ColourChannel::Amber => "amber",
        ColourChannel::Ultraviolet => "ultraviolet",
        ColourChannel::Hue => "hue",
        ColourChannel::Saturation => "saturation",
        ColourChannel::Value => "value",
        ColourChannel::Lightness => "lightness",
        ColourChannel::Temperature => "temperature",
        ColourChannel::Intensity => "intensity",
    }
}

#[cfg(test)]
#[path = "scene_tests.rs"]
mod tests;
