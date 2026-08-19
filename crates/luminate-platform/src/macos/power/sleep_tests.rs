// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::time::Duration;

use tokio::sync::mpsc::unbounded_channel;
use tokio::time::timeout;

use super::*;

#[tokio::test]
async fn a_missing_iokit_registration_is_reported_rather_than_fatal() {
    // A sandboxed build/CI runner without the usual root-power-domain
    // access must not panic; either the watcher registers and parks, or
    // it fails and logs why. Only a panic is unacceptable.
    let (sender, mut receiver) = unbounded_channel();
    start(sender);
    let _outcome = timeout(Duration::from_millis(200), receiver.recv()).await;
}
