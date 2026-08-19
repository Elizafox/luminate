// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Scene REST resources.

use super::IntoResponse as _;
use super::schemas::{OutcomeSchema, SceneSchema};
use super::{
    AuthenticatedClient, CaptureSceneRequest, CreateSceneRequest, ExpectedRevision, Json, Path,
    RecaptureSceneRequest, ReplaceSceneRequest, Response, SceneId, StatusCode, empty_response,
    error_response, json_response, outcome_response, problem_response,
};

#[utoipa::path(get, path = "/api/v0/scenes", responses((status = 200, body = Vec<SceneSchema>), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse)))]
pub(super) async fn list_scenes(session: AuthenticatedClient) -> Response {
    json_response(session.list_scenes().await)
}

#[utoipa::path(post, path = "/api/v0/scenes", request_body = CreateSceneRequest, responses((status = 201, body = SceneSchema), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse), (status = 422, body = crate::ProblemResponse)))]
pub(super) async fn create_scene(
    session: AuthenticatedClient,
    Json(p): Json<CreateSceneRequest>,
) -> Response {
    match session
        .create_scene(p.name, p.description, p.bindings)
        .await
    {
        Ok(value) => (StatusCode::CREATED, Json(value)).into_response(),
        Err(error) => error_response(&error),
    }
}

#[utoipa::path(post, path = "/api/v0/scenes/capture", request_body = CaptureSceneRequest, responses((status = 201, body = SceneSchema), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse), (status = 422, body = crate::ProblemResponse)))]
pub(super) async fn capture_scene(
    session: AuthenticatedClient,
    Json(p): Json<CaptureSceneRequest>,
) -> Response {
    match session
        .capture_scene(p.name, p.description, p.mode, p.targets)
        .await
    {
        Ok(value) => (StatusCode::CREATED, Json(value)).into_response(),
        Err(error) => error_response(&error),
    }
}

#[utoipa::path(get, path = "/api/v0/scenes/{id}", params(("id" = String, Path)), responses((status = 200, body = SceneSchema), (status = 404, body = crate::ProblemResponse), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse)))]
pub(super) async fn get_scene(session: AuthenticatedClient, Path(id): Path<String>) -> Response {
    match session.get_scene(SceneId::new(id)).await {
        Ok(Some(value)) => Json(value).into_response(),
        Ok(None) => problem_response(
            StatusCode::NOT_FOUND,
            "not-found",
            "scene not found",
            None,
            None,
        ),
        Err(error) => error_response(&error),
    }
}

#[utoipa::path(put, path = "/api/v0/scenes/{id}", params(("id" = String, Path)), request_body = ReplaceSceneRequest, responses((status = 200, body = SceneSchema), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse), (status = 409, body = crate::ProblemResponse), (status = 422, body = crate::ProblemResponse)))]
pub(super) async fn replace_scene(
    session: AuthenticatedClient,
    Path(id): Path<String>,
    Json(p): Json<ReplaceSceneRequest>,
) -> Response {
    json_response(
        session
            .replace_scene(
                SceneId::new(id),
                p.expected_revision,
                p.name,
                p.description,
                p.bindings,
            )
            .await,
    )
}

#[utoipa::path(post, path = "/api/v0/scenes/{id}/recapture", params(("id" = String, Path)), request_body = RecaptureSceneRequest, responses((status = 200, body = SceneSchema), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse), (status = 409, body = crate::ProblemResponse), (status = 422, body = crate::ProblemResponse)))]
pub(super) async fn recapture_scene(
    session: AuthenticatedClient,
    Path(id): Path<String>,
    Json(p): Json<RecaptureSceneRequest>,
) -> Response {
    json_response(
        session
            .recapture_scene(SceneId::new(id), p.expected_revision, p.mode, p.targets)
            .await,
    )
}

#[utoipa::path(delete, path = "/api/v0/scenes/{id}", params(("id" = String, Path)), request_body = ExpectedRevision, responses((status = 204), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse), (status = 404, body = crate::ProblemResponse), (status = 409, body = crate::ProblemResponse)))]
pub(super) async fn delete_scene(
    session: AuthenticatedClient,
    Path(id): Path<String>,
    Json(p): Json<ExpectedRevision>,
) -> Response {
    empty_response(
        session
            .delete_scene(SceneId::new(id), p.expected_revision)
            .await,
    )
}

#[utoipa::path(post, path = "/api/v0/scenes/{id}/apply", params(("id" = String, Path)), responses((status = 200, body = OutcomeSchema), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse), (status = 404, body = crate::ProblemResponse), (status = 422, body = crate::ProblemResponse)))]
pub(super) async fn apply_scene(session: AuthenticatedClient, Path(id): Path<String>) -> Response {
    outcome_response(session.apply_scene(SceneId::new(id)).await)
}
