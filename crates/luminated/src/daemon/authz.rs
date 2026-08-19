// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Per-request authorization: resource resolution and policy decisions.

use std::collections::HashMap;
use std::slice;

use futures_util::future::join_all;

use super::{
    Arc, AuthorizationPolicy, DaemonError, DaemonState, Decision, Mutex, Operation, PluginManager,
    Principal, Resource, Response, ResponseStatus, TargetId, device,
};

/// Authorizes `operation` against the resolved `devices`, after selector
/// expansion.
///
/// Authorization is all-or-nothing for collection selectors: every member
/// must be permitted. A denial for any member rejects the entire request
/// instead of silently filtering the target set.
pub(super) async fn authorize(
    state: &Arc<Mutex<DaemonState>>,
    plugin_manager: &PluginManager,
    policy: &dyn AuthorizationPolicy,
    principal: &Principal,
    operation: Operation,
    devices: &[device::DeviceId],
) -> Result<u64, Response> {
    let (resources, topology_generation) = {
        let state = state.lock().await;
        (
            resolve_resources(&state, plugin_manager, devices),
            state.topology_generation(),
        )
    };
    match policy.authorize(principal, operation, &resources).await {
        Decision::Allow
            if state
                .lock()
                .await
                .topology_generation_is(topology_generation) =>
        {
            Ok(topology_generation)
        }
        Decision::Allow => Err(authorization_conflict_response()),
        Decision::Deny { reason } => Err(permission_denied_response(reason)),
    }
}

pub(super) struct RequestAuthorizer<'a> {
    state: &'a Arc<Mutex<DaemonState>>,
    plugin_manager: &'a PluginManager,
    policy: &'a dyn AuthorizationPolicy,
    principal: &'a Principal,
    operation: Operation,
}

impl<'a> RequestAuthorizer<'a> {
    pub(super) fn new(
        state: &'a Arc<Mutex<DaemonState>>,
        plugin_manager: &'a PluginManager,
        policy: &'a dyn AuthorizationPolicy,
        principal: &'a Principal,
        operation: Operation,
    ) -> Self {
        Self {
            state,
            plugin_manager,
            policy,
            principal,
            operation,
        }
    }

    pub(super) async fn authorize(&self, devices: &[device::DeviceId]) -> Result<u64, Response> {
        authorize(
            self.state,
            self.plugin_manager,
            self.policy,
            self.principal,
            self.operation,
            devices,
        )
        .await
    }

    /// Returns the subset of `devices` independently allowed by the policy.
    ///
    /// This is for filtering observations, where hiding denied resources is
    /// the intended public behaviour. Mutations must use [`Self::authorize`]
    /// or [`Self::authorize_partial`] so a denial cannot be mistaken for a
    /// successful write.
    pub(super) async fn filter_devices(
        &self,
        devices: &[device::DeviceId],
    ) -> Result<(Vec<device::DeviceId>, u64), Response> {
        let (resources, topology_generation) = {
            let state = self.state.lock().await;
            (
                resolve_resources(&state, self.plugin_manager, devices),
                state.topology_generation(),
            )
        };
        let decisions = join_all(resources.iter().map(|resource| {
            self.policy
                .authorize(self.principal, self.operation, slice::from_ref(resource))
        }))
        .await;

        if !self
            .state
            .lock()
            .await
            .topology_generation_is(topology_generation)
        {
            return Err(authorization_conflict_response());
        }

        Ok((
            devices
                .iter()
                .zip(decisions)
                .filter_map(|(device, decision)| {
                    matches!(decision, Decision::Allow).then(|| device.clone())
                })
                .collect(),
            topology_generation,
        ))
    }

    pub(super) async fn topology_is_current(&self, generation: u64) -> bool {
        self.state.lock().await.topology_generation_is(generation)
    }

    /// See [`authorize_partial`].
    pub(super) async fn authorize_partial(
        &self,
        leaves: &[TargetId],
    ) -> (Vec<TargetId>, Vec<TargetId>, u64) {
        authorize_partial(
            self.state,
            self.plugin_manager,
            self.policy,
            self.principal,
            self.operation,
            leaves,
        )
        .await
    }
}

/// Builds the normalized [`Resource`] set an authorization decision judges,
/// from each device's daemon-known metadata.
fn resolve_resources(
    state: &DaemonState,
    plugin_manager: &PluginManager,
    devices: &[device::DeviceId],
) -> Vec<Resource> {
    devices
        .iter()
        .map(|device_id| {
            let device = state.device(device_id);
            Resource {
                device_id: device_id.clone(),
                provider_instance: plugin_manager.owner_name(device_id),
                host_attached: device.as_ref().is_some_and(|device| device.host_attached),
                collections: state
                    .collections_containing_device(device_id)
                    .into_iter()
                    .collect(),
            }
        })
        .collect()
}

/// Performs best-effort, per-leaf authorization for a collection-targeted
/// write.
///
/// Unlike [`authorize`], which evaluates a device (or collection) as a
/// single all-or-nothing resource set, this authorizes each leaf against
/// its owning device independently. This allows a caller authorized for
/// only part of a collection to mutate the members it is permitted to
/// access while explicitly reporting the denied members.
///
/// Authorization is performed once per distinct owning device (not per
/// leaf, since authorization is defined at device granularity; see
/// `resolve_resources`). These checks run concurrently rather than
/// sequentially, avoiding one IPC round trip per device when using a
/// `DynamicProviderPolicy`.
///
/// Returns `(applied, denied, topology_generation)`, where `applied` and
/// `denied` preserve the original relative ordering of `leaves`.
pub(super) async fn authorize_partial(
    state: &Arc<Mutex<DaemonState>>,
    plugin_manager: &PluginManager,
    policy: &dyn AuthorizationPolicy,
    principal: &Principal,
    operation: Operation,
    leaves: &[TargetId],
) -> (Vec<TargetId>, Vec<TargetId>, u64) {
    let mut unique_devices: Vec<device::DeviceId> = Vec::new();
    for leaf in leaves {
        if !unique_devices.contains(leaf.device_id()) {
            unique_devices.push(leaf.device_id().clone());
        }
    }

    let (resources, topology_generation) = {
        let state = state.lock().await;
        (
            resolve_resources(&state, plugin_manager, &unique_devices),
            state.topology_generation(),
        )
    };

    let decisions = join_all(
        resources
            .iter()
            .map(|resource| policy.authorize(principal, operation, slice::from_ref(resource))),
    )
    .await;

    let allowed: HashMap<device::DeviceId, bool> = unique_devices
        .into_iter()
        .zip(decisions)
        .map(|(device_id, decision)| (device_id, matches!(decision, Decision::Allow)))
        .collect();

    let mut applied = Vec::new();
    let mut denied = Vec::new();
    for leaf in leaves {
        if allowed.get(leaf.device_id()).copied().unwrap_or(false) {
            applied.push(leaf.clone());
        } else {
            denied.push(leaf.clone());
        }
    }

    (applied, denied, topology_generation)
}

fn permission_denied_response(reason: Option<String>) -> Response {
    Response {
        status: ResponseStatus::Error(luminate_protocol::OperationError {
            code: luminate_protocol::ErrorCode::PermissionDenied,
            message: reason.unwrap_or_else(|| "permission denied".to_owned()),
            retry_after_ms: None,
            applied_targets: Vec::new(),
        }),
    }
}

fn authorization_conflict_response() -> Response {
    DaemonError::AuthorizationConflict {
        reason: "device topology changed during authorization; retry the request".to_owned(),
    }
    .into()
}

#[cfg(test)]
#[path = "authz_tests.rs"]
mod tests;
