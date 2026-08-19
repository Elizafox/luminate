// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Transport-neutral authorization policy models.

mod audit;
mod document;
mod identity;
mod runtime;

pub use audit::*;
pub use document::*;
pub use identity::*;
pub use runtime::*;

#[cfg(test)]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use std::time::{Duration, SystemTime};

#[cfg(test)]
use crate::device::DeviceId;

#[cfg(test)]
#[path = "../policy_tests.rs"]
mod tests;
