// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use thiserror::Error;

use super::document::{AuthorizationDecision, PolicyDocument, PolicyRevision};
use super::identity::{Operation, RemotePrincipal, Resource};

/// One request supplied to an [`AccessPolicy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationRequest {
    /// Authenticated principal.
    pub principal: RemotePrincipal,

    /// Semantic operation being attempted.
    pub operation: Operation,

    /// Fully resolved concrete resources.
    pub resources: Vec<Resource>,
}

/// Boxed asynchronous result used by policy and storage traits.
pub type PolicyFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Failure to evaluate or persist policy state.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PolicyError {
    /// The expected policy revision did not match the stored revision.
    #[error("policy revision conflict: expected {expected}, found {actual}", expected = expected.0, actual = actual.0)]
    RevisionConflict {
        /// Revision supplied by the caller.
        expected: PolicyRevision,

        /// Current stored revision.
        actual: PolicyRevision,
    },

    /// A backend was unavailable or rejected an operation.
    #[error("{0}")]
    Unavailable(String),
}

/// Asynchronous, transport-neutral access policy.
pub trait AccessPolicy: Send + Sync {
    /// Evaluates one authorization request.
    fn authorize<'a>(
        &'a self,
        request: &'a AuthorizationRequest,
    ) -> PolicyFuture<'a, Result<AuthorizationDecision, PolicyError>>;

    /// Returns the active policy revision.
    fn revision(&self) -> PolicyRevision;
}

/// Runtime policy whose active document can be administered atomically.
pub trait ManagedAccessPolicy: AccessPolicy {
    /// Returns the active validated policy document.
    fn document(&self) -> PolicyFuture<'_, Result<Arc<PolicyDocument>, PolicyError>>;

    /// Persists and activates a complete replacement document.
    fn replace(
        &self,
        expected: PolicyRevision,
        replacement: PolicyDocument,
    ) -> PolicyFuture<'_, Result<Arc<PolicyDocument>, PolicyError>>;
}

/// Access policy backed by one immutable validated document.
#[derive(Debug, Clone)]
pub struct StaticAccessPolicy {
    document: Arc<PolicyDocument>,
}

impl StaticAccessPolicy {
    /// Creates an immutable policy from a validated document.
    #[must_use]
    pub fn new(document: PolicyDocument) -> Self {
        Self {
            document: Arc::new(document),
        }
    }

    /// Returns the validated policy document.
    #[must_use]
    pub fn document(&self) -> &PolicyDocument {
        &self.document
    }
}

impl AccessPolicy for StaticAccessPolicy {
    fn authorize<'a>(
        &'a self,
        request: &'a AuthorizationRequest,
    ) -> PolicyFuture<'a, Result<AuthorizationDecision, PolicyError>> {
        Box::pin(async move {
            Ok(self
                .document
                .evaluate(&request.principal, request.operation, &request.resources))
        })
    }

    fn revision(&self) -> PolicyRevision {
        self.document.revision()
    }
}

/// Storage-neutral access to a revisioned policy document.
pub trait PolicyStore: Send + Sync {
    /// Loads the current validated policy document.
    fn load(&self) -> PolicyFuture<'_, Result<Arc<PolicyDocument>, PolicyError>>;

    /// Atomically replaces the document when `expected` is current.
    fn replace(
        &self,
        expected: PolicyRevision,
        replacement: PolicyDocument,
    ) -> PolicyFuture<'_, Result<Arc<PolicyDocument>, PolicyError>>;
}

/// Read-only store containing one static policy document.
#[derive(Debug, Clone)]
pub struct StaticPolicyStore {
    document: Arc<PolicyDocument>,
}

impl StaticPolicyStore {
    /// Creates a read-only store.
    #[must_use]
    pub fn new(document: PolicyDocument) -> Self {
        Self {
            document: Arc::new(document),
        }
    }
}

impl PolicyStore for StaticPolicyStore {
    fn load(&self) -> PolicyFuture<'_, Result<Arc<PolicyDocument>, PolicyError>> {
        let document = Arc::clone(&self.document);
        Box::pin(async move { Ok(document) })
    }

    fn replace(
        &self,
        _expected: PolicyRevision,
        _replacement: PolicyDocument,
    ) -> PolicyFuture<'_, Result<Arc<PolicyDocument>, PolicyError>> {
        Box::pin(async {
            Err(PolicyError::Unavailable(
                "static policy store is read-only".to_owned(),
            ))
        })
    }
}

/// Thread-safe in-memory policy store with atomic revision replacement.
#[derive(Debug)]
pub struct InMemoryPolicyStore {
    document: Mutex<Arc<PolicyDocument>>,
}

impl InMemoryPolicyStore {
    /// Creates an in-memory store.
    #[must_use]
    pub fn new(document: PolicyDocument) -> Self {
        Self {
            document: Mutex::new(Arc::new(document)),
        }
    }
}

impl PolicyStore for InMemoryPolicyStore {
    fn load(&self) -> PolicyFuture<'_, Result<Arc<PolicyDocument>, PolicyError>> {
        Box::pin(async {
            Ok(Arc::clone(
                &self.document.lock().expect("policy store lock poisoned"),
            ))
        })
    }

    fn replace(
        &self,
        expected: PolicyRevision,
        replacement: PolicyDocument,
    ) -> PolicyFuture<'_, Result<Arc<PolicyDocument>, PolicyError>> {
        Box::pin(async move {
            let mut document = self.document.lock().expect("policy store lock poisoned");
            let actual = document.revision();
            if actual != expected {
                return Err(PolicyError::RevisionConflict { expected, actual });
            }

            let replacement = Arc::new(replacement);
            *document = Arc::clone(&replacement);
            Ok(replacement)
        })
    }
}

/// Store-backed access policy with immutable policy-administration recovery.
///
/// The recovery document is consulted only for recovery operations.
/// An allow from it overrides the active document, ensuring that runtime
/// replacement cannot remove recovery access. Recovery denials do not affect
/// evaluation by the active document.
pub struct RuntimeAccessPolicy {
    store: Arc<dyn PolicyStore>,
    recovery: PolicyDocument,
    active: Mutex<Arc<PolicyDocument>>,
    replacing: AtomicBool,
}

impl fmt::Debug for RuntimeAccessPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeAccessPolicy")
            .field("revision", &self.active_document().revision())
            .finish_non_exhaustive()
    }
}

impl RuntimeAccessPolicy {
    /// Loads the initial active document from `store`.
    ///
    /// # Errors
    ///
    /// Returns an error when the policy store cannot load its current
    /// document.
    pub async fn load(
        store: Arc<dyn PolicyStore>,
        recovery: PolicyDocument,
    ) -> Result<Self, PolicyError> {
        let active = store.load().await?;
        Ok(Self {
            store,
            recovery,
            active: Mutex::new(active),
            replacing: AtomicBool::new(false),
        })
    }

    fn active_document(&self) -> Arc<PolicyDocument> {
        Arc::clone(&active_document_guard(&self.active))
    }
}

impl AccessPolicy for RuntimeAccessPolicy {
    fn authorize<'a>(
        &'a self,
        request: &'a AuthorizationRequest,
    ) -> PolicyFuture<'a, Result<AuthorizationDecision, PolicyError>> {
        Box::pin(async move {
            let active = self.active_document();
            if matches!(
                request.operation,
                Operation::ManagePolicy
                    | Operation::ManageAuthentication
                    | Operation::AdministerFrontend,
            ) {
                let mut recovery = self.recovery.evaluate(
                    &request.principal,
                    request.operation,
                    &request.resources,
                );
                if recovery.is_allowed() {
                    recovery.revision = active.revision();
                    return Ok(recovery);
                }
            }

            Ok(active.evaluate(&request.principal, request.operation, &request.resources))
        })
    }

    fn revision(&self) -> PolicyRevision {
        self.active_document().revision()
    }
}

impl ManagedAccessPolicy for RuntimeAccessPolicy {
    fn document(&self) -> PolicyFuture<'_, Result<Arc<PolicyDocument>, PolicyError>> {
        let document = self.active_document();
        Box::pin(async move { Ok(document) })
    }

    fn replace(
        &self,
        expected: PolicyRevision,
        replacement: PolicyDocument,
    ) -> PolicyFuture<'_, Result<Arc<PolicyDocument>, PolicyError>> {
        Box::pin(async move {
            let _replacement = ReplacementGuard::acquire(&self.replacing)?;
            let replacement = self.store.replace(expected, replacement).await?;
            *active_document_guard(&self.active) = Arc::clone(&replacement);
            Ok(replacement)
        })
    }
}

struct ReplacementGuard<'a> {
    replacing: &'a AtomicBool,
}

impl<'a> ReplacementGuard<'a> {
    fn acquire(replacing: &'a AtomicBool) -> Result<Self, PolicyError> {
        replacing
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| PolicyError::Unavailable("policy replacement is already active".into()))?;
        Ok(Self { replacing })
    }
}

impl Drop for ReplacementGuard<'_> {
    fn drop(&mut self) {
        self.replacing.store(false, Ordering::Release);
    }
}

fn active_document_guard(
    active: &Mutex<Arc<PolicyDocument>>,
) -> MutexGuard<'_, Arc<PolicyDocument>> {
    active.lock().expect("policy activation lock poisoned")
}
