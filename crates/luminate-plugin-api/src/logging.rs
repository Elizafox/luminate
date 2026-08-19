// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Optional `tracing` bridge for plugins.
//!
//! A plugin `cdylib` has its own `tracing` subscriber state. [`init`] installs
//! a formatter inside the plugin and forwards each line through the daemon's
//! `PluginLogFn`.
//!
//! The daemon supplies `max_level`, keeping plugin verbosity consistent
//! without parsing configuration twice.

use std::error;
use std::ffi::{CStr, CString};
use std::io::{self, Write as _};

use tracing::level_filters;
use tracing_subscriber::fmt::MakeWriter;

use luminate_platform::terminal::TerminalSafeFields;

use crate::{PluginLogFn, PluginLogLevel};

/// Installs a process-global `tracing` subscriber that routes every event
/// through `log`, tagged with `plugin_name`, filtered at `max_level`. Call
/// once, from `PluginDescriptor::init`, using the `max_level` that init
/// receives from the daemon.
///
/// # Panics
///
/// Panics if a global `tracing` subscriber is already installed (e.g. this
/// is called more than once), mirroring `tracing_subscriber::fmt::init`'s
/// own behaviour. This is a programmer error to fix at the call site, not a
/// runtime condition a plugin should try to recover from.
///
/// A panic here unwinds inside `PluginDescriptor::init`, i.e. across the FFI
/// boundary; prefer [`try_init`] in a plugin's real init path so the failure
/// is returned rather than unwound.
#[allow(
    clippy::panic,
    reason = "The global logger may only be installed once; duplicate installation is a process setup bug."
)]
pub fn init(plugin_name: &'static CStr, log: PluginLogFn, max_level: PluginLogLevel) {
    if let Err(error) = try_init(plugin_name, log, max_level) {
        panic!("a global tracing subscriber is already installed for this plugin: {error}");
    }
}

/// Like [`init`], but returns an error instead of panicking when a global
/// `tracing` subscriber is already installed. Use this from a plugin's
/// `PluginDescriptor::init`, where a panic would unwind across the FFI
/// boundary (undefined behavior for the `PluginInitFn` ABI): on `Err`, the
/// plugin can skip installing its bridge and keep going rather than abort.
///
/// # Errors
///
/// Returns an error if a global `tracing` subscriber is already installed.
pub fn try_init(
    plugin_name: &'static CStr,
    log: PluginLogFn,
    max_level: PluginLogLevel,
) -> Result<(), Box<dyn error::Error + Send + Sync + 'static>> {
    tracing_subscriber::fmt()
        .without_time()
        .with_level(false)
        .with_target(false)
        .with_ansi(false)
        .with_max_level(level_filter_from_plugin_level(max_level))
        .fmt_fields(TerminalSafeFields)
        .with_writer(FfiMakeWriter { plugin_name, log })
        .try_init()
}

const fn level_filter_from_plugin_level(level: PluginLogLevel) -> level_filters::LevelFilter {
    match level {
        PluginLogLevel::Error => level_filters::LevelFilter::ERROR,
        PluginLogLevel::Warn => level_filters::LevelFilter::WARN,
        PluginLogLevel::Info => level_filters::LevelFilter::INFO,
        PluginLogLevel::Debug => level_filters::LevelFilter::DEBUG,
        PluginLogLevel::Trace => level_filters::LevelFilter::TRACE,
    }
}

#[derive(Clone, Copy)]
struct FfiMakeWriter {
    plugin_name: &'static CStr,
    log: PluginLogFn,
}

impl FfiMakeWriter {
    fn writer_for_level(self, level: PluginLogLevel) -> FfiWriter {
        FfiWriter {
            plugin_name: self.plugin_name,
            log: self.log,
            level,
            buffer: Vec::new(),
        }
    }
}

impl<'a> MakeWriter<'a> for FfiMakeWriter {
    type Writer = FfiWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.writer_for_level(PluginLogLevel::Info)
    }

    fn make_writer_for(&'a self, meta: &tracing::Metadata<'_>) -> Self::Writer {
        self.writer_for_level(level_from_tracing(*meta.level()))
    }
}

const fn level_from_tracing(level: tracing::Level) -> PluginLogLevel {
    match level {
        tracing::Level::ERROR => PluginLogLevel::Error,
        tracing::Level::WARN => PluginLogLevel::Warn,
        tracing::Level::INFO => PluginLogLevel::Info,
        tracing::Level::DEBUG => PluginLogLevel::Debug,
        tracing::Level::TRACE => PluginLogLevel::Trace,
    }
}

/// Buffers one formatted event and forwards it through the FFI callback.
struct FfiWriter {
    plugin_name: &'static CStr,
    log: PluginLogFn,
    level: PluginLogLevel,
    buffer: Vec<u8>,
}

impl io::Write for FfiWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let message = String::from_utf8_lossy(&self.buffer);
        let message = message.trim_end_matches('\n');
        let plugin_name = self.plugin_name.to_owned();
        if let Ok(message) = CString::new(message) {
            // SAFETY: `self.log` was handed to us by the daemon via `init` and
            // is valid for the plugin's lifetime per the plugin ABI contract;
            // both C strings are NUL-terminated and live for the duration of
            // this call only. The level crosses as its ABI byte.
            unsafe { (self.log)(plugin_name.as_ptr(), self.level.to_abi(), message.as_ptr()) };
        }

        self.buffer.clear();
        Ok(())
    }
}

impl Drop for FfiWriter {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}
