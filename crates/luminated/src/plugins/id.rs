// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! A stable identity for one loaded-plugin instance.

use uuid::Uuid;

/// Identifies one running instance of a loaded plugin.
///
/// Generated fresh every time a plugin is (re)loaded, so a reloaded plugin
/// gets a new identity rather than reusing its predecessor's. This lets
/// `PluginManager` key ownership and topology state by something that
/// survives a plugin being removed from the middle of the loaded set,
/// unlike a positional index into a `Vec`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct LoadedPluginId(Uuid);

impl LoadedPluginId {
    pub(super) fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
