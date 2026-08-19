// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Device discovery and state REST resources.

use super::IntoResponse as _;
use super::schemas::{DeviceSchema, DeviceStateSchema};
use super::{
    AuthenticatedClient, DeviceId, Json, Path, Response, StatusCode, empty_response,
    error_response, json_response, problem_response,
};

#[utoipa::path(get, path = "/api/v0/devices", responses(
    (status = 200, body = Vec<DeviceSchema>), (status = 401, body = crate::ProblemResponse),
    (status = 403, body = crate::ProblemResponse), (status = 503, body = crate::ProblemResponse)
))]
pub(super) async fn list_devices(session: AuthenticatedClient) -> Response {
    json_response(session.list_devices().await)
}

#[utoipa::path(get, path = "/api/v0/devices/{id}", params(("id" = String, Path)), responses(
    (status = 200, body = DeviceSchema), (status = 404, body = crate::ProblemResponse),
    (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse)
))]
pub(super) async fn get_device(session: AuthenticatedClient, Path(id): Path<String>) -> Response {
    match session.get_device(DeviceId::new(id)).await {
        Ok(Some(value)) => Json(value).into_response(),
        Ok(None) => problem_response(
            StatusCode::NOT_FOUND,
            "not-found",
            "device not found",
            None,
            None,
        ),
        Err(error) => error_response(&error),
    }
}

#[utoipa::path(get, path = "/api/v0/devices/withdrawn", responses(
    (status = 200, body = Vec<String>), (status = 401, body = crate::ProblemResponse),
    (status = 403, body = crate::ProblemResponse)
))]
pub(super) async fn withdrawn(session: AuthenticatedClient) -> Response {
    json_response(session.list_withdrawn_devices().await)
}

#[utoipa::path(get, path = "/api/v0/devices/{id}/state", params(("id" = String, Path)), responses(
    (status = 200, body = DeviceStateSchema), (status = 404, body = crate::ProblemResponse),
    (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse)
))]
pub(super) async fn device_state(session: AuthenticatedClient, Path(id): Path<String>) -> Response {
    match session.get_state(DeviceId::new(id)).await {
        Ok(Some(value)) => Json(value).into_response(),
        Ok(None) => problem_response(
            StatusCode::NOT_FOUND,
            "not-found",
            "device state not found",
            None,
            None,
        ),
        Err(error) => error_response(&error),
    }
}

#[utoipa::path(post, path = "/api/v0/devices/{id}/refresh", params(("id" = String, Path)), responses(
    (status = 204), (status = 401, body = crate::ProblemResponse),
    (status = 403, body = crate::ProblemResponse), (status = 422, body = crate::ProblemResponse)
))]
pub(super) async fn refresh(session: AuthenticatedClient, Path(id): Path<String>) -> Response {
    empty_response(session.refresh_state(DeviceId::new(id)).await)
}

#[utoipa::path(delete, path = "/api/v0/devices/{id}/purge", params(("id" = String, Path)), responses(
    (status = 204), (status = 401, body = crate::ProblemResponse),
    (status = 403, body = crate::ProblemResponse), (status = 404, body = crate::ProblemResponse)
))]
pub(super) async fn purge(session: AuthenticatedClient, Path(id): Path<String>) -> Response {
    empty_response(session.purge_withdrawn_device(DeviceId::new(id)).await)
}

#[utoipa::path(post, path = "/api/v0/rescan", responses(
    (status = 204), (status = 401, body = crate::ProblemResponse),
    (status = 403, body = crate::ProblemResponse), (status = 503, body = crate::ProblemResponse)
))]
pub(super) async fn rescan(session: AuthenticatedClient) -> Response {
    empty_response(session.rescan().await)
}
