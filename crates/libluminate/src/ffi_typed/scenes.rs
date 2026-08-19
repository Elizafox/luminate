// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "each FFI entry point documents one pointer contract covering its repeated field reads"
)]

use super::appearance_slots::{LuminateAppearanceSlotInput, read_appearance_slot_inputs};
use super::effects::read_target;
use super::*;

/// Mutable deep-copying editor seeded from an existing scene.
pub struct LuminateSceneBuilder {
    id: SceneId,
    expected_revision: u64,
    name: String,
    description: Option<String>,
    bindings: Vec<SceneBinding>,
}

pub(crate) struct SceneBuilderSubmission {
    pub(crate) id: SceneId,
    pub(crate) expected_revision: u64,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) bindings: Vec<SceneBinding>,
}

null_safe_free!(
    "Releases a scene builder created by `luminate_scene_builder_from_scene`. Null is a no-op.",
    luminate_scene_builder_free,
    LuminateSceneBuilder
);

/// Sparse state input for one scene binding.
#[repr(C)]
pub struct LuminateSceneTargetStateInput {
    /// Optional owned/borrowed effect handle. `Effect::Off` is rejected.
    pub appearance: *const LuminateEffect,
    pub has_brightness: bool,
    pub brightness: u32,
    pub has_emission: bool,
    /// One of the `LUMINATE_EMISSION_*` values.
    pub emission: LuminateEmissionState,
    pub appearance_slots: *const LuminateAppearanceSlotInput,
    pub appearance_slot_count: usize,
}

/// One explicitly-authored scene binding.
#[repr(C)]
pub struct LuminateSceneBindingInput {
    /// Null for frozen binding; otherwise the gating collection id.
    pub dynamic_collection_id: *const c_char,
    pub target: LuminateTarget,
    pub state: LuminateSceneTargetStateInput,
}

unsafe fn read_binding(v: &LuminateSceneBindingInput) -> Result<SceneBinding, LuminateStatus> {
    // SAFETY: upheld by the enclosing public function's pointer contract.
    let target = unsafe { read_target(ptr::from_ref(&v.target)) }?;
    let state = unsafe { read_scene_target_state(&v.state) }?;
    if v.dynamic_collection_id.is_null() {
        Ok(SceneBinding::Frozen { target, state })
    } else {
        // SAFETY: upheld by the enclosing public function's pointer contract.
        let collection =
            unsafe { read_required_str(v.dynamic_collection_id, "dynamic_collection_id") }?;
        Ok(SceneBinding::DynamicCollectionMember {
            collection: CollectionId::new(collection),
            target,
            state,
        })
    }
}

pub(super) unsafe fn read_scene_target_state(
    value: &LuminateSceneTargetStateInput,
) -> Result<SceneTargetState, LuminateStatus> {
    let appearance = if value.appearance.is_null() {
        None
    } else {
        effect_ref(value.appearance).cloned()
    };
    let emission = if value.has_emission {
        match value.emission {
            0 => Some(EmissionState::Dark),
            1 => Some(EmissionState::Emitting),
            other => {
                crate::ffi::set_last_error(format!("unrecognized emission state {other}"));
                return Err(LuminateStatus::InvalidArgument);
            }
        }
    } else {
        None
    };
    let state = SceneTargetState {
        appearance,
        brightness: value.has_brightness.then_some(value.brightness),
        emission,
        appearance_slots: {
            // SAFETY: upheld by the enclosing public function's pointer contract.
            let values = unsafe {
                read_appearance_slot_inputs(value.appearance_slots, value.appearance_slot_count)
            }?;
            (!values.is_empty()).then_some(values)
        },
    };
    if let Err(error) = state.validate() {
        crate::ffi::set_last_error(error.to_string());
        return Err(LuminateStatus::InvalidArgument);
    }
    Ok(state)
}

pub(crate) unsafe fn read_bindings(
    values: *const LuminateSceneBindingInput,
    count: usize,
) -> Result<Vec<SceneBinding>, LuminateStatus> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if values.is_null() {
        crate::ffi::set_last_error("scene bindings pointer is null");
        return Err(LuminateStatus::NullPointer);
    }
    // SAFETY: upheld by the enclosing public function's pointer contract.
    unsafe { std::slice::from_raw_parts(values, count) }
        .iter()
        // SAFETY: each slice element is a readable binding input.
        .map(|value| unsafe { read_binding(value) })
        .collect()
}

pub(crate) unsafe fn read_targets(
    values: *const LuminateTarget,
    count: usize,
) -> Result<Vec<TargetId>, LuminateStatus> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if values.is_null() {
        crate::ffi::set_last_error("scene targets pointer is null");
        return Err(LuminateStatus::NullPointer);
    }
    // SAFETY: upheld by the enclosing public function's pointer contract.
    unsafe { std::slice::from_raw_parts(values, count) }
        .iter()
        // SAFETY: each slice element is a readable target input.
        .map(|value| unsafe { read_target(ptr::from_ref(value)) })
        .collect()
}

pub(crate) unsafe fn optional_string(
    value: *const c_char,
    field: &'static str,
) -> Result<Option<String>, LuminateStatus> {
    if value.is_null() {
        Ok(None)
    } else {
        // SAFETY: upheld by the enclosing public function's pointer contract.
        unsafe { read_required_str(value, field) }.map(|value| Some(value.to_owned()))
    }
}

/// Creates an independent scene builder seeded from every editable field in
/// `scene`. Release it with `luminate_scene_builder_free`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_builder_from_scene(
    scene: *const LuminateScene,
    out_builder: *mut *mut LuminateSceneBuilder,
) -> LuminateStatus {
    ffi_guard(|| {
        let Some(scene) = (native_ref!(scene, Scene)) else {
            crate::ffi::set_last_error("scene pointer is null");
            return LuminateStatus::NullPointer;
        };
        let builder = LuminateSceneBuilder {
            id: scene.id.clone(),
            expected_revision: scene.revision,
            name: scene.name.clone(),
            description: scene.description.clone(),
            bindings: scene.bindings.clone(),
        };
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        if let Err(status) = unsafe { write_box(out_builder, builder, "scene builder") } {
            return status;
        }
        clear_last_error();
        LuminateStatus::Ok
    })
}

fn scene_builder_mut(
    builder: *mut LuminateSceneBuilder,
) -> Result<&'static mut LuminateSceneBuilder, LuminateStatus> {
    // SAFETY: the caller provides exclusive access to the opaque builder.
    let Some(builder) = (unsafe { builder.as_mut() }) else {
        crate::ffi::set_last_error("scene builder pointer is null");
        return Err(LuminateStatus::NullPointer);
    };
    Ok(builder)
}

fn scene_builder_ref(
    builder: *const LuminateSceneBuilder,
) -> Result<&'static LuminateSceneBuilder, LuminateStatus> {
    // SAFETY: the caller provides a live opaque builder.
    let Some(builder) = (unsafe { builder.as_ref() }) else {
        crate::ffi::set_last_error("scene builder pointer is null");
        return Err(LuminateStatus::NullPointer);
    };
    Ok(builder)
}

pub(crate) fn clone_scene_builder(
    builder: *const LuminateSceneBuilder,
) -> Result<SceneBuilderSubmission, LuminateStatus> {
    let builder = scene_builder_ref(builder)?;
    Ok(SceneBuilderSubmission {
        id: builder.id.clone(),
        expected_revision: builder.expected_revision,
        name: builder.name.clone(),
        description: builder.description.clone(),
        bindings: builder.bindings.clone(),
    })
}

/// Replaces the builder's required UTF-8 scene name.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_builder_set_name(
    builder: *mut LuminateSceneBuilder,
    name: *const c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        let builder = match scene_builder_mut(builder) {
            Ok(value) => value,
            Err(status) => return status,
        };
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let name = match unsafe { read_required_str(name, "scene name") } {
            Ok(value) => value.to_owned(),
            Err(status) => return status,
        };
        builder.name = name;
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Replaces the builder's UTF-8 description, or clears it when `description`
/// is null.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_builder_set_description(
    builder: *mut LuminateSceneBuilder,
    description: *const c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        let builder = match scene_builder_mut(builder) {
            Ok(value) => value,
            Err(status) => return status,
        };
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let description = match unsafe { optional_string(description, "scene description") } {
            Ok(value) => value,
            Err(status) => return status,
        };
        builder.description = description;
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Number of bindings currently held by a scene builder.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_builder_binding_count(
    builder: *const LuminateSceneBuilder,
) -> usize {
    scene_builder_ref(builder).map_or(0, |builder| builder.bindings.len())
}

/// Borrowed binding at `index`, or null when the builder is null or the index
/// is out of range. The view is invalidated by the next builder mutation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_builder_binding_at(
    builder: *const LuminateSceneBuilder,
    index: usize,
) -> *const LuminateSceneBinding {
    scene_builder_ref(builder)
        .ok()
        .and_then(|builder| builder.bindings.get(index))
        .map_or(ptr::null(), cast_ref)
}

/// Appends a deep copy of one validated binding input.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_builder_add_binding(
    builder: *mut LuminateSceneBuilder,
    binding: *const LuminateSceneBindingInput,
) -> LuminateStatus {
    ffi_guard(|| {
        let builder = match scene_builder_mut(builder) {
            Ok(value) => value,
            Err(status) => return status,
        };
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let Some(binding) = (unsafe { binding.as_ref() }) else {
            crate::ffi::set_last_error("scene binding pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: nested pointers are governed by the input contract.
        let binding = match unsafe { read_binding(binding) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        builder.bindings.push(binding);
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Replaces the binding at `index` with a deep copy of `binding`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_builder_replace_binding(
    builder: *mut LuminateSceneBuilder,
    index: usize,
    binding: *const LuminateSceneBindingInput,
) -> LuminateStatus {
    ffi_guard(|| {
        let builder = match scene_builder_mut(builder) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let Some(slot) = builder.bindings.get_mut(index) else {
            crate::ffi::set_last_error(format!("scene builder has no binding at index {index}"));
            return LuminateStatus::InvalidArgument;
        };
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let Some(binding) = (unsafe { binding.as_ref() }) else {
            crate::ffi::set_last_error("scene binding pointer is null");
            return LuminateStatus::NullPointer;
        };
        // SAFETY: nested pointers are governed by the input contract.
        let replacement = match unsafe { read_binding(binding) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        *slot = replacement;
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Removes the binding at `index`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_builder_remove_binding(
    builder: *mut LuminateSceneBuilder,
    index: usize,
) -> LuminateStatus {
    ffi_guard(|| {
        let builder = match scene_builder_mut(builder) {
            Ok(value) => value,
            Err(status) => return status,
        };
        if index >= builder.bindings.len() {
            crate::ffi::set_last_error(format!("scene builder has no binding at index {index}"));
            return LuminateStatus::InvalidArgument;
        }
        builder.bindings.remove(index);
        clear_last_error();
        LuminateStatus::Ok
    })
}

/// Replaces the source scene using the builder's stored identifier and
/// expected revision. The builder is neither consumed nor mutated.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_replace_scene_from_builder(
    client: *mut LuminateClient,
    builder: *const LuminateSceneBuilder,
    out: *mut *mut LuminateSceneSnapshot,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let builder = match scene_builder_ref(builder) {
            Ok(value) => value,
            Err(status) => return status,
        };
        // SAFETY: upheld by the enclosing function's documented pointer contract.
        let out = match unsafe { out_ptr_mut(out, "scene snapshot") } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let id = builder.id.clone();
        let expected_revision = builder.expected_revision;
        let name = builder.name.clone();
        let description = builder.description.clone();
        let bindings = builder.bindings.clone();
        let result = match call_client(client, move |client| async move {
            client
                .replace_scene(id, expected_revision, name, description, bindings)
                .await
        }) {
            Ok(value) => value,
            Err(status) => return status,
        };
        match result {
            Ok(scene) => {
                *out = Box::into_raw(Box::new(LuminateSceneSnapshot(scene)));
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}

/// Creates an explicitly-authored scene.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_create_scene(
    client: *mut LuminateClient,
    name: *const c_char,
    description: *const c_char,
    bindings: *const LuminateSceneBindingInput,
    binding_count: usize,
    out: *mut *mut LuminateSceneSnapshot,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        // SAFETY: upheld by the enclosing function's pointer contract.
        let name = match unsafe { read_required_str(name, "name") } {
            Ok(value) => value.to_owned(),
            Err(error) => return error,
        };
        // SAFETY: upheld by the enclosing function's pointer contract.
        let description = match unsafe { optional_string(description, "description") } {
            Ok(value) => value,
            Err(error) => return error,
        };
        // SAFETY: upheld by the enclosing function's pointer contract.
        let bindings = match unsafe { read_bindings(bindings, binding_count) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        // SAFETY: upheld by the enclosing function's pointer contract.
        let out = match unsafe { out_ptr_mut(out, "scene snapshot") } {
            Ok(value) => value,
            Err(error) => return error,
        };
        let result = match call_client(client, move |client| async move {
            client.create_scene(name, description, bindings).await
        }) {
            Ok(value) => value,
            Err(error) => return error,
        };
        match result {
            Ok(scene) => {
                *out = Box::into_raw(Box::new(LuminateSceneSnapshot(scene)));
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}

/// Captures intended state. A non-null `dynamic_collection_id` captures its
/// current leaves when `target_count` is zero.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_capture_scene(
    client: *mut LuminateClient,
    name: *const c_char,
    description: *const c_char,
    dynamic_collection_id: *const c_char,
    targets: *const LuminateTarget,
    target_count: usize,
    out: *mut *mut LuminateSceneSnapshot,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: all reads are covered by the enclosing pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let name = match unsafe { read_required_str(name, "name") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        let description = match unsafe { optional_string(description, "description") } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let targets = match unsafe { read_targets(targets, target_count) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let mode = if dynamic_collection_id.is_null() {
            SceneCaptureMode::Frozen
        } else {
            let id = match unsafe {
                read_required_str(dynamic_collection_id, "dynamic_collection_id")
            } {
                Ok(v) => v.to_owned(),
                Err(e) => return e,
            };
            SceneCaptureMode::DynamicCollectionMembers {
                collection: CollectionId::new(id),
            }
        };
        let out = match unsafe { out_ptr_mut(out, "scene snapshot") } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let result = match call_client(client, move |client| async move {
            client.capture_scene(name, description, mode, targets).await
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(scene) => {
                *out = Box::into_raw(Box::new(LuminateSceneSnapshot(scene)));
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}

/// Replaces an explicitly-authored scene.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_replace_scene(
    client: *mut LuminateClient,
    id: *const c_char,
    expected_revision: u64,
    name: *const c_char,
    description: *const c_char,
    bindings: *const LuminateSceneBindingInput,
    binding_count: usize,
    out: *mut *mut LuminateSceneSnapshot,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: all reads are covered by the enclosing pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let id = match unsafe { read_required_str(id, "scene_id") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        let name = match unsafe { read_required_str(name, "name") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        let description = match unsafe { optional_string(description, "description") } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let bindings = match unsafe { read_bindings(bindings, binding_count) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let out = match unsafe { out_ptr_mut(out, "scene snapshot") } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let result = match call_client(client, move |client| async move {
            client
                .replace_scene(
                    SceneId::new(id),
                    expected_revision,
                    name,
                    description,
                    bindings,
                )
                .await
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(scene) => {
                *out = Box::into_raw(Box::new(LuminateSceneSnapshot(scene)));
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}

/// Recaptures intended state at an expected revision.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_recapture_scene(
    client: *mut LuminateClient,
    id: *const c_char,
    expected_revision: u64,
    dynamic_collection_id: *const c_char,
    targets: *const LuminateTarget,
    target_count: usize,
    out: *mut *mut LuminateSceneSnapshot,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: all reads are covered by the enclosing pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let id = match unsafe { read_required_str(id, "scene_id") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        let targets = match unsafe { read_targets(targets, target_count) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let mode = if dynamic_collection_id.is_null() {
            SceneCaptureMode::Frozen
        } else {
            let collection = match unsafe {
                read_required_str(dynamic_collection_id, "dynamic_collection_id")
            } {
                Ok(v) => v.to_owned(),
                Err(e) => return e,
            };
            SceneCaptureMode::DynamicCollectionMembers {
                collection: CollectionId::new(collection),
            }
        };
        let out = match unsafe { out_ptr_mut(out, "scene snapshot") } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let result = match call_client(client, move |client| async move {
            client
                .recapture_scene(SceneId::new(id), expected_revision, mode, targets)
                .await
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(scene) => {
                *out = Box::into_raw(Box::new(LuminateSceneSnapshot(scene)));
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}

/// Deletes a scene at an expected revision.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_delete_scene(
    client: *mut LuminateClient,
    id: *const c_char,
    expected_revision: u64,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let id = match unsafe { read_required_str(id, "scene_id") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        let result = match call_client(client, move |client| async move {
            client
                .delete_scene(SceneId::new(id), expected_revision)
                .await
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(()) => {
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}

/// Lists observable scenes.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_list_scenes(
    client: *mut LuminateClient,
    out: *mut *mut LuminateSceneList,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let out = match unsafe { out_ptr_mut(out, "scene list") } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let result = match call_client(client, |client| async move { client.list_scenes().await }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(scenes) => {
                *out = Box::into_raw(Box::new(LuminateSceneList(scenes)));
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}

/// Gets one observable scene.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_get_scene(
    client: *mut LuminateClient,
    id: *const c_char,
    out: *mut *mut LuminateSceneSnapshot,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let id = match unsafe { read_required_str(id, "scene_id") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        let out = match unsafe { out_ptr_mut(out, "scene snapshot") } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let result = match call_client(client, move |client| async move {
            client.get_scene(SceneId::new(id)).await
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(Some(scene)) => {
                *out = Box::into_raw(Box::new(LuminateSceneSnapshot(scene)));
                clear_last_error();
                LuminateStatus::Ok
            }
            Ok(None) => store_error(&Error::NotFound("scene".to_owned())),
            Err(error) => store_error(&error),
        }
    })
}

/// Applies a scene immediately. Authorization exclusions are available
/// through ordinary Rust APIs; this convenience call reports successful
/// completion only.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_apply_scene(
    client: *mut LuminateClient,
    id: *const c_char,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        let id = match unsafe { read_required_str(id, "scene_id") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        let result = match call_client(client, move |client| async move {
            client.apply_scene(SceneId::new(id)).await
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(_) => {
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_list_count(v: *const LuminateSceneList) -> usize {
    // SAFETY: null is accepted.
    unsafe { v.as_ref() }.map_or(0, |v| v.0.len())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_list_at(
    v: *const LuminateSceneList,
    index: usize,
) -> *const LuminateScene {
    // SAFETY: null is accepted.
    unsafe { v.as_ref() }
        .and_then(|v| v.0.get(index))
        .map_or(ptr::null(), cast_ref)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_snapshot_scene(
    v: *const LuminateSceneSnapshot,
) -> *const LuminateScene {
    // SAFETY: null is accepted.
    unsafe { v.as_ref() }.map_or(ptr::null(), |v| cast_ref(&v.0))
}

str_accessor!(
    "The scene identifier.",
    luminate_scene_id,
    LuminateScene,
    Scene,
    |v: &Scene| v.id.as_str()
);
str_accessor!(
    "The scene name.",
    luminate_scene_name,
    LuminateScene,
    Scene,
    |v: &Scene| v.name.as_str()
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_revision(v: *const LuminateScene) -> u64 {
    native_ref!(v, Scene).map_or(0, |v| v.revision)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_description(v: *const LuminateScene) -> LuminateStringView {
    native_ref!(v, Scene).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| optional_sv(v.description.as_deref()),
    )
}

vec_accessors!(
    "Number of scene bindings.",
    "Borrowed scene binding at index.",
    luminate_scene_binding_count,
    luminate_scene_binding_at,
    LuminateScene,
    Scene,
    LuminateSceneBinding,
    bindings
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_binding_target(
    v: *const LuminateSceneBinding,
) -> *const LuminateTargetView {
    native_ref!(v, SceneBinding).map_or(ptr::null(), |v| cast_ref(v.target()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_binding_collection_id(
    v: *const LuminateSceneBinding,
) -> LuminateStringView {
    native_ref!(v, SceneBinding).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| optional_sv(v.dynamic_collection().map(CollectionId::as_str)),
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_binding_appearance(
    v: *const LuminateSceneBinding,
) -> *const LuminateEffectView {
    native_ref!(v, SceneBinding).map_or(ptr::null(), |v| {
        v.state().appearance.as_ref().map_or(ptr::null(), cast_ref)
    })
}

/// Number of appearance-slot values stored by this binding.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_binding_appearance_slot_count(
    v: *const LuminateSceneBinding,
) -> usize {
    native_ref!(v, SceneBinding)
        .and_then(|value| value.state().appearance_slots.as_ref())
        .map_or(0, Vec::len)
}

/// Borrowed appearance-slot value at `index`, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_binding_appearance_slot_at(
    v: *const LuminateSceneBinding,
    index: usize,
) -> *const LuminateAppearanceSlotValue {
    native_ref!(v, SceneBinding)
        .and_then(|value| value.state().appearance_slots.as_ref())
        .and_then(|values| values.get(index))
        .map_or(ptr::null(), cast_ref)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_binding_has_brightness(
    v: *const LuminateSceneBinding,
) -> bool {
    native_ref!(v, SceneBinding).is_some_and(|v| v.state().brightness.is_some())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_binding_brightness(v: *const LuminateSceneBinding) -> u32 {
    native_ref!(v, SceneBinding)
        .and_then(|v| v.state().brightness)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_binding_has_emission(
    v: *const LuminateSceneBinding,
) -> bool {
    native_ref!(v, SceneBinding).is_some_and(|v| v.state().emission.is_some())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_binding_emission(
    v: *const LuminateSceneBinding,
) -> LuminateEmissionState {
    native_ref!(v, SceneBinding)
        .and_then(|v| v.state().emission)
        .map_or(u32::MAX, |value| value as u32)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_scene_owner(
    v: *const LuminateScene,
) -> *const LuminateOwnerIdentity {
    native_ref!(v, Scene).map_or(ptr::null(), |v| cast_ref(&v.owner))
}

#[cfg(test)]
#[path = "scenes_tests.rs"]
mod tests;
