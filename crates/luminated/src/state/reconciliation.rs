// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Per-device reconciliation status tracking: reachability, the
//! reconcile/verify state machine, and the failure/drift diagnostics
//! surfaced through `DeviceStateStatus`.

use std::time::{SystemTime, UNIX_EPOCH};

use luminate_core::device::DeviceId;
use luminate_core::state::{AdoptedFacet, AdoptionStatus, Reachability, ReconciliationStatus};

use super::DaemonState;
use super::observation::set_adoption_status;

impl DaemonState {
    /// Begin reconciliation of the given device.
    pub fn begin_reconciliation(&mut self, device: &DeviceId) {
        let status = self.device_status_mut(device);
        status.reconciliation = ReconciliationStatus::Reconciling;
        status.latest_attempt_ms = Some(now_ms());
        status.latest_error = None;
    }

    /// Set a device reconciliation state to failed.
    pub fn fail_reconciliation(&mut self, device: &DeviceId, diagnostic: String) {
        let mut keys = self
            .observations
            .keys()
            .filter(|(target, _)| target.device_id() == device)
            .cloned()
            .collect::<Vec<_>>();
        keys.sort_by_key(|key| format!("{key:?}"));
        for key in keys {
            if let Some(observation) = self.observations.get_mut(&key) {
                observation.stale = true;
            }
        }

        let status = self.device_status_mut(device);
        status.reachability = Reachability::Unavailable;
        status.reconciliation = ReconciliationStatus::Failed;
        status.latest_attempt_ms = Some(now_ms());
        status.latest_error = Some(diagnostic);
    }

    /// Set a device adoption persistence state to failed.
    pub fn fail_adoption_persistence(
        &mut self,
        device: &DeviceId,
        candidate: &[AdoptedFacet],
        diagnostic: String,
    ) {
        let status = self.device_status_mut(device);
        status.reconciliation = ReconciliationStatus::Failed;
        status.latest_error = Some(diagnostic);
        for facet in candidate {
            set_adoption_status(
                status,
                facet.target.clone(),
                facet.value.kind(),
                AdoptionStatus::PersistenceFailed,
            );
        }
    }

    /// Mark device reconciliation as complete.
    pub fn complete_reconciliation(&mut self, device: &DeviceId) {
        let status = self.device_status_mut(device);
        status.reachability = Reachability::Reachable;
        status.reconciliation = ReconciliationStatus::Complete;
        status.latest_attempt_ms = Some(now_ms());
    }

    /// Mark device reconciliation as drifted.
    pub fn mark_reconciliation_drifted(&mut self, device: &DeviceId, diagnostic: String) {
        let status = self.device_status_mut(device);
        status.reconciliation = ReconciliationStatus::Drifted;
        status.latest_error = Some(diagnostic);
    }

    /// Mark device reconciliation as partially failed.
    pub(crate) fn partial_reconciliation_failure(&mut self, device: &DeviceId, diagnostic: String) {
        let status = self.device_status_mut(device);
        status.reachability = Reachability::Reachable;
        status.reconciliation = ReconciliationStatus::Failed;
        status.latest_attempt_ms = Some(now_ms());
        status.latest_error = Some(diagnostic);
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
#[path = "reconciliation_tests.rs"]
mod tests;
