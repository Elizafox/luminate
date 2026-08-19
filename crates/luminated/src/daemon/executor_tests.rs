// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

use std::ops::Deref;
use std::thread;

use tokio::sync::oneshot;

use super::super::tests_support::{remove_request_test_dir, request_test_context};
use super::super::*;
use super::*;

use luminate_core::policy::PrincipalId;
use luminate_platform::secure_storage::ensure_private_directory;
use luminate_platform::test_support::TestDir;

impl MutationExecutor {
    async fn execute(
        &self,
        devices: Vec<device::DeviceId>,
        job: MutationJob,
    ) -> Result<(), DaemonError> {
        self.execute_inner(devices, None, job, true).await
    }
}

struct TestStatePath {
    _directory: TestDir,
    path: PathBuf,
}

impl Deref for TestStatePath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<Path> for TestStatePath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

fn test_mutation_executor(name: &str) -> (MutationExecutor, TestStatePath) {
    let runtime_dir = TestDir::new(name);
    let state_path = runtime_dir.join("state.json");
    let state = Arc::new(Mutex::new(DaemonState::default()));
    let (events, _) = EventPublisher::test_channel(&state, 16);
    (
        MutationExecutor {
            state,
            state_path: Arc::from(state_path.clone().into_boxed_path()),
            commit: Arc::new(sync::Mutex::new(())),
            device_sequencers: Arc::new(sync::Mutex::new(HashMap::new())),
            events,
            operator_events: false,
            transitions: Arc::new(TransitionRegistry::default()),
        },
        TestStatePath {
            _directory: runtime_dir,
            path: state_path,
        },
    )
}

#[test]
fn device_normalization_and_sequencer_pruning_are_stable() {
    let mut devices = vec![
        device::DeviceId::new("zeta"),
        device::DeviceId::new("alpha"),
        device::DeviceId::new("zeta"),
    ];
    normalize_devices(&mut devices);
    assert_eq!(
        devices
            .iter()
            .map(device::DeviceId::as_str)
            .collect::<Vec<_>>(),
        ["alpha", "zeta"]
    );

    let (executor, state_path) = test_mutation_executor("sequencer-pruning");
    let retained = device::DeviceId::new("retained");
    let removed = device::DeviceId::new("removed");
    let held =
        executor.sequencers_for_new_devices(&HashSet::new(), &[retained.clone(), removed.clone()]);
    let held_removed = Arc::clone(&held[0]);
    drop(held);

    executor.prune_device_sequencers(&HashSet::from([retained.clone()]));
    let known = executor.device_sequencers.lock().expect("lock poisoned");
    assert!(known.contains_key(&retained));
    assert!(
        known.contains_key(&removed),
        "an externally held lock must survive pruning"
    );
    drop(known);

    drop(held_removed);
    executor.prune_device_sequencers(&HashSet::from([retained.clone()]));
    let known = executor.device_sequencers.lock().expect("lock poisoned");
    assert!(known.contains_key(&retained));
    assert!(!known.contains_key(&removed));
    drop(known);

    fs::remove_dir(state_path.parent().expect("state path parent"))
        .expect("remove runtime directory");
}

#[test]
fn mutation_jobs_handle_panics_plain_errors_and_success() {
    let runtime_dir = TestDir::new("mutation-outcomes");
    ensure_private_directory(&runtime_dir).expect("create runtime directory");
    let state_path = runtime_dir.join("state.json");
    let state = Arc::new(Mutex::new(DaemonState::default()));
    let commit = sync::Mutex::new(());

    let error = run_mutation_job::<()>(
        &state,
        &state_path,
        &commit,
        Box::new(|_| panic!("deliberate mutation panic")),
        false,
    )
    .expect_err("panic should become a typed error");
    assert!(error.to_string().contains("panicked"));
    assert!(!state_path.exists(), "failed work must not be persisted");

    let error = run_mutation_job::<()>(
        &state,
        &state_path,
        &commit,
        Box::new(|_| Err(DaemonError::Internal("hardware failed".to_owned()))),
        false,
    )
    .expect_err("ordinary failure should be preserved");
    assert!(error.to_string().contains("hardware failed"));
    assert!(!state_path.exists(), "failed work must not be persisted");

    run_mutation_job(&state, &state_path, &commit, Box::new(|_| Ok(())), false)
        .expect("successful work should persist");
    assert!(state_path.exists());
    fs::remove_file(state_path).expect("remove state file");
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[cfg(unix)]
mod persistence_failure_event {
    use std::fmt::Debug;
    use std::os::unix::fs::PermissionsExt as _;

    use tracing::field::{Field, Visit};
    use tracing::{Event as TracingEvent, Subscriber, subscriber};
    use tracing_subscriber::layer::{Context, SubscriberExt as _};
    use tracing_subscriber::{Layer, Registry};

    use super::*;

    #[derive(Default)]
    struct CapturedOperatorEvent {
        id: Option<u64>,
    }

    impl Visit for CapturedOperatorEvent {
        fn record_u64(&mut self, field: &Field, value: u64) {
            if field.name() == "id" {
                self.id = Some(value);
            }
        }

        fn record_debug(&mut self, _field: &Field, _value: &dyn Debug) {}
    }

    struct CaptureOperatorEvents(Arc<sync::Mutex<Vec<CapturedOperatorEvent>>>);

    impl<S: Subscriber> Layer<S> for CaptureOperatorEvents {
        fn on_event(&self, event: &TracingEvent<'_>, _context: Context<'_, S>) {
            if event.metadata().target() != OperatorEvent::TARGET {
                return;
            }

            let mut captured = CapturedOperatorEvent::default();
            event.record(&mut captured);
            self.0.lock().expect("lock poisoned").push(captured);
        }
    }

    #[test]
    fn mutation_executor_reports_persistence_failure_after_successful_job() {
        let runtime_dir = TestDir::uncreated("persist-failure");
        fs::create_dir_all(&runtime_dir).expect("create runtime directory");
        fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o777))
            .expect("make state directory unsafe");

        let state = Arc::new(Mutex::new(DaemonState::default()));
        let state_path: Arc<Path> = Arc::from(runtime_dir.join("state.json").into_boxed_path());
        let commit = sync::Mutex::new(());
        let captured = Arc::new(sync::Mutex::new(Vec::new()));
        let tracing = Registry::default().with(CaptureOperatorEvents(Arc::clone(&captured)));
        let error = subscriber::with_default(tracing, || {
            run_mutation_job(&state, &state_path, &commit, Box::new(|_| Ok(())), true)
        })
        .expect_err("persistence failure must be client-visible");
        assert!(
            error.to_string().contains("failed to persist daemon state"),
            "unexpected error: {error}"
        );
        let captured = captured.lock().expect("lock poisoned");
        assert_eq!(captured.len(), 1);
        assert_eq!(
            captured[0].id,
            Some(u64::from(OperatorEvent::PersistedStateSaveFailed.id()))
        );
        drop(captured);
        fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700))
            .expect("restore permissions for cleanup");
        fs::remove_dir(runtime_dir).expect("remove runtime directory");
    }
}

#[test]
fn partial_mutation_state_is_persisted_before_error_is_returned() {
    let runtime_dir = TestDir::new("persist-partial");
    ensure_private_directory(&runtime_dir).expect("create runtime directory");
    let state_path = runtime_dir.join("state.json");
    let state = Arc::new(Mutex::new(DaemonState::default()));
    let commit = sync::Mutex::new(());

    let error = run_mutation_job::<()>(
        &state,
        &state_path,
        &commit,
        Box::new(move |state| {
            state
                .blocking_lock()
                .create_collection(
                    "Already applied".to_owned(),
                    None,
                    OwnerIdentity::Principal(
                        PrincipalId::new("unix", "1000").expect("valid principal"),
                    ),
                    None,
                    Vec::new(),
                )
                .expect("create collection before reporting failure");
            Err(DaemonError::PartialMutation {
                diagnostic: "later member failed".to_owned(),
                applied_targets: Vec::new(),
            })
        }),
        false,
    )
    .expect_err("partial mutation remains client-visible");

    assert!(matches!(error, DaemonError::PartialMutation { .. }));
    let persisted = persistence::load(&state_path).expect("load partial state");
    assert_eq!(persisted.collections.len(), 1);
    fs::remove_file(state_path).expect("remove state file");
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[test]
fn withdrawn_device_purge_is_persisted_atomically() {
    let runtime_dir = TestDir::new("persist-purge");
    ensure_private_directory(&runtime_dir).expect("create runtime directory");
    let state_path = runtime_dir.join("state.json");
    let device = device::DeviceId::new("retired-device");
    let target = TargetId::Device(device.clone());
    let mut daemon_state = DaemonState::default();
    assert_eq!(
        daemon_state.restore_persisted_retaining_withdrawn(
            vec![TargetStateEntry {
                target,
                state: TargetState::Brightness(42),
            }],
            true,
        ),
        (0, 1, 0)
    );
    let state = Arc::new(Mutex::new(daemon_state));
    let commit = sync::Mutex::new(());

    run_mutation_job(
        &state,
        &state_path,
        &commit,
        Box::new(move |state| {
            state.blocking_lock().purge_withdrawn_device(&device)?;
            Ok(())
        }),
        false,
    )
    .expect("persist purge");

    let persisted = persistence::load(&state_path).expect("load purged state");
    assert!(persisted.entries.is_empty());
    fs::remove_file(state_path).expect("remove state file");
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_mutation_publishes_state_dirty_bit() {
    let (executor, state_path) = test_mutation_executor("state-event");
    let device = device::DeviceId::new("changed-device");
    let mut events = executor.events.subscribe();

    executor
        .execute(vec![device.clone()], Box::new(|_| Ok(())))
        .await
        .expect("commit mutation");

    assert_eq!(
        events.recv().await.expect("receive state event"),
        Event::StateChanged {
            devices: vec![device],
        }
    );
    let _ = fs::remove_file(&state_path);
    let _ = fs::remove_dir(state_path.parent().expect("state parent"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardware_only_work_is_not_persisted_or_announced() {
    let (executor, state_path) = test_mutation_executor("hardware-only");
    let device = device::DeviceId::new("streamed-device");
    let mut events = executor.events.subscribe();

    executor
        .execute_hardware_only(vec![device.clone()], Box::new(|| Ok(())))
        .await
        .expect("run hardware-only work");

    assert!(!state_path.exists());
    assert_eq!(executor.state.lock().await.generation(&device), 0);
    assert!(matches!(
        events.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    fs::remove_dir(state_path.parent().expect("state parent")).expect("remove runtime directory");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardware_only_work_and_mutations_share_device_ordering() {
    let (executor, state_path) = test_mutation_executor("stream-mutation-sequencing");
    let device = device::DeviceId::new("device-a");
    let order = Arc::new(sync::Mutex::new(Vec::new()));
    let (started_tx, started_rx) = oneshot::channel();

    let frame = {
        let executor = executor.clone();
        let order = Arc::clone(&order);
        let device = device.clone();
        tokio::spawn(async move {
            executor
                .execute_hardware_only(
                    vec![device],
                    Box::new(move || {
                        order.lock().expect("order lock").push("frame-start");
                        let _ = started_tx.send(());
                        thread::sleep(Duration::from_millis(40));
                        order.lock().expect("order lock").push("frame-end");
                        Ok(())
                    }),
                )
                .await
        })
    };
    started_rx.await.expect("frame started");
    let mutation = {
        let executor = executor.clone();
        let order = Arc::clone(&order);
        let device = device.clone();
        tokio::spawn(async move {
            executor
                .execute(
                    vec![device],
                    Box::new(move |_| {
                        order.lock().expect("order lock").push("mutation");
                        Ok(())
                    }),
                )
                .await
        })
    };

    frame.await.expect("frame task").expect("frame upload");
    mutation
        .await
        .expect("mutation task")
        .expect("ordinary mutation");
    assert_eq!(
        *order.lock().expect("order lock"),
        ["frame-start", "frame-end", "mutation"]
    );
    let _ = fs::remove_file(&state_path);
    fs::remove_dir(state_path.parent().expect("state parent")).expect("remove runtime directory");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authorized_mutation_rejects_a_stale_topology_generation() {
    let (state, _manager, executor, runtime_dir) =
        request_test_context("authorization-topology-race");
    let device = device::DeviceId::new("request-device");
    let authorized_generation = state.lock().await.topology_generation();
    state
        .lock()
        .await
        .replace_devices_preserving_withdrawn_state(Vec::new());
    let job_ran = Arc::new(sync::atomic::AtomicBool::new(false));
    let job_ran_in_job = Arc::clone(&job_ran);

    let error = executor
        .execute_authorized(
            vec![device],
            authorized_generation,
            Box::new(move |_| {
                job_ran_in_job.store(true, sync::atomic::Ordering::Relaxed);
                Ok(())
            }),
        )
        .await
        .expect_err("stale authorization must be rejected");

    assert!(matches!(error, DaemonError::AuthorizationConflict { .. }));
    assert!(!job_ran.load(sync::atomic::Ordering::Relaxed));
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn rejected_persistence_candidate_restores_previous_live_and_durable_state() {
    let (executor, state_path) = test_mutation_executor("persistence-candidate-rollback");
    executor
        .execute(Vec::new(), Box::new(|_| Ok(())))
        .await
        .expect("write initial snapshot");
    let original = fs::read(&state_path).expect("read initial snapshot");

    let error = executor
        .execute(
            Vec::new(),
            Box::new(|state| {
                state.blocking_lock().create_collection(
                    "x".repeat(persistence::MAX_TEXT_BYTES + 1),
                    None,
                    OwnerIdentity::Principal(
                        PrincipalId::new("unix", "1000").expect("valid principal"),
                    ),
                    None,
                    Vec::new(),
                )?;
                Ok(())
            }),
        )
        .await
        .expect_err("oversized candidate must fail");

    assert!(error.to_string().contains("collection name"));
    assert!(executor.state.lock().await.collections().is_empty());
    assert_eq!(
        fs::read(&state_path).expect("read durable snapshot"),
        original
    );
    let loaded = persistence::load(&state_path).expect("load durable snapshot");
    assert!(loaded.collections.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_device_jobs_keep_hardware_and_commit_order_aligned() {
    let (executor, state_path) = test_mutation_executor("same-device-sequencing");
    let device = device::DeviceId::new("device-a");
    let order = Arc::new(sync::Mutex::new(Vec::new()));
    let (started_tx, started_rx) = oneshot::channel();

    let first = {
        let executor = executor.clone();
        let order = Arc::clone(&order);
        let device = device.clone();
        tokio::spawn(async move {
            executor
                .execute(
                    vec![device],
                    Box::new(move |_| {
                        order.lock().expect("order lock").push("first-start");
                        let _ = started_tx.send(());
                        thread::sleep(Duration::from_millis(40));
                        order.lock().expect("order lock").push("first-end");
                        Ok(())
                    }),
                )
                .await
        })
    };
    started_rx.await.expect("first started");
    let second = {
        let executor = executor.clone();
        let order = Arc::clone(&order);
        let device = device.clone();
        tokio::spawn(async move {
            executor
                .execute(
                    vec![device],
                    Box::new(move |_| {
                        order.lock().expect("order lock").push("second");
                        Ok(())
                    }),
                )
                .await
        })
    };
    first.await.expect("first task").expect("first mutation");
    second.await.expect("second task").expect("second mutation");
    assert_eq!(
        *order.lock().expect("order lock"),
        ["first-start", "first-end", "second"]
    );
    assert_eq!(executor.state.lock().await.generation(&device), 2);
    let _ = fs::remove_file(&state_path);
    let _ = fs::remove_dir(state_path.parent().expect("state parent"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unrelated_device_jobs_can_do_hardware_work_concurrently() {
    let (executor, state_path) = test_mutation_executor("cross-device-concurrency");
    let barrier = Arc::new(sync::Barrier::new(2));
    let spawn = |device: &'static str| {
        let executor = executor.clone();
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            executor
                .execute(
                    vec![device::DeviceId::new(device)],
                    Box::new(move |_| {
                        barrier.wait();
                        Ok(())
                    }),
                )
                .await
        })
    };
    let (left, right) = tokio::join!(spawn("left"), spawn("right"));
    left.expect("left task").expect("left mutation");
    right.expect("right task").expect("right mutation");
    let _ = fs::remove_file(&state_path);
    let _ = fs::remove_dir(state_path.parent().expect("state parent"));
}
