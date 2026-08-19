// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Logging setup used when the daemon is hosted by the Windows SCM.

use anyhow::{Context as _, Result};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer as _, fmt};

use luminate_platform::default_path;
use luminate_platform::terminal::TerminalSafeFields;
use luminate_platform::windows::event_log::{self, EventLogLayer};

use crate::operator_event::OperatorEvent;

const RETAINED_LOG_FILES: usize = 7;

/// Keeps the service file writer alive until SCM shutdown reporting finishes.
pub(crate) struct ServiceLogging {
    _file_guard: Option<WorkerGuard>,
}

impl ServiceLogging {
    /// Installs the service-mode tracing subscriber.
    ///
    /// The file and Event Log sinks degrade independently. Startup fails only
    /// when neither sink can be initialized.
    pub(crate) fn initialize() -> Result<Self> {
        let event_layer = event_layer();
        let file_layer = file_layer();
        let event_error = event_layer.as_ref().err().map(ToString::to_string);
        let file_error = file_layer.as_ref().err().map(ToString::to_string);

        if let (Some(event_error), Some(file_error)) = (&event_error, &file_error) {
            anyhow::bail!(
                "neither Windows service log sink could be initialized: file log: {file_error}; Event Log: {event_error}"
            );
        }

        let event_layer = event_layer.ok();
        let (file_layer, file_guard) = match file_layer {
            Ok((layer, guard)) => (Some(layer), Some(guard)),
            Err(_) => (None, None),
        };

        tracing_subscriber::registry()
            .with(file_layer)
            .with(event_layer)
            .try_init()
            .context("installing the Windows service logging subscriber")?;

        if let Some(error) = event_error {
            tracing::warn!(%error, "Windows Event Log is unavailable; continuing with file logging only");
        }
        if let Some(error) = file_error {
            OperatorEvent::ServiceLoggingDegraded.emit(&format!(
                "Windows service file logging is unavailable: {error}"
            ));
        }

        Ok(Self {
            _file_guard: file_guard,
        })
    }
}

fn event_layer() -> Result<EventLogLayer> {
    if !event_log::event_source_is_registered()
        .context("checking the luminated Event Log source")?
    {
        anyhow::bail!("the luminated Event Log source is not registered");
    }

    EventLogLayer::new(OperatorEvent::TARGET).context("opening the luminated Event Log source")
}

fn file_layer() -> Result<(
    impl tracing_subscriber::Layer<tracing_subscriber::Registry>,
    WorkerGuard,
)> {
    let directory = default_path::logs().context("resolving the Windows service log directory")?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("luminated")
        .filename_suffix("log")
        .max_log_files(RETAINED_LOG_FILES)
        .build(&directory)
        .with_context(|| format!("opening the service log directory {}", directory.display()))?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let layer = fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .fmt_fields(TerminalSafeFields)
        .with_filter(filter);

    Ok((layer, guard))
}
