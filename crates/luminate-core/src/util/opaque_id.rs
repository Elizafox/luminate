// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Shared boilerplate for opaque string-newtype identifiers.

/// Generates `new` and `as_str` for opaque string newtypes.
///
/// Struct definitions stay explicit so each type retains its own derives,
/// serialization, and additional constructors. `$new_doc` documents the
/// generated constructor.
macro_rules! declare_opaque_id {
    ($ty:ty, $new_doc:literal) => {
        impl $ty {
            #[doc = $new_doc]
            #[must_use]
            #[inline]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Borrows the identifier as a string.
            #[must_use]
            #[inline]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

pub(crate) use declare_opaque_id;
