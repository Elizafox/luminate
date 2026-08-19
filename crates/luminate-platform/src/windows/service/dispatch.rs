// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Attaching the process to the Service Control Manager and routing SCM's
//! `service_main` callback into the registered service runner.

use std::ffi::OsString;
use std::sync::OnceLock;

use windows_service::{Error, define_windows_service, service_dispatcher};

use super::error::ServiceError;
use super::handler::{ServiceContext, register_handler};
use super::{ERROR_FAILED_SERVICE_CONTROLLER_CONNECT, SERVICE_NAME};

type ServiceRunner = fn(ServiceContext);

static SERVICE_RUNNER: OnceLock<ServiceRunner> = OnceLock::new();

define_windows_service!(ffi_service_main, service_main);

/// Result of attempting to attach the process to the Service Control Manager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// The process was not launched by SCM and should continue in console mode.
    Console,

    /// SCM launched the service and its service-main function has returned.
    Service,
}

/// Attempts SCM dispatch, falling back only for an ordinary console launch.
///
/// # Errors
///
/// Returns an error if the runner was already registered or dispatcher setup
/// fails for any reason other than an ordinary non-SCM launch.
pub fn dispatch(runner: ServiceRunner) -> Result<DispatchOutcome, ServiceError> {
    SERVICE_RUNNER
        .set(runner)
        .map_err(|_| ServiceError::RunnerAlreadyRegistered)?;

    match service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        Ok(()) => Ok(DispatchOutcome::Service),
        Err(Error::Winapi(error))
            if error.raw_os_error() == Some(ERROR_FAILED_SERVICE_CONTROLLER_CONNECT) =>
        {
            Ok(DispatchOutcome::Console)
        }
        Err(error) => Err(ServiceError::Dispatcher(error)),
    }
}

fn service_main(_arguments: Vec<OsString>) {
    let Some(runner) = SERVICE_RUNNER.get().copied() else {
        return;
    };
    let Ok(context) = register_handler() else {
        return;
    };

    runner(context);
}
