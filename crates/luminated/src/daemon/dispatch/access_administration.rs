// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Daemon policy and authentication administration.

use std::time::SystemTime;

use luminate_core::policy::{
    PolicyDocument, PolicyDocumentSource, PolicyError, PolicyRevision, PrincipalId,
};
use luminate_protocol::{ErrorCode, OperationError, Response, ResponseStatus};

use super::{DispatchContext, RequestAuthorizer};
use crate::authentication::EphemeralCapacityError;
use crate::authorization::Operation;

pub(super) async fn get_policy(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    let Some(administration) = ctx.access_administration else {
        return unavailable("access administration is unavailable");
    };
    match administration.policy.document().await {
        Ok(document) => Response {
            status: ResponseStatus::AccessPolicy(Box::new(document.source().clone())),
        },
        Err(error) => policy_error(&error),
    }
}

pub(super) async fn replace_policy(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    expected: PolicyRevision,
    replacement: PolicyDocumentSource,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    let Some(administration) = ctx.access_administration else {
        return unavailable("access administration is unavailable");
    };
    if let Some(response) = durable_intent(ctx, Operation::ManagePolicy) {
        return response;
    }
    let replacement = match PolicyDocument::new(replacement) {
        Ok(document) => document,
        Err(error) => return invalid(error.to_string()),
    };
    match administration.policy.replace(expected, replacement).await {
        Ok(document) => Response {
            status: ResponseStatus::AccessPolicy(Box::new(document.source().clone())),
        },
        Err(error) => policy_error(&error),
    }
}

pub(super) async fn create_token(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: String,
    subject: PrincipalId,
    expires_at: Option<SystemTime>,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    let Some(administration) = ctx.access_administration else {
        return unavailable("access administration is unavailable");
    };
    if let Some(response) = durable_intent(ctx, Operation::ManageAuthentication) {
        return response;
    }
    match administration
        .authentication
        .create_token(&id, subject, expires_at)
    {
        Ok((metadata, secret)) => Response {
            status: ResponseStatus::TokenCreated { metadata, secret },
        },
        Err(error) => invalid(error.to_string()),
    }
}

pub(super) async fn list_tokens(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    let Some(administration) = ctx.access_administration else {
        return unavailable("access administration is unavailable");
    };
    Response {
        status: ResponseStatus::Tokens(administration.authentication.list_tokens()),
    }
}

pub(super) async fn revoke_token(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: String,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    let Some(administration) = ctx.access_administration else {
        return unavailable("access administration is unavailable");
    };
    if let Some(response) = durable_intent(ctx, Operation::ManageAuthentication) {
        return response;
    }
    match administration.authentication.revoke_token(&id) {
        Ok(true) => Response {
            status: ResponseStatus::Ack,
        },
        Ok(false) => error(
            ErrorCode::NotFound,
            format!("token {id} was not found or was already revoked"),
        ),
        Err(error) => unavailable(error.to_string()),
    }
}

pub(super) async fn rotate_token(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    id: String,
    expires_at: Option<SystemTime>,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    let Some(administration) = ctx.access_administration else {
        return unavailable("access administration is unavailable");
    };
    if let Some(response) = durable_intent(ctx, Operation::ManageAuthentication) {
        return response;
    }
    match administration.authentication.rotate_token(&id, expires_at) {
        Ok((metadata, secret)) => Response {
            status: ResponseStatus::TokenCreated { metadata, secret },
        },
        Err(token_error) if token_error.to_string().contains("does not exist") => {
            error(ErrorCode::NotFound, token_error.to_string())
        }
        Err(error) => unavailable(error.to_string()),
    }
}

pub(super) async fn create_attestation(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    name: String,
    subject: PrincipalId,
    verified_groups: Vec<String>,
    expires_at: Option<SystemTime>,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    let Some(administration) = ctx.access_administration else {
        return unavailable("access administration is unavailable");
    };
    if !administration
        .frontend_actors
        .contains(&ctx.principal.rate_limit_key())
    {
        return error(
            ErrorCode::PermissionDenied,
            "front-end actor is not registered".to_owned(),
        );
    }
    if let Some(response) = durable_intent(ctx, Operation::AdministerFrontend) {
        return response;
    }
    match administration
        .authentication
        .create_attestation_with_frontend(
            name,
            ctx.principal.clone(),
            ctx.policy.session_principal(),
            ctx.policy.session_credential_id(),
            subject,
            verified_groups,
            expires_at,
        ) {
        Ok((metadata, secret)) => Response {
            status: ResponseStatus::AttestationCreated { metadata, secret },
        },
        Err(capacity_error)
            if capacity_error
                .downcast_ref::<EphemeralCapacityError>()
                .is_some() =>
        {
            error(ErrorCode::RateLimited, capacity_error.to_string())
        }
        Err(error) => invalid(error.to_string()),
    }
}

pub(super) async fn list_attestations(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    let Some(administration) = ctx.access_administration else {
        return unavailable("access administration is unavailable");
    };
    if !administration
        .frontend_actors
        .contains(&ctx.principal.rate_limit_key())
    {
        return error(
            ErrorCode::PermissionDenied,
            "front-end actor is not registered".to_owned(),
        );
    }
    Response {
        status: ResponseStatus::Attestations(
            administration
                .authentication
                .list_attestations(ctx.principal),
        ),
    }
}

pub(super) async fn revoke_attestation(
    ctx: &DispatchContext<'_>,
    auth: &RequestAuthorizer<'_>,
    name: String,
) -> Response {
    if let Err(response) = auth.authorize(&[]).await {
        return response;
    }
    let Some(administration) = ctx.access_administration else {
        return unavailable("access administration is unavailable");
    };
    if !administration
        .frontend_actors
        .contains(&ctx.principal.rate_limit_key())
    {
        return error(
            ErrorCode::PermissionDenied,
            "front-end actor is not registered".to_owned(),
        );
    }
    if let Some(response) = durable_intent(ctx, Operation::AdministerFrontend) {
        return response;
    }
    if administration
        .authentication
        .revoke_attestation(&name, ctx.principal)
    {
        Response {
            status: ResponseStatus::Ack,
        }
    } else {
        error(
            ErrorCode::NotFound,
            format!("attestation {name} was not found"),
        )
    }
}

fn durable_intent(ctx: &DispatchContext<'_>, operation: Operation) -> Option<Response> {
    let Some(administration) = ctx.access_administration else {
        return Some(unavailable("access administration is unavailable"));
    };
    match administration.audit.record(
        ctx.principal,
        operation,
        &[],
        true,
        Some("administrative mutation intent"),
    ) {
        Ok(()) => None,
        Err(error) if ctx.principal.is_intrinsic_recovery() => {
            tracing::warn!(error = %error, "break-glass administration continuing without durable audit");
            None
        }
        Err(error) => Some(unavailable(format!(
            "durable administration audit failed: {error}"
        ))),
    }
}

fn policy_error(policy_error: &PolicyError) -> Response {
    match policy_error {
        PolicyError::RevisionConflict { .. } => {
            error(ErrorCode::Conflict, policy_error.to_string())
        }
        PolicyError::Unavailable(_) => unavailable(policy_error.to_string()),
    }
}

fn invalid(message: String) -> Response {
    error(ErrorCode::InvalidArgument, message)
}

fn unavailable(message: impl Into<String>) -> Response {
    error(ErrorCode::Unavailable, message.into())
}

fn error(code: ErrorCode, message: String) -> Response {
    Response {
        status: ResponseStatus::Error(OperationError {
            code,
            message,
            retry_after_ms: None,
            applied_targets: Vec::new(),
        }),
    }
}
