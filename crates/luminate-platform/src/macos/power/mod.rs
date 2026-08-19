// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! macOS suspend/resume and device-change sources, both backed by `IOKit`
//! notifications delivered through a `CFRunLoop` on a dedicated thread; see
//! [`sleep`] and [`hotplug`] for why nothing here uses tokio directly.

pub(crate) mod hotplug;
pub(crate) mod sleep;

use crate::power::PowerEventSender;

/// Starts every macOS source this build supports.
pub(crate) fn start(sender: PowerEventSender) {
    sleep::start(sender.clone());
    hotplug::start(sender);
}
