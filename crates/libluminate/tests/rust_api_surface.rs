// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use luminate::{
        CctEmulation, ColourChannelValue, ColourError, EffectArguments, EffectDirection,
        HardwareEffectId, OwnerIdentity, ReconciliationPolicy,
    };

    #[test]
    fn curated_types_are_available_from_the_crate_root() {
        let _ = size_of::<(
            ColourChannelValue,
            ColourError,
            CctEmulation,
            ReconciliationPolicy,
            OwnerIdentity,
            EffectArguments,
            EffectDirection,
            HardwareEffectId,
        )>();
    }

    #[allow(
        unused_imports,
        reason = "the wildcard import itself is the public API compile check"
    )]
    use luminate::prelude::*;

    #[test]
    fn ordinary_client_types_are_available() {
        let target = TargetId::device("lamp");
        let selector = Selector::Target(target);
        let colour = Colour::rgb(Rgb::new(1, 2, 3));
        let effect = Effect::Static { colour };
        let outcome = CollectionOutcome::default();

        assert!(matches!(selector, Selector::Target(_)));
        assert!(matches!(effect, Effect::Static { .. }));
        assert!(outcome.applied.is_empty());
    }

    #[test]
    fn access_administration_handles_are_available_in_ordinary_rust_usage() {
        fn accepts_policy(_: luminate::PolicyAdministration<'_>) {}
        fn accepts_authentication(_: luminate::AuthenticationAdministration<'_>) {}

        let policy: fn(&Client) -> luminate::PolicyAdministration<'_> =
            Client::policy_administration;
        let authentication: fn(&Client) -> luminate::AuthenticationAdministration<'_> =
            Client::authentication_administration;
        let _ = (
            accepts_policy,
            accepts_authentication,
            policy,
            authentication,
        );
    }
}
