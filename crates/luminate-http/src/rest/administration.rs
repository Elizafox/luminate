// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Management and policy administration REST resources.

use super::IntoResponse as _;
use super::schemas::{
    ManagementChangeSetSchema, ManagementPatchSchema, ManagementSnapshotSchema,
    PolicyDocumentSchema, ReplacePolicySchema,
};
use super::{
    AuthenticatedClient, Json, ManagementPatch, PolicyDocument, ReplacePolicyRequest, Response,
    StatusCode, error_response, json_response, problem_response,
};

#[utoipa::path(get, path = "/api/v0/management", responses((status = 200, body = ManagementSnapshotSchema), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse)))]
pub(super) async fn management(session: AuthenticatedClient) -> Response {
    json_response(session.get_management().await)
}

#[utoipa::path(patch, path = "/api/v0/management", request_body = ManagementPatchSchema, responses((status = 200, body = ManagementChangeSetSchema), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse), (status = 409, body = crate::ProblemResponse), (status = 422, body = crate::ProblemResponse)))]
pub(super) async fn patch_management(
    session: AuthenticatedClient,
    Json(p): Json<ManagementPatch>,
) -> Response {
    json_response(session.patch_management(p).await)
}

#[utoipa::path(get, path = "/api/v0/policy", responses((status = 200, body = PolicyDocumentSchema), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse)))]
pub(super) async fn policy(session: AuthenticatedClient) -> Response {
    match session.policy_administration().get().await {
        Ok(document) => Json(document.source().clone()).into_response(),
        Err(error) => error_response(&error),
    }
}

#[utoipa::path(put, path = "/api/v0/policy", request_body = ReplacePolicySchema, responses((status = 200, body = PolicyDocumentSchema), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse), (status = 409, body = crate::ProblemResponse), (status = 422, body = crate::ProblemResponse)))]
pub(super) async fn replace_policy(
    session: AuthenticatedClient,
    Json(p): Json<ReplacePolicyRequest>,
) -> Response {
    let document = match PolicyDocument::new(p.document) {
        Ok(value) => value,
        Err(error) => {
            return problem_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid-policy",
                &error.to_string(),
                None,
                None,
            );
        }
    };
    match session
        .policy_administration()
        .replace(p.expected_revision, document)
        .await
    {
        Ok(document) => Json(document.source().clone()).into_response(),
        Err(error) => error_response(&error),
    }
}
