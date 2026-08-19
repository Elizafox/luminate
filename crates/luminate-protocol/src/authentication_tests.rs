// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for bounded authentication secrets and event tickets.

use super::*;

#[test]
fn credential_debug_is_redacted() {
    let credential = Credential::new(b"do-not-log-me").expect("valid credential");
    let diagnostic = format!("{credential:?}");

    assert_eq!(diagnostic, "Credential([REDACTED])");
    assert!(!diagnostic.contains("do-not-log-me"));
}

#[test]
fn credential_is_bounded() {
    assert!(matches!(Credential::new([]), Err(CredentialError::Empty)));
    assert_eq!(
        Credential::new(vec![0; MAX_CREDENTIAL_BYTES])
            .expect("maximum-sized credential")
            .expose()
            .len(),
        MAX_CREDENTIAL_BYTES
    );
    assert!(matches!(
        Credential::new(vec![0; MAX_CREDENTIAL_BYTES + 1]),
        Err(CredentialError::TooLarge { .. })
    ));
}

#[test]
fn credential_deserialization_reapplies_the_constructor_bounds() {
    let mut empty_input = Vec::new();
    ciborium::into_writer(&Vec::<u8>::new(), &mut empty_input).expect("serialize empty input");
    let empty = ciborium::from_reader::<Credential, _>(empty_input.as_slice())
        .expect_err("an empty serialized credential must be rejected");
    assert!(empty.to_string().contains("credential is empty"));

    let mut oversized = Vec::new();
    ciborium::into_writer(&vec![0_u8; MAX_CREDENTIAL_BYTES + 1], &mut oversized)
        .expect("serialize oversized input");
    let error = ciborium::from_reader::<Credential, _>(oversized.as_slice())
        .expect_err("an oversized serialized credential must be rejected");
    assert!(error.to_string().contains("maximum"));
}

#[test]
fn continuation_is_redacted_and_independently_bounded() {
    let continuation = Continuation::new(b"provider-private-state").expect("valid continuation");
    let diagnostic = format!("{continuation:?}");

    assert_eq!(diagnostic, "Continuation([REDACTED])");
    assert!(!diagnostic.contains("provider-private-state"));
    assert!(matches!(
        Continuation::new([]),
        Err(ContinuationError::Empty)
    ));
    assert_eq!(
        Continuation::new(vec![0; MAX_CONTINUATION_BYTES])
            .expect("maximum-sized continuation")
            .expose()
            .len(),
        MAX_CONTINUATION_BYTES
    );
    assert!(matches!(
        Continuation::new(vec![0; MAX_CONTINUATION_BYTES + 1]),
        Err(ContinuationError::TooLarge { .. })
    ));
}

#[test]
fn continuation_deserialization_reapplies_the_constructor_bounds() {
    let mut empty_input = Vec::new();
    ciborium::into_writer(&Vec::<u8>::new(), &mut empty_input).expect("serialize empty input");
    let empty = ciborium::from_reader::<Continuation, _>(empty_input.as_slice())
        .expect_err("an empty serialized continuation must be rejected");
    assert!(empty.to_string().contains("provider continuation is empty"));

    let mut oversized = Vec::new();
    ciborium::into_writer(&vec![0_u8; MAX_CONTINUATION_BYTES + 1], &mut oversized)
        .expect("serialize oversized input");
    let error = ciborium::from_reader::<Continuation, _>(oversized.as_slice())
        .expect_err("an oversized serialized continuation must be rejected");
    assert!(error.to_string().contains("maximum"));
}

#[test]
fn event_tickets_are_redacted_bounded_and_compared_by_value() {
    let ticket = EventTicket::new("one-use-secret").expect("valid event ticket");
    assert_eq!(ticket.expose(), b"one-use-secret");
    assert_eq!(format!("{ticket:?}"), "EventTicket([REDACTED])");
    assert_eq!(
        ticket,
        EventTicket::new("one-use-secret").expect("equal event ticket")
    );
    assert_ne!(
        ticket,
        EventTicket::new("one-use-secreu").expect("same-length unequal event ticket")
    );
    assert_ne!(
        ticket,
        EventTicket::new("short").expect("different-length event ticket")
    );
    assert!(matches!(EventTicket::new([]), Err(CredentialError::Empty)));
}

#[test]
fn event_ticket_round_trip_preserves_value_and_rejects_invalid_input() {
    let ticket = EventTicket::new("ticket").expect("valid event ticket");
    let mut encoded = Vec::new();
    ciborium::into_writer(&ticket, &mut encoded).expect("serialize event ticket");
    assert_eq!(
        ciborium::from_reader::<EventTicket, _>(encoded.as_slice())
            .expect("deserialize event ticket"),
        ticket
    );
    assert!(ciborium::from_reader::<EventTicket, _>([0x80].as_slice()).is_err());
}
