// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Entry point and logging setup for the Luminate daemon.

use std::io::IsTerminal as _;
use std::io::{self, Write as _};
use std::process::ExitCode;
use std::{env, ffi::OsStr};

use anyhow::Result;
use luminate_platform::process_title::set_process_title;
use luminate_platform::terminal::{TerminalSafeFields, escape};
use tokio::runtime::{Builder, Runtime};
#[cfg(windows)]
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

#[cfg(windows)]
use luminate_platform::windows::service::{
    DispatchOutcome, ServiceContext, ServiceShutdown, dispatch,
};
#[cfg(windows)]
use operator_event::OperatorEvent;

mod atomic_file;
mod audit;
mod authentication;
mod authentication_provider;
mod authorization;
mod daemon;
mod device_config;
mod error;
mod managed_config;
mod normalize;
mod operator_event;
mod persistence;
mod plugin_host;
mod plugins;
mod policy_persistence;
#[cfg(any(windows, target_os = "macos", test))]
mod service_command;
mod state;
#[cfg(windows)]
mod windows_logging;

fn main() -> ExitCode {
    match run_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(
                io::stderr().lock(),
                "{}",
                escape(&format!("error: {error:#}"))
            );
            ExitCode::FAILURE
        }
    }
}

fn run_main() -> Result<()> {
    if env::args_os().nth(1).as_deref() == Some(OsStr::new("state")) {
        return run_state_command();
    }

    if let Some(path) = plugin_host::inspection_invocation_from_args()? {
        set_process_title("luminate: plugin inspector");
        return plugin_host::run_inspection(&path);
    }
    if let Some(path) = plugin_host::setup_invocation_from_args()? {
        set_process_title("luminate: plugin setup");
        return plugin_host::run_setup_step(&path);
    }
    if let Some((path, max_log_level)) = plugin_host::invocation_from_args()? {
        set_process_title("luminate: plugin host");
        return plugin_host::run(&path, max_log_level);
    }
    set_process_title("luminate: main daemon");

    #[cfg(windows)]
    {
        if service_command::run_from_args()? {
            return Ok(());
        }

        match dispatch(run_windows_service)? {
            DispatchOutcome::Service => return Ok(()),
            DispatchOutcome::Console => {}
        }
    }

    // Unlike Windows, launchd execs `luminated` as an ordinary foreground
    // process and stops it with `SIGTERM`: there is no SCM-style dispatcher
    // to hand control to, so `RunContext::console()` below is already the
    // right shape for a launchd-run process. See
    // `docs/development/architecture/macos-service.md`, "Context".
    #[cfg(target_os = "macos")]
    if service_command::run_from_args()? {
        return Ok(());
    }

    // Default to `info` when `RUST_LOG` is unset so the daemon produces useful
    // logs out of the box (nothing in packaging sets `RUST_LOG`), while still
    // honoring an explicit `RUST_LOG`. Diagnostic values can originate in
    // plugins, hardware, configuration, or clients, so the field formatter
    // escapes every terminal control while retaining trusted colour styling.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(io::stdout().is_terminal())
        .fmt_fields(TerminalSafeFields)
        .init();

    // `RunContext::console` installs Tokio signal handlers, which panic unless
    // a reactor is running on the current thread. Building a runtime does not
    // enter its context, so construct the context inside `block_on` rather than
    // eagerly as a `block_on` argument.
    runtime()?.block_on(async { daemon::run(daemon::RunContext::console()?).await })
}

#[allow(
    clippy::print_stdout,
    reason = "The offline administrative command reports its backup path to the operator."
)]
fn run_state_command() -> Result<()> {
    let mut arguments = env::args_os();
    let _program = arguments.next();
    let _state = arguments.next();
    let operation = arguments
        .next()
        .ok_or_else(|| anyhow::anyhow!("state command requires an operation"))?;
    anyhow::ensure!(
        operation == "reset-owned-objects",
        "unknown state operation: {}",
        operation.to_string_lossy()
    );

    let mut confirmed = false;
    for argument in arguments {
        if argument == "--confirm" {
            confirmed = true;
        } else {
            anyhow::bail!(
                "unexpected argument to state reset-owned-objects: {}",
                argument.to_string_lossy()
            );
        }
    }

    let path = daemon::configured_state_path()?;
    let backup = persistence::reset_owned_objects(&path, confirmed)?;
    println!(
        "cleared persisted collections and scenes; backup written to {}",
        escape(&backup.to_string_lossy())
    );
    Ok(())
}

fn runtime() -> Result<Runtime> {
    Ok(Builder::new_multi_thread().enable_all().build()?)
}

#[cfg(windows)]
fn run_windows_service(context: ServiceContext) {
    let status = context.status.clone();
    let Ok(logging) = windows_logging::ServiceLogging::initialize() else {
        let _ = status.stopped_with_error(1);
        return;
    };
    OperatorEvent::ServiceStarting.emit("Windows service initialization started");
    let result = runtime().and_then(|runtime| runtime.block_on(run_service_daemon(context)));
    let status_result = match result {
        Ok(()) => {
            OperatorEvent::ServiceStopped.emit("Windows service stopped");
            status.stopped(0)
        }
        Err(error) => {
            OperatorEvent::ServiceStopped.emit(&format!("Windows service failed: {error:#}"));
            status.stopped_with_error(1)
        }
    };

    // There is no reliable fallback once SCM status reporting itself fails.
    let _ = status_result;
    drop(logging);
}

#[cfg(windows)]
async fn run_service_daemon(context: ServiceContext) -> Result<()> {
    let ServiceContext {
        mut shutdown,
        power_events,
        status,
    } = context;
    let (daemon_shutdown_tx, daemon_shutdown_rx) = mpsc::channel(1);
    tokio::spawn(async move {
        let Some(request) = shutdown.recv().await else {
            return;
        };
        match request {
            ServiceShutdown::Stop => {
                OperatorEvent::ServiceStopRequested.emit("Windows service stop requested");
            }
            ServiceShutdown::Preshutdown => {
                OperatorEvent::ServicePreshutdownRequested
                    .emit("Windows service preshutdown requested");
            }
        }
        let _ = daemon_shutdown_tx.send(()).await;
    });
    let (lifecycle_tx, mut lifecycle_rx) = mpsc::unbounded_channel();
    let daemon = daemon::run(daemon::RunContext::service(
        daemon_shutdown_rx,
        lifecycle_tx,
        power_events,
    ));
    tokio::pin!(daemon);

    loop {
        tokio::select! {
            biased;

            result = &mut daemon => return result,
            event = lifecycle_rx.recv() => match event {
                Some(daemon::LifecycleEvent::StartupProgress(_)) => status.startup_progress()?,
                Some(daemon::LifecycleEvent::Ready) => {
                    status.running()?;
                    OperatorEvent::ServiceReady.emit("Windows service is ready");
                }
                None => anyhow::bail!("daemon lifecycle event source closed before completion"),
            }
        }
    }
}
