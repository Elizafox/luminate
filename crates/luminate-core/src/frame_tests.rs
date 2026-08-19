// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;
use crate::capability::ColourChannel;
use crate::colour::ColourChannelValue;

fn sample_colour(value: u32) -> Colour {
    Colour::additive(vec![ColourChannelValue::new(ColourChannel::Red, value)])
        .expect("sample colour is valid")
}

#[test]
fn full_payload_round_trips_through_serialization() {
    let envelope = FrameEnvelope {
        generation: 3,
        sequence: 42,
        payload: FramePayload::Full(vec![sample_colour(10), sample_colour(20)]),
        commit: false,
    };

    let encoded = serde_json::to_string(&envelope).expect("encode");
    let decoded: FrameEnvelope = serde_json::from_str(&encoded).expect("decode");

    assert_eq!(decoded, envelope);
}

#[test]
fn partial_payload_round_trips_through_serialization() {
    let envelope = FrameEnvelope {
        generation: 1,
        sequence: 7,
        payload: FramePayload::Partial(vec![(0, sample_colour(5)), (12, sample_colour(6))]),
        commit: true,
    };

    let encoded = serde_json::to_string(&envelope).expect("encode");
    let decoded: FrameEnvelope = serde_json::from_str(&encoded).expect("decode");

    assert_eq!(decoded, envelope);
}
