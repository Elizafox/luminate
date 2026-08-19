// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Supervision for bounded executable authentication providers.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Context as _;
use luminate_protocol::AUTHENTICATION_PROVIDER_PROTOCOL_VERSION;
use luminate_protocol::framing;
use luminate_protocol::{
    Continuation, Credential, ProviderHello, ProviderHelloResponse, ProviderIdentity,
    ProviderRequest, ProviderResponse, validate_identity,
};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::timeout;

const STARTUP_DEADLINE: Duration = Duration::from_secs(5);
const EXCHANGE_DEADLINE: Duration = Duration::from_secs(5);
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(1);

/// One supervised provider process.
pub(crate) struct AuthenticationProvider {
    name: String,
    authority: String,
    executable: PathBuf,
    initialization: Credential,
    exchange: Mutex<ProviderProcess>,
    next_id: AtomicU64,
    sessions: Arc<Semaphore>,
}

struct ProviderProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl AuthenticationProvider {
    /// Spawns and handshakes with a configured provider.
    #[cfg(test)]
    pub(crate) async fn start(
        executable: impl Into<PathBuf>,
        initialization: Credential,
    ) -> anyhow::Result<Self> {
        Self::start_inner(
            "test".to_owned(),
            "test".to_owned(),
            executable.into(),
            initialization,
            64,
        )
        .await
    }

    pub(crate) async fn start_configured(
        name: String,
        authority: String,
        executable: PathBuf,
        initialization: Credential,
        max_sessions: usize,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            executable.is_absolute(),
            "provider executable path must be absolute"
        );
        let metadata = fs::symlink_metadata(&executable)
            .with_context(|| format!("inspect authentication provider {}", executable.display()))?;
        anyhow::ensure!(
            metadata.is_file(),
            "authentication provider executable is not a regular file"
        );
        Self::start_inner(name, authority, executable, initialization, max_sessions).await
    }

    async fn start_inner(
        name: String,
        authority: String,
        executable: PathBuf,
        initialization: Credential,
        max_sessions: usize,
    ) -> anyhow::Result<Self> {
        let process = spawn(&executable, initialization.clone()).await?;
        Ok(Self {
            name,
            authority,
            executable,
            initialization,
            exchange: Mutex::new(process),
            next_id: AtomicU64::new(0),
            sessions: Arc::new(Semaphore::new(max_sessions)),
        })
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn authority(&self) -> &str {
        &self.authority
    }

    pub(crate) fn acquire_session(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.sessions).try_acquire_owned().ok()
    }

    /// Authenticates one opaque credential within the provider deadline.
    pub(crate) async fn authenticate(
        &self,
        credential: Credential,
    ) -> anyhow::Result<Option<ProviderIdentity>> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.call(ProviderRequest::Authenticate { id, credential }, id)
            .await
    }

    /// Revalidates one provider continuation within the provider deadline.
    pub(crate) async fn revalidate(
        &self,
        continuation: Continuation,
    ) -> anyhow::Result<Option<ProviderIdentity>> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.call(ProviderRequest::Revalidate { id, continuation }, id)
            .await
    }

    /// Asks the provider to exit, then forcibly terminates it if it does not
    /// stop within the shutdown deadline.
    pub(crate) async fn shutdown(&self) {
        let mut process = self.exchange.lock().await;
        let graceful = async {
            framing::send(&mut process.stdin, &ProviderRequest::Shutdown).await?;
            process.child.wait().await.map_err(anyhow::Error::from)
        };

        if !matches!(timeout(SHUTDOWN_DEADLINE, graceful).await, Ok(Ok(_))) {
            let _ = process.child.kill().await;
        }
    }

    async fn call(
        &self,
        request: ProviderRequest,
        expected_id: u64,
    ) -> anyhow::Result<Option<ProviderIdentity>> {
        let mut process = self.exchange.lock().await;
        let exchange = async {
            framing::send(&mut process.stdin, &request).await?;
            framing::receive::<ProviderResponse>(&mut process.stdout).await
        };
        let response = match timeout(EXCHANGE_DEADLINE, exchange).await {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                restart(&mut process, &self.executable, self.initialization.clone()).await?;
                return Err(error.into());
            }
            Err(error) => {
                restart(&mut process, &self.executable, self.initialization.clone()).await?;
                return Err(error.into());
            }
        };
        match response {
            ProviderResponse::Authenticated { id, identity } if id == expected_id => {
                validate_identity(&identity).map_err(anyhow::Error::msg)?;
                Ok(Some(identity))
            }
            ProviderResponse::Rejected { id, .. } if id == expected_id => Ok(None),
            ProviderResponse::Authenticated { .. } | ProviderResponse::Rejected { .. } => {
                anyhow::bail!("authentication provider returned a mismatched exchange ID")
            }
        }
    }
}

async fn restart(
    process: &mut ProviderProcess,
    executable: &Path,
    initialization: Credential,
) -> anyhow::Result<()> {
    let _ = process.child.kill().await;
    // A failed exchange causes one bounded restart. Repeated failures remain
    // fail-closed and each call is naturally paced by the startup deadline.
    *process = spawn(executable, initialization).await?;
    Ok(())
}

async fn spawn(executable: &Path, initialization: Credential) -> anyhow::Result<ProviderProcess> {
    let mut child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("authentication provider stdin was not piped"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("authentication provider stdout was not piped"))?;
    let handshake = async {
        framing::send(&mut stdin, &ProviderHello::new(initialization)).await?;
        framing::receive::<ProviderHelloResponse>(&mut stdout).await
    };
    let response = timeout(STARTUP_DEADLINE, handshake)
        .await
        .context("authentication provider handshake timed out")?
        .context("authentication provider handshake failed")?;
    match response {
        ProviderHelloResponse::Ready { protocol_version }
            if protocol_version == AUTHENTICATION_PROVIDER_PROTOCOL_VERSION => {}
        ProviderHelloResponse::Ready { .. } => {
            anyhow::bail!("authentication provider protocol version mismatch");
        }
        ProviderHelloResponse::Rejected { reason } => {
            anyhow::bail!("authentication provider initialization rejected: {reason}");
        }
    }
    Ok(ProviderProcess {
        child,
        stdin,
        stdout,
    })
}

#[cfg(test)]
#[path = "authentication_provider_tests.rs"]
mod tests;
