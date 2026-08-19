// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! C bindings for the transport-neutral policy model
//! (`luminate_core::policy`): building and validating a
//! [`PolicyDocument`], constructing a [`RemotePrincipal`], and evaluating
//! one authorization request against a built document.
//!
//! Nothing here talks to a daemon or a `LuminateClient`; this is pure,
//! synchronous computation, matching `PolicyDocument::evaluate`'s own
//! signature. Daemon policy administration consumes these owned documents
//! without exposing Rust layout.

#![allow(
    clippy::similar_names,
    reason = "C accessors use role_index and rule_index together throughout"
)]

use super::*;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use luminate_core::device::DeviceId;
use luminate_core::policy::{
    Binding, Operation, PolicyDocument, PolicyDocumentSource, PolicyRevision, RemotePrincipal,
    Resource, ResourceConstraints, Role, Rule, RuleEffect, RuleId,
};

/// A mutable, deep-copying builder for a [`LuminatePolicyDocument`].
///
/// Every setter/adder validates and copies its input immediately; the
/// builder never borrows caller memory past the call that filled it.
/// `luminate_policy_document_build` does not consume the builder, so it may
/// keep being edited and rebuilt (for example after bumping its revision).
pub struct LuminatePolicyDocumentBuilder(PolicyDocumentSource);

/// A validated, immutable access policy document, built by
/// `luminate_policy_document_build` and evaluated by
/// `luminate_policy_document_evaluate`.
pub struct LuminatePolicyDocument(pub(super) PolicyDocument);

/// Borrowed role within an owning policy document.
pub struct LuminatePolicyRole;

/// Borrowed rule within an owning policy role.
pub struct LuminatePolicyRule;

/// Borrowed binding within an owning policy document.
pub struct LuminatePolicyBinding;

impl LuminatePolicyDocument {
    /// Returns the wrapped validated document, for use by other `ffi_typed`
    /// modules building a built-in `StaticAccessPolicy` around it.
    pub(crate) const fn as_document(&self) -> &PolicyDocument {
        &self.0
    }

    /// Wraps an already validated document, for other `ffi_typed` modules'
    /// tests that construct one directly via the plain Rust policy API
    /// rather than the C builder.
    pub(crate) const fn from_document(document: PolicyDocument) -> Self {
        Self(document)
    }
}

/// An owned remote principal: exact opaque authority, subject, and verified
/// group strings, with no credentials or arbitrary claims.
pub struct LuminateRemotePrincipal(RemotePrincipal);

/// An owned authorization decision, returned by
/// `luminate_policy_document_evaluate`.
pub struct LuminateAuthorizationEvaluation(luminate_core::policy::AuthorizationDecision);

null_safe_free!(
    "Releases a `LuminatePolicyDocumentBuilder` created by \
     `luminate_policy_document_builder_new`. Null is a no-op.",
    luminate_policy_document_builder_free,
    LuminatePolicyDocumentBuilder
);
null_safe_free!(
    "Releases a `LuminatePolicyDocument` returned by \
     `luminate_policy_document_build`. Null is a no-op.",
    luminate_policy_document_free,
    LuminatePolicyDocument
);
null_safe_free!(
    "Releases a `LuminateRemotePrincipal` created by \
     `luminate_remote_principal_new`. Null is a no-op.",
    luminate_remote_principal_free,
    LuminateRemotePrincipal
);
null_safe_free!(
    "Releases a `LuminateAuthorizationEvaluation` returned by \
     `luminate_policy_document_evaluate`. Null is a no-op.",
    luminate_authorization_evaluation_free,
    LuminateAuthorizationEvaluation
);

fn operation_from_u32(value: u32) -> Result<Operation, LuminateStatus> {
    match value {
        0 => Ok(Operation::Observe),
        1 => Ok(Operation::Refresh),
        2 => Ok(Operation::Control),
        3 => Ok(Operation::HardwareAdministration),
        4 => Ok(Operation::DaemonAdministration),
        5 => Ok(Operation::ManagePlugins),
        6 => Ok(Operation::CreateCollection),
        7 => Ok(Operation::DestroyCollection),
        8 => Ok(Operation::ModifyCollection),
        9 => Ok(Operation::AdministerCollections),
        10 => Ok(Operation::ManagePolicy),
        11 => Ok(Operation::ManageAuthentication),
        12 => Ok(Operation::AdministerFrontend),
        13 => Ok(Operation::CreateScene),
        14 => Ok(Operation::ModifyScene),
        15 => Ok(Operation::DestroyScene),
        16 => Ok(Operation::AdministerScenes),
        _ => {
            crate::ffi::set_last_error(format!("unrecognized policy operation {value}"));
            Err(LuminateStatus::InvalidArgument)
        }
    }
}

fn rule_effect_from_u32(value: u32) -> Result<RuleEffect, LuminateStatus> {
    match value {
        0 => Ok(RuleEffect::Allow),
        1 => Ok(RuleEffect::Deny),
        _ => {
            crate::ffi::set_last_error(format!("unrecognized rule effect {value}"));
            Err(LuminateStatus::InvalidArgument)
        }
    }
}

pub(crate) fn null_view() -> LuminateStringView {
    LuminateStringView {
        data: ptr::null(),
        len: 0,
    }
}

/// Reads `count` NUL-terminated UTF-8 strings from a caller-owned array,
/// copying each one. `count == 0` returns an empty vector without requiring
/// `values` to be non-null, matching this codebase's other optional-array
/// input conventions.
pub(crate) unsafe fn read_str_array(
    values: *const *const c_char,
    count: usize,
    name: &'static str,
) -> Result<Vec<String>, LuminateStatus> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if values.is_null() {
        crate::ffi::set_last_error(format!("{name} pointer is null"));
        return Err(LuminateStatus::NullPointer);
    }
    // SAFETY: upheld by the enclosing function's documented C pointer contract:
    // `values` points to `count` valid, readable string pointers.
    let slice = unsafe { std::slice::from_raw_parts(values, count) };
    slice
        .iter()
        // SAFETY: every element of `slice` is a valid NUL-terminated UTF-8
        // string pointer per the enclosing function's documented contract.
        .map(|&value| unsafe { read_required_str(value, name) }.map(str::to_owned))
        .collect()
}

unsafe fn read_operations(
    values: *const u32,
    count: usize,
) -> Result<BTreeSet<Operation>, LuminateStatus> {
    if count == 0 {
        return Ok(BTreeSet::new());
    }
    if values.is_null() {
        crate::ffi::set_last_error("rule operations pointer is null");
        return Err(LuminateStatus::NullPointer);
    }
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let slice = unsafe { std::slice::from_raw_parts(values, count) };
    slice.iter().copied().map(operation_from_u32).collect()
}

/// Optional constraints applied to every resource a rule is checked against.
/// An empty array field (`*_count == 0`) means "no constraint on this
/// dimension"; `values` need not be non-null in that case.
#[repr(C)]
pub struct LuminateResourceConstraintsInput {
    pub device_ids: *const *const c_char,
    pub device_id_count: usize,
    pub provider_instances: *const *const c_char,
    pub provider_instance_count: usize,
    pub has_host_attached: bool,
    pub host_attached: bool,
    pub collections: *const *const c_char,
    pub collection_count: usize,
}

pub(crate) unsafe fn read_resource_constraints(
    v: &LuminateResourceConstraintsInput,
) -> Result<ResourceConstraints, LuminateStatus> {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let device_ids =
        unsafe { read_str_array(v.device_ids, v.device_id_count, "constraint device_ids") }?
            .into_iter()
            .map(DeviceId::new)
            .collect();
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let provider_instances = unsafe {
        read_str_array(
            v.provider_instances,
            v.provider_instance_count,
            "constraint provider_instances",
        )
    }?
    .into_iter()
    .collect();
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let collections =
        unsafe { read_str_array(v.collections, v.collection_count, "constraint collections") }?
            .into_iter()
            .map(CollectionId::new)
            .collect();
    Ok(ResourceConstraints {
        device_ids,
        provider_instances,
        host_attached: v.has_host_attached.then_some(v.host_attached),
        collections,
    })
}

/// One input rule for `luminate_policy_document_builder_role_add_rule`.
/// `effect` is one of the `LUMINATE_RULE_EFFECT_*` values and each entry of
/// `operations` is one of the `LUMINATE_POLICY_OP_*` values. `reason` may be
/// null. `cache_hint_ms` is read only when `has_cache_hint_ms` is `true`, and
/// is clamped to `luminate_core::policy::MAX_CACHE_HINT` by validation, not
/// by this input struct.
#[repr(C)]
pub struct LuminateRuleInput {
    pub id: *const c_char,
    pub effect: LuminateRuleEffect,
    pub operations: *const LuminatePolicyOperation,
    pub operation_count: usize,
    pub resources: LuminateResourceConstraintsInput,
    pub reason: *const c_char,
    pub has_cache_hint_ms: bool,
    pub cache_hint_ms: u64,
}

unsafe fn read_rule_input(v: &LuminateRuleInput) -> Result<Rule, LuminateStatus> {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let id = unsafe { read_required_str(v.id, "rule id") }?;
    let id = RuleId::new(id.to_owned()).map_err(|error| {
        crate::ffi::set_last_error(format!("invalid rule id: {error}"));
        LuminateStatus::InvalidArgument
    })?;
    let effect = rule_effect_from_u32(v.effect)?;
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let operations = unsafe { read_operations(v.operations, v.operation_count) }?;
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let resources = unsafe { read_resource_constraints(&v.resources) }?;
    let reason = if v.reason.is_null() {
        None
    } else {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        Some(unsafe { read_required_str(v.reason, "rule reason") }?.to_owned())
    };
    let cache_hint = v
        .has_cache_hint_ms
        .then(|| Duration::from_millis(v.cache_hint_ms));
    Ok(Rule {
        id,
        effect,
        operations,
        resources,
        reason,
        cache_hint,
    })
}

fn role_entry<'a>(source: &'a mut PolicyDocumentSource, name: &str) -> &'a mut Role {
    source.roles.entry(name.to_owned()).or_insert_with(|| Role {
        parents: BTreeSet::new(),
        rules: Vec::new(),
    })
}

/// Creates an empty policy document builder at the given revision. Release
/// with `luminate_policy_document_builder_free`.
///
/// # Safety
///
/// `out_builder` must be a valid non-null out-pointer.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_builder_new(
    revision: u64,
    out_builder: *mut *mut LuminatePolicyDocumentBuilder,
) -> LuminateStatus {
    ffi_guard(|| {
        let source = PolicyDocumentSource {
            revision: PolicyRevision(revision),
            roles: BTreeMap::new(),
            bindings: Vec::new(),
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        if let Err(status) = unsafe {
            write_box(
                out_builder,
                LuminatePolicyDocumentBuilder(source),
                "policy document builder",
            )
        } {
            return status;
        }
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Seeds a mutable builder with a complete, independent copy of a validated
/// policy document. Building the result without mutation preserves the
/// document's observable contents and authorization behaviour. Release the
/// builder with `luminate_policy_document_builder_free`.
///
/// # Safety
///
/// `document` must be a valid non-null document pointer. `out_builder` must be
/// a valid non-null out-pointer.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_builder_from_document(
    document: *const LuminatePolicyDocument,
    out_builder: *mut *mut LuminatePolicyDocumentBuilder,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let Some(document) = (unsafe { document.as_ref() }) else {
            crate::ffi::set_last_error("policy document pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        if let Err(status) = unsafe {
            write_box(
                out_builder,
                LuminatePolicyDocumentBuilder(document.0.source().clone()),
                "policy document builder",
            )
        } {
            return status;
        }
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Sets the builder's revision, overwriting any previous value.
///
/// # Safety
///
/// `builder` must be a valid, exclusively used builder pointer.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_builder_set_revision(
    builder: *mut LuminatePolicyDocumentBuilder,
    revision: u64,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            crate::ffi::set_last_error("policy document builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        builder.0.revision = PolicyRevision(revision);
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Adds one parent role to a role, creating both the role and its parent
/// entry if either is not yet present. A missing referenced parent is a
/// build-time validation error, not a builder-time one.
///
/// # Safety
///
/// `builder` must be a valid, exclusively used builder pointer. `role` and
/// `parent` must point to valid NUL-terminated UTF-8 strings.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_builder_role_add_parent(
    builder: *mut LuminatePolicyDocumentBuilder,
    role: *const c_char,
    parent: *const c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            crate::ffi::set_last_error("policy document builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let role = match unsafe { read_required_str(role, "role") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let parent = match unsafe { read_required_str(parent, "parent") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        role_entry(&mut builder.0, &parent);
        role_entry(&mut builder.0, &role).parents.insert(parent);
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Removes a role and its contents by name. References to that role in parent
/// sets and bindings are not rewritten; the caller must update them before
/// building the document.
///
/// # Safety
///
/// `builder` must be a valid, exclusively used builder pointer. `role` must
/// point to a valid NUL-terminated UTF-8 string.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_builder_remove_role(
    builder: *mut LuminatePolicyDocumentBuilder,
    role: *const c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            crate::ffi::set_last_error("policy document builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let role = match unsafe { read_required_str(role, "role") } {
            Ok(value) => value,
            Err(status) => return status,
        };
        if builder.0.roles.remove(role).is_none() {
            crate::ffi::set_last_error(format!("policy role {role:?} does not exist"));
            return LuminateStatus::InvalidArgument;
        }
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Removes one parent from a role. Both the role and parent relationship must
/// already exist.
///
/// # Safety
///
/// All pointers must be valid and non-null. `role` and `parent` must point to
/// NUL-terminated UTF-8 strings.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_builder_role_remove_parent(
    builder: *mut LuminatePolicyDocumentBuilder,
    role: *const c_char,
    parent: *const c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            crate::ffi::set_last_error("policy document builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let role = match unsafe { read_required_str(role, "role") } {
            Ok(value) => value,
            Err(status) => return status,
        };
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let parent = match unsafe { read_required_str(parent, "parent") } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let Some(role_value) = builder.0.roles.get_mut(role) else {
            crate::ffi::set_last_error(format!("policy role {role:?} does not exist"));
            return LuminateStatus::InvalidArgument;
        };
        if !role_value.parents.remove(parent) {
            crate::ffi::set_last_error(format!(
                "policy role {role:?} does not have parent {parent:?}"
            ));
            return LuminateStatus::InvalidArgument;
        }
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Appends one rule to a role, creating the role if not yet present.
///
/// # Safety
///
/// `builder` must be a valid, exclusively used builder pointer. `role` must
/// point to a valid NUL-terminated UTF-8 string. `rule` must be a valid,
/// non-null pointer to a fully populated `LuminateRuleInput`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_builder_role_add_rule(
    builder: *mut LuminatePolicyDocumentBuilder,
    role_name: *const c_char,
    rule: *const LuminateRuleInput,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            crate::ffi::set_last_error("policy document builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let role_name = match unsafe { read_required_str(role_name, "role") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let Some(rule) = (unsafe { rule.as_ref() }) else {
            crate::ffi::set_last_error("rule pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let rule = match unsafe { read_rule_input(rule) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        role_entry(&mut builder.0, &role_name).rules.push(rule);
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Replaces one rule at `index`, preserving its position.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_builder_role_replace_rule(
    builder: *mut LuminatePolicyDocumentBuilder,
    role_name: *const c_char,
    index: usize,
    rule: *const LuminateRuleInput,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            crate::ffi::set_last_error("policy document builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let role_name = match unsafe { read_required_str(role_name, "role") } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let Some(role) = builder.0.roles.get_mut(role_name) else {
            crate::ffi::set_last_error(format!("policy role {role_name:?} does not exist"));
            return LuminateStatus::InvalidArgument;
        };
        let Some(slot) = role.rules.get_mut(index) else {
            crate::ffi::set_last_error(format!(
                "policy role {role_name:?} has no rule at index {index}"
            ));
            return LuminateStatus::InvalidArgument;
        };
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let Some(rule) = (unsafe { rule.as_ref() }) else {
            crate::ffi::set_last_error("rule pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: nested pointers are governed by the input contract.
        let replacement = match unsafe { read_rule_input(rule) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        *slot = replacement;
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Removes one rule at `index` from a role.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_builder_role_remove_rule(
    builder: *mut LuminatePolicyDocumentBuilder,
    role_name: *const c_char,
    index: usize,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            crate::ffi::set_last_error("policy document builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let role_name = match unsafe { read_required_str(role_name, "role") } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let Some(role) = builder.0.roles.get_mut(role_name) else {
            crate::ffi::set_last_error(format!("policy role {role_name:?} does not exist"));
            return LuminateStatus::InvalidArgument;
        };
        if index >= role.rules.len() {
            crate::ffi::set_last_error(format!(
                "policy role {role_name:?} has no rule at index {index}"
            ));
            return LuminateStatus::InvalidArgument;
        }
        role.rules.remove(index);
        clear_last_error();
        LuminateStatus::Ok
    })
}

unsafe fn read_binding_input(
    authority: *const c_char,
    subjects: *const *const c_char,
    subject_count: usize,
    groups: *const *const c_char,
    group_count: usize,
    roles: *const *const c_char,
    role_count: usize,
) -> Result<Binding, LuminateStatus> {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let authority = unsafe { read_required_str(authority, "binding authority") }?.to_owned();
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let subjects = unsafe { read_str_array(subjects, subject_count, "binding subjects") }?
        .into_iter()
        .collect::<BTreeSet<_>>();
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let groups = unsafe { read_str_array(groups, group_count, "binding groups") }?
        .into_iter()
        .collect::<BTreeSet<_>>();
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let roles = unsafe { read_str_array(roles, role_count, "binding roles") }?
        .into_iter()
        .collect::<BTreeSet<_>>();
    Ok(Binding {
        authority,
        subjects,
        groups,
        roles,
    })
}

/// Appends one binding.
///
/// # Safety
///
/// `builder` must be a valid, exclusively used builder pointer. `authority`
/// must point to a valid NUL-terminated UTF-8 string. `subjects`, `groups`,
/// and `roles` must each point to their respective `*_count` valid string
/// pointers, or may be null when their count is `0`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_builder_add_binding(
    builder: *mut LuminatePolicyDocumentBuilder,
    authority: *const c_char,
    subjects: *const *const c_char,
    subject_count: usize,
    groups: *const *const c_char,
    group_count: usize,
    roles: *const *const c_char,
    role_count: usize,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            crate::ffi::set_last_error("policy document builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: nested pointers are governed by the input contract.
        let binding = match unsafe {
            read_binding_input(
                authority,
                subjects,
                subject_count,
                groups,
                group_count,
                roles,
                role_count,
            )
        } {
            Ok(value) => value,
            Err(status) => return status,
        };
        builder.0.bindings.push(binding);
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Replaces one binding at `index`, preserving its position.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_builder_replace_binding(
    builder: *mut LuminatePolicyDocumentBuilder,
    index: usize,
    authority: *const c_char,
    subjects: *const *const c_char,
    subject_count: usize,
    groups: *const *const c_char,
    group_count: usize,
    roles: *const *const c_char,
    role_count: usize,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            crate::ffi::set_last_error("policy document builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        let Some(slot) = builder.0.bindings.get_mut(index) else {
            crate::ffi::set_last_error(format!("policy document has no binding at index {index}"));
            return LuminateStatus::InvalidArgument;
        };
        // SAFETY: nested pointers are governed by the input contract.
        let binding = match unsafe {
            read_binding_input(
                authority,
                subjects,
                subject_count,
                groups,
                group_count,
                roles,
                role_count,
            )
        } {
            Ok(value) => value,
            Err(status) => return status,
        };
        *slot = binding;
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Removes one binding at `index`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_builder_remove_binding(
    builder: *mut LuminatePolicyDocumentBuilder,
    index: usize,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            crate::ffi::set_last_error("policy document builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        if index >= builder.0.bindings.len() {
            crate::ffi::set_last_error(format!("policy document has no binding at index {index}"));
            return LuminateStatus::InvalidArgument;
        }
        builder.0.bindings.remove(index);
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Validates the builder's current contents and writes a new, immutable
/// `LuminatePolicyDocument`. Does not consume or clear the builder, which may
/// keep being edited and rebuilt afterwards. Release the returned document
/// with `luminate_policy_document_free`.
///
/// # Safety
///
/// `builder` must be a valid builder pointer. `out_document` must be a valid
/// non-null out-pointer.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_build(
    builder: *const LuminatePolicyDocumentBuilder,
    out_document: *mut *mut LuminatePolicyDocument,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let Some(builder) = (unsafe { builder.as_ref() }) else {
            crate::ffi::set_last_error("policy document builder pointer is null");
            return LuminateStatus::NullPointer;
        };
        match PolicyDocument::new(builder.0.clone()) {
            Ok(document) => {
                // SAFETY: upheld by the enclosing function's documented C pointer contract.
                if let Err(status) = unsafe {
                    write_box(
                        out_document,
                        LuminatePolicyDocument(document),
                        "policy document",
                    )
                } {
                    return status;
                }
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(error) => {
                crate::ffi::set_last_error(format!("invalid policy document: {error}"));
                LuminateStatus::InvalidArgument
            }
        }
    })
}

/// Returns a built document's revision, or `0` if `document` is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_revision(
    document: *const LuminatePolicyDocument,
) -> u64 {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { document.as_ref() }.map_or(0, |v| v.0.revision().0)
}

fn role_at(document: &PolicyDocument, index: usize) -> Option<(&str, &Role)> {
    document
        .roles()
        .iter()
        .nth(index)
        .map(|(name, role)| (name.as_str(), role))
}

fn role_ref(role: *const LuminatePolicyRole) -> Option<&'static Role> {
    // SAFETY: the C contract requires a view returned by
    // `luminate_policy_document_role_at`.
    unsafe { role.cast::<Role>().as_ref() }
}

fn rule_ref(rule: *const LuminatePolicyRule) -> Option<&'static Rule> {
    // SAFETY: the C contract requires a view returned by
    // `luminate_policy_role_rule_at`.
    unsafe { rule.cast::<Rule>().as_ref() }
}

fn binding_ref(binding: *const LuminatePolicyBinding) -> Option<&'static Binding> {
    // SAFETY: the C contract requires a view returned by
    // `luminate_policy_document_binding_at`.
    unsafe { binding.cast::<Binding>().as_ref() }
}

/// Number of roles in a built document.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_role_count(
    document: *const LuminatePolicyDocument,
) -> usize {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { document.as_ref() }.map_or(0, |v| v.0.roles().len())
}

/// Role name at `role_index`, in canonical order.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_role_name_at(
    document: *const LuminatePolicyDocument,
    role_index: usize,
) -> LuminateStringView {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { document.as_ref() }
        .and_then(|v| role_at(&v.0, role_index))
        .map_or(null_view(), |(name, _)| sv(name))
}

/// Borrowed role at `role_index`, in canonical order.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_role_at(
    document: *const LuminatePolicyDocument,
    role_index: usize,
) -> *const LuminatePolicyRole {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { document.as_ref() }
        .and_then(|v| role_at(&v.0, role_index))
        .map_or(ptr::null(), |(_, role)| cast_ref(role))
}

/// Number of parent roles for a role.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_role_parent_count(
    role: *const LuminatePolicyRole,
) -> usize {
    role_ref(role).map_or(0, |role| role.parents.len())
}

/// Parent role name at `index`, in canonical order.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_role_parent_at(
    role: *const LuminatePolicyRole,
    index: usize,
) -> LuminateStringView {
    role_ref(role)
        .and_then(|role| role.parents.iter().nth(index))
        .map_or(null_view(), |parent| sv(parent))
}

/// Number of rules on a role.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_role_rule_count(role: *const LuminatePolicyRole) -> usize {
    role_ref(role).map_or(0, |role| role.rules.len())
}

/// Borrowed rule at `index`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_role_rule_at(
    role: *const LuminatePolicyRole,
    index: usize,
) -> *const LuminatePolicyRule {
    role_ref(role)
        .and_then(|role| role.rules.get(index))
        .map_or(ptr::null(), cast_ref)
}

/// Stable ID of a rule.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_rule_id(
    rule: *const LuminatePolicyRule,
) -> LuminateStringView {
    rule_ref(rule).map_or(null_view(), |rule| sv(rule.id.as_str()))
}

/// Effect of a rule, or `LUMINATE_DISCRIMINANT_INVALID`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_rule_effect(
    rule: *const LuminatePolicyRule,
) -> LuminateRuleEffect {
    rule_ref(rule).map_or(u32::MAX, |rule| match rule.effect {
        RuleEffect::Allow => 0,
        RuleEffect::Deny => 1,
    })
}

/// Number of semantic operations matched by a rule.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_rule_operation_count(
    rule: *const LuminatePolicyRule,
) -> usize {
    rule_ref(rule).map_or(0, |rule| rule.operations.len())
}

fn operation_to_u32(operation: Operation) -> u32 {
    match operation {
        Operation::Observe => 0,
        Operation::Refresh => 1,
        Operation::Control => 2,
        Operation::HardwareAdministration => 3,
        Operation::DaemonAdministration => 4,
        Operation::ManagePlugins => 5,
        Operation::CreateCollection => 6,
        Operation::DestroyCollection => 7,
        Operation::ModifyCollection => 8,
        Operation::AdministerCollections => 9,
        Operation::ManagePolicy => 10,
        Operation::ManageAuthentication => 11,
        Operation::AdministerFrontend => 12,
        Operation::CreateScene => 13,
        Operation::ModifyScene => 14,
        Operation::DestroyScene => 15,
        Operation::AdministerScenes => 16,
    }
}

/// Semantic operation at `operation_index`, in canonical order.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_rule_operation_at(
    rule: *const LuminatePolicyRule,
    index: usize,
) -> u32 {
    rule_ref(rule)
        .and_then(|rule| rule.operations.iter().nth(index))
        .map_or(u32::MAX, |operation| operation_to_u32(*operation))
}

/// Safe reason attached to a rule, or an absent view.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_rule_reason(
    rule: *const LuminatePolicyRule,
) -> LuminateStringView {
    rule_ref(rule)
        .and_then(|rule| rule.reason.as_deref())
        .map_or(null_view(), sv)
}

/// Writes a rule's cache hint in milliseconds when present.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_rule_cache_hint_ms(
    rule: *const LuminatePolicyRule,
    out_cache_hint_ms: *mut u64,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(hint), Some(out_cache_hint_ms)) =
        (rule_ref(rule).and_then(|rule| rule.cache_hint), unsafe {
            out_cache_hint_ms.as_mut()
        })
    else {
        return false;
    };
    *out_cache_hint_ms = u64::try_from(hint.as_millis()).unwrap_or(u64::MAX);
    true
}

macro_rules! rule_constraint_string_accessors {
    ($count_fn:ident, $at_fn:ident, $field:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $count_fn(rule: *const LuminatePolicyRule) -> usize {
            rule_ref(rule).map_or(0, |rule| rule.resources.$field.len())
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $at_fn(
            rule: *const LuminatePolicyRule,
            index: usize,
        ) -> LuminateStringView {
            rule_ref(rule)
                .and_then(|rule| rule.resources.$field.iter().nth(index))
                .map_or(null_view(), |value| sv(value.as_str()))
        }
    };
}

rule_constraint_string_accessors!(
    luminate_policy_rule_device_id_count,
    luminate_policy_rule_device_id_at,
    device_ids
);
rule_constraint_string_accessors!(
    luminate_policy_rule_provider_instance_count,
    luminate_policy_rule_provider_instance_at,
    provider_instances
);
rule_constraint_string_accessors!(
    luminate_policy_rule_collection_count,
    luminate_policy_rule_collection_at,
    collections
);

/// Writes a rule's host-attachment constraint when present.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_rule_host_attached(
    rule: *const LuminatePolicyRule,
    out_host_attached: *mut bool,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(host_attached), Some(out_host_attached)) = (
        rule_ref(rule).and_then(|rule| rule.resources.host_attached),
        unsafe { out_host_attached.as_mut() },
    ) else {
        return false;
    };
    *out_host_attached = host_attached;
    true
}

/// Number of bindings in a built document.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_binding_count(
    document: *const LuminatePolicyDocument,
) -> usize {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { document.as_ref() }.map_or(0, |v| v.0.bindings().len())
}

/// Borrowed binding at `index`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_binding_at(
    document: *const LuminatePolicyDocument,
    index: usize,
) -> *const LuminatePolicyBinding {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { document.as_ref() }
        .and_then(|v| v.0.bindings().get(index))
        .map_or(ptr::null(), cast_ref)
}

/// Authority matched by a binding.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_binding_authority(
    binding: *const LuminatePolicyBinding,
) -> LuminateStringView {
    binding_ref(binding).map_or(null_view(), |binding| sv(&binding.authority))
}

macro_rules! binding_string_accessors {
    ($count_fn:ident, $at_fn:ident, $field:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $count_fn(binding: *const LuminatePolicyBinding) -> usize {
            binding_ref(binding).map_or(0, |binding| binding.$field.len())
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $at_fn(
            binding: *const LuminatePolicyBinding,
            index: usize,
        ) -> LuminateStringView {
            binding_ref(binding)
                .and_then(|binding| binding.$field.iter().nth(index))
                .map_or(null_view(), |value| sv(value))
        }
    };
}

binding_string_accessors!(
    luminate_policy_binding_subject_count,
    luminate_policy_binding_subject_at,
    subjects
);
binding_string_accessors!(
    luminate_policy_binding_group_count,
    luminate_policy_binding_group_at,
    groups
);
binding_string_accessors!(
    luminate_policy_binding_role_count,
    luminate_policy_binding_role_at,
    roles
);

/// Creates an owned remote principal from exact, opaque identity strings.
/// Release with `luminate_remote_principal_free`.
///
/// # Safety
///
/// `authority` and `subject` must point to valid NUL-terminated UTF-8
/// strings. `groups` must point to `group_count` valid string pointers, or
/// may be null when `group_count` is `0`. `out_principal` must be a valid
/// non-null out-pointer.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_remote_principal_new(
    authority: *const c_char,
    subject: *const c_char,
    groups: *const *const c_char,
    group_count: usize,
    out_principal: *mut *mut LuminateRemotePrincipal,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let authority = match unsafe { read_required_str(authority, "principal authority") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let subject = match unsafe { read_required_str(subject, "principal subject") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let groups = match unsafe { read_str_array(groups, group_count, "principal groups") } {
            Ok(v) => v,
            Err(e) => return e,
        };
        match RemotePrincipal::new(authority, subject, groups) {
            Ok(principal) => {
                // SAFETY: upheld by the enclosing function's documented C pointer contract.
                if let Err(status) = unsafe {
                    write_box(
                        out_principal,
                        LuminateRemotePrincipal(principal),
                        "remote principal",
                    )
                } {
                    return status;
                }
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(error) => {
                crate::ffi::set_last_error(format!("invalid remote principal: {error}"));
                LuminateStatus::InvalidArgument
            }
        }
    })
}

/// The principal's identity provider or trust domain.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_remote_principal_authority(
    v: *const LuminateRemotePrincipal,
) -> LuminateStringView {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(null_view(), |v| sv(v.0.authority()))
}

/// The principal's authority-local subject identifier.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_remote_principal_subject(
    v: *const LuminateRemotePrincipal,
) -> LuminateStringView {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(null_view(), |v| sv(v.0.subject()))
}

/// Number of verified groups on this principal.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_remote_principal_group_count(
    v: *const LuminateRemotePrincipal,
) -> usize {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(0, |v| v.0.groups().len())
}

/// Group at `index` in ascending sorted order, or an absent view if out of
/// range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_remote_principal_group_at(
    v: *const LuminateRemotePrincipal,
    index: usize,
) -> LuminateStringView {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }
        .and_then(|v| v.0.groups().iter().nth(index))
        .map_or(null_view(), |g| sv(g))
}

/// One resource an evaluated request would affect. `provider_instance` may
/// be null. `collections` may be null when `collection_count` is `0`.
#[repr(C)]
pub struct LuminateResourceInput {
    pub device_id: *const c_char,
    pub provider_instance: *const c_char,
    pub host_attached: bool,
    pub collections: *const *const c_char,
    pub collection_count: usize,
}

unsafe fn read_resource_input(v: &LuminateResourceInput) -> Result<Resource, LuminateStatus> {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let device_id = unsafe { read_required_str(v.device_id, "resource device_id") }?.to_owned();
    let provider_instance = if v.provider_instance.is_null() {
        None
    } else {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        Some(
            unsafe { read_required_str(v.provider_instance, "resource provider_instance") }?
                .to_owned(),
        )
    };
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let collections =
        unsafe { read_str_array(v.collections, v.collection_count, "resource collections") }?
            .into_iter()
            .map(CollectionId::new)
            .collect();
    Ok(Resource {
        device_id: DeviceId::new(device_id),
        provider_instance,
        host_attached: v.host_attached,
        collections,
    })
}

unsafe fn read_resources(
    values: *const LuminateResourceInput,
    count: usize,
) -> Result<Vec<Resource>, LuminateStatus> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if values.is_null() {
        crate::ffi::set_last_error("resources pointer is null");
        return Err(LuminateStatus::NullPointer);
    }
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let slice = unsafe { std::slice::from_raw_parts(values, count) };
    slice
        .iter()
        // SAFETY: every element of `slice` is a valid `LuminateResourceInput`.
        .map(|v| unsafe { read_resource_input(v) })
        .collect()
}

/// Evaluates one authorization request against a built document and writes
/// an owned `LuminateAuthorizationEvaluation`. `operation` is one of the
/// `LUMINATE_POLICY_OP_*` values. `resources` may be null when
/// `resource_count` is `0`, meaning the request has not yet been resolved to
/// concrete resources. Release the result with
/// `luminate_authorization_evaluation_free`.
///
/// # Safety
///
/// `document` and `principal` must be valid pointers. `resources` must point
/// to `resource_count` valid `LuminateResourceInput`s. `out_evaluation` must
/// be a valid non-null out-pointer.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_policy_document_evaluate(
    document: *const LuminatePolicyDocument,
    principal: *const LuminateRemotePrincipal,
    operation: LuminatePolicyOperation,
    resources: *const LuminateResourceInput,
    resource_count: usize,
    out_evaluation: *mut *mut LuminateAuthorizationEvaluation,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let Some(document) = (unsafe { document.as_ref() }) else {
            crate::ffi::set_last_error("policy document pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let Some(principal) = (unsafe { principal.as_ref() }) else {
            crate::ffi::set_last_error("remote principal pointer is null");
            return LuminateStatus::NullPointer;
        };
        let operation = match operation_from_u32(operation) {
            Ok(v) => v,
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let resources = match unsafe { read_resources(resources, resource_count) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let decision = document.0.evaluate(&principal.0, operation, &resources);
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        if let Err(status) = unsafe {
            write_box(
                out_evaluation,
                LuminateAuthorizationEvaluation(decision),
                "authorization evaluation",
            )
        } {
            return status;
        }
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Whether the evaluated request is allowed. Returns `false` if `v` is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_authorization_evaluation_is_allowed(
    v: *const LuminateAuthorizationEvaluation,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.is_some_and(|v| v.0.is_allowed())
}

/// The decision's validated caller-safe diagnostic, or an absent view.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_authorization_evaluation_reason(
    v: *const LuminateAuthorizationEvaluation,
) -> LuminateStringView {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(null_view(), |v| optional_sv(v.0.reason.as_deref()))
}

/// The stable identifier of the matching rule, or an absent view for a
/// default denial.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_authorization_evaluation_audit_rule(
    v: *const LuminateAuthorizationEvaluation,
) -> LuminateStringView {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(null_view(), |v| {
        optional_sv(v.0.audit_rule.as_ref().map(RuleId::as_str))
    })
}

/// Whether the decision carries a cache lifetime hint.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_authorization_evaluation_has_cache_hint_ms(
    v: *const LuminateAuthorizationEvaluation,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.is_some_and(|v| v.0.cache_hint.is_some())
}

/// The decision's cache lifetime hint in milliseconds, clamped to
/// `luminate_core::policy::MAX_CACHE_HINT`; `0` if absent or `v` is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_authorization_evaluation_cache_hint_ms(
    v: *const LuminateAuthorizationEvaluation,
) -> u64 {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }
        .and_then(|v| v.0.cache_hint)
        .map_or(0, |hint| {
            u64::try_from(hint.as_millis()).unwrap_or(u64::MAX)
        })
}

/// The policy revision under which the decision was made.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_authorization_evaluation_revision(
    v: *const LuminateAuthorizationEvaluation,
) -> u64 {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(0, |v| v.0.revision.0)
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
