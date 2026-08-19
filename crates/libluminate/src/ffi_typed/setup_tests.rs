// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

fn bytes(view: LuminateStringView) -> &'static [u8] {
    if view.data.is_null() {
        return &[];
    }
    // SAFETY: the test keeps the workflow list backing the view alive.
    unsafe { std::slice::from_raw_parts(view.data.cast(), view.len) }
}

fn assert_completed_session(completed: *const LuminatePluginSetupSession, revision: &mut u64) {
    // SAFETY: the caller keeps `completed` and `revision` live for every call.
    unsafe {
        assert_eq!(
            luminate_plugin_setup_session_state(completed),
            LUMINATE_PLUGIN_SETUP_COMPLETED
        );
        assert!(luminate_plugin_setup_session_revision(completed, revision));
        assert_eq!(*revision, 7);
        assert!(!luminate_plugin_setup_session_revision(
            completed,
            ptr::null_mut()
        ));
        assert_eq!(
            bytes(luminate_plugin_setup_session_message(completed)),
            b"Connected."
        );
    }
}

#[test]
fn workflow_list_accessors_are_borrowed_and_null_safe() {
    let list = Box::new(LuminatePluginSetupWorkflowList(vec![
        PluginSetupWorkflow::new(
            "example",
            "pair",
            "Pair hardware",
            "Connect nearby hardware.",
            PluginSetupWorkflowKind::Provision,
        ),
    ]));
    let list = Box::into_raw(list);

    // SAFETY: `list` remains live until the final free.
    unsafe {
        assert_eq!(luminate_plugin_setup_workflow_list_count(list), 1);
        let workflow = luminate_plugin_setup_workflow_list_at(list, 0);
        assert_eq!(
            bytes(luminate_plugin_setup_workflow_plugin(workflow)),
            b"example"
        );
        assert_eq!(bytes(luminate_plugin_setup_workflow_id(workflow)), b"pair");
        assert_eq!(
            bytes(luminate_plugin_setup_workflow_label(workflow)),
            b"Pair hardware"
        );
        assert_eq!(
            bytes(luminate_plugin_setup_workflow_description(workflow)),
            b"Connect nearby hardware."
        );
        assert_eq!(
            luminate_plugin_setup_workflow_kind(workflow),
            LUMINATE_PLUGIN_SETUP_PROVISION
        );
        assert!(luminate_plugin_setup_workflow_list_at(list, 1).is_null());

        assert_eq!(luminate_plugin_setup_workflow_list_count(ptr::null()), 0);
        assert!(luminate_plugin_setup_workflow_list_at(ptr::null(), 0).is_null());
        assert_eq!(luminate_plugin_setup_workflow_kind(ptr::null()), u32::MAX);
        assert!(
            luminate_plugin_setup_workflow_plugin(ptr::null())
                .data
                .is_null()
        );
        assert!(
            luminate_plugin_setup_workflow_description(ptr::null())
                .data
                .is_null()
        );

        luminate_plugin_setup_workflow_list_free(list);
        luminate_plugin_setup_workflow_list_free(ptr::null_mut());
    }
}

#[test]
fn session_accessors_cover_choice_applying_and_completion_states() {
    let id =
        PluginSetupSessionId::parse("0123456789abcdef0123456789abcdef").expect("valid session ID");
    let session = Box::into_raw(Box::new(LuminatePluginSetupSession(PluginSetupSession {
        id,
        plugin: "example".to_owned(),
        workflow: "pair".to_owned(),
        generation: 2,
        state: PluginSetupSessionState::Choice {
            prompt: "Choose hardware.".to_owned(),
            choices: vec![crate::PluginSetupChoice {
                id: "first".to_owned(),
                label: "First device".to_owned(),
                description: Some("Nearby".to_owned()),
            }],
        },
    })));
    let mut revision = 9;

    // SAFETY: `session` remains live until the final free.
    unsafe {
        assert_eq!(
            luminate_plugin_setup_session_state(session),
            LUMINATE_PLUGIN_SETUP_CHOICE
        );
        assert_eq!(luminate_plugin_setup_session_generation(session), 2);
        assert_eq!(
            bytes(luminate_plugin_setup_session_plugin(session)),
            b"example"
        );
        assert_eq!(
            bytes(luminate_plugin_setup_session_message(session)),
            b"Choose hardware."
        );
        assert_eq!(luminate_plugin_setup_session_choice_count(session), 1);
        let choice = luminate_plugin_setup_session_choice_at(session, 0);
        assert!(!choice.is_null());
        assert_eq!(bytes(luminate_plugin_setup_choice_id(choice)), b"first");
        assert_eq!(
            bytes(luminate_plugin_setup_choice_label(choice)),
            b"First device"
        );
        assert_eq!(
            bytes(luminate_plugin_setup_choice_description(choice)),
            b"Nearby"
        );
        assert!(!luminate_plugin_setup_session_revision(
            session,
            &raw mut revision
        ));
        assert_eq!(revision, 9);
        luminate_plugin_setup_session_free(session);
    }

    let applying = Box::into_raw(Box::new(LuminatePluginSetupSession(PluginSetupSession {
        id: PluginSetupSessionId::parse("11111111111111111111111111111111")
            .expect("valid session ID"),
        plugin: "example".to_owned(),
        workflow: "pair".to_owned(),
        generation: 3,
        state: PluginSetupSessionState::Applying,
    })));
    // SAFETY: `applying` remains live until the final free.
    unsafe {
        assert_eq!(
            luminate_plugin_setup_session_state(applying),
            LUMINATE_PLUGIN_SETUP_APPLYING
        );
        assert!(
            luminate_plugin_setup_session_message(applying)
                .data
                .is_null()
        );
        luminate_plugin_setup_session_free(applying);
    }

    let completed = Box::into_raw(Box::new(LuminatePluginSetupSession(PluginSetupSession {
        id: PluginSetupSessionId::parse("fedcba9876543210fedcba9876543210")
            .expect("valid session ID"),
        plugin: "example".to_owned(),
        workflow: "pair".to_owned(),
        generation: 3,
        state: PluginSetupSessionState::Completed {
            summary: "Connected.".to_owned(),
            revision: 7,
        },
    })));
    // SAFETY: `completed` remains live until the final free.
    unsafe {
        revision = u64::MAX;
        assert_completed_session(completed, &mut revision);
        luminate_plugin_setup_session_free(completed);
        luminate_plugin_setup_session_free(ptr::null_mut());
        assert!(
            luminate_plugin_setup_session_message(ptr::null())
                .data
                .is_null()
        );
        assert!(luminate_plugin_setup_session_id(ptr::null()).data.is_null());
        assert!(!luminate_plugin_setup_session_revision(
            ptr::null(),
            &raw mut revision
        ));
        assert_eq!(revision, 7);
    }
}
