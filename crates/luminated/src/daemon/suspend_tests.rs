// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use luminate_core::device::DeviceId;

use super::super::tests_support::{remove_request_test_dir, request_test_context};
use super::super::*;
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quiescing_marks_observations_stale_without_touching_desired_state() {
    use luminate_core::colour::Colour;
    use luminate_core::rgb::Rgb;

    let (state, manager, mutations, runtime_dir) = request_test_context("quiesce-observations");
    let target = TargetId::device("request-device");
    let colour = Colour::rgb(Rgb::new(4, 5, 6));

    // A successful write records an `assumed` observation alongside the
    // desired entry; that is the knowledge a suspend invalidates.
    state
        .lock()
        .await
        .set_static_colour_for_test(target.clone(), colour.clone())
        .expect("record desired colour");
    assert!(
        state
            .lock()
            .await
            .device_state_status(target.device_id())
            .is_some_and(|status| status.observations.iter().any(|facet| !facet.stale)),
        "precondition: at least one observation should start fresh"
    );

    quiesce_for_suspend(&mutations, &manager).await;

    let status = state
        .lock()
        .await
        .device_state_status(target.device_id())
        .expect("device status after quiesce");
    assert!(
        status.observations.iter().all(|facet| facet.stale),
        "every observation must be stale after a suspend"
    );

    // Intent is untouched: the user still wants this colour, and resume
    // must be able to restore it.
    assert!(
        state
            .lock()
            .await
            .target_states()
            .iter()
            .any(|entry| entry.target == target),
        "desired state must survive a suspend"
    );

    remove_request_test_dir(&runtime_dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quiescing_is_harmless_with_nothing_to_quiesce() {
    let (_state, manager, mutations, runtime_dir) = request_test_context("quiesce-empty");
    quiesce_for_suspend(&mutations, &manager).await;
    remove_request_test_dir(&runtime_dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quiescing_ends_active_streams_and_reports_the_affected_device() {
    let (state, manager, mutations, runtime_dir) = request_test_context("quiesce-stream");
    let target = TargetId::device("request-device");
    state
        .lock()
        .await
        .begin_frame_stream(&target)
        .expect("begin daemon frame stream");
    let mut events = mutations.events.subscribe();

    quiesce_for_suspend(&mutations, &manager).await;

    assert!(
        state.lock().await.streaming_targets().is_empty(),
        "no frame stream may survive suspend"
    );
    let event = events.recv().await.expect("state change event");
    assert!(matches!(
        event.event,
        Event::StateChanged { devices }
            if devices == vec![DeviceId::new("request-device")]
    ));
    remove_request_test_dir(&runtime_dir);
}

#[test]
fn resume_requests_a_rescan_and_survives_a_departed_coordinator() {
    let (sender, mut requested) = mpsc::unbounded_channel();
    let rescans = RescanRequester::new(sender);
    reconcile_after_resume(&rescans);
    assert_eq!(
        requested
            .try_recv()
            .expect("resume should request a rescan"),
        TopologyNotification::Rescan(RescanReason::Resume)
    );

    // A resume racing daemon shutdown must warn, not panic: there is
    // nothing to recover when the daemon is already going away.
    drop(requested);
    reconcile_after_resume(&rescans);
}
