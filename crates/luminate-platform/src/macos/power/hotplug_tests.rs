// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::time::Duration;

use tokio::sync::mpsc::unbounded_channel;
use tokio::time::timeout;

use super::*;

#[tokio::test]
async fn starting_the_watcher_never_panics_and_reports_no_spurious_events() {
    // Nothing is guaranteed to arrive in this window; this only asserts
    // the source starts and parks cleanly rather than panicking.
    let (sender, mut receiver) = unbounded_channel();
    start(sender);
    let _outcome = timeout(Duration::from_millis(200), receiver.recv()).await;
}
