// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::time::Duration;

use tokio::sync::mpsc::unbounded_channel;
use tokio::time::timeout;

use super::*;

#[tokio::test]
async fn a_missing_system_bus_is_reported_rather_than_fatal() {
    // CI and containers routinely have no system bus, and a desktop has a
    // real logind. Both must be fine: the watcher either reports an error
    // (no bus, or a bus that refused) or parks on the signal stream. The
    // one unacceptable outcome is a panic, which would take the daemon's
    // whole runtime with it.
    let (sender, _receiver) = unbounded_channel();
    let _outcome = timeout(Duration::from_secs(5), run(sender)).await;
}
