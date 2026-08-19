// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use super::*;
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Example {
    enabled: bool,
}

#[test]
fn empty_configuration_deserializes_to_an_empty_object_shape() {
    #[derive(Debug, Default, Deserialize, PartialEq, Eq)]
    #[serde(default)]
    struct Empty {
        enabled: bool,
    }

    assert_eq!(
        ciborium::from_reader::<Empty, _>(empty_configuration().as_slice())
            .expect("deserialize empty configuration"),
        Empty::default()
    );
}

#[test]
fn typed_configuration_deserializes() {
    let value = serde_json::json!({"enabled": true});
    let mut encoded = Vec::new();
    ciborium::into_writer(&value, &mut encoded).expect("encode configuration");
    assert_eq!(
        ciborium::from_reader::<Example, _>(encoded.as_slice())
            .expect("deserialize typed configuration"),
        Example { enabled: true }
    );
}

#[test]
fn decode_configuration_bytes_accepts_a_valid_object() {
    let mut encoded = Vec::new();
    ciborium::into_writer(&serde_json::json!({"enabled": true}), &mut encoded)
        .expect("serialize a CBOR object");
    assert_eq!(decode_configuration_bytes(&encoded), encoded);
}

#[test]
fn decode_configuration_bytes_falls_back_to_empty_for_malformed_cbor() {
    let malformed = [0xff_u8];
    assert_eq!(
        decode_configuration_bytes(&malformed),
        empty_configuration()
    );
}

#[test]
fn decode_configuration_bytes_falls_back_to_empty_for_a_non_object_value() {
    let mut encoded = Vec::new();
    ciborium::into_writer(&serde_json::json!([1, 2, 3]), &mut encoded)
        .expect("serialize a CBOR array");
    assert_eq!(decode_configuration_bytes(&encoded), empty_configuration());
}
