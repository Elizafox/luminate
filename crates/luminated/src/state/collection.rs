// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! User-created, cross-device target collections.
//!
//! A collection's membership is an explicit, client-managed DAG: it only
//! changes through [`DaemonState::create_collection`],
//! [`DaemonState::add_collection_member`], and
//! [`DaemonState::remove_collection_member`], each of which validates the
//! whole prospective graph before committing anything. That makes every
//! collection reachable through the registry cycle-free and reference-valid
//! by construction, so [`DaemonState::resolve_collection_leaves`] can expand
//! nested collections with a bounded traversal rather than a lazily-checked
//! one.
//!
//! Ownership (who may destroy or structurally modify a collection) is
//! deliberately not enforced here: `DaemonState` methods are unauthenticated
//! data operations, mirroring every other mutation in this module. The
//! dispatch layer is responsible for checking a caller's principal against
//! [`Collection::owner`] (or an elevated override permission) before
//! calling into these methods, exactly as it already resolves authorization
//! before every other state mutation.

use std::collections::{HashMap, HashSet};

use luminate_core::collection::{
    Collection, CollectionCategory, CollectionGraph, CollectionGraphError, CollectionId,
    CollectionMember, OwnerIdentity,
};
use luminate_core::device::DeviceId;
use luminate_core::target::TargetId;
use luminate_protocol::UnsupportedPolicy;

use crate::error::DaemonError;

use super::DaemonState;
use super::target_state::TargetState;

impl DaemonState {
    /// Every collection currently registered, for listing and snapshotting.
    #[must_use]
    pub fn collections(&self) -> &HashMap<CollectionId, Collection> {
        &self.collections
    }

    /// Restores persisted collections, replacing whatever is registered.
    pub fn restore_collections(&mut self, collections: HashMap<CollectionId, Collection>) {
        self.collections = collections;
    }
}

impl DaemonState {
    /// Creates a new collection with a fresh, server-generated id.
    ///
    /// A newly generated id cannot already appear in `members` (the
    /// caller cannot reference an id it hasn't been told yet), so
    /// self-reference is impossible here; it can only arise later through
    /// [`Self::add_collection_member`]. Duplicate entries in `members` are
    /// silently collapsed to one.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::CollectionNotFound`] if `members` references a
    /// nested collection id that doesn't exist.
    pub fn create_collection(
        &mut self,
        name: String,
        description: Option<String>,
        owner: OwnerIdentity,
        kind: Option<CollectionCategory>,
        members: Vec<CollectionMember>,
    ) -> Result<CollectionId, DaemonError> {
        for member in &members {
            if let CollectionMember::Collection(nested) = member
                && !self.collections.contains_key(nested)
            {
                return Err(DaemonError::CollectionNotFound(nested.clone()));
            }
        }

        let mut deduplicated = Vec::with_capacity(members.len());
        for member in members {
            if !deduplicated.contains(&member) {
                deduplicated.push(member);
            }
        }

        let id = CollectionId::generate();
        self.collections.insert(
            id.clone(),
            Collection {
                id: id.clone(),
                name,
                description,
                owner,
                kind,
                members: deduplicated,
            },
        );
        if let Err(error) = self.ensure_persistable() {
            self.collections.remove(&id);
            return Err(error);
        }
        Ok(id)
    }

    /// Destroys `id`, refusing while any other collection still references
    /// it (a "restrict" delete, matching how this state already tolerates
    /// but never silently rewrites other structures on a caller's behalf;
    /// see the module doc). The caller must remove the reference first.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::CollectionNotFound`] if `id` is unknown, or
    /// [`DaemonError::CollectionInUse`] if another collection still lists it
    /// as a member.
    pub fn destroy_collection(&mut self, id: &CollectionId) -> Result<Collection, DaemonError> {
        if !self.collections.contains_key(id) {
            return Err(DaemonError::CollectionNotFound(id.clone()));
        }

        let referenced_by = self.collections_referencing(id);
        if !referenced_by.is_empty() {
            return Err(DaemonError::CollectionInUse {
                id: id.clone(),
                referenced_by,
            });
        }

        if let Some(scene) = self.scenes.values().find(|scene| {
            scene
                .bindings
                .iter()
                .any(|binding| binding.dynamic_collection() == Some(id))
        }) {
            return Err(DaemonError::CollectionReferencedByScene {
                collection: id.clone(),
                scene: scene.id.clone(),
            });
        }

        match self.collections.remove(id) {
            Some(collection) => Ok(collection),
            None => Err(DaemonError::CollectionNotFound(id.clone())),
        }
    }

    /// Adds `member` to `id`'s membership. A no-op if `member` is already
    /// present.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::CollectionNotFound`] if `id` (or, for a nested
    /// collection member, that nested id) doesn't exist,
    /// [`DaemonError::CollectionSelfReference`] if `member` is `id` itself,
    /// or [`DaemonError::CollectionCycle`] if `member` is a collection that
    /// already (transitively) contains `id`.
    pub fn add_collection_member(
        &mut self,
        id: &CollectionId,
        member: CollectionMember,
    ) -> Result<(), DaemonError> {
        if !self.collections.contains_key(id) {
            return Err(DaemonError::CollectionNotFound(id.clone()));
        }

        if let CollectionMember::Collection(nested) = &member {
            if nested == id {
                return Err(DaemonError::CollectionSelfReference(id.clone()));
            }
            if !self.collections.contains_key(nested) {
                return Err(DaemonError::CollectionNotFound(nested.clone()));
            }
            let mut visited = HashSet::new();
            if self.collection_reaches(nested, id, &mut visited) {
                return Err(DaemonError::CollectionCycle(nested.clone()));
            }
        }

        let Some(collection) = self.collections.get_mut(id) else {
            return Err(DaemonError::CollectionNotFound(id.clone()));
        };
        let previous_members = collection.members.clone();
        if !collection.members.contains(&member) {
            collection.members.push(member);
        }
        if let Err(error) = self.ensure_persistable() {
            if let Some(collection) = self.collections.get_mut(id) {
                collection.members = previous_members;
            }
            return Err(error);
        }
        Ok(())
    }

    /// Removes `member` from `id`'s membership. A no-op if `member` is
    /// absent.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::CollectionNotFound`] if `id` doesn't exist.
    pub fn remove_collection_member(
        &mut self,
        id: &CollectionId,
        member: &CollectionMember,
    ) -> Result<(), DaemonError> {
        let collection = self
            .collections
            .get_mut(id)
            .ok_or_else(|| DaemonError::CollectionNotFound(id.clone()))?;
        collection.members.retain(|existing| existing != member);
        Ok(())
    }

    /// Looks up one collection by id.
    #[must_use]
    pub fn collection(&self, id: &CollectionId) -> Option<&Collection> {
        self.collections.get(id)
    }

    /// Every collection whose transitive membership currently reaches
    /// `device_id`, for ACL rule matching
    /// (`daemon::authz::resolve_resources`). A device can belong to any
    /// number of collections at once (nested membership, several rooms'
    /// worth of aggregation), unlike a location, which is exclusive.
    #[must_use]
    pub(crate) fn collections_containing_device(&self, device_id: &DeviceId) -> Vec<CollectionId> {
        CollectionGraph::new(self.collections.values())
            .collections_containing(|target| target.device_id() == device_id)
    }

    /// Expands `id` to the flat, deduplicated set of concrete leaf targets
    /// it (transitively) covers, resolving nested collections along the way.
    /// Safe against unbounded recursion because every collection reachable
    /// through the registry is already validated cycle-free at
    /// construction time (see the module doc).
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::CollectionNotFound`] if `id`, or a nested
    /// collection it references, is not registered.
    pub fn resolve_collection_leaves(
        &self,
        id: &CollectionId,
    ) -> Result<Vec<TargetId>, DaemonError> {
        CollectionGraph::new(self.collections.values())
            .resolve_leaves(id)
            .map_err(|error| match error {
                CollectionGraphError::CollectionNotFound(id) => DaemonError::CollectionNotFound(id),
                CollectionGraphError::DuplicateCollection(id) => DaemonError::Internal(format!(
                    "duplicate collection identifier: {}",
                    id.as_str()
                )),
                CollectionGraphError::Cycle(id) => DaemonError::CollectionCycle(id),
            })
    }

    /// Applies reject/skip capability filtering to an already-authorized set
    /// of collection leaves, without touching hardware.
    ///
    /// `members` is normally the ACL-authorized subset of a collection's
    /// leaves (see `daemon::authz::authorize_partial`), not the collection's
    /// full membership. Authorization and capability filtering are separate
    /// stages applied in sequence rather than a single resolution step.
    ///
    /// This separation allows the daemon to release its global state lock
    /// before isolated plugin hosts perform the potentially slow hardware
    /// work.
    ///
    /// # Errors
    ///
    /// Under [`UnsupportedPolicy::Reject`], returns the validation error from
    /// the first unsupported member.
    pub(crate) fn prepare_collection_mutation(
        &self,
        members: &[TargetId],
        state: &TargetState,
        policy: UnsupportedPolicy,
    ) -> Result<Vec<TargetId>, DaemonError> {
        if policy == UnsupportedPolicy::Reject {
            for member in members {
                self.validate_target_state(member, state)?;
            }
            return Ok(members.to_vec());
        }
        let supported = members
            .iter()
            .filter(|member| self.validate_target_state(member, state).is_ok())
            .cloned()
            .collect::<Vec<_>>();
        if supported.is_empty()
            && matches!(state, TargetState::Effect(_))
            && let Some(member) = members.first()
        {
            self.validate_target_state(member, state)?;
        }
        Ok(supported)
    }

    /// Records device-scope intended state for the successful prefix of a
    /// collection fan-out that could not be completed.
    pub(crate) fn record_collection_partial_success(
        &mut self,
        members: &[TargetId],
        state: &TargetState,
    ) -> Result<(), DaemonError> {
        let previous = self.target_states().to_vec();
        for member in members {
            self.insert_target_state(member.clone(), state.clone());
        }
        if let Err(error) = self.ensure_persistable() {
            self.target_state = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Whether `from` (transitively, through nested collection members)
    /// reaches `target`. Used to reject an `add_collection_member` that
    /// would close a cycle before it is ever committed.
    fn collection_reaches(
        &self,
        from: &CollectionId,
        target: &CollectionId,
        visited: &mut HashSet<CollectionId>,
    ) -> bool {
        if from == target {
            return true;
        }
        if !visited.insert(from.clone()) {
            return false;
        }
        let Some(collection) = self.collections.get(from) else {
            return false;
        };
        collection.members.iter().any(|member| match member {
            CollectionMember::Collection(nested) => {
                self.collection_reaches(nested, target, visited)
            }
            CollectionMember::Target(_) => false,
        })
    }

    /// Every collection that directly lists `id` as a member, for the
    /// restrict-delete check in [`Self::destroy_collection`].
    fn collections_referencing(&self, id: &CollectionId) -> Vec<CollectionId> {
        self.collections
            .values()
            .filter(|collection| &collection.id != id)
            .filter(|collection| {
                collection.members.iter().any(
                    |member| matches!(member, CollectionMember::Collection(nested) if nested == id),
                )
            })
            .map(|collection| collection.id.clone())
            .collect()
    }
}

#[cfg(test)]
#[path = "collection_tests.rs"]
mod tests;
