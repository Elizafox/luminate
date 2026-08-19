// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Async length-prefixed CBOR framing for supervisor ↔ host IPC.
//!
//! This is separate from consumer protocol framing because the two have
//! independent compatibility contracts.
//!
//! See [`crate::sync_io`] for the equivalent primitive over a synchronous
//! `Read`/`Write` pair, used by the plugin host's piped-stdio transport.

use ciborium::{de, ser};
use std::io;
use std::num;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

/// Maximum accepted frame length in bytes for a supervisor ↔ host IPC
/// channel. Standardized on the plugin host's historical value (larger than
/// the policy host's former 8 MiB cap). Policy payloads are expected to remain
/// small, while plugin topology and batch payloads may need the additional
/// headroom.
pub const MAX_FRAME_LEN: u32 = 16 * 1024 * 1024;

/// Errors from sending or receiving one length-prefixed frame.
#[derive(Debug, thiserror::Error)]
pub enum FramingError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("payload is too large to encode a length prefix: {0}")]
    PayloadTooLarge(#[from] num::TryFromIntError),

    #[error("frame of {len} bytes exceeds the {max} byte limit")]
    FrameTooLarge { len: u32, max: u32 },

    #[error("CBOR encoding error: {0}")]
    CborEncode(#[from] ser::Error<io::Error>),

    #[error("CBOR decoding error: {0}")]
    CborDecode(#[from] de::Error<io::Error>),
}

pub(crate) fn encode<T: Serialize>(value: &T) -> Result<(u32, Vec<u8>), FramingError> {
    let mut payload = Vec::new();
    ciborium::into_writer(value, &mut payload)?;
    let len = u32::try_from(payload.len())?;
    checked_len(len)?;
    Ok((len, payload))
}

pub(crate) fn checked_len(len: u32) -> Result<usize, FramingError> {
    if len > MAX_FRAME_LEN {
        return Err(FramingError::FrameTooLarge {
            len,
            max: MAX_FRAME_LEN,
        });
    }
    Ok(len as usize)
}

pub(crate) fn decode<T: DeserializeOwned>(payload: &[u8]) -> Result<T, FramingError> {
    Ok(ciborium::from_reader(payload)?)
}

/// Serializes `value` as CBOR and writes it as one length-prefixed frame.
///
/// # Errors
///
/// Returns an error if `value` fails to serialize, is too large to encode a
/// `u32` length prefix, exceeds [`MAX_FRAME_LEN`], or the underlying write
/// fails.
pub async fn send<T>(stream: &mut (impl AsyncWrite + Unpin), value: &T) -> Result<(), FramingError>
where
    T: Serialize,
{
    let (len, payload) = encode(value)?;

    stream.write_u32(len).await?;
    stream.write_all(&payload).await?;
    stream.flush().await?;

    Ok(())
}

/// Reads one length-prefixed frame and deserializes it as CBOR.
///
/// # Errors
///
/// Returns an error if the underlying read fails, the declared frame length
/// exceeds [`MAX_FRAME_LEN`], or the frame fails to deserialize as `T`.
pub async fn receive<T>(stream: &mut (impl AsyncRead + Unpin)) -> Result<T, FramingError>
where
    T: DeserializeOwned,
{
    let len = stream.read_u32().await?;
    let mut payload = vec![0; checked_len(len)?];
    stream.read_exact(&mut payload).await?;
    decode(&payload)
}

#[cfg(test)]
#[path = "framing_tests.rs"]
mod tests;
