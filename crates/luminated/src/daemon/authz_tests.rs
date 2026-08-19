// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::future::Future;
use std::pin::Pin;

use luminate_core::capability;
use luminate_plugin_api::DeviceDescriptor;
use tokio::sync::Notify;

use super::super::tests_support::{
    empty_plugin_manager, remove_request_test_dir, request_test_context, test_principal,
};
use super::super::*;
use super::*;

#[tokio::test]
async fn resolve_resources_defaults_unknown_devices() {
    let (state, manager, _mutations, runtime_dir) = request_test_context("resolve-resources");
    let device = device::DeviceId::new("request-device");
    let unknown_device = device::DeviceId::new("missing-device");

    let resources = {
        let state = state.lock().await;
        resolve_resources(&state, &manager, &[device, unknown_device])
    };

    assert_eq!(resources.len(), 2);
    assert_eq!(
        resources[0].device_id,
        device::DeviceId::new("request-device")
    );
    assert!(!resources[0].host_attached);
    assert!(resources[0].collections.is_empty());
    assert_eq!(
        resources[1].device_id,
        device::DeviceId::new("missing-device")
    );
    assert!(!resources[1].host_attached);
    assert!(resources[1].collections.is_empty());

    remove_request_test_dir(&runtime_dir);
}

#[tokio::test]
async fn resolve_resources_reports_every_enclosing_collection() {
    use luminate_core::collection::CollectionMember;
    use luminate_core::target::TargetId;

    let (state, manager, _mutations, runtime_dir) =
        request_test_context("resolve-resources-collections");
    let device = device::DeviceId::new("request-device");

    let (inner, outer) = {
        let mut state = state.lock().await;
        let inner = state
            .create_collection(
                "Nook".to_owned(),
                None,
                OwnerIdentity::Uid(1000),
                None,
                vec![CollectionMember::Target(TargetId::Device(device.clone()))],
            )
            .expect("create inner collection");
        let outer = state
            .create_collection(
                "Living room".to_owned(),
                None,
                OwnerIdentity::Uid(1000),
                None,
                vec![CollectionMember::Collection(inner.clone())],
            )
            .expect("create outer collection");
        (inner, outer)
    };

    let resources = {
        let state = state.lock().await;
        resolve_resources(&state, &manager, &[device])
    };

    assert_eq!(resources.len(), 1);
    let collections = resources[0].collections.clone();
    let expected = [inner, outer].into_iter().collect();
    assert_eq!(
        collections, expected,
        "a device reports every collection reaching it, direct or nested"
    );

    remove_request_test_dir(&runtime_dir);
}

fn bare_descriptor(id: &str) -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: id.to_owned(),
        name: id.to_owned(),
        vendor: None,
        model: None,
        surfaces: Vec::new(),
        groups: Vec::new(),
        capabilities: capability::CapabilitySet::default(),
        category: None,
        physical_tags: Vec::new(),
        host_attached: false,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// Allows only the named device; denies every other resource set,
/// including an empty one.
struct AllowOnlyPolicy {
    allowed: device::DeviceId,
}

struct PausingPolicy {
    entered: Arc<Notify>,
    resume: Arc<Notify>,
}

impl AuthorizationPolicy for PausingPolicy {
    fn authorize<'a>(
        &'a self,
        _principal: &'a Principal,
        _operation: Operation,
        _resources: &'a [Resource],
    ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        Box::pin(async move {
            self.entered.notify_one();
            self.resume.notified().await;
            Decision::Allow
        })
    }

    fn name(&self) -> &'static str {
        "pausing-test-policy"
    }
}

impl AuthorizationPolicy for AllowOnlyPolicy {
    fn authorize<'a>(
        &'a self,
        _principal: &'a Principal,
        _operation: Operation,
        resources: &'a [Resource],
    ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        let decision = if resources
            .iter()
            .all(|resource| resource.device_id == self.allowed)
            && !resources.is_empty()
        {
            Decision::Allow
        } else {
            Decision::Deny { reason: None }
        };
        Box::pin(async move { decision })
    }

    fn name(&self) -> &'static str {
        "allow-only-test-policy"
    }
}

#[tokio::test]
async fn authorize_partial_splits_leaves_by_their_owning_devices_decision() {
    let allowed_device = device::DeviceId::new("allowed-device");
    let denied_device = device::DeviceId::new("denied-device");
    let daemon_state = DaemonState::from_descriptors(&[
        bare_descriptor(allowed_device.as_str()),
        bare_descriptor(denied_device.as_str()),
    ])
    .expect("build two-device daemon state");
    let state = Arc::new(Mutex::new(daemon_state));
    let manager = empty_plugin_manager();
    let policy = AllowOnlyPolicy {
        allowed: allowed_device.clone(),
    };

    let leaves = vec![
        TargetId::Device(denied_device.clone()),
        TargetId::Device(allowed_device.clone()),
        // A second leaf on the already-allowed device: one `authorize`
        // call per distinct device, not per leaf.
        TargetId::surface(allowed_device.as_str(), "ring"),
    ];

    let (applied, denied, _generation) = authorize_partial(
        &state,
        &manager,
        &policy,
        &test_principal(),
        Operation::Control,
        &leaves,
    )
    .await;

    assert_eq!(
        applied,
        vec![
            TargetId::Device(allowed_device.clone()),
            TargetId::surface(allowed_device.as_str(), "ring"),
        ],
        "applied must preserve the leaves' relative order"
    );
    assert_eq!(denied, vec![TargetId::Device(denied_device)]);
}

#[tokio::test]
async fn authorize_partial_denies_every_leaf_when_the_policy_denies_all() {
    let device_a = device::DeviceId::new("device-a");
    let device_b = device::DeviceId::new("device-b");
    let daemon_state = DaemonState::from_descriptors(&[
        bare_descriptor(device_a.as_str()),
        bare_descriptor(device_b.as_str()),
    ])
    .expect("build two-device daemon state");
    let state = Arc::new(Mutex::new(daemon_state));
    let manager = empty_plugin_manager();
    let policy = AllowOnlyPolicy {
        allowed: device::DeviceId::new("nobody-owns-this-device"),
    };
    let leaves = vec![
        TargetId::Device(device_a.clone()),
        TargetId::Device(device_b.clone()),
    ];

    let (applied, denied, _generation) = authorize_partial(
        &state,
        &manager,
        &policy,
        &test_principal(),
        Operation::Control,
        &leaves,
    )
    .await;

    assert!(applied.is_empty());
    assert_eq!(denied, leaves);
}

#[tokio::test]
async fn filter_devices_returns_only_independently_observable_devices() {
    let allowed_device = device::DeviceId::new("allowed-device");
    let denied_device = device::DeviceId::new("denied-device");
    let daemon_state = DaemonState::from_descriptors(&[
        bare_descriptor(allowed_device.as_str()),
        bare_descriptor(denied_device.as_str()),
    ])
    .expect("build two-device daemon state");
    let state = Arc::new(Mutex::new(daemon_state));
    let manager = empty_plugin_manager();
    let policy = AllowOnlyPolicy {
        allowed: allowed_device.clone(),
    };
    let principal = test_principal();
    let authorizer =
        RequestAuthorizer::new(&state, &manager, &policy, &principal, Operation::Observe);

    let (visible, _generation) = authorizer
        .filter_devices(&[denied_device, allowed_device.clone()])
        .await
        .expect("stable topology");

    assert_eq!(visible, vec![allowed_device]);
}

#[tokio::test]
async fn authorization_rejects_topology_replaced_while_policy_is_deciding() {
    let device = device::DeviceId::new("request-device");
    let descriptor = bare_descriptor(device.as_str());
    let state = Arc::new(Mutex::new(
        DaemonState::from_descriptors(slice::from_ref(&descriptor)).expect("build daemon state"),
    ));
    let manager = Arc::new(empty_plugin_manager());
    let entered = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let policy = Arc::new(PausingPolicy {
        entered: Arc::clone(&entered),
        resume: Arc::clone(&resume),
    });
    let principal = Arc::new(test_principal());
    let task_state = Arc::clone(&state);
    let task_manager = Arc::clone(&manager);
    let task_policy = Arc::clone(&policy);
    let task_principal = Arc::clone(&principal);
    let task_device = device.clone();
    let authorization = tokio::spawn(async move {
        authorize(
            &task_state,
            &task_manager,
            task_policy.as_ref(),
            &task_principal,
            Operation::Observe,
            slice::from_ref(&task_device),
        )
        .await
    });

    entered.notified().await;
    let replacement = DaemonState::from_descriptors(slice::from_ref(&descriptor))
        .expect("build replacement state")
        .devices();
    state
        .lock()
        .await
        .replace_devices_preserving_withdrawn_state(replacement);
    resume.notify_one();

    let response = authorization
        .await
        .expect("join authorization")
        .expect_err("stale authorization must conflict");
    assert!(matches!(
        response.status,
        ResponseStatus::Error(OperationError {
            code: ErrorCode::Conflict,
            ..
        })
    ));
}
