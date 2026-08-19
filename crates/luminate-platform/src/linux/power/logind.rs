// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Linux suspend/resume source: systemd-logind's `PrepareForSleep` signal,
//! held open by a `delay` inhibitor lock.
//!
//! elogind exposes the identical `org.freedesktop.login1` interface, so this
//! covers non-systemd distributions (Devuan, Artix, Gentoo/OpenRC, Alpine
//! with elogind) without any separate code path.
//!
//! The inhibitor is what makes quiescing meaningful. `PrepareForSleep(true)`
//! on its own is only a heads-up (logind proceeds regardless), but while a
//! `delay` lock is held it waits, bounded by its own `InhibitDelayMaxSec`.
//! Taking the lock therefore converts "we were told" into "we have time", and
//! the lock's file descriptor rides inside the [`SuspendLease`] so the delay
//! ends exactly when the caller has finished, not on a timer we guessed.

use futures_util::StreamExt as _;
use zbus::zvariant::OwnedFd;
use zbus::{Connection, Proxy};

use crate::power::{PowerEventSender, SuspendLease, SystemPowerEvent};

const LOGIND_SERVICE: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const LOGIND_MANAGER: &str = "org.freedesktop.login1.Manager";

/// What the inhibitor tells logind (and, via `loginctl list-inhibitors`, the
/// operator) about why the suspend is being held.
const INHIBIT_WHAT: &str = "sleep";
const INHIBIT_WHO: &str = "Luminate";
const INHIBIT_WHY: &str = "Quiescing lighting hardware before suspend";
/// `delay` rather than `block`: Luminate needs a moment before the machine
/// goes down, and has no business preventing a suspend outright.
const INHIBIT_MODE: &str = "delay";

/// Starts the watcher, or logs why it could not start and returns.
pub(crate) fn start(events: PowerEventSender) {
    tokio::spawn(async move {
        match run(events).await {
            Ok(()) => tracing::debug!("logind suspend/resume watcher ended"),
            Err(error) => tracing::info!(
                error = %error,
                "no logind suspend/resume source; suspend will not be handled automatically"
            ),
        }
    });
}

async fn run(events: PowerEventSender) -> zbus::Result<()> {
    let connection = Connection::system().await?;
    let manager = Proxy::new(&connection, LOGIND_SERVICE, LOGIND_PATH, LOGIND_MANAGER).await?;

    let mut signals = manager.receive_signal("PrepareForSleep").await?;

    // Take the first lock before reporting readiness, so a suspend racing
    // startup is already delayed by the time the first signal could arrive.
    let mut inhibitor = take_inhibitor(&manager).await;
    tracing::info!(
        inhibited = inhibitor.is_some(),
        "watching logind for suspend and resume"
    );

    while let Some(signal) = signals.next().await {
        let going_to_sleep: bool = match signal.body().deserialize() {
            Ok(going_to_sleep) => going_to_sleep,
            Err(error) => {
                tracing::warn!(error = %error, "malformed PrepareForSleep signal; ignoring");
                continue;
            }
        };

        if going_to_sleep {
            // Hand the lock to the caller inside the lease. It quiesces and
            // then drops the lease, which closes the descriptor and lets
            // logind proceed, so the suspend waits exactly as long as the
            // work takes.
            let lease = SuspendLease {
                inhibitor: inhibitor.take(),
            };
            if events.send(SystemPowerEvent::Suspending(lease)).is_err() {
                return Ok(());
            }
        } else {
            if events.send(SystemPowerEvent::Resumed).is_err() {
                return Ok(());
            }
            // Re-arm for the next cycle. Without this the second suspend of a
            // session would get no delay at all.
            inhibitor = take_inhibitor(&manager).await;
        }
    }

    Ok(())
}

/// Takes a `delay` inhibitor lock, returning `None` if logind refuses.
///
/// A refusal is survivable: the caller still hears about the suspend, it just
/// has no guaranteed window to act in. Losing the quiesce is much better than
/// refusing to watch for suspends at all.
async fn take_inhibitor(manager: &Proxy<'_>) -> Option<OwnedFd> {
    match manager
        .call_method(
            "Inhibit",
            &(INHIBIT_WHAT, INHIBIT_WHO, INHIBIT_WHY, INHIBIT_MODE),
        )
        .await
        .and_then(|reply| reply.body().deserialize::<OwnedFd>())
    {
        Ok(inhibitor) => Some(inhibitor),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "could not take a logind sleep inhibitor; suspend will not wait for Luminate"
            );
            None
        }
    }
}

#[cfg(test)]
#[path = "logind_tests.rs"]
mod tests;
