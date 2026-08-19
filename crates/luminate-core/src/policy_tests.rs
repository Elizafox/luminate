// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::future::Future;
use std::slice;
use std::task::{Context, Poll, Waker};

use super::*;

#[test]
fn principal_id_rejects_empty_components() {
    assert!(PrincipalId::new("", "alice").is_err());
    assert!(PrincipalId::new("oidc.example", "").is_err());
}

#[test]
fn principal_uses_canonical_identity() {
    let id = PrincipalId::new("oidc.example", "alice").expect("valid identity");
    let principal =
        Principal::from_id(id.clone(), ["operators".to_owned()]).expect("valid principal");

    assert_eq!(principal.id(), &id);
    assert_eq!(principal.authority(), "oidc.example");
    assert_eq!(principal.subject(), "alice");
    assert!(principal.groups().contains("operators"));
}

#[test]
fn identity_bounds_and_session_lease_boundaries_fail_closed() {
    let too_many_groups = (0..=MAX_VERIFIED_GROUPS)
        .map(|index| format!("group-{index}"))
        .collect::<Vec<_>>();
    assert!(Principal::new("oidc.example", "alice", too_many_groups).is_err());
    assert!(Principal::new("oidc.example", "alice", [String::new()]).is_err());

    let unix = Actor::Unix {
        uid: 42,
        gid: 7,
        pid: None,
    };
    let windows = Actor::Windows {
        sid: "S-1-5-21-42".to_owned(),
        pid: Some(10),
    };
    assert_eq!(
        unix.principal_id().expect("valid Unix actor").authority(),
        "unix"
    );
    assert_eq!(
        windows
            .principal_id()
            .expect("valid Windows actor")
            .subject(),
        "S-1-5-21-42"
    );
    assert!(
        Actor::Windows {
            sid: String::new(),
            pid: None,
        }
        .principal_id()
        .is_err()
    );

    let expiry = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
    assert!(AuthenticationLease::unlimited().is_valid_at(expiry));
    assert!(AuthenticationLease::until(expiry).is_valid_at(expiry - Duration::from_secs(1)));
    assert!(!AuthenticationLease::until(expiry).is_valid_at(expiry));
}

#[test]
fn frontend_actor_round_trip_revalidates_credential_identity() {
    let principal = Principal::new("service", "http", ["frontends".to_owned()])
        .expect("valid service principal");
    let actor = FrontendActor::new(principal.clone(), "credential-1").expect("valid actor");
    assert_eq!(actor.principal(), &principal);
    assert_eq!(actor.credential_id(), "credential-1");

    let encoded = serde_json::to_value(&actor).expect("serialize actor");
    assert_eq!(
        serde_json::from_value::<FrontendActor>(encoded).expect("deserialize actor"),
        actor
    );
    assert!(FrontendActor::new(principal, "").is_err());
    assert!(
        serde_json::from_value::<FrontendActor>(serde_json::json!({
            "principal": {"authority": "service", "subject": "http", "groups": []},
            "credential_id": ""
        }))
        .is_err()
    );
}

#[test]
fn session_scope_unions_allow_only_grants() {
    let device = DeviceId::new("device-a");
    let resource = Resource {
        device_id: device.clone(),
        provider_instance: Some("demo".to_owned()),
        host_attached: false,
        collections: BTreeSet::new(),
    };
    let scope = SessionScope::new(vec![
        ScopeGrant {
            operations: BTreeSet::from([Operation::Observe]),
            resources: ResourceConstraints::default(),
        },
        ScopeGrant {
            operations: BTreeSet::from([Operation::Control]),
            resources: ResourceConstraints {
                device_ids: BTreeSet::from([device]),
                ..ResourceConstraints::default()
            },
        },
    ])
    .expect("valid scope");

    assert!(scope.allows(Operation::Observe, slice::from_ref(&resource)));
    assert!(scope.allows(Operation::Control, slice::from_ref(&resource)));
    assert!(!scope.allows(Operation::ManagePolicy, &[]));
}

#[test]
fn matching_limit_ceilings_use_most_restrictive_values() {
    let global = LimitCeilings {
        connections: Some(20),
        subscriptions: None,
    };
    let principal = LimitCeilings {
        connections: Some(2),
        subscriptions: Some(1),
    };

    assert_eq!(
        global.most_restrictive(principal),
        LimitCeilings {
            connections: Some(2),
            subscriptions: Some(1),
        }
    );
}

#[test]
fn removed_limit_and_role_quota_fields_are_rejected() {
    assert!(serde_json::from_str::<LimitCeilings>(r#"{"concurrent_requests":4}"#).is_err());
    assert!(serde_json::from_str::<Role>(r#"{"parents":[],"rules":[],"quotas":{}}"#).is_err());
}

#[test]
fn session_evaluation_intersects_lease_scope_and_policy() {
    let document = PolicyDocument::new(source()).expect("valid policy");
    let subject = Principal::new("oidc.example", "alice", []).expect("valid principal");
    let session = AuthenticatedSession {
        actor: Actor::Unix {
            uid: 1000,
            gid: 1000,
            pid: Some(42),
        },
        subject,
        source: AuthenticationSource::Bearer,
        credential_id: Some("credential-1".to_owned()),
        lease: AuthenticationLease::until(SystemTime::UNIX_EPOCH + Duration::from_secs(20)),
        scope: SessionScope::new(vec![ScopeGrant {
            operations: BTreeSet::from([Operation::Observe]),
            resources: ResourceConstraints::default(),
        }])
        .expect("valid scope"),
    };

    assert!(
        document
            .evaluate_session(
                &session,
                Operation::Observe,
                &[],
                SystemTime::UNIX_EPOCH + Duration::from_secs(10)
            )
            .is_allowed()
    );
    assert!(
        !document
            .evaluate_session(
                &session,
                Operation::Control,
                &[],
                SystemTime::UNIX_EPOCH + Duration::from_secs(10)
            )
            .is_allowed()
    );
    assert!(
        !document
            .evaluate_session(
                &session,
                Operation::Observe,
                &[],
                SystemTime::UNIX_EPOCH + Duration::from_secs(20)
            )
            .is_allowed()
    );
}

#[test]
fn presets_are_materialized_and_recovery_requires_direct_peer() {
    let source = materialize_presets(PolicyRevision(1), [Preset::Viewer, Preset::Administrator]);
    let document = PolicyDocument::new(source).expect("valid presets");
    assert!(document.roles().contains_key("viewer"));
    assert!(document.roles().contains_key("administrator"));
    assert!(
        document.roles()["administrator"].rules[0]
            .operations
            .contains(&Operation::ManageAuthentication)
    );

    let principal = Principal::new("local", "recovery", []).expect("valid principal");
    let recovery = RecoveryPolicy::new(BTreeSet::from([principal.id().clone()]));
    let peer = AuthenticatedSession {
        actor: Actor::Unix {
            uid: 0,
            gid: 0,
            pid: None,
        },
        subject: principal.clone(),
        source: AuthenticationSource::Peer,
        credential_id: None,
        lease: AuthenticationLease::unlimited(),
        scope: SessionScope::new(vec![ScopeGrant {
            operations: BTreeSet::from([Operation::ManagePolicy]),
            resources: ResourceConstraints::default(),
        }])
        .expect("valid scope"),
    };
    assert!(recovery.allows(&peer, Operation::ManagePolicy));

    let mut delegated = peer;
    delegated.source = AuthenticationSource::Bearer;
    assert!(!recovery.allows(&delegated, Operation::ManagePolicy));
}

fn ready<T>(future: impl Future<Output = T>) -> T {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = Box::pin(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("in-memory policy future unexpectedly pending"),
    }
}

fn rule(id: &str) -> Rule {
    Rule {
        id: RuleId::new(id).expect("valid rule ID"),
        effect: RuleEffect::Allow,
        operations: BTreeSet::from([Operation::Observe]),
        resources: ResourceConstraints::default(),
        reason: None,
        cache_hint: None,
    }
}

fn source() -> PolicyDocumentSource {
    PolicyDocumentSource {
        revision: PolicyRevision(7),
        roles: BTreeMap::from([(
            "viewer".to_owned(),
            Role {
                parents: BTreeSet::new(),
                rules: vec![rule("view")],
            },
        )]),
        bindings: vec![Binding {
            authority: "oidc.example".to_owned(),
            subjects: BTreeSet::from(["alice".to_owned()]),
            groups: BTreeSet::new(),
            roles: BTreeSet::from(["viewer".to_owned()]),
        }],
    }
}

fn recovery_source() -> PolicyDocumentSource {
    PolicyDocumentSource {
        revision: PolicyRevision(1),
        roles: BTreeMap::from([(
            "recovery".to_owned(),
            Role {
                parents: BTreeSet::new(),
                rules: vec![Rule {
                    id: RuleId::new("recover-policy").expect("valid rule ID"),
                    effect: RuleEffect::Allow,
                    operations: BTreeSet::from([
                        Operation::ManagePolicy,
                        Operation::ManageAuthentication,
                        Operation::AdministerFrontend,
                    ]),
                    resources: ResourceConstraints::default(),
                    reason: None,
                    cache_hint: None,
                }],
            },
        )]),
        bindings: vec![Binding {
            authority: "local".to_owned(),
            subjects: BTreeSet::from(["recovery".to_owned()]),
            groups: BTreeSet::new(),
            roles: BTreeSet::from(["recovery".to_owned()]),
        }],
    }
}

#[test]
fn rejects_missing_parent() {
    let mut source = source();
    source
        .roles
        .get_mut("viewer")
        .expect("viewer role")
        .parents
        .insert("missing".to_owned());

    let error = PolicyDocument::new(source).expect_err("missing parent must fail");

    assert!(error.to_string().contains("missing parent"));
}

#[test]
fn rejects_inheritance_cycle() {
    let mut source = source();
    source.roles.insert(
        "loop".to_owned(),
        Role {
            parents: BTreeSet::from(["viewer".to_owned()]),
            rules: Vec::new(),
        },
    );
    source
        .roles
        .get_mut("viewer")
        .expect("viewer role")
        .parents
        .insert("loop".to_owned());

    let error = PolicyDocument::new(source).expect_err("cycle must fail");

    assert!(error.to_string().contains("cycle"));
}

#[test]
fn rejects_duplicate_rule_ids_across_roles() {
    let mut source = source();
    source.roles.insert(
        "other".to_owned(),
        Role {
            parents: BTreeSet::new(),
            rules: vec![rule("view")],
        },
    );

    let error = PolicyDocument::new(source).expect_err("duplicate rule ID must fail");

    assert!(error.to_string().contains("duplicate rule ID"));
}

#[test]
fn rejects_selectorless_binding() {
    let mut source = source();
    source.bindings[0].subjects.clear();

    let error = PolicyDocument::new(source).expect_err("selectorless binding must fail");

    assert!(error.to_string().contains("subject or group"));
}

#[test]
fn policy_validation_rejects_empty_rules_constraints_and_binding_grants() {
    let mut empty_operations = source();
    empty_operations
        .roles
        .get_mut("viewer")
        .expect("role")
        .rules[0]
        .operations
        .clear();
    assert!(PolicyDocument::new(empty_operations).is_err());

    let mut empty_reason = source();
    empty_reason.roles.get_mut("viewer").expect("role").rules[0].reason = Some(String::new());
    assert!(PolicyDocument::new(empty_reason).is_err());

    let mut empty_provider = source();
    empty_provider.roles.get_mut("viewer").expect("role").rules[0]
        .resources
        .provider_instances
        .insert(String::new());
    assert!(PolicyDocument::new(empty_provider).is_err());

    let mut no_roles = source();
    no_roles.bindings[0].roles.clear();
    assert!(PolicyDocument::new(no_roles).is_err());

    let mut missing_role = source();
    missing_role.bindings[0].roles = BTreeSet::from(["missing".to_owned()]);
    assert!(PolicyDocument::new(missing_role).is_err());
}

#[test]
fn session_scope_validation_rejects_empty_grants_during_construction_and_decode() {
    assert!(SessionScope::new(Vec::new()).is_err());
    assert!(
        SessionScope::new(vec![ScopeGrant {
            operations: BTreeSet::new(),
            resources: ResourceConstraints::default(),
        }])
        .is_err()
    );
    assert!(serde_json::from_value::<SessionScope>(serde_json::json!([])).is_err());
    assert!(
        serde_json::from_value::<SessionScope>(serde_json::json!([{
            "operations": [],
            "resources": {}
        }]))
        .is_err()
    );
}

#[test]
fn deserialize_revalidates_document() {
    let invalid = serde_json::json!({
        "revision": 1,
        "roles": {},
        "bindings": [{
            "authority": "example",
            "subjects": ["alice"],
            "groups": [],
            "roles": ["missing"]
        }]
    });

    let error = serde_json::from_value::<PolicyDocument>(invalid).expect_err("invalid document");

    assert!(error.to_string().contains("missing role"));
}

#[test]
fn policy_serialization_is_deterministic() {
    let document = PolicyDocument::new(source()).expect("valid document");

    let first = serde_json::to_vec(&document).expect("serialize policy");
    let second = serde_json::to_vec(&document).expect("serialize policy");

    assert_eq!(first, second);
}

#[test]
fn policy_serialization_canonicalizes_semantically_unordered_entries() {
    let mut first = source();
    first.roles.get_mut("viewer").expect("viewer role").rules =
        vec![rule("z-last"), rule("a-first")];
    first.bindings.push(Binding {
        authority: "another.example".to_owned(),
        subjects: BTreeSet::from(["bob".to_owned()]),
        groups: BTreeSet::new(),
        roles: BTreeSet::from(["viewer".to_owned()]),
    });
    let mut second = first.clone();
    second
        .roles
        .get_mut("viewer")
        .expect("viewer role")
        .rules
        .reverse();
    second.bindings.reverse();

    let first = PolicyDocument::new(first).expect("valid first document");
    let second = PolicyDocument::new(second).expect("valid second document");

    assert_eq!(
        serde_json::to_vec(&first).expect("serialize first policy"),
        serde_json::to_vec(&second).expect("serialize second policy")
    );
}

#[test]
fn matching_deny_overrides_allow() {
    let mut source = source();
    let role = source.roles.get_mut("viewer").expect("viewer role");
    role.rules.push(Rule {
        id: RuleId::new("deny-host").expect("valid rule ID"),
        effect: RuleEffect::Deny,
        operations: BTreeSet::from([Operation::Observe]),
        resources: ResourceConstraints {
            host_attached: Some(true),
            ..ResourceConstraints::default()
        },
        reason: Some("host devices are hidden".to_owned()),
        cache_hint: Some(Duration::from_secs(900)),
    });
    let document = PolicyDocument::new(source).expect("valid document");
    let principal = RemotePrincipal::new("oidc.example", "alice", []).expect("valid principal");
    let resources = [Resource {
        device_id: DeviceId::new("keyboard"),
        provider_instance: Some("local".to_owned()),
        host_attached: true,
        collections: BTreeSet::new(),
    }];

    let decision = document.evaluate(&principal, Operation::Observe, &resources);

    assert_eq!(decision.outcome, DecisionOutcome::Deny);
    assert_eq!(
        decision.audit_rule.as_ref().map(RuleId::as_str),
        Some("deny-host")
    );
    assert_eq!(decision.cache_hint, Some(MAX_CACHE_HINT));
}

#[test]
fn constrained_rule_requires_resources_and_matches_every_resource() {
    let mut source = source();
    source.roles.get_mut("viewer").expect("viewer role").rules[0]
        .resources
        .provider_instances
        .insert("permitted".to_owned());
    let document = PolicyDocument::new(source).expect("valid document");
    let principal = RemotePrincipal::new("oidc.example", "alice", []).expect("valid principal");
    let permitted = Resource {
        device_id: DeviceId::new("one"),
        provider_instance: Some("permitted".to_owned()),
        host_attached: false,
        collections: BTreeSet::new(),
    };
    let denied = Resource {
        device_id: DeviceId::new("two"),
        provider_instance: Some("other".to_owned()),
        host_attached: false,
        collections: BTreeSet::new(),
    };

    assert!(
        !document
            .evaluate(&principal, Operation::Observe, &[])
            .is_allowed()
    );
    assert!(
        document
            .evaluate(&principal, Operation::Observe, slice::from_ref(&permitted))
            .is_allowed()
    );
    assert!(
        !document
            .evaluate(&principal, Operation::Observe, &[permitted, denied])
            .is_allowed()
    );
}

#[test]
fn exact_group_binding_inherits_roles() {
    let mut source = source();
    source.bindings[0].subjects.clear();
    source.bindings[0].groups.insert("operators".to_owned());
    source.roles.insert(
        "operator".to_owned(),
        Role {
            parents: BTreeSet::from(["viewer".to_owned()]),
            rules: Vec::new(),
        },
    );
    source.bindings[0].roles = BTreeSet::from(["operator".to_owned()]);
    let document = PolicyDocument::new(source).expect("valid document");
    let principal = RemotePrincipal::new("oidc.example", "bob", ["operators".to_owned()])
        .expect("valid principal");

    assert!(
        document
            .evaluate(&principal, Operation::Observe, &[])
            .is_allowed()
    );
}

#[test]
fn unmatched_principal_is_denied_by_default() {
    let document = PolicyDocument::new(source()).expect("valid document");
    let principal = RemotePrincipal::new("other.example", "alice", []).expect("valid principal");

    let decision = document.evaluate(&principal, Operation::Observe, &[]);

    assert_eq!(decision.outcome, DecisionOutcome::Deny);
    assert_eq!(decision.audit_rule, None);
    assert_eq!(decision.reason.as_deref(), Some("access denied by default"));
}

#[test]
fn rejects_oversized_reason() {
    let mut source = source();
    source.roles.get_mut("viewer").expect("viewer role").rules[0].reason =
        Some("x".repeat(MAX_REASON_BYTES + 1));

    let error = PolicyDocument::new(source).expect_err("oversized reason must fail");

    assert!(error.to_string().contains("reason exceeds"));
}

#[test]
fn rejects_empty_deserialized_rule_id() {
    let mut value = serde_json::to_value(source()).expect("serialize source");
    value["roles"]["viewer"]["rules"][0]["id"] = serde_json::json!("");

    let error =
        serde_json::from_value::<PolicyDocument>(value).expect_err("empty rule ID must fail");

    assert!(error.to_string().contains("rule ID must not be empty"));
}

#[test]
fn static_policy_is_object_safe_and_evaluates_asynchronously() {
    let policy: Box<dyn AccessPolicy> = Box::new(StaticAccessPolicy::new(
        PolicyDocument::new(source()).expect("valid document"),
    ));
    let request = AuthorizationRequest {
        principal: RemotePrincipal::new("oidc.example", "alice", []).expect("valid principal"),
        operation: Operation::Observe,
        resources: Vec::new(),
    };

    let decision = ready(policy.authorize(&request)).expect("policy available");

    assert!(decision.is_allowed());
    assert_eq!(policy.revision(), PolicyRevision(7));
}

#[test]
fn in_memory_policy_store_replaces_with_compare_and_swap() {
    let store =
        InMemoryPolicyStore::new(PolicyDocument::new(source()).expect("valid initial document"));
    let mut replacement = source();
    replacement.revision = PolicyRevision(8);

    let stored = ready(store.replace(
        PolicyRevision(7),
        PolicyDocument::new(replacement).expect("valid replacement"),
    ))
    .expect("matching revision");

    assert_eq!(stored.revision(), PolicyRevision(8));
    assert!(matches!(
        ready(store.replace(
            PolicyRevision(7),
            PolicyDocument::new(source()).expect("valid stale replacement")
        )),
        Err(PolicyError::RevisionConflict {
            expected: PolicyRevision(7),
            actual: PolicyRevision(8)
        })
    ));
}

#[test]
fn runtime_policy_preserves_frontend_recovery_allowlist_only() {
    let store: Arc<dyn PolicyStore> = Arc::new(InMemoryPolicyStore::new(
        PolicyDocument::new(source()).expect("valid initial document"),
    ));
    let policy = ready(RuntimeAccessPolicy::load(
        store,
        PolicyDocument::new(recovery_source()).expect("valid recovery document"),
    ))
    .expect("load runtime policy");
    let principal =
        RemotePrincipal::new("local", "recovery", []).expect("valid recovery principal");

    let manage_policy = ready(policy.authorize(&AuthorizationRequest {
        principal: principal.clone(),
        operation: Operation::ManagePolicy,
        resources: Vec::new(),
    }))
    .expect("evaluate policy management");
    let manage_auth = ready(policy.authorize(&AuthorizationRequest {
        principal: principal.clone(),
        operation: Operation::ManageAuthentication,
        resources: Vec::new(),
    }))
    .expect("evaluate authentication management");
    let administer_frontend = ready(policy.authorize(&AuthorizationRequest {
        principal: principal.clone(),
        operation: Operation::AdministerFrontend,
        resources: Vec::new(),
    }))
    .expect("evaluate frontend administration");
    let observe = ready(policy.authorize(&AuthorizationRequest {
        principal,
        operation: Operation::Observe,
        resources: Vec::new(),
    }))
    .expect("evaluate observation");

    assert!(manage_policy.is_allowed());
    assert!(manage_auth.is_allowed());
    assert!(administer_frontend.is_allowed());
    assert_eq!(manage_policy.revision, PolicyRevision(7));
    assert_eq!(manage_auth.revision, PolicyRevision(7));
    assert_eq!(administer_frontend.revision, PolicyRevision(7));
    assert_eq!(
        manage_policy.audit_rule.as_ref().map(RuleId::as_str),
        Some("recover-policy")
    );
    assert_eq!(
        manage_auth.audit_rule.as_ref().map(RuleId::as_str),
        Some("recover-policy")
    );
    assert_eq!(
        administer_frontend.audit_rule.as_ref().map(RuleId::as_str),
        Some("recover-policy")
    );
    assert!(!observe.is_allowed());
}

#[test]
fn runtime_policy_persists_before_activating_replacement() {
    let store: Arc<dyn PolicyStore> = Arc::new(InMemoryPolicyStore::new(
        PolicyDocument::new(source()).expect("valid initial document"),
    ));
    let policy = ready(RuntimeAccessPolicy::load(
        Arc::clone(&store),
        PolicyDocument::new(recovery_source()).expect("valid recovery document"),
    ))
    .expect("load runtime policy");
    let mut replacement = source();
    replacement.revision = PolicyRevision(8);
    replacement.bindings[0].subjects = BTreeSet::from(["bob".to_owned()]);

    let activated = ready(policy.replace(
        PolicyRevision(7),
        PolicyDocument::new(replacement).expect("valid replacement"),
    ))
    .expect("replace runtime policy");

    assert_eq!(activated.revision(), PolicyRevision(8));
    assert_eq!(policy.revision(), PolicyRevision(8));
    assert_eq!(
        ready(store.load()).expect("load persisted policy"),
        activated
    );
}

#[test]
fn runtime_policy_does_not_activate_failed_replacement() {
    let store: Arc<dyn PolicyStore> = Arc::new(StaticPolicyStore::new(
        PolicyDocument::new(source()).expect("valid initial document"),
    ));
    let policy = ready(RuntimeAccessPolicy::load(
        store,
        PolicyDocument::new(recovery_source()).expect("valid recovery document"),
    ))
    .expect("load runtime policy");
    let mut replacement = source();
    replacement.revision = PolicyRevision(8);

    let result = ready(policy.replace(
        PolicyRevision(7),
        PolicyDocument::new(replacement).expect("valid replacement"),
    ));

    assert!(matches!(result, Err(PolicyError::Unavailable(_))));
    assert_eq!(policy.revision(), PolicyRevision(7));
}

#[test]
fn in_memory_audit_sink_records_snapshot() {
    let sink = InMemoryAuditSink::new();
    let record = AuditRecord {
        principal: RemotePrincipal::new("oidc.example", "alice", []).expect("valid principal"),
        frontend_actor: None,
        operation: Operation::ManagePolicy,
        outcome: DecisionOutcome::Deny,
        revision: PolicyRevision(7),
        audit_rule: None,
        reason: Some("access denied by default".to_owned()),
    };

    ready(sink.record(record.clone())).expect("record audit event");

    assert_eq!(sink.records().expect("audit snapshot"), vec![record]);
}
