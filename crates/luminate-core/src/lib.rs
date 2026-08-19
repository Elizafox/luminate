// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![doc = "Shared, serializable lighting topology, capability, targeting, effect, and state models.\n\nThese types form Luminate's canonical consumer vocabulary. Identifiers are opaque strings: construct them with their `new` methods and do not infer hardware identity from their contents. Capability values describe what a target accepts; callers should inspect them before constructing mutations."]
#![deny(missing_docs)]

/// Named firmware-stored appearances on one physical surface.
pub mod appearance_slot;

/// Target capability descriptions and value constraints.
pub mod capability;

/// User-created, cross-device target aggregates.
pub mod collection;

/// Portable colour values and encodings.
pub mod colour;
pub mod control;

/// Devices and stable device identifiers.
pub mod device;

/// Portable and vendor-advertised lighting effects.
pub mod effect;

/// Individually addressable elements and their geometry.
pub mod element;

/// Streamed frame payloads carried by frame-upload capable targets.
pub mod frame;

/// Logical target groups and membership.
pub mod group;

/// Transport-neutral authorization policy models and evaluation.
pub mod policy;

/// Eight-bit additive RGB values.
pub mod rgb;

/// Persistent snapshots of intended target state.
pub mod scene;

/// Compact, fixed-layout pixel data for the shared-memory frame fast path.
pub mod shm_frame;

pub mod state;

/// Addressable device surfaces and layouts.
pub mod surface;

/// Safe, structured mutation target identifiers.
pub mod target;
pub mod transition;

/// Reusable model utilities.
pub mod util;
