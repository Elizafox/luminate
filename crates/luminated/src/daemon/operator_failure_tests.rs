// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::subscriber;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::{Layer, Registry};

use super::*;

#[derive(Default)]
struct CapturedEvent {
    id: Option<u64>,
    message: Option<String>,
}

impl Visit for CapturedEvent {
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "id" {
            self.id = Some(value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        }
    }
}

struct CaptureLayer(Arc<Mutex<Vec<CapturedEvent>>>);

impl<S: Subscriber> Layer<S> for CaptureLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if event.metadata().target() != OperatorEvent::TARGET {
            return;
        }

        let mut captured = CapturedEvent::default();
        event.record(&mut captured);
        self.0.lock().expect("capture lock poisoned").push(captured);
    }
}

#[test]
fn operator_failure_is_emitted_only_when_enabled() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = Registry::default().with(CaptureLayer(Arc::clone(&captured)));

    subscriber::with_default(subscriber, || {
        let error = report_operator_failure::<()>(
            false,
            OperatorEvent::ConfigurationLoadFailed,
            "load failed",
            Err(anyhow::anyhow!("console error")),
        )
        .expect_err("the original failure must be returned");
        assert_eq!(error.to_string(), "console error");

        let error = report_operator_failure::<()>(
            true,
            OperatorEvent::ConfigurationLoadFailed,
            "load failed",
            Err(anyhow::anyhow!("service error")),
        )
        .expect_err("the original failure must be returned");
        assert_eq!(error.to_string(), "service error");
    });

    let captured = captured.lock().expect("capture lock poisoned");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].id, Some(200));
    assert_eq!(
        captured[0].message.as_deref(),
        Some("load failed: service error")
    );
}
