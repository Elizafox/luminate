// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

use std::sync::Mutex;

use serde::de::Error as _;
use serde::{Deserialize, Serialize};

use super::document::{
    DecisionOutcome, PolicyRevision, RuleId, ValidationError, validate_nonempty,
};
use super::identity::{Operation, RemotePrincipal};
use super::runtime::{PolicyError, PolicyFuture};

/// Authenticated service actor retained alongside a delegated principal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FrontendActor {
    /// Authenticated service principal which requested delegation.
    principal: RemotePrincipal,

    /// Stable identifier of the credential used by the service principal.
    credential_id: String,
}

impl FrontendActor {
    /// Constructs an authenticated front-end actor.
    ///
    /// # Errors
    ///
    /// Returns an error when the credential identifier is empty.
    pub fn new(
        principal: RemotePrincipal,
        credential_id: impl Into<String>,
    ) -> Result<Self, ValidationError> {
        let credential_id = credential_id.into();
        validate_nonempty("front-end actor credential ID", &credential_id)?;
        Ok(Self {
            principal,
            credential_id,
        })
    }

    /// Returns the authenticated service principal.
    #[must_use]
    pub const fn principal(&self) -> &RemotePrincipal {
        &self.principal
    }

    /// Returns the stable credential identifier.
    #[must_use]
    pub fn credential_id(&self) -> &str {
        &self.credential_id
    }
}

impl<'de> Deserialize<'de> for FrontendActor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WireActor {
            principal: RemotePrincipal,
            credential_id: String,
        }

        let wire = WireActor::deserialize(deserializer)?;
        Self::new(wire.principal, wire.credential_id).map_err(D::Error::custom)
    }
}

/// Immutable authorization audit event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    /// Principal whose request was evaluated.
    pub principal: RemotePrincipal,

    /// Authenticated service actor when `principal` was delegated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontend_actor: Option<FrontendActor>,

    /// Semantic operation evaluated.
    pub operation: Operation,

    /// Result of evaluation.
    pub outcome: DecisionOutcome,

    /// Policy revision used.
    pub revision: PolicyRevision,

    /// Matching stable rule identifier, if any.
    pub audit_rule: Option<RuleId>,

    /// Safe diagnostic, if any.
    pub reason: Option<String>,
}

/// Storage-neutral authorization audit destination.
pub trait AuditSink: Send + Sync {
    /// Records one immutable audit event.
    fn record(&self, record: AuditRecord) -> PolicyFuture<'_, Result<(), PolicyError>>;
}

/// Audit destination that intentionally discards records.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullAuditSink;

impl AuditSink for NullAuditSink {
    fn record(&self, _record: AuditRecord) -> PolicyFuture<'_, Result<(), PolicyError>> {
        Box::pin(async { Ok(()) })
    }
}

/// Thread-safe in-memory audit destination.
#[derive(Debug, Default)]
pub struct InMemoryAuditSink {
    records: Mutex<Vec<AuditRecord>>,
}

impl InMemoryAuditSink {
    /// Creates an empty in-memory audit destination.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
        }
    }

    /// Returns a snapshot of recorded events.
    ///
    /// # Errors
    ///
    /// This in-memory implementation currently has no ordinary error path; the
    /// result is retained for compatibility with other audit stores.
    ///
    /// # Panics
    ///
    /// Panics if another thread panicked while holding the store lock.
    pub fn records(&self) -> Result<Vec<AuditRecord>, PolicyError> {
        Ok(self
            .records
            .lock()
            .expect("audit sink lock poisoned")
            .clone())
    }
}

impl AuditSink for InMemoryAuditSink {
    fn record(&self, record: AuditRecord) -> PolicyFuture<'_, Result<(), PolicyError>> {
        Box::pin(async move {
            self.records
                .lock()
                .expect("audit sink lock poisoned")
                .push(record);
            Ok(())
        })
    }
}
