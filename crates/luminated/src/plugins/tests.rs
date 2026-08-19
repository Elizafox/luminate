// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Unit tests for plugin discovery, ownership, and routing.

use anyhow::Result;
use luminate_core::capability::{
    BufferingMode, CapabilityScope, CapabilitySet, FrameUpdateMode, FrameUploadCapability,
    ReadableFacet, ReadbackFidelity, ShmFrameCapability, ShmFrameShape, StateReadbackCapability,
};
use luminate_core::effect::Effect;
use luminate_core::frame::{FrameEnvelope, FramePayload};
use luminate_core::shm_frame::ShmPixelFormat;
use luminate_core::state::StateFacetKind;
use std::collections::HashSet;
use std::ffi::CString;
use std::path::Path;
use std::{fs, ptr, slice};

use luminate_core::device::DeviceId;
use luminate_platform::test_support::TestDir;
use luminate_plugin_api::{
    HardwareBus, PLUGIN_ABI_VERSION, PluginProbeHint, PluginUpdate, PluginUpdateOperation,
};

use super::apply::{
    apply_outcome_error, apply_outcome_to_result, fill_results, group_updates_by_owner,
};
use super::loading::{
    classify_load_outcome, device_has_exact_readback, validate_startup_candidate,
    warn_if_adopt_has_no_exact_readback,
};
use super::managed::{ManagedPluginAction, managed_plugin_action, run_managed_plugin_action};
use super::ownership::{arbitrate_ownership, owned_descriptors};
use super::*;
use crate::device_config::{
    AuthorizationConfig, DaemonConfig, ManagementConfig, PluginActivationMode,
    PluginManagementConfig,
};
use crate::managed_config::ManagedConfig;
use crate::plugin_host::{
    ApplyOutcome, RawProbeHint, ensure_abi_compatible, read_bus_slice, read_probe_hints,
};
use crate::state::DaemonState;
use tokio::sync::Mutex as AsyncMutex;

fn unique_temp_dir(name: &str) -> TestDir {
    TestDir::new(name)
}

#[test]
fn group_updates_by_owner_buckets_by_owning_plugin() {
    let keyboard_owner = LoadedPluginId::new();
    let aw_elc_owner = LoadedPluginId::new();
    let mut owner_by_device = HashMap::new();
    owner_by_device.insert("keyboard".to_owned(), keyboard_owner);
    owner_by_device.insert("aw-elc".to_owned(), aw_elc_owner);

    let updates = vec![
        (
            TargetId::Device(DeviceId::new("keyboard")),
            PluginUpdateOperation::Clear,
        ),
        (
            TargetId::Device(DeviceId::new("aw-elc")),
            PluginUpdateOperation::Clear,
        ),
        (
            TargetId::Device(DeviceId::new("keyboard")),
            PluginUpdateOperation::SetBrightness { value: 50 },
        ),
    ];

    let (by_plugin, results) = group_updates_by_owner(&owner_by_device, &updates);

    assert_eq!(by_plugin.get(&keyboard_owner), Some(&vec![0, 2]));
    assert_eq!(by_plugin.get(&aw_elc_owner), Some(&vec![1]));
    assert!(results.iter().all(Option::is_none));
}

#[test]
fn group_updates_by_owner_records_unowned_device_immediately() {
    let owner_by_device = HashMap::new();

    let updates = vec![(
        TargetId::Device(DeviceId::new("ghost-device")),
        PluginUpdateOperation::Clear,
    )];

    let (by_plugin, mut results) = group_updates_by_owner(&owner_by_device, &updates);

    assert!(by_plugin.is_empty());
    match results.remove(0) {
        Some(Err(DaemonError::DeviceUnowned(device))) => assert_eq!(device, "ghost-device"),
        other => panic!("expected DeviceUnowned, got {other:?}"),
    }
}

#[test]
fn group_updates_by_owner_preserves_original_indices_across_devices() {
    let owner = LoadedPluginId::new();
    let mut owner_by_device = HashMap::new();
    owner_by_device.insert("a".to_owned(), owner);

    let updates = vec![
        (
            TargetId::Device(DeviceId::new("unowned")),
            PluginUpdateOperation::Clear,
        ),
        (
            TargetId::Device(DeviceId::new("a")),
            PluginUpdateOperation::Clear,
        ),
    ];

    let (by_plugin, results) = group_updates_by_owner(&owner_by_device, &updates);

    assert_eq!(by_plugin.get(&owner), Some(&vec![1]));
    assert!(matches!(
        results[0],
        Some(Err(DaemonError::DeviceUnowned(_)))
    ));
    assert!(results[1].is_none());
}

#[test]
fn update_and_frame_routes_fail_cleanly_when_topology_has_no_owner() {
    let manager = empty_plugin_manager();
    let target = TargetId::device("ghost");
    let device = DeviceId::new("ghost");
    let effect = Effect::Off;

    assert!(matches!(
        manager.read_state(
            &device,
            luminate_plugin_api::PluginReadRequest { targets: Vec::new() }
        ),
        Err(DaemonError::DeviceUnowned(id)) if id == "ghost"
    ));
    for result in [
        manager.apply_effect(&target, &effect),
        manager.apply_brightness(&target, 20),
        manager.apply_clear(&target),
        manager.apply_save_current(&target),
        manager.apply_frame(
            &target,
            &FrameEnvelope {
                generation: 1,
                sequence: 0,
                payload: FramePayload::Full(Vec::new()),
                commit: false,
            },
        ),
    ] {
        assert!(matches!(result, Err(DaemonError::DeviceUnowned(id)) if id == "ghost"));
    }

    let mut descriptor = descriptor("ghost");
    descriptor.capabilities.frame_upload = Some(FrameUploadCapability {
        scope: CapabilityScope::Device,
        update_mode: FrameUpdateMode::FullFrameOnly,
        max_rate_hz: None,
        atomic: false,
        buffering: BufferingMode::Immediate,
        shm: None,
    });
    let state = Arc::new(AsyncMutex::new(
        DaemonState::from_descriptors(&[descriptor]).expect("build frame-route state"),
    ));
    let generation = state
        .blocking_lock()
        .begin_frame_stream(&target)
        .expect("begin frame stream");
    let frame = FrameEnvelope {
        generation,
        sequence: 0,
        payload: FramePayload::Full(Vec::new()),
        commit: false,
    };
    assert!(matches!(
        manager.apply_one_frame(&state, &target, &frame),
        Err(DaemonError::DeviceUnowned(id)) if id == "ghost"
    ));
}

#[test]
fn apply_batch_reports_an_owner_that_was_unloaded() {
    let manager = empty_plugin_manager();
    let owner = LoadedPluginId::new();
    manager
        .topology
        .lock()
        .expect("topology lock")
        .owner_by_device
        .insert("ghost".to_owned(), owner);
    let updates = vec![(TargetId::device("ghost"), PluginUpdateOperation::Clear)];

    let results = manager.apply_batch(&updates);
    assert!(matches!(
        results.as_slice(),
        [Err(DaemonError::Internal(message))]
            if message.contains("is no longer loaded")
    ));
}

#[test]
fn shared_memory_stream_facade_falls_back_without_a_loaded_owner() {
    let manager = empty_plugin_manager();
    let target = TargetId::device("ghost");
    let capability = ShmFrameCapability {
        shape: ShmFrameShape::Linear { pixel_count: 1 },
        pixel_formats: vec![ShmPixelFormat::Rgb8],
        max_rate_hz: None,
    };
    let frame = FrameEnvelope {
        generation: 1,
        sequence: 0,
        payload: FramePayload::Full(Vec::new()),
        commit: false,
    };
    assert!(!manager.begin_shm_stream(&target, &capability, 1));
    assert!(!manager.try_apply_shm_frame(&target, &frame));
    assert!(!manager.has_active_shm_stream(&target));
    manager.end_shm_stream(&target, 1);
    manager.end_all_shm_streams(slice::from_ref(&target));
    assert!(!manager.end_shm_client_stream(&target, 1));
    assert!(manager.end_all_shm_client_streams(&[target]).is_empty());
}

#[test]
fn client_shared_memory_registry_rejects_empty_formats_and_is_idempotent() {
    let registry = ShmClientSubscriberRegistry::default();
    let manager = Arc::new(empty_plugin_manager());
    let state = Arc::new(AsyncMutex::new(DaemonState::default()));
    let target = TargetId::device("ghost-client");
    let capability = ShmFrameCapability {
        shape: ShmFrameShape::Linear { pixel_count: 1 },
        pixel_formats: Vec::new(),
        max_rate_hz: None,
    };

    assert!(
        registry
            .begin(&target, &capability, 1, manager, state)
            .expect("empty formats should be a clean fallback")
            .is_none()
    );
    assert!(!registry.end(&target, 1));
    assert!(registry.force_end(&target).is_none());
}

#[test]
fn shared_object_extension_is_recognized() {
    let extension = luminate_platform::dynamic_library_extension();
    assert!(is_shared_object(Path::new(&format!(
        "/plugins/libfoo.{extension}"
    ))));
    assert!(!is_shared_object(Path::new("/plugins/readme.txt")));
    assert!(!is_shared_object(Path::new("/plugins/no-extension")));
}

#[test]
fn resolve_configured_plugin_explicit_path_is_used_as_is() {
    let plugin = PluginConfig {
        name: None,
        path: Some(PathBuf::from("/does/not/exist.so")),
        required: false,
        config: toml::Table::new(),
        ..PluginConfig::default()
    };
    let resolved = resolve_configured_plugin(&plugin, &[])
        .expect("explicit path should resolve without touching the filesystem");
    assert_eq!(resolved, Some(PathBuf::from("/does/not/exist.so")));
}

#[test]
fn resolve_configured_plugin_finds_by_lib_prefixed_name() {
    let dir = unique_temp_dir("lib-prefix");
    let [lib_prefixed, _] = luminate_platform::dynamic_library_candidates("foo");
    let so_path = dir.join(lib_prefixed);
    fs::write(&so_path, b"").expect("write stub plugin file");

    let plugin = PluginConfig {
        name: Some("foo".to_owned()),
        path: None,
        required: false,
        config: toml::Table::new(),
        ..PluginConfig::default()
    };
    let resolved = resolve_configured_plugin(&plugin, slice::from_ref(&dir.to_path_buf()))
        .expect("lookup should succeed");
    assert_eq!(resolved, Some(so_path));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn resolve_configured_plugin_finds_by_bare_name() {
    let dir = unique_temp_dir("bare-name");
    let [_, bare_named] = luminate_platform::dynamic_library_candidates("foo");
    let so_path = dir.join(bare_named);
    fs::write(&so_path, b"").expect("write stub plugin file");

    let plugin = PluginConfig {
        name: Some("foo".to_owned()),
        path: None,
        required: false,
        config: toml::Table::new(),
        ..PluginConfig::default()
    };
    let resolved = resolve_configured_plugin(&plugin, slice::from_ref(&dir.to_path_buf()))
        .expect("lookup should succeed");
    assert_eq!(resolved, Some(so_path));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn resolve_configured_plugin_finds_cargo_artifact_by_canonical_name() {
    let dir = unique_temp_dir("cargo-artifact-name");
    let [lib_prefixed, _] = luminate_platform::dynamic_library_candidates("luminate_plugin_govee");
    let so_path = dir.join(lib_prefixed);
    fs::write(&so_path, b"").expect("write stub plugin file");

    let plugin = PluginConfig {
        name: Some("luminate-plugin-govee".to_owned()),
        path: None,
        required: false,
        config: toml::Table::new(),
        ..PluginConfig::default()
    };
    let resolved = resolve_configured_plugin(&plugin, slice::from_ref(&dir.to_path_buf()))
        .expect("lookup should succeed");
    assert_eq!(resolved, Some(so_path));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn resolve_configured_plugin_optional_missing_is_none() {
    let plugin = PluginConfig {
        name: Some("missing".to_owned()),
        path: None,
        required: false,
        config: toml::Table::new(),
        ..PluginConfig::default()
    };
    let resolved =
        resolve_configured_plugin(&plugin, &[]).expect("missing optional plugin is not an error");
    assert_eq!(resolved, None);
}

#[test]
fn resolve_configured_plugin_required_missing_is_an_error() {
    let plugin = PluginConfig {
        name: Some("missing".to_owned()),
        path: None,
        required: true,
        config: toml::Table::new(),
        ..PluginConfig::default()
    };
    assert!(resolve_configured_plugin(&plugin, &[]).is_err());
}

#[test]
fn resolve_configured_plugin_empty_entry_is_an_error() {
    let plugin = PluginConfig {
        name: None,
        path: None,
        required: false,
        config: toml::Table::new(),
        ..PluginConfig::default()
    };
    assert!(resolve_configured_plugin(&plugin, &[]).is_err());
}

#[test]
fn discover_candidates_dedups_explicit_and_autoloaded_same_file() {
    let dir = unique_temp_dir("dedup");
    let [lib_prefixed, _] = luminate_platform::dynamic_library_candidates("foo");
    let so_path = dir.join(lib_prefixed);
    fs::write(&so_path, b"").expect("write stub plugin file");

    let config = DaemonConfig {
        socket_path: PathBuf::new(),
        event_socket_path: None,
        state_path: PathBuf::new(),
        plugin_dirs: vec![dir.to_path_buf()],
        management: ManagementConfig::default(),
        plugin_management: PluginManagementConfig {
            activation: PluginActivationMode::All,
        },
        default_unsupported_policy: luminate_protocol::UnsupportedPolicy::Skip,
        reconciliation_policy: None,
        device_reconciliation: HashMap::new(),
        cct_emulation: None,
        plugins: vec![PluginConfig {
            name: None,
            path: Some(so_path.clone()),
            required: true,
            config: toml::Table::from_iter([(
                "endpoint".to_owned(),
                toml::Value::String("192.0.2.10".to_owned()),
            )]),
            ..PluginConfig::default()
        }],
        prefer_shm: true,
        prefer_client_shm: true,
        authorization: AuthorizationConfig::default(),
    };

    let candidates = discover_candidates(&config).expect("discovery should succeed");
    assert_eq!(candidates.len(), 1);
    assert!(
        candidates[0].required,
        "explicit config entry should win the dedup and keep required=true"
    );
    assert_eq!(
        candidates[0].configuration["endpoint"],
        toml::Value::String("192.0.2.10".to_owned())
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn discover_candidates_skips_missing_plugin_dir() {
    let config = DaemonConfig {
        socket_path: PathBuf::new(),
        event_socket_path: None,
        state_path: PathBuf::new(),
        plugin_dirs: vec![PathBuf::from("/does/not/exist/luminate-plugins")],
        management: ManagementConfig::default(),
        plugin_management: PluginManagementConfig {
            activation: PluginActivationMode::All,
        },
        default_unsupported_policy: luminate_protocol::UnsupportedPolicy::Skip,
        reconciliation_policy: None,
        device_reconciliation: HashMap::new(),
        cct_emulation: None,
        plugins: Vec::new(),
        prefer_shm: true,
        prefer_client_shm: true,
        authorization: AuthorizationConfig::default(),
    };

    let candidates = discover_candidates(&config).expect("a missing autoload dir is not an error");
    assert!(candidates.is_empty());
}

fn base_config(plugin_dirs: Vec<PathBuf>) -> DaemonConfig {
    DaemonConfig {
        socket_path: PathBuf::new(),
        event_socket_path: None,
        state_path: PathBuf::new(),
        plugin_dirs,
        management: ManagementConfig::default(),
        plugin_management: PluginManagementConfig {
            activation: PluginActivationMode::All,
        },
        default_unsupported_policy: luminate_protocol::UnsupportedPolicy::Skip,
        reconciliation_policy: None,
        device_reconciliation: HashMap::new(),
        cct_emulation: None,
        plugins: Vec::new(),
        prefer_shm: true,
        prefer_client_shm: true,
        authorization: AuthorizationConfig::default(),
    }
}

#[test]
fn discover_candidates_autoloads_every_shared_object_in_sorted_order() {
    let dir = unique_temp_dir("autoload-multi");
    let extension = luminate_platform::dynamic_library_extension();
    fs::write(dir.join(format!("libb.{extension}")), b"").expect("write stub plugin file");
    fs::write(dir.join(format!("liba.{extension}")), b"").expect("write stub plugin file");
    fs::write(dir.join("readme.txt"), b"").expect("write non-plugin file");

    let config = base_config(vec![dir.to_path_buf()]);
    let candidates = discover_candidates(&config).expect("discovery should succeed");
    assert_eq!(candidates.len(), 2);
    assert!(candidates.iter().all(|candidate| !candidate.required));
    assert!(
        candidates[0].path.file_name() < candidates[1].path.file_name(),
        "autoloaded candidates should be sorted"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn discover_candidates_skips_an_unreadable_plugin_directory() {
    let dir = unique_temp_dir("unreadable-dir-parent");
    let not_a_dir = dir.join("not-a-directory");
    fs::write(&not_a_dir, b"").expect("write stub file standing in for a directory");

    let config = base_config(vec![not_a_dir]);
    let candidates =
        discover_candidates(&config).expect("an unreadable autoload dir is not a hard error");
    assert!(candidates.is_empty());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn abi_mismatch_is_rejected() {
    assert!(ensure_abi_compatible(PLUGIN_ABI_VERSION + 1).is_err());
}

#[test]
fn matching_abi_is_accepted() {
    assert!(ensure_abi_compatible(PLUGIN_ABI_VERSION).is_ok());
}

fn metadata(name: &str, priority: i32) -> LoadedPluginMetadata {
    LoadedPluginMetadata {
        name: name.to_owned(),
        version: "0.1.0".to_owned(),
        priority,
        recommended_reconciliation: None,
        probe_outcome: luminate_plugin_api::ProbeOutcome::Ready,
        buses: Vec::new(),
        vendors: Vec::new(),
        probe_hints: Vec::new(),
        path: PathBuf::new(),
    }
}

fn descriptor(id: &str) -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: id.to_owned(),
        name: id.to_owned(),
        vendor: None,
        model: None,
        surfaces: Vec::new(),
        groups: Vec::new(),
        capabilities: CapabilitySet::default(),
        category: None,
        physical_tags: Vec::new(),
        host_attached: false,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

#[test]
fn exact_readback_detection_ignores_best_effort_facets() {
    let mut device = descriptor("readable");
    device.capabilities.state_readback = StateReadbackCapability::Readable {
        facets: vec![ReadableFacet {
            facet: StateFacetKind::Brightness,
            fidelity: ReadbackFidelity::BestEffort,
        }],
        read_disturbs_output: false,
        notifies_external_changes: false,
    };
    assert!(!device_has_exact_readback(&device));

    let StateReadbackCapability::Readable { facets, .. } = &mut device.capabilities.state_readback
    else {
        unreachable!("test installed readable capability");
    };
    facets[0].fidelity = ReadbackFidelity::Exact;
    assert!(device_has_exact_readback(&device));
}

/// Two plugins both claim device id "shared". Without filtering, both
/// descriptors would reach `normalize_devices`, which hard-errors on the
/// duplicate ID. That means the priority logic meant to resolve this never
/// gets a chance to run. `device_descriptors()` must filter the loser out
/// before normalization ever sees it.
#[test]
fn device_descriptors_filters_out_ownership_conflict_losers() {
    let loaded_metadata = vec![
        metadata("low-priority-plugin", 1),
        metadata("high-priority-plugin", 10),
    ];
    let plugin0_descriptors = vec![descriptor("shared"), descriptor("low-only")];
    let plugin1_descriptors = vec![descriptor("shared"), descriptor("high-only")];
    let descriptor_sets = vec![plugin0_descriptors.clone(), plugin1_descriptors.clone()];
    let owner_by_device = arbitrate_ownership(&loaded_metadata, &descriptor_sets);

    let result = owned_descriptors(
        &owner_by_device,
        [
            (0, plugin0_descriptors.as_slice()),
            (1, plugin1_descriptors.as_slice()),
        ]
        .into_iter(),
    );

    let ids: HashSet<&str> = result
        .iter()
        .map(|descriptor| descriptor.id.as_str())
        .collect();
    assert_eq!(ids, HashSet::from(["shared", "low-only", "high-only"]));
    assert_eq!(
        result
            .iter()
            .filter(|descriptor| descriptor.id == "shared")
            .count(),
        1,
        "the losing plugin's duplicate descriptor for a contested device must not survive filtering"
    );
}

fn claimed_descriptor(id: &str, exclusivity: ClaimExclusivity) -> DeviceDescriptor {
    let mut descriptor = descriptor(id);
    descriptor.claims.push(luminate_plugin_api::HardwareClaim {
        bus: HardwareBus::Hid,
        physical_identity: "1234:5678:serial".to_owned(),
        control_domain: "lighting".to_owned(),
        exclusivity,
    });
    descriptor
}

#[test]
fn exclusive_claim_resolves_different_logical_ids_by_priority() {
    let metadata = vec![metadata("low", 1), metadata("high", 10)];
    let descriptors = vec![
        vec![claimed_descriptor(
            "generic-keyboard",
            ClaimExclusivity::Exclusive,
        )],
        vec![claimed_descriptor(
            "vendor-keyboard",
            ClaimExclusivity::Exclusive,
        )],
    ];

    let owners = arbitrate_ownership(&metadata, &descriptors);
    assert_eq!(owners, HashMap::from([("vendor-keyboard".to_owned(), 1)]));
}

#[test]
fn equal_priority_exclusive_claim_keeps_first_loaded() {
    let metadata = vec![metadata("first", 5), metadata("second", 5)];
    let descriptors = vec![
        vec![claimed_descriptor(
            "first-device",
            ClaimExclusivity::Exclusive,
        )],
        vec![claimed_descriptor(
            "second-device",
            ClaimExclusivity::Exclusive,
        )],
    ];

    let owners = arbitrate_ownership(&metadata, &descriptors);
    assert_eq!(owners, HashMap::from([("first-device".to_owned(), 0)]));
}

#[test]
fn shared_claims_can_coexist() {
    let metadata = vec![metadata("first", 5), metadata("second", 5)];
    let descriptors = vec![
        vec![claimed_descriptor("first-device", ClaimExclusivity::Shared)],
        vec![claimed_descriptor(
            "second-device",
            ClaimExclusivity::Shared,
        )],
    ];

    let owners = arbitrate_ownership(&metadata, &descriptors);
    assert_eq!(
        owners,
        HashMap::from([
            ("first-device".to_owned(), 0),
            ("second-device".to_owned(), 1),
        ])
    );
}

#[test]
fn exclusive_claim_falls_back_when_winner_withdraws() {
    let metadata = vec![metadata("fallback", 1), metadata("preferred", 10)];
    let fallback = claimed_descriptor("fallback-device", ClaimExclusivity::Exclusive);
    let preferred = claimed_descriptor("preferred-device", ClaimExclusivity::Exclusive);
    let owners = arbitrate_ownership(&metadata, &[vec![fallback.clone()], vec![preferred]]);
    assert_eq!(owners, HashMap::from([("preferred-device".to_owned(), 1)]));

    let owners = arbitrate_ownership(&metadata, &[vec![fallback], Vec::new()]);
    assert_eq!(owners, HashMap::from([("fallback-device".to_owned(), 0)]));
}

#[test]
fn aggregate_startup_validation_rejects_cross_plugin_name_conflict() {
    let first = descriptor("first-device");
    let mut conflicting = descriptor("second-device");
    conflicting.name.clone_from(&first.name);
    let descriptors_by_plugin = vec![vec![first]];
    let accepted_metadata = vec![metadata("accepted", 0)];
    let accepted_ids = vec![LoadedPluginId::new()];
    let candidate_metadata = metadata("conflicting", 0);

    let error = validate_startup_candidate(
        &accepted_metadata,
        &accepted_ids,
        &descriptors_by_plugin,
        LoadedPluginId::new(),
        &candidate_metadata,
        &[conflicting],
    )
    .expect_err("cross-plugin presentation-name conflict must be rejected as a candidate");
    assert!(format!("{error:#}").contains("duplicate device name"));
}

#[test]
fn invalid_optional_plugin_is_skipped_not_fatal() {
    // A malformed (e.g. invalid-topology) optional plugin must be isolated,
    // not abort the whole daemon.
    let outcome: Result<()> = Err(anyhow::anyhow!("plugin demo produced an invalid topology"));
    let classified = classify_load_outcome(outcome, false, Path::new("/plugins/demo.so"))
        .expect("an optional plugin failure must not abort startup");
    assert!(
        classified.is_none(),
        "an invalid optional plugin should be skipped"
    );
}

#[test]
fn invalid_required_plugin_aborts_startup() {
    let outcome: Result<()> = Err(anyhow::anyhow!("plugin demo produced an invalid topology"));
    let error = classify_load_outcome(outcome, true, Path::new("/plugins/demo.so"))
        .expect_err("a required plugin failure must abort startup");
    assert!(
        error.to_string().contains("required plugin failed to load"),
        "unexpected error: {error}"
    );
}

#[test]
fn successfully_loaded_plugin_is_kept() {
    let classified = classify_load_outcome(Ok(7_u8), false, Path::new("/plugins/demo.so"))
        .expect("a loaded plugin is not an error");
    assert_eq!(classified, Some(7));
}

#[test]
fn read_bus_slice_maps_unknown_codes_to_unknown() {
    // A plugin (buggy, or built against a different revision) can present a
    // bus code no `PluginBus` variant covers. Reading the array must not be
    // UB: `read_bus_slice` reads raw `u32`s and validates, so `99` becomes
    // `Unknown` while the surrounding valid codes are preserved.
    let codes: [u32; 3] = [PluginBus::Hid.to_abi(), 99, PluginBus::Usb.to_abi()];
    let buses = read_bus_slice(codes.as_ptr().cast::<PluginBus>(), codes.len());
    assert_eq!(
        buses,
        vec![PluginBus::Hid, PluginBus::Unknown, PluginBus::Usb]
    );
}

#[test]
fn read_bus_slice_of_null_or_empty_is_empty() {
    assert!(read_bus_slice(ptr::null(), 0).is_empty());
    let codes = [PluginBus::Hid.to_abi()];
    assert!(read_bus_slice(codes.as_ptr().cast::<PluginBus>(), 0).is_empty());
}

#[test]
fn plugin_target_from_target_id_covers_every_target_shape() {
    use luminate_core::element::ElementId;
    use luminate_core::group::GroupId;
    use luminate_core::surface::SurfaceId;

    match plugin_target_from_target_id(&TargetId::Device(DeviceId::new("keyboard"))) {
        PluginTarget::Device { device } => assert_eq!(device, "keyboard"),
        target @ (PluginTarget::Surface { .. }
        | PluginTarget::Element { .. }
        | PluginTarget::Group { .. }) => panic!("expected Device, got {target:?}"),
    }
    match plugin_target_from_target_id(&TargetId::Surface {
        device: DeviceId::new("keyboard"),
        surface: SurfaceId::new("zones"),
    }) {
        PluginTarget::Surface { device, surface } => {
            assert_eq!(device, "keyboard");
            assert_eq!(surface, "zones");
        }
        target @ (PluginTarget::Device { .. }
        | PluginTarget::Element { .. }
        | PluginTarget::Group { .. }) => panic!("expected Surface, got {target:?}"),
    }
    match plugin_target_from_target_id(&TargetId::Element {
        device: DeviceId::new("keyboard"),
        surface: SurfaceId::new("zones"),
        element: ElementId::new("zone-1"),
    }) {
        PluginTarget::Element {
            device,
            surface,
            element,
        } => {
            assert_eq!(device, "keyboard");
            assert_eq!(surface, "zones");
            assert_eq!(element, "zone-1");
        }
        target @ (PluginTarget::Device { .. }
        | PluginTarget::Surface { .. }
        | PluginTarget::Group { .. }) => panic!("expected Element, got {target:?}"),
    }
    match plugin_target_from_target_id(&TargetId::Group {
        device: DeviceId::new("keyboard"),
        group: GroupId::new("wasd"),
    }) {
        PluginTarget::Group { device, group } => {
            assert_eq!(device, "keyboard");
            assert_eq!(group, "wasd");
        }
        target @ (PluginTarget::Device { .. }
        | PluginTarget::Surface { .. }
        | PluginTarget::Element { .. }) => panic!("expected Group, got {target:?}"),
    }
}

fn set_brightness_update() -> PluginUpdate {
    PluginUpdate {
        target: plugin_target_from_target_id(&TargetId::Device(DeviceId::new("keyboard"))),
        operation: PluginUpdateOperation::SetBrightness { value: 50 },
    }
}

#[test]
fn apply_outcome_to_result_maps_applied_to_ok() {
    let update = set_brightness_update();
    assert!(apply_outcome_to_result("demo", &update, ApplyOutcome::Applied).is_ok());
}

#[test]
fn apply_outcome_to_result_maps_every_failure_variant() {
    let update = set_brightness_update();

    assert!(matches!(
        apply_outcome_to_result(
            "demo",
            &update,
            ApplyOutcome::Unsupported("no brightness facet".to_owned()),
        ),
        Err(DaemonError::PluginUnsupportedUpdate { plugin, device, .. })
            if plugin == "demo" && device == "keyboard"
    ));
    assert!(matches!(
        apply_outcome_to_result(
            "demo",
            &update,
            ApplyOutcome::InvalidArgument("value out of range".to_owned()),
        ),
        Err(DaemonError::PluginInvalidUpdate { plugin, device, .. })
            if plugin == "demo" && device == "keyboard"
    ));
    assert!(matches!(
        apply_outcome_to_result("demo", &update, ApplyOutcome::Io("timed out".to_owned())),
        Err(DaemonError::PluginIo { plugin, device, .. })
            if plugin == "demo" && device == "keyboard"
    ));
    assert!(matches!(
        apply_outcome_to_result(
            "demo",
            &update,
            ApplyOutcome::Unavailable("offline".to_owned()),
        ),
        Err(DaemonError::PluginUnavailable { plugin, device, .. })
            if plugin == "demo" && device == "keyboard"
    ));
    assert!(matches!(
        apply_outcome_to_result(
            "demo",
            &update,
            ApplyOutcome::RateLimited {
                diagnostic: "slow down".to_owned(),
                retry_after_ms: Some(250),
            },
        ),
        Err(DaemonError::PluginRateLimited { plugin, device, retry_after_ms: Some(250), .. })
            if plugin == "demo" && device == "keyboard"
    ));
    assert!(matches!(
        apply_outcome_to_result(
            "demo",
            &update,
            ApplyOutcome::Internal("panic recovered".to_owned()),
        ),
        Err(DaemonError::PluginInternal { plugin, device, .. })
            if plugin == "demo" && device == "keyboard"
    ));
}

#[test]
fn apply_outcome_error_maps_applied_to_ok_too() {
    // `apply_frame` routes through `apply_outcome_error` directly (it has no
    // success-side logging to do), so `Applied` must still short-circuit to
    // `Ok` there exactly as it does via `apply_outcome_to_result`.
    assert!(apply_outcome_error("demo", "keyboard", ApplyOutcome::Applied).is_ok());
}

#[test]
fn fill_results_writes_every_named_index_and_ignores_out_of_range() {
    let mut results: Vec<Option<Result<(), DaemonError>>> = vec![None, None, None];
    let mut calls = 0_usize;

    // `99` has no matching slot in `results`; `fill_results` must skip it via
    // `.get_mut` rather than panicking on an out-of-range index, and it must
    // not even invoke the closure for it.
    fill_results(&mut results, &[0, 2, 99], || {
        calls += 1;
        Err(DaemonError::Internal("batch failed".to_owned()))
    });

    assert_eq!(
        calls, 2,
        "the closure runs only for indices that exist in results"
    );
    assert!(matches!(results[0], Some(Err(DaemonError::Internal(_)))));
    assert!(
        results[1].is_none(),
        "index 1 was not named and must stay untouched"
    );
    assert!(matches!(results[2], Some(Err(DaemonError::Internal(_)))));
}

#[test]
fn warn_if_adopt_has_no_exact_readback_is_a_no_op_for_non_adopt_policy() {
    // Any recommended policy other than `Adopt` short-circuits before the
    // readback scan even runs, regardless of what the descriptors advertise.
    let mut plugin_metadata = metadata("demo", 0);
    plugin_metadata.recommended_reconciliation = Some(control::ReconciliationPolicy::Restore);
    warn_if_adopt_has_no_exact_readback(&plugin_metadata, &[descriptor("device")]);
}

#[test]
fn warn_if_adopt_has_no_exact_readback_is_a_no_op_when_a_device_has_exact_readback() {
    let mut plugin_metadata = metadata("demo", 0);
    plugin_metadata.recommended_reconciliation = Some(control::ReconciliationPolicy::Adopt);
    let mut device = descriptor("device");
    device.capabilities.state_readback = StateReadbackCapability::Readable {
        facets: vec![ReadableFacet {
            facet: StateFacetKind::Brightness,
            fidelity: ReadbackFidelity::Exact,
        }],
        read_disturbs_output: false,
        notifies_external_changes: false,
    };
    warn_if_adopt_has_no_exact_readback(&plugin_metadata, &[device]);
}

#[test]
fn warn_if_adopt_has_no_exact_readback_warns_when_no_device_has_exact_readback() {
    // Adopt reconciliation with no exact-readback facet anywhere is exactly
    // the misconfiguration this warns about; it must reach the log call
    // rather than returning early.
    let mut plugin_metadata = metadata("demo", 0);
    plugin_metadata.recommended_reconciliation = Some(control::ReconciliationPolicy::Adopt);
    warn_if_adopt_has_no_exact_readback(&plugin_metadata, &[descriptor("device")]);
}

#[test]
fn validate_startup_candidate_accepts_compatible_topology_and_returns_owners() {
    let accepted_metadata = vec![metadata("accepted", 0)];
    let accepted_id = LoadedPluginId::new();
    let descriptors_by_plugin = vec![vec![descriptor("first-device")]];
    let candidate_metadata = metadata("candidate", 0);
    let candidate_id = LoadedPluginId::new();
    let candidate_descriptors = vec![descriptor("second-device")];

    let owners = validate_startup_candidate(
        &accepted_metadata,
        &[accepted_id],
        &descriptors_by_plugin,
        candidate_id,
        &candidate_metadata,
        &candidate_descriptors,
    )
    .expect("disjoint device IDs and names must not conflict");

    assert_eq!(
        owners,
        HashMap::from([
            ("first-device".to_owned(), accepted_id),
            ("second-device".to_owned(), candidate_id),
        ])
    );
}

fn empty_plugin_manager() -> PluginManager {
    PluginManager::load(&base_config(Vec::new()), &ManagedConfig::default(), false)
        .expect("load empty plugin manager")
}

#[test]
fn unload_plugin_reports_not_found_for_an_unknown_name() {
    let manager = empty_plugin_manager();
    let error = manager
        .unload_plugin("does-not-exist")
        .expect_err("unloading an unknown plugin must fail");
    assert!(matches!(error, DaemonError::PluginNotFound(name) if name == "does-not-exist"));
}

#[test]
fn reload_plugin_reports_not_found_for_an_unknown_name() {
    let manager = empty_plugin_manager();
    let error = manager
        .reload_plugin("does-not-exist")
        .expect_err("reloading an unknown plugin must fail");
    assert!(matches!(error, DaemonError::PluginNotFound(name) if name == "does-not-exist"));
}

#[test]
fn empty_manager_exposes_empty_management_and_reconciliation_state() {
    let manager = empty_plugin_manager();
    let global = DaemonConfig::default();
    let configuration = ManagedConfig::default();

    assert!(manager.management_plugins().is_empty());
    assert_eq!(manager.setup_workflows("does-not-exist"), None);
    assert!(manager.manageable_plugins().is_empty());
    assert!(
        manager
            .apply_managed_config(&global, &configuration)
            .expect("empty catalogue has no configuration errors")
            .is_empty()
    );

    let reconciliation = manager.reconcile_managed_plugins(&[]);
    assert!(!reconciliation.activated);
    assert!(reconciliation.topology.is_none());
}

#[test]
fn managed_plugin_actions_cover_runtime_transitions() {
    assert_eq!(
        managed_plugin_action(&PluginRuntimeState::Loaded, false, false),
        Some(ManagedPluginAction::Deactivate)
    );
    assert_eq!(
        managed_plugin_action(&PluginRuntimeState::Inactive, true, false),
        Some(ManagedPluginAction::Activate)
    );
    assert_eq!(
        managed_plugin_action(&PluginRuntimeState::Failed("nope".to_owned()), true, false),
        Some(ManagedPluginAction::Activate)
    );
    assert_eq!(
        managed_plugin_action(&PluginRuntimeState::Loaded, true, true),
        Some(ManagedPluginAction::Restart)
    );

    for runtime in [
        PluginRuntimeState::Inactive,
        PluginRuntimeState::Loading,
        PluginRuntimeState::Failed("nope".to_owned()),
    ] {
        assert_eq!(managed_plugin_action(&runtime, false, true), None);
    }
    assert_eq!(
        managed_plugin_action(&PluginRuntimeState::Loaded, true, false),
        None
    );
}

#[test]
fn failed_managed_restart_keeps_withdrawn_topology_for_commit() {
    let withdrawn_device = DeviceId::new("withdrawn");
    let outcome = run_managed_plugin_action(
        ManagedPluginAction::Restart,
        || Err(anyhow::anyhow!("replacement host failed")),
        || {
            Ok(TopologyReconcile {
                devices: Vec::new(),
                changed_devices: vec![withdrawn_device.clone()],
            })
        },
    );

    assert!(!outcome.activated);
    assert!(outcome.error.is_some());
    assert_eq!(outcome.reconciliations.len(), 1);
    assert!(outcome.reconciliations[0].devices.is_empty());
    assert_eq!(
        outcome.reconciliations[0].changed_devices,
        vec![withdrawn_device]
    );
}

#[test]
fn managed_actions_preserve_successful_phases_and_stop_after_failure() {
    use std::cell::Cell;

    let reconcile = || TopologyReconcile {
        devices: Vec::new(),
        changed_devices: Vec::new(),
    };
    let restarted = run_managed_plugin_action(
        ManagedPluginAction::Restart,
        || Ok(reconcile()),
        || Ok(reconcile()),
    );
    assert!(restarted.activated);
    assert!(restarted.error.is_none());
    assert_eq!(
        restarted.reconciliations.len(),
        2,
        "a restart commits withdrawal before replacement topology"
    );

    let activation_called = Cell::new(false);
    let failed = run_managed_plugin_action(
        ManagedPluginAction::Restart,
        || {
            activation_called.set(true);
            Ok(reconcile())
        },
        || Err(anyhow::anyhow!("withdrawal failed")),
    );
    assert!(!activation_called.get());
    assert!(!failed.activated);
    assert!(failed.reconciliations.is_empty());
    assert!(failed.error.is_some());

    let deactivated = run_managed_plugin_action(
        ManagedPluginAction::Deactivate,
        || Err(anyhow::anyhow!("activation must not run")),
        || Ok(reconcile()),
    );
    assert!(!deactivated.activated);
    assert_eq!(deactivated.reconciliations.len(), 1);
}

#[test]
fn reload_all_is_a_no_op_when_nothing_is_loaded() {
    let manager = empty_plugin_manager();
    assert!(manager.reload_all().is_empty());
}

#[test]
fn read_probe_hints_tolerates_an_unknown_kind() {
    // A probe hint whose `kind` discriminant is invalid must be read
    // without UB (via the `RawProbeHint` mirror) and its `value` still
    // returned. The daemon only logs the unrecognized kind.
    let value = CString::new("vidpid:0d62:1234").expect("no interior nul");
    let raw = [RawProbeHint {
        kind: 4242,
        value: value.as_ptr(),
    }];
    let hints = read_probe_hints(raw.as_ptr().cast::<PluginProbeHint>(), raw.len())
        .expect("an unknown kind must not fail the read");
    assert_eq!(hints, vec!["vidpid:0d62:1234".to_owned()]);
}
