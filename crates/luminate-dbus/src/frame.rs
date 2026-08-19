// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Native D-Bus request conversion for ordinary frame upload.

use luminate::{FrameEnvelope, FramePayload};
use zbus::zvariant::{DeserializeDict, Type};

use crate::effect_request::StaticColourRequest;
use crate::error::MethodError;

#[derive(Debug, DeserializeDict, Type)]
#[zvariant(signature = "a{sv}", rename_all = "PascalCase", deny_unknown_fields)]
pub(crate) struct Request {
    generation: u32,
    sequence: u64,
    kind: String,
    colours: Option<Vec<StaticColourRequest>>,
    pixels: Option<Vec<(u32, StaticColourRequest)>>,
    commit: bool,
}

impl Request {
    pub(crate) fn into_envelope(self) -> Result<FrameEnvelope, MethodError> {
        let payload = match self.kind.as_str() {
            "full" => {
                if self.pixels.is_some() {
                    return Err(invalid("full frame must not contain Pixels"));
                }
                FramePayload::Full(
                    self.colours
                        .ok_or_else(|| invalid("full frame is missing Colours"))?
                        .iter()
                        .map(StaticColourRequest::to_colour)
                        .collect::<Result<_, _>>()?,
                )
            }
            "partial" => {
                if self.colours.is_some() {
                    return Err(invalid("partial frame must not contain Colours"));
                }
                FramePayload::Partial(
                    self.pixels
                        .ok_or_else(|| invalid("partial frame is missing Pixels"))?
                        .iter()
                        .map(|(index, colour)| Ok((*index, colour.to_colour()?)))
                        .collect::<Result<_, MethodError>>()?,
                )
            }
            value => return Err(invalid(format!("unknown frame payload kind {value:?}"))),
        };
        Ok(FrameEnvelope {
            generation: self.generation,
            sequence: self.sequence,
            payload,
            commit: self.commit,
        })
    }
}

fn invalid(message: impl Into<String>) -> MethodError {
    MethodError::InvalidArgument(message.into())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn colour() -> StaticColourRequest {
        StaticColourRequest {
            model: "additive".into(),
            channels: HashMap::from([("red".into(), 1), ("green".into(), 2), ("blue".into(), 3)]),
        }
    }

    #[test]
    fn frame_requests_cover_full_partial_and_inconsistent_shapes() {
        let full = Request {
            generation: 2,
            sequence: 3,
            kind: "full".into(),
            colours: Some(vec![colour()]),
            pixels: None,
            commit: true,
        }
        .into_envelope()
        .expect("full frame");
        assert!(matches!(full.payload, FramePayload::Full(values) if values.len() == 1));

        let partial = Request {
            generation: 2,
            sequence: 4,
            kind: "partial".into(),
            colours: None,
            pixels: Some(vec![(7, colour())]),
            commit: false,
        }
        .into_envelope()
        .expect("partial frame");
        assert!(matches!(partial.payload, FramePayload::Partial(values) if values.len() == 1));

        assert!(
            Request {
                generation: 2,
                sequence: 5,
                kind: "full".into(),
                colours: Some(vec![colour()]),
                pixels: Some(vec![(0, colour())]),
                commit: false,
            }
            .into_envelope()
            .is_err()
        );
    }
}
