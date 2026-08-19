// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::future;

use super::*;

#[tokio::test]
async fn queued_shutdown_is_observed_at_a_startup_checkpoint() {
    let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
    shutdown_tx.send(()).await.expect("queue shutdown request");
    let mut shutdown = ShutdownSource::Requested(shutdown_rx);

    assert!(stop_after_startup_checkpoint(&mut shutdown).expect("observe queued shutdown"));
}

#[tokio::test]
async fn shutdown_interrupts_an_asynchronous_startup_phase() {
    let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
    let mut shutdown = ShutdownSource::Requested(shutdown_rx);
    let phase = run_startup_phase(&mut shutdown, future::pending::<anyhow::Result<()>>());

    shutdown_tx.send(()).await.expect("request shutdown");

    assert_eq!(phase.await.expect("interrupt startup phase"), None);
}

#[tokio::test]
async fn completed_asynchronous_startup_phase_returns_its_value() {
    let (_shutdown_tx, shutdown_rx) = mpsc::channel(1);
    let mut shutdown = ShutdownSource::Requested(shutdown_rx);

    assert_eq!(
        run_startup_phase(&mut shutdown, future::ready(Ok(42)))
            .await
            .expect("complete startup phase"),
        Some(42)
    );
}
