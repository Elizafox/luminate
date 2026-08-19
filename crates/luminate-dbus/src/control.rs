// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Native D-Bus request and outcome conversion for selector-based control.

use luminate::{CollectionId, CollectionOutcome, EmissionState, Selector, UnsupportedPolicy};
use zbus::zvariant::{DeserializeDict, Type};

use crate::error::MethodError;
use crate::management::{Dictionary, dictionary, owned};
use crate::path::{canonical_id, parse_canonical_id};

#[derive(Debug, DeserializeDict, Type)]
#[zvariant(signature = "a{sv}", rename_all = "PascalCase", deny_unknown_fields)]
pub(crate) struct SelectorRequest {
    kind: String,
    target: Option<String>,
    collection: Option<String>,
    targets: Option<Vec<String>>,
}

impl SelectorRequest {
    pub(crate) fn into_selector(self) -> Result<Selector, MethodError> {
        match self.kind.as_str() {
            "target" => {
                self.require_absent("Collection", self.collection.as_ref())?;
                self.require_absent("Targets", self.targets.as_ref())?;
                Ok(Selector::Target(parse_canonical_id(
                    self.target
                        .as_deref()
                        .ok_or_else(|| invalid("target selector is missing Target"))?,
                )?))
            }
            "collection" => {
                self.require_absent("Target", self.target.as_ref())?;
                self.require_absent("Targets", self.targets.as_ref())?;
                Ok(Selector::Collection(CollectionId::new(
                    self.collection
                        .ok_or_else(|| invalid("collection selector is missing Collection"))?,
                )))
            }
            "targets" => {
                self.require_absent("Target", self.target.as_ref())?;
                self.require_absent("Collection", self.collection.as_ref())?;
                Ok(Selector::Targets(
                    self.targets
                        .ok_or_else(|| invalid("targets selector is missing Targets"))?
                        .into_iter()
                        .map(|target| parse_canonical_id(&target))
                        .collect::<Result<_, _>>()?,
                ))
            }
            kind => Err(invalid(format!("unknown selector kind {kind:?}"))),
        }
    }

    fn require_absent<T>(&self, name: &str, value: Option<&T>) -> Result<(), MethodError> {
        if value.is_some() {
            return Err(invalid(format!(
                "{} selector must not contain {name}",
                self.kind
            )));
        }
        Ok(())
    }
}

pub(crate) fn unsupported_policy(
    has_policy: bool,
    policy: &str,
) -> Result<Option<UnsupportedPolicy>, MethodError> {
    match (has_policy, policy) {
        (false, "") => Ok(None),
        (false, _) => Err(invalid(
            "OnUnsupported must be empty when HasOnUnsupported is false",
        )),
        (true, "skip") => Ok(Some(UnsupportedPolicy::Skip)),
        (true, "reject") => Ok(Some(UnsupportedPolicy::Reject)),
        (true, value) => Err(invalid(format!("unknown unsupported policy {value:?}"))),
    }
}

pub(crate) fn emission_state(value: &str) -> Result<EmissionState, MethodError> {
    match value {
        "dark" => Ok(EmissionState::Dark),
        "emitting" => Ok(EmissionState::Emitting),
        value => Err(invalid(format!("unknown emission state {value:?}"))),
    }
}

pub(crate) fn outcome(value: &CollectionOutcome) -> Result<Dictionary, MethodError> {
    Ok(dictionary([
        (
            "Applied",
            owned(value.applied.iter().map(canonical_id).collect::<Vec<_>>())?,
        ),
        (
            "Denied",
            owned(value.denied.iter().map(canonical_id).collect::<Vec<_>>())?,
        ),
    ]))
}

fn invalid(message: impl Into<String>) -> MethodError {
    MethodError::InvalidArgument(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_cover_every_shape_and_reject_mixed_fields() {
        assert!(matches!(
            SelectorRequest {
                kind: "target".into(),
                target: Some("device:lamp".into()),
                collection: None,
                targets: None,
            }
            .into_selector()
            .expect("target selector"),
            Selector::Target(_)
        ));
        assert!(matches!(
            SelectorRequest {
                kind: "collection".into(),
                target: None,
                collection: Some("room".into()),
                targets: None,
            }
            .into_selector()
            .expect("collection selector"),
            Selector::Collection(_)
        ));
        assert!(matches!(
            SelectorRequest {
                kind: "targets".into(),
                target: None,
                collection: None,
                targets: Some(vec!["device:a".into(), "device:b".into()]),
            }
            .into_selector()
            .expect("targets selector"),
            Selector::Targets(targets) if targets.len() == 2
        ));
        assert!(
            SelectorRequest {
                kind: "target".into(),
                target: Some("device:lamp".into()),
                collection: Some("room".into()),
                targets: None,
            }
            .into_selector()
            .is_err()
        );
    }

    #[test]
    fn unsupported_policy_requires_consistent_presence() {
        assert_eq!(unsupported_policy(false, "").expect("absent"), None);
        assert_eq!(
            unsupported_policy(true, "skip").expect("skip"),
            Some(UnsupportedPolicy::Skip)
        );
        assert!(unsupported_policy(false, "skip").is_err());
        assert!(unsupported_policy(true, "sometimes").is_err());
        assert_eq!(emission_state("dark").expect("dark"), EmissionState::Dark);
        assert_eq!(
            emission_state("emitting").expect("emitting"),
            EmissionState::Emitting
        );
        assert!(emission_state("sometimes").is_err());
    }
}
