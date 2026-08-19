// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;
use crate::ffi::{diagnostics::luminate_last_error_message, set_last_error};
use luminate_core::element::ElementId;
use luminate_core::group::{GroupId, GroupKind};
use luminate_core::surface::SurfaceId;

fn bytes(view: LuminateStringView) -> Option<&'static [u8]> {
    if view.data.is_null() {
        None
    } else {
        // SAFETY: test inputs remain alive while their borrowed views are inspected.
        Some(unsafe { std::slice::from_raw_parts(view.data.cast(), view.len) })
    }
}

fn assert_f32_eq(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < f32::EPSILON);
}

fn element(kind: ElementKind, geometry: Option<ElementGeometry>) -> Element {
    Element {
        id: ElementId::new("escape"),
        name: Some("Escape".to_owned()),
        kind,
        geometry,
        physical_tags: vec!["shape:keycap".to_owned(), "position:escape".to_owned()],
        capabilities: CapabilitySet::default(),
        notes: vec!["element note".to_owned()],
        warnings: vec!["element warning".to_owned()],
    }
}

fn surface(kind: SurfaceKind, elements: Vec<Element>) -> Surface {
    Surface {
        id: SurfaceId::new("keys"),
        name: "Keys".to_owned(),
        kind,
        physical_tags: vec!["layout:grid".to_owned(), "position:front".to_owned()],
        elements,
        capabilities: CapabilitySet::default(),
        notes: vec!["surface note".to_owned()],
        warnings: vec!["surface warning".to_owned()],
    }
}

fn group(members: Vec<GroupMember>) -> Group {
    Group {
        id: GroupId::new("gaming"),
        name: "Gaming".to_owned(),
        description: Some("Gaming controls".to_owned()),
        kind: GroupKind::User,
        members,
        capabilities: CapabilitySet::default(),
        notes: vec!["group note".to_owned()],
        warnings: vec!["group warning".to_owned()],
    }
}

#[test]
fn surface_accessors_cover_shapes_and_nested_values() {
    let element = element(ElementKind::Key, None);
    let surfaces = [
        surface(SurfaceKind::Opaque, vec![element]),
        surface(SurfaceKind::Zone, Vec::new()),
        surface(SurfaceKind::Linear { length: 3.5 }, Vec::new()),
        surface(
            SurfaceKind::Sparse2d {
                width: 4.0,
                height: 2.0,
            },
            Vec::new(),
        ),
        surface(SurfaceKind::Matrix { rows: 6, cols: 22 }, Vec::new()),
    ];

    for (surface, expected_kind) in surfaces.iter().zip(0_u32..) {
        let ptr = ptr::from_ref(surface).cast();
        // SAFETY: `ptr` refers to a live `Surface` for every accessor call.
        assert_eq!(unsafe { luminate_surface_kind(ptr) }, expected_kind);
    }

    let opaque: *const LuminateSurface = ptr::from_ref(&surfaces[0]).cast();
    // SAFETY: `opaque` refers to a live `Surface` for every accessor call.
    unsafe {
        assert_eq!(bytes(luminate_surface_id(opaque)), Some(b"keys".as_slice()));
        assert_eq!(
            bytes(luminate_surface_name(opaque)),
            Some(b"Keys".as_slice())
        );
        assert_eq!(luminate_surface_element_count(opaque), 1);
        assert!(!luminate_surface_element_at(opaque, 0).is_null());
        assert!(luminate_surface_element_at(opaque, 1).is_null());
        assert_eq!(luminate_surface_physical_tag_count(opaque), 2);
        assert_eq!(
            bytes(luminate_surface_physical_tag_at(opaque, 0)),
            Some(b"layout:grid".as_slice())
        );
        assert_eq!(
            bytes(luminate_surface_physical_tag_at(opaque, 1)),
            Some(b"position:front".as_slice())
        );
        assert_eq!(bytes(luminate_surface_physical_tag_at(opaque, 2)), None);
        assert_eq!(luminate_surface_note_count(opaque), 1);
        assert_eq!(
            bytes(luminate_surface_note_at(opaque, 0)),
            Some(b"surface note".as_slice())
        );
        assert_eq!(luminate_surface_warning_count(opaque), 1);
        assert_eq!(
            bytes(luminate_surface_warning_at(opaque, 0)),
            Some(b"surface warning".as_slice())
        );
        assert!(!luminate_surface_capabilities(opaque).is_null());
        let mut unchanged = 9.0;
        assert!(!luminate_surface_linear_length(opaque, &raw mut unchanged));
        assert_f32_eq(unchanged, 9.0);
    }

    let linear: *const LuminateSurface = ptr::from_ref(&surfaces[2]).cast();
    let sparse: *const LuminateSurface = ptr::from_ref(&surfaces[3]).cast();
    let matrix: *const LuminateSurface = ptr::from_ref(&surfaces[4]).cast();
    // SAFETY: each pointer refers to its corresponding live `Surface`.
    unsafe {
        let mut length = 0.0;
        assert!(luminate_surface_linear_length(linear, &raw mut length));
        assert_f32_eq(length, 3.5);
        let mut sparse_size = LuminatePoint { x: 0.0, y: 0.0 };
        assert!(luminate_surface_sparse_size(sparse, &raw mut sparse_size));
        assert_eq!((sparse_size.x, sparse_size.y), (4.0, 2.0));
        let mut matrix_size = LuminateMatrixCell { row: 0, column: 0 };
        assert!(luminate_surface_matrix_size(matrix, &raw mut matrix_size));
        assert_eq!((matrix_size.row, matrix_size.column), (6, 22));
    }
}

#[test]
fn element_accessors_cover_kinds_and_geometries() {
    let geometries = [
        None,
        Some(ElementGeometry::Rect {
            x: 0.1,
            y: 0.2,
            w: 0.3,
            h: 0.4,
        }),
        Some(ElementGeometry::Point { x: 0.5, y: 0.6 }),
        Some(ElementGeometry::Linear { position: 0.7 }),
        Some(ElementGeometry::MatrixCell { row: 2, col: 3 }),
    ];
    let kinds = [
        ElementKind::Key,
        ElementKind::Led,
        ElementKind::Zone,
        ElementKind::Logo,
        ElementKind::RingSegment,
    ];
    let elements: Vec<_> = kinds
        .into_iter()
        .zip(geometries)
        .map(|(kind, geometry)| element(kind, geometry))
        .collect();

    for (expected_kind, element) in (0_u32..).zip(&elements) {
        let ptr = ptr::from_ref(element).cast();
        // SAFETY: `ptr` refers to a live `Element` for every accessor call.
        unsafe {
            assert_eq!(luminate_element_kind(ptr), expected_kind);
            assert_eq!(luminate_element_geometry_kind(ptr), expected_kind);
        }
    }

    let rect: *const LuminateElement = ptr::from_ref(&elements[1]).cast();
    let point: *const LuminateElement = ptr::from_ref(&elements[2]).cast();
    let linear: *const LuminateElement = ptr::from_ref(&elements[3]).cast();
    let matrix: *const LuminateElement = ptr::from_ref(&elements[4]).cast();
    // SAFETY: each pointer refers to its corresponding live `Element`.
    unsafe {
        assert_eq!(bytes(luminate_element_id(rect)), Some(b"escape".as_slice()));
        assert_eq!(
            bytes(luminate_element_name(rect)),
            Some(b"Escape".as_slice())
        );
        let mut value = LuminateRect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        };
        assert!(luminate_element_geometry_rect(rect, &raw mut value));
        assert_eq!(
            (value.x, value.y, value.width, value.height),
            (0.1, 0.2, 0.3, 0.4)
        );
        let mut point_value = LuminatePoint { x: 0.0, y: 0.0 };
        assert!(luminate_element_geometry_point(point, &raw mut point_value));
        assert_eq!((point_value.x, point_value.y), (0.5, 0.6));
        let mut position = 0.0;
        assert!(luminate_element_geometry_linear(linear, &raw mut position));
        assert_f32_eq(position, 0.7);
        let mut cell = LuminateMatrixCell { row: 0, column: 0 };
        assert!(luminate_element_geometry_matrix_cell(matrix, &raw mut cell));
        assert_eq!((cell.row, cell.column), (2, 3));
        assert!(!luminate_element_capabilities(rect).is_null());
        assert_eq!(luminate_element_physical_tag_count(rect), 2);
        assert_eq!(
            bytes(luminate_element_physical_tag_at(rect, 0)),
            Some(b"shape:keycap".as_slice())
        );
        assert_eq!(
            bytes(luminate_element_physical_tag_at(rect, 1)),
            Some(b"position:escape".as_slice())
        );
        assert_eq!(bytes(luminate_element_physical_tag_at(rect, 2)), None);
        assert_eq!(luminate_element_note_count(rect), 1);
        assert_eq!(
            bytes(luminate_element_note_at(rect, 0)),
            Some(b"element note".as_slice())
        );
        assert_eq!(luminate_element_warning_count(rect), 1);
        assert_eq!(
            bytes(luminate_element_warning_at(rect, 0)),
            Some(b"element warning".as_slice())
        );
    }
}

#[test]
fn group_accessors_cover_member_variants() {
    let group = group(vec![
        GroupMember::Surface(SurfaceId::new("keys")),
        GroupMember::Element {
            surface: SurfaceId::new("keys"),
            element: ElementId::new("escape"),
        },
        GroupMember::Group(GroupId::new("nested")),
    ]);
    let ptr: *const LuminateGroup = ptr::from_ref(&group).cast();

    // SAFETY: `ptr` and each returned member pointer borrow the live `group`.
    unsafe {
        assert_eq!(bytes(luminate_group_id(ptr)), Some(b"gaming".as_slice()));
        assert_eq!(bytes(luminate_group_name(ptr)), Some(b"Gaming".as_slice()));
        assert_eq!(
            bytes(luminate_group_description(ptr)),
            Some(b"Gaming controls".as_slice())
        );
        assert_eq!(luminate_group_kind(ptr), GroupKind::User as u32);
        assert_eq!(luminate_group_member_count(ptr), 3);
        assert!(luminate_group_member_at(ptr, 3).is_null());
        assert!(!luminate_group_capabilities(ptr).is_null());
        assert_eq!(luminate_group_note_count(ptr), 1);
        assert_eq!(
            bytes(luminate_group_note_at(ptr, 0)),
            Some(b"group note".as_slice())
        );
        assert_eq!(luminate_group_warning_count(ptr), 1);
        assert_eq!(
            bytes(luminate_group_warning_at(ptr, 0)),
            Some(b"group warning".as_slice())
        );

        let surface = luminate_group_member_at(ptr, 0);
        assert_eq!(luminate_group_member_kind(surface), 0);
        assert_eq!(
            bytes(luminate_group_member_surface_id(surface)),
            Some(b"keys".as_slice())
        );
        assert_eq!(bytes(luminate_group_member_element_id(surface)), None);

        let element = luminate_group_member_at(ptr, 1);
        assert_eq!(luminate_group_member_kind(element), 1);
        assert_eq!(
            bytes(luminate_group_member_surface_id(element)),
            Some(b"keys".as_slice())
        );
        assert_eq!(
            bytes(luminate_group_member_element_id(element)),
            Some(b"escape".as_slice())
        );

        let nested = luminate_group_member_at(ptr, 2);
        assert_eq!(luminate_group_member_kind(nested), 2);
        assert_eq!(
            bytes(luminate_group_member_group_id(nested)),
            Some(b"nested".as_slice())
        );
    }
}

#[test]
fn null_and_absent_values_return_documented_sentinels() {
    let device = ptr::null::<LuminateDevice>();
    let surface = ptr::null::<LuminateSurface>();
    let null_element = ptr::null::<LuminateElement>();
    let group = ptr::null::<LuminateGroup>();
    let member = ptr::null::<LuminateGroupMember>();
    // SAFETY: every accessor explicitly accepts null and returns a sentinel.
    unsafe {
        assert_eq!(luminate_device_surface_count(device), 0);
        assert!(luminate_device_surface_at(device, 0).is_null());
        assert!(luminate_device_capabilities(device).is_null());
        assert_eq!(luminate_surface_kind(surface), u32::MAX);
        assert_eq!(luminate_surface_physical_tag_count(surface), 0);
        assert_eq!(bytes(luminate_surface_physical_tag_at(surface, 0)), None);
        let mut point = LuminatePoint { x: 1.0, y: 2.0 };
        let mut cell = LuminateMatrixCell { row: 1, column: 2 };
        let mut rect = LuminateRect {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        };
        let mut scalar = 5.0;
        assert!(!luminate_surface_sparse_size(surface, &raw mut point));
        assert_eq!((point.x, point.y), (1.0, 2.0));
        assert!(!luminate_surface_matrix_size(surface, &raw mut cell));
        assert_eq!((cell.row, cell.column), (1, 2));
        assert_eq!(luminate_element_kind(null_element), u32::MAX);
        assert_eq!(luminate_element_physical_tag_count(null_element), 0);
        assert_eq!(
            bytes(luminate_element_physical_tag_at(null_element, 0)),
            None
        );
        assert_eq!(luminate_element_geometry_kind(null_element), u32::MAX);
        assert!(!luminate_element_geometry_rect(null_element, &raw mut rect));
        assert_f32_eq(rect.width, 3.0);
        assert!(!luminate_element_geometry_point(
            null_element,
            &raw mut point
        ));
        assert_f32_eq(point.x, 1.0);
        assert!(!luminate_element_geometry_linear(
            null_element,
            &raw mut scalar
        ));
        assert_f32_eq(scalar, 5.0);
        assert!(!luminate_element_geometry_matrix_cell(
            null_element,
            &raw mut cell
        ));
        assert_eq!(cell.row, 1);
        assert_eq!(bytes(luminate_group_description(group)), None);
        assert_eq!(luminate_group_member_kind(member), u32::MAX);
        assert_eq!(bytes(luminate_group_member_surface_id(member)), None);
        assert_eq!(bytes(luminate_group_member_element_id(member)), None);
        assert_eq!(bytes(luminate_group_member_group_id(member)), None);
    }

    let mut unnamed = element(ElementKind::Led, None);
    unnamed.name = None;
    let unnamed: *const LuminateElement = ptr::from_ref(&unnamed).cast();
    // SAFETY: `unnamed` refers to a live `Element`.
    assert_eq!(bytes(unsafe { luminate_element_name(unnamed) }), None);
}

#[test]
fn physical_tag_accessors_cover_empty_vectors_without_mutating_last_error() {
    let mut empty_surface = surface(SurfaceKind::Opaque, Vec::new());
    empty_surface.physical_tags.clear();
    let empty_surface: *const LuminateSurface = ptr::from_ref(&empty_surface).cast();
    let mut empty_element = element(ElementKind::Led, None);
    empty_element.physical_tags.clear();
    let empty_element: *const LuminateElement = ptr::from_ref(&empty_element).cast();

    set_last_error("physical tag sentinel");

    // SAFETY: both pointers refer to live native values, and empty and
    // out-of-range access are explicitly supported.
    unsafe {
        assert_eq!(luminate_surface_physical_tag_count(empty_surface), 0);
        assert_eq!(
            bytes(luminate_surface_physical_tag_at(empty_surface, 0)),
            None
        );
        assert_eq!(luminate_element_physical_tag_count(empty_element), 0);
        assert_eq!(
            bytes(luminate_element_physical_tag_at(empty_element, 0)),
            None
        );

        assert_eq!(
            std::ffi::CStr::from_ptr(luminate_last_error_message()).to_bytes(),
            b"physical tag sentinel"
        );
    }
}
