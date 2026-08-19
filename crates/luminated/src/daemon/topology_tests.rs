// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::super::tests_support::{
    empty_plugin_manager, remove_request_test_dir, request_test_context, test_management_read_state,
};
use super::super::*;
use super::*;

fn observed(plugin_name: &str) -> TopologyNotification {
    TopologyNotification::PluginObserved(plugin_name.to_owned())
}

#[tokio::test]
async fn topology_batch_deduplicates_sorts_and_stops_on_closed_channel() {
    let manager = empty_plugin_manager();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    sender.send(observed("zeta")).expect("send first");
    sender.send(observed("alpha")).expect("send second");
    sender.send(observed("zeta")).expect("send duplicate");
    drop(sender);
    let first = receiver.recv().await.expect("receive first");
    assert_eq!(
        collect_topology_batch(first, &mut receiver, &manager).await,
        (vec!["alpha".to_owned(), "zeta".to_owned()], None),
        "an observation-only batch must not request a plugin-side rescan"
    );
}

#[test]
fn a_rescan_widens_the_batch_to_every_loaded_plugin_and_absorbs_observations() {
    // A resume produces a rescan and a flurry of plugin-observed
    // changes as hardware renumerates. The batch must be their union,
    // deduplicated, so no plugin is re-pulled twice for one wake-up.
    let observed = HashSet::from(["zeta".to_owned(), "alpha".to_owned()]);
    let loaded = vec!["alpha".to_owned(), "beta".to_owned()];
    assert_eq!(
        merge_rescan_batch(observed.clone(), Some(loaded)),
        ["alpha".to_owned(), "beta".to_owned(), "zeta".to_owned()]
    );

    // Without a rescan, only what plugins actually reported is re-pulled;
    // an idle daemon must not re-enumerate every plugin on one plugin's
    // notification.
    assert_eq!(
        merge_rescan_batch(observed, None),
        ["alpha".to_owned(), "zeta".to_owned()]
    );

    // A rescan with nothing observed still covers every loaded plugin.
    assert_eq!(
        merge_rescan_batch(HashSet::new(), Some(vec!["only".to_owned()])),
        ["only".to_owned()]
    );
}

#[tokio::test]
async fn a_rescan_notification_reaches_the_batch_collector() {
    // Covers the wiring `merge_rescan_batch`'s own test cannot: that a
    // `Rescan` arriving on the channel is recognized as one rather than
    // being dropped. The test manager loads no plugins, so the widened
    // set is empty and only the absorbed observation survives.
    let manager = empty_plugin_manager();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    sender
        .send(TopologyNotification::Rescan(RescanReason::Resume))
        .expect("send rescan");
    sender.send(observed("late-observer")).expect("send late");
    drop(sender);

    let first = receiver.recv().await.expect("receive first");
    assert_eq!(
        collect_topology_batch(first, &mut receiver, &manager).await,
        (vec!["late-observer".to_owned()], Some(RescanReason::Resume)),
        "the reason must survive the batch so plugin caches get invalidated"
    );
}

#[tokio::test]
async fn topology_coordinator_reports_unknown_plugins_and_exits_on_close() {
    let (state, manager, mutations, runtime_dir) = request_test_context("topology-coordinator");
    let (sender, receiver) = mpsc::unbounded_channel();
    let (events, _) = EventPublisher::test_channel(&state, 2);
    let task = tokio::spawn(run_topology_coordinator(
        receiver,
        mutations,
        manager,
        test_management_read_state(),
        events,
    ));

    sender
        .send(observed("not-loaded"))
        .expect("send unknown plugin notification");
    drop(sender);
    timeout(Duration::from_secs(1), task)
        .await
        .expect("coordinator should observe closed channel")
        .expect("coordinator task should not panic");
    drop(state);
    remove_request_test_dir(&runtime_dir);
}

#[test]
fn a_rescan_reconciles_every_owned_device_even_with_an_unchanged_topology() {
    // The Alienware case: a keyboard with a fixed topology comes back from
    // suspend dark but enumerating identically. Gating reconciliation on a
    // topology diff would skip exactly the device the resume exists to
    // restore.
    let keyboard = DeviceId::new("alienware-keyboard");
    let panel = DeviceId::new("alienware-aw-elc");
    let owned = [keyboard.clone(), panel.clone()];

    // Sorted, so the reconcile order is reproducible run to run.
    assert_eq!(
        devices_to_reconcile(&[], &owned, Some(RescanReason::Resume)),
        [panel.clone(), keyboard.clone()],
        "resume must reconcile owned devices even when nothing changed"
    );

    // A plugin noticing its own change and a broad Linux uevent both keep
    // narrow behaviour: neither may re-drive an unchanged device.
    assert!(
        devices_to_reconcile(&[], &owned, None).is_empty(),
        "an observation with no change must reconcile nothing"
    );
    assert!(
        devices_to_reconcile(&[], &owned, Some(RescanReason::DeviceChange)).is_empty(),
        "an unrelated device change must not reconcile owned devices"
    );
    assert_eq!(
        devices_to_reconcile(slice::from_ref(&panel), &owned, None),
        *slice::from_ref(&panel)
    );

    // A departed device is still worth a pass, and must not be listed
    // twice when it is also still owned.
    let departed = DeviceId::new("aa-unplugged");
    let union = devices_to_reconcile(
        &[departed.clone(), panel.clone()],
        &owned,
        Some(RescanReason::Operator),
    );
    assert_eq!(union, [departed, panel, keyboard]);
}

#[test]
fn resume_wins_over_device_changes_in_one_debounced_batch() {
    assert_eq!(
        merge_rescan_reason(Some(RescanReason::DeviceChange), RescanReason::Resume),
        RescanReason::Resume
    );
    assert_eq!(
        merge_rescan_reason(Some(RescanReason::Resume), RescanReason::DeviceChange),
        RescanReason::Resume
    );
    assert_eq!(
        merge_rescan_reason(Some(RescanReason::DeviceChange), RescanReason::Operator),
        RescanReason::Operator
    );
}

#[test]
fn topology_changes_publish_topology_then_state_events() {
    let keyboard = DeviceId::new("keyboard");
    let state = Arc::new(Mutex::new(DaemonState::default()));
    let (events, mut receiver) = EventPublisher::test_channel(&state, 4);
    publish_topology_changes(&events, vec![keyboard.clone()], vec![keyboard.clone()]);
    assert!(matches!(
        receiver.try_recv(),
        Ok(PublishedEvent { event: Event::TopologyChanged { devices }, .. }) if devices == [keyboard.clone()]
    ));
    assert!(matches!(
        receiver.try_recv(),
        Ok(PublishedEvent { event: Event::StateChanged { devices }, .. }) if devices == [keyboard.clone()]
    ));

    // A rescan that reconciled an unchanged device reports state only:
    // announcing a topology change would send every client re-fetching a
    // byte-identical device list.
    publish_topology_changes(&events, Vec::new(), vec![keyboard.clone()]);
    assert!(matches!(
        receiver.try_recv(),
        Ok(PublishedEvent { event: Event::StateChanged { devices }, .. }) if devices == [keyboard]
    ));
    assert!(matches!(
        receiver.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));

    publish_topology_changes(&events, Vec::new(), Vec::new());
    assert!(matches!(
        receiver.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}
