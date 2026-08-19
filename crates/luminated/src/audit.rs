// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Private daemon authorization audit persistence.

use std::fs::File;
use std::future::Future;
use std::io::Write as _;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use luminate_core::policy::Principal as SessionPrincipal;
use luminate_platform::secure_storage::{
    ensure_service_directory, open_or_create_private_file_for_append,
};
use serde::Serialize;

use crate::authorization::{AuthorizationPolicy, Decision, Operation, Principal, Resource};

/// One daemon authorization decision suitable for local audit storage.
#[derive(Debug, Serialize)]
struct Record {
    /// Unix timestamp at which the daemon made the decision.
    pub(crate) timestamp_unix_ms: u128,
    /// Kernel-authenticated actor.
    actor: Actor,
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<SessionIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frontend_actor: Option<SessionIdentity>,
    /// Operation evaluated.
    operation: &'static str,
    /// Resources considered by the policy.
    resources: Vec<ResourceSummary>,
    /// Whether the request was admitted.
    pub(crate) allowed: bool,
    /// Safe policy diagnostic, when one was returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct SessionIdentity {
    authority: String,
    subject: String,
    verified_groups: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    credential_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "platform", rename_all = "kebab-case")]
enum Actor {
    Unix {
        uid: u32,
        gid: u32,
        pid: Option<u32>,
    },
    Windows {
        sid: String,
        pid: Option<u32>,
    },
}

#[derive(Debug, Serialize)]
struct ResourceSummary {
    device_id: String,
    provider: String,
    host_attached: bool,
    collections: Vec<String>,
}

/// Destination for daemon authorization decisions.
pub(crate) trait Sink: Send + Sync {
    /// Records one decision. Implementations must not persist credentials.
    fn record(
        &self,
        actor: &Principal,
        operation: Operation,
        resources: &[Resource],
        allowed: bool,
        reason: Option<&str>,
    ) -> Result<()>;

    /// Records a decision with its authenticated and delegated identities.
    #[allow(
        clippy::too_many_arguments,
        reason = "audit records keep actor, subject, decision, and resource fields explicit"
    )]
    fn record_session(
        &self,
        actor: &Principal,
        subject: Option<&SessionPrincipal>,
        frontend_actor: Option<&SessionPrincipal>,
        subject_credential_id: Option<&str>,
        frontend_credential_id: Option<&str>,
        operation: Operation,
        resources: &[Resource],
        allowed: bool,
        reason: Option<&str>,
    ) -> Result<()> {
        let _ = (
            subject,
            frontend_actor,
            subject_credential_id,
            frontend_credential_id,
        );
        self.record(actor, operation, resources, allowed, reason)
    }
}

/// Decorates an authorization policy with best-effort durable auditing.
pub(crate) struct AuditedPolicy {
    inner: Arc<dyn AuthorizationPolicy>,
    sink: Arc<dyn Sink>,
}

impl AuditedPolicy {
    pub(crate) fn new(inner: Arc<dyn AuthorizationPolicy>, sink: Arc<dyn Sink>) -> Self {
        Self { inner, sink }
    }
}

impl AuthorizationPolicy for AuditedPolicy {
    fn authorize<'a>(
        &'a self,
        principal: &'a Principal,
        operation: Operation,
        resources: &'a [Resource],
    ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        Box::pin(async move {
            let decision = self.inner.authorize(principal, operation, resources).await;
            let (allowed, reason) = match &decision {
                Decision::Allow => (true, None),
                Decision::Deny { reason } => (false, reason.as_deref()),
            };
            let subject = self.inner.session_principal();
            let frontend_actor = self.inner.frontend_principal();
            let subject_credential_id = self.inner.session_credential_id();
            let frontend_credential_id = self.inner.frontend_credential_id();
            if let Err(error) = self.sink.record_session(
                principal,
                subject.as_ref(),
                frontend_actor.as_ref(),
                subject_credential_id.as_deref(),
                frontend_credential_id.as_deref(),
                operation,
                resources,
                allowed,
                reason,
            ) {
                tracing::warn!(error = %error, "failed to persist authorization audit record");
            }
            decision
        })
    }

    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn session_principal(&self) -> Option<SessionPrincipal> {
        self.inner.session_principal()
    }

    fn session_credential_id(&self) -> Option<String> {
        self.inner.session_credential_id()
    }

    fn frontend_principal(&self) -> Option<SessionPrincipal> {
        self.inner.frontend_principal()
    }

    fn frontend_credential_id(&self) -> Option<String> {
        self.inner.frontend_credential_id()
    }
}

/// Audit destination used when the audit file cannot be opened.
#[derive(Debug, Default)]
#[cfg(test)]
pub(crate) struct NullSink;

#[cfg(test)]
impl Sink for NullSink {
    fn record(
        &self,
        _actor: &Principal,
        _operation: Operation,
        _resources: &[Resource],
        _allowed: bool,
        _reason: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct UnavailableSink(String);

impl UnavailableSink {
    pub(crate) fn new(error: &impl ToString) -> Self {
        Self(error.to_string())
    }
}

impl Sink for UnavailableSink {
    fn record(
        &self,
        _actor: &Principal,
        _operation: Operation,
        _resources: &[Resource],
        _allowed: bool,
        _reason: Option<&str>,
    ) -> Result<()> {
        anyhow::bail!(self.0.clone())
    }
}

/// Append-only JSONL audit file owned by the daemon.
#[derive(Debug)]
pub(crate) struct JsonLinesSink {
    file: Mutex<File>,
}

impl JsonLinesSink {
    /// Opens or creates a private audit file.
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        ensure_service_directory(parent)
            .with_context(|| format!("failed to secure audit directory {}", parent.display()))?;
        let file = open_or_create_private_file_for_append(path)
            .with_context(|| format!("failed to open audit log {}", path.display()))?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }
}

impl Sink for JsonLinesSink {
    fn record(
        &self,
        actor: &Principal,
        operation: Operation,
        resources: &[Resource],
        allowed: bool,
        reason: Option<&str>,
    ) -> Result<()> {
        self.record_session(
            actor, None, None, None, None, operation, resources, allowed, reason,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "audit records keep actor, subject, decision, and resource fields explicit"
    )]
    fn record_session(
        &self,
        actor: &Principal,
        subject: Option<&SessionPrincipal>,
        frontend_actor: Option<&SessionPrincipal>,
        subject_credential_id: Option<&str>,
        frontend_credential_id: Option<&str>,
        operation: Operation,
        resources: &[Resource],
        allowed: bool,
        reason: Option<&str>,
    ) -> Result<()> {
        let timestamp_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock predates Unix epoch")?
            .as_millis();
        let mut payload = serde_json::to_vec(&Record {
            timestamp_unix_ms,
            actor: Actor::from(actor),
            subject: subject
                .map(|principal| SessionIdentity::new(principal, subject_credential_id)),
            frontend_actor: frontend_actor
                .map(|principal| SessionIdentity::new(principal, frontend_credential_id)),
            operation: operation_name(operation),
            resources: resources.iter().map(ResourceSummary::from).collect(),
            allowed,
            reason: reason.map(str::to_owned),
        })?;
        payload.push(b'\n');
        let mut file = self.file.lock().expect("audit sink lock poisoned");
        file.write_all(&payload)?;
        file.sync_data()?;
        Ok(())
    }
}

impl From<&SessionPrincipal> for SessionIdentity {
    fn from(principal: &SessionPrincipal) -> Self {
        Self {
            authority: principal.authority().to_owned(),
            subject: principal.subject().to_owned(),
            verified_groups: principal.groups().iter().cloned().collect(),
            credential_id: None,
        }
    }
}

impl SessionIdentity {
    fn new(principal: &SessionPrincipal, credential_id: Option<&str>) -> Self {
        Self {
            credential_id: credential_id.map(str::to_owned),
            ..Self::from(principal)
        }
    }
}

impl From<&Principal> for Actor {
    fn from(principal: &Principal) -> Self {
        match principal {
            Principal::Unix { uid, gid, pid } => Self::Unix {
                uid: *uid,
                gid: *gid,
                pid: *pid,
            },
            Principal::Windows { sid, pid } => Self::Windows {
                sid: sid.clone(),
                pid: *pid,
            },
        }
    }
}

impl From<&Resource> for ResourceSummary {
    fn from(resource: &Resource) -> Self {
        Self {
            device_id: resource.device_id.to_string(),
            provider: resource.provider_instance.clone().unwrap_or_default(),
            host_attached: resource.host_attached,
            collections: resource
                .collections
                .iter()
                .map(|id| id.as_str().to_owned())
                .collect(),
        }
    }
}

fn operation_name(operation: Operation) -> &'static str {
    match operation {
        Operation::Observe => "observe",
        Operation::Refresh => "refresh",
        Operation::Control => "control",
        Operation::HardwareAdministration => "hardware-administration",
        Operation::DaemonAdministration => "daemon-administration",
        Operation::ManagePlugins => "manage-plugins",
        Operation::ManagePolicy => "manage-policy",
        Operation::ManageAuthentication => "manage-authentication",
        Operation::AdministerFrontend => "administer-frontend",
        Operation::CreateCollection => "create-collection",
        Operation::DestroyCollection => "destroy-collection",
        Operation::ModifyCollection => "modify-collection",
        Operation::AdministerCollections => "administer-collections",
        Operation::CreateScene => "create-scene",
        Operation::ModifyScene => "modify-scene",
        Operation::DestroyScene => "destroy-scene",
        Operation::AdministerScenes => "administer-scenes",
    }
}

#[cfg(test)]
mod tests {
    use luminate_core::device::DeviceId;
    use luminate_core::policy::Principal as SessionPrincipal;
    use std::collections::BTreeSet;
    use std::env::temp_dir;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    use uuid::Uuid;

    use super::{JsonLinesSink, Sink as _};
    use crate::authorization::{Operation, Principal, Resource};

    #[test]
    fn audit_records_are_newline_delimited_and_do_not_contain_secrets() {
        let directory = temp_dir().join(format!("luminate-audit-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).expect("create audit directory");
        #[cfg(unix)]
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .expect("make audit directory private");
        let path = directory.join("audit.jsonl");
        let sink = JsonLinesSink::open(&path).expect("open audit sink");
        let actor = Principal::Unix {
            uid: 1000,
            gid: 1000,
            pid: Some(42),
        };
        let resource = Resource {
            device_id: DeviceId::new("keyboard"),
            provider_instance: Some("provider".to_owned()),
            host_attached: true,
            collections: BTreeSet::new(),
        };
        let subject = SessionPrincipal::new("http", "alice", ["operators".to_owned()])
            .expect("session subject");
        let frontend = SessionPrincipal::new("http", "frontend", Vec::<String>::new())
            .expect("front-end subject");
        sink.record_session(
            &actor,
            Some(&subject),
            Some(&frontend),
            Some("remote-token"),
            Some("frontend-token"),
            Operation::Observe,
            &[resource],
            true,
            None,
        )
        .expect("write audit record");

        let payload = fs::read_to_string(&path).expect("read audit record");
        assert_eq!(payload.lines().count(), 1);
        assert!(payload.contains("keyboard"));
        assert!(payload.contains("alice"));
        assert!(payload.contains("frontend"));
        assert!(payload.contains("remote-token"));
        assert!(payload.contains("frontend-token"));
        assert!(!payload.contains("display-once-secret"));
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path)
                .expect("read audit metadata")
                .permissions()
                .mode()
                & 0o077,
            0,
            "new audit file must not be accessible by group or others"
        );
        drop(sink);
        let _ = fs::remove_dir_all(directory);
    }

    #[cfg(unix)]
    #[test]
    fn opening_an_existing_permissive_audit_file_is_rejected() {
        let directory = temp_dir().join(format!("luminate-audit-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).expect("create audit directory");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .expect("make audit directory private");
        let path = directory.join("audit.jsonl");
        fs::write(&path, "existing\n").expect("create audit file");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640))
            .expect("make audit file group-readable");

        let error = JsonLinesSink::open(&path).expect_err("permissive file must be rejected");
        assert!(error.to_string().contains("failed to open audit log"));
        assert_eq!(
            fs::read_to_string(&path).expect("read untouched audit file"),
            "existing\n"
        );

        let _ = fs::remove_dir_all(directory);
    }

    #[cfg(unix)]
    #[test]
    fn opening_a_symlinked_audit_file_is_rejected_without_writing_its_target() {
        let directory = temp_dir().join(format!("luminate-audit-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).expect("create audit directory");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .expect("make audit directory private");
        let target = directory.join("target.jsonl");
        let path = directory.join("audit.jsonl");
        fs::write(&target, "decoy\n").expect("create symlink target");
        symlink(&target, &path).expect("create audit symlink");

        JsonLinesSink::open(&path).expect_err("symlinked file must be rejected");
        assert_eq!(
            fs::read_to_string(&target).expect("read untouched symlink target"),
            "decoy\n"
        );

        let _ = fs::remove_dir_all(directory);
    }
}
