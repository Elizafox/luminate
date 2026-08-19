// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! User-created, cross-device target aggregates.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use thiserror::Error;
use uuid::Uuid;

use crate::policy::PrincipalId;
use crate::target::TargetId;
use crate::util::declare_opaque_id;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// A named, user-created aggregate of targets, addressable as a target in
/// its own right. Unlike [`crate::group::Group`], which is confined to one
/// device, a collection can span devices, surfaces, elements, groups, and
/// other collections.
///
/// Membership is explicit and static: a collection only gains or loses
/// members through an explicit mutation, never through attribute matching.
/// That makes it possible to validate the whole membership graph as
/// cycle-free at every structural change, rather than re-checking lazily at
/// dispatch time.
pub struct Collection {
    /// Stable, server-generated identifier.
    pub id: CollectionId,

    /// Human-readable name.
    pub name: String,

    /// Optional human-readable description.
    pub description: Option<String>,

    /// The identity of the principal that created this collection. Only the
    /// owner (or a policy's elevated override) may destroy or structurally
    /// modify it.
    #[serde(alias = "owner_uid")]
    pub owner: OwnerIdentity,

    /// A presentation hint for how a client should group or iconify this
    /// collection (e.g. a physical room vs. an arbitrary logical grouping),
    /// not a functional capability. `None` means no hint is available; a
    /// client should fall back to generic presentation rather than error.
    /// See [`collection_category`] for well-known values a client can
    /// special-case.
    pub kind: Option<CollectionCategory>,

    /// Explicit member targets and nested collections.
    pub members: Vec<CollectionMember>,
}

/// The identity under which collection ownership is enforced.
///
/// The untagged representation preserves the legacy bare `owner_uid` integer
/// while also supporting Windows SIDs and daemon-authenticated principals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OwnerIdentity {
    /// A Unix uid, as captured from `SO_PEERCRED`.
    Uid(u32),

    /// A Windows security identifier, in its canonical string form.
    Sid(String),

    /// A daemon-authenticated principal identity.
    Principal(PrincipalId),
}

impl Collection {
    /// Returns whether `members` directly references the collection identified
    /// by `id`.
    ///
    /// Indirect cycles require the complete collection graph.
    #[must_use]
    #[inline]
    pub fn references_self(id: &CollectionId, members: &[CollectionMember]) -> bool {
        members.iter().any(
            |member| matches!(member, CollectionMember::Collection(member_id) if member_id == id),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// One member of a [`Collection`]: either a concrete target or another
/// collection, nested by reference. Mirrors [`crate::group::GroupMember`],
/// which plays the same role one scope down (within a single device).
pub enum CollectionMember {
    /// A concrete device, surface, element, or group target.
    Target(TargetId),

    /// Another collection, nested by reference.
    Collection(CollectionId),
}

/// An open, client-extensible collection classification string.
///
/// It is a presentation hint, not a functional capability. See
/// [`collection_category`] for well-known values.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CollectionCategory(String);

declare_opaque_id!(
    CollectionCategory,
    "Creates a presentation category from an open string identifier."
);

/// Well-known [`CollectionCategory`] string values a GUI can match against
/// for a built-in icon/presentation. This list is not exhaustive; a client
/// may use other values, which a GUI should render with a generic fallback.
pub mod collection_category {
    /// A physical place, such as a room.
    pub const LOCATION: &str = "location";

    /// An arbitrary logical grouping with no physical-space meaning (e.g.
    /// "streaming setup" spanning several rooms).
    pub const LOGICAL_GROUPING: &str = "logical-grouping";

    /// A named zone smaller or looser than a whole room (e.g. "desk", "one
    /// wall of lights").
    pub const ZONE: &str = "zone";
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
/// Opaque, server-generated stable identifier for a collection.
pub struct CollectionId(String);

declare_opaque_id!(
    CollectionId,
    "Wraps an existing identifier value, e.g. one round-tripped through \
     serialization. Prefer [`Self::generate`] when minting a fresh \
     collection identifier."
);

impl CollectionId {
    /// Mints a fresh, randomly generated identifier.
    #[must_use]
    #[inline]
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

/// A borrowed, reusable view of a collection membership graph.
///
/// The graph preserves each collection's member order. Resolution
/// deduplicates both concrete leaves and collections reached through diamond
/// paths. Unlike a daemon-owned registry, a graph supplied by another
/// transport or store is not assumed to be valid: every traversal detects
/// missing references and cycles.
pub struct CollectionGraph<'a> {
    collections: HashMap<&'a CollectionId, &'a Collection>,
    duplicates: HashSet<&'a CollectionId>,
}

impl<'a> CollectionGraph<'a> {
    /// Builds a graph over `collections`.
    ///
    /// Duplicate identifiers are retained as a graph error so their
    /// resolution cannot depend on input iteration order.
    pub fn new(collections: impl IntoIterator<Item = &'a Collection>) -> Self {
        let mut indexed = HashMap::new();
        let mut duplicates = HashSet::new();
        for collection in collections {
            if indexed.insert(&collection.id, collection).is_some() {
                duplicates.insert(&collection.id);
            }
        }
        Self {
            collections: indexed,
            duplicates,
        }
    }

    /// Expands `id` to its flat, deduplicated concrete leaf targets.
    ///
    /// Leaves retain their first depth-first occurrence in the collection
    /// membership graph.
    ///
    /// # Errors
    ///
    /// Returns [`CollectionGraphError::CollectionNotFound`] when `id` or a
    /// nested collection is absent, and [`CollectionGraphError::Cycle`] when
    /// reachable membership contains a cycle.
    pub fn resolve_leaves(&self, id: &CollectionId) -> Result<Vec<TargetId>, CollectionGraphError> {
        let mut leaves = Vec::new();
        let mut expanded = HashSet::new();
        let mut visiting = HashSet::new();
        self.collect_leaves(id, &mut visiting, &mut expanded, &mut leaves)?;
        Ok(leaves)
    }

    /// Returns every collection whose transitive membership contains a
    /// target accepted by `matches`.
    ///
    /// Results are sorted by collection identifier for deterministic output.
    /// Invalid collections fail closed and are omitted.
    #[must_use]
    pub fn collections_containing(
        &self,
        mut matches: impl FnMut(&TargetId) -> bool,
    ) -> Vec<CollectionId> {
        let mut ids: Vec<_> = self.collections.keys().copied().collect();
        ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        ids.into_iter()
            .filter(|id| {
                self.resolve_leaves(id)
                    .is_ok_and(|leaves| leaves.iter().any(&mut matches))
            })
            .cloned()
            .collect()
    }

    /// Projects every collection through a concrete-target visibility rule.
    ///
    /// A projected collection contains only targets accepted by
    /// `target_visible`. A nested collection link is retained only when that
    /// nested collection is itself visible after projection. Empty
    /// collections are omitted unless `empty_visible` explicitly accepts
    /// them. Results are sorted by collection identifier.
    ///
    /// # Errors
    ///
    /// Returns [`CollectionGraphError::CollectionNotFound`] for a missing
    /// nested reference, and [`CollectionGraphError::Cycle`] for any cycle.
    pub fn project(
        &self,
        mut target_visible: impl FnMut(&TargetId) -> bool,
        mut empty_visible: impl FnMut(&CollectionId) -> bool,
    ) -> Result<Vec<Collection>, CollectionGraphError> {
        let mut ids: Vec<_> = self.collections.keys().copied().collect();
        ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));

        let mut projected = Vec::new();
        let mut cache = HashMap::new();
        let mut visiting = HashSet::new();
        for id in ids {
            if let Some(collection) = self.project_one(
                id,
                &mut target_visible,
                &mut empty_visible,
                &mut visiting,
                &mut cache,
            )? {
                projected.push(collection);
            }
        }
        Ok(projected)
    }

    fn collect_leaves(
        &self,
        id: &CollectionId,
        visiting: &mut HashSet<CollectionId>,
        expanded: &mut HashSet<CollectionId>,
        leaves: &mut Vec<TargetId>,
    ) -> Result<(), CollectionGraphError> {
        if expanded.contains(id) {
            return Ok(());
        }
        if self.duplicates.contains(id) {
            return Err(CollectionGraphError::DuplicateCollection(id.clone()));
        }
        if !visiting.insert(id.clone()) {
            return Err(CollectionGraphError::Cycle(id.clone()));
        }

        let collection = self
            .collections
            .get(id)
            .ok_or_else(|| CollectionGraphError::CollectionNotFound(id.clone()))?;
        for member in &collection.members {
            match member {
                CollectionMember::Target(target) => {
                    if !leaves.contains(target) {
                        leaves.push(target.clone());
                    }
                }
                CollectionMember::Collection(nested) => {
                    self.collect_leaves(nested, visiting, expanded, leaves)?;
                }
            }
        }

        visiting.remove(id);
        expanded.insert(id.clone());
        Ok(())
    }

    fn project_one(
        &self,
        id: &CollectionId,
        target_visible: &mut impl FnMut(&TargetId) -> bool,
        empty_visible: &mut impl FnMut(&CollectionId) -> bool,
        visiting: &mut HashSet<CollectionId>,
        cache: &mut HashMap<CollectionId, Option<Collection>>,
    ) -> Result<Option<Collection>, CollectionGraphError> {
        if let Some(projected) = cache.get(id) {
            return Ok(projected.clone());
        }
        if self.duplicates.contains(id) {
            return Err(CollectionGraphError::DuplicateCollection(id.clone()));
        }
        if !visiting.insert(id.clone()) {
            return Err(CollectionGraphError::Cycle(id.clone()));
        }

        let collection = self
            .collections
            .get(id)
            .ok_or_else(|| CollectionGraphError::CollectionNotFound(id.clone()))?;
        let mut members = Vec::new();
        for member in &collection.members {
            match member {
                CollectionMember::Target(target) if target_visible(target) => {
                    members.push(member.clone());
                }
                CollectionMember::Target(_) => {}
                CollectionMember::Collection(nested) => {
                    if self
                        .project_one(nested, target_visible, empty_visible, visiting, cache)?
                        .is_some()
                    {
                        members.push(member.clone());
                    }
                }
            }
        }

        visiting.remove(id);
        if members.is_empty() && !empty_visible(id) {
            cache.insert(id.clone(), None);
            return Ok(None);
        }

        let mut collection = (*collection).clone();
        collection.members = members;
        cache.insert(id.clone(), Some(collection.clone()));
        Ok(Some(collection))
    }
}

/// A malformed collection graph encountered during resolution or projection.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CollectionGraphError {
    /// A requested or nested collection identifier is absent.
    #[error("collection not found: {}", .0.as_str())]
    CollectionNotFound(CollectionId),

    /// More than one collection uses the same identifier.
    #[error("duplicate collection identifier: {}", .0.as_str())]
    DuplicateCollection(CollectionId),

    /// Membership reaches a collection already active in the traversal.
    #[error("collection membership cycle includes: {}", .0.as_str())]
    Cycle(CollectionId),
}

#[cfg(test)]
#[path = "collection_tests.rs"]
mod tests;
