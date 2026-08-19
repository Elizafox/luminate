// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Native D-Bus representation of persistent cross-device collections.

use luminate::CollectionId;
use luminate::collection::{Collection, CollectionCategory, CollectionMember, OwnerIdentity};
use zbus::zvariant::{DeserializeDict, Type};

use crate::error::MethodError;
use crate::management::{Dictionary, dictionary, owned};
use crate::path::canonical_id;
use crate::path::parse_canonical_id;

pub(crate) type CreateParts = (
    String,
    Option<String>,
    Option<CollectionCategory>,
    Vec<CollectionMember>,
);

#[derive(Debug, DeserializeDict, Type)]
#[zvariant(signature = "a{sv}", rename_all = "PascalCase", deny_unknown_fields)]
pub(crate) struct CreateRequest {
    name: String,
    description: Option<String>,
    category: Option<String>,
    members: Vec<MemberRequest>,
}

impl CreateRequest {
    pub(crate) fn into_parts(self) -> Result<CreateParts, MethodError> {
        Ok((
            self.name,
            self.description,
            self.category.map(CollectionCategory::new),
            self.members
                .into_iter()
                .map(MemberRequest::into_member)
                .collect::<Result<_, _>>()?,
        ))
    }
}

#[derive(Debug, DeserializeDict, Type)]
#[zvariant(signature = "a{sv}", rename_all = "PascalCase", deny_unknown_fields)]
pub(crate) struct MemberRequest {
    kind: String,
    target: Option<String>,
    collection: Option<String>,
}

impl MemberRequest {
    pub(crate) fn into_member(self) -> Result<CollectionMember, MethodError> {
        match self.kind.as_str() {
            "target" => {
                if self.collection.is_some() {
                    return Err(invalid(
                        "target collection member must not contain Collection",
                    ));
                }
                let target = self
                    .target
                    .ok_or_else(|| invalid("target collection member is missing Target"))?;
                Ok(CollectionMember::Target(parse_canonical_id(&target)?))
            }
            "collection" => {
                if self.target.is_some() {
                    return Err(invalid("nested collection member must not contain Target"));
                }
                let collection = self
                    .collection
                    .ok_or_else(|| invalid("nested collection member is missing Collection"))?;
                Ok(CollectionMember::Collection(CollectionId::new(collection)))
            }
            kind => Err(invalid(format!("unknown collection member kind {kind:?}"))),
        }
    }
}

pub(crate) fn record(value: Collection) -> Result<Dictionary, MethodError> {
    let mut result = dictionary([
        ("Id", owned(value.id.as_str().to_owned())?),
        ("Name", owned(value.name)?),
        ("HasDescription", owned(value.description.is_some())?),
        ("Owner", owned(owner(value.owner)?)?),
        ("HasCategory", owned(value.kind.is_some())?),
        (
            "Members",
            owned(
                value
                    .members
                    .into_iter()
                    .map(member)
                    .collect::<Result<Vec<_>, _>>()?,
            )?,
        ),
    ]);
    if let Some(description) = value.description {
        result.insert("Description".into(), owned(description)?);
    }
    if let Some(category) = value.kind {
        result.insert("Category".into(), owned(category.as_str().to_owned())?);
    }
    Ok(result)
}

fn owner(value: OwnerIdentity) -> Result<Dictionary, MethodError> {
    match value {
        OwnerIdentity::Uid(uid) => Ok(dictionary([("Kind", owned("uid")?), ("Uid", owned(uid)?)])),
        OwnerIdentity::Sid(sid) => Ok(dictionary([("Kind", owned("sid")?), ("Sid", owned(sid)?)])),
        OwnerIdentity::Principal(principal) => Ok(dictionary([
            ("Kind", owned("principal")?),
            ("Authority", owned(principal.authority().to_owned())?),
            ("Subject", owned(principal.subject().to_owned())?),
        ])),
    }
}

fn member(value: CollectionMember) -> Result<Dictionary, MethodError> {
    match value {
        CollectionMember::Target(target) => Ok(dictionary([
            ("Kind", owned("target")?),
            ("Target", owned(canonical_id(&target))?),
        ])),
        CollectionMember::Collection(collection) => Ok(dictionary([
            ("Kind", owned("collection")?),
            ("Collection", owned(collection.as_str().to_owned())?),
        ])),
    }
}

fn invalid(message: impl Into<String>) -> MethodError {
    MethodError::InvalidArgument(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use luminate::policy::PrincipalId;
    use luminate::{CollectionCategory, CollectionId, TargetId};

    #[test]
    fn collection_record_preserves_owner_optional_fields_and_member_order() {
        let encoded = record(Collection {
            id: CollectionId::new("desk"),
            name: "Desk".into(),
            description: Some("Work lights".into()),
            owner: OwnerIdentity::Uid(1000),
            kind: Some(CollectionCategory::new("location")),
            members: vec![
                CollectionMember::Target(TargetId::device("keyboard")),
                CollectionMember::Collection(CollectionId::new("monitor-lights")),
            ],
        })
        .expect("encode collection");

        let members =
            Vec::<Dictionary>::try_from(encoded["Members"].try_clone().expect("clone members"))
                .expect("members should be dictionaries");
        assert_eq!(members.len(), 2);
        assert!(members[0].contains_key("Target"));
        assert!(members[1].contains_key("Collection"));
        assert!(
            bool::try_from(
                encoded["HasDescription"]
                    .try_clone()
                    .expect("clone presence")
            )
            .expect("description presence should be boolean")
        );
    }

    #[test]
    fn owner_conversion_preserves_every_identity_variant() {
        let uid = owner(OwnerIdentity::Uid(1000)).expect("encode uid");
        let sid = owner(OwnerIdentity::Sid("S-1-5-21".into())).expect("encode sid");
        let principal = owner(OwnerIdentity::Principal(
            PrincipalId::new("local", "operator").expect("valid principal"),
        ))
        .expect("encode principal");

        assert!(uid.contains_key("Uid"));
        assert!(sid.contains_key("Sid"));
        assert!(principal.contains_key("Authority"));
        assert!(principal.contains_key("Subject"));
    }

    #[test]
    fn member_requests_require_the_identifier_matching_their_kind() {
        assert_eq!(
            MemberRequest {
                kind: "target".into(),
                target: Some("device:keyboard".into()),
                collection: None,
            }
            .into_member()
            .expect("valid target member"),
            CollectionMember::Target(TargetId::device("keyboard"))
        );
        assert_eq!(
            MemberRequest {
                kind: "collection".into(),
                target: None,
                collection: Some("desk".into()),
            }
            .into_member()
            .expect("valid nested collection"),
            CollectionMember::Collection(CollectionId::new("desk"))
        );

        for request in [
            MemberRequest {
                kind: "target".into(),
                target: None,
                collection: None,
            },
            MemberRequest {
                kind: "collection".into(),
                target: Some("device:keyboard".into()),
                collection: Some("desk".into()),
            },
            MemberRequest {
                kind: "selector".into(),
                target: None,
                collection: None,
            },
        ] {
            assert!(request.into_member().is_err());
        }
    }
}
