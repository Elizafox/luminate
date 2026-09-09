// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for configured browser-origin validation and normalization.

use super::*;

#[test]
fn configured_origins_are_strict_and_normalized() {
    let policy = OriginPolicy::from_configured(
        &["https://Example.COM:443".to_owned()],
        AuthenticationMode::Direct,
    )
    .expect("valid HTTPS origin");
    assert_eq!(
        policy.decision(&HeaderValue::from_static("https://example.com")),
        OriginDecision::Allow("https://example.com".to_owned())
    );

    for origin in [
        "null",
        "https://example.com/path",
        "https://user@example.com",
        "https://*.example.com",
        "http://127.0.0.1:3000",
    ] {
        assert!(
            OriginPolicy::from_configured(&[origin.to_owned()], AuthenticationMode::Direct)
                .is_err(),
            "accepted {origin}"
        );
    }
    assert!(
        OriginPolicy::from_configured(
            &["http://127.0.0.1:3000".to_owned()],
            AuthenticationMode::InsecureDevelopment,
        )
        .is_ok()
    );
}
