// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Generic CBOR and C-string encoding helpers used across the plugin ABI.

use std::ffi::CString;

use serde::{Deserialize, Serialize};

/// Decodes one CBOR ABI payload into a typed plugin model.
///
/// # Errors
///
/// Returns a diagnostic when the payload is not valid CBOR for `T`.
pub fn decode_cbor<T: for<'de> Deserialize<'de>>(payload: &[u8]) -> Result<T, String> {
    ciborium::from_reader(payload).map_err(|error| error.to_string())
}

/// Encodes one typed plugin model for a CBOR ABI payload.
///
/// # Errors
///
/// Returns a diagnostic if `value` cannot be represented as CBOR.
pub fn encode_cbor<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, String> {
    let mut payload = Vec::new();
    ciborium::into_writer(value, &mut payload).map_err(|error| error.to_string())?;
    Ok(payload)
}

/// Turns an owned string into the NUL-terminated [`CString`] a plugin hands
/// back across the C ABI, stripping any interior NUL bytes first.
///
/// Stripping lets the terminator be appended without another validating scan.
#[must_use]
pub fn sanitize_cstring(value: String) -> CString {
    let mut bytes = value.into_bytes();
    bytes.retain(|byte| *byte != 0);
    bytes.push(0);
    // SAFETY: interior NULs were removed above and exactly one trailing
    // terminator appended, so `bytes` is a valid NUL-terminated C string.
    unsafe { CString::from_vec_with_nul_unchecked(bytes) }
}

#[cfg(test)]
#[path = "codec_tests.rs"]
mod tests;
