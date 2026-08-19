// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::{
    AssertUnwindSafe, Client, Error, EventSubscription, LuminateStatus, Runtime, create_runtime,
    mpsc, panic, set_last_error, thread,
};
use std::future::Future;
use std::mem;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::task::AbortHandle;

/// A unit of work submitted to a client's asynchronous runtime.
type Job = Box<dyn FnOnce(Arc<Client>) + Send>;
pub(crate) type CompletionJob = Box<dyn FnOnce() + Send>;

/// External handle over a connected client's shared execution domain.
///
/// Clones are internal retained references used by resources which may outlive
/// the public `LuminateClient` handle.
#[derive(Clone)]
pub(crate) struct FfiClient {
    core: Arc<FfiClientCore>,
}

/// Immutable connection metadata copied out of the runtime-owned client.
struct FfiClientMetadata {
    daemon_version: String,
    socket_path: PathBuf,
    event_socket_path: PathBuf,
    session: crate::SessionMetadata,
}

/// Owns a connected client and its asynchronous execution domain.
///
/// The runtime thread accepts jobs and spawns each one as an independent task,
/// allowing the client's request-ID routing to serve concurrent callers.
struct FfiClientCore {
    /// `Some` until dropped; closing it lets the runtime leave its receive loop
    /// and shut down its spawned tasks.
    sender: Option<UnboundedSender<Job>>,
    worker: Option<thread::JoinHandle<()>>,
    completion_sender: Option<mpsc::Sender<CompletionJob>>,
    completion_worker: Option<thread::JoinHandle<()>>,
    metadata: Option<FfiClientMetadata>,
}

#[cfg(test)]
impl FfiClient {
    pub(crate) fn stopped() -> Self {
        Self {
            core: Arc::new(FfiClientCore {
                sender: None,
                worker: None,
                completion_sender: None,
                completion_worker: None,
                metadata: None,
            }),
        }
    }
}

impl FfiClient {
    fn metadata(&self) -> Result<&FfiClientMetadata, LuminateStatus> {
        self.core.metadata.as_ref().ok_or_else(|| {
            set_last_error("libluminate client execution domain is not running");
            LuminateStatus::Internal
        })
    }

    pub(crate) fn daemon_version(&self) -> Result<&str, LuminateStatus> {
        Ok(&self.metadata()?.daemon_version)
    }

    pub(crate) fn socket_path(&self) -> Result<&PathBuf, LuminateStatus> {
        Ok(&self.metadata()?.socket_path)
    }

    pub(crate) fn event_socket_path(&self) -> Result<&PathBuf, LuminateStatus> {
        Ok(&self.metadata()?.event_socket_path)
    }

    pub(crate) fn session(&self) -> Result<&crate::SessionMetadata, LuminateStatus> {
        Ok(&self.metadata()?.session)
    }

    pub(crate) fn dispatch(&self, completion: CompletionJob) -> Result<(), LuminateStatus> {
        let Some(sender) = self.core.completion_sender.as_ref() else {
            set_last_error("libluminate completion dispatcher is not running");
            return Err(LuminateStatus::Internal);
        };
        sender.send(completion).map_err(|_| {
            set_last_error("libluminate completion dispatcher is not running");
            LuminateStatus::Internal
        })
    }
}

/// Owns an event subscription on its originating client's execution domain.
#[derive(Clone)]
pub(crate) struct FfiSubscription {
    pub(super) client: FfiClient,
    pub(super) subscription: Arc<Mutex<EventSubscription>>,
    #[cfg(test)]
    pub(super) started_tx: Option<mpsc::Sender<()>>,
}

impl Drop for FfiClientCore {
    fn drop(&mut self) {
        // Close the channel before joining so an idle worker can exit.
        drop(self.sender.take());
        if let Some(worker) = self.worker.take()
            && worker.thread().id() != thread::current().id()
        {
            let _ = worker.join();
        }
        drop(self.completion_sender.take());
        if let Some(worker) = self.completion_worker.take()
            && worker.thread().id() != thread::current().id()
        {
            let _ = worker.join();
        }
    }
}

/// Spawns the worker thread, builds its runtime, and runs `connect` there.
///
/// This keeps every `Runtime::block_on` call off the caller's thread. The
/// function returns once the worker reports whether the connection succeeded.
pub(super) fn spawn_ffi_client<F>(connect: F) -> Result<FfiClient, Error>
where
    F: FnOnce(&Runtime) -> Result<Client, Error> + Send + 'static,
{
    match start_ffi_client(connect)?.finish() {
        Ok(client) => Ok(client),
        Err(failure) => Err(failure.into_error()),
    }
}

pub(crate) struct FfiClientAttempt {
    job_tx: UnboundedSender<Job>,
    worker: thread::JoinHandle<()>,
    ready_rx: mpsc::Receiver<Result<FfiClientMetadata, Error>>,
    completion_tx: mpsc::Sender<CompletionJob>,
    completion_worker: Option<thread::JoinHandle<()>>,
    connect_abort_rx: Option<mpsc::Receiver<AbortHandle>>,
}

pub(crate) struct FfiClientFailure {
    error: Error,
    completion_tx: Option<mpsc::Sender<CompletionJob>>,
    completion_worker: Option<thread::JoinHandle<()>>,
}

impl FfiClientFailure {
    pub(crate) fn error(&self) -> &Error {
        &self.error
    }

    pub(crate) fn into_error(mut self) -> Error {
        drop(self.completion_tx.take());
        if let Some(worker) = self.completion_worker.take() {
            let _ = worker.join();
        }
        mem::replace(
            &mut self.error,
            Error::Internal("connection failure was already consumed".to_owned()),
        )
    }

    pub(crate) fn into_error_without_join(mut self) -> Error {
        drop(self.completion_tx.take());
        drop(self.completion_worker.take());
        mem::replace(
            &mut self.error,
            Error::Internal("connection failure was already consumed".to_owned()),
        )
    }
}

impl FfiClientAttempt {
    pub(crate) fn completion_sender(&self) -> mpsc::Sender<CompletionJob> {
        self.completion_tx.clone()
    }

    pub(crate) fn take_connect_abort_handle(&mut self) -> Option<AbortHandle> {
        self.connect_abort_rx.take()?.recv().ok()
    }

    pub(crate) fn finish(mut self) -> Result<FfiClient, FfiClientFailure> {
        let result = self.ready_rx.recv();
        match result {
            Ok(Ok(metadata)) => Ok(FfiClient {
                core: Arc::new(FfiClientCore {
                    sender: Some(self.job_tx.clone()),
                    worker: Some(self.worker),
                    completion_sender: Some(self.completion_tx.clone()),
                    completion_worker: self.completion_worker.take(),
                    metadata: Some(metadata),
                }),
            }),
            Ok(Err(error)) => {
                let _ = self.worker.join();
                Err(FfiClientFailure {
                    error,
                    completion_tx: Some(self.completion_tx.clone()),
                    completion_worker: self.completion_worker.take(),
                })
            }
            Err(_) => {
                let _ = self.worker.join();
                Err(FfiClientFailure {
                    error: Error::Internal(
                        "client worker thread exited before signaling readiness".to_owned(),
                    ),
                    completion_tx: Some(self.completion_tx.clone()),
                    completion_worker: self.completion_worker.take(),
                })
            }
        }
    }
}

pub(crate) fn start_ffi_client<F>(connect: F) -> Result<FfiClientAttempt, Error>
where
    F: FnOnce(&Runtime) -> Result<Client, Error> + Send + 'static,
{
    let (job_tx, mut job_rx) = unbounded_channel::<Job>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<FfiClientMetadata, Error>>();

    let (completion_tx, completion_rx) = mpsc::channel::<CompletionJob>();
    let completion_worker = thread::Builder::new()
        .name("luminate-ffi-completion".to_owned())
        .spawn(move || {
            while let Ok(completion) = completion_rx.recv() {
                let _ = panic::catch_unwind(AssertUnwindSafe(completion));
            }
        })
        .map_err(|error| Error::Internal(format!("failed to spawn completion thread: {error}")))?;

    let worker = thread::Builder::new()
        .name("luminate-ffi-client".to_owned())
        .spawn(move || {
            let runtime = match create_runtime() {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
            };
            let client = match connect(&runtime) {
                Ok(client) => Arc::new(client),
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
            };
            let metadata = FfiClientMetadata {
                daemon_version: client.daemon_version().to_owned(),
                socket_path: client.socket_path().to_owned(),
                event_socket_path: client.event_socket_path(),
                session: client.session().clone(),
            };
            // The caller stopped waiting for connection setup, so no one can
            // use this client.
            if ready_tx.send(Ok(metadata)).is_err() {
                return;
            }
            drop(ready_tx);

            runtime.block_on(async move {
                while let Some(job) = job_rx.recv().await {
                    let client = Arc::clone(&client);
                    // Creating a future is consumer-independent internal work,
                    // but still keep a panic from stopping the domain.
                    let _ = panic::catch_unwind(AssertUnwindSafe(|| job(client)));
                }
            });
        })
        .map_err(|error| {
            Error::Internal(format!("failed to spawn client worker thread: {error}"))
        })?;

    Ok(FfiClientAttempt {
        job_tx,
        worker,
        ready_rx,
        completion_tx,
        completion_worker: Some(completion_worker),
        connect_abort_rx: None,
    })
}

pub(crate) fn start_ffi_client_async<F, Fut>(connect: F) -> Result<FfiClientAttempt, Error>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<Client, Error>> + Send + 'static,
{
    let (job_tx, mut job_rx) = unbounded_channel::<Job>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<FfiClientMetadata, Error>>();
    let (abort_tx, abort_rx) = mpsc::channel();
    let (completion_tx, completion_rx) = mpsc::channel::<CompletionJob>();
    let completion_worker = thread::Builder::new()
        .name("luminate-ffi-completion".to_owned())
        .spawn(move || {
            while let Ok(completion) = completion_rx.recv() {
                let _ = panic::catch_unwind(AssertUnwindSafe(completion));
            }
        })
        .map_err(|error| Error::Internal(format!("failed to spawn completion thread: {error}")))?;

    let worker = thread::Builder::new()
        .name("luminate-ffi-client".to_owned())
        .spawn(move || {
            let runtime = match create_runtime() {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
            };
            let connect_task = runtime.spawn(connect());
            if abort_tx.send(connect_task.abort_handle()).is_err() {
                connect_task.abort();
                return;
            }
            let client = match runtime.block_on(connect_task) {
                Ok(Ok(client)) => Arc::new(client),
                Ok(Err(error)) => {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(Error::Internal(format!(
                        "connection task stopped before completion: {error}"
                    ))));
                    return;
                }
            };
            let metadata = FfiClientMetadata {
                daemon_version: client.daemon_version().to_owned(),
                socket_path: client.socket_path().to_owned(),
                event_socket_path: client.event_socket_path(),
                session: client.session().clone(),
            };
            if ready_tx.send(Ok(metadata)).is_err() {
                return;
            }
            drop(ready_tx);
            runtime.block_on(async move {
                while let Some(job) = job_rx.recv().await {
                    let client = Arc::clone(&client);
                    let _ = panic::catch_unwind(AssertUnwindSafe(|| job(client)));
                }
            });
        })
        .map_err(|error| {
            Error::Internal(format!("failed to spawn client worker thread: {error}"))
        })?;

    Ok(FfiClientAttempt {
        job_tx,
        worker,
        ready_rx,
        completion_tx,
        completion_worker: Some(completion_worker),
        connect_abort_rx: Some(abort_rx),
    })
}

pub(crate) fn spawn_ffi_subscription(
    client: &FfiClient,
    subscription: EventSubscription,
) -> FfiSubscription {
    FfiSubscription {
        client: client.clone(),
        subscription: Arc::new(Mutex::new(subscription)),
        #[cfg(test)]
        started_tx: None,
    }
}

/// Spawns `f` on the client's runtime, blocking this caller until it returns.
///
/// The calling thread never enters a Tokio runtime; it only sends the job
/// and waits for the reply.
///
/// Panics from `f` are contained by the worker's panic barrier. The reply
/// channel is dropped, causing this method to return `Internal` instead
/// of aborting the process.
pub(crate) fn call_client<R, F, Fut>(client: &FfiClient, f: F) -> Result<R, LuminateStatus>
where
    R: Send + 'static,
    F: FnOnce(Arc<Client>) -> Fut + Send + 'static,
    Fut: Future<Output = R> + Send + 'static,
{
    let Some(sender) = client.core.sender.as_ref() else {
        set_last_error("libluminate client worker thread is not running");
        return Err(LuminateStatus::Internal);
    };

    let (result_tx, result_rx) = mpsc::channel::<R>();
    let job: Job = Box::new(move |client| {
        tokio::spawn(async move {
            let result = f(client).await;
            let _ = result_tx.send(result);
        });
    });

    if sender.send(job).is_err() {
        set_last_error("libluminate client worker thread is not running");
        return Err(LuminateStatus::Internal);
    }

    result_rx.recv().map_err(|_| {
        set_last_error("libluminate client worker thread stopped before responding");
        LuminateStatus::Internal
    })
}

pub(crate) fn spawn_client_task<F, Fut>(
    client: &FfiClient,
    operation: Arc<super::async_operation::LuminateAsyncOperation>,
    f: F,
) -> Result<(), LuminateStatus>
where
    F: FnOnce(Arc<Client>, Arc<super::async_operation::LuminateAsyncOperation>) -> Fut
        + Send
        + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let Some(sender) = client.core.sender.as_ref() else {
        set_last_error("libluminate client worker thread is not running");
        return Err(LuminateStatus::Internal);
    };
    let job: Job = Box::new(move |client| {
        let task_operation = Arc::clone(&operation);
        let task = tokio::spawn(f(client, task_operation));
        operation.set_abort_handle(task.abort_handle());
    });
    sender.send(job).map_err(|_| {
        set_last_error("libluminate client worker thread is not running");
        LuminateStatus::Internal
    })
}

pub(crate) fn call_subscription(
    subscription: &FfiSubscription,
) -> Result<Result<crate::Event, Error>, LuminateStatus> {
    let inner = Arc::clone(&subscription.subscription);
    call_client(&subscription.client, move |_| async move {
        let mut subscription = inner.lock().await;
        subscription.next_event().await
    })
}
