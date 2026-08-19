// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Independently versioned protocol for executable authentication providers.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::{AUTHENTICATION_PROVIDER_PROTOCOL_VERSION, Continuation, Credential};

/// Maximum provider name, authority, subject, group, and identifier length.
pub const MAX_PROVIDER_TEXT_BYTES: usize = 1024;
/// Maximum groups returned for one subject.
pub const MAX_PROVIDER_GROUPS: usize = 256;

/// First daemon message sent to a newly spawned provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHello {
    /// Protocol version required by the daemon.
    pub protocol_version: u32,

    /// Provider-private initialization data.
    pub initialization: Credential,
}

impl ProviderHello {
    /// Constructs a provider hello for the current protocol version.
    #[must_use]
    pub const fn new(initialization: Credential) -> Self {
        Self {
            protocol_version: AUTHENTICATION_PROVIDER_PROTOCOL_VERSION,
            initialization,
        }
    }
}

/// Provider readiness response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "outcome")]
pub enum ProviderHelloResponse {
    /// The provider is ready for exchanges.
    Ready {
        /// Protocol version supported by the provider.
        protocol_version: u32,
    },

    /// Initialization failed with a sanitized diagnostic.
    Rejected {
        /// Sanitized explanation of the initialization failure.
        reason: String,
    },
}

/// One multiplexable provider exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "exchange")]
pub enum ProviderRequest {
    /// Authenticate a new credential.
    Authenticate {
        /// Concurrent exchange identifier.
        id: u64,

        /// Opaque client credential.
        credential: Credential,
    },

    /// Refresh an existing provider-issued lease.
    Revalidate {
        /// Concurrent exchange identifier.
        id: u64,

        /// Opaque continuation returned by the provider.
        continuation: Continuation,
    },

    /// Gracefully stop the provider.
    Shutdown,
}

/// Successful provider identity facts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderIdentity {
    /// Authority-local subject. The daemon supplies the configured authority.
    pub subject: String,

    /// Verified provider group claims.
    pub verified_groups: Vec<String>,

    /// Sanitized identifier for audit and revocation.
    pub credential_id: String,

    /// Credential expiry.
    pub expires_at: SystemTime,

    /// Opaque state required by the next revalidation.
    pub continuation: Continuation,
}

/// Response to an authentication or revalidation exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "outcome")]
pub enum ProviderResponse {
    /// Authentication succeeded.
    Authenticated {
        /// Identifier copied from the corresponding request.
        id: u64,

        /// Identity established by the provider.
        identity: ProviderIdentity,
    },
    /// Authentication failed without exposing credential details.
    Rejected {
        /// Identifier copied from the corresponding request.
        id: u64,

        /// Sanitized explanation suitable for the caller.
        reason: String,
    },
}

/// Validates all provider-controlled text and collection bounds.
///
/// # Errors
///
/// Returns a sanitized error when a provider response exceeds a bound or
/// contains empty identity fields.
pub fn validate_identity(identity: &ProviderIdentity) -> Result<(), &'static str> {
    validate_text(&identity.subject)?;
    validate_text(&identity.credential_id)?;
    if identity.verified_groups.len() > MAX_PROVIDER_GROUPS {
        return Err("provider returned too many groups");
    }
    for group in &identity.verified_groups {
        validate_text(group)?;
    }
    Ok(())
}

fn validate_text(value: &str) -> Result<(), &'static str> {
    if value.is_empty() {
        return Err("provider returned an empty identity field");
    }
    if value.len() > MAX_PROVIDER_TEXT_BYTES {
        return Err("provider returned an oversized identity field");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> ProviderIdentity {
        ProviderIdentity {
            subject: "alice".to_owned(),
            verified_groups: vec!["operators".to_owned()],
            credential_id: "key-1".to_owned(),
            expires_at: SystemTime::now(),
            continuation: Continuation::new(b"continuation").expect("valid continuation"),
        }
    }

    #[test]
    fn provider_identity_bounds_are_enforced() {
        assert_eq!(validate_identity(&identity()), Ok(()));

        let mut oversized = identity();
        oversized.subject = "x".repeat(MAX_PROVIDER_TEXT_BYTES + 1);
        assert!(validate_identity(&oversized).is_err());

        let mut too_many_groups = identity();
        too_many_groups.verified_groups = (0..=MAX_PROVIDER_GROUPS)
            .map(|index| index.to_string())
            .collect();
        assert!(validate_identity(&too_many_groups).is_err());
    }
}
