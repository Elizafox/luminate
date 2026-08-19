// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::time::Duration;

use tokio::time::timeout;

use super::*;

#[test]
#[allow(
    clippy::drop_non_drop,
    reason = "on a build with nothing to hold, SuspendLease has no Drop impl at all; this test \
                  exists precisely to prove that constructing and dropping one is still harmless \
                  on such a build."
)]
fn a_lease_holding_nothing_drops_cleanly() {
    drop(SuspendLease::default());
}

#[tokio::test]
async fn starting_sources_never_fails_and_reports_no_spurious_events() {
    let mut receiver = start_sources();
    match timeout(Duration::from_millis(50), receiver.recv()).await {
        Err(_elapsed) => {}
        Ok(None) => {}
        Ok(Some(event)) => panic!("no source should emit unprompted, got {event:?}"),
    }
}
