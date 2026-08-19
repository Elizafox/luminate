// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Streamed frame payloads carried by `FrameUploadCapability` targets.
//!
//! A `FrameEnvelope` is intentionally delivery-mechanism-agnostic: today it
//! travels one frame per request/response round trip on the ordinary client
//! protocol socket, but `generation`/`sequence` give any future delivery
//! mechanism (for example a bounded, best-effort streaming channel) enough
//! information to detect stale or out-of-order frames without this type
//! needing to change.

use serde::{Deserialize, Serialize};

use crate::colour::Colour;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One frame addressed to a target that advertises `FrameUploadCapability`.
pub struct FrameEnvelope {
    /// Identifies the stream this frame belongs to. Bumped each time a
    /// stream (re)starts, so a frame from a stream that has since ended or
    /// restarted is recognizably stale rather than silently applied.
    pub generation: u32,

    /// Monotonically increasing within one `generation`, so an
    /// out-of-order or duplicate frame can be detected and dropped.
    pub sequence: u64,

    /// The pixel data itself.
    pub payload: FramePayload,

    /// Requests that this frame (and any frames staged since the last
    /// commit) become visible now. Meaningful only for a target whose
    /// `FrameUploadCapability::buffering` is `ExplicitCommit`; ignored
    /// otherwise, since `Immediate` and `DoubleBuffered` targets decide for
    /// themselves when a frame becomes visible.
    pub commit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// The pixel data carried by one `FrameEnvelope`.
pub enum FramePayload {
    /// The complete state of the target's addressable scope, in the
    /// plugin-defined pixel order. Required when `update_mode` is
    /// `FullFrameOnly`.
    Full(Vec<Colour>),

    /// Only the changed pixels, as `(index, colour)` pairs in the same
    /// plugin-defined order a `Full` payload would use. Only valid when
    /// `update_mode` is `Partial` or `Both`.
    Partial(Vec<(u32, Colour)>),
}

#[cfg(test)]
#[path = "frame_tests.rs"]
mod tests;
