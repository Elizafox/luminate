// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Terminal-output safety tests.

use std::io;
use std::sync::{Arc, Mutex};

use tracing::subscriber;
use tracing_subscriber::fmt::MakeWriter;

use super::{TerminalSafeFields, escape, escape_json};

#[test]
fn escape_covers_c0_del_and_c1_without_changing_text() {
    assert_eq!(escape("ordinary Ünïcödé"), "ordinary Ünïcödé");
    assert_eq!(
        escape("nul\0 line\n esc\u{1b} del\u{7f} csi\u{9b}"),
        "nul\\u{0} line\\n esc\\u{1b} del\\u{7f} csi\\u{9b}"
    );
}

#[test]
fn json_escaping_preserves_values_and_structural_lines() {
    let serialized = serde_json::to_string_pretty(&serde_json::json!({
        "message": "line\n\u{1b}[2J\u{9b}31m\u{7f}"
    }))
    .expect("serialize hostile JSON");
    let escaped = escape_json(&serialized);

    assert!(!escaped.contains('\u{1b}'));
    assert!(!escaped.contains('\u{9b}'));
    assert!(!escaped.contains('\u{7f}'));
    assert!(escaped.contains('\n'));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&escaped).expect("parse escaped JSON"),
        serde_json::from_str::<serde_json::Value>(&serialized).expect("parse original JSON")
    );
}

#[derive(Clone, Debug, Default)]
struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("test destination lock poisoned")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for SharedWriter {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn tracing_fields_escape_controls_and_preserve_record_framing() {
    let destination = SharedWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_level(false)
        .with_target(false)
        .with_ansi(false)
        .fmt_fields(TerminalSafeFields)
        .with_writer(destination.clone())
        .finish();
    subscriber::with_default(subscriber, || {
        tracing::info!("hostile\0\n\u{1b}[2J\u{9b}31m");
    });

    let bytes = destination.0.lock().expect("lock test output").clone();
    let output = String::from_utf8(bytes).expect("UTF-8 output");
    assert_eq!(output, "hostile\\u{0}\\n\\x1b[2J\\u{9b}31m\n");
}
