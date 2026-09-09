// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::future;
#[cfg(unix)]
use std::os::unix::net::UnixListener as StdUnixListener;
use std::process;

#[cfg(unix)]
use super::super::tests_support::{empty_plugin_manager, test_management_read_state, test_policy};
use super::*;
use crate::device_config::PluginActivationMode;
use luminate_platform::test_support::TestDir;

#[tokio::test]
async fn connection_cleanup_drains_completed_and_aborts_stuck_tasks() {
    let mut completed = JoinSet::new();
    completed.spawn(async {});
    completed.spawn(async { panic!("deliberate connection task panic") });
    cleanup_connection_tasks_with_timeout(&mut completed, Duration::from_secs(1))
        .await
        .expect("completed tasks should drain");

    let mut stuck = JoinSet::new();
    stuck.spawn(future::pending());
    cleanup_connection_tasks_with_timeout(&mut stuck, Duration::from_millis(10))
        .await
        .expect("stuck tasks should be aborted");
    assert!(stuck.is_empty());

    cleanup_connection_tasks(JoinSet::new())
        .await
        .expect("empty production cleanup should succeed");
}

#[test]
fn connection_limiter_isolates_uids_and_reuses_released_capacity() {
    let mut limiter = ConnectionLimiter::new(4, 2);
    let first = limiter
        .try_acquire(RateLimitKey::Uid(1000))
        .expect("first UID slot");
    let second = limiter
        .try_acquire(RateLimitKey::Uid(1000))
        .expect("second UID slot");
    assert!(matches!(
        limiter.try_acquire(RateLimitKey::Uid(1000)),
        Err(ConnectionLimitReached::PerUid)
    ));

    let other_uid = limiter
        .try_acquire(RateLimitKey::Uid(1001))
        .expect("another UID retains its own capacity");
    drop(first);
    assert!(limiter.try_acquire(RateLimitKey::Uid(1000)).is_ok());

    drop(second);
    drop(other_uid);
    limiter.prune_idle_uids();
    assert!(!limiter.per_principal.contains_key(&RateLimitKey::Uid(1001)));
}

#[test]
fn connection_limiter_isolates_sid_principals_separately_from_uids() {
    let mut limiter = ConnectionLimiter::new(4, 1);
    let uid_permit = limiter
        .try_acquire(RateLimitKey::Uid(1000))
        .expect("uid slot");
    let sid_permit = limiter
        .try_acquire(RateLimitKey::Sid("S-1-5-21-1-2-3-1000".to_owned()))
        .expect("a same-numeric-looking SID must not share the UID's slot");
    drop(uid_permit);
    drop(sid_permit);
}

#[test]
fn connection_limiter_preserves_the_global_bound() {
    let mut limiter = ConnectionLimiter::new(2, 2);
    let _first = limiter
        .try_acquire(RateLimitKey::Uid(1000))
        .expect("first global slot");
    let _second = limiter
        .try_acquire(RateLimitKey::Uid(1001))
        .expect("second global slot");

    assert!(matches!(
        limiter.try_acquire(RateLimitKey::Uid(1002)),
        Err(ConnectionLimitReached::Global)
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn owned_listeners_clean_up_and_accept_independently() {
    use std::os::unix::fs::PermissionsExt as _;

    let runtime_dir = TestDir::new("owned-listeners");
    fs::create_dir_all(&runtime_dir).expect("create runtime directory");
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700))
        .expect("secure runtime directory");
    let primary_path = runtime_dir.join("primary.sock");
    let event_path = runtime_dir.join("event.sock");
    let primary =
        bind_listener(&primary_path, &ListenerAccess::OwnerOnly).expect("bind primary listener");
    let event =
        bind_listener(&event_path, &ListenerAccess::OwnerOnly).expect("bind event listener");
    let (sender, mut receiver) = mpsc::channel(2);
    let primary_task = tokio::spawn(run_accept_worker(
        primary,
        ListenerKind::Primary,
        sender.clone(),
    ));
    let event_task = tokio::spawn(run_accept_worker(event, ListenerKind::Event, sender));

    let _primary_client = UnixStream::connect(&primary_path)
        .await
        .expect("connect primary client");
    let _event_client = UnixStream::connect(&event_path)
        .await
        .expect("connect event client");
    let first = timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("first accept timeout")
        .expect("first accept worker stopped")
        .expect("first accept failed");
    let second = timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("second accept timeout")
        .expect("second accept worker stopped")
        .expect("second accept failed");
    assert!(matches!(
        (first.kind, second.kind),
        (ListenerKind::Primary, ListenerKind::Event) | (ListenerKind::Event, ListenerKind::Primary)
    ));

    primary_task.abort();
    event_task.abort();
    let _ = primary_task.await;
    let _ = event_task.await;
    assert!(!primary_path.exists());
    assert!(!event_path.exists());
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[cfg(unix)]
#[tokio::test]
async fn owned_listener_drop_warns_instead_of_removing_a_replaced_socket() {
    use std::os::unix::fs::PermissionsExt as _;

    let runtime_dir = TestDir::new("owned-listener-drop-replaced");
    fs::create_dir_all(&runtime_dir).expect("create runtime directory");
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700))
        .expect("secure runtime directory");
    let path = runtime_dir.join("luminated.sock");

    let owned = bind_listener(&path, &ListenerAccess::OwnerOnly).expect("bind owned listener");
    // Simulate another process replacing the socket out from under this
    // one between bind and shutdown: `Drop` must refuse to remove it
    // (exercised directly in `remove_owned_socket`'s own tests) and log a
    // warning instead of silently continuing.
    fs::remove_file(&path).expect("remove original socket");
    let _replacement = StdUnixListener::bind(&path).expect("bind replacement socket");

    drop(owned);
    assert!(path.exists(), "the replacement socket must survive Drop");

    fs::remove_file(&path).expect("remove replacement socket");
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[cfg(unix)]
#[tokio::test]
async fn accept_worker_stops_once_its_receiver_is_gone() {
    use std::os::unix::fs::PermissionsExt as _;

    let runtime_dir = TestDir::new("accept-worker-receiver-gone");
    fs::create_dir_all(&runtime_dir).expect("create runtime directory");
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700))
        .expect("secure runtime directory");
    let path = runtime_dir.join("luminated.sock");
    let listener = bind_listener(&path, &ListenerAccess::OwnerOnly).expect("bind listener");

    let (sender, receiver) = mpsc::channel(1);
    let worker = tokio::spawn(run_accept_worker(listener, ListenerKind::Primary, sender));
    // Dropping the receiver before the worker's `send` completes forces
    // the worker down its "nobody is listening anymore" exit path rather
    // than the happy-path delivery.
    drop(receiver);

    let _client = UnixStream::connect(&path).await.expect("connect client");
    timeout(Duration::from_secs(1), worker)
        .await
        .expect("worker should exit once its receiver is dropped")
        .expect("worker task should not panic");
    // The worker's `Listener` is dropped along with the task,
    // which removes the socket file itself.
    assert!(!path.exists());
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[cfg(unix)]
#[tokio::test]
async fn accept_loop_accepts_both_kinds_and_enforces_the_per_uid_connection_limit() {
    use std::os::unix::fs::PermissionsExt as _;
    use tokio::io::AsyncReadExt as _;

    let runtime_dir = TestDir::new("accept-loop-limits");
    fs::create_dir_all(&runtime_dir).expect("create runtime directory");
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700))
        .expect("secure runtime directory");
    let primary_path = runtime_dir.join("primary.sock");
    let event_path = runtime_dir.join("event.sock");
    let primary =
        bind_listener(&primary_path, &ListenerAccess::OwnerOnly).expect("bind primary listener");
    let event =
        bind_listener(&event_path, &ListenerAccess::OwnerOnly).expect("bind event listener");

    let state = Arc::new(Mutex::new(DaemonState::default()));
    let manager = empty_plugin_manager();
    let state_path: Arc<Path> = Arc::from(runtime_dir.join("state.json").into_boxed_path());
    let (topology_tx, topology_rx) = mpsc::unbounded_channel();
    let (_shutdown_tx, shutdown_rx) = mpsc::channel(1);
    let task = tokio::spawn(accept_loop(
        primary,
        event,
        state,
        manager,
        test_management_read_state(),
        state_path,
        topology_rx,
        RescanRequester::new(topology_tx),
        test_policy(),
        Arc::new(NullSink),
        ShutdownSource::Requested(shutdown_rx),
        LifecycleReporter::discarding(),
        false,
        start_sources(),
    ));

    // Every client below connects without sending a handshake, so each
    // spawned connection task blocks on `handle_connection`'s
    // handshake-receive timeout (five seconds) rather than completing
    // immediately. That keeps their connection-limiter permits held for
    // the duration of this test, which is what makes the per-UID
    // rejection below deterministic rather than a timing race.
    let mut held_connections = Vec::new();
    for _ in 0..MAX_CONNECTIONS_PER_UID {
        held_connections.push(
            UnixStream::connect(&primary_path)
                .await
                .expect("connect primary client"),
        );
    }

    // One more connection from the same UID must be rejected outright:
    // the daemon accepts the socket only to immediately drop it without
    // ever reading a handshake, so this client observes EOF right away
    // instead of hanging until the handshake timeout.
    let mut rejected = UnixStream::connect(&primary_path)
        .await
        .expect("connect over-limit client");
    let mut buffer = [0_u8; 1];
    let read = timeout(Duration::from_secs(1), rejected.read(&mut buffer))
        .await
        .expect("over-limit client should observe a prompt disconnect")
        .expect("reading a dropped connection should not error");
    assert_eq!(read, 0, "over-limit connection must be closed, not served");

    // The event listener has its own subscriber-slot bookkeeping; make
    // sure it is reachable independently of the primary limiter.
    let _event_client = UnixStream::connect(&event_path)
        .await
        .expect("connect event client");

    drop(held_connections);
    drop(rejected);
    task.abort();
    let _ = task.await;
    fs::remove_dir_all(runtime_dir).expect("remove runtime directory");
}

#[cfg(unix)]
#[tokio::test]
async fn accept_loop_reports_ready_before_honouring_an_injected_shutdown() {
    use std::os::unix::fs::PermissionsExt as _;

    let runtime_dir = TestDir::new("injected-shutdown");
    fs::create_dir_all(&runtime_dir).expect("create runtime directory");
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700))
        .expect("secure runtime directory");
    let primary = bind_listener(
        &runtime_dir.join("primary.sock"),
        &ListenerAccess::OwnerOnly,
    )
    .expect("bind primary");
    let event = bind_listener(&runtime_dir.join("event.sock"), &ListenerAccess::OwnerOnly)
        .expect("bind event");
    let state_path: Arc<Path> = Arc::from(runtime_dir.join("state.json").into_boxed_path());
    let (topology_tx, topology_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
    let (lifecycle_tx, mut lifecycle_rx) = mpsc::unbounded_channel();
    let context = RunContext::reporting(shutdown_rx, lifecycle_tx);
    let task = tokio::spawn(accept_loop(
        primary,
        event,
        Arc::new(Mutex::new(DaemonState::default())),
        empty_plugin_manager(),
        test_management_read_state(),
        state_path,
        topology_rx,
        RescanRequester::new(topology_tx),
        test_policy(),
        Arc::new(NullSink),
        context.shutdown,
        context.lifecycle,
        false,
        start_sources(),
    ));

    assert_eq!(
        timeout(Duration::from_secs(1), lifecycle_rx.recv())
            .await
            .expect("accept loop should report readiness promptly"),
        Some(LifecycleEvent::Ready)
    );
    shutdown_tx.send(()).await.expect("request daemon shutdown");
    timeout(Duration::from_secs(1), task)
        .await
        .expect("accept loop should stop promptly")
        .expect("accept loop task should not panic")
        .expect("injected shutdown should be graceful");

    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[cfg(unix)]
#[tokio::test]
#[allow(
    unsafe_code,
    reason = "libc::raise() takes one integer, touches no memory, and only queues a signal to \
                  this same process; it is the ordinary way for a test to deliver a signal to \
                  itself, and the standard library exposes no safe wrapper for it."
)]
async fn sigusr1_requests_a_rescan_without_stopping_the_daemon() {
    use std::os::unix::fs::PermissionsExt as _;
    use tokio::signal::unix::{SignalKind, signal};

    // Register SIGUSR1 in the test process *before* anything raises it.
    // Its default disposition is terminate, so without this the signal
    // could kill the whole test binary in the window before `accept_loop`
    // installs its own handler. Held for the duration of the test; tokio
    // delivers to every registered stream, so `accept_loop` still sees it.
    let _guard = signal(SignalKind::user_defined1()).expect("register SIGUSR1 for the test");

    let runtime_dir = TestDir::new("sigusr1-rescan");
    fs::create_dir_all(&runtime_dir).expect("create runtime directory");
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700))
        .expect("secure runtime directory");
    let primary = bind_listener(
        &runtime_dir.join("primary.sock"),
        &ListenerAccess::OwnerOnly,
    )
    .expect("bind primary");
    let event = bind_listener(&runtime_dir.join("event.sock"), &ListenerAccess::OwnerOnly)
        .expect("bind event");
    let state_path: Arc<Path> = Arc::from(runtime_dir.join("state.json").into_boxed_path());
    let (_coordinator_tx, coordinator_rx) = mpsc::unbounded_channel();
    // A separate channel from the coordinator's so the test can observe
    // what the signal handler actually sends. In production both halves
    // are the same channel.
    let (rescan_tx, mut rescan_rx) = mpsc::unbounded_channel();
    let (_shutdown_tx, shutdown_rx) = mpsc::channel(1);
    let task = tokio::spawn(accept_loop(
        primary,
        event,
        Arc::new(Mutex::new(DaemonState::default())),
        empty_plugin_manager(),
        test_management_read_state(),
        state_path,
        coordinator_rx,
        RescanRequester::new(rescan_tx),
        test_policy(),
        Arc::new(NullSink),
        ShutdownSource::Requested(shutdown_rx),
        LifecycleReporter::discarding(),
        false,
        start_sources(),
    ));

    // Raise repeatedly until the request lands: `accept_loop` installs its
    // handler asynchronously, so a single early signal can be delivered
    // only to this test's own registration and never reach the daemon.
    let observed = timeout(Duration::from_secs(5), async {
        loop {
            // SAFETY: raising a signal already registered above, in this
            // process only.
            assert_eq!(unsafe { libc::raise(libc::SIGUSR1) }, 0);
            if let Ok(notification) = timeout(Duration::from_millis(50), rescan_rx.recv()).await {
                break notification.expect("rescan sender should outlive the loop");
            }
        }
    })
    .await
    .expect("SIGUSR1 should reach the accept loop");

    assert_eq!(
        observed,
        TopologyNotification::Rescan(RescanReason::Operator)
    );
    assert!(
        !task.is_finished(),
        "SIGUSR1 must request a rescan, not shut the daemon down"
    );

    task.abort();
    let _ = task.await;
    fs::remove_dir_all(runtime_dir).expect("remove runtime directory");
}

#[test]
fn load_config_honours_explicit_files_and_path_overrides() {
    let runtime_dir = TestDir::new("load-config");
    fs::create_dir_all(&runtime_dir).expect("create runtime directory");
    let config_path = runtime_dir.join("luminated.toml");
    fs::write(
        &config_path,
        "[plugin_management]\nactivation = \"explicit\"\n",
    )
    .expect("write explicit daemon config");
    let socket = runtime_dir.join("primary.sock");
    let event_socket = runtime_dir.join("events.sock");
    let state = runtime_dir.join("state.json");

    let run_case = |case: &str, explicit: Option<&Path>| {
        let mut command =
            process::Command::new(env::current_exe().expect("locate daemon test executable"));
        command
            .arg("--ignored")
            .arg("--exact")
            .arg("daemon::listener::tests::load_config_child_probe")
            .arg("--test-threads=1")
            .env("LUMINATED_CONFIG_TEST_CASE", case)
            .env(SOCKET_PATH_ENV, &socket)
            .env(EVENT_SOCKET_PATH_ENV, &event_socket)
            .env(STATE_PATH_ENV, &state);
        if let Some(explicit) = explicit {
            command.env(CONFIG_PATH_ENV, explicit);
        } else {
            command.env_remove(CONFIG_PATH_ENV);
        }
        let output = command.output().expect("run isolated config probe");
        assert!(
            output.status.success(),
            "config probe {case} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run_case("missing", Some(&runtime_dir.join("missing.toml")));
    run_case("explicit", Some(&config_path));
    run_case("fallback", None);

    fs::remove_file(config_path).expect("remove explicit config");
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[test]
#[ignore = "run in an isolated child by the parent config test"]
fn load_config_child_probe() {
    let env_path = |name| PathBuf::from(env::var_os(name).expect("probe path variable"));
    match env::var("LUMINATED_CONFIG_TEST_CASE").expect("config probe case") {
        case if case == "missing" => {
            let error = load_config().expect_err("explicit missing config must be fatal");
            assert!(error.to_string().contains(CONFIG_PATH_ENV));
        }
        case if case == "explicit" => {
            let config = load_config().expect("load config and runtime overrides");
            assert_eq!(
                config.plugin_activation_mode(),
                PluginActivationMode::Explicit
            );
            assert_eq!(config.socket_path, env_path(SOCKET_PATH_ENV));
            assert_eq!(
                config.event_socket_path,
                Some(env_path(EVENT_SOCKET_PATH_ENV))
            );
            assert_eq!(config.state_path, env_path(STATE_PATH_ENV));
        }
        case if case == "fallback" => {
            let config = load_config().expect("load compiled defaults with runtime overrides");
            assert_eq!(config.socket_path, env_path(SOCKET_PATH_ENV));
        }
        case => panic!("unknown config probe case: {case}"),
    }
}
