// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Shared length-prefixed CBOR framing for daemon IPC.

use ciborium::de;
use ciborium::ser;
use std::io;
use std::num;
use tokio::time;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::time::Duration;
use tokio::time::error::Elapsed;

/// Maximum accepted frame length in bytes.
///
/// This comfortably accommodates topology payloads while bounding allocation
/// from an untrusted length prefix.
pub const MAX_FRAME_LEN: u32 = 8 * 1024 * 1024;

/// Timeout duration for I/O operations.
pub const IO_TIMEOUT: Duration = Duration::from_secs(10);

/// Errors from sending or receiving one length-prefixed frame.
#[derive(Debug, thiserror::Error)]
pub enum FramingError {
    /// The underlying transport failed.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// The serialized payload cannot be represented by the wire length prefix.
    #[error("payload is too large to encode a length prefix: {0}")]
    PayloadTooLarge(#[from] num::TryFromIntError),

    /// The frame exceeds the configured wire-size limit.
    #[error("frame of {len} bytes exceeds the {max} byte limit")]
    FrameTooLarge {
        /// Declared or serialized frame length.
        len: u32,

        /// Maximum accepted frame length.
        max: u32,
    },

    /// The value could not be serialized as CBOR.
    #[error("CBOR encoding error: {0}")]
    CborEncode(#[from] ser::Error<io::Error>),

    /// The payload could not be deserialized as CBOR.
    #[error("CBOR decoding error: {0}")]
    CborDecode(#[from] de::Error<io::Error>),

    /// A started frame was not completed before its deadline.
    #[error("Operation timed out")]
    Timeout(#[from] Elapsed),
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
    let mut payload = Vec::new();
    ciborium::into_writer(value, &mut payload)?;
    let len = u32::try_from(payload.len())?;

    // Keep the wire limit symmetric in both directions.
    if len > MAX_FRAME_LEN {
        return Err(FramingError::FrameTooLarge {
            len,
            max: MAX_FRAME_LEN,
        });
    }

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
    receive_payload(stream, len).await
}

/// Waits without an idle deadline for a peer to begin a frame, then requires
/// the remainder of that frame to arrive within `frame_timeout`.
///
/// This is intended for an established request loop: an idle connection is
/// healthy, while a peer that sends a partial length prefix or payload must not
/// retain its connection slot indefinitely.
///
/// # Errors
///
/// Returns the same framing errors as [`receive`], plus
/// [`FramingError::Timeout`] if a started frame is not completed in time.
pub async fn receive_with_frame_timeout<T>(
    stream: &mut (impl AsyncRead + Unpin),
    frame_timeout: Duration,
) -> Result<T, FramingError>
where
    T: DeserializeOwned,
{
    let mut prefix = [0_u8; 4];
    stream.read_exact(&mut prefix[..1]).await?;

    time::timeout(frame_timeout, async {
        stream.read_exact(&mut prefix[1..]).await?;
        receive_payload(stream, u32::from_be_bytes(prefix)).await
    })
    .await?
}

async fn receive_payload<T>(
    stream: &mut (impl AsyncRead + Unpin),
    len: u32,
) -> Result<T, FramingError>
where
    T: DeserializeOwned,
{
    if len > MAX_FRAME_LEN {
        return Err(FramingError::FrameTooLarge {
            len,
            max: MAX_FRAME_LEN,
        });
    }

    let mut payload = vec![0; len as usize];
    stream.read_exact(&mut payload).await?;
    Ok(ciborium::from_reader(payload.as_slice())?)
}

#[cfg(test)]
#[path = "framing_tests.rs"]
mod tests;
