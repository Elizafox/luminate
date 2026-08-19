// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

vec_accessors!(
    "Number of surfaces on a device.",
    "Borrowed surface at `index`, or null if out of range.",
    luminate_device_surface_count,
    luminate_device_surface_at,
    LuminateDevice,
    Device,
    LuminateSurface,
    surfaces
);
vec_accessors!(
    "Number of groups a device belongs to.",
    "Borrowed group at `index`, or null if out of range.",
    luminate_device_group_count,
    luminate_device_group_at,
    LuminateDevice,
    Device,
    LuminateGroup,
    groups
);
string_vec_accessors!(
    "Number of physical-form tags attached to the device.",
    "Physical-form tag at `index`, or an absent view if out of range.",
    luminate_device_physical_tag_count,
    luminate_device_physical_tag_at,
    LuminateDevice,
    Device,
    physical_tags
);
string_vec_accessors!(
    "Number of informational notes attached to the device.",
    "Note text at `index`, or an absent view if out of range.",
    luminate_device_note_count,
    luminate_device_note_at,
    LuminateDevice,
    Device,
    notes
);
string_vec_accessors!(
    "Number of warnings attached to the device.",
    "Warning text at `index`, or an absent view if out of range.",
    luminate_device_warning_count,
    luminate_device_warning_at,
    LuminateDevice,
    Device,
    warnings
);
/// Borrowed capability set for the device.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_device_capabilities(
    v: *const LuminateDevice,
) -> *const LuminateCapabilitySet {
    native_ref!(v, Device).map_or(ptr::null(), |v| cast_ref(&v.capabilities))
}

str_accessor!(
    "The surface's identifier, unique within its owning device.",
    luminate_surface_id,
    LuminateSurface,
    Surface,
    |v: &Surface| v.id.as_str()
);
str_accessor!(
    "The surface's human-readable display name.",
    luminate_surface_name,
    LuminateSurface,
    Surface,
    |v: &Surface| v.name.as_str()
);
vec_accessors!(
    "Number of elements on this surface.",
    "Borrowed element at `index`, or null if out of range.",
    luminate_surface_element_count,
    luminate_surface_element_at,
    LuminateSurface,
    Surface,
    LuminateElement,
    elements
);
string_vec_accessors!(
    "Number of physical-form tags attached to the surface.",
    "Physical-form tag at `index`, or an absent view if out of range.",
    luminate_surface_physical_tag_count,
    luminate_surface_physical_tag_at,
    LuminateSurface,
    Surface,
    physical_tags
);
string_vec_accessors!(
    "Number of informational notes attached to the surface.",
    "Note text at `index`, or an absent view if out of range.",
    luminate_surface_note_count,
    luminate_surface_note_at,
    LuminateSurface,
    Surface,
    notes
);
string_vec_accessors!(
    "Number of warnings attached to the surface.",
    "Warning text at `index`, or an absent view if out of range.",
    luminate_surface_warning_count,
    luminate_surface_warning_at,
    LuminateSurface,
    Surface,
    warnings
);
/// Borrowed capability set for the surface.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_surface_capabilities(
    v: *const LuminateSurface,
) -> *const LuminateCapabilitySet {
    native_ref!(v, Surface).map_or(ptr::null(), |v| cast_ref(&v.capabilities))
}

/// The surface's shape; one of the `LUMINATE_SURFACE_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_surface_kind(v: *const LuminateSurface) -> LuminateSurfaceKind {
    native_ref!(v, Surface).map_or(u32::MAX, |v| match v.kind {
        SurfaceKind::Opaque => 0,
        SurfaceKind::Zone => 1,
        SurfaceKind::Linear { .. } => 2,
        SurfaceKind::Sparse2d { .. } => 3,
        SurfaceKind::Matrix { .. } => 4,
    })
}

/// Length of a linear surface, or 0 unless the surface kind is linear.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_surface_linear_length(
    v: *const LuminateSurface,
    out_length: *mut f32,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(length), Some(out_length)) = (
        native_ref!(v, Surface).and_then(|v| match v.kind {
            SurfaceKind::Linear { length } => Some(length),
            _ => None,
        }),
        unsafe { out_length.as_mut() },
    ) else {
        return false;
    };
    *out_length = length;
    true
}

/// Width/height of a sparse-2D surface, or zeroed unless the surface kind is
/// sparse-2d.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_surface_sparse_size(
    v: *const LuminateSurface,
    out_size: *mut LuminatePoint,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some((width, height)), Some(out_size)) = (
        native_ref!(v, Surface).and_then(|v| match v.kind {
            SurfaceKind::Sparse2d { width, height } => Some((width, height)),
            _ => None,
        }),
        unsafe { out_size.as_mut() },
    ) else {
        return false;
    };
    *out_size = LuminatePoint {
        x: width,
        y: height,
    };
    true
}

/// Row/column dimensions of a matrix surface, or zeroed unless the surface
/// kind is matrix.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_surface_matrix_size(
    v: *const LuminateSurface,
    out_size: *mut LuminateMatrixCell,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some((rows, cols)), Some(out_size)) = (
        native_ref!(v, Surface).and_then(|v| match v.kind {
            SurfaceKind::Matrix { rows, cols } => Some((rows, cols)),
            _ => None,
        }),
        unsafe { out_size.as_mut() },
    ) else {
        return false;
    };
    *out_size = LuminateMatrixCell {
        row: rows,
        column: cols,
    };
    true
}

str_accessor!(
    "The element's identifier, unique within its owning surface.",
    luminate_element_id,
    LuminateElement,
    Element,
    |v: &Element| v.id.as_str()
);
/// The element's human-readable display name, or an absent view if not set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_element_name(v: *const LuminateElement) -> LuminateStringView {
    native_ref!(v, Element).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| optional_sv(v.name.as_deref()),
    )
}

/// The element's role within its surface; one of the `LUMINATE_ELEMENT_*`
/// values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_element_kind(v: *const LuminateElement) -> LuminateElementKind {
    native_ref!(v, Element).map_or(u32::MAX, |v| match v.kind {
        ElementKind::Key => 0,
        ElementKind::Led => 1,
        ElementKind::Zone => 2,
        ElementKind::Logo => 3,
        ElementKind::RingSegment => 4,
    })
}

/// Which geometry field is populated; one of the `LUMINATE_GEOMETRY_*`
/// values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_element_geometry_kind(
    v: *const LuminateElement,
) -> LuminateGeometryKind {
    native_ref!(v, Element).map_or(u32::MAX, |v| match v.geometry {
        None => 0,
        Some(ElementGeometry::Rect { .. }) => 1,
        Some(ElementGeometry::Point { .. }) => 2,
        Some(ElementGeometry::Linear { .. }) => 3,
        Some(ElementGeometry::MatrixCell { .. }) => 4,
    })
}

/// The element's rectangle geometry, or zeroed unless geometry kind is rect.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_element_geometry_rect(
    v: *const LuminateElement,
    out_rect: *mut LuminateRect,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some((x, y, width, height)), Some(out_rect)) = (
        native_ref!(v, Element).and_then(|v| match v.geometry {
            Some(ElementGeometry::Rect { x, y, w, h }) => Some((x, y, w, h)),
            _ => None,
        }),
        unsafe { out_rect.as_mut() },
    ) else {
        return false;
    };
    *out_rect = LuminateRect {
        x,
        y,
        width,
        height,
    };
    true
}

/// The element's point geometry, or zeroed unless geometry kind is point.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_element_geometry_point(
    v: *const LuminateElement,
    out_point: *mut LuminatePoint,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some((x, y)), Some(out_point)) = (
        native_ref!(v, Element).and_then(|v| match v.geometry {
            Some(ElementGeometry::Point { x, y }) => Some((x, y)),
            _ => None,
        }),
        unsafe { out_point.as_mut() },
    ) else {
        return false;
    };
    *out_point = LuminatePoint { x, y };
    true
}

/// The element's linear position, or 0 unless geometry kind is linear.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_element_geometry_linear(
    v: *const LuminateElement,
    out_position: *mut f32,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some(position), Some(out_position)) = (
        native_ref!(v, Element).and_then(|v| match v.geometry {
            Some(ElementGeometry::Linear { position }) => Some(position),
            _ => None,
        }),
        unsafe { out_position.as_mut() },
    ) else {
        return false;
    };
    *out_position = position;
    true
}

/// The element's matrix cell, or zeroed unless geometry kind is matrix cell.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_element_geometry_matrix_cell(
    v: *const LuminateElement,
    out_cell: *mut LuminateMatrixCell,
) -> bool {
    // SAFETY: upheld by the enclosing function's documented C pointer contract.
    let (Some((row, column)), Some(out_cell)) = (
        native_ref!(v, Element).and_then(|v| match v.geometry {
            Some(ElementGeometry::MatrixCell { row, col }) => Some((row, col)),
            _ => None,
        }),
        unsafe { out_cell.as_mut() },
    ) else {
        return false;
    };
    *out_cell = LuminateMatrixCell { row, column };
    true
}

/// Borrowed capability set for the element.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_element_capabilities(
    v: *const LuminateElement,
) -> *const LuminateCapabilitySet {
    native_ref!(v, Element).map_or(ptr::null(), |v| cast_ref(&v.capabilities))
}

string_vec_accessors!(
    "Number of physical-form tags attached to the element.",
    "Physical-form tag at `index`, or an absent view if out of range.",
    luminate_element_physical_tag_count,
    luminate_element_physical_tag_at,
    LuminateElement,
    Element,
    physical_tags
);
string_vec_accessors!(
    "Number of informational notes attached to the element.",
    "Note text at `index`, or an absent view if out of range.",
    luminate_element_note_count,
    luminate_element_note_at,
    LuminateElement,
    Element,
    notes
);
string_vec_accessors!(
    "Number of warnings attached to the element.",
    "Warning text at `index`, or an absent view if out of range.",
    luminate_element_warning_count,
    luminate_element_warning_at,
    LuminateElement,
    Element,
    warnings
);

str_accessor!(
    "The group's identifier, unique within its owning device.",
    luminate_group_id,
    LuminateGroup,
    Group,
    |v: &Group| v.id.as_str()
);
str_accessor!(
    "The group's human-readable display name.",
    luminate_group_name,
    LuminateGroup,
    Group,
    |v: &Group| v.name.as_str()
);
/// The group's description, or an absent view if not set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_group_description(v: *const LuminateGroup) -> LuminateStringView {
    native_ref!(v, Group).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| optional_sv(v.description.as_deref()),
    )
}

/// How the group was created and is managed; one of the `LUMINATE_GROUP_*`
/// values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_group_kind(v: *const LuminateGroup) -> LuminateGroupKind {
    native_ref!(v, Group).map_or(u32::MAX, |v| v.kind as u32)
}

vec_accessors!(
    "Number of members in the group.",
    "Borrowed member at `index`, or null if out of range.",
    luminate_group_member_count,
    luminate_group_member_at,
    LuminateGroup,
    Group,
    LuminateGroupMember,
    members
);
/// Borrowed capability set for the group.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_group_capabilities(
    v: *const LuminateGroup,
) -> *const LuminateCapabilitySet {
    native_ref!(v, Group).map_or(ptr::null(), |v| cast_ref(&v.capabilities))
}

string_vec_accessors!(
    "Number of informational notes attached to the group.",
    "Note text at `index`, or an absent view if out of range.",
    luminate_group_note_count,
    luminate_group_note_at,
    LuminateGroup,
    Group,
    notes
);
string_vec_accessors!(
    "Number of warnings attached to the group.",
    "Warning text at `index`, or an absent view if out of range.",
    luminate_group_warning_count,
    luminate_group_warning_at,
    LuminateGroup,
    Group,
    warnings
);
/// What this member refers to; one of the `LUMINATE_GROUP_MEMBER_*` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_group_member_kind(
    v: *const LuminateGroupMember,
) -> LuminateGroupMemberKind {
    native_ref!(v, GroupMember).map_or(u32::MAX, |v| match v {
        GroupMember::Surface(_) => 0,
        GroupMember::Element { .. } => 1,
        GroupMember::Group(_) => 2,
    })
}

/// The referenced surface's id, or an absent view unless this member is a
/// surface or element.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_group_member_surface_id(
    v: *const LuminateGroupMember,
) -> LuminateStringView {
    native_ref!(v, GroupMember).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| match v {
            GroupMember::Surface(x) | GroupMember::Element { surface: x, .. } => sv(x.as_str()),
            GroupMember::Group(_) => optional_sv(None),
        },
    )
}

/// The referenced element's id, or an absent view unless this member is an
/// element.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_group_member_element_id(
    v: *const LuminateGroupMember,
) -> LuminateStringView {
    native_ref!(v, GroupMember).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| match v {
            GroupMember::Element { element, .. } => sv(element.as_str()),
            _ => optional_sv(None),
        },
    )
}

/// The referenced group's id, or an absent view unless this member is a
/// group.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luminate_group_member_group_id(
    v: *const LuminateGroupMember,
) -> LuminateStringView {
    native_ref!(v, GroupMember).map_or(
        LuminateStringView {
            data: ptr::null(),
            len: 0,
        },
        |v| match v {
            GroupMember::Group(x) => sv(x.as_str()),
            _ => optional_sv(None),
        },
    )
}

#[cfg(test)]
#[path = "models_tests.rs"]
mod tests;
