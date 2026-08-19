// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! D-Bus error-name and structured-payload tests.

use super::*;
use zbus::DBusError as _;
use zbus::message::Message;

fn reply(error: &MethodError) -> Message {
    let call = Message::method_call("/", "Test")
        .expect("build method call")
        .build(&())
        .expect("serialize method call");
    error
        .create_reply(&call.header())
        .expect("serialize error reply")
}

#[test]
fn public_errors_have_stable_dbus_names() {
    let error = MethodError::from(luminate::Error::NotFound("lamp".into()));
    assert_eq!(error.name().as_str(), "org.luminate.Error.NotFound");
}

#[test]
fn rate_limit_reply_carries_typed_retry_guidance() {
    let error = MethodError::from(luminate::Error::RateLimited {
        message: "provider asked us to wait".into(),
        retry_after_ms: Some(750),
    });

    let body = reply(&error)
        .body()
        .deserialize::<(String, bool, u64)>()
        .expect("deserialize rate-limit error body");
    assert_eq!(
        body,
        ("rate limited: provider asked us to wait".into(), true, 750)
    );
}

#[test]
fn absent_retry_guidance_has_an_explicit_presence_flag() {
    let error = MethodError::from(luminate::Error::RateLimited {
        message: "provider is busy".into(),
        retry_after_ms: None,
    });

    let body = reply(&error)
        .body()
        .deserialize::<(String, bool, u64)>()
        .expect("deserialize rate-limit error body");
    assert_eq!(body, ("rate limited: provider is busy".into(), false, 0));
}

#[test]
fn partial_mutation_reply_carries_canonical_target_ids() {
    let error = MethodError::from(luminate::Error::PartialMutation {
        message: "later target failed".into(),
        applied_targets: vec![
            luminate::TargetId::device("keyboard0"),
            luminate::TargetId::element("keyboard0", "keys", "escape"),
        ],
    });

    let body = reply(&error)
        .body()
        .deserialize::<(String, Vec<String>)>()
        .expect("deserialize partial-mutation error body");
    assert_eq!(
        body,
        (
            "partial mutation: later target failed".into(),
            vec![
                "device:keyboard0".into(),
                "device:keyboard0/surface:keys/element:escape".into(),
            ],
        )
    );
}
