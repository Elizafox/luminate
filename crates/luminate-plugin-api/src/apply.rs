// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Structured mutation results, the plugin-side error type they're built
//! from, and the CBOR dispatch adapters plugins use to implement
//! `apply_update_cbor`, `apply_batch_cbor`, and `frame_upload_cbor`.

use std::fmt;
use std::slice;
use std::str;
use std::time::Duration;

use thiserror::Error;

use crate::topology::{PluginFrameUpload, PluginUpdate, PluginUpdateBatch};

/// Maximum UTF-8 diagnostic payload carried with one mutation result.
pub const PLUGIN_APPLY_DIAGNOSTIC_CAPACITY: usize = 256;

/// Structured status code stored as a raw byte at the native ABI boundary.
/// Unknown values are rejected by the plugin host without constructing an
/// invalid Rust enum discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginApplyStatus {
    Applied,
    Unsupported,
    InvalidArgument,
    Io,
    Unavailable,
    RateLimited,
    Internal,
}

impl PluginApplyStatus {
    const fn to_abi(self) -> u8 {
        match self {
            Self::Applied => 0,
            Self::Unsupported => 1,
            Self::InvalidArgument => 2,
            Self::Io => 3,
            Self::Internal => 4,
            Self::Unavailable => 5,
            Self::RateLimited => 6,
        }
    }

    /// Decodes an untrusted ABI status byte.
    #[must_use]
    pub const fn from_abi(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Applied),
            1 => Some(Self::Unsupported),
            2 => Some(Self::InvalidArgument),
            3 => Some(Self::Io),
            4 => Some(Self::Internal),
            5 => Some(Self::Unavailable),
            6 => Some(Self::RateLimited),
            _ => None,
        }
    }
}

/// The diagnostic was truncated to [`PLUGIN_APPLY_DIAGNOSTIC_CAPACITY`] at a
/// UTF-8 character boundary.
pub const PLUGIN_APPLY_DIAGNOSTIC_TRUNCATED: u8 = 1;

/// Caller-owned result slot for a single plugin mutation.
///
/// Diagnostics are informational only. The host validates `code`, `flags`,
/// `diagnostic_len`, and UTF-8 before using them; behaviour is selected only by
/// the structured status code.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginApplyResult {
    pub code: u8,
    pub flags: u8,
    pub diagnostic_len: u16,
    /// Minimum retry delay in milliseconds for `RateLimited`; zero otherwise.
    pub retry_after_ms: u64,
    pub diagnostic: [u8; PLUGIN_APPLY_DIAGNOSTIC_CAPACITY],
}

impl PluginApplyResult {
    #[must_use]
    pub const fn applied() -> Self {
        Self {
            code: PluginApplyStatus::Applied.to_abi(),
            flags: 0,
            diagnostic_len: 0,
            retry_after_ms: 0,
            diagnostic: [0; PLUGIN_APPLY_DIAGNOSTIC_CAPACITY],
        }
    }

    #[must_use]
    pub fn unsupported(diagnostic: impl AsRef<str>) -> Self {
        Self::failure(PluginApplyStatus::Unsupported, diagnostic.as_ref())
    }

    #[must_use]
    pub fn invalid_argument(diagnostic: impl AsRef<str>) -> Self {
        Self::failure(PluginApplyStatus::InvalidArgument, diagnostic.as_ref())
    }

    #[must_use]
    pub fn io(diagnostic: impl AsRef<str>) -> Self {
        Self::failure(PluginApplyStatus::Io, diagnostic.as_ref())
    }

    #[must_use]
    pub fn unavailable(diagnostic: impl AsRef<str>) -> Self {
        Self::failure(PluginApplyStatus::Unavailable, diagnostic.as_ref())
    }

    #[must_use]
    pub fn rate_limited(diagnostic: impl AsRef<str>) -> Self {
        Self::failure(PluginApplyStatus::RateLimited, diagnostic.as_ref())
    }

    /// Constructs a rate-limit result with provider retry guidance.
    #[must_use]
    pub fn rate_limited_after(diagnostic: impl AsRef<str>, retry_after: Duration) -> Self {
        let mut result = Self::rate_limited(diagnostic);
        result.retry_after_ms = u64::try_from(retry_after.as_millis()).unwrap_or(u64::MAX);
        result
    }

    #[must_use]
    pub fn internal(diagnostic: impl AsRef<str>) -> Self {
        Self::failure(PluginApplyStatus::Internal, diagnostic.as_ref())
    }

    fn failure(status: PluginApplyStatus, message: &str) -> Self {
        let mut length = message.len().min(PLUGIN_APPLY_DIAGNOSTIC_CAPACITY);
        while !message.is_char_boundary(length) {
            length -= 1;
        }
        let mut diagnostic = [0; PLUGIN_APPLY_DIAGNOSTIC_CAPACITY];
        if let (Some(destination), Some(source)) = (
            diagnostic.get_mut(..length),
            message.as_bytes().get(..length),
        ) {
            destination.copy_from_slice(source);
        }
        Self {
            code: status.to_abi(),
            flags: u8::from(length < message.len()) * PLUGIN_APPLY_DIAGNOSTIC_TRUNCATED,
            diagnostic_len: u16::try_from(length).unwrap_or(u16::MAX),
            retry_after_ms: 0,
            diagnostic,
        }
    }

    /// Validates and decodes an untrusted result slot.
    ///
    /// # Errors
    ///
    /// Returns a static diagnostic when the status, flags, length, or UTF-8
    /// payload violates the ABI contract.
    pub fn decode(&self) -> Result<(PluginApplyStatus, &str), &'static str> {
        let status = PluginApplyStatus::from_abi(self.code).ok_or("unknown apply status code")?;
        if self.flags & !PLUGIN_APPLY_DIAGNOSTIC_TRUNCATED != 0 {
            return Err("unknown apply result flags");
        }
        let length = usize::from(self.diagnostic_len);
        let bytes = self
            .diagnostic
            .get(..length)
            .ok_or("apply diagnostic length exceeds capacity")?;
        let diagnostic = str::from_utf8(bytes).map_err(|_| "apply diagnostic is not UTF-8")?;
        Ok((status, diagnostic))
    }

    /// Returns provider retry guidance for a rate-limit result.
    #[must_use]
    pub const fn retry_after_ms(&self) -> Option<u64> {
        if matches!(
            PluginApplyStatus::from_abi(self.code),
            Some(PluginApplyStatus::RateLimited)
        ) && self.retry_after_ms != 0
        {
            Some(self.retry_after_ms)
        } else {
            None
        }
    }
}

impl fmt::Debug for PluginApplyResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PluginApplyResult")
            .field("decoded", &self.decode())
            .finish()
    }
}

/// A typed plugin failure mapped onto the native ABI result codes.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PluginError {
    /// The target is not owned by the plugin or is malformed for its topology.
    #[error("{0}")]
    InvalidTarget(String),
    /// The operation or one of its arguments is invalid for the target.
    #[error("{0}")]
    InvalidArgument(String),
    /// The operation is valid in the shared model but unsupported here.
    #[error("{0}")]
    Unsupported(String),
    /// The addressed hardware is temporarily absent or unreachable.
    #[error("{0}")]
    Unavailable(String),
    /// Work was refused temporarily and may succeed after the stated delay.
    #[error("{diagnostic}")]
    RateLimited {
        /// Human-readable explanation of the refusal.
        diagnostic: String,
        /// Minimum delay before retrying.
        retry_after: Duration,
    },
    /// A transport operation failed while communicating with hardware.
    #[error("{0}")]
    Io(String),
    /// The plugin violated an invariant or encountered an internal defect.
    #[error("{0}")]
    Internal(String),
}

impl PluginError {
    /// Converts this error to the closest existing ABI result category.
    #[must_use]
    pub fn into_apply_result(self) -> PluginApplyResult {
        match self {
            Self::InvalidTarget(diagnostic) | Self::InvalidArgument(diagnostic) => {
                PluginApplyResult::invalid_argument(diagnostic)
            }
            Self::Unsupported(diagnostic) => PluginApplyResult::unsupported(diagnostic),
            Self::Unavailable(diagnostic) => PluginApplyResult::unavailable(diagnostic),
            Self::RateLimited {
                diagnostic,
                retry_after,
            } => PluginApplyResult::rate_limited_after(diagnostic, retry_after),
            Self::Io(diagnostic) => PluginApplyResult::io(diagnostic),
            Self::Internal(diagnostic) => PluginApplyResult::internal(diagnostic),
        }
    }

    /// Returns the human-readable diagnostic without discarding its category.
    #[must_use]
    pub fn diagnostic(&self) -> &str {
        match self {
            Self::InvalidTarget(diagnostic)
            | Self::InvalidArgument(diagnostic)
            | Self::Unsupported(diagnostic)
            | Self::Unavailable(diagnostic)
            | Self::Io(diagnostic)
            | Self::Internal(diagnostic)
            | Self::RateLimited { diagnostic, .. } => diagnostic,
        }
    }
}

/// Writes one result to a caller-owned ABI slot. Returns `0` when the pointer
/// is null and `1` after a successful write.
///
/// # Safety
///
/// A non-null `result` must point to one writable, properly aligned
/// [`PluginApplyResult`] for the duration of this call.
pub unsafe fn write_apply_result(result: *mut PluginApplyResult, value: PluginApplyResult) -> u8 {
    if result.is_null() {
        return 0;
    }
    // SAFETY: upheld by the caller as documented above.
    unsafe { result.write(value) };
    1
}

/// Decodes one CBOR update, passes it to `apply`, and writes the result.
///
/// Invalid pointers and CBOR are handled consistently for every
/// plugin that uses this adapter. Device-specific validation remains in
/// `apply`.
///
/// # Safety
///
/// Either pointer may be null. Otherwise, `update_cbor` must point to
/// `update_len` bytes that remain valid for this call, and `result` must
/// point to one writable, properly aligned [`PluginApplyResult`].
pub unsafe fn dispatch_update_cbor(
    update_cbor: *const u8,
    update_len: usize,
    result: *mut PluginApplyResult,
    apply: impl FnOnce(&PluginUpdate) -> PluginApplyResult,
) -> u8 {
    if result.is_null() {
        tracing::warn!("received null update result buffer");
        return 0;
    }

    let outcome = if update_cbor.is_null() {
        tracing::warn!("received null update payload");
        PluginApplyResult::invalid_argument("received null update payload")
    } else {
        // SAFETY: upheld by the caller; null was checked above.
        let payload = unsafe { slice::from_raw_parts(update_cbor, update_len) };
        let update = match ciborium::from_reader(payload) {
            Ok(update) => update,
            Err(error) => {
                tracing::warn!(error = %error, "failed to decode CBOR update");
                // SAFETY: `result` was checked above and is valid by the
                // caller's contract.
                return unsafe {
                    write_apply_result(
                        result,
                        PluginApplyResult::invalid_argument(error.to_string()),
                    )
                };
            }
        };
        apply(&update)
    };

    // SAFETY: `result` was checked above and is valid by the caller's contract.
    unsafe { write_apply_result(result, outcome) }
}

/// Decodes one CBOR [`PluginFrameUpload`] (target plus frame envelope,
/// mirroring how [`dispatch_update_cbor`] decodes a target-bearing
/// [`PluginUpdate`]), passes it to `upload`, and writes the result. Stream
/// ownership, generation/sequence ordering, and rate limiting are host-side
/// concerns applied before this callback is ever invoked.
///
/// # Safety
///
/// Either pointer may be null. Otherwise, `frame_upload_cbor` must point to a
/// byte string that remains valid for this call, and `result` must point to
/// one writable, properly aligned [`PluginApplyResult`].
pub unsafe fn dispatch_frame_upload_cbor(
    frame_upload_cbor: *const u8,
    frame_upload_len: usize,
    result: *mut PluginApplyResult,
    upload: impl FnOnce(&PluginFrameUpload) -> PluginApplyResult,
) -> u8 {
    if result.is_null() {
        tracing::warn!("received null frame result buffer");
        return 0;
    }

    let outcome = if frame_upload_cbor.is_null() {
        tracing::warn!("received null frame payload");
        PluginApplyResult::invalid_argument("received null frame payload")
    } else {
        // SAFETY: upheld by the caller; null was checked above.
        let payload = unsafe { slice::from_raw_parts(frame_upload_cbor, frame_upload_len) };
        let frame_upload = match ciborium::from_reader(payload) {
            Ok(frame_upload) => frame_upload,
            Err(error) => {
                tracing::warn!(error = %error, "failed to decode CBOR frame upload");
                // SAFETY: `result` was checked above and is valid by the
                // caller's contract.
                return unsafe {
                    write_apply_result(
                        result,
                        PluginApplyResult::invalid_argument(error.to_string()),
                    )
                };
            }
        };
        upload(&frame_upload)
    };

    // SAFETY: `result` was checked above and is valid by the caller's contract.
    unsafe { write_apply_result(result, outcome) }
}

/// Decodes a CBOR batch and applies each update in order.
///
/// This adapter suits plugins that do not need to coalesce a batch into one
/// hardware transaction. Plugins with a native batch operation should decode
/// and process the batch as a whole instead.
///
/// # Safety
///
/// Either pointer may be null. Otherwise, `batch_cbor` must point to
/// `batch_len` bytes that remain valid for this call, and `results` must
/// point to `results_len` writable, properly aligned [`PluginApplyResult`]
/// values.
pub unsafe fn dispatch_update_batch_cbor(
    batch_cbor: *const u8,
    batch_len: usize,
    results: *mut PluginApplyResult,
    results_len: usize,
    mut apply: impl FnMut(&PluginUpdate) -> PluginApplyResult,
) -> u8 {
    // SAFETY: upheld by the caller.
    let Some(batch) = (unsafe { decode_update_batch(batch_cbor, batch_len, results, results_len) })
    else {
        return 0;
    };

    // SAFETY: the caller guarantees this buffer, and its length now matches
    // the decoded update count (checked above).
    let results = unsafe { slice::from_raw_parts_mut(results, results_len) };
    for (slot, update) in results.iter_mut().zip(&batch.updates) {
        *slot = apply(update);
    }
    1
}

/// Decodes a CBOR-encoded update batch and validates it against the caller's
/// results-slot count, without applying any update. Shared by
/// [`dispatch_update_batch_cbor`] and the SDK's native-batch adapter, which
/// otherwise duplicate this exact null/decode/length-validation preamble
/// before diverging on how they apply the batch.
///
/// # Safety
///
/// Either pointer may be null. Otherwise, `batch_cbor` must point to
/// `batch_len` bytes that remain valid for this call, and `results` is only
/// checked for null here (the caller writes through it afterwards, subject to
/// its own safety contract).
pub(crate) unsafe fn decode_update_batch(
    batch_cbor: *const u8,
    batch_len: usize,
    results: *const PluginApplyResult,
    results_len: usize,
) -> Option<PluginUpdateBatch> {
    if batch_cbor.is_null() || results.is_null() {
        tracing::warn!("received null batch payload or results buffer");
        return None;
    }

    // SAFETY: upheld by the caller; null was checked above.
    let payload = unsafe { slice::from_raw_parts(batch_cbor, batch_len) };
    let batch: PluginUpdateBatch = match ciborium::from_reader(payload) {
        Ok(batch) => batch,
        Err(error) => {
            tracing::warn!(error = %error, "failed to parse batch");
            return None;
        }
    };

    if batch.updates.len() != results_len {
        tracing::warn!(
            expected = results_len,
            actual = batch.updates.len(),
            "batch results buffer length mismatch"
        );
        return None;
    }

    Some(batch)
}

#[cfg(test)]
#[path = "apply_tests.rs"]
mod tests;
