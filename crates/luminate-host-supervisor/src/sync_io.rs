// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Synchronous length-prefixed CBOR framing for supervisor ↔ host IPC.
//!
//! Error types and frame limits are shared with the async transport.

use std::io::{Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::framing::{FramingError, checked_len, decode, encode};

/// Serializes `value` as CBOR and writes it as one length-prefixed frame.
///
/// # Errors
///
/// Returns an error if `value` fails to serialize, is too large to encode a
/// `u32` length prefix, exceeds [`crate::MAX_FRAME_LEN`], or the underlying write
/// fails.
pub fn write_frame<W: Write>(writer: &mut W, value: &impl Serialize) -> Result<(), FramingError> {
    let (len, payload) = encode(value)?;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

/// Reads one length-prefixed frame and deserializes it as CBOR.
///
/// # Errors
///
/// Returns an error if the underlying read fails, the declared frame length
/// exceeds [`crate::MAX_FRAME_LEN`], or the frame fails to deserialize as `T`.
pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> Result<T, FramingError> {
    let mut length = [0_u8; 4];
    reader.read_exact(&mut length)?;
    let len = u32::from_be_bytes(length);
    let mut payload = vec![0_u8; checked_len(len)?];
    reader.read_exact(&mut payload)?;
    decode(&payload)
}

#[cfg(test)]
#[path = "sync_io_tests.rs"]
mod tests;
