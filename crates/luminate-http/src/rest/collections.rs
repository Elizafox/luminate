// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Collection REST resources.

use super::IntoResponse as _;
use super::schemas::{CollectionSchema, CollectionStateSchema, IdSchema};
use super::{
    AuthenticatedClient, CollectionId, CollectionIdResponse, CollectionMemberRequest,
    CreateCollectionRequest, Json, Path, Response, StatusCode, empty_response, error_response,
    json_response, problem_response,
};

#[utoipa::path(get, path = "/api/v0/collections", responses((status = 200, body = Vec<CollectionSchema>), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse)))]
pub(super) async fn list_collections(session: AuthenticatedClient) -> Response {
    json_response(session.list_collections().await)
}

#[utoipa::path(post, path = "/api/v0/collections", request_body = CreateCollectionRequest, responses((status = 201, body = IdSchema), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse), (status = 422, body = crate::ProblemResponse)))]
pub(super) async fn create_collection(
    session: AuthenticatedClient,
    Json(p): Json<CreateCollectionRequest>,
) -> Response {
    match session
        .create_collection(p.name, p.description, p.kind, p.members)
        .await
    {
        Ok(id) => (StatusCode::CREATED, Json(CollectionIdResponse { id })).into_response(),
        Err(error) => error_response(&error),
    }
}

#[utoipa::path(get, path = "/api/v0/collections/{id}", params(("id" = String, Path)), responses((status = 200, body = CollectionSchema), (status = 404, body = crate::ProblemResponse), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse)))]
pub(super) async fn get_collection(
    session: AuthenticatedClient,
    Path(id): Path<String>,
) -> Response {
    match session.get_collection(CollectionId::new(id)).await {
        Ok(Some(value)) => Json(value).into_response(),
        Ok(None) => problem_response(
            StatusCode::NOT_FOUND,
            "not-found",
            "collection not found",
            None,
            None,
        ),
        Err(error) => error_response(&error),
    }
}

#[utoipa::path(delete, path = "/api/v0/collections/{id}", params(("id" = String, Path)), responses((status = 204), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse), (status = 404, body = crate::ProblemResponse)))]
pub(super) async fn destroy_collection(
    session: AuthenticatedClient,
    Path(id): Path<String>,
) -> Response {
    empty_response(session.destroy_collection(CollectionId::new(id)).await)
}

#[utoipa::path(get, path = "/api/v0/collections/{id}/state", params(("id" = String, Path)), responses((status = 200, body = CollectionStateSchema), (status = 404, body = crate::ProblemResponse), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse)))]
pub(super) async fn collection_state(
    session: AuthenticatedClient,
    Path(id): Path<String>,
) -> Response {
    match session.get_collection_state(CollectionId::new(id)).await {
        Ok(Some(value)) => Json(value).into_response(),
        Ok(None) => problem_response(
            StatusCode::NOT_FOUND,
            "not-found",
            "collection state not found",
            None,
            None,
        ),
        Err(error) => error_response(&error),
    }
}

#[utoipa::path(post, path = "/api/v0/collections/{id}/members", params(("id" = String, Path)), request_body = CollectionMemberRequest, responses((status = 204), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse), (status = 422, body = crate::ProblemResponse)))]
pub(super) async fn add_member(
    session: AuthenticatedClient,
    Path(id): Path<String>,
    Json(p): Json<CollectionMemberRequest>,
) -> Response {
    empty_response(
        session
            .add_collection_member(CollectionId::new(id), p.member)
            .await,
    )
}

#[utoipa::path(delete, path = "/api/v0/collections/{id}/members", params(("id" = String, Path)), request_body = CollectionMemberRequest, responses((status = 204), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse), (status = 422, body = crate::ProblemResponse)))]
pub(super) async fn remove_member(
    session: AuthenticatedClient,
    Path(id): Path<String>,
    Json(p): Json<CollectionMemberRequest>,
) -> Response {
    empty_response(
        session
            .remove_collection_member(CollectionId::new(id), p.member)
            .await,
    )
}
