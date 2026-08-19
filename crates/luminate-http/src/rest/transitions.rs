// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Transition lifecycle REST resources.

use super::IntoResponse as _;
use super::schemas::{IdSchema, TransitionStatusSchema};
use super::{
    AppState, AuthenticatedClient, Json, Path, Response, StartTransitionRequest, State, StatusCode,
    StoredTransition, TransitionId, TransitionIdResponse, error_response, json_response,
    problem_response,
};

#[utoipa::path(post, path = "/api/v0/transitions", request_body = StartTransitionRequest, responses((status = 201, body = IdSchema), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse), (status = 409, body = crate::ProblemResponse), (status = 422, body = crate::ProblemResponse)))]
pub(super) async fn start_transition(
    State(s): State<AppState>,
    session: AuthenticatedClient,
    Json(p): Json<StartTransitionRequest>,
) -> Response {
    let result = match p {
        StartTransitionRequest::SceneToScene {
            scene,
            destination,
            options,
        } => {
            session
                .transitions()
                .scene_to_scene(scene, destination, options)
                .await
        }
        StartTransitionRequest::CurrentToScene {
            destination,
            options,
        } => {
            session
                .transitions()
                .current_to_scene(destination, options)
                .await
        }
        StartTransitionRequest::SceneToStates {
            scene,
            states,
            options,
        } => {
            session
                .transitions()
                .scene_to_states(scene, states, options)
                .await
        }
        StartTransitionRequest::CurrentToStates { states, options } => {
            session
                .transitions()
                .current_to_states(states, options)
                .await
        }
    };
    match result {
        Ok(transition) => {
            let id = transition.id.clone();
            let principal = session.session().subject.clone();
            s.transitions.lock().await.insert(
                id.clone(),
                StoredTransition {
                    principal,
                    id: id.clone(),
                },
            );
            (StatusCode::CREATED, Json(TransitionIdResponse { id })).into_response()
        }
        Err(error) => error_response(&error),
    }
}

#[utoipa::path(get, path = "/api/v0/transitions/{id}", params(("id" = String, Path)), responses((status = 200, body = TransitionStatusSchema), (status = 404, body = crate::ProblemResponse), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse)))]
pub(super) async fn transition_status(
    State(s): State<AppState>,
    session: AuthenticatedClient,
    Path(id): Path<String>,
) -> Response {
    let id = TransitionId::new(id);
    let stored = s.transitions.lock().await.get(&id).cloned();
    let Some(stored) = stored else {
        return problem_response(
            StatusCode::NOT_FOUND,
            "not-found",
            "transition not found",
            None,
            None,
        );
    };
    if stored.principal != session.session().subject {
        return problem_response(
            StatusCode::NOT_FOUND,
            "not-found",
            "transition not found",
            None,
            None,
        );
    }
    json_response(session.transitions().get(stored.id.clone()).await)
}

#[utoipa::path(delete, path = "/api/v0/transitions/{id}", params(("id" = String, Path)), responses((status = 200, body = TransitionStatusSchema), (status = 404, body = crate::ProblemResponse), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse)))]
pub(super) async fn abort_transition(
    State(s): State<AppState>,
    session: AuthenticatedClient,
    Path(id): Path<String>,
) -> Response {
    let id = TransitionId::new(id);
    let mut transitions = s.transitions.lock().await;
    if transitions
        .get(&id)
        .is_some_and(|stored| stored.principal != session.session().subject)
    {
        return problem_response(
            StatusCode::NOT_FOUND,
            "not-found",
            "transition not found",
            None,
            None,
        );
    }
    let Some(stored) = transitions.remove(&id) else {
        return problem_response(
            StatusCode::NOT_FOUND,
            "not-found",
            "transition not found",
            None,
            None,
        );
    };
    drop(transitions);
    json_response(session.transitions().abort(stored.id).await)
}

#[utoipa::path(post, path = "/api/v0/transitions/{id}/wait", params(("id" = String, Path)), responses((status = 200, body = TransitionStatusSchema), (status = 404, body = crate::ProblemResponse), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse)))]
pub(super) async fn wait_transition(
    State(s): State<AppState>,
    session: AuthenticatedClient,
    Path(id): Path<String>,
) -> Response {
    let id = TransitionId::new(id);
    let mut transitions = s.transitions.lock().await;
    if transitions
        .get(&id)
        .is_some_and(|stored| stored.principal != session.session().subject)
    {
        return problem_response(
            StatusCode::NOT_FOUND,
            "not-found",
            "transition not found",
            None,
            None,
        );
    }
    let Some(stored) = transitions.remove(&id) else {
        return problem_response(
            StatusCode::NOT_FOUND,
            "not-found",
            "transition not found",
            None,
            None,
        );
    };
    drop(transitions);
    json_response(session.transitions().wait(stored.id).await)
}
