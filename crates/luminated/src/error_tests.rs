// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use luminate_core::device::DeviceId;

use super::*;

#[test]
fn target_not_found_maps_to_not_found() {
    let error = DaemonError::TargetNotFound(TargetId::Device(DeviceId::new("device0")));
    assert_eq!(error.error_code(), ErrorCode::NotFound);
}

#[test]
fn structured_plugin_failures_map_to_public_codes() {
    let unsupported = DaemonError::PluginUnsupportedUpdate {
        plugin: "some-plugin".to_owned(),
        device: "device0".to_owned(),
        diagnostic: "not supported".to_owned(),
    };
    let invalid = DaemonError::PluginInvalidUpdate {
        plugin: "read-only-plugin".to_owned(),
        device: "sensor0".to_owned(),
        diagnostic: "bad argument".to_owned(),
    };
    let io = DaemonError::PluginIo {
        plugin: "network-plugin".to_owned(),
        device: "bulb0".to_owned(),
        diagnostic: "timed out".to_owned(),
    };
    let unavailable = DaemonError::PluginUnavailable {
        plugin: "network-plugin".to_owned(),
        device: "bulb0".to_owned(),
        diagnostic: "offline".to_owned(),
    };
    let rate_limited = DaemonError::PluginRateLimited {
        plugin: "hid-plugin".to_owned(),
        device: "keyboard0".to_owned(),
        diagnostic: "retry later".to_owned(),
        retry_after_ms: Some(1_000),
    };
    let internal = DaemonError::PluginInternal {
        plugin: "broken-plugin".to_owned(),
        device: "device0".to_owned(),
        diagnostic: "invariant failed".to_owned(),
    };
    assert_eq!(unsupported.error_code(), ErrorCode::Unsupported);
    assert_eq!(invalid.error_code(), ErrorCode::InvalidArgument);
    assert_eq!(io.error_code(), ErrorCode::Io);
    assert_eq!(unavailable.error_code(), ErrorCode::Unavailable);
    assert_eq!(rate_limited.error_code(), ErrorCode::RateLimited);
    assert_eq!(internal.error_code(), ErrorCode::Internal);
}

#[test]
fn capability_errors_map_to_public_codes() {
    let target = TargetId::Device(DeviceId::new("device0"));
    assert_eq!(
        DaemonError::UnsupportedCapability {
            target: target.clone(),
            reason: "no colour output".to_owned(),
        }
        .error_code(),
        ErrorCode::Unsupported
    );
    assert_eq!(
        DaemonError::InvalidArgument {
            target,
            reason: "brightness out of range".to_owned(),
        }
        .error_code(),
        ErrorCode::InvalidArgument
    );
}

#[test]
fn unknown_state_maps_to_unknown_state() {
    let error = DaemonError::UnknownState {
        target: TargetId::Device(DeviceId::new("device0")),
    };
    assert_eq!(error.error_code(), ErrorCode::UnknownState);
}

#[test]
fn internal_inconsistencies_map_to_internal() {
    assert_eq!(
        DaemonError::DeviceUnowned("device0".to_owned()).error_code(),
        ErrorCode::Internal
    );
    assert_eq!(
        DaemonError::Internal("index out of range".to_owned()).error_code(),
        ErrorCode::Internal
    );
    assert_eq!(
        DaemonError::Other(anyhow::anyhow!("unexpected")).error_code(),
        ErrorCode::Internal
    );
}
