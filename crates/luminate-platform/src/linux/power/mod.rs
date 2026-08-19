// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Linux power/device sources: systemd-logind's `PrepareForSleep` signal and
//! udev netlink device-change notifications.

#[cfg(feature = "logind")]
pub(crate) mod logind;
pub(crate) mod uevent;

use crate::power::PowerEventSender;

/// Starts every Linux source this build supports.
pub(crate) fn start(sender: PowerEventSender) {
    #[cfg(feature = "logind")]
    logind::start(sender.clone());

    uevent::start(sender);
}
