// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::collections::BTreeSet;

use super::*;
use luminate_core::collection::{CollectionId, CollectionMember};
use luminate_core::device::DeviceId;
use luminate_core::effect::Effect;
use luminate_core::frame::{FrameEnvelope, FramePayload};
use luminate_core::policy::{
    AccessPolicy, AuthorizationDecision, AuthorizationRequest, DecisionOutcome,
    ManagedAccessPolicy, Operation as ScopeOperation, PolicyDocument, PolicyError, PolicyFuture,
    PolicyRevision, ResourceConstraints, ScopeGrant, SessionScope,
};
use luminate_core::target::TargetId;
use luminate_protocol::{
    Selector, SetAppearanceSlotsRequest, SetBrightnessRequest, SetEffectRequest,
};

fn test_principal() -> Principal {
    Principal::Unix {
        uid: 1000,
        gid: 1000,
        pid: Some(4242),
    }
}

fn test_windows_principal() -> Principal {
    Principal::Windows {
        sid: "S-1-5-21-1-2-3-1000".to_owned(),
        pid: Some(4242),
    }
}

#[derive(Debug)]
struct SubjectPolicy;

impl AccessPolicy for SubjectPolicy {
    fn authorize<'a>(
        &'a self,
        request: &'a AuthorizationRequest,
    ) -> PolicyFuture<'a, Result<AuthorizationDecision, PolicyError>> {
        Box::pin(async move {
            let allowed = request.principal.subject().starts_with("allowed");
            Ok(AuthorizationDecision {
                outcome: if allowed {
                    DecisionOutcome::Allow
                } else {
                    DecisionOutcome::Deny
                },
                reason: (!allowed).then(|| "front-end ceiling denied access".to_owned()),
                audit_rule: None,
                cache_hint: None,
                revision: PolicyRevision(1),
            })
        })
    }

    fn revision(&self) -> PolicyRevision {
        PolicyRevision(1)
    }
}

impl ManagedAccessPolicy for SubjectPolicy {
    fn document(&self) -> PolicyFuture<'_, Result<Arc<PolicyDocument>, PolicyError>> {
        Box::pin(async { Err(PolicyError::Unavailable("not used by this test".to_owned())) })
    }

    fn replace(
        &self,
        _expected: PolicyRevision,
        _replacement: PolicyDocument,
    ) -> PolicyFuture<'_, Result<Arc<PolicyDocument>, PolicyError>> {
        Box::pin(async { Err(PolicyError::Unavailable("not used by this test".to_owned())) })
    }
}

#[tokio::test]
async fn delegated_policy_intersects_the_subject_and_frontend_actor() {
    for (subject_allowed, frontend_allowed, expected) in [
        (true, true, Decision::Allow),
        (
            true,
            false,
            Decision::Deny {
                reason: Some("front-end ceiling denied access".to_owned()),
            },
        ),
        (
            false,
            true,
            Decision::Deny {
                reason: Some("front-end ceiling denied access".to_owned()),
            },
        ),
        (
            false,
            false,
            Decision::Deny {
                reason: Some("front-end ceiling denied access".to_owned()),
            },
        ),
    ] {
        let subject = if subject_allowed {
            "allowed-user"
        } else {
            "denied-user"
        };
        let frontend = if frontend_allowed {
            "allowed-frontend"
        } else {
            "denied-frontend"
        };
        let subject = SessionPrincipal::new("http", subject, Vec::<String>::new())
            .expect("delegated principal");
        let frontend = SessionPrincipal::new("http", frontend, Vec::<String>::new())
            .expect("front-end principal");
        let policy = AuthenticatedSubjectPolicy::new(Arc::new(SubjectPolicy), subject)
            .with_frontend_actor(frontend, Some("front-end-token".to_owned()));

        assert_eq!(
            policy
                .authorize(&test_principal(), Operation::Observe, &[])
                .await,
            expected
        );
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "An exhaustive table of every Request variant's expected Operation"
)]
fn every_request_variant_is_classified() {
    let target = TargetId::device("dev0");
    let device = DeviceId::new("dev0");
    let cases = [
        (Request::ServerInfo, Operation::Observe),
        (Request::ListDevices, Operation::Observe),
        (
            Request::ListWithdrawnDevices,
            Operation::DaemonAdministration,
        ),
        (
            Request::GetDevice { id: device.clone() },
            Operation::Observe,
        ),
        (
            Request::GetState {
                device: device.clone(),
            },
            Operation::Observe,
        ),
        (
            Request::RefreshState {
                device: device.clone(),
            },
            Operation::Refresh,
        ),
        (
            Request::PurgeWithdrawnDevice {
                device: device.clone(),
            },
            Operation::DaemonAdministration,
        ),
        (
            Request::SetEffect(SetEffectRequest {
                selector: Selector::Target(target.clone()),
                effect: Effect::Off,
                on_unsupported: None,
            }),
            Operation::Control,
        ),
        (
            Request::SetBrightness(SetBrightnessRequest {
                target: Selector::Target(target.clone()),
                value: 50,
                on_unsupported: None,
            }),
            Operation::Control,
        ),
        (
            Request::SetAppearanceSlots(SetAppearanceSlotsRequest {
                target: TargetId::surface("dev0", "surface"),
                values: Vec::new(),
            }),
            Operation::Control,
        ),
        (
            Request::ClearTarget {
                target: Selector::Target(target.clone()),
            },
            Operation::Control,
        ),
        (
            Request::SaveCurrent {
                target: Selector::Target(target.clone()),
            },
            Operation::HardwareAdministration,
        ),
        (
            Request::CreateCollection {
                name: "Living room".to_owned(),
                description: None,
                kind: None,
                members: Vec::new(),
            },
            Operation::CreateCollection,
        ),
        (
            Request::DestroyCollection {
                id: CollectionId::new("living-room"),
            },
            Operation::DestroyCollection,
        ),
        (
            Request::AddCollectionMember {
                id: CollectionId::new("living-room"),
                member: CollectionMember::Target(target.clone()),
            },
            Operation::ModifyCollection,
        ),
        (
            Request::RemoveCollectionMember {
                id: CollectionId::new("living-room"),
                member: CollectionMember::Target(target.clone()),
            },
            Operation::ModifyCollection,
        ),
        (Request::ListCollections, Operation::Observe),
        (
            Request::GetCollection {
                id: CollectionId::new("living-room"),
            },
            Operation::Observe,
        ),
        (
            Request::BeginFrameStream {
                target: target.clone(),
            },
            Operation::Control,
        ),
        (
            Request::UploadFrame {
                target: target.clone(),
                envelope: FrameEnvelope {
                    generation: 1,
                    sequence: 0,
                    payload: FramePayload::Full(Vec::new()),
                    commit: false,
                },
            },
            Operation::Control,
        ),
        (
            Request::EndFrameStream {
                target: target.clone(),
                generation: 1,
            },
            Operation::Control,
        ),
        (
            Request::BeginShmFrameStream {
                target: target.clone(),
            },
            Operation::Control,
        ),
        (
            Request::EndShmFrameStream {
                target,
                generation: 1,
            },
            Operation::Control,
        ),
        (Request::GetManagement, Operation::ManagePlugins),
        (
            Request::ListPluginSetupWorkflows {
                plugin: "example".to_owned(),
            },
            Operation::ManagePlugins,
        ),
        (
            Request::StartPluginSetup {
                plugin: "example".to_owned(),
                workflow: "pair".to_owned(),
            },
            Operation::ManagePlugins,
        ),
        (
            Request::RespondPluginSetup {
                session: luminate_protocol::PluginSetupSessionId::parse(
                    "0123456789abcdef0123456789abcdef",
                )
                .expect("valid setup session ID"),
                generation: 1,
                response: luminate_protocol::PluginSetupInteractionResponse::Confirmed,
            },
            Operation::ManagePlugins,
        ),
        (
            Request::GetPluginSetup {
                session: luminate_protocol::PluginSetupSessionId::parse(
                    "0123456789abcdef0123456789abcdef",
                )
                .expect("valid setup session ID"),
            },
            Operation::ManagePlugins,
        ),
        (
            Request::CancelPluginSetup {
                session: luminate_protocol::PluginSetupSessionId::parse(
                    "0123456789abcdef0123456789abcdef",
                )
                .expect("valid setup session ID"),
            },
            Operation::ManagePlugins,
        ),
        (
            Request::PatchManagement {
                patch: luminate_protocol::ManagementPatch {
                    expected_revision: 0,
                    mutations: Vec::new(),
                },
            },
            Operation::ManagePlugins,
        ),
    ];

    for (request, expected) in cases {
        assert_eq!(operation_for(&request), Some(expected));
    }
}

#[tokio::test]
async fn socket_access_policy_always_allows() {
    let policy = SocketAccessPolicy;
    assert_eq!(policy.name(), "socket-access");
    assert_eq!(
        policy
            .authorize(&test_principal(), Operation::DaemonAdministration, &[])
            .await,
        Decision::Allow
    );
    assert_eq!(
        policy
            .authorize(
                &test_principal(),
                Operation::HardwareAdministration,
                &[Resource {
                    device_id: DeviceId::new("dev0"),
                    provider_instance: Some("demo".to_owned()),
                    host_attached: true,
                    collections: BTreeSet::new(),
                }]
            )
            .await,
        Decision::Allow
    );
}

#[cfg(unix)]
#[test]
fn is_same_user_as_daemon_allows_a_matching_uid() {
    use luminate_platform::identity::daemon_own_uid;

    let daemon_uid = daemon_own_uid();
    let principal = Principal::Unix {
        uid: daemon_uid,
        gid: 0,
        pid: None,
    };
    assert!(principal.is_same_user_as_daemon());
}

#[cfg(unix)]
#[test]
fn is_same_user_as_daemon_denies_a_mismatched_uid() {
    use luminate_platform::identity::daemon_own_uid;

    let daemon_uid = daemon_own_uid();
    let principal = Principal::Unix {
        uid: daemon_uid.wrapping_add(1),
        gid: 0,
        pid: None,
    };
    assert!(!principal.is_same_user_as_daemon());
}

#[cfg(unix)]
#[test]
fn intrinsic_recovery_is_limited_to_root_and_the_daemon_uid() {
    use luminate_platform::identity::daemon_own_uid;

    assert!(
        Principal::Unix {
            uid: 0,
            gid: 0,
            pid: None,
        }
        .is_intrinsic_recovery()
    );
    assert!(
        Principal::Unix {
            uid: daemon_own_uid(),
            gid: 0,
            pid: None,
        }
        .is_intrinsic_recovery()
    );
    assert!(
        !Principal::Unix {
            uid: daemon_own_uid().wrapping_add(1),
            gid: 0,
            pid: None,
        }
        .is_intrinsic_recovery()
    );
    assert!(!test_windows_principal().is_intrinsic_recovery());
}

#[test]
fn is_same_user_as_daemon_denies_a_windows_principal_on_a_non_windows_daemon() {
    assert!(!test_windows_principal().is_same_user_as_daemon());
}

#[test]
fn rate_limit_key_distinguishes_uid_and_sid_principals() {
    assert_eq!(test_principal().rate_limit_key(), RateLimitKey::Uid(1000));
    assert_eq!(
        test_windows_principal().rate_limit_key(),
        RateLimitKey::Sid("S-1-5-21-1-2-3-1000".to_owned())
    );
}

#[test]
fn deny_decision_carries_an_optional_reason() {
    let decision = Decision::Deny {
        reason: Some("no HardwareAdministration outside business hours".to_owned()),
    };
    assert_eq!(
        decision,
        Decision::Deny {
            reason: Some("no HardwareAdministration outside business hours".to_owned())
        }
    );
    assert_ne!(decision, Decision::Allow);
}

#[tokio::test]
async fn session_scope_reduces_even_an_allowing_policy() {
    let scope = SessionScope::new(vec![ScopeGrant {
        operations: [ScopeOperation::Observe].into_iter().collect(),
        resources: ResourceConstraints::default(),
    }])
    .expect("valid scope");
    let policy: Arc<dyn AuthorizationPolicy> = Arc::new(SocketAccessPolicy);
    let scoped = ScopedPolicy::new(policy, scope);

    assert_eq!(
        scoped
            .authorize(&test_principal(), Operation::Observe, &[])
            .await,
        Decision::Allow
    );
    assert!(matches!(
        scoped
            .authorize(&test_principal(), Operation::Control, &[])
            .await,
        Decision::Deny { .. }
    ));
}
