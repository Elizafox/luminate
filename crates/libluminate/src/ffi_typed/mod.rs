// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Typed C11 API. Owned roots keep the native model alive; every nested value
//! is a borrowed view and therefore needs neither copying nor a separate free.
//!
//! Read-only accessors deliberately avoid a panic barrier. They must remain
//! panic-free for every valid pointer and for null: use checked access such as
//! `Option::map_or` and slice `get`, never indexing or assumptions enforced by
//! `unwrap`. Fallible operations and allocations belong behind [`ffi_guard`].

#![allow(
    missing_docs,
    unsafe_code,
    clippy::absolute_paths,
    clippy::match_wildcard_for_single_variants,
    clippy::multiple_unsafe_ops_per_block,
    clippy::ptr_as_ptr,
    clippy::redundant_closure_for_method_calls,
    clippy::ref_as_ptr,
    clippy::semicolon_outside_block,
    clippy::single_match_else,
    clippy::struct_field_names,
    clippy::wildcard_enum_match_arm,
    clippy::wildcard_imports,
    reason = "The declarations are documented in the generated public C header."
)]

use std::ffi::c_char;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::ptr;

use luminate_core::appearance_slot::*;
use luminate_core::capability::*;
use luminate_core::collection::{
    Collection, CollectionCategory, CollectionId, CollectionMember, OwnerIdentity,
};
use luminate_core::colour::Colour;
use luminate_core::control::{Selector, UnsupportedPolicy};
use luminate_core::device::{Device, DeviceId};
use luminate_core::effect::{Effect, EffectArguments};
use luminate_core::element::{Element, ElementGeometry, ElementKind};
use luminate_core::group::{Group, GroupMember};
use luminate_core::rgb::Rgb;
use luminate_core::scene::{Scene, SceneBinding, SceneCaptureMode, SceneId, SceneTargetState};
use luminate_core::state::*;
use luminate_core::surface::{Surface, SurfaceKind};
use luminate_core::target::TargetId;
use luminate_core::transition::{
    HueDirection, TransitionColourInterpolation, TransitionFunction, TransitionId,
    TransitionOptions, TransitionOutcome, TransitionStatus, TransitionTargetState,
};

use crate::CollectionOutcome;
use crate::ffi::{
    LuminateClient, LuminateEventSubscription, LuminateStatus, call_client, call_subscription,
    clear_last_error, client_ref, ffi_guard, out_ptr_mut, read_path, read_required_str,
    spawn_ffi_subscription, store_error, subscription_ref, write_subscription,
};
use crate::{Error, Event};

#[cfg(test)]
mod test_support;

/// A length-suffixed string.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LuminateStringView {
    pub data: *const c_char,
    pub len: usize,
}

/// Borrowed collection or scene owner identity.
pub struct LuminateOwnerIdentity;

/// Kind of owner identity.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    clippy::enum_variant_names,
    reason = "Fully prefixed Rust variants generate unambiguous C constants."
)]
pub(crate) enum LuminateOwnerKind {
    LuminateOwnerKindUid = 0,
    LuminateOwnerKindSid = 1,
    LuminateOwnerKindPrincipal = 2,
    LuminateOwnerKindInvalid = u32::MAX,
}

fn owner_ref(owner: *const LuminateOwnerIdentity) -> Option<&'static OwnerIdentity> {
    // SAFETY: the C contract requires a view returned by a collection or scene
    // owner accessor.
    unsafe { owner.cast::<OwnerIdentity>().as_ref() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_owner_identity_kind(
    owner: *const LuminateOwnerIdentity,
) -> LuminateOwnerKind {
    owner_ref(owner).map_or(
        LuminateOwnerKind::LuminateOwnerKindInvalid,
        |owner| match owner {
            OwnerIdentity::Uid(_) => LuminateOwnerKind::LuminateOwnerKindUid,
            OwnerIdentity::Sid(_) => LuminateOwnerKind::LuminateOwnerKindSid,
            OwnerIdentity::Principal(_) => LuminateOwnerKind::LuminateOwnerKindPrincipal,
        },
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_owner_identity_uid(
    owner: *const LuminateOwnerIdentity,
    out_uid: *mut u32,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(OwnerIdentity::Uid(uid)), Some(out_uid)) =
        (owner_ref(owner), unsafe { out_uid.as_mut() })
    else {
        return false;
    };
    *out_uid = *uid;
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_owner_identity_sid(
    owner: *const LuminateOwnerIdentity,
) -> LuminateStringView {
    owner_ref(owner).map_or_else(policy::null_view, |owner| match owner {
        OwnerIdentity::Sid(sid) => sv(sid),
        OwnerIdentity::Uid(_) | OwnerIdentity::Principal(_) => policy::null_view(),
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_owner_identity_principal_authority(
    owner: *const LuminateOwnerIdentity,
) -> LuminateStringView {
    owner_ref(owner).map_or_else(policy::null_view, |owner| match owner {
        OwnerIdentity::Principal(principal) => sv(principal.authority()),
        OwnerIdentity::Uid(_) | OwnerIdentity::Sid(_) => policy::null_view(),
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_owner_identity_principal_subject(
    owner: *const LuminateOwnerIdentity,
) -> LuminateStringView {
    owner_ref(owner).map_or_else(policy::null_view, |owner| match owner {
        OwnerIdentity::Principal(principal) => sv(principal.subject()),
        OwnerIdentity::Uid(_) | OwnerIdentity::Sid(_) => policy::null_view(),
    })
}

/// An RGB triplet.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LuminateRgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// A rectangle.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LuminateRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A single point.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LuminatePoint {
    pub x: f32,
    pub y: f32,
}

/// A matrix cell.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LuminateMatrixCell {
    pub row: u16,
    pub column: u16,
}

/// An unsigned 16-bit range.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LuminateU16Range {
    pub minimum: u16,
    pub maximum: u16,
    pub step: u16,
}

/// An unsigned 32-bit range.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LuminateU32Range {
    pub minimum: u32,
    pub maximum: u32,
    pub step: u32,
}

/// A luminate target.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LuminateTarget {
    pub device_id: *const c_char,
    pub surface_id: *const c_char,
    pub element_id: *const c_char,
    pub group_id: *const c_char,
}

/// Owned list of all devices, returned by `luminate_client_list_devices` or a
/// baseline subscribe call; free with `luminate_topology_snapshot_free`.
pub struct LuminateTopologySnapshot(pub(crate) Vec<Device>);

/// Owned list of withdrawn device identifiers, returned by
/// `luminate_client_list_withdrawn_devices`.
pub struct LuminateWithdrawnDeviceList(pub(crate) Vec<DeviceId>);

/// Owned single device, returned by `luminate_client_get_device`; free with
/// `luminate_device_snapshot_free`.
pub struct LuminateDeviceSnapshot(pub(crate) Device);

/// Owned device state, returned by `luminate_client_get_state`; free with
/// `luminate_state_snapshot_free`.
pub struct LuminateStateSnapshot(pub(crate) DeviceStateStatus);

/// Owned collection appearance state, returned by
/// `luminate_client_get_collection_state`; free with
/// `luminate_collection_state_snapshot_free`.
#[allow(
    dead_code,
    reason = "opaque C wrapper is read through layout-compatible pointer views"
)]
pub struct LuminateCollectionStateSnapshot(pub(crate) CollectionStateStatus);

/// Owned topology- or state-changed event, returned by
/// `luminate_event_subscription_next`; free with `luminate_event_free`.
pub struct LuminateEvent(pub(crate) Event);

/// Owned effect created by one of the `luminate_effect_create_*` constructors.
/// Free with `luminate_effect_free`.
pub struct LuminateEffect;

/// Borrowed effect within an owning snapshot or list root.
pub struct LuminateEffectView;

/// Surface shape.
pub type LuminateSurfaceKind = u32;
/// Element role within a surface.
pub type LuminateElementKind = u32;
/// Which field of an element's geometry union is populated.
pub type LuminateGeometryKind = u32;
/// How a group was created and is managed.
pub type LuminateGroupKind = u32;
/// What a group member refers to.
pub type LuminateGroupMemberKind = u32;
/// What a collection member refers to.
pub type LuminateCollectionMemberKind = u32;
/// Granularity at which a capability applies.
pub type LuminateCapabilityScope = u32;
/// How colour channels are encoded.
pub type LuminateColourEncoding = u32;
/// Identity of one colour channel.
pub type LuminateColourChannel = u32;
/// Which frame update styles a target accepts.
pub type LuminateFrameUpdateMode = u32;
/// How uploaded frames are committed.
pub type LuminateBufferingMode = u32;
/// Packed shared-memory pixel encoding.
pub type LuminateShmPixelFormat = u32;
/// Shared-memory pixel-buffer layout.
pub type LuminateShmFrameShapeKind = u32;
/// What persistence a device offers.
pub type LuminatePersistenceKind = u32;
/// Whether persistence is optional or mandatory.
pub type LuminatePersistenceRequirement = u32;
/// Whether device state can be read back.
pub type LuminateStateReadbackKind = u32;
/// How trustworthy a readback is.
pub type LuminateReadbackFidelity = u32;
/// Which effect-parameter payload is populated.
pub type LuminateEffectParameterKind = u32;
/// A supported effect motion direction.
pub type LuminateEffectDirection = u32;
/// What a power-domain reference points at.
pub type LuminatePowerDomainKind = u32;
/// A readable or observable state facet.
pub type LuminateStateFacetKind = u32;
/// Whether a device is currently reachable.
pub type LuminateReachability = u32;
/// A device's reconciliation lifecycle status.
pub type LuminateReconciliationStatus = u32;
/// How trustworthy an observation is.
pub type LuminateObservationConfidence = u32;
/// Where an observation came from.
pub type LuminateObservationSource = u32;
/// Whether an adopted baseline was accepted.
pub type LuminateAdoptionStatus = u32;
/// What a target view addresses.
pub type LuminateTargetKind = u32;
/// Which appearance payload is populated.
pub type LuminateAppearanceKind = u32;
/// Which effective-appearance payload is populated.
pub type LuminateEffectiveAppearanceKind = u32;
/// Whether a target is emitting light.
pub type LuminateEmissionState = u32;
/// Whether a target's physical power is on.
pub type LuminatePhysicalPowerState = u32;
/// Which payload an effect carries.
pub type LuminateEffectKind = u32;
/// Which payload an event carries.
pub type LuminateEventKind = u32;
/// A semantic operation evaluated by an access policy.
pub type LuminatePolicyOperation = u32;
/// Whether a matching policy rule allows or denies.
pub type LuminateRuleEffect = u32;
/// Whether appearance-slot mutations may omit values.
pub type LuminateAppearanceSlotUpdatePolicy = u32;

/// Owned list of every registered collection, returned by
/// `luminate_client_list_collections`; free with
/// `luminate_collection_list_free`.
pub struct LuminateCollectionList(pub(crate) Vec<Collection>);

/// Owned single collection, returned by `luminate_client_get_collection`;
/// free with `luminate_collection_snapshot_free`.
pub struct LuminateCollectionSnapshot(pub(crate) Collection);

/// Owned list of observable scenes.
pub struct LuminateSceneList(pub(crate) Vec<Scene>);

/// Owned single scene snapshot.
pub struct LuminateSceneSnapshot(pub(crate) Scene);

/// Owned transition status snapshot.
pub struct LuminateTransitionSnapshot(pub(crate) TransitionStatus);

/// Owned wear-safe all-off plan; free with `luminate_all_off_plan_free`.
pub struct LuminateAllOffPlan(crate::AllOffPlan);

/// Borrowed view of a device within an owning topology or device snapshot.
pub struct LuminateDevice;

/// Borrowed view of a surface within an owning device.
pub struct LuminateSurface;

/// Borrowed view of an element within an owning surface.
pub struct LuminateElement;

/// Borrowed view of a group within an owning device.
pub struct LuminateGroup;

/// Borrowed view of one member entry within an owning group.
pub struct LuminateGroupMember;

/// Borrowed view of the capabilities advertised by a device, surface,
/// element, or group.
pub struct LuminateCapabilitySet;

/// Borrowed view of appearance slots advertised by a surface.
pub struct LuminateAppearanceSlotsCapability;

/// Borrowed view of one appearance-slot descriptor.
pub struct LuminateAppearanceSlotDescriptor;

/// Borrowed view of appearance operations supported by a slot.
pub struct LuminateAppearanceCapability;

/// Borrowed view of one appearance-slot value.
pub struct LuminateAppearanceSlotValue;

/// Borrowed view of one colour capability within an owning capability set.
pub struct LuminateColourCapability;

/// Borrowed view of one channel within an owning colour capability.
pub struct LuminateColourChannelCapability;

/// Borrowed view of the frame-upload capability within an owning capability
/// set.
pub struct LuminateFrameUploadCapability;

/// Borrowed view of the shared-memory fast-path capability within an owning
/// frame-upload capability.
pub struct LuminateShmFrameCapability;

/// Borrowed view of the hardware-effects capability within an owning
/// capability set.
pub struct LuminateHardwareEffectsCapability;

/// Borrowed view of one hardware-driven effect a device advertises, within
/// an owning hardware-effects capability.
pub struct LuminateHardwareEffectDescriptor;

/// Borrowed view of one configurable parameter of a hardware effect
/// descriptor.
pub struct LuminateEffectParameter;

/// Borrowed view of one choice option within an owning choice-kind effect
/// parameter.
pub struct LuminateEffectChoice;

/// Borrowed view of one readable facet within an owning capability set's
/// state-readback capability.
pub struct LuminateReadableFacet;

/// Borrowed view of the physical-power capability within an owning
/// capability set.
pub struct LuminatePhysicalPowerCapability;

/// Borrowed view of the power domain reference within an owning capability
/// set.
pub struct LuminatePowerDomainRef;

/// Borrowed view of a device's state within an owning state snapshot.
pub struct LuminateState;

/// Borrowed view of one observed facet value within an owning state.
pub struct LuminateFacetObservation;

/// Borrowed view of an observed or adopted facet's value.
pub struct LuminateFacetValue;

/// Borrowed view of a colour value within an owning facet value.
pub struct LuminateColour;

/// Borrowed view of the target an observation or adoption record describes.
pub struct LuminateTargetView;

/// Borrowed view of one facet adoption record within an owning state.
pub struct LuminateAdoption;

/// Borrowed view of a collection within an owning
/// `LuminateCollectionList`/`LuminateCollectionSnapshot`.
pub struct LuminateCollection;

/// Borrowed view of one member entry within an owning collection: either a
/// concrete target or a nested collection reference.
pub struct LuminateCollectionMember;

/// Borrowed view of a scene.
pub struct LuminateScene;

/// Borrowed view of one scene binding.
pub struct LuminateSceneBinding;

pub(crate) fn sv(value: &str) -> LuminateStringView {
    LuminateStringView {
        data: value.as_ptr().cast(),
        len: value.len(),
    }
}

fn optional_sv(value: Option<&str>) -> LuminateStringView {
    value.map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        sv,
    )
}

fn mapped_sv<T>(value: &T, get: for<'a> fn(&'a T) -> &'a str) -> LuminateStringView {
    sv(get(value))
}

fn cast_ref<T, U>(value: &T) -> *const U {
    ptr::from_ref(value).cast()
}

unsafe fn native<'a, T, U>(value: *const T) -> Option<&'a U> {
    // SAFETY: callers guarantee that a non-null pointer is aligned, points to
    // a live value with native type `U`, and remains valid for the returned
    // borrow. The typed C accessors expose this contract in the public header.
    unsafe { value.cast::<U>().as_ref() }
}

unsafe fn native_mut<'a, T, U>(value: *mut T) -> Option<&'a mut U> {
    // SAFETY: callers guarantee that a non-null pointer is aligned, points to
    // a live, exclusively-borrowed value with native type `U`, and remains
    // valid for the returned borrow. The typed C accessors expose this
    // contract in the public header.
    unsafe { value.cast::<U>().as_mut() }
}

/// Mutably borrows the native value behind an opaque typed-C pointer.
/// Callers must guarantee no other reference to the same value is live for
/// the duration of the borrow, the same exclusivity contract an ordinary
/// `&mut` reference requires. C cannot enforce that exclusivity, so the caller
/// must uphold it.
macro_rules! native_mut {
    ($value:expr, $native:ty) => {{
        // SAFETY: the enclosing C accessor's pointer contract requires any
        // non-null opaque pointer to reference a live, exclusively-borrowed
        // value of this type.
        unsafe { native_mut::<_, $native>($value) }
    }};
}

/// Borrows the native value behind an opaque typed-C pointer.
///
/// Accessor macros and hand-written read accessors build on this helper. Their
/// bodies must preserve this module's panic-free invariant because they return
/// directly across the C boundary without [`ffi_guard`].
macro_rules! native_ref {
    ($value:expr, $native:ty) => {{
        // SAFETY: the enclosing C accessor's pointer contract requires any
        // non-null opaque pointer to reference a live value of this type.
        unsafe { native::<_, $native>($value) }
    }};
}

/// Returns the typed C ABI version implemented by this build; bump when the
/// accessor surface changes incompatibly.
///
/// The same constant supplies the ELF SONAME and generated C header. Packaging
/// metadata is checked against it by the ABI-version integration test.
#[unsafe(no_mangle)]
pub extern "C" fn luminate_c_abi_version() -> u32 {
    crate::LUMINATE_C_ABI_VERSION
}

/// Defines a null-safe free function for an owned root type. `$doc` becomes
/// the generated function's doc comment.
///
// cbindgen can't see through this macro, which is why every expansion below
// is hand-declared again in the `trailer` of `cbindgen.toml`. Keep the two
// in sync if this macro's signature changes.
macro_rules! null_safe_free {
    ($doc:literal, $fn:ident, $ty:ty) => {
        #[doc = $doc]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn(value: *mut $ty) {
            if !value.is_null() {
                // SAFETY: upheld by the enclosing function's documented C pointer contract.
                let _ = std::panic::catch_unwind(AssertUnwindSafe(|| unsafe {
                    drop(Box::from_raw(value))
                }));
            }
        }
    };
}

null_safe_free!(
    "Releases a `LuminateTopologySnapshot` returned by \
     `luminate_client_list_devices` or a baseline subscribe call. Null is a \
     no-op.",
    luminate_topology_snapshot_free,
    LuminateTopologySnapshot
);
null_safe_free!(
    "Releases a `LuminateWithdrawnDeviceList` returned by \
     `luminate_client_list_withdrawn_devices`. Null is a no-op.",
    luminate_withdrawn_device_list_free,
    LuminateWithdrawnDeviceList
);
null_safe_free!(
    "Releases a `LuminateDeviceSnapshot` returned by \
     `luminate_client_get_device`. Null is a no-op.",
    luminate_device_snapshot_free,
    LuminateDeviceSnapshot
);
null_safe_free!(
    "Releases a `LuminateStateSnapshot` returned by \
     `luminate_client_get_state`. Null is a no-op.",
    luminate_state_snapshot_free,
    LuminateStateSnapshot
);
null_safe_free!(
    "Releases a `LuminateCollectionStateSnapshot` returned by \
     `luminate_client_get_collection_state`. Null is a no-op.",
    luminate_collection_state_snapshot_free,
    LuminateCollectionStateSnapshot
);
null_safe_free!(
    "Releases a `LuminateEvent` returned by \
     `luminate_event_subscription_next`. Null is a no-op.",
    luminate_event_free,
    LuminateEvent
);
null_safe_free!(
    "Releases a `LuminateCollectionList` returned by \
     `luminate_client_list_collections`. Null is a no-op.",
    luminate_collection_list_free,
    LuminateCollectionList
);
null_safe_free!(
    "Releases a `LuminateCollectionSnapshot` returned by \
     `luminate_client_get_collection`. Null is a no-op.",
    luminate_collection_snapshot_free,
    LuminateCollectionSnapshot
);
null_safe_free!(
    "Releases a `LuminateSceneList`. Null is a no-op.",
    luminate_scene_list_free,
    LuminateSceneList
);
null_safe_free!(
    "Releases a `LuminateSceneSnapshot`. Null is a no-op.",
    luminate_scene_snapshot_free,
    LuminateSceneSnapshot
);
/// Releases a `LuminateEffect` created by one of the
/// `luminate_effect_create_*` constructors, or returned by
/// `luminate_facet_value_effect`. Null is a no-op.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_effect_free(value: *mut LuminateEffect) {
    if !value.is_null() {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let _ = std::panic::catch_unwind(|| unsafe {
            drop(Box::from_raw(value.cast::<Effect>()));
        });
    }
}

unsafe fn write_box<T>(
    out: *mut *mut T,
    value: T,
    what: &'static str,
) -> Result<(), LuminateStatus> {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let out = unsafe { out_ptr_mut(out, what) }?;
    *out = Box::into_raw(Box::new(value));
    Ok(())
}

/// Fetches the full device topology from the daemon and writes an owned
/// `LuminateTopologySnapshot` to `out`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_list_devices(
    client: *mut LuminateClient,
    out: *mut *mut LuminateTopologySnapshot,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        if out.is_null() {
            crate::ffi::set_last_error("topology snapshot output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let result = match call_client(client, |c| async move { c.list_devices().await }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(v) => {
                if let Err(e) =
                    // SAFETY: `out` was checked for null above; the caller guarantees it is writable.
                    unsafe {
                        write_box(out, LuminateTopologySnapshot(v), "topology snapshot")
                    }
                {
                    return e;
                }
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(e) => store_error(&e),
        }
    })
}

/// Fetches the identifiers of absent devices with retained state and writes an
/// owned `LuminateWithdrawnDeviceList` to `out`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_list_withdrawn_devices(
    client: *mut LuminateClient,
    out: *mut *mut LuminateWithdrawnDeviceList,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(value) => value,
            Err(error) => return error,
        };
        if out.is_null() {
            crate::ffi::set_last_error("withdrawn device list output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let result = match call_client(client, |client| async move {
            client.list_withdrawn_devices().await
        }) {
            Ok(value) => value,
            Err(error) => return error,
        };
        match result {
            Ok(value) => {
                // SAFETY: `out` was checked for null above; the caller guarantees it is writable.
                if let Err(error) = unsafe {
                    write_box(
                        out,
                        LuminateWithdrawnDeviceList(value),
                        "withdrawn device list",
                    )
                } {
                    return error;
                }
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(error) => store_error(&error),
        }
    })
}

/// Fetches a single device by id and writes an owned `LuminateDeviceSnapshot`
/// to `out`; returns not-found if no such device exists.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_get_device(
    client: *mut LuminateClient,
    id: *const c_char,
    out: *mut *mut LuminateDeviceSnapshot,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let id = match unsafe { read_required_str(id, "device_id") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        if out.is_null() {
            crate::ffi::set_last_error("device snapshot output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let requested = id.clone();
        let result = match call_client(client, move |c| async move {
            c.get_device(DeviceId::new(requested)).await
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(Some(v)) => {
                if let Err(e) =
                    // SAFETY: `out` was checked for null above; the caller guarantees it is writable.
                    unsafe { write_box(out, LuminateDeviceSnapshot(v), "device snapshot") }
                {
                    return e;
                }
                clear_last_error();
                LuminateStatus::Ok
            }
            Ok(None) => store_error(&Error::NotFound(format!("device '{id}'"))),
            Err(e) => store_error(&e),
        }
    })
}

/// Fetches a single device's state by id and writes an owned
/// `LuminateStateSnapshot` to `out`; returns not-found if no such device
/// exists.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_get_state(
    client: *mut LuminateClient,
    id: *const c_char,
    out: *mut *mut LuminateStateSnapshot,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let id = match unsafe { read_required_str(id, "device_id") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        if out.is_null() {
            crate::ffi::set_last_error("state snapshot output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let requested = id.clone();
        let result = match call_client(client, move |c| async move {
            c.get_state(DeviceId::new(requested)).await
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(Some(v)) => {
                if let Err(e) =
                    // SAFETY: `out` was checked for null above; the caller guarantees it is writable.
                    unsafe { write_box(out, LuminateStateSnapshot(v), "state snapshot") }
                {
                    return e;
                }
                clear_last_error();
                LuminateStatus::Ok
            }
            Ok(None) => store_error(&Error::NotFound(format!("device '{id}'"))),
            Err(e) => store_error(&e),
        }
    })
}

/// Fetches one collection's aggregate appearance state by id and writes an
/// owned `LuminateCollectionStateSnapshot` to `out`; returns not-found if no
/// such collection exists.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_get_collection_state(
    client: *mut LuminateClient,
    id: *const c_char,
    out: *mut *mut LuminateCollectionStateSnapshot,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let id = match unsafe { read_required_str(id, "collection_id") } {
            Ok(v) => v.to_owned(),
            Err(e) => return e,
        };
        if out.is_null() {
            crate::ffi::set_last_error("collection state snapshot output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let requested = id.clone();
        let result = match call_client(client, move |c| async move {
            c.get_collection_state(CollectionId::new(requested)).await
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(Some(value)) => {
                // SAFETY: `out` was checked for null above; the caller
                // guarantees it is writable.
                if let Err(error) = unsafe {
                    write_box(
                        out,
                        LuminateCollectionStateSnapshot(value),
                        "collection state snapshot",
                    )
                } {
                    return error;
                }
                clear_last_error();
                LuminateStatus::Ok
            }
            Ok(None) => store_error(&Error::NotFound(format!("collection '{id}'"))),
            Err(error) => store_error(&error),
        }
    })
}

/// Blocks until the next event arrives on the subscription and writes an
/// owned `LuminateEvent` to `out`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_event_subscription_next(
    subscription: *mut LuminateEventSubscription,
    out: *mut *mut LuminateEvent,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let subscription = match unsafe { subscription_ref(subscription) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        if out.is_null() {
            crate::ffi::set_last_error("event output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let result = match call_subscription(subscription) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok(v) => {
                // SAFETY: upheld by the enclosing function's documented C pointer contract.
                if let Err(e) = unsafe { write_box(out, LuminateEvent(v), "event") } {
                    return e;
                }
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(e) => store_error(&e),
        }
    })
}

unsafe fn baseline(
    client: *mut LuminateClient,
    path: Option<PathBuf>,
    out_sub: *mut *mut LuminateEventSubscription,
    out: *mut *mut LuminateTopologySnapshot,
) -> LuminateStatus {
    ffi_guard(|| {
        // SAFETY: upheld by the enclosing function's documented C pointer contract.
        let client = match unsafe { client_ref(client) } {
            Ok(v) => v,
            Err(e) => return e,
        };
        if out_sub.is_null() || out.is_null() {
            crate::ffi::set_last_error("subscription or topology output pointer is null");
            return LuminateStatus::NullPointer;
        }
        let result = match call_client(client, move |c| async move {
            match path {
                Some(path) => c.subscribe_with_baseline_path(path).await,
                None => c.subscribe_with_baseline().await,
            }
        }) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match result {
            Ok((subscription, devices)) => {
                let subscription = spawn_ffi_subscription(client, subscription);
                // SAFETY: upheld by the enclosing function's documented C pointer contract.
                unsafe {
                    write_subscription(out_sub, subscription);
                    *out = Box::into_raw(Box::new(LuminateTopologySnapshot(devices)));
                }
                clear_last_error();
                LuminateStatus::Ok
            }
            Err(e) => store_error(&e),
        }
    })
}

/// Subscribes to events using the client's default event socket path and
/// writes the current topology as a baseline `LuminateTopologySnapshot` to
/// `out`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_subscribe_with_baseline(
    client: *mut LuminateClient,
    sub: *mut *mut LuminateEventSubscription,
    out: *mut *mut LuminateTopologySnapshot,
) -> LuminateStatus {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { baseline(client, None, sub, out) }
}

/// Subscribes to events using an explicit event socket `path` and writes the
/// current topology as a baseline `LuminateTopologySnapshot` to `out`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_client_subscribe_with_baseline_path(
    client: *mut LuminateClient,
    path: *const c_char,
    sub: *mut *mut LuminateEventSubscription,
    out: *mut *mut LuminateTopologySnapshot,
) -> LuminateStatus {
    if path.is_null() {
        crate::ffi::set_last_error("event path pointer is null");
        return LuminateStatus::NullPointer;
    }
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let path = match unsafe { read_path(path) } {
        Ok(v) => PathBuf::from(v),
        Err(e) => return e,
    };
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { baseline(client, Some(path), sub, out) }
}

/// Number of devices in the topology snapshot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_topology_snapshot_device_count(
    v: *const LuminateTopologySnapshot,
) -> usize {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(0, |v| v.0.len())
}

/// Number of identifiers in a withdrawn device list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_withdrawn_device_list_count(
    value: *const LuminateWithdrawnDeviceList,
) -> usize {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { value.as_ref() }.map_or(0, |value| value.0.len())
}

/// Borrowed identifier at `index`, or an empty view if out of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_withdrawn_device_list_at(
    value: *const LuminateWithdrawnDeviceList,
    index: usize,
) -> LuminateStringView {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { value.as_ref() }
        .and_then(|value| value.0.get(index))
        .map_or_else(policy::null_view, |device| sv(device.as_str()))
}

/// Borrowed device at `index`, or null if out of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_topology_snapshot_device_at(
    v: *const LuminateTopologySnapshot,
    i: usize,
) -> *const LuminateDevice {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }
        .and_then(|v| v.0.get(i))
        .map_or(ptr::null(), cast_ref)
}

/// The borrowed device held by this snapshot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_device_snapshot_device(
    v: *const LuminateDeviceSnapshot,
) -> *const LuminateDevice {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(ptr::null(), |v| cast_ref(&v.0))
}

/// The borrowed state held by this snapshot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_state_snapshot_state(
    v: *const LuminateStateSnapshot,
) -> *const LuminateState {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    unsafe { v.as_ref() }.map_or(ptr::null(), |v| cast_ref(&v.0))
}

/// Defines a string accessor function. `$doc` becomes the generated
/// function's doc comment.
macro_rules! str_accessor {
    ($doc:literal, $fn:ident, $opaque:ty, $native:ty, $body:expr) => {
        #[doc = $doc]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn(v: *const $opaque) -> LuminateStringView {
            native_ref!(v, $native).map_or(
                LuminateStringView {
                    data: ptr::null(),
                    len: 0,
                },
                |v| mapped_sv(v, $body),
            )
        }
    };
}

str_accessor!(
    "The device's stable identifier.",
    luminate_device_id,
    LuminateDevice,
    Device,
    |v: &Device| v.id.as_str()
);
str_accessor!(
    "The device's human-readable display name.",
    luminate_device_name,
    LuminateDevice,
    Device,
    |v: &Device| v.name.as_str()
);
/// The device's vendor name, or an absent view if not set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_device_vendor(v: *const LuminateDevice) -> LuminateStringView {
    native_ref!(v, Device).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| optional_sv(v.vendor.as_deref()),
    )
}

/// The device's model name, or an absent view if not set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_device_model(v: *const LuminateDevice) -> LuminateStringView {
    native_ref!(v, Device).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| optional_sv(v.model.as_deref()),
    )
}

/// The daemon-configured provider instance responsible for the device, or an
/// absent view when ownership is unknown.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_device_provider_instance(
    v: *const LuminateDevice,
) -> LuminateStringView {
    native_ref!(v, Device).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| optional_sv(v.provider_instance.as_deref()),
    )
}

/// The device's category, or an absent view if not set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_device_category(v: *const LuminateDevice) -> LuminateStringView {
    native_ref!(v, Device).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| optional_sv(v.category.as_ref().map(|x| x.as_str())),
    )
}

/// Whether the device is physically attached to this host.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_device_host_attached(v: *const LuminateDevice) -> bool {
    native_ref!(v, Device).is_some_and(|v| v.host_attached)
}

/// Defines the `count` half of a `count`/`at` accessor pair over a `Vec<_>`
/// field, shared by [`vec_accessors`] and [`string_vec_accessors`]. `$doc`
/// becomes the generated function's doc comment.
macro_rules! vec_count_accessor {
    ($doc:literal, $count:ident, $opaque:ty, $native:ty, $field:ident) => {
        #[doc = $doc]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $count(v: *const $opaque) -> usize {
            native_ref!(v, $native).map_or(0, |v| v.$field.len())
        }
    };
}

/// Defines a `count`/`at` accessor pair over a `Vec<T>` field, where `at`
/// returns a borrowed pointer. `$count_doc`/`$at_doc` become the generated
/// functions' doc comments.
macro_rules! vec_accessors {
    ($count_doc:literal,$at_doc:literal,$count:ident,$at:ident,$opaque:ty,$native:ty,$item:ty,$field:ident) => {
        vec_count_accessor!($count_doc, $count, $opaque, $native, $field);
        #[doc = $at_doc]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $at(v: *const $opaque, i: usize) -> *const $item {
            native_ref!(v, $native)
                .and_then(|v| v.$field.get(i))
                .map_or(ptr::null(), cast_ref)
        }
    };
}

/// Defines a `count`/`at` accessor pair over a `Vec<String>` field, where
/// `at` returns a `LuminateStringView`. `$count_doc`/`$at_doc` become the
/// generated functions' doc comments.
macro_rules! string_vec_accessors {
    ($count_doc:literal,$at_doc:literal,$count:ident,$at:ident,$opaque:ty,$native:ty,$field:ident) => {
        vec_count_accessor!($count_doc, $count, $opaque, $native, $field);
        #[doc = $at_doc]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $at(v: *const $opaque, i: usize) -> LuminateStringView {
            native_ref!(v, $native)
                .and_then(|v| v.$field.get(i))
                .map_or(
                    LuminateStringView {
                        data: ptr::null(),
                        len: 0,
                    },
                    |v| sv(v),
                )
        }
    };
}

fn effect_ref(value: *const LuminateEffect) -> Option<&'static Effect> {
    // SAFETY: callers uphold the documented owned-effect pointer contract.
    unsafe { value.cast::<Effect>().as_ref() }
}

fn effect_view_ref(value: *const LuminateEffectView) -> Option<&'static Effect> {
    // SAFETY: callers uphold the documented borrowed-effect-view pointer contract.
    unsafe { value.cast::<Effect>().as_ref() }
}

pub(crate) mod access_administration;
mod all_off;
pub(crate) mod appearance_slots;
mod capabilities;
pub(crate) mod collections;
mod colour_input;
pub(crate) mod effects;
pub(crate) mod frame;
pub(crate) mod management;
mod models;
pub(crate) mod policy;
pub(crate) mod scenes;
pub(crate) mod selector;
pub(crate) mod setup;
pub(crate) mod shm;
mod state;
pub(crate) mod transitions;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
