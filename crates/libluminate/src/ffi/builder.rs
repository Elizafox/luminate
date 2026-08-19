// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::{
    AssertUnwindSafe, Authentication, BTreeSet, ClientBuilder, Credential,
    LuminateAuthenticationSource, LuminateClient, LuminateClientBuilder,
    LuminateResourceConstraintsInput, LuminateSessionMetadata, LuminateSessionScopeBuilder,
    LuminateStatus, LuminateStringView, Operation, ResourceConstraints, ScopeGrant, SessionScope,
    UNIX_EPOCH, c_char, clear_last_error, client_ref, ffi_guard, once, panic, ptr, read_path,
    read_required_str, read_resource_constraints, sanitize_cstring, set_last_error, slice,
    spawn_ffi_client, store_error, write_client,
};
use std::ffi::CString;

use crate::ffi_typed::LuminatePolicyOperation;

/// Creates a client builder using peer authentication and the default socket.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_builder_new(
    out_builder: *mut *mut LuminateClientBuilder,
) -> LuminateStatus {
    ffi_guard(|| {
        if out_builder.is_null() {
            set_last_error("client builder output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let builder = LuminateClientBuilder {
            path: None,
            authentication: Authentication::Peer,
            scope: None,
        };
        unsafe { *out_builder = Box::into_raw(Box::new(builder)) };
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Releases a client builder. Null is a no-op.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_builder_free(builder: *mut LuminateClientBuilder) {
    if !builder.is_null() {
        let _ = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            drop(Box::from_raw(builder));
        }));
    }
}

/// Selects peer authentication.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_builder_authenticate_peer(
    builder: *mut LuminateClientBuilder,
) -> LuminateStatus {
    ffi_guard(|| {
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            set_last_error("client builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        builder.authentication = Authentication::Peer;
        clear_last_error();
        LuminateStatus::Ok
    })
}

unsafe fn read_credential(value: *const u8, len: usize) -> Result<Credential, LuminateStatus> {
    if value.is_null() {
        set_last_error("credential pointer is null");
        return Err(LuminateStatus::NullPointer);
    }
    let bytes = unsafe { slice::from_raw_parts(value, len) };
    Credential::new(bytes).map_err(|error| {
        set_last_error(error.to_string());
        LuminateStatus::InvalidArgument
    })
}

/// Selects daemon bearer-token authentication and deep-copies the credential.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_builder_authenticate_bearer(
    builder: *mut LuminateClientBuilder,
    credential: *const u8,
    credential_len: usize,
) -> LuminateStatus {
    ffi_guard(|| {
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            set_last_error("client builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        let credential = match unsafe { read_credential(credential, credential_len) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        builder.authentication = Authentication::Bearer { credential };
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Selects a socket path and deep-copies it.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_builder_set_path(
    builder: *mut LuminateClientBuilder,
    path: *const c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            set_last_error("client builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        builder.path = match unsafe { read_path(path) } {
            Ok(path) => Some(path.into()),
            Err(status) => return status,
        };
        clear_last_error();
        LuminateStatus::Ok
    })
}

unsafe fn read_authentication_name(
    value: *const c_char,
    field: &'static str,
) -> Result<String, LuminateStatus> {
    unsafe { read_required_str(value, field) }.map(str::to_owned)
}

/// Selects actor-bound attestation authentication and deep-copies its inputs.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_builder_authenticate_attestation(
    builder: *mut LuminateClientBuilder,
    name: *const c_char,
    credential: *const u8,
    credential_len: usize,
) -> LuminateStatus {
    ffi_guard(|| {
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            set_last_error("client builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        let name = match unsafe { read_authentication_name(name, "attestation name") } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let credential = match unsafe { read_credential(credential, credential_len) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        builder.authentication = Authentication::Attestation { name, credential };
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Selects an external authentication provider and deep-copies its inputs.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_builder_authenticate_external(
    builder: *mut LuminateClientBuilder,
    provider: *const c_char,
    credential: *const u8,
    credential_len: usize,
) -> LuminateStatus {
    ffi_guard(|| {
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            set_last_error("client builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        let provider = match unsafe { read_authentication_name(provider, "provider name") } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let credential = match unsafe { read_credential(credential, credential_len) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        builder.authentication = Authentication::External {
            provider,
            credential,
        };
        clear_last_error();
        LuminateStatus::Ok
    })
}

fn scope_operation(value: LuminatePolicyOperation) -> Option<Operation> {
    Some(match value {
        0 => Operation::Observe,
        1 => Operation::Refresh,
        2 => Operation::Control,
        3 => Operation::HardwareAdministration,
        4 => Operation::DaemonAdministration,
        5 => Operation::ManagePlugins,
        6 => Operation::CreateCollection,
        7 => Operation::DestroyCollection,
        8 => Operation::ModifyCollection,
        9 => Operation::AdministerCollections,
        10 => Operation::ManagePolicy,
        11 => Operation::ManageAuthentication,
        12 => Operation::AdministerFrontend,
        13 => Operation::CreateScene,
        14 => Operation::ModifyScene,
        15 => Operation::DestroyScene,
        16 => Operation::AdministerScenes,
        _ => return None,
    })
}

/// Adds one unconstrained, allow-only operation grant to the session scope.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_builder_add_scope_operation(
    builder: *mut LuminateClientBuilder,
    operation: LuminatePolicyOperation,
) -> LuminateStatus {
    ffi_guard(|| {
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            set_last_error("client builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        let Some(operation) = scope_operation(operation) else {
            set_last_error(format!("unrecognized scope operation {operation}"));
            return LuminateStatus::InvalidArgument;
        };
        let mut grants = builder
            .scope
            .as_ref()
            .map_or_else(Vec::new, |scope| scope.grants().to_vec());
        grants.push(ScopeGrant {
            operations: once(operation).collect(),
            resources: ResourceConstraints::default(),
        });
        builder.scope = match SessionScope::new(grants) {
            Ok(scope) => Some(scope),
            Err(error) => {
                set_last_error(error.to_string());
                return LuminateStatus::InvalidArgument;
            }
        };
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Creates an empty session-scope builder.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_session_scope_builder_new(
    out_scope: *mut *mut LuminateSessionScopeBuilder,
) -> LuminateStatus {
    ffi_guard(|| {
        if out_scope.is_null() {
            set_last_error("session scope builder output pointer is null");
            return LuminateStatus::NullPointer;
        }
        unsafe { *out_scope = Box::into_raw(Box::new(LuminateSessionScopeBuilder(Vec::new()))) };
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Releases a session-scope builder. Null is a no-op.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_session_scope_builder_free(
    scope: *mut LuminateSessionScopeBuilder,
) {
    if !scope.is_null() {
        let _ = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            drop(Box::from_raw(scope));
        }));
    }
}

/// Adds one unconstrained multi-operation grant, deep-copying the operation array.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_session_scope_builder_add_grant(
    scope: *mut LuminateSessionScopeBuilder,
    operations: *const LuminatePolicyOperation,
    operation_count: usize,
) -> LuminateStatus {
    ffi_guard(|| {
        let Some(scope) = (unsafe { scope.as_mut() }) else {
            set_last_error("session scope builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        if operation_count == 0 {
            set_last_error("session scope grant has no operations");
            return LuminateStatus::InvalidArgument;
        }
        if operations.is_null() {
            set_last_error("session scope operations pointer is null");
            return LuminateStatus::NullPointer;
        }
        let operations = unsafe { slice::from_raw_parts(operations, operation_count) };
        let mut typed = BTreeSet::new();
        for &operation in operations {
            let Some(operation) = scope_operation(operation) else {
                set_last_error(format!("unrecognized scope operation {operation}"));
                return LuminateStatus::InvalidArgument;
            };
            typed.insert(operation);
        }
        scope.0.push(ScopeGrant {
            operations: typed,
            resources: ResourceConstraints::default(),
        });
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Adds one constrained multi-operation grant, deep-copying every input.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_session_scope_builder_add_constrained_grant(
    scope: *mut LuminateSessionScopeBuilder,
    operations: *const LuminatePolicyOperation,
    operation_count: usize,
    resources: *const LuminateResourceConstraintsInput,
) -> LuminateStatus {
    ffi_guard(|| {
        let Some(scope) = (unsafe { scope.as_mut() }) else {
            set_last_error("session scope builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        if operation_count == 0 {
            set_last_error("session scope grant has no operations");
            return LuminateStatus::InvalidArgument;
        }
        if operations.is_null() || resources.is_null() {
            set_last_error("session scope grant input pointer is null");
            return LuminateStatus::NullPointer;
        }
        let operations = unsafe { slice::from_raw_parts(operations, operation_count) };
        let mut typed = BTreeSet::new();
        for &operation in operations {
            let Some(operation) = scope_operation(operation) else {
                set_last_error(format!("unrecognized scope operation {operation}"));
                return LuminateStatus::InvalidArgument;
            };
            typed.insert(operation);
        }
        let resources = unsafe { &*resources };
        let resources = match unsafe { read_resource_constraints(resources) } {
            Ok(resources) => resources,
            Err(status) => return status,
        };
        scope.0.push(ScopeGrant {
            operations: typed,
            resources,
        });
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Deep-copies a validated scope into the client builder.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_builder_set_scope(
    builder: *mut LuminateClientBuilder,
    scope: *const LuminateSessionScopeBuilder,
) -> LuminateStatus {
    ffi_guard(|| {
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            set_last_error("client builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        let Some(scope) = (unsafe { scope.as_ref() }) else {
            set_last_error("session scope builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        builder.scope = match SessionScope::new(scope.0.clone()) {
            Ok(scope) => Some(scope),
            Err(error) => {
                set_last_error(error.to_string());
                return LuminateStatus::InvalidArgument;
            }
        };
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Connects without consuming the builder.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_builder_connect(
    builder: *const LuminateClientBuilder,
    out_client: *mut *mut LuminateClient,
) -> LuminateStatus {
    ffi_guard(|| {
        let Some(builder) = (unsafe { builder.as_ref() }) else {
            set_last_error("client builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        if out_client.is_null() {
            set_last_error("client output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let path = builder.path.clone();
        let authentication = builder.authentication.clone();
        let scope = builder.scope.clone();
        match spawn_ffi_client(move |runtime| {
            let mut configured = ClientBuilder::new().authentication(authentication);
            if let Some(path) = path {
                configured = configured.path(path);
            }
            if let Some(scope) = scope {
                configured = configured.scope(scope);
            }
            runtime.block_on(configured.connect())
        }) {
            Ok(client) => {
                clear_last_error();
                unsafe { write_client(out_client, client) };
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}

/// Copies this client's sanitized session metadata into an owned snapshot.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_get_session_metadata(
    client: *const LuminateClient,
    out_metadata: *mut *mut LuminateSessionMetadata,
) -> LuminateStatus {
    ffi_guard(|| {
        if out_metadata.is_null() {
            set_last_error("session metadata output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let client = match unsafe { client_ref(client.cast_mut()) } {
            Ok(client) => client,
            Err(status) => return status,
        };
        let metadata = match client.session() {
            Ok(metadata) => metadata.clone(),
            Err(status) => return status,
        };
        let (source, source_name) = match metadata.source {
            luminate_protocol::AuthenticationSource::Peer => {
                (LuminateAuthenticationSource::Peer, None)
            }
            luminate_protocol::AuthenticationSource::Bearer => {
                (LuminateAuthenticationSource::Bearer, None)
            }
            luminate_protocol::AuthenticationSource::Attestation { name } => (
                LuminateAuthenticationSource::Attestation,
                Some(sanitize_cstring(name)),
            ),
            luminate_protocol::AuthenticationSource::External { provider } => (
                LuminateAuthenticationSource::External,
                Some(sanitize_cstring(provider)),
            ),
        };
        let expires_at_unix_ms = metadata.expires_at.and_then(|expiry| {
            let duration = expiry.duration_since(UNIX_EPOCH).ok()?;
            u64::try_from(duration.as_millis()).ok()
        });
        let snapshot = LuminateSessionMetadata {
            authority: sanitize_cstring(metadata.subject.authority().to_owned()),
            subject: sanitize_cstring(metadata.subject.subject().to_owned()),
            verified_groups: metadata
                .verified_groups
                .into_iter()
                .map(sanitize_cstring)
                .collect(),
            source,
            source_name,
            credential_id: metadata.credential_id.map(sanitize_cstring),
            expires_at_unix_ms,
        };
        unsafe { *out_metadata = Box::into_raw(Box::new(snapshot)) };
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Releases a session metadata snapshot. Null is a no-op.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_session_metadata_free(metadata: *mut LuminateSessionMetadata) {
    if !metadata.is_null() {
        let _ = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            drop(Box::from_raw(metadata));
        }));
    }
}

fn cstring_view(value: &CString) -> LuminateStringView {
    LuminateStringView {
        data: value.as_ptr(),
        len: value.as_bytes().len(),
    }
}

fn optional_cstring_view(value: Option<&CString>) -> LuminateStringView {
    value.map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        cstring_view,
    )
}

/// Returns the authenticated authority from a metadata snapshot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_session_metadata_authority(
    metadata: *const LuminateSessionMetadata,
) -> LuminateStringView {
    unsafe { metadata.as_ref() }.map_or_else(
        || optional_cstring_view(None),
        |metadata| cstring_view(&metadata.authority),
    )
}

/// Returns the authenticated subject from a metadata snapshot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_session_metadata_subject(
    metadata: *const LuminateSessionMetadata,
) -> LuminateStringView {
    unsafe { metadata.as_ref() }.map_or_else(
        || optional_cstring_view(None),
        |metadata| cstring_view(&metadata.subject),
    )
}

/// Returns the number of verified authentication groups.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_session_metadata_group_count(
    metadata: *const LuminateSessionMetadata,
) -> usize {
    unsafe { metadata.as_ref() }.map_or(0, |metadata| metadata.verified_groups.len())
}

/// Returns one verified authentication group, or an empty view out of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_session_metadata_group_at(
    metadata: *const LuminateSessionMetadata,
    index: usize,
) -> LuminateStringView {
    unsafe { metadata.as_ref() }
        .and_then(|metadata| metadata.verified_groups.get(index))
        .map_or_else(|| optional_cstring_view(None), cstring_view)
}

/// Returns the authentication-source discriminant.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_session_metadata_source(
    metadata: *const LuminateSessionMetadata,
) -> LuminateAuthenticationSource {
    unsafe { metadata.as_ref() }.map_or(LuminateAuthenticationSource::Unknown, |metadata| {
        metadata.source
    })
}

/// Returns the attestation/provider name when the source has one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_session_metadata_source_name(
    metadata: *const LuminateSessionMetadata,
) -> LuminateStringView {
    unsafe { metadata.as_ref() }.map_or_else(
        || optional_cstring_view(None),
        |metadata| optional_cstring_view(metadata.source_name.as_ref()),
    )
}

/// Returns the non-secret credential ID when one exists.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_session_metadata_credential_id(
    metadata: *const LuminateSessionMetadata,
) -> LuminateStringView {
    unsafe { metadata.as_ref() }.map_or_else(
        || optional_cstring_view(None),
        |metadata| optional_cstring_view(metadata.credential_id.as_ref()),
    )
}

/// Writes the expiry in Unix milliseconds and returns whether one exists.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_session_metadata_expires_at_unix_ms(
    metadata: *const LuminateSessionMetadata,
    out_expiry: *mut u64,
) -> bool {
    let (Some(metadata), Some(out_expiry)) =
        (unsafe { metadata.as_ref() }, unsafe { out_expiry.as_mut() })
    else {
        return false;
    };
    let Some(expiry) = metadata.expires_at_unix_ms else {
        return false;
    };
    *out_expiry = expiry;
    true
}
