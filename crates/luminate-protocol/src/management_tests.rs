// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for managed-configuration wire models and redaction.

use super::*;
use crate::{ErrorCode, Event, OperationError, Request, ResponseStatus};

#[test]
fn write_only_debug_never_contains_the_value() {
    let mutation = ManagementMutation::SetPluginSetting {
        plugin: "example".to_owned(),
        key: "credentials.token".to_owned(),
        value: WriteOnly::new(SettingValue::String("super-secret".to_owned())),
    };

    let debug = format!("{mutation:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("super-secret"));
}

#[test]
fn management_mutation_debug_covers_every_variant() {
    let preferences = DaemonPreferences {
        default_unsupported_policy: Some(UnsupportedPolicy::Reject),
        reconciliation_policy: Some(ReconciliationPolicy::Adopt),
        device_reconciliation: vec![DeviceReconciliationPreference {
            device: DeviceId::new("lamp"),
            policy: ReconciliationPolicy::Restore,
        }],
        cct_emulation: Some(CctEmulation::Auto),
        prefer_shm: Some(true),
        prefer_client_shm: Some(false),
    };
    let mutations = [
        ManagementMutation::SetDaemonPreferences(preferences),
        ManagementMutation::SetPluginEnabled {
            plugin: "demo".to_owned(),
            enabled: Some(true),
        },
        ManagementMutation::SetPluginReconciliation {
            plugin: "demo".to_owned(),
            reconciliation: Some(ReconciliationPolicy::Leave),
        },
        ManagementMutation::SetPluginSetting {
            plugin: "demo".to_owned(),
            key: "mode".to_owned(),
            value: WriteOnly::new(SettingValue::String("safe".to_owned())),
        },
        ManagementMutation::ClearPluginSetting {
            plugin: "demo".to_owned(),
            key: "mode".to_owned(),
        },
    ];

    for mutation in mutations {
        let debug = format!("{mutation:?}");
        assert!(debug.contains("demo") || debug.contains("SetDaemonPreferences"));
    }
}

#[test]
fn write_only_value_survives_transport_serialization() {
    let value = WriteOnly::new(SettingValue::String("transport-secret".to_owned()));
    let mut bytes = Vec::new();
    ciborium::into_writer(&value, &mut bytes).expect("serialize write-only value");
    let decoded: WriteOnly<SettingValue> =
        ciborium::from_reader(bytes.as_slice()).expect("deserialize write-only value");

    assert_eq!(
        decoded.into_inner(),
        SettingValue::String("transport-secret".to_owned())
    );
}

#[test]
fn reported_redaction_serializes_without_a_value() {
    let mut bytes = Vec::new();
    ciborium::into_writer(&ReportedSettingValue::Redacted, &mut bytes)
        .expect("serialize redacted value");
    let decoded: ReportedSettingValue =
        ciborium::from_reader(bytes.as_slice()).expect("deserialize redacted value");

    assert_eq!(decoded, ReportedSettingValue::Redacted);
    assert!(!String::from_utf8_lossy(&bytes).contains("secret"));
}

#[test]
fn change_records_carry_keys_but_no_values() {
    let changes = ManagementChangeSet {
        revision: 9,
        changes: vec![ManagementChange::PluginSettingChanged {
            plugin: "example".to_owned(),
            key: "credentials.token".to_owned(),
            sensitive: true,
        }],
    };
    let mut bytes = Vec::new();
    ciborium::into_writer(&changes, &mut bytes).expect("serialize changes");
    let decoded: ManagementChangeSet =
        ciborium::from_reader(bytes.as_slice()).expect("deserialize changes");

    assert_eq!(decoded, changes);
}

#[test]
fn management_patch_request_preserves_secret_only_for_the_receiver() {
    let request = Request::PatchManagement {
        patch: ManagementPatch {
            expected_revision: 4,
            mutations: vec![ManagementMutation::SetPluginSetting {
                plugin: "example".to_owned(),
                key: "credentials.token".to_owned(),
                value: WriteOnly::new(SettingValue::String("transport-secret".to_owned())),
            }],
        },
    };
    let debug = format!("{request:?}");
    assert!(!debug.contains("transport-secret"));

    let mut bytes = Vec::new();
    ciborium::into_writer(&request, &mut bytes).expect("serialize management patch request");
    let decoded: Request =
        ciborium::from_reader(bytes.as_slice()).expect("deserialize management patch request");

    let Request::PatchManagement { patch } = decoded else {
        panic!("decoded the wrong request variant");
    };
    let Some(ManagementMutation::SetPluginSetting { value, .. }) =
        patch.mutations.into_iter().next()
    else {
        panic!("decoded the wrong management mutation");
    };
    assert_eq!(
        value.into_inner(),
        SettingValue::String("transport-secret".to_owned())
    );
}

#[test]
fn management_response_and_event_carry_only_redacted_changes() {
    let changes = ManagementChangeSet {
        revision: 5,
        changes: vec![ManagementChange::PluginSettingChanged {
            plugin: "example".to_owned(),
            key: "credentials.token".to_owned(),
            sensitive: true,
        }],
    };
    let response = ResponseStatus::ManagementPatched(changes.clone());
    let event = Event::ConfigurationChanged { changes };

    let mut response_bytes = Vec::new();
    ciborium::into_writer(&response, &mut response_bytes).expect("serialize management response");
    let decoded_response: ResponseStatus =
        ciborium::from_reader(response_bytes.as_slice()).expect("deserialize management response");
    assert!(matches!(
        decoded_response,
        ResponseStatus::ManagementPatched(ManagementChangeSet { revision: 5, .. })
    ));

    let mut event_bytes = Vec::new();
    ciborium::into_writer(&event, &mut event_bytes).expect("serialize management event");
    let decoded_event: Event =
        ciborium::from_reader(event_bytes.as_slice()).expect("deserialize management event");
    assert_eq!(decoded_event, event);
}

#[test]
fn stale_revision_uses_the_structured_conflict_error() {
    let status = ResponseStatus::Error(OperationError {
        code: ErrorCode::Conflict,
        message: "expected revision 4, current revision is 5".to_owned(),
        retry_after_ms: None,
        applied_targets: Vec::new(),
    });
    let mut bytes = Vec::new();
    ciborium::into_writer(&status, &mut bytes).expect("serialize conflict response");
    let decoded: ResponseStatus =
        ciborium::from_reader(bytes.as_slice()).expect("deserialize conflict response");

    assert!(matches!(
        decoded,
        ResponseStatus::Error(OperationError {
            code: ErrorCode::Conflict,
            ..
        })
    ));
}
