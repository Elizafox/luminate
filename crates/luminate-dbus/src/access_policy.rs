// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Strict D-Bus projection of daemon access-policy documents.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::time::Duration;

use luminate::policy::{
    Binding, Operation, PolicyDocument, PolicyDocumentSource, PolicyRevision, ResourceConstraints,
    Role, Rule, RuleEffect, RuleId,
};
use luminate::{CollectionId, DeviceId};
use zbus::zvariant::OwnedValue;

use crate::error::MethodError;
use crate::management::{Dictionary, dictionary, owned};

const SCHEMA_VERSION: u32 = 2;
type Constraints = (Vec<String>, Vec<String>, bool, bool, Vec<String>);
type RuleWire = (
    String,
    String,
    Vec<String>,
    Constraints,
    bool,
    String,
    bool,
    u64,
);
type RoleWire = (String, Vec<String>, Vec<RuleWire>);
type BindingWire = (String, Vec<String>, Vec<String>, Vec<String>);

pub(crate) fn encode(document: &PolicyDocument) -> Result<Dictionary, MethodError> {
    let source = document.source();
    let roles = source
        .roles
        .iter()
        .map(|(name, role)| role_wire(name, role))
        .collect::<Vec<_>>();
    let bindings = source.bindings.iter().map(binding_wire).collect::<Vec<_>>();
    Ok(dictionary([
        ("SchemaVersion", owned(SCHEMA_VERSION)?),
        ("Revision", owned(source.revision.0)?),
        ("Roles", owned(roles)?),
        ("Bindings", owned(bindings)?),
    ]))
}

pub(crate) fn decode(mut value: Dictionary) -> Result<PolicyDocument, MethodError> {
    let schema = take::<u32>(&mut value, "SchemaVersion")?;
    if schema != SCHEMA_VERSION {
        return Err(invalid(format!(
            "unsupported policy schema version {schema}"
        )));
    }
    let revision = PolicyRevision(take(&mut value, "Revision")?);
    let roles = take::<Vec<RoleWire>>(&mut value, "Roles")?
        .into_iter()
        .map(|wire| Ok((wire.0.clone(), role(wire)?)))
        .collect::<Result<BTreeMap<_, _>, MethodError>>()?;
    let bindings = take::<Vec<BindingWire>>(&mut value, "Bindings")?
        .into_iter()
        .map(binding)
        .collect::<Vec<_>>();
    if let Some(field) = value.keys().next() {
        return Err(invalid(format!("unknown policy field {field:?}")));
    }
    PolicyDocument::new(PolicyDocumentSource {
        revision,
        roles,
        bindings,
    })
    .map_err(|error| invalid(error.to_string()))
}

fn role_wire(name: &str, value: &Role) -> RoleWire {
    (
        name.to_owned(),
        value.parents.iter().cloned().collect(),
        value.rules.iter().map(rule_wire).collect(),
    )
}

fn rule_wire(value: &Rule) -> RuleWire {
    (
        value.id.as_str().to_owned(),
        match value.effect {
            RuleEffect::Allow => "allow",
            RuleEffect::Deny => "deny",
        }
        .to_owned(),
        value
            .operations
            .iter()
            .copied()
            .map(operation_name)
            .map(str::to_owned)
            .collect(),
        constraints_wire(&value.resources),
        value.reason.is_some(),
        value.reason.clone().unwrap_or_default(),
        value.cache_hint.is_some(),
        value
            .cache_hint
            .and_then(|duration| u64::try_from(duration.as_millis()).ok())
            .unwrap_or_default(),
    )
}

fn constraints_wire(value: &ResourceConstraints) -> Constraints {
    (
        value.device_ids.iter().map(ToString::to_string).collect(),
        value.provider_instances.iter().cloned().collect(),
        value.host_attached.is_some(),
        value.host_attached.unwrap_or_default(),
        value
            .collections
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect(),
    )
}

fn binding_wire(value: &Binding) -> BindingWire {
    (
        value.authority.clone(),
        value.subjects.iter().cloned().collect(),
        value.groups.iter().cloned().collect(),
        value.roles.iter().cloned().collect(),
    )
}

fn role((_, parents, rules): RoleWire) -> Result<Role, MethodError> {
    Ok(Role {
        parents: parents.into_iter().collect(),
        rules: rules.into_iter().map(rule).collect::<Result<_, _>>()?,
    })
}

fn rule(
    (id, effect, operations, resources, has_reason, reason, has_cache, cache_ms): RuleWire,
) -> Result<Rule, MethodError> {
    Ok(Rule {
        id: RuleId::new(id).map_err(|error| invalid(error.to_string()))?,
        effect: match effect.as_str() {
            "allow" => RuleEffect::Allow,
            "deny" => RuleEffect::Deny,
            _ => return Err(invalid(format!("unknown rule effect {effect:?}"))),
        },
        operations: operations
            .into_iter()
            .map(|value| parse_operation(&value))
            .collect::<Result<BTreeSet<_>, _>>()?,
        resources: ResourceConstraints {
            device_ids: resources.0.into_iter().map(DeviceId::new).collect(),
            provider_instances: resources.1.into_iter().collect(),
            host_attached: resources.2.then_some(resources.3),
            collections: resources.4.into_iter().map(CollectionId::new).collect(),
        },
        reason: has_reason.then_some(reason),
        cache_hint: has_cache.then_some(Duration::from_millis(cache_ms)),
    })
}

fn binding(value: BindingWire) -> Binding {
    Binding {
        authority: value.0,
        subjects: value.1.into_iter().collect(),
        groups: value.2.into_iter().collect(),
        roles: value.3.into_iter().collect(),
    }
}

fn operation_name(value: Operation) -> &'static str {
    match value {
        Operation::Observe => "observe",
        Operation::Refresh => "refresh",
        Operation::Control => "control",
        Operation::HardwareAdministration => "hardware-administration",
        Operation::DaemonAdministration => "daemon-administration",
        Operation::ManagePlugins => "manage-plugins",
        Operation::CreateCollection => "create-collection",
        Operation::DestroyCollection => "destroy-collection",
        Operation::ModifyCollection => "modify-collection",
        Operation::AdministerCollections => "administer-collections",
        Operation::ManagePolicy => "manage-policy",
        Operation::ManageAuthentication => "manage-authentication",
        Operation::AdministerFrontend => "administer-frontend",
        Operation::CreateScene => "create-scene",
        Operation::ModifyScene => "modify-scene",
        Operation::DestroyScene => "destroy-scene",
        Operation::AdministerScenes => "administer-scenes",
    }
}

fn parse_operation(value: &str) -> Result<Operation, MethodError> {
    Ok(match value {
        "observe" => Operation::Observe,
        "refresh" => Operation::Refresh,
        "control" => Operation::Control,
        "hardware-administration" => Operation::HardwareAdministration,
        "daemon-administration" => Operation::DaemonAdministration,
        "manage-plugins" => Operation::ManagePlugins,
        "create-collection" => Operation::CreateCollection,
        "destroy-collection" => Operation::DestroyCollection,
        "modify-collection" => Operation::ModifyCollection,
        "administer-collections" => Operation::AdministerCollections,
        "manage-policy" => Operation::ManagePolicy,
        "manage-authentication" => Operation::ManageAuthentication,
        "administer-frontend" => Operation::AdministerFrontend,
        "create-scene" => Operation::CreateScene,
        "modify-scene" => Operation::ModifyScene,
        "destroy-scene" => Operation::DestroyScene,
        "administer-scenes" => Operation::AdministerScenes,
        _ => return Err(invalid(format!("unknown policy operation {value:?}"))),
    })
}

fn take<T>(value: &mut Dictionary, field: &str) -> Result<T, MethodError>
where
    T: TryFrom<OwnedValue>,
    T::Error: Display,
{
    value
        .remove(field)
        .ok_or_else(|| invalid(format!("policy field {field:?} is required")))?
        .try_into()
        .map_err(|error| {
            invalid(format!(
                "policy field {field:?} has the wrong type: {error}"
            ))
        })
}

fn invalid(message: String) -> MethodError {
    MethodError::InvalidArgument(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use luminate::policy::{Preset, materialize_presets};

    #[test]
    fn policy_dictionary_round_trips_and_rejects_unknown_fields() {
        let document = PolicyDocument::new(materialize_presets(
            PolicyRevision(7),
            [Preset::Administrator],
        ))
        .expect("valid policy");
        let encoded = encode(&document).expect("encode policy");
        assert_eq!(decode(encoded.clone()).expect("decode policy"), document);

        let mut malformed = encoded;
        malformed.insert("Surprise".to_owned(), owned(true).expect("owned value"));
        assert!(matches!(
            decode(malformed),
            Err(MethodError::InvalidArgument(_))
        ));
    }
}
