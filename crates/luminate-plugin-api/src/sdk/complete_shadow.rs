// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Transactional staging for complete hardware-state shadows.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

/// Whether a plugin has an authoritative shadow of a complete hardware surface.
#[derive(Clone, Copy, Debug)]
pub enum CompleteShadow<'a, T: ?Sized> {
    /// Some part of the hardware state is unknown.
    Unknown,
    /// Every element of the hardware state is known.
    Complete(&'a T),
}

/// Why a partial update could not be staged against a complete shadow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompleteShadowError {
    /// The plugin does not know every sibling element's current state.
    Unknown,
    /// The stored frame does not have the required number of elements.
    WrongLength,
    /// The requested element is outside the complete surface.
    UnknownElement,
}

impl fmt::Display for CompleteShadowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => formatter.write_str("complete hardware state is unknown"),
            Self::WrongLength => formatter.write_str("complete hardware state has the wrong size"),
            Self::UnknownElement => formatter.write_str("element is outside the hardware surface"),
        }
    }
}

impl Error for CompleteShadowError {}

/// Stages indexed changes without modifying the authoritative frame.
///
/// The returned frame can be committed to the shadow only after its complete
/// hardware write succeeds.
///
/// # Errors
///
/// Returns an error when the shadow is unknown or malformed, or an update
/// index is outside the frame.
pub fn stage_frame_updates<T: Clone>(
    shadow: &CompleteShadow<'_, [T]>,
    expected_len: usize,
    updates: impl IntoIterator<Item = (usize, T)>,
) -> Result<Vec<T>, CompleteShadowError> {
    let CompleteShadow::Complete(current) = *shadow else {
        return Err(CompleteShadowError::Unknown);
    };
    if current.len() != expected_len {
        return Err(CompleteShadowError::WrongLength);
    }

    let mut staged = current.to_vec();
    for (index, value) in updates {
        let element = staged
            .get_mut(index)
            .ok_or(CompleteShadowError::UnknownElement)?;
        *element = value;
    }
    Ok(staged)
}

/// Stages keyed changes when every required key is present in the shadow.
///
/// The returned map can be committed to the shadow only after its complete
/// hardware write succeeds.
///
/// # Errors
///
/// Returns an error when the shadow is unknown, omits a required key, or an
/// update names an element outside the required key set.
pub fn stage_map_updates<K, V>(
    shadow: &CompleteShadow<'_, BTreeMap<K, V>>,
    required_keys: impl IntoIterator<Item = K>,
    updates: impl IntoIterator<Item = (K, V)>,
) -> Result<BTreeMap<K, V>, CompleteShadowError>
where
    K: Clone + Ord,
    V: Clone,
{
    let CompleteShadow::Complete(current) = *shadow else {
        return Err(CompleteShadowError::Unknown);
    };
    let required = required_keys.into_iter().collect::<Vec<_>>();
    if required.iter().any(|key| !current.contains_key(key)) {
        return Err(CompleteShadowError::WrongLength);
    }

    let mut staged = current.clone();
    for (key, value) in updates {
        if !required.contains(&key) {
            return Err(CompleteShadowError::UnknownElement);
        }
        staged.insert(key, value);
    }
    Ok(staged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_frame_cannot_be_staged() {
        assert_eq!(
            stage_frame_updates::<u8>(&CompleteShadow::Unknown, 2, [(0, 1)]),
            Err(CompleteShadowError::Unknown)
        );
    }

    #[test]
    fn frame_staging_does_not_mutate_the_complete_shadow() {
        let current = vec![1, 2, 3];
        let staged =
            stage_frame_updates(&CompleteShadow::Complete(current.as_slice()), 3, [(1, 9)])
                .expect("complete frame should stage");

        assert_eq!(current, vec![1, 2, 3]);
        assert_eq!(staged, vec![1, 9, 3]);
    }
}
