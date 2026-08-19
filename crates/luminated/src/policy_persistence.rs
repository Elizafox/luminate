// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Crash-safe persistence for the daemon's validated access policy.

#[cfg(test)]
use std::fs;
use std::future::Future;
use std::io::{ErrorKind, Read as _};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
use std::{env, process};

use anyhow::{Context as _, Result};
use luminate_core::policy::{
    PolicyDocument, PolicyDocumentSource, PolicyError, PolicyRevision, PolicyStore,
};
use luminate_platform::secure_storage::{ensure_service_directory, open_private_file_for_read};

use crate::atomic_file;
const MAX_POLICY_FILE_BYTES: usize = 4 * 1024 * 1024;
static TEMPORARY_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A private, revisioned policy document stored beside daemon state.
#[derive(Debug, Clone)]
pub(crate) struct PolicyFileStore {
    path: PathBuf,
    fallback: Option<Arc<PolicyDocument>>,
}

impl PolicyFileStore {
    /// Creates a file-backed policy store at `path`.
    #[must_use]
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            fallback: None,
        }
    }

    pub(crate) fn with_fallback(path: PathBuf, fallback: PolicyDocument) -> Self {
        Self {
            path,
            fallback: Some(Arc::new(fallback)),
        }
    }

    /// Returns the configured policy path.
    #[must_use]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Loads and validates the policy document.
    ///
    /// A missing policy file is reported as `None`; callers choose the
    /// appropriate recovery or first-run document rather than silently
    /// changing the daemon's security posture here.
    pub(crate) fn load_document(&self) -> Result<Option<PolicyDocument>> {
        let file = match open_private_file_for_read(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to open policy file {}", self.path.display())
                });
            }
        };

        let read_limit = u64::try_from(MAX_POLICY_FILE_BYTES)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let mut contents = Vec::new();
        file.take(read_limit)
            .read_to_end(&mut contents)
            .with_context(|| format!("failed to read policy file {}", self.path.display()))?;
        anyhow::ensure!(
            contents.len() <= MAX_POLICY_FILE_BYTES,
            "policy file {} exceeds the {} byte limit",
            self.path.display(),
            MAX_POLICY_FILE_BYTES
        );

        let source: PolicyDocumentSource = serde_json::from_slice(&contents)
            .with_context(|| format!("failed to parse policy file {}", self.path.display()))?;
        PolicyDocument::new(source).map(Some).map_err(|error| {
            anyhow::anyhow!("policy file {} is invalid: {error}", self.path.display())
        })
    }

    /// Replaces the policy if its current revision equals `expected`.
    ///
    /// The validated replacement is written to a private temporary file,
    /// flushed, renamed, and followed by a directory sync. A stale expected
    /// revision is rejected before any filesystem mutation.
    pub(crate) fn replace_document(
        &self,
        expected: PolicyRevision,
        replacement: PolicyDocument,
    ) -> Result<PolicyDocument, PolicyError> {
        let current = self
            .load_document()
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        let actual = current.as_ref().map_or_else(
            || {
                self.fallback
                    .as_ref()
                    .map_or(PolicyRevision(0), |value| value.revision())
            },
            PolicyDocument::revision,
        );
        if actual != expected {
            return Err(PolicyError::RevisionConflict { expected, actual });
        }

        self.write(&replacement)
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        Ok(replacement)
    }

    fn write(&self, document: &PolicyDocument) -> Result<()> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        ensure_service_directory(parent)
            .with_context(|| format!("failed to prepare policy directory {}", parent.display()))?;

        let payload = serde_json::to_vec_pretty(document.source())?;
        anyhow::ensure!(
            payload.len() <= MAX_POLICY_FILE_BYTES,
            "policy document exceeds the {MAX_POLICY_FILE_BYTES} byte limit"
        );
        let sequence = TEMPORARY_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".{}.{}.tmp",
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("policy"),
            sequence
        ));
        atomic_file::replace(&self.path, &temporary, &payload).with_context(|| {
            format!(
                "failed to atomically replace policy file {}",
                self.path.display()
            )
        })?;
        Ok(())
    }
}

impl PolicyStore for PolicyFileStore {
    fn load(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<PolicyDocument>, PolicyError>> + Send + '_>> {
        Box::pin(async move {
            PolicyFileStore::load_document(self)
                .map_err(|error| PolicyError::Unavailable(error.to_string()))?
                .map(Arc::new)
                .or_else(|| self.fallback.clone())
                .ok_or_else(|| {
                    PolicyError::Unavailable("policy has not been initialized".to_owned())
                })
        })
    }

    fn replace(
        &self,
        expected: PolicyRevision,
        replacement: PolicyDocument,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<PolicyDocument>, PolicyError>> + Send + '_>> {
        Box::pin(async move {
            PolicyFileStore::replace_document(self, expected, replacement).map(Arc::new)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use luminate_core::policy::{Binding, Preset, materialize_presets};
    use luminate_platform::secure_storage::create_private_file;
    use std::collections::BTreeSet;
    use std::io::Write as _;

    fn store(name: &str) -> PolicyFileStore {
        let path = env::temp_dir()
            .join(format!(
                "luminate-policy-{name}-{}-{}",
                process::id(),
                TEMPORARY_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ))
            .join("policy.json");
        PolicyFileStore::new(path)
    }

    fn document(revision: u64) -> PolicyDocument {
        let mut source = materialize_presets(PolicyRevision(revision), [Preset::Administrator]);
        source.bindings.push(Binding {
            authority: "unix".to_owned(),
            subjects: BTreeSet::from(["1000".to_owned()]),
            groups: BTreeSet::new(),
            roles: BTreeSet::from(["administrator".to_owned()]),
        });
        PolicyDocument::new(source).expect("test policy is valid")
    }

    #[test]
    fn missing_policy_is_not_silently_created() {
        let store = store("missing");
        assert!(
            store
                .load_document()
                .expect("load missing policy")
                .is_none()
        );
    }

    #[test]
    fn replacement_is_atomic_and_revision_checked() {
        let store = store("replace");
        let first = document(1);
        assert_eq!(
            store
                .replace_document(PolicyRevision(0), first)
                .expect("write first")
                .revision(),
            PolicyRevision(1)
        );
        let stale = store.replace_document(PolicyRevision(0), document(2));
        assert!(matches!(stale, Err(PolicyError::RevisionConflict { .. })));
        assert_eq!(
            store
                .load_document()
                .expect("reload policy")
                .expect("policy exists")
                .revision(),
            PolicyRevision(1)
        );
        let _ = fs::remove_dir_all(store.path().parent().expect("test policy has a parent"));
    }

    #[test]
    fn invalid_policy_is_rejected_on_load() {
        let store = store("invalid");
        fs::create_dir_all(store.path().parent().expect("test policy has a parent"))
            .expect("create test policy directory");
        let mut file = create_private_file(store.path()).expect("create invalid private policy");
        file.write_all(b"{}\n").expect("write invalid policy");
        assert!(store.load_document().is_err());
        let _ = fs::remove_dir_all(store.path().parent().expect("test policy has a parent"));
    }
}
