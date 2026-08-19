// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Connection authentication and one-use event-session tickets.

use std::collections::HashMap;
#[cfg(test)]
use std::fs;
use std::io::{ErrorKind, Read as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use luminate_core::policy::{Principal as SessionPrincipal, PrincipalId, SessionScope};
use luminate_platform::secure_random::fill_bytes;
use luminate_platform::secure_storage::{
    ensure_service_directory, open_existing_file_without_following_symlinks,
    open_private_file_for_read,
};
use luminate_protocol::{
    AttestationMetadata, Authentication, AuthenticationRequest, AuthenticationResponse,
    AuthenticationSource, Credential, EventTicket, SessionMetadata, TokenMetadata,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use tokio::sync::{OwnedSemaphorePermit, broadcast};
use tokio::time::sleep;

use crate::atomic_file;
use crate::authentication_provider::AuthenticationProvider;
use crate::authorization::Principal;
use crate::device_config::AuthenticationProviderConfig;

const EVENT_TICKET_BYTES: usize = 32;
const EVENT_TICKET_LIFETIME: Duration = Duration::from_secs(30);
const MAX_EVENT_TICKETS: usize = 1_024;
const MAX_EVENT_TICKETS_PER_ACTOR: usize = 32;
const MAX_TICKET_GENERATION_ATTEMPTS: usize = 8;
const MAX_ATTESTATIONS: usize = 1_024;
const MAX_ATTESTATIONS_PER_ACTOR: usize = 64;
const TOKEN_BYTES: usize = 32;
const TOKEN_HASH_DOMAIN: &[u8] = b"luminate daemon token v1\0";
const MAX_TOKEN_FILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_PROVIDER_INITIALIZATION_BYTES: usize = 16 * 1024;
static TOKEN_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, thiserror::Error)]
pub(crate) enum EphemeralCapacityError {
    #[error("global event-ticket capacity reached")]
    GlobalEventTickets,
    #[error("event-ticket capacity reached for this actor")]
    ActorEventTickets,
    #[error("global attestation capacity reached")]
    GlobalAttestations,
    #[error("attestation capacity reached for this actor")]
    ActorAttestations,
}

/// Metadata and verifier for one daemon-managed bearer credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TokenRecord {
    id: String,
    subject: PrincipalId,
    hash: [u8; 32],
    expires_at: Option<SystemTime>,
    revoked: bool,
}

/// In-memory token index. Persistence and audited administration own the
/// serialized records; plaintext secrets never enter that representation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct TokenStore {
    records: HashMap<String, TokenRecord>,
}

impl TokenStore {
    /// Creates a token and returns its display-once secret.
    pub(crate) fn create(
        &mut self,
        id: String,
        subject: PrincipalId,
        expires_at: Option<SystemTime>,
    ) -> anyhow::Result<Credential> {
        anyhow::ensure!(!id.trim().is_empty(), "token ID must not be empty");
        anyhow::ensure!(!self.records.contains_key(&id), "token ID already exists");
        let secret = generate_token()?;
        self.records.insert(
            id.clone(),
            TokenRecord {
                id,
                subject,
                hash: token_hash(secret.expose()),
                expires_at,
                revoked: false,
            },
        );
        Ok(secret)
    }

    /// Revokes a token. Existing session revocation is handled by the
    /// connection lease registry that consumes this mutation.
    pub(crate) fn revoke(&mut self, id: &str) -> bool {
        self.records.get_mut(id).is_some_and(|record| {
            let changed = !record.revoked;
            record.revoked = true;
            changed
        })
    }

    fn rotate(&mut self, id: &str, expires_at: Option<SystemTime>) -> anyhow::Result<Credential> {
        let record = self
            .records
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("token ID does not exist"))?;
        let secret = generate_token()?;
        record.hash = token_hash(secret.expose());
        record.expires_at = expires_at;
        record.revoked = false;
        Ok(secret)
    }

    fn authenticate(&self, presented: &[u8], now: SystemTime) -> Option<&TokenRecord> {
        self.records.values().find(|record| {
            !record.revoked
                && record.expires_at.is_none_or(|expiry| expiry > now)
                && bool::from(record.hash.ct_eq(&token_hash(presented)))
        })
    }

    fn metadata(&self) -> Vec<TokenMetadata> {
        let mut records = self
            .records
            .values()
            .map(|record| TokenMetadata {
                id: record.id.clone(),
                subject: record.subject.clone(),
                expires_at: record.expires_at,
                revoked: record.revoked,
            })
            .collect::<Vec<_>>();
        records.sort_by(|left, right| left.id.cmp(&right.id));
        records
    }
}

fn token_hash(secret: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(TOKEN_HASH_DOMAIN);
    digest.update(secret);
    digest.finalize().into()
}

fn generate_token() -> anyhow::Result<Credential> {
    let mut bytes = [0_u8; TOKEN_BYTES];
    fill_bytes(&mut bytes)?;
    Credential::new(bytes).map_err(Into::into)
}

fn load_tokens(path: &Path) -> anyhow::Result<TokenStore> {
    let file = match open_private_file_for_read(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(TokenStore::default()),
        Err(error) => return Err(error.into()),
    };
    let limit = u64::try_from(MAX_TOKEN_FILE_BYTES)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut payload = Vec::new();
    file.take(limit).read_to_end(&mut payload)?;
    anyhow::ensure!(
        payload.len() <= MAX_TOKEN_FILE_BYTES,
        "token file exceeds the {MAX_TOKEN_FILE_BYTES} byte limit"
    );
    serde_json::from_slice(&payload).map_err(Into::into)
}

fn write_tokens(path: &Path, tokens: &TokenStore) -> anyhow::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    ensure_service_directory(parent)?;
    let payload = serde_json::to_vec_pretty(tokens)?;
    anyhow::ensure!(
        payload.len() <= MAX_TOKEN_FILE_BYTES,
        "token file exceeds the {MAX_TOKEN_FILE_BYTES} byte limit"
    );
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("tokens"),
        TOKEN_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    atomic_file::replace(path, &temporary, &payload)?;
    Ok(())
}

#[derive(Debug)]
struct PendingTicket {
    actor: Principal,
    expires_at: Instant,
    scope: Option<SessionScope>,
    subject: Option<SessionPrincipal>,
    frontend_actor: Option<SessionPrincipal>,
    credential_id: Option<String>,
    frontend_credential_id: Option<String>,
    authentication_expiry: Option<SystemTime>,
    revocation: Option<broadcast::Receiver<()>>,
}

pub(crate) struct ConsumedTicket {
    pub(crate) scope: Option<SessionScope>,
    pub(crate) subject: Option<SessionPrincipal>,
    pub(crate) frontend_actor: Option<SessionPrincipal>,
    pub(crate) credential_id: Option<String>,
    pub(crate) frontend_credential_id: Option<String>,
    pub(crate) revocation: Option<broadcast::Receiver<()>>,
    pub(crate) authentication_expiry: Option<SystemTime>,
}

/// Shared registry of short-lived tickets minted during primary authentication.
#[derive(Clone, Default)]
pub(crate) struct AuthenticationService {
    tickets: Arc<Mutex<HashMap<[u8; EVENT_TICKET_BYTES], PendingTicket>>>,
    tokens: Arc<Mutex<TokenStore>>,
    attestations: Arc<Mutex<HashMap<String, AttestationRecord>>>,
    providers: Arc<Mutex<HashMap<String, Arc<AuthenticationProvider>>>>,
    token_path: Arc<Option<PathBuf>>,
    token_sessions: Arc<Mutex<HashMap<String, broadcast::Sender<()>>>>,
}

#[derive(Debug, Clone)]
struct AttestationRecord {
    actor: Principal,
    frontend_actor: Option<SessionPrincipal>,
    frontend_credential_id: Option<String>,
    subject: PrincipalId,
    verified_groups: Vec<String>,
    credential_id: String,
    hash: [u8; 32],
    expires_at: Option<SystemTime>,
}

impl AuthenticationService {
    pub(crate) fn load(token_path: PathBuf) -> anyhow::Result<Self> {
        let tokens = load_tokens(&token_path)?;
        Ok(Self {
            tickets: Arc::default(),
            tokens: Arc::new(Mutex::new(tokens)),
            attestations: Arc::default(),
            providers: Arc::default(),
            token_path: Arc::new(Some(token_path)),
            token_sessions: Arc::default(),
        })
    }

    /// Starts configured providers and publishes only providers which completed
    /// their bounded handshake successfully.
    pub(crate) async fn start_providers(
        &self,
        configurations: &[AuthenticationProviderConfig],
    ) -> anyhow::Result<()> {
        for configuration in configurations {
            let result = start_provider(configuration).await;
            match result {
                Ok(provider) => {
                    tracing::info!(
                        provider = provider.name(),
                        authority = provider.authority(),
                        "external authentication provider is ready"
                    );
                    self.providers
                        .lock()
                        .expect("lock poisoned")
                        .insert(configuration.name.clone(), Arc::new(provider));
                }
                Err(error) if !configuration.required => {
                    tracing::warn!(
                        provider = configuration.name,
                        error = %error,
                        "optional external authentication provider is unavailable"
                    );
                }
                Err(error) => {
                    self.shutdown_providers().await;
                    return Err(error).with_context(|| {
                        format!(
                            "required external authentication provider {} failed to start",
                            configuration.name
                        )
                    });
                }
            }
        }
        Ok(())
    }

    /// Cooperatively stops every supervised external authentication provider.
    pub(crate) async fn shutdown_providers(&self) {
        let providers = self
            .providers
            .lock()
            .expect("lock poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();

        for provider in providers {
            provider.shutdown().await;
        }
    }

    pub(crate) fn create_token(
        &self,
        id: &str,
        subject: PrincipalId,
        expires_at: Option<SystemTime>,
    ) -> anyhow::Result<(TokenMetadata, Credential)> {
        let mut tokens = self.tokens.lock().expect("lock poisoned");
        let mut replacement = tokens.clone();
        let secret = replacement.create(id.to_owned(), subject, expires_at)?;
        self.persist_tokens(&replacement)?;
        *tokens = replacement;
        let metadata = tokens
            .metadata()
            .into_iter()
            .find(|record| record.id == id)
            .ok_or_else(|| anyhow::anyhow!("created token record is unavailable"))?;
        Ok((metadata, secret))
    }

    pub(crate) fn list_tokens(&self) -> Vec<TokenMetadata> {
        self.tokens.lock().expect("lock poisoned").metadata()
    }

    #[cfg(test)]
    pub(crate) fn create_attestation(
        &self,
        name: String,
        actor: Principal,
        subject: PrincipalId,
        verified_groups: Vec<String>,
        expires_at: Option<SystemTime>,
    ) -> anyhow::Result<(AttestationMetadata, Credential)> {
        self.create_attestation_with_frontend(
            name,
            actor,
            None,
            None,
            subject,
            verified_groups,
            expires_at,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "attestation creation keeps the delegated and front-end identities explicit"
    )]
    pub(crate) fn create_attestation_with_frontend(
        &self,
        name: String,
        actor: Principal,
        frontend_actor: Option<SessionPrincipal>,
        frontend_credential_id: Option<String>,
        subject: PrincipalId,
        verified_groups: Vec<String>,
        expires_at: Option<SystemTime>,
    ) -> anyhow::Result<(AttestationMetadata, Credential)> {
        anyhow::ensure!(
            !name.trim().is_empty(),
            "attestation name must not be empty"
        );
        anyhow::ensure!(
            expires_at.is_none_or(|expiry| expiry > SystemTime::now()),
            "attestation expiry must be in the future"
        );
        let verified_principal =
            SessionPrincipal::new(subject.authority(), subject.subject(), verified_groups)?;
        let verified_groups = verified_principal
            .groups()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let mut attestations = self.attestations.lock().expect("lock poisoned");
        let now = SystemTime::now();
        attestations.retain(|_, record| record.expires_at.is_none_or(|expiry| expiry > now));
        anyhow::ensure!(
            !attestations.contains_key(&name),
            "attestation name already exists"
        );
        if attestations.len() >= MAX_ATTESTATIONS {
            return Err(EphemeralCapacityError::GlobalAttestations.into());
        }
        if attestations
            .values()
            .filter(|record| record.actor.rate_limit_key() == actor.rate_limit_key())
            .count()
            >= MAX_ATTESTATIONS_PER_ACTOR
        {
            return Err(EphemeralCapacityError::ActorAttestations.into());
        }
        let secret = generate_token()?;
        let credential_id = format!("attestation:{name}");
        attestations.insert(
            name.clone(),
            AttestationRecord {
                actor,
                frontend_actor,
                frontend_credential_id,
                subject: subject.clone(),
                verified_groups: verified_groups.clone(),
                credential_id: credential_id.clone(),
                hash: token_hash(secret.expose()),
                expires_at,
            },
        );
        Ok((
            AttestationMetadata {
                name,
                subject,
                verified_groups,
                credential_id,
                expires_at,
            },
            secret,
        ))
    }

    pub(crate) fn list_attestations(&self, actor: &Principal) -> Vec<AttestationMetadata> {
        let mut attestations = self.attestations.lock().expect("lock poisoned");
        let now = SystemTime::now();
        attestations.retain(|_, record| record.expires_at.is_none_or(|expiry| expiry > now));
        let mut records = attestations
            .iter()
            .filter(|(_, record)| record.actor == *actor)
            .map(|(name, record)| AttestationMetadata {
                name: name.clone(),
                subject: record.subject.clone(),
                verified_groups: record.verified_groups.clone(),
                credential_id: record.credential_id.clone(),
                expires_at: record.expires_at,
            })
            .collect::<Vec<_>>();
        records.sort_by(|left, right| left.name.cmp(&right.name));
        records
    }

    pub(crate) fn revoke_attestation(&self, name: &str, actor: &Principal) -> bool {
        let mut attestations = self.attestations.lock().expect("lock poisoned");
        if attestations
            .get(name)
            .is_none_or(|record| record.actor != *actor)
        {
            return false;
        }
        attestations.remove(name).is_some()
    }

    pub(crate) fn revoke_token(&self, id: &str) -> anyhow::Result<bool> {
        let mut tokens = self.tokens.lock().expect("lock poisoned");
        let mut replacement = tokens.clone();
        let changed = replacement.revoke(id);
        if !changed {
            return Ok(false);
        }
        self.persist_tokens(&replacement)?;
        *tokens = replacement;
        drop(tokens);
        if let Some(sessions) = self.token_sessions.lock().expect("lock poisoned").get(id) {
            let _ = sessions.send(());
        }
        Ok(true)
    }

    pub(crate) fn rotate_token(
        &self,
        id: &str,
        expires_at: Option<SystemTime>,
    ) -> anyhow::Result<(TokenMetadata, Credential)> {
        let mut tokens = self.tokens.lock().expect("lock poisoned");
        let mut replacement = tokens.clone();
        let secret = replacement.rotate(id, expires_at)?;
        self.persist_tokens(&replacement)?;
        *tokens = replacement;
        let metadata = tokens
            .metadata()
            .into_iter()
            .find(|record| record.id == id)
            .ok_or_else(|| anyhow::anyhow!("rotated token record is unavailable"))?;
        drop(tokens);
        if let Some(sessions) = self.token_sessions.lock().expect("lock poisoned").get(id) {
            let _ = sessions.send(());
        }
        Ok((metadata, secret))
    }

    fn persist_tokens(&self, tokens: &TokenStore) -> anyhow::Result<()> {
        match self.token_path.as_ref() {
            Some(path) => write_tokens(path, tokens),
            None => Ok(()),
        }
    }
    #[cfg(test)]
    pub(crate) fn mint(
        &self,
        actor: &Principal,
        scope: Option<SessionScope>,
        subject: Option<SessionPrincipal>,
        credential_id: Option<String>,
        authentication_expiry: Option<SystemTime>,
        revocation: Option<broadcast::Receiver<()>>,
    ) -> anyhow::Result<EventTicket> {
        self.mint_with_frontend(
            actor,
            scope,
            subject,
            None,
            credential_id,
            None,
            authentication_expiry,
            revocation,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "event tickets copy the complete authenticated session boundary"
    )]
    pub(crate) fn mint_with_frontend(
        &self,
        actor: &Principal,
        scope: Option<SessionScope>,
        subject: Option<SessionPrincipal>,
        frontend_actor: Option<SessionPrincipal>,
        credential_id: Option<String>,
        frontend_credential_id: Option<String>,
        authentication_expiry: Option<SystemTime>,
        revocation: Option<broadcast::Receiver<()>>,
    ) -> anyhow::Result<EventTicket> {
        let now = Instant::now();
        let mut tickets = self.tickets.lock().expect("lock poisoned");
        tickets.retain(|_, pending| pending.expires_at > now);
        if tickets.len() >= MAX_EVENT_TICKETS {
            return Err(EphemeralCapacityError::GlobalEventTickets.into());
        }
        if tickets
            .values()
            .filter(|pending| pending.actor.rate_limit_key() == actor.rate_limit_key())
            .count()
            >= MAX_EVENT_TICKETS_PER_ACTOR
        {
            return Err(EphemeralCapacityError::ActorEventTickets.into());
        }

        let mut unique_value = None;
        for _ in 0..MAX_TICKET_GENERATION_ATTEMPTS {
            let mut value = [0_u8; EVENT_TICKET_BYTES];
            fill_bytes(&mut value)?;
            if !tickets.contains_key(&value) {
                unique_value = Some(value);
                break;
            }
        }
        let value = unique_value
            .ok_or_else(|| anyhow::anyhow!("could not generate a unique event ticket"))?;
        tickets.insert(
            value,
            PendingTicket {
                actor: actor.clone(),
                expires_at: now + EVENT_TICKET_LIFETIME,
                scope,
                subject,
                frontend_actor,
                credential_id,
                frontend_credential_id,
                authentication_expiry,
                revocation,
            },
        );
        EventTicket::new(value).map_err(Into::into)
    }

    pub(crate) fn prune_expired(&self) {
        let now = Instant::now();
        self.tickets
            .lock()
            .expect("lock poisoned")
            .retain(|_, pending| pending.expires_at > now);
        let now = SystemTime::now();
        self.attestations
            .lock()
            .expect("lock poisoned")
            .retain(|_, record| record.expires_at.is_none_or(|expiry| expiry > now));
    }

    #[cfg(test)]
    pub(crate) fn mint_event_ticket(&self, actor: &Principal) -> EventTicket {
        self.mint(actor, None, None, None, None, None)
            .expect("test event ticket should be minted")
    }

    /// Consumes `ticket` when it is live and belongs to the accepting actor.
    pub(crate) fn consume(
        &self,
        ticket: &EventTicket,
        actor: &Principal,
    ) -> Result<ConsumedTicket, ()> {
        let Ok(value) = <[u8; EVENT_TICKET_BYTES]>::try_from(ticket.expose()) else {
            return Err(());
        };
        let now = Instant::now();
        let mut tickets = self.tickets.lock().expect("lock poisoned");
        tickets.retain(|_, pending| pending.expires_at > now);
        let matches = tickets
            .get(&value)
            .is_some_and(|pending| pending.actor == *actor && pending.expires_at > now);
        if !matches {
            return Err(());
        }
        let pending = tickets.remove(&value).ok_or(())?;
        drop(tickets);
        let revocation = if pending.revocation.is_some() {
            pending.revocation
        } else if let Some(id) = pending.credential_id.as_ref() {
            let tokens = self.tokens.lock().expect("lock poisoned");
            let active = tokens.records.get(id).is_some_and(|record| {
                !record.revoked
                    && record
                        .expires_at
                        .is_none_or(|expiry| expiry > SystemTime::now())
            });
            if !active {
                return Err(());
            }
            let receiver = self
                .token_sessions
                .lock()
                .expect("lock poisoned")
                .get(id)
                .map(broadcast::Sender::subscribe)
                .ok_or(())?;
            Some(receiver)
        } else {
            None
        };
        Ok(ConsumedTicket {
            scope: pending.scope,
            subject: pending.subject,
            frontend_actor: pending.frontend_actor,
            credential_id: pending.credential_id,
            frontend_credential_id: pending.frontend_credential_id,
            revocation,
            authentication_expiry: pending.authentication_expiry,
        })
    }

    /// Authenticates one negotiated connection.
    #[allow(
        clippy::too_many_lines,
        reason = "Keeping all authentication methods in one exhaustive match makes their shared session-metadata and ticket construction auditable."
    )]
    #[cfg(test)]
    pub(crate) async fn authenticate(
        &self,
        request: &AuthenticationRequest,
        actor: &Principal,
    ) -> anyhow::Result<(AuthenticationResponse, Option<broadcast::Receiver<()>>)> {
        let (response, revocation, _frontend_actor, _frontend_credential_id) =
            self.authenticate_with_frontend(request, actor).await?;
        Ok((response, revocation))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one exhaustive match keeps every authentication method's shared session construction auditable"
    )]
    pub(crate) async fn authenticate_with_frontend(
        &self,
        request: &AuthenticationRequest,
        actor: &Principal,
    ) -> anyhow::Result<(
        AuthenticationResponse,
        Option<broadcast::Receiver<()>>,
        Option<SessionPrincipal>,
        Option<String>,
    )> {
        let mut frontend_actor = None;
        let mut frontend_credential_id = None;
        let (subject, verified_groups, source, credential_id, expires_at, revocation) =
            match &request.authentication {
                Authentication::Peer => {
                    let subject = match actor {
                        Principal::Unix { uid, .. } => PrincipalId::new("unix", uid.to_string()),
                        Principal::Windows { sid, .. } => PrincipalId::new("windows", sid.clone()),
                    }?;
                    (
                        subject,
                        Vec::new(),
                        AuthenticationSource::Peer,
                        None,
                        None,
                        None,
                    )
                }
                Authentication::Bearer { credential } => {
                    let tokens = self.tokens.lock().expect("lock poisoned");
                    let Some(token) = tokens.authenticate(credential.expose(), SystemTime::now())
                    else {
                        return Ok((
                            AuthenticationResponse::Rejected {
                                reason: "authentication rejected".to_owned(),
                            },
                            None,
                            None,
                            None,
                        ));
                    };
                    let revocation = self
                        .token_sessions
                        .lock()
                        .expect("lock poisoned")
                        .entry(token.id.clone())
                        .or_insert_with(|| broadcast::channel(1).0)
                        .subscribe();
                    (
                        token.subject.clone(),
                        Vec::new(),
                        AuthenticationSource::Bearer,
                        Some(token.id.clone()),
                        token.expires_at,
                        Some(revocation),
                    )
                }
                Authentication::Attestation { name, credential } => {
                    let mut attestations = self.attestations.lock().expect("lock poisoned");
                    let valid = attestations.get(name).is_some_and(|attestation| {
                        attestation.actor == *actor
                            && attestation
                                .expires_at
                                .is_none_or(|expiry| expiry > SystemTime::now())
                            && attestation
                                .frontend_credential_id
                                .as_ref()
                                .is_none_or(|id| {
                                    self.tokens
                                        .lock()
                                        .expect("lock poisoned")
                                        .records
                                        .get(id)
                                        .is_some_and(|record| {
                                            !record.revoked
                                                && record
                                                    .expires_at
                                                    .is_none_or(|expiry| expiry > SystemTime::now())
                                        })
                                })
                            && bool::from(attestation.hash.ct_eq(&token_hash(credential.expose())))
                    });
                    if !valid {
                        return Ok((
                            AuthenticationResponse::Rejected {
                                reason: "authentication rejected".to_owned(),
                            },
                            None,
                            None,
                            None,
                        ));
                    }
                    let Some(attestation) = attestations.remove(name) else {
                        return Ok((
                            AuthenticationResponse::Rejected {
                                reason: "authentication rejected".to_owned(),
                            },
                            None,
                            None,
                            None,
                        ));
                    };
                    frontend_actor = attestation.frontend_actor;
                    frontend_credential_id = attestation.frontend_credential_id.clone();
                    let revocation = attestation.frontend_credential_id.as_ref().and_then(|id| {
                        self.token_sessions
                            .lock()
                            .expect("lock poisoned")
                            .get(id)
                            .map(broadcast::Sender::subscribe)
                    });
                    (
                        attestation.subject.clone(),
                        attestation.verified_groups,
                        AuthenticationSource::Attestation { name: name.clone() },
                        Some(attestation.credential_id.clone()),
                        attestation.expires_at,
                        revocation,
                    )
                }
                Authentication::External {
                    provider,
                    credential,
                } => {
                    let configured = self
                        .providers
                        .lock()
                        .expect("lock poisoned")
                        .get(provider)
                        .cloned();
                    let Some(configured) = configured else {
                        return Ok((
                            AuthenticationResponse::Rejected {
                                reason: "authentication method is not configured".to_owned(),
                            },
                            None,
                            None,
                            None,
                        ));
                    };
                    let Some(session_permit) = configured.acquire_session() else {
                        return Ok((
                            AuthenticationResponse::Rejected {
                                reason: "authentication provider session capacity reached"
                                    .to_owned(),
                            },
                            None,
                            None,
                            None,
                        ));
                    };
                    let Some(identity) = configured.authenticate(credential.clone()).await? else {
                        return Ok((
                            AuthenticationResponse::Rejected {
                                reason: "authentication rejected".to_owned(),
                            },
                            None,
                            None,
                            None,
                        ));
                    };
                    let subject = PrincipalId::new(
                        configured.authority().to_owned(),
                        identity.subject.clone(),
                    )?;
                    let (revocation_sender, revocation) = broadcast::channel(1);
                    spawn_provider_lease(
                        Arc::clone(&configured),
                        identity.clone(),
                        revocation_sender,
                        session_permit,
                    );
                    (
                        subject,
                        identity.verified_groups,
                        AuthenticationSource::External {
                            provider: provider.clone(),
                        },
                        Some(identity.credential_id),
                        None,
                        Some(revocation),
                    )
                }
            };
        Ok((
            AuthenticationResponse::Authenticated {
                session: SessionMetadata {
                    subject,
                    verified_groups,
                    source,
                    credential_id,
                    expires_at,
                },
            },
            revocation,
            frontend_actor,
            frontend_credential_id,
        ))
    }
}

async fn start_provider(
    configuration: &AuthenticationProviderConfig,
) -> anyhow::Result<AuthenticationProvider> {
    let file = open_existing_file_without_following_symlinks(&configuration.initialization_file)
        .with_context(|| {
            format!(
                "open initialization file {}",
                configuration.initialization_file.display()
            )
        })?;
    let limit = u64::try_from(MAX_PROVIDER_INITIALIZATION_BYTES)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut initialization = Vec::new();
    file.take(limit).read_to_end(&mut initialization)?;
    anyhow::ensure!(
        initialization.len() <= MAX_PROVIDER_INITIALIZATION_BYTES,
        "provider initialization file exceeds the {MAX_PROVIDER_INITIALIZATION_BYTES} byte limit"
    );
    let initialization = Credential::new(initialization)?;
    AuthenticationProvider::start_configured(
        configuration.name.clone(),
        configuration.authority.clone(),
        configuration.executable.clone(),
        initialization,
        configuration.max_sessions,
    )
    .await
}

fn spawn_provider_lease(
    provider: Arc<AuthenticationProvider>,
    mut identity: luminate_protocol::ProviderIdentity,
    revocation: broadcast::Sender<()>,
    session_permit: OwnedSemaphorePermit,
) {
    tokio::spawn(async move {
        let _session_permit = session_permit;
        loop {
            let now = SystemTime::now();
            let Ok(remaining) = identity.expires_at.duration_since(now) else {
                break;
            };
            if revocation.receiver_count() == 0 {
                return;
            }
            sleep(remaining / 2).await;
            if revocation.receiver_count() == 0 {
                return;
            }
            let Ok(Some(next)) = provider.revalidate(identity.continuation.clone()).await else {
                break;
            };
            if next.subject != identity.subject
                || next.verified_groups != identity.verified_groups
                || next.credential_id != identity.credential_id
                || next.expires_at <= SystemTime::now()
            {
                break;
            }
            identity = next;
        }
        tracing::warn!(
            provider = provider.name(),
            "external authentication session was revoked"
        );
        let _ = revocation.send(());
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use luminate_platform::test_support::TestDir;

    fn actor() -> Principal {
        Principal::Unix {
            uid: 1000,
            gid: 1000,
            pid: Some(42),
        }
    }

    #[test]
    fn event_tickets_are_actor_bound_and_one_use() {
        let registry = AuthenticationService::default();
        let ticket = registry
            .mint(&actor(), None, None, None, None, None)
            .expect("mint ticket");
        let other = Principal::Unix {
            uid: 1001,
            gid: 1001,
            pid: Some(43),
        };

        assert!(registry.consume(&ticket, &other).is_err());
        let consumed = registry.consume(&ticket, &actor()).expect("consume ticket");
        assert!(consumed.scope.is_none());
        assert!(consumed.subject.is_none());
        assert!(consumed.credential_id.is_none());
        assert!(consumed.frontend_credential_id.is_none());
        assert!(consumed.revocation.is_none());
        assert!(consumed.authentication_expiry.is_none());
        assert!(registry.consume(&ticket, &actor()).is_err());
    }

    #[test]
    fn event_tickets_preserve_delegated_credential_attribution() {
        let registry = AuthenticationService::default();
        let subject = SessionPrincipal::new("http", "alice", ["operators".to_owned()])
            .expect("delegated subject");
        let frontend = SessionPrincipal::new("http", "frontend", Vec::<String>::new())
            .expect("front-end subject");
        let (_revocation_sender, revocation) = broadcast::channel(1);
        let ticket = registry
            .mint_with_frontend(
                &actor(),
                None,
                Some(subject.clone()),
                Some(frontend.clone()),
                Some("attestation:http-1".to_owned()),
                Some("frontend-token".to_owned()),
                None,
                Some(revocation),
            )
            .expect("mint delegated ticket");

        let consumed = registry.consume(&ticket, &actor()).expect("consume ticket");
        assert_eq!(consumed.subject, Some(subject));
        assert_eq!(consumed.frontend_actor, Some(frontend));
        assert_eq!(
            consumed.credential_id.as_deref(),
            Some("attestation:http-1")
        );
        assert_eq!(
            consumed.frontend_credential_id.as_deref(),
            Some("frontend-token")
        );
    }

    #[tokio::test]
    async fn event_ticket_carries_session_revocation() {
        let service = AuthenticationService::default();
        let (sender, receiver) = broadcast::channel(1);
        let ticket = service
            .mint(&actor(), None, None, None, None, Some(receiver))
            .expect("mint revocable ticket");
        let mut consumed = service
            .consume(&ticket, &actor())
            .expect("consume revocable ticket")
            .revocation
            .expect("revocation receiver");

        sender.send(()).expect("send revocation");
        consumed.recv().await.expect("receive revocation");
    }

    #[test]
    fn event_ticket_capacity_is_bounded_and_actor_isolated() {
        let service = AuthenticationService::default();
        let first_actor = actor();
        for _ in 0..MAX_EVENT_TICKETS_PER_ACTOR {
            service
                .mint(&first_actor, None, None, None, None, None)
                .expect("mint within actor capacity");
        }
        assert!(
            service
                .mint(&first_actor, None, None, None, None, None)
                .is_err()
        );

        let other = Principal::Unix {
            uid: 1001,
            gid: 1001,
            pid: Some(43),
        };
        assert!(service.mint(&other, None, None, None, None, None).is_ok());
        assert!(service.tickets.lock().expect("lock poisoned").len() <= MAX_EVENT_TICKETS);
    }

    #[tokio::test]
    async fn authentication_does_not_eagerly_mint_an_event_ticket() {
        let service = AuthenticationService::default();
        let request = AuthenticationRequest {
            authentication: Authentication::Peer,
            scope: None,
        };
        let (response, _) = service
            .authenticate(&request, &actor())
            .await
            .expect("authenticate peer");

        assert!(matches!(
            response,
            AuthenticationResponse::Authenticated { .. }
        ));
        assert!(service.tickets.lock().expect("lock poisoned").is_empty());
    }

    #[tokio::test]
    async fn attestations_are_actor_bound_and_consumed_once() {
        let service = AuthenticationService::default();
        let subject = PrincipalId::new("unix", "2000").expect("subject");
        let (_metadata, secret) = service
            .create_attestation(
                "dbus-1".to_owned(),
                actor(),
                subject.clone(),
                vec!["20".to_owned(), "10".to_owned(), "20".to_owned()],
                None,
            )
            .expect("create attestation");
        let listed = service.list_attestations(&actor());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].verified_groups, ["10", "20"]);

        let request = AuthenticationRequest {
            authentication: Authentication::Attestation {
                name: "dbus-1".to_owned(),
                credential: secret.clone(),
            },
            scope: None,
        };
        let other = Principal::Unix {
            uid: 1001,
            gid: 1001,
            pid: Some(43),
        };
        let (rejected, _) = service
            .authenticate(&request, &other)
            .await
            .expect("reject other actor");
        assert!(matches!(rejected, AuthenticationResponse::Rejected { .. }));

        let (accepted, _) = service
            .authenticate(&request, &actor())
            .await
            .expect("accept attestation");
        assert!(matches!(
            accepted,
            AuthenticationResponse::Authenticated { session }
                if session.subject == subject && session.verified_groups == ["10", "20"]
        ));
        let (reused, _) = service
            .authenticate(&request, &actor())
            .await
            .expect("reject reused attestation");
        assert!(matches!(reused, AuthenticationResponse::Rejected { .. }));
    }

    #[tokio::test]
    async fn attestation_preserves_frontend_ceiling_and_revocation() {
        let service = AuthenticationService::default();
        let frontend_id = PrincipalId::new("http", "frontend").expect("front-end ID");
        let frontend = SessionPrincipal::new("http", "frontend", Vec::<String>::new())
            .expect("front-end principal");
        let frontend_secret = service
            .create_token("frontend-token", frontend_id, None)
            .expect("create front-end token")
            .1;
        let bearer_request = AuthenticationRequest {
            authentication: Authentication::Bearer {
                credential: frontend_secret,
            },
            scope: None,
        };
        let (_response, _revocation) = service
            .authenticate(&bearer_request, &actor())
            .await
            .expect("authenticate front end");

        let delegated_id = PrincipalId::new("http", "delegated").expect("delegated ID");
        let (_metadata, secret) = service
            .create_attestation_with_frontend(
                "http-delegation".to_owned(),
                actor(),
                Some(frontend.clone()),
                Some("frontend-token".to_owned()),
                delegated_id,
                Vec::new(),
                None,
            )
            .expect("create delegated attestation");
        let request = AuthenticationRequest {
            authentication: Authentication::Attestation {
                name: "http-delegation".to_owned(),
                credential: secret,
            },
            scope: None,
        };
        let (_response, mut revocation, returned_frontend, returned_frontend_credential_id) =
            service
                .authenticate_with_frontend(&request, &actor())
                .await
                .expect("authenticate delegation");
        assert_eq!(
            returned_frontend_credential_id.as_deref(),
            Some("frontend-token")
        );
        assert_eq!(returned_frontend, Some(frontend));

        service
            .revoke_token("frontend-token")
            .expect("revoke front-end token");
        revocation
            .as_mut()
            .expect("delegated revocation receiver")
            .recv()
            .await
            .expect("receive front-end revocation");
    }

    #[test]
    fn bearer_tokens_are_random_hashed_expiring_and_revocable() {
        let mut tokens = TokenStore::default();
        let subject = PrincipalId::new("local", "operator").expect("valid principal");
        let secret = tokens
            .create("desk".to_owned(), subject.clone(), None)
            .expect("create token");

        let record = tokens
            .authenticate(secret.expose(), SystemTime::now())
            .expect("authenticate token");
        assert_eq!(record.subject, subject);
        assert_ne!(record.hash.as_slice(), secret.expose());
        assert!(tokens.authenticate(b"wrong", SystemTime::now()).is_none());
        assert!(tokens.revoke("desk"));
        assert!(
            tokens
                .authenticate(secret.expose(), SystemTime::now())
                .is_none()
        );
    }

    #[tokio::test]
    async fn persisted_token_is_secret_free_and_revocation_notifies_active_session() {
        let directory = TestDir::new("token");
        let path = directory.join("tokens.json");
        let service = AuthenticationService::load(path.clone()).expect("load token service");
        let subject = PrincipalId::new("local", "alice").expect("valid principal");
        let (_metadata, secret) = service
            .create_token("front-end", subject.clone(), None)
            .expect("create token");
        let persisted = fs::read(&path).expect("read token file");
        assert!(
            !persisted
                .windows(secret.expose().len())
                .any(|window| window == secret.expose())
        );

        let reloaded = AuthenticationService::load(path.clone()).expect("reload token service");
        let request = AuthenticationRequest {
            authentication: Authentication::Bearer {
                credential: secret.clone(),
            },
            scope: None,
        };
        let (response, mut revocation) = reloaded
            .authenticate(&request, &actor())
            .await
            .expect("authenticate token");
        assert!(matches!(
            response,
            AuthenticationResponse::Authenticated { session }
                if session.subject == subject && session.credential_id.as_deref() == Some("front-end")
        ));
        reloaded.revoke_token("front-end").expect("revoke token");
        revocation
            .as_mut()
            .expect("token session revocation receiver")
            .recv()
            .await
            .expect("receive revocation");

        let (_metadata, rotated) = reloaded
            .rotate_token("front-end", None)
            .expect("rotate token");
        let (old_response, _) = reloaded
            .authenticate(&request, &actor())
            .await
            .expect("authenticate old secret");
        assert!(matches!(
            old_response,
            AuthenticationResponse::Rejected { .. }
        ));
        let rotated_request = AuthenticationRequest {
            authentication: Authentication::Bearer {
                credential: rotated,
            },
            scope: None,
        };
        let (rotated_response, _) = reloaded
            .authenticate(&rotated_request, &actor())
            .await
            .expect("authenticate rotated secret");
        assert!(matches!(
            rotated_response,
            AuthenticationResponse::Authenticated { .. }
        ));
    }

    #[test]
    fn expired_bearer_tokens_are_rejected() {
        let mut tokens = TokenStore::default();
        let secret = tokens
            .create(
                "old".to_owned(),
                PrincipalId::new("local", "old-user").expect("valid principal"),
                Some(SystemTime::UNIX_EPOCH),
            )
            .expect("create token");

        assert!(
            tokens
                .authenticate(secret.expose(), SystemTime::now())
                .is_none()
        );
    }

    #[test]
    fn token_administration_rejects_invalid_and_repeated_mutations() {
        let mut tokens = TokenStore::default();
        let subject = PrincipalId::new("local", "operator").expect("valid principal");

        assert!(
            tokens
                .create("  ".to_owned(), subject.clone(), None)
                .is_err()
        );
        tokens
            .create("desk".to_owned(), subject.clone(), None)
            .expect("create token");
        assert!(tokens.create("desk".to_owned(), subject, None).is_err());
        assert!(!tokens.revoke("missing"));
        assert!(tokens.revoke("desk"));
        assert!(!tokens.revoke("desk"));
        assert!(tokens.rotate("missing", None).is_err());
    }

    #[test]
    fn attestations_reject_invalid_metadata_and_are_actor_scoped() {
        let service = AuthenticationService::default();
        let subject = PrincipalId::new("local", "alice").expect("valid principal");
        assert!(
            service
                .create_attestation(String::new(), actor(), subject.clone(), Vec::new(), None)
                .is_err()
        );
        assert!(
            service
                .create_attestation(
                    "expired".to_owned(),
                    actor(),
                    subject.clone(),
                    Vec::new(),
                    Some(SystemTime::UNIX_EPOCH)
                )
                .is_err()
        );

        service
            .create_attestation(
                "web".to_owned(),
                actor(),
                subject,
                vec!["operators".to_owned()],
                None,
            )
            .expect("create attestation");
        assert!(
            service
                .create_attestation(
                    "web".to_owned(),
                    actor(),
                    PrincipalId::new("local", "bob").expect("valid principal"),
                    Vec::new(),
                    None
                )
                .is_err()
        );
        let other = Principal::Unix {
            uid: 2000,
            gid: 2000,
            pid: None,
        };
        assert!(service.list_attestations(&other).is_empty());
        assert!(!service.revoke_attestation("web", &other));
        assert!(service.revoke_attestation("web", &actor()));
        assert!(!service.revoke_attestation("web", &actor()));
    }

    #[test]
    fn expired_attestations_are_absent_and_names_are_reusable() {
        let service = AuthenticationService::default();
        let subject = PrincipalId::new("local", "alice").expect("valid principal");
        service
            .create_attestation(
                "short-lived".to_owned(),
                actor(),
                subject.clone(),
                Vec::new(),
                Some(SystemTime::now() + Duration::from_secs(30)),
            )
            .expect("create attestation");
        service
            .attestations
            .lock()
            .expect("lock poisoned")
            .get_mut("short-lived")
            .expect("attestation")
            .expires_at = Some(SystemTime::UNIX_EPOCH);

        assert!(service.list_attestations(&actor()).is_empty());
        assert!(
            service
                .create_attestation("short-lived".to_owned(), actor(), subject, Vec::new(), None,)
                .is_ok()
        );
    }
}
