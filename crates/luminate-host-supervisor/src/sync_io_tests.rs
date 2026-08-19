// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use super::*;
use crate::MAX_FRAME_LEN;

#[test]
fn write_then_read_round_trips() {
    let mut buffer = Vec::new();
    write_frame(&mut buffer, &"hello".to_owned()).expect("write frame");
    let received: String = read_frame(&mut buffer.as_slice()).expect("read frame");
    assert_eq!(received, "hello");
}

#[test]
fn oversized_declared_length_is_rejected_before_reading_the_payload() {
    let oversized = MAX_FRAME_LEN + 1;
    let mut framed = oversized.to_be_bytes().to_vec();
    framed.extend_from_slice(b"unused");
    match read_frame::<_, String>(&mut framed.as_slice()) {
        Err(FramingError::FrameTooLarge { len, max }) => {
            assert_eq!(len, MAX_FRAME_LEN + 1);
            assert_eq!(max, MAX_FRAME_LEN);
        }
        other => panic!("expected FrameTooLarge, got {other:?}"),
    }
}

#[test]
fn truncated_frame_is_rejected() {
    let mut framed = 4_u32.to_be_bytes().to_vec();
    framed.extend_from_slice(b"ab");
    assert!(read_frame::<_, String>(&mut framed.as_slice()).is_err());
}
