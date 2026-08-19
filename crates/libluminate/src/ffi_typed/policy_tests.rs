// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "Safety is documented before each C-boundary assertion; Clippy cannot associate comments outside assertion macro expansions."
)]

use super::*;
use std::ffi::CString;

fn bytes(view: LuminateStringView) -> Option<&'static [u8]> {
    if view.data.is_null() {
        None
    } else {
        // SAFETY: test inputs remain alive while their borrowed views are inspected.
        Some(unsafe { std::slice::from_raw_parts(view.data.cast(), view.len) })
    }
}

fn text(view: LuminateStringView) -> Option<String> {
    bytes(view).map(|b| String::from_utf8(b.to_vec()).expect("valid UTF-8"))
}

fn new_builder(revision: u64) -> *mut LuminatePolicyDocumentBuilder {
    let mut builder = ptr::null_mut();
    // SAFETY: `builder` is a writable output pointer.
    assert_eq!(
        unsafe { luminate_policy_document_builder_new(revision, &raw mut builder) },
        LuminateStatus::Ok
    );
    builder
}

fn new_principal(authority: &str, subject: &str, groups: &[&str]) -> *mut LuminateRemotePrincipal {
    let authority = CString::new(authority).expect("no interior NUL");
    let subject = CString::new(subject).expect("no interior NUL");
    let group_cstrings: Vec<CString> = groups
        .iter()
        .map(|g| CString::new(*g).expect("no interior NUL"))
        .collect();
    let group_ptrs: Vec<*const c_char> = group_cstrings.iter().map(|c| c.as_ptr()).collect();
    let mut principal = ptr::null_mut();
    // SAFETY: all pointers are valid for the duration of the call.
    assert_eq!(
        unsafe {
            luminate_remote_principal_new(
                authority.as_ptr(),
                subject.as_ptr(),
                group_ptrs.as_ptr(),
                group_ptrs.len(),
                &raw mut principal,
            )
        },
        LuminateStatus::Ok
    );
    principal
}

fn empty_resource_constraints() -> LuminateResourceConstraintsInput {
    LuminateResourceConstraintsInput {
        device_ids: ptr::null(),
        device_id_count: 0,
        provider_instances: ptr::null(),
        provider_instance_count: 0,
        has_host_attached: false,
        host_attached: false,
        collections: ptr::null(),
        collection_count: 0,
    }
}

fn add_allow_rule(
    builder: *mut LuminatePolicyDocumentBuilder,
    role_name: &str,
    id: &str,
    operations: &[u32],
) {
    let role_c = CString::new(role_name).expect("no interior NUL");
    let id_c = CString::new(id).expect("no interior NUL");
    let rule = LuminateRuleInput {
        id: id_c.as_ptr(),
        effect: 0,
        operations: operations.as_ptr(),
        operation_count: operations.len(),
        resources: empty_resource_constraints(),
        reason: ptr::null(),
        has_cache_hint_ms: false,
        cache_hint_ms: 0,
    };
    // SAFETY: all pointers are valid for the duration of the call.
    assert_eq!(
        unsafe {
            luminate_policy_document_builder_role_add_rule(
                builder,
                role_c.as_ptr(),
                &raw const rule,
            )
        },
        LuminateStatus::Ok
    );
}

fn add_binding(
    builder: *mut LuminatePolicyDocumentBuilder,
    authority: &str,
    subjects: &[&str],
    roles: &[&str],
) {
    let authority_c = CString::new(authority).expect("no interior NUL");
    let subject_cstrings: Vec<CString> = subjects
        .iter()
        .map(|s| CString::new(*s).expect("no interior NUL"))
        .collect();
    let subject_ptrs: Vec<*const c_char> = subject_cstrings.iter().map(|c| c.as_ptr()).collect();
    let role_cstrings: Vec<CString> = roles
        .iter()
        .map(|r| CString::new(*r).expect("no interior NUL"))
        .collect();
    let role_ptrs: Vec<*const c_char> = role_cstrings.iter().map(|c| c.as_ptr()).collect();
    // SAFETY: all pointers are valid for the duration of the call.
    assert_eq!(
        unsafe {
            luminate_policy_document_builder_add_binding(
                builder,
                authority_c.as_ptr(),
                subject_ptrs.as_ptr(),
                subject_ptrs.len(),
                ptr::null(),
                0,
                role_ptrs.as_ptr(),
                role_ptrs.len(),
            )
        },
        LuminateStatus::Ok
    );
}

fn build(
    builder: *const LuminatePolicyDocumentBuilder,
) -> Result<*mut LuminatePolicyDocument, LuminateStatus> {
    let mut document = ptr::null_mut();
    // SAFETY: `document` is a writable output pointer.
    let status = unsafe { luminate_policy_document_build(builder, &raw mut document) };
    if status == LuminateStatus::Ok {
        Ok(document)
    } else {
        Err(status)
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "The test exercises one seeded builder through every related edit and lifetime boundary."
)]
fn seeded_builder_is_independent_and_supports_policy_edits() {
    let original_builder = new_builder(12);
    add_allow_rule(original_builder, "viewer", "view", &[0]);
    add_allow_rule(original_builder, "viewer", "refresh", &[1]);
    add_allow_rule(original_builder, "unused", "unused-rule", &[0]);
    let viewer = CString::new("viewer").expect("valid string");
    let base = CString::new("base").expect("valid string");
    // SAFETY: both role strings remain live for the call.
    assert_eq!(
        unsafe {
            luminate_policy_document_builder_role_add_parent(
                original_builder,
                viewer.as_ptr(),
                base.as_ptr(),
            )
        },
        LuminateStatus::Ok
    );
    add_binding(original_builder, "local", &["alice"], &["viewer"]);
    add_binding(original_builder, "local", &["nobody"], &["unused"]);
    let original = build(original_builder).expect("valid original document");
    let mut seeded = ptr::null_mut();

    // SAFETY: the document and output pointer are live.
    unsafe {
        assert_eq!(
            luminate_policy_document_builder_from_document(original, ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_policy_document_builder_from_document(original, &raw mut seeded),
            LuminateStatus::Ok
        );
    }
    let round_trip = build(seeded).expect("unmodified seeded document remains valid");
    let principal = new_principal("local", "alice", &[]);
    // SAFETY: all three owned policy values remain live for the comparison.
    unsafe {
        assert_eq!((*round_trip).0.source(), (*original).0.source());
        assert_eq!(
            (*round_trip)
                .0
                .evaluate(&(*principal).0, Operation::Observe, &[]),
            (*original)
                .0
                .evaluate(&(*principal).0, Operation::Observe, &[])
        );
        luminate_remote_principal_free(principal);
        luminate_policy_document_free(round_trip);
    }
    // SAFETY: the original roots are independently owned and no longer needed.
    unsafe {
        luminate_policy_document_free(original);
        luminate_policy_document_builder_free(original_builder);
    }

    let unused = CString::new("unused").expect("valid string");
    let replacement_id = CString::new("control").expect("valid string");
    let replacement_operations = [2_u32];
    let replacement = LuminateRuleInput {
        id: replacement_id.as_ptr(),
        effect: 0,
        operations: replacement_operations.as_ptr(),
        operation_count: replacement_operations.len(),
        resources: empty_resource_constraints(),
        reason: ptr::null(),
        has_cache_hint_ms: false,
        cache_hint_ms: 0,
    };
    let authority = CString::new("local").expect("valid string");
    let bob = CString::new("bob").expect("valid string");
    let subjects = [bob.as_ptr()];
    let roles = [viewer.as_ptr()];

    // SAFETY: all builder inputs remain live for their calls.
    unsafe {
        assert_eq!(
            luminate_policy_document_builder_set_revision(seeded, 13),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_policy_document_builder_role_replace_rule(
                seeded,
                viewer.as_ptr(),
                0,
                &raw const replacement,
            ),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_policy_document_builder_role_remove_rule(seeded, viewer.as_ptr(), 1),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_policy_document_builder_role_remove_parent(
                seeded,
                viewer.as_ptr(),
                base.as_ptr(),
            ),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_policy_document_builder_remove_role(seeded, base.as_ptr()),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_policy_document_builder_remove_binding(seeded, 1),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_policy_document_builder_replace_binding(
                seeded,
                0,
                authority.as_ptr(),
                subjects.as_ptr(),
                subjects.len(),
                ptr::null(),
                0,
                roles.as_ptr(),
                roles.len(),
            ),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_policy_document_builder_remove_role(seeded, unused.as_ptr()),
            LuminateStatus::Ok
        );
    }

    let edited = build(seeded).expect("edited seeded document remains valid");
    // SAFETY: the edited document remains live for all accessor calls.
    unsafe {
        assert_eq!(luminate_policy_document_revision(edited), 13);
        assert_eq!(luminate_policy_document_role_count(edited), 1);
        assert_eq!(luminate_policy_document_binding_count(edited), 1);
        let role = luminate_policy_document_role_at(edited, 0);
        assert_eq!(luminate_policy_role_rule_count(role), 1);
        assert_eq!(
            text(luminate_policy_rule_id(luminate_policy_role_rule_at(
                role, 0
            ))),
            Some("control".to_owned())
        );

        assert_eq!(
            luminate_policy_document_builder_remove_binding(seeded, 2),
            LuminateStatus::InvalidArgument
        );
        assert_eq!(
            luminate_policy_document_builder_role_remove_rule(seeded, viewer.as_ptr(), 2),
            LuminateStatus::InvalidArgument
        );
        luminate_policy_document_free(edited);
        luminate_policy_document_builder_free(seeded);
    }
}

#[test]
fn builder_produces_a_document_that_allows_a_bound_subject() {
    let builder = new_builder(7);
    add_allow_rule(builder, "viewer", "view", &[0]); // LUMINATE_POLICY_OP_OBSERVE
    add_binding(builder, "oidc.example", &["alice"], &["viewer"]);

    let document = build(builder).expect("valid document");
    assert_eq!(unsafe { luminate_policy_document_revision(document) }, 7);

    let principal = new_principal("oidc.example", "alice", &[]);
    let mut evaluation = ptr::null_mut();
    // SAFETY: all pointers are valid for the duration of the call.
    let status = unsafe {
        luminate_policy_document_evaluate(
            document,
            principal,
            0,
            ptr::null(),
            0,
            &raw mut evaluation,
        )
    };
    assert_eq!(status, LuminateStatus::Ok);
    // SAFETY: `evaluation` was just populated.
    unsafe {
        assert!(luminate_authorization_evaluation_is_allowed(evaluation));
        assert_eq!(
            text(luminate_authorization_evaluation_audit_rule(evaluation)),
            Some("view".to_owned())
        );
        assert_eq!(luminate_authorization_evaluation_revision(evaluation), 7);
        luminate_authorization_evaluation_free(evaluation);
        luminate_remote_principal_free(principal);
        luminate_policy_document_free(document);
        luminate_policy_document_builder_free(builder);
    }
}

#[test]
fn unmatched_principal_is_denied_by_default() {
    let builder = new_builder(1);
    add_allow_rule(builder, "viewer", "view", &[0]);
    add_binding(builder, "oidc.example", &["alice"], &["viewer"]);
    let document = build(builder).expect("valid document");

    let principal = new_principal("oidc.example", "mallory", &[]);
    let mut evaluation = ptr::null_mut();
    // SAFETY: all pointers are valid for the duration of the call.
    let status = unsafe {
        luminate_policy_document_evaluate(
            document,
            principal,
            0,
            ptr::null(),
            0,
            &raw mut evaluation,
        )
    };
    assert_eq!(status, LuminateStatus::Ok);
    // SAFETY: `evaluation` was just populated.
    unsafe {
        assert!(!luminate_authorization_evaluation_is_allowed(evaluation));
        assert!(
            text(luminate_authorization_evaluation_audit_rule(evaluation)).is_none(),
            "default denial has no matching rule"
        );
        luminate_authorization_evaluation_free(evaluation);
        luminate_remote_principal_free(principal);
        luminate_policy_document_free(document);
        luminate_policy_document_builder_free(builder);
    }
}

#[test]
fn build_rejects_a_binding_that_grants_a_missing_role() {
    let builder = new_builder(1);
    add_binding(builder, "oidc.example", &["alice"], &["ghost-role"]);

    let error = build(builder).expect_err("missing role must fail validation");
    assert_eq!(error, LuminateStatus::InvalidArgument);
    // SAFETY: `builder` was never freed elsewhere in this test.
    unsafe { luminate_policy_document_builder_free(builder) };
}

#[test]
fn build_rejects_a_role_inheritance_cycle() {
    let builder = new_builder(1);
    let a = CString::new("a").expect("no interior NUL");
    let b = CString::new("b").expect("no interior NUL");
    // SAFETY: all pointers are valid for the duration of each call.
    unsafe {
        assert_eq!(
            luminate_policy_document_builder_role_add_parent(builder, a.as_ptr(), b.as_ptr()),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_policy_document_builder_role_add_parent(builder, b.as_ptr(), a.as_ptr()),
            LuminateStatus::Ok
        );
    }

    let error = build(builder).expect_err("role cycle must fail validation");
    assert_eq!(error, LuminateStatus::InvalidArgument);
    // SAFETY: `builder` was never freed elsewhere in this test.
    unsafe { luminate_policy_document_builder_free(builder) };
}

#[test]
fn cache_hint_round_trips() {
    let builder = new_builder(1);
    let role_name = CString::new("operator").expect("no interior NUL");
    let id = CString::new("allow-observe").expect("no interior NUL");
    let operations = [0_u32];
    let rule = LuminateRuleInput {
        id: id.as_ptr(),
        effect: 0,
        operations: operations.as_ptr(),
        operation_count: operations.len(),
        resources: empty_resource_constraints(),
        reason: ptr::null(),
        has_cache_hint_ms: true,
        cache_hint_ms: 60_000,
    };
    // SAFETY: pointers are valid for the duration of the call.
    unsafe {
        assert_eq!(
            luminate_policy_document_builder_role_add_rule(
                builder,
                role_name.as_ptr(),
                &raw const rule,
            ),
            LuminateStatus::Ok
        );
    }
    add_binding(builder, "oidc.example", &["bob"], &["operator"]);
    let document = build(builder).expect("valid document");

    let principal = new_principal("oidc.example", "bob", &[]);
    let mut evaluation = ptr::null_mut();
    // SAFETY: all pointers are valid for the duration of the call.
    let status = unsafe {
        luminate_policy_document_evaluate(
            document,
            principal,
            0,
            ptr::null(),
            0,
            &raw mut evaluation,
        )
    };
    assert_eq!(status, LuminateStatus::Ok);
    // SAFETY: `evaluation` was just populated.
    unsafe {
        assert!(luminate_authorization_evaluation_is_allowed(evaluation));
        assert!(luminate_authorization_evaluation_has_cache_hint_ms(
            evaluation
        ));
        assert_eq!(
            luminate_authorization_evaluation_cache_hint_ms(evaluation),
            60_000
        );
        luminate_authorization_evaluation_free(evaluation);
        luminate_remote_principal_free(principal);
        luminate_policy_document_free(document);
        luminate_policy_document_builder_free(builder);
    }
}

#[test]
fn principal_rejects_empty_authority() {
    let mut principal = ptr::null_mut();
    let authority = CString::new("").expect("no interior NUL");
    let subject = CString::new("alice").expect("no interior NUL");
    // SAFETY: all pointers are valid for the duration of the call.
    let status = unsafe {
        luminate_remote_principal_new(
            authority.as_ptr(),
            subject.as_ptr(),
            ptr::null(),
            0,
            &raw mut principal,
        )
    };
    assert_eq!(status, LuminateStatus::InvalidArgument);
    assert!(principal.is_null());
}

#[test]
fn null_builder_pointer_is_rejected() {
    let mut document = ptr::null_mut();
    // SAFETY: a deliberately null builder pointer exercises the null-check path.
    let status = unsafe { luminate_policy_document_build(ptr::null(), &raw mut document) };
    assert_eq!(status, LuminateStatus::NullPointer);
    assert!(document.is_null());
}

#[test]
fn frees_are_null_safe() {
    // SAFETY: every argument here is deliberately null.
    unsafe {
        luminate_policy_document_builder_free(ptr::null_mut());
        luminate_policy_document_free(ptr::null_mut());
        luminate_remote_principal_free(ptr::null_mut());
        luminate_authorization_evaluation_free(ptr::null_mut());
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "One built document stays alive while every borrowed policy and principal view is checked."
)]
fn document_and_principal_accessors_round_trip_every_policy_field() {
    let builder = new_builder(3);
    let base = CString::new("base").expect("no interior NUL");
    let operator = CString::new("operator").expect("no interior NUL");
    let rule_id = CString::new("restricted-control").expect("no interior NUL");
    let reason = CString::new("maintenance window").expect("no interior NUL");
    let operations: Vec<u32> = (0..=10).collect();
    let device = CString::new("keyboard").expect("no interior NUL");
    let provider = CString::new("usb").expect("no interior NUL");
    let collection = CString::new("desk").expect("no interior NUL");
    let devices = [device.as_ptr()];
    let providers = [provider.as_ptr()];
    let collections = [collection.as_ptr()];
    let rule = LuminateRuleInput {
        id: rule_id.as_ptr(),
        effect: 1,
        operations: operations.as_ptr(),
        operation_count: operations.len(),
        resources: LuminateResourceConstraintsInput {
            device_ids: devices.as_ptr(),
            device_id_count: devices.len(),
            provider_instances: providers.as_ptr(),
            provider_instance_count: providers.len(),
            has_host_attached: true,
            host_attached: true,
            collections: collections.as_ptr(),
            collection_count: collections.len(),
        },
        reason: reason.as_ptr(),
        has_cache_hint_ms: true,
        cache_hint_ms: 250,
    };
    let authority = CString::new("oidc.example").expect("no interior NUL");
    let subject = CString::new("alice").expect("no interior NUL");
    let group = CString::new("operators").expect("no interior NUL");
    let subjects = [subject.as_ptr()];
    let groups = [group.as_ptr()];
    let roles = [operator.as_ptr()];

    // SAFETY: all pointers refer to live local values for each call.
    unsafe {
        assert_eq!(
            luminate_policy_document_builder_role_add_parent(
                builder,
                operator.as_ptr(),
                base.as_ptr()
            ),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_policy_document_builder_role_add_rule(
                builder,
                operator.as_ptr(),
                &raw const rule
            ),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_policy_document_builder_add_binding(
                builder,
                authority.as_ptr(),
                subjects.as_ptr(),
                subjects.len(),
                groups.as_ptr(),
                groups.len(),
                roles.as_ptr(),
                roles.len(),
            ),
            LuminateStatus::Ok
        );
        assert_eq!(
            luminate_policy_document_builder_set_revision(builder, 9),
            LuminateStatus::Ok
        );
    }

    let document = build(builder).expect("valid document");
    // `base` sorts before `operator`.
    let role_index = 1;
    // SAFETY: `document` remains alive throughout the accessor checks.
    unsafe {
        assert_eq!(luminate_policy_document_revision(document), 9);
        assert_eq!(luminate_policy_document_role_count(document), 2);
        assert_eq!(
            text(luminate_policy_document_role_name_at(document, role_index)),
            Some("operator".to_owned())
        );
        let role = luminate_policy_document_role_at(document, role_index);
        assert!(!role.is_null());
        assert_eq!(luminate_policy_role_parent_count(role), 1);
        assert_eq!(
            text(luminate_policy_role_parent_at(role, 0)),
            Some("base".to_owned())
        );
        assert_eq!(luminate_policy_role_rule_count(role), 1);
        let rule = luminate_policy_role_rule_at(role, 0);
        assert!(!rule.is_null());
        assert_eq!(
            text(luminate_policy_rule_id(rule)),
            Some("restricted-control".to_owned())
        );
        assert_eq!(luminate_policy_rule_effect(rule), 1);
        assert_eq!(luminate_policy_rule_operation_count(rule), operations.len());
        for (index, expected) in operations.iter().copied().enumerate() {
            assert_eq!(luminate_policy_rule_operation_at(rule, index), expected);
        }
        assert_eq!(
            text(luminate_policy_rule_reason(rule)),
            Some("maintenance window".to_owned())
        );
        let mut cache_hint_ms = 0;
        assert!(luminate_policy_rule_cache_hint_ms(
            rule,
            &raw mut cache_hint_ms
        ));
        assert_eq!(cache_hint_ms, 250);
        assert_eq!(luminate_policy_rule_device_id_count(rule), 1);
        assert_eq!(
            text(luminate_policy_rule_device_id_at(rule, 0)),
            Some("keyboard".to_owned())
        );
        assert_eq!(luminate_policy_rule_provider_instance_count(rule), 1);
        assert_eq!(
            text(luminate_policy_rule_provider_instance_at(rule, 0)),
            Some("usb".to_owned())
        );
        assert_eq!(luminate_policy_rule_collection_count(rule), 1);
        assert_eq!(
            text(luminate_policy_rule_collection_at(rule, 0)),
            Some("desk".to_owned())
        );
        let mut host_attached = false;
        assert!(luminate_policy_rule_host_attached(
            rule,
            &raw mut host_attached
        ));
        assert!(host_attached);

        assert_eq!(luminate_policy_document_binding_count(document), 1);
        let binding = luminate_policy_document_binding_at(document, 0);
        assert!(!binding.is_null());
        assert_eq!(
            text(luminate_policy_binding_authority(binding)),
            Some("oidc.example".to_owned())
        );
        assert_eq!(luminate_policy_binding_subject_count(binding), 1);
        assert_eq!(
            text(luminate_policy_binding_subject_at(binding, 0)),
            Some("alice".to_owned())
        );
        assert_eq!(luminate_policy_binding_group_count(binding), 1);
        assert_eq!(
            text(luminate_policy_binding_group_at(binding, 0)),
            Some("operators".to_owned())
        );
        assert_eq!(luminate_policy_binding_role_count(binding), 1);
        assert_eq!(
            text(luminate_policy_binding_role_at(binding, 0)),
            Some("operator".to_owned())
        );
    }

    let principal = new_principal("oidc.example", "alice", &["operators", "admins"]);
    // SAFETY: `principal` remains alive throughout the accessor checks.
    unsafe {
        assert_eq!(
            text(luminate_remote_principal_authority(principal)),
            Some("oidc.example".to_owned())
        );
        assert_eq!(
            text(luminate_remote_principal_subject(principal)),
            Some("alice".to_owned())
        );
        assert_eq!(luminate_remote_principal_group_count(principal), 2);
        assert_eq!(
            text(luminate_remote_principal_group_at(principal, 0)),
            Some("admins".to_owned())
        );
        assert_eq!(
            text(luminate_remote_principal_group_at(principal, 1)),
            Some("operators".to_owned())
        );

        luminate_remote_principal_free(principal);
        luminate_policy_document_free(document);
        luminate_policy_document_builder_free(builder);
    }
}

#[test]
fn policy_inputs_and_out_of_range_accessors_fail_safely() {
    let builder = new_builder(1);
    let role = CString::new("role").expect("no interior NUL");
    let invalid_rule_id = CString::new("").expect("no interior NUL");
    let invalid_operations = [u32::MAX];
    let invalid_rule = LuminateRuleInput {
        id: invalid_rule_id.as_ptr(),
        effect: 0,
        operations: ptr::null(),
        operation_count: 0,
        resources: empty_resource_constraints(),
        reason: ptr::null(),
        has_cache_hint_ms: false,
        cache_hint_ms: 0,
    };

    // SAFETY: non-null pointers refer to live local values; nulls deliberately
    // exercise each C boundary's validation.
    unsafe {
        assert_eq!(
            luminate_policy_document_builder_new(0, ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_policy_document_builder_set_revision(ptr::null_mut(), 2),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_policy_document_builder_role_add_parent(builder, ptr::null(), role.as_ptr()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_policy_document_builder_role_add_rule(builder, role.as_ptr(), ptr::null()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_policy_document_builder_role_add_rule(
                builder,
                role.as_ptr(),
                &raw const invalid_rule
            ),
            LuminateStatus::InvalidArgument
        );

        let bad_operation_rule = LuminateRuleInput {
            id: role.as_ptr(),
            effect: 0,
            operations: invalid_operations.as_ptr(),
            operation_count: 1,
            resources: empty_resource_constraints(),
            reason: ptr::null(),
            has_cache_hint_ms: false,
            cache_hint_ms: 0,
        };
        assert_eq!(
            luminate_policy_document_builder_role_add_rule(
                builder,
                role.as_ptr(),
                &raw const bad_operation_rule
            ),
            LuminateStatus::InvalidArgument
        );

        let document = build(builder).expect("empty document is valid");
        assert_eq!(luminate_policy_document_role_count(ptr::null()), 0);
        assert!(text(luminate_policy_document_role_name_at(document, 99)).is_none());
        assert!(luminate_policy_document_role_at(document, 99).is_null());
        assert_eq!(luminate_policy_rule_effect(ptr::null()), u32::MAX);
        assert_eq!(luminate_policy_rule_operation_at(ptr::null(), 99), u32::MAX);
        let mut host_attached = true;
        assert!(!luminate_policy_rule_host_attached(
            ptr::null(),
            &raw mut host_attached
        ));
        assert!(host_attached);
        assert!(luminate_policy_document_binding_at(document, 99).is_null());
        assert!(text(luminate_policy_binding_authority(ptr::null())).is_none());

        luminate_policy_document_free(document);
        luminate_policy_document_builder_free(builder);
    }
}
