// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use super::{PluginBus, PluginLogLevel, ProbeHintKind, ProbeOutcome, RescanReason, abi_bool};

#[test]
fn probe_outcome_abi_round_trips_and_rejects_unknown_values() {
    for outcome in [
        ProbeOutcome::Unsupported,
        ProbeOutcome::Dormant,
        ProbeOutcome::Ready,
    ] {
        assert_eq!(ProbeOutcome::from_abi(outcome.to_abi()), Some(outcome));
    }
    assert_eq!(ProbeOutcome::from_abi(3), None);
    assert_eq!(ProbeOutcome::from_abi(u8::MAX), None);
}

#[test]
fn plugin_bus_abi_codes_round_trip() {
    for bus in [
        PluginBus::Unknown,
        PluginBus::Usb,
        PluginBus::Hid,
        PluginBus::I2c,
        PluginBus::Platform,
        PluginBus::Network,
    ] {
        assert_eq!(PluginBus::from_abi(bus.to_abi()), Some(bus));
    }
}

#[test]
fn plugin_bus_from_abi_rejects_unknown_codes() {
    // A discriminant no variant covers must be rejectable, not UB.
    assert_eq!(PluginBus::from_abi(6), None);
    assert_eq!(PluginBus::from_abi(u32::MAX), None);
}

#[test]
fn probe_hint_kind_abi_codes_round_trip() {
    for kind in [
        ProbeHintKind::None,
        ProbeHintKind::UsbVidPid,
        ProbeHintKind::HidVidPid,
        ProbeHintKind::DmiMatch,
    ] {
        assert_eq!(ProbeHintKind::from_abi(kind.to_abi()), Some(kind));
    }
    assert_eq!(ProbeHintKind::from_abi(4), None);
}

#[test]
fn plugin_log_level_abi_bytes_round_trip() {
    for level in [
        PluginLogLevel::Error,
        PluginLogLevel::Warn,
        PluginLogLevel::Info,
        PluginLogLevel::Debug,
        PluginLogLevel::Trace,
    ] {
        assert_eq!(PluginLogLevel::from_abi(level.to_abi()), Some(level));
    }
    assert_eq!(PluginLogLevel::from_abi(5), None);
    assert_eq!(PluginLogLevel::from_abi(u8::MAX), None);
}

#[test]
fn rescan_reason_abi_bytes_round_trip_and_reject_unknown_values() {
    for reason in [
        RescanReason::Resume,
        RescanReason::DeviceChange,
        RescanReason::Operator,
    ] {
        assert_eq!(RescanReason::from_abi(reason.to_abi()), Some(reason));
    }
    assert_eq!(RescanReason::from_abi(3), None);
    assert_eq!(RescanReason::from_abi(u8::MAX), None);
}

#[test]
fn abi_bool_decode_is_total_over_every_byte() {
    // A plugin may write any result byte; the decode must be defined for
    // all of them (nonzero = true), never producing an invalid `bool`.
    assert!(!abi_bool(0));
    assert!(abi_bool(1));
    assert!(abi_bool(2));
    assert!(abi_bool(u8::MAX));
}
