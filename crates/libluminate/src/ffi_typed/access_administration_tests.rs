// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::borrow_as_ptr,
    clippy::too_many_lines,
    reason = "these tests deliberately call the public C ABI with borrowed Rust fixtures"
)]

use super::*;
use crate::ffi::FfiClient;
use std::ffi::CString;

fn principal() -> PrincipalId {
    PrincipalId::new("local", "alice").expect("valid principal")
}

fn view(value: LuminateStringView) -> Option<&'static str> {
    if value.data.is_null() {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(value.data.cast::<u8>(), value.len) };
    std::str::from_utf8(bytes).ok()
}

#[test]
fn token_views_cover_display_once_metadata_expiry_and_boundaries() {
    let expiry = UNIX_EPOCH + Duration::from_millis(1_234);
    let token = LuminateCreatedToken {
        metadata: TokenMetadata {
            id: "desk".to_owned(),
            subject: principal(),
            expires_at: Some(expiry),
            revoked: false,
        },
        secret: Credential::new("display-once").expect("credential"),
    };
    let mut copied = [0_u8; 4];
    assert_eq!(
        unsafe { luminate_created_token_secret(&token, copied.as_mut_ptr(), copied.len()) },
        12
    );
    assert_eq!(&copied, b"disp");
    assert_eq!(
        unsafe { luminate_created_token_secret(&token, ptr::null_mut(), 0) },
        12
    );
    assert_eq!(
        view(unsafe { luminate_created_token_id(&token) }),
        Some("desk")
    );
    assert_eq!(
        view(unsafe { luminate_created_token_authority(&token) }),
        Some("local")
    );
    assert_eq!(
        view(unsafe { luminate_created_token_subject(&token) }),
        Some("alice")
    );
    let mut expires_at = 0;
    assert!(unsafe { luminate_created_token_expires_at_unix_ms(&token, &mut expires_at) });
    assert_eq!(expires_at, 1_234);

    let tokens = LuminateTokenList(vec![
        token.metadata.clone(),
        TokenMetadata {
            id: "revoked".to_owned(),
            subject: principal(),
            expires_at: None,
            revoked: true,
        },
    ]);
    assert_eq!(unsafe { luminate_token_list_count(&tokens) }, 2);
    let revoked = unsafe { luminate_token_list_at(&tokens, 1) };
    assert!(!revoked.is_null());
    assert_eq!(view(unsafe { luminate_token_id(revoked) }), Some("revoked"));
    assert_eq!(
        view(unsafe { luminate_token_authority(revoked) }),
        Some("local")
    );
    assert_eq!(
        view(unsafe { luminate_token_subject(revoked) }),
        Some("alice")
    );
    assert!(unsafe { luminate_token_revoked(revoked) });
    assert!(!unsafe { luminate_token_expires_at_unix_ms(revoked, &mut expires_at) });
    assert!(unsafe { luminate_token_list_at(&tokens, 9) }.is_null());
    assert_eq!(unsafe { luminate_token_list_count(ptr::null()) }, 0);
}

#[test]
fn attestation_views_cover_groups_expiry_and_null_sentinels() {
    let attestation = LuminateCreatedAttestation {
        metadata: AttestationMetadata {
            name: "browser".to_owned(),
            subject: principal(),
            verified_groups: vec!["operators".to_owned(), "staff".to_owned()],
            credential_id: "attestation:browser".to_owned(),
            expires_at: Some(UNIX_EPOCH + Duration::from_secs(5)),
        },
        secret: Credential::new("secret").expect("credential"),
    };
    assert_eq!(
        unsafe { luminate_created_attestation_group_count(&attestation) },
        2
    );
    assert_eq!(
        view(unsafe { luminate_created_attestation_group_at(&attestation, 0) }),
        Some("operators")
    );
    assert!(view(unsafe { luminate_created_attestation_group_at(&attestation, 9) }).is_none());
    assert_eq!(
        view(unsafe { luminate_created_attestation_credential_id(&attestation) }),
        Some("attestation:browser")
    );
    assert_eq!(
        view(unsafe { luminate_created_attestation_name(&attestation) }),
        Some("browser")
    );
    assert_eq!(
        view(unsafe { luminate_created_attestation_authority(&attestation) }),
        Some("local")
    );
    assert_eq!(
        view(unsafe { luminate_created_attestation_subject(&attestation) }),
        Some("alice")
    );
    let mut expires_at = 0;
    assert!(unsafe {
        luminate_created_attestation_expires_at_unix_ms(&attestation, &mut expires_at)
    });
    assert_eq!(expires_at, 5_000);

    let records = LuminateAttestationList(vec![attestation.metadata.clone()]);
    assert_eq!(unsafe { luminate_attestation_list_count(&records) }, 1);
    let record = unsafe { luminate_attestation_list_at(&records, 0) };
    assert!(!record.is_null());
    assert_eq!(
        view(unsafe { luminate_attestation_name(record) }),
        Some("browser")
    );
    assert_eq!(
        view(unsafe { luminate_attestation_authority(record) }),
        Some("local")
    );
    assert_eq!(
        view(unsafe { luminate_attestation_subject(record) }),
        Some("alice")
    );
    assert_eq!(
        view(unsafe { luminate_attestation_credential_id(record) }),
        Some("attestation:browser")
    );
    assert_eq!(unsafe { luminate_attestation_group_count(record) }, 2);
    assert_eq!(
        view(unsafe { luminate_attestation_group_at(record, 1) }),
        Some("staff")
    );
    assert_eq!(
        unsafe { luminate_created_attestation_secret(ptr::null(), ptr::null_mut(), 0) },
        0
    );
    assert!(!unsafe { luminate_attestation_expires_at_unix_ms(record, ptr::null_mut()) });
    assert_eq!(unsafe { luminate_attestation_list_count(ptr::null()) }, 0);
}

#[test]
fn optional_expiry_distinguishes_absent_valid_and_out_of_range_values() {
    assert_eq!(
        optional_expiry(false, u64::MAX).expect("absent expiry"),
        None
    );
    assert_eq!(
        optional_expiry(true, 1_000).expect("valid expiry"),
        Some(UNIX_EPOCH + Duration::from_secs(1))
    );
}

#[test]
fn administration_operations_validate_complete_inputs_before_dispatch() {
    let client = FfiClient::stopped();
    let client = (&raw const client).cast::<LuminateClient>();
    let name = CString::new("browser").expect("C string");
    let id = CString::new("desk-token").expect("C string");
    let authority = CString::new("local").expect("C string");
    let subject = CString::new("alice").expect("C string");
    let operator = CString::new("operators").expect("C string");
    let groups = [operator.as_ptr()];
    let mut created_attestation = ptr::null_mut();
    let mut attestations = ptr::null_mut();
    let mut created_token = ptr::null_mut();
    let mut tokens = ptr::null_mut();

    // A stopped worker lets these calls exercise the complete public input
    // path without depending on a daemon or treating its replies as fixtures.
    unsafe {
        assert_eq!(
            luminate_client_create_attestation(
                client,
                name.as_ptr(),
                authority.as_ptr(),
                subject.as_ptr(),
                false,
                0,
                &raw mut created_attestation,
            ),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_create_principal_attestation(
                client,
                name.as_ptr(),
                authority.as_ptr(),
                subject.as_ptr(),
                groups.as_ptr(),
                groups.len(),
                true,
                1_000,
                &raw mut created_attestation,
            ),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_list_attestations(client, &raw mut attestations),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_revoke_attestation(client, name.as_ptr()),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_create_token(
                client,
                id.as_ptr(),
                authority.as_ptr(),
                subject.as_ptr(),
                false,
                0,
                &raw mut created_token,
            ),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_list_tokens(client, &raw mut tokens),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_revoke_token(client, id.as_ptr()),
            LuminateStatus::Internal
        );
        assert_eq!(
            luminate_client_rotate_token(client, id.as_ptr(), true, 2_000, &raw mut created_token,),
            LuminateStatus::Internal
        );
    }
}

#[test]
fn administration_operations_reject_invalid_public_inputs() {
    let client = FfiClient::stopped();
    let client = (&raw const client).cast::<LuminateClient>();
    let valid = CString::new("valid").expect("C string");
    let empty = CString::new("").expect("C string");
    let invalid_utf8 = [0xff_u8, 0];
    let mut created_attestation = ptr::null_mut();
    let mut created_token = ptr::null_mut();

    unsafe {
        assert_eq!(
            luminate_client_create_principal_attestation(
                client,
                ptr::null(),
                valid.as_ptr(),
                valid.as_ptr(),
                ptr::null(),
                0,
                false,
                0,
                &raw mut created_attestation,
            ),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_create_principal_attestation(
                client,
                invalid_utf8.as_ptr().cast(),
                valid.as_ptr(),
                valid.as_ptr(),
                ptr::null(),
                0,
                false,
                0,
                &raw mut created_attestation,
            ),
            LuminateStatus::InvalidUtf8
        );
        assert_eq!(
            luminate_client_create_principal_attestation(
                client,
                valid.as_ptr(),
                empty.as_ptr(),
                valid.as_ptr(),
                ptr::null(),
                0,
                false,
                0,
                &raw mut created_attestation,
            ),
            LuminateStatus::InvalidArgument
        );
        assert_eq!(
            luminate_client_create_principal_attestation(
                client,
                valid.as_ptr(),
                valid.as_ptr(),
                valid.as_ptr(),
                ptr::null(),
                1,
                false,
                0,
                &raw mut created_attestation,
            ),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_create_token(
                client,
                valid.as_ptr(),
                valid.as_ptr(),
                valid.as_ptr(),
                false,
                0,
                ptr::null_mut(),
            ),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_create_token(
                client,
                invalid_utf8.as_ptr().cast(),
                valid.as_ptr(),
                valid.as_ptr(),
                false,
                0,
                &raw mut created_token,
            ),
            LuminateStatus::InvalidUtf8
        );
        assert_eq!(
            luminate_client_create_token(
                client,
                valid.as_ptr(),
                empty.as_ptr(),
                valid.as_ptr(),
                false,
                0,
                &raw mut created_token,
            ),
            LuminateStatus::InvalidArgument
        );
        assert_eq!(
            luminate_client_list_tokens(client, ptr::null_mut()),
            LuminateStatus::NullPointer
        );
        assert_eq!(
            luminate_client_list_attestations(client, ptr::null_mut()),
            LuminateStatus::NullPointer
        );
    }
}
