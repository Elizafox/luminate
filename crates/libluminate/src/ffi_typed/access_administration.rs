// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! C bindings for daemon policy and authentication administration.

#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "This module is an exported C boundary whose function contracts document the caller-owned pointers used by its small unsafe expressions."
)]

use super::policy::{LuminatePolicyDocument, null_view, read_str_array};
use super::*;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::ffi::{LuminateClient, client_ref};
use crate::{AttestationMetadata, Credential, TokenMetadata};
use luminate_core::policy::{PolicyRevision, PrincipalId};

#[cfg(test)]
#[path = "access_administration_tests.rs"]
mod tests;

pub struct LuminateTokenList(pub(crate) Vec<TokenMetadata>);

/// Borrowed token metadata within an owning token list.
pub struct LuminateToken;

pub struct LuminateCreatedToken {
    pub(crate) metadata: TokenMetadata,
    pub(crate) secret: Credential,
}

pub struct LuminateAttestationList(pub(crate) Vec<AttestationMetadata>);

/// Borrowed attestation metadata within an owning attestation list.
pub struct LuminateAttestation;

pub struct LuminateCreatedAttestation {
    pub(crate) metadata: AttestationMetadata,
    pub(crate) secret: Credential,
}

pub(crate) fn optional_expiry(
    has_expiry: bool,
    unix_ms: u64,
) -> Result<Option<SystemTime>, LuminateStatus> {
    if !has_expiry {
        return Ok(None);
    }
    UNIX_EPOCH
        .checked_add(Duration::from_millis(unix_ms))
        .map(Some)
        .ok_or_else(|| {
            crate::ffi::set_last_error("expiry timestamp is out of range");
            LuminateStatus::InvalidArgument
        })
}

null_safe_free!(
    "Releases a daemon token list. Null is a no-op.",
    luminate_token_list_free,
    LuminateTokenList
);
null_safe_free!(
    "Releases an attestation list. Null is a no-op.",
    luminate_attestation_list_free,
    LuminateAttestationList
);
null_safe_free!(
    "Releases a display-once created attestation and erases its secret. Null is a no-op.",
    luminate_created_attestation_free,
    LuminateCreatedAttestation
);

/// Creates an identity-only actor-bound attestation.
///
/// On success, ownership of `*out_attestation` transfers to the caller; free
/// it with `luminate_created_attestation_free`. The secret is displayed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_create_attestation(
    client: *const LuminateClient,
    name: *const c_char,
    authority: *const c_char,
    subject: *const c_char,
    has_expiry: bool,
    expires_at_unix_ms: u64,
    out_attestation: *mut *mut LuminateCreatedAttestation,
) -> LuminateStatus {
    unsafe {
        luminate_client_create_principal_attestation(
            client,
            name,
            authority,
            subject,
            ptr::null(),
            0,
            has_expiry,
            expires_at_unix_ms,
            out_attestation,
        )
    }
}

/// Creates a display-once attestation carrying verified principal groups.
///
/// Every string is deep-copied. On success, ownership of `*out_attestation`
/// transfers to the caller; free it with
/// `luminate_created_attestation_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_create_principal_attestation(
    client: *const LuminateClient,
    name: *const c_char,
    authority: *const c_char,
    subject: *const c_char,
    verified_groups: *const *const c_char,
    verified_group_count: usize,
    has_expiry: bool,
    expires_at_unix_ms: u64,
    out_attestation: *mut *mut LuminateCreatedAttestation,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client.cast_mut()) } {
            Ok(client) => client,
            Err(status) => return status,
        };
        if out_attestation.is_null() {
            crate::ffi::set_last_error("created attestation output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let name = match unsafe { read_required_str(name, "attestation name") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let authority = match unsafe { read_required_str(authority, "attestation authority") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let subject = match unsafe { read_required_str(subject, "attestation subject") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let subject = match PrincipalId::new(authority, subject) {
            Ok(value) => value,
            Err(error) => {
                crate::ffi::set_last_error(error.to_string());
                return LuminateStatus::InvalidArgument;
            }
        };
        let verified_groups = match unsafe {
            read_str_array(
                verified_groups,
                verified_group_count,
                "attestation verified groups",
            )
        } {
            Ok(groups) => groups,
            Err(status) => return status,
        };
        let expires_at = if has_expiry {
            let Some(expiry) = UNIX_EPOCH.checked_add(Duration::from_millis(expires_at_unix_ms))
            else {
                crate::ffi::set_last_error("attestation expiry is out of range");
                return LuminateStatus::InvalidArgument;
            };
            Some(expiry)
        } else {
            None
        };
        match call_client(client, move |client| async move {
            client
                .authentication_administration()
                .create_principal_attestation(name, subject, verified_groups, expires_at)
                .await
        }) {
            Ok(Ok(created)) => {
                unsafe {
                    *out_attestation = Box::into_raw(Box::new(LuminateCreatedAttestation {
                        metadata: created.metadata,
                        secret: created.secret,
                    }));
                }
                clear_last_error();
                LuminateStatus::Ok
            }
            Ok(Err(error)) => store_error(&error),
            Err(status) => status,
        }
    })
}

/// Lists attestations bound to the client's kernel actor.
///
/// On success, ownership of `*out_attestations` transfers to the caller; free
/// it with `luminate_attestation_list_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_list_attestations(
    client: *const LuminateClient,
    out_attestations: *mut *mut LuminateAttestationList,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client.cast_mut()) } {
            Ok(client) => client,
            Err(status) => return status,
        };
        if out_attestations.is_null() {
            crate::ffi::set_last_error("attestation list output pointer is null");
            return LuminateStatus::NullPointer;
        }
        match call_client(client, |client| async move {
            client
                .authentication_administration()
                .list_attestations()
                .await
        }) {
            Ok(Ok(records)) => {
                unsafe {
                    *out_attestations = Box::into_raw(Box::new(LuminateAttestationList(records)));
                }
                clear_last_error();
                LuminateStatus::Ok
            }
            Ok(Err(error)) => store_error(&error),
            Err(status) => status,
        }
    })
}

/// Revokes an actor-bound attestation by name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_revoke_attestation(
    client: *const LuminateClient,
    name: *const c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client.cast_mut()) } {
            Ok(client) => client,
            Err(status) => return status,
        };
        let name = match unsafe { read_required_str(name, "attestation name") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        match call_client(client, move |client| async move {
            client
                .authentication_administration()
                .revoke_attestation(name)
                .await
        }) {
            Ok(Ok(())) => {
                clear_last_error();
                LuminateStatus::Ok
            }
            Ok(Err(error)) => store_error(&error),
            Err(status) => status,
        }
    })
}
null_safe_free!(
    "Releases a display-once created token. Null is a no-op.",
    luminate_created_token_free,
    LuminateCreatedToken
);

/// Reads the active daemon access policy.
///
/// On success, ownership of `*out_policy` transfers to the caller; free it
/// with `luminate_policy_document_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_get_access_policy(
    client: *const LuminateClient,
    out_policy: *mut *mut LuminatePolicyDocument,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client.cast_mut()) } {
            Ok(client) => client,
            Err(status) => return status,
        };
        if out_policy.is_null() {
            crate::ffi::set_last_error("access policy output pointer is null");
            return LuminateStatus::NullPointer;
        }
        match call_client(client, |client| async move {
            client.policy_administration().get().await
        }) {
            Ok(Ok(document)) => {
                unsafe {
                    *out_policy =
                        Box::into_raw(Box::new(LuminatePolicyDocument::from_document(document)));
                }
                clear_last_error();
                LuminateStatus::Ok
            }
            Ok(Err(error)) => store_error(&error),
            Err(status) => status,
        }
    })
}

/// Revision-replaces the daemon access policy.
///
/// `replacement` remains caller-owned. On success, ownership of `*out_policy`
/// transfers to the caller; free it with `luminate_policy_document_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_replace_access_policy(
    client: *const LuminateClient,
    expected_revision: u64,
    replacement: *const LuminatePolicyDocument,
    out_policy: *mut *mut LuminatePolicyDocument,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client.cast_mut()) } {
            Ok(client) => client,
            Err(status) => return status,
        };
        let Some(replacement) = (unsafe { replacement.as_ref() }) else {
            crate::ffi::set_last_error("replacement access policy pointer is null");
            return LuminateStatus::NullPointer;
        };
        if out_policy.is_null() {
            crate::ffi::set_last_error("access policy output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let replacement = replacement.as_document().clone();
        match call_client(client, move |client| async move {
            client
                .policy_administration()
                .replace(PolicyRevision(expected_revision), replacement)
                .await
        }) {
            Ok(Ok(document)) => {
                unsafe {
                    *out_policy =
                        Box::into_raw(Box::new(LuminatePolicyDocument::from_document(document)));
                }
                clear_last_error();
                LuminateStatus::Ok
            }
            Ok(Err(error)) => store_error(&error),
            Err(status) => status,
        }
    })
}

/// Creates a daemon bearer token and returns its display-once secret.
///
/// On success, ownership of `*out_token` transfers to the caller; free it with
/// `luminate_created_token_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_create_token(
    client: *const LuminateClient,
    id: *const c_char,
    authority: *const c_char,
    subject: *const c_char,
    has_expiry: bool,
    expires_at_unix_ms: u64,
    out_token: *mut *mut LuminateCreatedToken,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client.cast_mut()) } {
            Ok(client) => client,
            Err(status) => return status,
        };
        if out_token.is_null() {
            crate::ffi::set_last_error("created token output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let id = match unsafe { read_required_str(id, "token ID") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let authority = match unsafe { read_required_str(authority, "token authority") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let subject = match unsafe { read_required_str(subject, "token subject") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let subject = match PrincipalId::new(authority, subject) {
            Ok(value) => value,
            Err(error) => {
                crate::ffi::set_last_error(error.to_string());
                return LuminateStatus::InvalidArgument;
            }
        };
        let expires_at = match optional_expiry(has_expiry, expires_at_unix_ms) {
            Ok(expiry) => expiry,
            Err(status) => return status,
        };
        match call_client(client, move |client| async move {
            client
                .authentication_administration()
                .create_token(id, subject, expires_at)
                .await
        }) {
            Ok(Ok(created)) => {
                unsafe {
                    *out_token = Box::into_raw(Box::new(LuminateCreatedToken {
                        metadata: created.metadata,
                        secret: created.secret,
                    }));
                }
                clear_last_error();
                LuminateStatus::Ok
            }
            Ok(Err(error)) => store_error(&error),
            Err(status) => status,
        }
    })
}

/// Lists sanitized daemon bearer-token metadata.
///
/// On success, ownership of `*out_tokens` transfers to the caller; free it
/// with `luminate_token_list_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_list_tokens(
    client: *const LuminateClient,
    out_tokens: *mut *mut LuminateTokenList,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client.cast_mut()) } {
            Ok(client) => client,
            Err(status) => return status,
        };
        if out_tokens.is_null() {
            crate::ffi::set_last_error("token list output pointer is null");
            return LuminateStatus::NullPointer;
        }
        match call_client(client, |client| async move {
            client.authentication_administration().list_tokens().await
        }) {
            Ok(Ok(tokens)) => {
                unsafe { *out_tokens = Box::into_raw(Box::new(LuminateTokenList(tokens))) };
                clear_last_error();
                LuminateStatus::Ok
            }
            Ok(Err(error)) => store_error(&error),
            Err(status) => status,
        }
    })
}

/// Revokes a daemon bearer token by identifier.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_revoke_token(
    client: *const LuminateClient,
    id: *const c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client.cast_mut()) } {
            Ok(client) => client,
            Err(status) => return status,
        };
        let id = match unsafe { read_required_str(id, "token ID") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        match call_client(client, move |client| async move {
            client
                .authentication_administration()
                .revoke_token(id)
                .await
        }) {
            Ok(Ok(())) => {
                clear_last_error();
                LuminateStatus::Ok
            }
            Ok(Err(error)) => store_error(&error),
            Err(status) => status,
        }
    })
}

/// Rotates a daemon bearer token and returns its replacement secret once.
///
/// On success, ownership of `*out_token` transfers to the caller; free it with
/// `luminate_created_token_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_rotate_token(
    client: *const LuminateClient,
    id: *const c_char,
    has_expiry: bool,
    expires_at_unix_ms: u64,
    out_token: *mut *mut LuminateCreatedToken,
) -> LuminateStatus {
    ffi_guard(|| {
        let client = match unsafe { client_ref(client.cast_mut()) } {
            Ok(client) => client,
            Err(status) => return status,
        };
        if out_token.is_null() {
            crate::ffi::set_last_error("rotated token output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let id = match unsafe { read_required_str(id, "token ID") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        let expires_at = match optional_expiry(has_expiry, expires_at_unix_ms) {
            Ok(expiry) => expiry,
            Err(status) => return status,
        };
        match call_client(client, move |client| async move {
            client
                .authentication_administration()
                .rotate_token(id, expires_at)
                .await
        }) {
            Ok(Ok(created)) => {
                unsafe {
                    *out_token = Box::into_raw(Box::new(LuminateCreatedToken {
                        metadata: created.metadata,
                        secret: created.secret,
                    }));
                }
                clear_last_error();
                LuminateStatus::Ok
            }
            Ok(Err(error)) => store_error(&error),
            Err(status) => status,
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_created_token_secret(
    token: *const LuminateCreatedToken,
    buffer: *mut u8,
    buffer_len: usize,
) -> usize {
    let Some(token) = (unsafe { token.as_ref() }) else {
        return 0;
    };
    let secret = token.secret.expose();
    if !buffer.is_null() && buffer_len != 0 {
        let length = secret.len().min(buffer_len);
        unsafe { ptr::copy_nonoverlapping(secret.as_ptr(), buffer, length) };
    }
    secret.len()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_created_token_id(
    token: *const LuminateCreatedToken,
) -> LuminateStringView {
    unsafe { token.as_ref() }.map_or_else(null_view, |token| sv(&token.metadata.id))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_created_token_authority(
    token: *const LuminateCreatedToken,
) -> LuminateStringView {
    unsafe { token.as_ref() }.map_or_else(null_view, |token| sv(token.metadata.subject.authority()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_created_token_subject(
    token: *const LuminateCreatedToken,
) -> LuminateStringView {
    unsafe { token.as_ref() }.map_or_else(null_view, |token| sv(token.metadata.subject.subject()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_token_list_count(tokens: *const LuminateTokenList) -> usize {
    unsafe { tokens.as_ref() }.map_or(0, |tokens| tokens.0.len())
}

fn token_ref(token: *const LuminateToken) -> Option<&'static TokenMetadata> {
    // SAFETY: the C contract requires a view returned by `luminate_token_list_at`.
    unsafe { token.cast::<TokenMetadata>().as_ref() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_token_list_at(
    tokens: *const LuminateTokenList,
    index: usize,
) -> *const LuminateToken {
    unsafe { tokens.as_ref() }
        .and_then(|tokens| tokens.0.get(index))
        .map_or(ptr::null(), cast_ref)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_token_id(token: *const LuminateToken) -> LuminateStringView {
    token_ref(token).map_or_else(null_view, |token| sv(&token.id))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_token_authority(
    token: *const LuminateToken,
) -> LuminateStringView {
    token_ref(token).map_or_else(null_view, |token| sv(token.subject.authority()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_token_subject(token: *const LuminateToken) -> LuminateStringView {
    token_ref(token).map_or_else(null_view, |token| sv(token.subject.subject()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_token_revoked(token: *const LuminateToken) -> bool {
    token_ref(token).is_some_and(|token| token.revoked)
}

fn expiry_unix_ms(metadata: &TokenMetadata) -> Option<u64> {
    system_time_unix_ms(metadata.expires_at?)
}

fn system_time_unix_ms(expiry: SystemTime) -> Option<u64> {
    let duration = expiry.duration_since(UNIX_EPOCH).ok()?;
    u64::try_from(duration.as_millis()).ok()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_created_token_expires_at_unix_ms(
    token: *const LuminateCreatedToken,
    out_expiry: *mut u64,
) -> bool {
    let (Some(token), Some(out_expiry)) =
        (unsafe { token.as_ref() }, unsafe { out_expiry.as_mut() })
    else {
        return false;
    };
    let Some(expiry) = expiry_unix_ms(&token.metadata) else {
        return false;
    };
    *out_expiry = expiry;
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_token_expires_at_unix_ms(
    token: *const LuminateToken,
    out_expiry: *mut u64,
) -> bool {
    let (Some(token), Some(out_expiry)) = (token_ref(token), unsafe { out_expiry.as_mut() }) else {
        return false;
    };
    let Some(expiry) = expiry_unix_ms(token) else {
        return false;
    };
    *out_expiry = expiry;
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_created_attestation_secret(
    attestation: *const LuminateCreatedAttestation,
    buffer: *mut u8,
    buffer_len: usize,
) -> usize {
    let Some(attestation) = (unsafe { attestation.as_ref() }) else {
        return 0;
    };
    let secret = attestation.secret.expose();
    if !buffer.is_null() && buffer_len != 0 {
        let length = secret.len().min(buffer_len);
        unsafe { ptr::copy_nonoverlapping(secret.as_ptr(), buffer, length) };
    }
    secret.len()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_created_attestation_name(
    attestation: *const LuminateCreatedAttestation,
) -> LuminateStringView {
    unsafe { attestation.as_ref() }
        .map_or_else(null_view, |attestation| sv(&attestation.metadata.name))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_created_attestation_authority(
    attestation: *const LuminateCreatedAttestation,
) -> LuminateStringView {
    unsafe { attestation.as_ref() }.map_or_else(null_view, |attestation| {
        sv(attestation.metadata.subject.authority())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_created_attestation_subject(
    attestation: *const LuminateCreatedAttestation,
) -> LuminateStringView {
    unsafe { attestation.as_ref() }.map_or_else(null_view, |attestation| {
        sv(attestation.metadata.subject.subject())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_created_attestation_group_count(
    attestation: *const LuminateCreatedAttestation,
) -> usize {
    unsafe { attestation.as_ref() }
        .map_or(0, |attestation| attestation.metadata.verified_groups.len())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_created_attestation_group_at(
    attestation: *const LuminateCreatedAttestation,
    index: usize,
) -> LuminateStringView {
    unsafe { attestation.as_ref() }
        .and_then(|attestation| attestation.metadata.verified_groups.get(index))
        .map_or_else(null_view, |group| sv(group))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_created_attestation_credential_id(
    attestation: *const LuminateCreatedAttestation,
) -> LuminateStringView {
    unsafe { attestation.as_ref() }.map_or_else(null_view, |attestation| {
        sv(&attestation.metadata.credential_id)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_created_attestation_expires_at_unix_ms(
    attestation: *const LuminateCreatedAttestation,
    out_expiry: *mut u64,
) -> bool {
    let (Some(attestation), Some(out_expiry)) = (unsafe { attestation.as_ref() }, unsafe {
        out_expiry.as_mut()
    }) else {
        return false;
    };
    let Some(expiry) = attestation
        .metadata
        .expires_at
        .and_then(system_time_unix_ms)
    else {
        return false;
    };
    *out_expiry = expiry;
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_attestation_list_count(
    attestations: *const LuminateAttestationList,
) -> usize {
    unsafe { attestations.as_ref() }.map_or(0, |records| records.0.len())
}

fn attestation_ref(
    attestation: *const LuminateAttestation,
) -> Option<&'static AttestationMetadata> {
    // SAFETY: the C contract requires a view returned by `luminate_attestation_list_at`.
    unsafe { attestation.cast::<AttestationMetadata>().as_ref() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_attestation_list_at(
    attestations: *const LuminateAttestationList,
    index: usize,
) -> *const LuminateAttestation {
    unsafe { attestations.as_ref() }
        .and_then(|attestations| attestations.0.get(index))
        .map_or(ptr::null(), cast_ref)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_attestation_name(
    attestation: *const LuminateAttestation,
) -> LuminateStringView {
    attestation_ref(attestation).map_or_else(null_view, |record| sv(&record.name))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_attestation_authority(
    attestation: *const LuminateAttestation,
) -> LuminateStringView {
    attestation_ref(attestation).map_or_else(null_view, |record| sv(record.subject.authority()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_attestation_subject(
    attestation: *const LuminateAttestation,
) -> LuminateStringView {
    attestation_ref(attestation).map_or_else(null_view, |record| sv(record.subject.subject()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_attestation_group_count(
    attestation: *const LuminateAttestation,
) -> usize {
    attestation_ref(attestation).map_or(0, |record| record.verified_groups.len())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_attestation_group_at(
    attestation: *const LuminateAttestation,
    index: usize,
) -> LuminateStringView {
    attestation_ref(attestation)
        .and_then(|record| record.verified_groups.get(index))
        .map_or_else(null_view, |group| sv(group))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_attestation_credential_id(
    attestation: *const LuminateAttestation,
) -> LuminateStringView {
    attestation_ref(attestation).map_or_else(null_view, |record| sv(&record.credential_id))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_attestation_expires_at_unix_ms(
    attestation: *const LuminateAttestation,
    out_expiry: *mut u64,
) -> bool {
    let (Some(attestation), Some(out_expiry)) =
        (attestation_ref(attestation), unsafe { out_expiry.as_mut() })
    else {
        return false;
    };
    let Some(expiry) = attestation.expires_at.and_then(system_time_unix_ms) else {
        return false;
    };
    *out_expiry = expiry;
    true
}
