// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use super::*;

#[test]
fn cbor_round_trips_a_typed_value() {
    let payload = encode_cbor(&vec![1_u32, 2, 3]).expect("encode succeeds");
    let decoded: Vec<u32> = decode_cbor(&payload).expect("decode succeeds");
    assert_eq!(decoded, vec![1, 2, 3]);
}

#[test]
fn decode_cbor_rejects_malformed_payloads() {
    let malformed = [0xff_u8];
    let result: Result<Vec<u32>, String> = decode_cbor(&malformed);
    assert!(result.is_err());
}

#[test]
fn sanitize_cstring_strips_interior_nuls_and_terminates_once() {
    let sanitized = sanitize_cstring("a\0b\0c".to_owned());
    assert_eq!(sanitized.as_bytes(), b"abc");
    assert_eq!(sanitized.as_bytes_with_nul(), b"abc\0");
}

#[test]
fn sanitize_cstring_handles_empty_and_clean_strings() {
    assert_eq!(sanitize_cstring(String::new()).as_bytes(), b"");
    assert_eq!(sanitize_cstring("clean".to_owned()).as_bytes(), b"clean");
}
