// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Bounded authentication exchange performed after protocol negotiation.

use std::fmt;
use std::time::SystemTime;

use luminate_core::policy::{PrincipalId, SessionScope};
use serde::de::Error;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Maximum credential size accepted by the consumer protocol.
pub const MAX_CREDENTIAL_BYTES: usize = 16 * 1024;

/// Maximum opaque provider continuation size.
pub const MAX_CONTINUATION_BYTES: usize = 16 * 1024;

/// A credential buffer that is redacted from diagnostics and erased on drop.
#[derive(Clone, Serialize, Zeroize, ZeroizeOnDrop)]
#[serde(transparent)]
pub struct Credential(Vec<u8>);

impl Credential {
    /// Copies a bounded credential into storage that is redacted from
    /// diagnostics and zeroized on drop.
    ///
    /// # Errors
    ///
    /// Returns an error when the credential is empty or exceeds
    /// [`MAX_CREDENTIAL_BYTES`].
    pub fn new(value: impl AsRef<[u8]>) -> Result<Self, CredentialError> {
        let value = value.as_ref();
        if value.is_empty() {
            return Err(CredentialError::Empty);
        }
        if value.len() > MAX_CREDENTIAL_BYTES {
            return Err(CredentialError::TooLarge {
                actual: value.len(),
                maximum: MAX_CREDENTIAL_BYTES,
            });
        }
        Ok(Self(value.to_vec()))
    }

    /// Borrows the credential for transmission or verification.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for Credential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Credential([REDACTED])")
    }
}

impl<'de> Deserialize<'de> for Credential {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut value = Vec::<u8>::deserialize(deserializer)?;
        let credential = Self::new(&value).map_err(Error::custom);
        value.zeroize();
        credential
    }
}

/// Invalid credential input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CredentialError {
    /// Credentials cannot be empty.
    #[error("credential is empty")]
    Empty,

    /// The credential exceeds the accepted size limit.
    #[error("credential is {actual} bytes; maximum is {maximum}")]
    TooLarge {
        /// Supplied byte length.
        actual: usize,

        /// Accepted byte length.
        maximum: usize,
    },
}

/// Opaque provider state required to revalidate an authenticated session.
///
/// Continuations are redacted from diagnostics and erased on drop. They are
/// distinct from credentials so their bounds and validation remain tied to
/// the provider-session lifecycle rather than the client authentication
/// exchange.
#[derive(Clone, Serialize, Zeroize, ZeroizeOnDrop)]
#[serde(transparent)]
pub struct Continuation(Vec<u8>);

impl Continuation {
    /// Copies a bounded provider continuation into protected storage.
    ///
    /// # Errors
    ///
    /// Returns an error when the continuation is empty or exceeds
    /// [`MAX_CONTINUATION_BYTES`].
    pub fn new(value: impl AsRef<[u8]>) -> Result<Self, ContinuationError> {
        let value = value.as_ref();
        if value.is_empty() {
            return Err(ContinuationError::Empty);
        }
        if value.len() > MAX_CONTINUATION_BYTES {
            return Err(ContinuationError::TooLarge {
                actual: value.len(),
                maximum: MAX_CONTINUATION_BYTES,
            });
        }
        Ok(Self(value.to_vec()))
    }

    /// Borrows the continuation for transmission or revalidation.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for Continuation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Continuation([REDACTED])")
    }
}

impl<'de> Deserialize<'de> for Continuation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut value = Vec::<u8>::deserialize(deserializer)?;
        let continuation = Self::new(&value).map_err(Error::custom);
        value.zeroize();
        continuation
    }
}

/// Invalid provider-continuation input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ContinuationError {
    /// Continuations cannot be empty.
    #[error("provider continuation is empty")]
    Empty,

    /// The continuation exceeds the accepted size limit.
    #[error("provider continuation is {actual} bytes; maximum is {maximum}")]
    TooLarge {
        /// Supplied byte length.
        actual: usize,

        /// Accepted byte length.
        maximum: usize,
    },
}

/// Authentication request for one connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticationRequest {
    /// Authentication method and credential selected by the caller.
    pub authentication: Authentication,

    /// Optional voluntary reduction of the connection's authority.
    pub scope: Option<SessionScope>,
}

/// Authentication method selected for one connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "method")]
pub enum Authentication {
    /// Bind the session directly to kernel peer credentials.
    Peer,

    /// Authenticate with a daemon-managed opaque bearer token.
    Bearer {
        /// Secret token value.
        credential: Credential,
    },

    /// Authenticate a front-end delegation using configured actor attestation.
    Attestation {
        /// Configured attestation name.
        name: String,

        /// Opaque attestation value.
        credential: Credential,
    },

    /// Dispatch a credential to a configured external provider.
    External {
        /// Provider configuration name.
        provider: String,

        /// Opaque provider credential.
        credential: Credential,
    },
}

/// Sanitized authentication source returned to clients and audit records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "method")]
pub enum AuthenticationSource {
    /// Kernel peer credentials.
    Peer,

    /// Daemon-managed token.
    Bearer,

    /// Configured front-end attestation.
    Attestation {
        /// Administrator-selected attestation name.
        name: String,
    },

    /// Configured external provider.
    External {
        /// Configured provider name.
        provider: String,
    },
}

/// Sanitized metadata for one actor-bound front-end attestation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestationMetadata {
    /// Administrator-selected record name.
    pub name: String,

    /// Subject authenticated by this attestation.
    pub subject: PrincipalId,

    /// Groups verified by the front end for the attested subject.
    pub verified_groups: Vec<String>,

    /// Non-secret identifier included in audit records.
    pub credential_id: String,

    /// Expiry of the attestation, when finite.
    pub expires_at: Option<SystemTime>,
}

/// Caller-visible metadata for the authenticated connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMetadata {
    /// Authenticated subject.
    pub subject: PrincipalId,

    /// Verified groups supplied by the authentication method.
    pub verified_groups: Vec<String>,

    /// Sanitized authentication method.
    pub source: AuthenticationSource,

    /// Non-secret credential identifier, when one exists.
    pub credential_id: Option<String>,

    /// Authentication expiry, when finite.
    pub expires_at: Option<SystemTime>,
}

/// Sanitized metadata for one daemon-managed bearer token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenMetadata {
    /// Stable, non-secret token identifier.
    pub id: String,

    /// Subject authenticated by the token.
    pub subject: PrincipalId,

    /// Credential expiry, when finite.
    pub expires_at: Option<SystemTime>,

    /// Whether the token has been revoked.
    pub revoked: bool,
}

/// Result of the post-negotiation authentication phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "outcome")]
pub enum AuthenticationResponse {
    /// Authentication succeeded and the connection is ready for requests.
    Authenticated {
        /// Metadata for the authenticated session.
        session: SessionMetadata,
    },

    /// Authentication failed without exposing provider or credential details.
    Rejected {
        /// Sanitized explanation suitable for the caller.
        reason: String,
    },
}

/// A short-lived, one-use credential for binding an event connection to a
/// primary authenticated session.
#[derive(Clone, Serialize, Zeroize, ZeroizeOnDrop)]
#[serde(transparent)]
pub struct EventTicket(Credential);

impl EventTicket {
    /// Constructs a ticket from bounded opaque bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is empty or too large.
    pub fn new(value: impl AsRef<[u8]>) -> Result<Self, CredentialError> {
        Credential::new(value).map(Self)
    }

    /// Borrows the ticket for transmission or verification.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        self.0.expose()
    }
}

impl fmt::Debug for EventTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EventTicket([REDACTED])")
    }
}

impl PartialEq for EventTicket {
    fn eq(&self, other: &Self) -> bool {
        let left = self.expose();
        let right = other.expose();
        if left.len() != right.len() {
            return false;
        }
        left.iter()
            .zip(right)
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
    }
}

impl Eq for EventTicket {}

impl<'de> Deserialize<'de> for EventTicket {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Credential::deserialize(deserializer).map(Self)
    }
}

#[cfg(test)]
#[path = "authentication_tests.rs"]
mod tests;
