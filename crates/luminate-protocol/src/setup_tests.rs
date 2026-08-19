// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for plugin setup workflow wire models.

use crate::{
    PluginSetupInteractionResponse, PluginSetupSession, PluginSetupSessionId,
    PluginSetupSessionState, PluginSetupWorkflow, PluginSetupWorkflowKind, Request, ResponseStatus,
};

#[test]
fn workflow_request_and_response_round_trip() {
    let request = Request::ListPluginSetupWorkflows {
        plugin: "example".to_owned(),
    };
    let mut request_bytes = Vec::new();
    ciborium::into_writer(&request, &mut request_bytes).expect("serialize setup request");
    let decoded_request: Request =
        ciborium::from_reader(request_bytes.as_slice()).expect("deserialize setup request");
    assert!(matches!(
        decoded_request,
        Request::ListPluginSetupWorkflows { plugin } if plugin == "example"
    ));

    let workflow = PluginSetupWorkflow {
        plugin: "example".to_owned(),
        id: "pair".to_owned(),
        label: "Pair hardware".to_owned(),
        description: "Connect nearby hardware.".to_owned(),
        kind: PluginSetupWorkflowKind::Provision,
    };
    let response = ResponseStatus::PluginSetupWorkflows(vec![workflow.clone()]);
    let mut response_bytes = Vec::new();
    ciborium::into_writer(&response, &mut response_bytes).expect("serialize setup response");
    let decoded_response: ResponseStatus =
        ciborium::from_reader(response_bytes.as_slice()).expect("deserialize setup response");
    assert!(matches!(
        decoded_response,
        ResponseStatus::PluginSetupWorkflows(workflows) if workflows == vec![workflow]
    ));
}

#[test]
fn interactive_session_requests_and_snapshots_round_trip() {
    let session =
        PluginSetupSessionId::parse("0123456789abcdef0123456789abcdef").expect("valid session ID");
    let request = Request::RespondPluginSetup {
        session: session.clone(),
        generation: 4,
        response: PluginSetupInteractionResponse::Confirmed,
    };
    let mut bytes = Vec::new();
    ciborium::into_writer(&request, &mut bytes).expect("serialize session request");
    let decoded: Request =
        ciborium::from_reader(bytes.as_slice()).expect("deserialize session request");
    assert!(matches!(
        decoded,
        Request::RespondPluginSetup {
            session: decoded_session,
            generation: 4,
            response: PluginSetupInteractionResponse::Confirmed,
        } if decoded_session == session
    ));

    let response = ResponseStatus::PluginSetupSession(Box::new(PluginSetupSession {
        id: session,
        plugin: "example".to_owned(),
        workflow: "pair".to_owned(),
        generation: 5,
        state: PluginSetupSessionState::Completed {
            summary: "Connected hardware.".to_owned(),
            revision: 9,
        },
    }));
    let mut bytes = Vec::new();
    ciborium::into_writer(&response, &mut bytes).expect("serialize session response");
    let decoded: ResponseStatus =
        ciborium::from_reader(bytes.as_slice()).expect("deserialize session response");
    assert!(matches!(
        decoded,
        ResponseStatus::PluginSetupSession(session)
            if session.generation == 5
                && matches!(session.state, PluginSetupSessionState::Completed { revision: 9, .. })
    ));
}

#[test]
fn session_identifiers_reject_noncanonical_values() {
    assert!(PluginSetupSessionId::parse("ABCDEF0123456789ABCDEF0123456789").is_err());
    assert!(PluginSetupSessionId::parse("short").is_err());
}
