// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use super::{reconciliation_policy_from_abi, reconciliation_policy_to_abi};

#[test]
fn reconciliation_recommendation_abi_is_total_and_rejects_unknown_values() {
    use luminate_core::control::ReconciliationPolicy;

    for policy in [
        None,
        Some(ReconciliationPolicy::Restore),
        Some(ReconciliationPolicy::Adopt),
        Some(ReconciliationPolicy::Leave),
    ] {
        assert_eq!(
            reconciliation_policy_from_abi(reconciliation_policy_to_abi(policy)),
            Some(policy)
        );
    }
    assert_eq!(reconciliation_policy_from_abi(u8::MAX), None);
}
