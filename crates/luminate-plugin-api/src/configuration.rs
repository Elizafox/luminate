// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Immutable daemon-supplied configuration for the current plugin instance.
//!
//! A plugin's explicit `[[plugins]]` entry may contain a nested `config` table.
//! The daemon serializes that table to CBOR and delivers it privately to each
//! supervised plugin host during initialization. The declaration macro stores
//! it here before calling the plugin's optional `init` function.

use std::io;
use std::slice;
use std::sync::OnceLock;

use ciborium::de::Error as CborError;
use serde::de::DeserializeOwned;

static CONFIGURATION: OnceLock<Vec<u8>> = OnceLock::new();
const EMPTY_CONFIGURATION: &[u8] = &[0xa0];

/// Deserializes this plugin instance's configuration into a plugin-owned type.
///
/// An unconfigured or autoloaded plugin receives an empty CBOR map. Plugin
/// implementations should normally use `#[serde(default, deny_unknown_fields)]`
/// so defaults are explicit and misspelled options fail during their own
/// initialization.
///
/// # Errors
///
/// Returns a CBOR type/schema error when the daemon-supplied object cannot be
/// represented by `T`.
pub fn deserialize<T: DeserializeOwned>() -> Result<T, CborError<io::Error>> {
    ciborium::from_reader(CONFIGURATION.get_or_init(empty_configuration).as_slice())
}

/// Returns whether the daemon supplied an empty configuration table.
#[must_use]
pub fn is_empty() -> bool {
    matches!(
        ciborium::from_reader::<ciborium::Value, _>(
            CONFIGURATION.get_or_init(empty_configuration).as_slice()
        ),
        Ok(ciborium::Value::Map(entries)) if entries.is_empty()
    )
}

fn empty_configuration() -> Vec<u8> {
    EMPTY_CONFIGURATION.to_vec()
}

/// Installs the host-owned configuration during the ABI initialization call.
///
/// # Safety
///
/// `configuration_cbor` must be null or point to `configuration_len` bytes
/// valid for this call. The host passes a non-null CBOR map; defensive
/// fallbacks keep a mismatched caller from unwinding across FFI.
#[doc(hidden)]
pub unsafe fn init(configuration_cbor: *const u8, configuration_len: usize) {
    let bytes = if configuration_cbor.is_null() {
        empty_configuration()
    } else {
        // SAFETY: upheld by the caller; the bytes are copied before this
        // function returns.
        let bytes = unsafe { slice::from_raw_parts(configuration_cbor, configuration_len) };
        decode_configuration_bytes(bytes)
    };
    let _already_initialized = CONFIGURATION.set(bytes);
}

/// Validates that daemon-supplied CBOR is a map, falling back to an empty map
/// for malformed CBOR or a non-map value.
/// Split out from `init` so this fallback logic is testable without touching
/// the process-global `CONFIGURATION` cell.
fn decode_configuration_bytes(bytes: &[u8]) -> Vec<u8> {
    match ciborium::from_reader::<ciborium::Value, _>(bytes) {
        Ok(ciborium::Value::Map(_)) => bytes.to_vec(),
        _ => empty_configuration(),
    }
}

#[cfg(test)]
#[path = "configuration_tests.rs"]
mod tests;
