// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Versioned, crash-safe persistence for daemon target state, collections, and scenes.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{ErrorKind, Read as _};
use std::mem;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use luminate_core::appearance_slot::{AppearanceSlotId, AppearanceSlotValue};
use luminate_core::collection::{Collection, CollectionId, CollectionMember, OwnerIdentity};
use luminate_core::effect::Effect;
use luminate_core::scene::{Scene, SceneBinding, SceneId};
use luminate_core::state::{AdoptedFacet, AppearanceSlotsState, AppearanceState, FacetValue};
use luminate_core::target::TargetId;
use luminate_platform::durability::sync_directory;
use luminate_platform::secure_random::fill_bytes;
use luminate_platform::secure_storage::{ensure_service_directory, open_private_file_for_read};
use serde::{Deserialize, Serialize};

use crate::atomic_file;
use crate::state::target_state::{TargetState, TargetStateEntry};

/// Version stamped into every state file this daemon writes. Each field added
/// to [`PersistedState`] is `#[serde(default)]`, so an older file (missing the
/// newer fields) still loads and an older daemon ignores fields it doesn't know.
/// The version drives semantics (e.g. entry ordering), not accept/reject.
///
/// History:
/// - **1**: legacy layout, target entries only, written in no guaranteed order.
///
/// - **2**: target entries written in stable dependency order (see
///   [`ORDERED_SINCE_VERSION`] / `preserve_order`).
///
/// - **3**: adds per-device `location_overrides`.
///
/// - **4**: adds per-location standing `location_defaults`.
///
/// - **5**: adds the durable adopted physical baseline.
///
/// - **6**: adds user-created `collections`, written alongside
///   `location_overrides`/`location_defaults`.
///
/// - **7**: removes `location_overrides`/`location_defaults`; collections
///   fully replace location-based fan-out. A v6-or-earlier file's location
///   fields are silently dropped on load (pre-release version, so acceptable).
///
/// - **8**: `Collection.owner_uid: u32` becomes `Collection.owner:
///   OwnerIdentity` (`Uid(u32)` | `Sid(String)`, serialized untagged), to
///   support Windows named-pipe principals with no uid equivalent. Fully
///   backward-compatible on read: `#[serde(alias = "owner_uid")]` plus
///   untagged's transparent newtype encoding means a pre-v8 file's bare
///   `owner_uid` integer deserializes directly into `OwnerIdentity::Uid`;
///   no pre-v8 daemon could have produced any other kind of owner, so no
///   data is lost or guessed at.
///
/// - **9**: migrates the persisted colour representation.
///
/// - **10**: adds persistent daemon-managed scenes.
///
/// - **11**: ownership must be represented by a daemon principal. Existing
///   UID/SID-owned collections and scenes are refused rather than guessed at;
///   [`reset_owned_objects`] is the explicit recovery path.
///
/// - **12**: target state and scenes may contain appearance-slot values. On
///   activation of the replacement Alienware topology, legacy AC and battery
///   surface appearances are migrated into slots on `power-button`.
///
/// - **13**: every saved and loaded snapshot passes the same structural and
///   resource-limit validation.
const PERSISTENCE_VERSION: u32 = 13;

/// First persistence version that writes target entries in a stable, dependency
/// order (see `preserve_order`). Kept separate from [`PERSISTENCE_VERSION`] so
/// that bumping the format for unrelated additions (e.g. location overrides in
/// v3, standing defaults in v4) does not retroactively mark ordered v2 files as
/// unordered.
const ORDERED_SINCE_VERSION: u32 = 2;

/// Upper bound on the persisted state file we read into memory before
/// deserializing. A hostile or corrupt oversized file is treated as empty
/// state rather than being allowed to exhaust memory/CPU at startup, mirroring
/// the socket frame limit that this out-of-band file would otherwise bypass.
const MAX_STATE_FILE_BYTES: usize = 4 * 1024 * 1024;

/// Upper bound on the number of target entries accepted from a state file,
/// independent of byte size, as a second guard on unbounded collection growth.
const MAX_STATE_ENTRIES: usize = 100_000;

/// Upper bound on independently addressable scenes in one snapshot.
const MAX_SCENES: usize = 10_000;

/// Upper bound on all bindings across all scenes in one snapshot.
const MAX_SCENE_BINDINGS: usize = 100_000;

/// Upper bound on all appearance-slot values across desired state and scenes.
const MAX_APPEARANCE_SLOT_VALUES: usize = 100_000;

/// Upper bound on user-created collections in one snapshot.
const MAX_COLLECTIONS: usize = 10_000;

/// Upper bound on all direct collection memberships in one snapshot.
const MAX_COLLECTION_MEMBERS: usize = 100_000;

/// Upper bound on confirmed adopted facets in one snapshot.
const MAX_ADOPTED_FACETS: usize = 100_000;

/// Upper bound on one user-authored name or description.
pub(crate) const MAX_TEXT_BYTES: usize = 64 * 1024;

/// Upper bound on one serialized collection or scene.
const MAX_OBJECT_BYTES: usize = 256 * 1024;

/// On-disk shape for persisted target state.
///
/// `TargetId` is an enum, so it cannot serialize as a JSON object key; state is
/// stored as an explicit list of (target, state) entries instead.
#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedState {
    #[serde(default = "legacy_persistence_version")]
    version: u32,

    targets: Vec<TargetStateEntry>,

    /// User-created collections, keyed by id. Added in persistence version 6;
    /// `default` keeps older state files loadable.
    #[serde(default)]
    collections: HashMap<CollectionId, Collection>,

    /// Persistent sparse intended-state snapshots. Added in version 10.
    #[serde(default)]
    scenes: HashMap<SceneId, Scene>,

    /// Kept below desired overlays and persisted separately so adoption never
    /// changes individual-override authority.
    #[serde(default)]
    adopted_baseline: Vec<AdoptedFacet>,
}

/// A persistence document proven to be inside the complete snapshot domain.
///
/// Construction is the only route to serialization or restoration, keeping
/// save and load closed over exactly the same invariants and limits.
struct ValidatedPersistedSnapshot {
    state: PersistedState,
    payload: Vec<u8>,
}

impl ValidatedPersistedSnapshot {
    #[allow(
        clippy::too_many_lines,
        reason = "The constructor deliberately keeps the complete snapshot-domain checklist together."
    )]
    fn new(state: PersistedState) -> Result<Self> {
        validate_count("target entries", state.targets.len(), MAX_STATE_ENTRIES)?;
        validate_count("collections", state.collections.len(), MAX_COLLECTIONS)?;
        validate_count("scenes", state.scenes.len(), MAX_SCENES)?;
        validate_count(
            "adopted facets",
            state.adopted_baseline.len(),
            MAX_ADOPTED_FACETS,
        )?;

        let mut collection_members = 0_usize;
        let mut collection_ids = state.collections.keys().collect::<Vec<_>>();
        collection_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        for id in collection_ids {
            let collection = state
                .collections
                .get(id)
                .context("persisted snapshot collection index changed during validation")?;
            anyhow::ensure!(
                id == &collection.id,
                "persisted snapshot collection key does not match embedded identifier"
            );
            validate_text("collection name", &collection.name)?;
            if let Some(description) = &collection.description {
                validate_text("collection description", description)?;
            }
            collection_members = collection_members
                .checked_add(collection.members.len())
                .context("persisted snapshot collection members count overflowed")?;
            let mut unique = HashSet::with_capacity(collection.members.len());
            anyhow::ensure!(
                collection
                    .members
                    .iter()
                    .all(|member| unique.insert(member)),
                "persisted snapshot collection contains duplicate membership"
            );
            validate_object_size("collection", collection)?;
        }
        validate_count(
            "collection members",
            collection_members,
            MAX_COLLECTION_MEMBERS,
        )?;

        validate_collection_graph(&state.collections)?;

        let mut bindings = 0_usize;
        let mut appearance_slots = 0_usize;
        for entry in &state.targets {
            if let TargetState::AppearanceSlots(values) = &entry.state {
                validate_unique_slots("target appearance-slot values", values)?;
                appearance_slots =
                    checked_total("appearance-slot values", appearance_slots, values.len())?;
            }
        }
        let mut scene_ids = state.scenes.keys().collect::<Vec<_>>();
        scene_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        for id in scene_ids {
            let scene = state
                .scenes
                .get(id)
                .context("persisted snapshot scene index changed during validation")?;
            anyhow::ensure!(
                id == &scene.id,
                "persisted snapshot scene key does not match embedded identifier"
            );
            scene
                .validate()
                .context("persisted snapshot scene is invalid")?;
            validate_text("scene name", &scene.name)?;
            if let Some(description) = &scene.description {
                validate_text("scene description", description)?;
            }
            bindings = checked_total("scene bindings", bindings, scene.bindings.len())?;
            for binding in &scene.bindings {
                if let Some(collection) = binding.dynamic_collection() {
                    anyhow::ensure!(
                        state.collections.contains_key(collection),
                        "persisted snapshot scene references missing collection {collection:?}"
                    );
                }
                appearance_slots = checked_total(
                    "appearance-slot values",
                    appearance_slots,
                    binding
                        .state()
                        .appearance_slots
                        .as_ref()
                        .map_or(0, Vec::len),
                )?;
                if let Some(values) = &binding.state().appearance_slots {
                    validate_unique_slots("scene appearance-slot values", values)?;
                }
            }
            validate_object_size("scene", scene)?;
        }
        for facet in &state.adopted_baseline {
            if let FacetValue::AppearanceSlots(slots) = &facet.value {
                validate_unique_slots("adopted appearance-slot values", &slots.values)?;
                appearance_slots = checked_total(
                    "appearance-slot values",
                    appearance_slots,
                    slots.values.len(),
                )?;
            }
        }
        validate_count("scene bindings", bindings, MAX_SCENE_BINDINGS)?;
        validate_count(
            "appearance-slot values",
            appearance_slots,
            MAX_APPEARANCE_SLOT_VALUES,
        )?;

        let payload = serde_json::to_vec_pretty(&state)?;
        validate_count("serialized bytes", payload.len(), MAX_STATE_FILE_BYTES)?;
        Ok(Self { state, payload })
    }

    fn into_loaded(self) -> LoadedState {
        LoadedState {
            preserve_order: self.state.version >= ORDERED_SINCE_VERSION,
            entries: self.state.targets,
            collections: self.state.collections,
            scenes: self.state.scenes,
            adopted_baseline: self.state.adopted_baseline,
        }
    }
}

fn checked_total(dimension: &str, current: usize, additional: usize) -> Result<usize> {
    current.checked_add(additional).with_context(|| {
        format!("persisted snapshot resource limit exceeded: {dimension} count overflowed")
    })
}

fn validate_count(dimension: &str, actual: usize, maximum: usize) -> Result<()> {
    anyhow::ensure!(
        actual <= maximum,
        "persisted snapshot resource limit exceeded: {dimension} is {actual}; maximum is {maximum}"
    );
    Ok(())
}

fn validate_text(dimension: &str, value: &str) -> Result<()> {
    validate_count(dimension, value.len(), MAX_TEXT_BYTES)
}

fn validate_unique_slots(dimension: &str, values: &[AppearanceSlotValue]) -> Result<()> {
    let mut slots = HashSet::with_capacity(values.len());
    anyhow::ensure!(
        values.iter().all(|value| slots.insert(&value.slot)),
        "persisted snapshot {dimension} contain duplicate slot identifiers"
    );
    Ok(())
}

fn validate_collection_graph(collections: &HashMap<CollectionId, Collection>) -> Result<()> {
    let mut complete = HashSet::with_capacity(collections.len());
    let mut visiting = HashSet::with_capacity(collections.len());
    let mut roots = collections.keys().cloned().collect::<Vec<_>>();
    roots.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    for root in &roots {
        if complete.contains(root) {
            continue;
        }
        visiting.insert(root.clone());
        let mut stack = vec![(root.clone(), 0_usize)];
        while let Some((id, member_index)) = stack.last_mut() {
            let collection = collections.get(id).with_context(|| {
                format!("persisted snapshot references missing collection {id:?}")
            })?;
            let Some(member) = collection.members.get(*member_index) else {
                let completed = id.clone();
                stack.pop();
                visiting.remove(&completed);
                complete.insert(completed);
                continue;
            };
            *member_index += 1;
            let CollectionMember::Collection(nested) = member else {
                continue;
            };
            anyhow::ensure!(
                collections.contains_key(nested),
                "persisted snapshot references missing collection {nested:?}"
            );
            if complete.contains(nested) {
                continue;
            }
            anyhow::ensure!(
                visiting.insert(nested.clone()),
                "persisted snapshot collection graph contains a cycle at {nested:?}"
            );
            stack.push((nested.clone(), 0));
        }
    }
    Ok(())
}

fn validate_object_size(dimension: &str, value: &impl Serialize) -> Result<()> {
    let size = serde_json::to_vec(value)?.len();
    validate_count(
        &format!("{dimension} serialized bytes"),
        size,
        MAX_OBJECT_BYTES,
    )
}

#[derive(Debug, Default)]
pub struct LoadedState {
    pub entries: Vec<TargetStateEntry>,

    pub preserve_order: bool,

    pub collections: HashMap<CollectionId, Collection>,

    pub scenes: HashMap<SceneId, Scene>,

    pub adopted_baseline: Vec<AdoptedFacet>,
}

const fn legacy_persistence_version() -> u32 {
    1
}

/// Loads persisted target state from `path`.
///
/// A missing file is treated as empty state. A file that exists but fails to
/// parse, exceeds [`MAX_STATE_FILE_BYTES`], or holds more than
/// [`MAX_STATE_ENTRIES`] entries is moved to a timestamped `.corrupt` sidecar
/// on a best-effort basis, then logged and treated as empty state rather than
/// failing daemon startup. Cached lighting state is a fallback, not a source of
/// truth, but retaining the rejected file makes later diagnosis possible. The
/// final path component is opened with `O_NOFOLLOW`, so a symlink swapped in at
/// the state path is rejected rather than followed.
///
/// # Errors
///
/// Returns an error only if `path` exists but cannot be opened or read (for
/// example, a permissions failure, a symlink at the state path, or another I/O
/// error). A missing file or a file with invalid/oversized contents does not
/// produce an error; see above.
#[allow(
    clippy::too_many_lines,
    reason = "load keeps rejection, retention, migration, and bounded validation in one linear path"
)]
pub fn load(path: &Path) -> Result<LoadedState> {
    load_internal(path, true)
}

/// Loads state for the offline ownership-reset command without applying the
/// normal startup refusal for legacy UID/SID owners.
fn load_for_reset(path: &Path) -> Result<LoadedState> {
    load_internal(path, false)
}

#[allow(
    clippy::too_many_lines,
    reason = "The reset loader shares the normal bounded parsing and retention path."
)]
fn load_internal(path: &Path, reject_legacy_owners: bool) -> Result<LoadedState> {
    let file = match open_private_file_for_read(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(LoadedState::default()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to open state file {}", path.display()));
        }
    };

    // Read at most one byte past the cap so an oversized file is *detected*
    // rather than silently truncated (a truncated read would parse as corrupt
    // and be indistinguishable from a genuinely truncated file).
    let read_limit = u64::try_from(MAX_STATE_FILE_BYTES)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut contents = String::new();
    let read = (&file)
        .take(read_limit)
        .read_to_string(&mut contents)
        .with_context(|| format!("failed to read state file {}", path.display()))?;
    if read > MAX_STATE_FILE_BYTES {
        drop(file);
        let retained_path = retain_rejected_state(path);
        tracing::warn!(
            path = %path.display(),
            retained_path = retained_path.as_ref().map(|path| path.display().to_string()),
            limit = MAX_STATE_FILE_BYTES,
            "persisted state file exceeds size limit; starting with empty state"
        );
        return Ok(LoadedState::default());
    }

    let mut value: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(value) => value,
        Err(error) => {
            drop(file);
            let retained_path = retain_rejected_state(path);
            tracing::warn!(
                path = %path.display(),
                retained_path = retained_path.as_ref().map(|path| path.display().to_string()),
                error = %error,
                "failed to parse persisted state file; starting with empty state"
            );
            return Ok(LoadedState::default());
        }
    };
    if value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1)
        <= 8
        && let Err(error) = migrate_v8_colours(&mut value)
    {
        drop(file);
        let retained_path = retain_rejected_state(path);
        tracing::warn!(
            path = %path.display(),
            retained_path = retained_path.as_ref().map(|path| path.display().to_string()),
            error = %error,
            "failed to migrate persisted state file; starting with empty state"
        );
        return Ok(LoadedState::default());
    }

    let mut parsed: PersistedState = match serde_json::from_value(value) {
        Ok(parsed) => parsed,
        Err(error) => {
            drop(file);
            let retained_path = retain_rejected_state(path);
            tracing::warn!(
                path = %path.display(),
                retained_path = retained_path.as_ref().map(|path| path.display().to_string()),
                error = %error,
                "failed to parse persisted state file; starting with empty state"
            );
            return Ok(LoadedState::default());
        }
    };

    let migrated_alienware = match migrate_alienware_power_button(&mut parsed) {
        Ok(migrated) => migrated,
        Err(error) => {
            drop(file);
            let retained_path = retain_rejected_state(path);
            tracing::warn!(
                path = %path.display(),
                retained_path = retained_path.as_ref().map(|path| path.display().to_string()),
                error,
                "failed to migrate Alienware power-button state; starting with empty state"
            );
            return Ok(LoadedState::default());
        }
    };
    if migrated_alienware {
        let backup = migration_backup_path(path);
        if !backup.exists() {
            fs::copy(path, &backup).with_context(|| {
                format!(
                    "failed to preserve pre-migration state {} at {}",
                    path.display(),
                    backup.display()
                )
            })?;
        }
        tracing::info!(
            path = %path.display(),
            backup = %backup.display(),
            "migrated Alienware power-button state in memory; preserved the original snapshot"
        );
    }

    if reject_legacy_owners {
        if let Some((id, _)) = parsed.collections.iter().find(|(_, collection)| {
            matches!(
                collection.owner,
                OwnerIdentity::Uid(_) | OwnerIdentity::Sid(_)
            )
        }) {
            anyhow::bail!(
                "persisted collection {id:?} has legacy UID/SID ownership; run `luminated state reset-owned-objects --confirm` while the daemon is stopped"
            );
        }
        if let Some((id, _)) = parsed
            .scenes
            .iter()
            .find(|(_, scene)| matches!(scene.owner, OwnerIdentity::Uid(_) | OwnerIdentity::Sid(_)))
        {
            anyhow::bail!(
                "persisted scene {id:?} has legacy UID/SID ownership; run `luminated state reset-owned-objects --confirm` while the daemon is stopped"
            );
        }
    }

    match ValidatedPersistedSnapshot::new(parsed) {
        Ok(snapshot) => Ok(snapshot.into_loaded()),
        Err(error) => {
            drop(file);
            let retained_path = retain_rejected_state(path);
            tracing::warn!(
                path = %path.display(),
                retained_path = retained_path.as_ref().map(|path| path.display().to_string()),
                error = %error,
                "persisted snapshot failed validation; starting with empty state"
            );
            Ok(LoadedState::default())
        }
    }
}

fn migrate_v8_colours(value: &mut serde_json::Value) -> Result<(), String> {
    let targets = value
        .get_mut("targets")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| "persisted targets must be an array".to_owned())?;
    for entry in targets {
        let state = entry
            .get_mut("state")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| "persisted target state must be an object".to_owned())?;
        if let Some(colour) = state.remove("Colour") {
            let colour = migrate_v8_colour(colour)?;
            state.insert(
                "Effect".to_owned(),
                serde_json::json!({"Static": {"colour": colour}}),
            );
        } else if let Some(effect) = state.get_mut("Effect")
            && let Some(static_effect) = effect.get_mut("Static")
            && let Some(colour) = static_effect.get_mut("colour")
        {
            *colour = if colour.get("r").is_some() {
                let red = colour
                    .get("r")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| "legacy static red channel is invalid".to_owned())?;
                let green = colour
                    .get("g")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| "legacy static green channel is invalid".to_owned())?;
                let blue = colour
                    .get("b")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| "legacy static blue channel is invalid".to_owned())?;
                serde_json::json!({"Additive": [
                    {"channel": "Red", "value": red},
                    {"channel": "Green", "value": green},
                    {"channel": "Blue", "value": blue}
                ]})
            } else {
                migrate_v8_colour(colour.take())?
            };
        }
    }
    Ok(())
}

const ALIENWARE_DEVICE: &str = "alienware-aw-elc";
const LEGACY_POWER_BUTTON_AC: &str = "power-button-ac";
const LEGACY_POWER_BUTTON_BATTERY: &str = "power-button-battery";
const POWER_BUTTON: &str = "power-button";

fn migrate_alienware_power_button(state: &mut PersistedState) -> Result<bool, String> {
    let mut migrated = false;
    let mut desired_slots = HashSet::new();
    for entry in &mut state.targets {
        let Some(slot) = legacy_power_button_slot(&entry.target) else {
            continue;
        };
        if !desired_slots.insert(slot) {
            return Err(format!(
                "persisted state has duplicate legacy Alienware {slot} target entries"
            ));
        }
        let TargetState::Effect(effect) = &entry.state else {
            return Err(format!(
                "legacy Alienware target {:?} has state that cannot be represented as an appearance slot",
                entry.target
            ));
        };
        entry.target = TargetId::surface(ALIENWARE_DEVICE, POWER_BUTTON);
        entry.state = TargetState::AppearanceSlots(vec![AppearanceSlotValue {
            slot: AppearanceSlotId::new(slot),
            effect: effect.clone(),
        }]);
        migrated = true;
    }

    let mut adopted_slots = HashSet::new();
    for facet in &mut state.adopted_baseline {
        let Some(slot) = legacy_power_button_slot(&facet.target) else {
            continue;
        };
        if !adopted_slots.insert(slot) {
            return Err(format!(
                "adopted baseline has duplicate legacy Alienware {slot} facets"
            ));
        }
        let FacetValue::Appearance(appearance) = &facet.value else {
            return Err(format!(
                "legacy Alienware adopted facet {:?} cannot be represented as an appearance slot",
                facet.target
            ));
        };
        let effect = match appearance {
            AppearanceState::Static(colour) => Effect::Static {
                colour: colour.clone(),
            },
            AppearanceState::Effect(effect) => effect.clone(),
            AppearanceState::Mixed => {
                return Err(format!(
                    "legacy Alienware adopted facet {:?} has a mixed appearance",
                    facet.target
                ));
            }
        };
        facet.target = TargetId::surface(ALIENWARE_DEVICE, POWER_BUTTON);
        facet.value = FacetValue::AppearanceSlots(AppearanceSlotsState {
            values: vec![AppearanceSlotValue {
                slot: AppearanceSlotId::new(slot),
                effect,
            }],
            complete: false,
        });
        migrated = true;
    }

    let mut scene_ids = state.scenes.keys().cloned().collect::<Vec<_>>();
    scene_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    for id in scene_ids {
        let scene = state.scenes.get_mut(&id).ok_or_else(|| {
            format!(
                "persisted scene {} disappeared during migration",
                id.as_str()
            )
        })?;
        migrated |= migrate_alienware_scene(scene)?;
    }

    Ok(migrated)
}

fn migrate_alienware_scene(scene: &mut Scene) -> Result<bool, String> {
    let mut migrated = false;
    let mut bindings = Vec::with_capacity(scene.bindings.len());
    for mut binding in mem::take(&mut scene.bindings) {
        let Some(slot) = legacy_power_button_slot(binding.target()) else {
            bindings.push(binding);
            continue;
        };
        let state = match &mut binding {
            SceneBinding::Frozen { target, state }
            | SceneBinding::DynamicCollectionMember { target, state, .. } => {
                *target = TargetId::surface(ALIENWARE_DEVICE, POWER_BUTTON);
                state
            }
        };
        let effect = state.appearance.take().ok_or_else(|| {
            format!(
                "scene {} legacy Alienware binding has no appearance to migrate",
                scene.id.as_str()
            )
        })?;
        if state.brightness.is_some()
            || state.emission.is_some()
            || state.appearance_slots.is_some()
        {
            return Err(format!(
                "scene {} legacy Alienware binding contains incompatible additional state",
                scene.id.as_str()
            ));
        }
        state.appearance_slots = Some(vec![AppearanceSlotValue {
            slot: AppearanceSlotId::new(slot),
            effect,
        }]);
        let new_value = state
            .appearance_slots
            .as_ref()
            .and_then(|values| values.first())
            .cloned()
            .ok_or_else(|| format!("scene {} has an empty migrated binding", scene.id.as_str()))?;

        if let Some(existing) = bindings
            .iter_mut()
            .find(|existing| same_scene_binding_identity(existing, &binding))
        {
            let existing_values = match existing {
                SceneBinding::Frozen { state, .. }
                | SceneBinding::DynamicCollectionMember { state, .. } => {
                    state.appearance_slots.as_mut()
                }
            }
            .ok_or_else(|| {
                format!(
                    "scene {} has a contradictory power-button binding",
                    scene.id.as_str()
                )
            })?;
            if existing_values
                .iter()
                .any(|value| value.slot == new_value.slot)
            {
                return Err(format!(
                    "scene {} has duplicate legacy Alienware {} bindings",
                    scene.id.as_str(),
                    new_value.slot.as_str()
                ));
            }
            existing_values.push(new_value);
        } else {
            bindings.push(binding);
        }
        migrated = true;
    }
    scene.bindings = bindings;
    Ok(migrated)
}

fn same_scene_binding_identity(left: &SceneBinding, right: &SceneBinding) -> bool {
    match (left, right) {
        (SceneBinding::Frozen { target: left, .. }, SceneBinding::Frozen { target: right, .. }) => {
            left == right
        }
        (
            SceneBinding::DynamicCollectionMember {
                collection: left_collection,
                target: left_target,
                ..
            },
            SceneBinding::DynamicCollectionMember {
                collection: right_collection,
                target: right_target,
                ..
            },
        ) => left_collection == right_collection && left_target == right_target,
        _ => false,
    }
}

fn legacy_power_button_slot(target: &TargetId) -> Option<&'static str> {
    let TargetId::Surface { device, surface } = target else {
        return None;
    };
    if device.as_str() != ALIENWARE_DEVICE {
        return None;
    }
    match surface.as_str() {
        LEGACY_POWER_BUTTON_AC => Some("ac"),
        LEGACY_POWER_BUTTON_BATTERY => Some("battery"),
        _ => None,
    }
}

fn migration_backup_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    path.with_file_name(format!("{file_name}.pre-appearance-slots-v12.bak"))
}

fn migrate_v8_colour(value: serde_json::Value) -> Result<serde_json::Value, String> {
    if value.get("Additive").is_some()
        || value.get("Hsv").is_some()
        || value.get("Hsl").is_some()
        || value.get("Cct").is_some()
        || value.get("Monochrome").is_some()
    {
        return Ok(value);
    }
    let encoding = value
        .get("encoding")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "legacy colour encoding is missing".to_owned())?;
    let channels = value
        .get("channels")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "legacy colour channels are missing".to_owned())?;
    let component = |name: &str| {
        channels
            .iter()
            .find(|channel| {
                channel.get("channel").and_then(serde_json::Value::as_str) == Some(name)
            })
            .and_then(|channel| channel.get("value"))
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("legacy {encoding} colour is missing {name}"))
    };
    match encoding {
        "Additive" => {
            if channels.is_empty() {
                return Err("legacy additive colour has no channels".to_owned());
            }
            Ok(serde_json::json!({"Additive": channels}))
        }
        "Hsv" => Ok(serde_json::json!({"Hsv": {
            "hue": component("Hue")?,
            "saturation": component("Saturation")?,
            "value": component("Value")?
        }})),
        "Hsl" => Ok(serde_json::json!({"Hsl": {
            "hue": component("Hue")?,
            "saturation": component("Saturation")?,
            "lightness": component("Lightness")?
        }})),
        "Cct" => Ok(serde_json::json!({"Cct": {"kelvin": component("Temperature")?}})),
        "Monochrome" => {
            Ok(serde_json::json!({"Monochrome": {"intensity": component("Intensity")?}}))
        }
        other => Err(format!("unknown legacy colour encoding {other}")),
    }
}

/// Moves a rejected state file aside without allowing preservation failure to
/// prevent daemon startup.
fn retain_rejected_state(path: &Path) -> Option<PathBuf> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let retained_path = parent.join(format!(
        "{file_name}.{timestamp}-{:016x}.corrupt",
        random_temp_token()
    ));

    if let Err(error) = fs::rename(path, &retained_path) {
        tracing::warn!(
            path = %path.display(),
            error = %error,
            "failed to retain rejected persisted state file"
        );
        return None;
    }

    if let Err(error) = sync_parent_directory(&retained_path) {
        tracing::warn!(
            retained_path = %retained_path.display(),
            error = %error,
            "retained rejected persisted state file, but failed to sync its directory"
        );
    }

    Some(retained_path)
}

pub fn save_complete(
    path: &Path,
    target_state: &[TargetStateEntry],
    collections: &HashMap<CollectionId, Collection>,
    scenes: &HashMap<SceneId, Scene>,
    adopted_baseline: &[AdoptedFacet],
) -> Result<()> {
    let parent = prepare_state_dir(path)?;

    let snapshot = ValidatedPersistedSnapshot::new(PersistedState {
        version: PERSISTENCE_VERSION,
        targets: target_state.to_vec(),
        collections: collections.clone(),
        scenes: scenes.clone(),
        adopted_baseline: adopted_baseline.to_vec(),
    })?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    let tmp_path = parent.join(format!(".{file_name}.{:016x}.tmp", random_temp_token()));

    atomic_file::replace(path, &tmp_path, &snapshot.payload)
        .with_context(|| format!("failed to atomically replace state file {}", path.display()))?;

    Ok(())
}

pub(crate) fn validate_complete(
    target_state: &[TargetStateEntry],
    collections: &HashMap<CollectionId, Collection>,
    scenes: &HashMap<SceneId, Scene>,
    adopted_baseline: &[AdoptedFacet],
) -> Result<()> {
    ValidatedPersistedSnapshot::new(PersistedState {
        version: PERSISTENCE_VERSION,
        targets: target_state.to_vec(),
        collections: collections.clone(),
        scenes: scenes.clone(),
        adopted_baseline: adopted_baseline.to_vec(),
    })?;
    Ok(())
}

/// Backs up state and clears only legacy-owned collections and scenes.
///
/// Target state and the adopted physical baseline are retained. The caller
/// must explicitly confirm the operation, and the daemon must be stopped so a
/// concurrent mutation cannot replace the backup or be lost by the rewrite.
///
/// # Errors
///
/// Returns an error when confirmation is absent, state cannot be loaded or
/// backed up, or the replacement cannot be written atomically.
pub fn reset_owned_objects(path: &Path, confirmed: bool) -> Result<PathBuf> {
    anyhow::ensure!(
        confirmed,
        "refusing to reset owned objects without --confirm"
    );
    anyhow::ensure!(
        path.exists(),
        "cannot reset owned objects because state file {} does not exist",
        path.display()
    );

    let loaded = load_for_reset(path)?;
    let backup = backup_path(path);
    fs::copy(path, &backup).with_context(|| {
        format!(
            "failed to back up state file {} to {}",
            path.display(),
            backup.display()
        )
    })?;

    if let Err(error) = save_complete(
        path,
        &loaded.entries,
        &HashMap::new(),
        &HashMap::new(),
        &loaded.adopted_baseline,
    ) {
        tracing::error!(
            path = %path.display(),
            backup = %backup.display(),
            error = %error,
            "owned-object reset failed after backup; original state remains available in backup"
        );
        return Err(error).context("failed to write reset state");
    }

    Ok(backup)
}

fn backup_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    path.with_file_name(format!(
        "{file_name}.pre-access-control-reset-{timestamp}-{:016x}.bak",
        random_temp_token()
    ))
}

/// Ensures the parent directory of `path` exists, is owned by the daemon's
/// user, and cannot be written by another identity, returning that directory.
///
/// Like `prepare_socket_path`, this rejects directories writable by other
/// users (for example, a shared `/tmp`) so state files cannot be
/// redirected, replaced, or pre-created by another local user.
///
/// Packaged deployments normally create this directory with the desired
/// permissions during `ExecStartPre`; this logic primarily supports
/// development and fallback cases where the daemon must create the parent
/// directory itself.
fn prepare_state_dir(path: &Path) -> Result<&Path> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("state path must have a parent directory")?;

    ensure_service_directory(parent)
        .with_context(|| format!("failed to secure state directory {}", parent.display()))?;

    Ok(parent)
}

/// Returns an unpredictable temporary-file name component.
///
/// The platform's secure RNG (see [`luminate_platform::secure_random`]) is
/// preferred so another local user cannot reliably pre-create the chosen
/// name in a shared directory. If that RNG is unreachable, a time/PID/counter
/// mix provides a best-effort fallback.
///
/// Correctness does not depend on unpredictability: the temporary file is
/// always created with exclusive-create semantics (`O_EXCL` on Unix,
/// `CREATE_NEW` on Windows), so an existing name is never clobbered.
/// Randomness merely makes collisions and deliberate pre-creation
/// impractical.
fn random_temp_token() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            elapsed
                .as_secs()
                .wrapping_mul(1_000_000_000)
                .wrapping_add(u64::from(elapsed.subsec_nanos()))
        });
    let mut token =
        u64::from(process::id()).rotate_left(32) ^ COUNTER.fetch_add(1, Ordering::Relaxed) ^ seed;

    let mut bytes = [0_u8; 8];
    if fill_bytes(&mut bytes).is_ok() {
        token ^= u64::from_ne_bytes(bytes);
    }
    token
}

fn sync_parent_directory(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };

    sync_directory(parent)
        .with_context(|| format!("failed to sync state directory {}", parent.display()))
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
