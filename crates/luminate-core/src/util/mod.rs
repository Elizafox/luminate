// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Small reusable model utilities.

/// Discrete inclusive numeric ranges.
pub mod discrete;

pub use discrete::DiscreteRange;

mod opaque_id;

pub(crate) use opaque_id::declare_opaque_id;
