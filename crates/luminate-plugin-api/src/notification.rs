// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Safe plugin-side access to the daemon's topology-change callback.

use std::ffi::CStr;
use std::sync::OnceLock;

use crate::PluginNotifyFn;

struct NotificationBridge {
    plugin_name: &'static CStr,
    notify: PluginNotifyFn,
}

static BRIDGE: OnceLock<NotificationBridge> = OnceLock::new();

/// Installs the daemon notification callback for this plugin image.
///
/// Each plugin `cdylib` has its own copy of this crate's statics, so one bridge
/// is sufficient and cannot collide with another loaded plugin.
pub fn init(plugin_name: &'static CStr, notify: PluginNotifyFn) {
    let _ = BRIDGE.set(NotificationBridge {
        plugin_name,
        notify,
    });
}

/// Marks this plugin's topology as dirty.
///
/// The daemon coalesces notifications and re-pulls `topology_cbor`; callers do
/// not need to debounce genuine device-set changes themselves. Calling this
/// before plugin initialization is a harmless no-op.
pub fn topology_changed() {
    let Some(bridge) = BRIDGE.get() else {
        return;
    };

    // SAFETY: the bridge comes from the daemon during plugin initialization,
    // and the static plugin name remains valid for the process lifetime.
    unsafe { (bridge.notify)(bridge.plugin_name.as_ptr()) };
}
