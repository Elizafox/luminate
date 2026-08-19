// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use super::{Compatibility, HostHello, SupervisorHello};

#[test]
fn supervisor_hello_carries_the_current_protocol_version() {
    let hello = SupervisorHello::new();
    assert_eq!(
        hello.protocol_version,
        crate::HOST_SUPERVISOR_PROTOCOL_VERSION
    );
}

#[test]
fn compatibility_check_accepts_a_matching_version() {
    assert_eq!(
        Compatibility::check(crate::HOST_SUPERVISOR_PROTOCOL_VERSION),
        Compatibility::Compatible
    );
}

#[test]
fn compatibility_check_rejects_a_mismatched_version() {
    let mismatched = crate::HOST_SUPERVISOR_PROTOCOL_VERSION.wrapping_add(1);
    match Compatibility::check(mismatched) {
        Compatibility::Incompatible {
            supported_protocol_version,
            reason,
        } => {
            assert_eq!(
                supported_protocol_version,
                crate::HOST_SUPERVISOR_PROTOCOL_VERSION
            );
            assert!(reason.is_none());
        }
        other @ Compatibility::Compatible => panic!("expected Incompatible, got {other:?}"),
    }
}

#[test]
fn host_hello_round_trips_through_cbor() {
    let hello = HostHello {
        compatibility: Compatibility::Incompatible {
            supported_protocol_version: 1,
            reason: Some("host built against an older protocol".to_owned()),
        },
        protocol_version: 2,
        host_version: "0.1.0".to_owned(),
    };
    let mut encoded = Vec::new();
    ciborium::into_writer(&hello, &mut encoded).expect("encode host hello");
    let decoded: HostHello = ciborium::from_reader(encoded.as_slice()).expect("decode host hello");
    assert_eq!(decoded, hello);
}
