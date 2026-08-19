// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Inclusive numeric ranges constrained to a discrete step size.

use core::ops::{Rem, Sub};

use serde::{Deserialize, Serialize};

/// An inclusive discrete range.
///
/// Valid values begin at `min`, advance by `step`, and do not exceed `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscreteRange<T> {
    /// Inclusive minimum.
    pub min: T,

    /// Inclusive maximum.
    pub max: T,

    /// Increment between valid values.
    pub step: T,
}

impl<T> DiscreteRange<T> {
    /// Creates an inclusive range with the given step.
    #[inline]
    pub const fn new(min: T, max: T, step: T) -> Self {
        Self { min, max, step }
    }
}

impl<T> DiscreteRange<T>
where
    T: Copy + PartialOrd + Default + Sub<Output = T> + Rem<Output = T>,
{
    /// Whether `value` is within the range and on a step boundary from `min`.
    ///
    /// A zero step admits no values.
    #[must_use]
    #[inline]
    pub fn contains(&self, value: T) -> bool {
        let zero = T::default();
        self.step != zero
            && value >= self.min
            && value <= self.max
            && (value - self.min) % self.step == zero
    }
}
