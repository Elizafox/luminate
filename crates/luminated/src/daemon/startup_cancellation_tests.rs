// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::process;

use super::*;
use luminate_platform::test_support::TestDir;
use tokio::sync::oneshot;

pub(super) struct TestMilestonePause {
    target: StartupMilestone,
    handoff: sync::Mutex<Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>>,
}

impl TestMilestonePause {
    fn new(target: StartupMilestone) -> (Arc<Self>, oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (paused_tx, paused_rx) = oneshot::channel();
        let (resume_tx, resume_rx) = oneshot::channel();
        let pause = Arc::new(Self {
            target,
            handoff: sync::Mutex::new(Some((paused_tx, resume_rx))),
        });
        (pause, paused_rx, resume_tx)
    }

    pub(super) async fn pause_if_matching(&self, milestone: StartupMilestone) {
        if milestone != self.target {
            return;
        }
        let handoff = self
            .handoff
            .lock()
            .expect("milestone pause handoff lock poisoned")
            .take();
        if let Some((paused, resume)) = handoff {
            let _ = paused.send(());
            let _ = resume.await;
        }
    }
}

impl RunContext {
    fn reporting_paused_at(
        shutdown: mpsc::Receiver<()>,
        lifecycle: mpsc::UnboundedSender<LifecycleEvent>,
        target: StartupMilestone,
    ) -> (Self, oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (pause, paused_rx, resume_tx) = TestMilestonePause::new(target);
        let context = Self {
            shutdown: ShutdownSource::Requested(shutdown),
            lifecycle: LifecycleReporter::new_paused(lifecycle, pause),
            listener_access: ListenerAccess::OwnerOnly,
            operator_events: false,
            power_events: PowerEventSource::Platform,
        };
        (context, paused_rx, resume_tx)
    }
}

impl LifecycleReporter {
    fn new_paused(
        events: mpsc::UnboundedSender<LifecycleEvent>,
        pause: Arc<TestMilestonePause>,
    ) -> Self {
        Self {
            events: Some(events),
            test_pause: TestPause::Paused(pause),
        }
    }
}

const MILESTONE_TARGET_ENV: &str = "LUMINATED_MILESTONE_PAUSE_TARGET";

/// Every [`StartupMilestone`], in the order `run()` reports them.
const MILESTONES_IN_ORDER: [StartupMilestone; 6] = [
    StartupMilestone::ConfigurationLoaded,
    StartupMilestone::PluginsLoaded,
    StartupMilestone::ListenersBound,
    StartupMilestone::PersistedStateRestored,
    StartupMilestone::StartupReconciled,
    StartupMilestone::AuthorizationReady,
];

fn milestone_name(milestone: StartupMilestone) -> &'static str {
    match milestone {
        StartupMilestone::ConfigurationLoaded => "ConfigurationLoaded",
        StartupMilestone::PluginsLoaded => "PluginsLoaded",
        StartupMilestone::ListenersBound => "ListenersBound",
        StartupMilestone::PersistedStateRestored => "PersistedStateRestored",
        StartupMilestone::StartupReconciled => "StartupReconciled",
        StartupMilestone::AuthorizationReady => "AuthorizationReady",
    }
}

fn milestone_from_name(name: &str) -> StartupMilestone {
    *MILESTONES_IN_ORDER
        .iter()
        .find(|milestone| milestone_name(**milestone) == name)
        .unwrap_or_else(|| panic!("unknown startup milestone probe target: {name}"))
}

#[test]
fn cancellation_stops_daemon_run_at_each_startup_milestone() {
    for &target in &MILESTONES_IN_ORDER {
        let runtime_dir = TestDir::new(&format!(
            "startup-cancel-{}",
            milestone_name(target).to_lowercase()
        ));
        fs::create_dir_all(&runtime_dir).expect("create runtime directory");
        let config_path = runtime_dir.join("luminated.toml");
        fs::write(
            &config_path,
            "[plugin_management]\nactivation = \"explicit\"\n",
        )
        .expect("write daemon config");
        let socket = runtime_dir.join("primary.sock");
        let event_socket = runtime_dir.join("events.sock");
        let state = runtime_dir.join("state.json");

        let mut command =
            process::Command::new(env::current_exe().expect("locate daemon test executable"));
        command
            .arg("--ignored")
            .arg("--exact")
            .arg("daemon::startup_cancellation_tests::milestone_pause_child_probe")
            .arg("--test-threads=1")
            .env(MILESTONE_TARGET_ENV, milestone_name(target))
            .env(CONFIG_PATH_ENV, &config_path)
            .env(SOCKET_PATH_ENV, &socket)
            .env(EVENT_SOCKET_PATH_ENV, &event_socket)
            .env(STATE_PATH_ENV, &state);
        let output = command.output().expect("run isolated milestone probe");
        assert!(
            output.status.success(),
            "milestone probe {} failed: {}",
            milestone_name(target),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[tokio::test]
#[ignore = "run in an isolated child by cancellation_stops_daemon_run_at_each_startup_milestone"]
async fn milestone_pause_child_probe() {
    let target = milestone_from_name(
        &env::var(MILESTONE_TARGET_ENV).expect("milestone probe target variable"),
    );

    let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
    let (lifecycle_tx, mut lifecycle_rx) = mpsc::unbounded_channel();
    let (context, paused_rx, resume_tx) =
        RunContext::reporting_paused_at(shutdown_rx, lifecycle_tx, target);

    let run_task = tokio::spawn(run(context));

    timeout(Duration::from_secs(5), paused_rx)
        .await
        .expect("run() should pause at the target milestone promptly")
        .expect("run() dropped the pause signal before pausing");

    // `run()` is now parked immediately after reporting `target` and
    // before that milestone's shutdown checkpoint. Every earlier
    // milestone, and only those, must already have been reported.
    let mut observed = Vec::new();
    while let Ok(event) = lifecycle_rx.try_recv() {
        observed.push(event);
    }
    let target_index = MILESTONES_IN_ORDER
        .iter()
        .position(|milestone| *milestone == target)
        .expect("target milestone is one of MILESTONES_IN_ORDER");
    let expected: Vec<LifecycleEvent> = MILESTONES_IN_ORDER[..=target_index]
        .iter()
        .map(|milestone| LifecycleEvent::StartupProgress(*milestone))
        .collect();
    assert_eq!(
        observed,
        expected,
        "run() should report exactly the milestones up to and including {}",
        milestone_name(target)
    );

    shutdown_tx.send(()).await.expect("queue shutdown request");
    resume_tx
        .send(())
        .expect("release run() from its milestone pause");

    let result = timeout(Duration::from_secs(5), run_task)
        .await
        .expect("run() should stop promptly once its paused checkpoint observes the request")
        .expect("run() task should not panic");
    result.expect("cancelling at a startup milestone should be a clean shutdown, not an error");

    assert_eq!(
        lifecycle_rx.recv().await,
        None,
        "no further milestone or readiness event should follow the one cancellation targeted"
    );
}
