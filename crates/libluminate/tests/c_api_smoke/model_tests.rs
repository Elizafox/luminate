// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

declare_c_test!(
    c_consumer_uses_generated_header_against_mock_daemon,
    include_str!("../c/basic.c")
);
declare_c_test!(
    c_consumer_integrates_synchronous_api,
    include_str!("../c/sync_api_integration.c")
);
declare_c_test!(
    documented_c_topology_example_runs_against_mock_daemon,
    include_str!("../../examples/topology.c")
);
declare_c_test!(
    documented_c_async_example_runs_against_mock_daemon,
    include_str!("../../examples/async.c")
);
declare_c_test!(
    c_consumer_subscribes_and_owns_typed_event,
    include_str!("../c/events.c"),
    with_events
);
declare_c_test!(
    c_consumer_exercises_target_shapes,
    include_str!("../c/targets.c")
);
declare_c_test!(
    c_consumer_exercises_frame_stream_lifecycle,
    include_str!("../c/frame_stream.c")
);

#[test]
fn c_consumer_exercises_collection_crud_and_models() {
    let socket_path = unique_path("collection-model-sock");
    let server = spawn_collection_model_daemon(&socket_path);
    run_c_consumer(
        "collections",
        include_str!("../c/collections.c"),
        &[socket_path.as_os_str()],
    );
    server
        .join()
        .expect("collection model daemon should not panic");
    let _ = fs::remove_file(socket_path);
}

declare_c_test!(
    c_consumer_exercises_management_and_parity_surface,
    include_str!("../c/management.c")
);
declare_c_test!(
    c_consumer_exercises_plugin_setup_workflow_discovery,
    include_str!("../c/setup.c")
);
declare_c_test!(
    c_consumer_exercises_scene_and_transition_validation,
    include_str!("../c/scenes.c"),
    without_daemon
);
declare_c_test!(
    c_consumer_uses_explicit_event_path,
    include_str!("../c/explicit_events.c"),
    with_explicit_events
);

#[test]
fn c_consumer_builds_and_evaluates_policy_models() {
    run_c_consumer("policy_model", include_str!("../c/policy_model.c"), &[]);
}

#[test]
fn c_consumer_walks_complete_typed_models_and_effects() {
    let socket_path = unique_path("typed-models-sock");
    let server = spawn_typed_model_daemon(&socket_path);
    run_c_consumer(
        "typed_models",
        include_str!("../c/typed_models.c"),
        &[socket_path.as_os_str()],
    );
    server.join().expect("typed model daemon should not panic");
    let _ = fs::remove_file(socket_path);
}
