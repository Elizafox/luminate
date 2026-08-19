// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Shared `#[cfg(test)]` fixtures used across more than one `daemon` submodule's
//! test suite. Fixtures scoped to a single concern's tests stay local to that
//! concern's own test module instead of living here.

#![cfg(test)]

use std::process;

use luminate_platform::test_support::TestDir;

use super::executor::MutationExecutor;
use super::*;
use crate::device_config::{PluginActivationMode, PluginManagementConfig};

pub(super) fn test_principal() -> Principal {
    Principal::Unix {
        uid: 1000,
        gid: 1000,
        pid: Some(process::id()),
    }
}

pub(super) fn test_windows_principal() -> Principal {
    Principal::Windows {
        sid: "S-1-5-21-1-2-3-1000".to_owned(),
        pid: Some(process::id()),
    }
}

pub(super) fn test_policy() -> Arc<dyn AuthorizationPolicy> {
    Arc::new(SocketAccessPolicy)
}

/// A rescan requester whose notifications go nowhere, for tests that must
/// supply one but don't exercise rescanning. Tests that do care hold their
/// own channel so they can observe what was requested.
pub(super) fn discarding_rescans() -> RescanRequester {
    RescanRequester::new(mpsc::unbounded_channel().0)
}

pub(super) fn empty_plugin_manager() -> Arc<PluginManager> {
    let config = DaemonConfig {
        plugin_dirs: Vec::new(),
        plugin_management: PluginManagementConfig {
            activation: PluginActivationMode::Explicit,
        },
        ..DaemonConfig::default()
    };
    Arc::new(
        PluginManager::load(&config, &managed_config::ManagedConfig::default(), false)
            .expect("load empty plugin manager"),
    )
}

pub(super) fn test_management_read_state() -> Arc<ManagementReadState> {
    let managed_path =
        env::temp_dir().join(format!("luminated-test-managed-{}.toml", process::id()));
    let global = DaemonConfig::default();
    Arc::new(ManagementReadState {
        effective: RwLock::new(global.clone()),
        global,
        managed_path,
        managed: Mutex::new(managed_config::ManagedConfig::default()),
    })
}

pub(super) fn request_test_descriptor() -> luminate_plugin_api::DeviceDescriptor {
    use luminate_core::capability::{
        BrightnessCapability, BufferingMode, CapabilityScope, CapabilitySet, ColourCapability,
        FrameUpdateMode, FrameUploadCapability,
    };

    luminate_plugin_api::DeviceDescriptor {
        claims: Vec::new(),
        id: "request-device".to_owned(),
        name: "Request Device".to_owned(),
        vendor: None,
        model: None,
        surfaces: Vec::new(),
        groups: Vec::new(),
        capabilities: CapabilitySet {
            colour: vec![ColourCapability::rgb8()],
            cct_emulation: CctEmulation::Auto,
            brightness: BrightnessCapability::Independent {
                bits: 8,
                maximum: 100,
                scope: CapabilityScope::Device,
            },
            persistence: PersistenceCapability::CurrentState {
                requirement: PersistenceRequirement::Required,
                explicit_commit: false,
                readback: false,
            },
            frame_upload: Some(FrameUploadCapability {
                scope: CapabilityScope::Device,
                update_mode: FrameUpdateMode::FullFrameOnly,
                max_rate_hz: None,
                atomic: false,
                buffering: BufferingMode::Immediate,
                shm: None,
            }),
            emission: true,
            ..CapabilitySet::default()
        },
        category: None,
        physical_tags: Vec::new(),
        host_attached: false,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

pub(super) fn request_test_context(
    name: &str,
) -> (
    Arc<Mutex<DaemonState>>,
    Arc<PluginManager>,
    MutationExecutor,
    TestDir,
) {
    let runtime_dir = TestDir::new(name);
    let state_path: Arc<Path> = Arc::from(runtime_dir.join("state.json").into_boxed_path());
    let state = Arc::new(Mutex::new(
        DaemonState::from_descriptors(&[request_test_descriptor()]).expect("build request state"),
    ));
    let manager = empty_plugin_manager();
    let (events, _) = EventPublisher::test_channel(&state, 16);
    let mutations = MutationExecutor {
        state: Arc::clone(&state),
        state_path,
        commit: Arc::new(sync::Mutex::new(())),
        device_sequencers: Arc::new(sync::Mutex::new(HashMap::new())),
        events,
        operator_events: false,
        transitions: Arc::new(transition::TransitionRegistry::default()),
    };
    (state, manager, mutations, runtime_dir)
}

pub(super) fn remove_request_test_dir(runtime_dir: &Path) {
    let _ = fs::remove_file(runtime_dir.join("state.json"));
    fs::remove_dir(runtime_dir).expect("remove request test directory");
}

pub(super) fn assert_error_response(response: &Response) {
    assert!(matches!(response.status, ResponseStatus::Error { .. }));
}
