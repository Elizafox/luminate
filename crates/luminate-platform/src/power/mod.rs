// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Platform sources that tell a host application the machine is suspending,
//! resuming, or that its devices changed.
//!
//! Sources only discover and emit [`SystemPowerEvent`]; consumers own the
//! resulting policy.
//!
//! Every source is best-effort by construction: a machine with no reachable
//! source still runs, and a caller can drive the same work through whatever
//! manual rescan trigger it exposes.

#[cfg(target_os = "macos")]
use crate::macos::power::sleep::PowerChangeAck as MacOsPowerChangeAck;
use tokio::sync::mpsc;
#[cfg(all(target_os = "linux", feature = "logind"))]
use zbus::zvariant::OwnedFd;

/// Something happened that suspend/resume handling cares about.
#[derive(Debug)]
pub enum SystemPowerEvent {
    /// The machine is about to suspend. The caller quiesces and then drops
    /// the [`SuspendLease`], which is what allows the suspend to proceed.
    Suspending(SuspendLease),

    /// The machine resumed.
    Resumed,

    /// Devices appeared or disappeared. Not a power event, but it lands here
    /// because it drives the same rescan: a resume that re-enumerates USB
    /// hardware produces exactly this, and so does an ordinary hotplug.
    DevicesChanged,
}

/// Holds the machine at the point of suspend until the caller has quiesced.
///
/// Dropping the lease releases the platform's suspend hold: it closes the
/// logind inhibitor on Linux or sends the `IOKit` acknowledgement on macOS.
/// Tying that release to ownership means the machine cannot suspend before
/// the caller finishes quiescing.
///
/// Observer-only sources use an empty lease, whose drop is a no-op. Windows
/// is one such source; see [`crate::windows::power`] for why holding a power
/// request at this point would not help.
#[derive(Debug, Default)]
pub struct SuspendLease {
    /// The inhibitor file descriptor, closed on drop. `None` for a source
    /// that cannot delay the suspend.
    #[cfg(all(target_os = "linux", feature = "logind"))]
    #[allow(
        dead_code,
        reason = "held purely for its Drop: closing this descriptor is what releases logind's \
                  delay inhibitor and lets the suspend proceed. Nothing reads it, and nothing \
                  should."
    )]
    pub(crate) inhibitor: Option<OwnedFd>,

    /// The `IOKit` sleep acknowledgement, fired on drop. `None` is not a
    /// currently reachable state (macOS's source always constructs one),
    /// but the field stays an `Option` for the same reason the logind
    /// inhibitor above is one: a lease built by [`SuspendLease::default`]
    /// still needs to be valid.
    #[cfg(target_os = "macos")]
    #[allow(
        dead_code,
        reason = "held purely for its Drop: dropping this is what sends IOAllowPowerChange and \
                  lets the suspend proceed. Nothing reads it, and nothing should."
    )]
    pub(crate) allow_change: Option<MacOsPowerChangeAck>,
}

/// Sender half handed to each platform source.
pub type PowerEventSender = mpsc::UnboundedSender<SystemPowerEvent>;

/// Receiver half a caller drains.
pub type PowerEventReceiver = mpsc::UnboundedReceiver<SystemPowerEvent>;

/// Starts every power/device source this platform and build support,
/// returning the receiver the caller drains.
///
/// Never fails: a source that cannot start logs why and is skipped, because
/// no suspend integration is a degraded host, not a broken one.
#[must_use]
pub fn start_sources() -> PowerEventReceiver {
    let (sender, receiver) = mpsc::unbounded_channel();

    start_platform_sources(sender.clone());

    // Let the channel close when no source started.
    drop(sender);

    receiver
}

#[cfg(target_os = "linux")]
fn start_platform_sources(sender: PowerEventSender) {
    use crate::linux::power::start;
    start(sender);
}

#[cfg(target_os = "macos")]
fn start_platform_sources(sender: PowerEventSender) {
    use crate::macos::power::start;
    start(sender);
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn start_platform_sources(_sender: PowerEventSender) {
    tracing::info!(
        "no suspend/resume source for this platform; fall back to a manual rescan trigger to \
         re-enumerate hardware by hand"
    );
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
