// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use super::*;

#[test]
fn identical_inputs_produce_identical_names() {
    let first = service_names("govee", "device-1/surface-a").expect("valid names");
    let second = service_names("govee", "device-1/surface-a").expect("valid names");

    assert_eq!(first, second);
}

#[test]
fn embedded_separators_do_not_collide_across_the_plugin_target_boundary() {
    // Without escaping, ("a/b", "c") and ("a", "b/c") would render to
    // the same joined path. Escaping the embedded `/` in each input
    // before joining must keep them distinct.
    let first = service_names("a/b", "c").expect("valid names");
    let second = service_names("a", "b/c").expect("valid names");

    assert_ne!(first, second);
}

#[test]
fn different_plugin_names_produce_different_service_names() {
    let first = service_names("govee", "device-1").expect("valid names");
    let second = service_names("wled", "device-1").expect("valid names");

    assert_ne!(first, second);
}

#[test]
fn different_targets_produce_different_service_names() {
    let first = service_names("govee", "device-1").expect("valid names");
    let second = service_names("govee", "device-2").expect("valid names");

    assert_ne!(first, second);
}

#[test]
fn publish_subscribe_and_event_names_differ() {
    let names = service_names("govee", "device-1").expect("valid names");

    assert_ne!(names.publish_subscribe, names.event);
}

#[test]
fn non_ascii_and_control_bytes_are_escaped_rather_than_rejected() {
    let names = service_names("plugin", "café\0/x").expect("valid names");

    assert!(names.publish_subscribe.as_str().is_ascii());
    assert!(!names.publish_subscribe.as_str().contains('\0'));
}

#[test]
fn empty_target_path_is_still_a_valid_distinct_name() {
    let names = service_names("plugin", "").expect("valid names");

    assert!(names.publish_subscribe.as_str().starts_with(NAMESPACE));
}

#[test]
fn pathologically_long_input_is_reported_rather_than_panicking_or_truncating() {
    let long_target = "x".repeat(4096);

    let error = service_names("plugin", &long_target).unwrap_err();

    assert_eq!(error, ServiceNameError::ExceedsMaximumLength);
}

#[test]
fn client_service_names_identical_inputs_produce_identical_names() {
    let first = client_service_names("device-1/surface-a").expect("valid names");
    let second = client_service_names("device-1/surface-a").expect("valid names");

    assert_eq!(first, second);
}

#[test]
fn client_service_names_different_targets_produce_different_names() {
    let first = client_service_names("device-1").expect("valid names");
    let second = client_service_names("device-2").expect("valid names");

    assert_ne!(first, second);
}

#[test]
fn client_service_names_never_collide_with_the_daemon_plugin_host_leg() {
    let client = client_service_names("device-1").expect("valid names");
    let daemon_plugin_host = service_names("plugin", "device-1").expect("valid names");

    assert_ne!(
        client.publish_subscribe,
        daemon_plugin_host.publish_subscribe
    );
    assert_ne!(client.event, daemon_plugin_host.event);
}

#[test]
fn client_service_names_publish_subscribe_and_event_names_differ() {
    let names = client_service_names("device-1").expect("valid names");

    assert_ne!(names.publish_subscribe, names.event);
}
