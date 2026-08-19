// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::{
    Client, Error, PolicyDocument, PolicyRevision, PrincipalId, Request, ResponseStatus, Result,
    SystemTime, TokenMetadata, unexpected_response,
};

/// Daemon access-policy administration for one authenticated client.
#[derive(Debug, Clone, Copy)]
pub struct PolicyAdministration<'client> {
    pub(super) client: &'client Client,
}

#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "Administration responses share the client's standard defensive unexpected-response handling."
)]
impl PolicyAdministration<'_> {
    /// Reads the active validated policy document.
    ///
    /// # Errors
    ///
    /// Returns an authorization, availability, or protocol error.
    pub async fn get(self) -> Result<PolicyDocument> {
        match self.client.request(Request::GetAccessPolicy).await?.status {
            ResponseStatus::AccessPolicy(source) => PolicyDocument::new(*source).map_err(|error| {
                Error::Protocol(format!("daemon returned invalid policy: {error}"))
            }),
            other => Err(unexpected_response("access policy", &other)),
        }
    }

    /// Atomically replaces the active policy at `expected`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] for a stale revision, or another
    /// authorization, persistence, validation, or protocol error.
    pub async fn replace(
        self,
        expected: PolicyRevision,
        replacement: PolicyDocument,
    ) -> Result<PolicyDocument> {
        match self
            .client
            .request(Request::ReplaceAccessPolicy {
                expected,
                replacement: replacement.source().clone(),
            })
            .await?
            .status
        {
            ResponseStatus::AccessPolicy(source) => PolicyDocument::new(*source).map_err(|error| {
                Error::Protocol(format!("daemon returned invalid policy: {error}"))
            }),
            other => Err(unexpected_response("replaced access policy", &other)),
        }
    }
}

/// Display-once result of creating a daemon bearer token.
#[derive(Debug, Clone)]
pub struct CreatedToken {
    /// Sanitized persistent token metadata.
    pub metadata: TokenMetadata,

    /// Secret credential returned only by the creation call.
    pub secret: luminate_protocol::Credential,
}

/// Display-once result of creating an actor-bound attestation.
#[derive(Debug, Clone)]
pub struct CreatedAttestation {
    /// Sanitized attestation metadata.
    pub metadata: luminate_protocol::AttestationMetadata,

    /// Secret credential returned only by the creation call.
    pub secret: luminate_protocol::Credential,
}

/// Daemon authentication-record administration for one authenticated client.
#[derive(Debug, Clone, Copy)]
pub struct AuthenticationAdministration<'client> {
    pub(super) client: &'client Client,
}

#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "Administration responses share the client's standard defensive unexpected-response handling."
)]
impl AuthenticationAdministration<'_> {
    /// Creates an attestation bound to this connection's kernel actor.
    ///
    /// # Errors
    ///
    /// Returns an authorization, validation, availability, or protocol error.
    pub async fn create_attestation(
        self,
        name: impl Into<String>,
        subject: PrincipalId,
        expires_at: Option<SystemTime>,
    ) -> Result<CreatedAttestation> {
        self.create_principal_attestation(name, subject, Vec::new(), expires_at)
            .await
    }

    /// Creates an attestation carrying a front-end-verified principal.
    ///
    /// # Errors
    ///
    /// Returns an authorization, registration, validation, availability, or
    /// protocol error.
    pub async fn create_principal_attestation(
        self,
        name: impl Into<String>,
        subject: PrincipalId,
        verified_groups: Vec<String>,
        expires_at: Option<SystemTime>,
    ) -> Result<CreatedAttestation> {
        match self
            .client
            .request(Request::CreateAttestation {
                name: name.into(),
                subject,
                verified_groups,
                expires_at,
            })
            .await?
            .status
        {
            ResponseStatus::AttestationCreated { metadata, secret } => {
                Ok(CreatedAttestation { metadata, secret })
            }
            other => Err(unexpected_response("created attestation", &other)),
        }
    }

    /// Lists attestations bound to this connection's kernel actor.
    ///
    /// # Errors
    ///
    /// Returns an authorization, availability, or protocol error.
    pub async fn list_attestations(self) -> Result<Vec<luminate_protocol::AttestationMetadata>> {
        match self.client.request(Request::ListAttestations).await?.status {
            ResponseStatus::Attestations(records) => Ok(records),
            other => Err(unexpected_response("attestation list", &other)),
        }
    }

    /// Revokes an attestation bound to this connection's kernel actor.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] for an unknown record, or another
    /// authorization, availability, or protocol error.
    pub async fn revoke_attestation(self, name: impl Into<String>) -> Result<()> {
        self.client
            .expect_ack(Request::RevokeAttestation { name: name.into() })
            .await
    }

    /// Creates a daemon-managed bearer token.
    ///
    /// # Errors
    ///
    /// Returns an authorization, validation, persistence, or protocol error.
    pub async fn create_token(
        self,
        id: impl Into<String>,
        subject: PrincipalId,
        expires_at: Option<SystemTime>,
    ) -> Result<CreatedToken> {
        match self
            .client
            .request(Request::CreateToken {
                id: id.into(),
                subject,
                expires_at,
            })
            .await?
            .status
        {
            ResponseStatus::TokenCreated { metadata, secret } => {
                Ok(CreatedToken { metadata, secret })
            }
            other => Err(unexpected_response("created token", &other)),
        }
    }

    /// Lists sanitized daemon token records.
    ///
    /// # Errors
    ///
    /// Returns an authorization, availability, or protocol error.
    pub async fn list_tokens(self) -> Result<Vec<TokenMetadata>> {
        match self.client.request(Request::ListTokens).await?.status {
            ResponseStatus::Tokens(tokens) => Ok(tokens),
            other => Err(unexpected_response("token list", &other)),
        }
    }

    /// Rotates a token, returning its new display-once secret and closing
    /// sessions authenticated with the prior secret.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] for an unknown token, or another
    /// authorization, persistence, or protocol error.
    pub async fn rotate_token(
        self,
        id: impl Into<String>,
        expires_at: Option<SystemTime>,
    ) -> Result<CreatedToken> {
        match self
            .client
            .request(Request::RotateToken {
                id: id.into(),
                expires_at,
            })
            .await?
            .status
        {
            ResponseStatus::TokenCreated { metadata, secret } => {
                Ok(CreatedToken { metadata, secret })
            }
            other => Err(unexpected_response("rotated token", &other)),
        }
    }

    /// Revokes a token and closes its active sessions.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] for an unknown or already revoked token,
    /// or another authorization, persistence, or protocol error.
    pub async fn revoke_token(self, id: impl Into<String>) -> Result<()> {
        self.client
            .expect_ack(Request::RevokeToken { id: id.into() })
            .await
    }
}
