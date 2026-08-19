// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Error type returned by Windows service dispatch, lifecycle, and
//! install/uninstall operations.

use std::io;

use thiserror::Error;
use windows_service::Error as WindowsServiceError;

use super::SERVICE_NAME;

/// Error returned while attaching to or communicating with SCM.
#[derive(Debug, Error)]
pub enum ServiceError {
    /// The service dispatcher failed.
    #[error("service dispatcher failed: {0}")]
    Dispatcher(#[source] WindowsServiceError),

    /// Connecting to SCM or operating on the registered service failed.
    #[error("service management operation failed: {0}")]
    Manager(#[source] WindowsServiceError),

    /// Resolving or validating the service executable path failed.
    #[error("validating service executable path failed: {0}")]
    ExecutablePath(#[source] io::Error),

    /// Creating or hardening the machine-wide data root failed.
    #[error("creating or hardening the machine data directory failed: {0}")]
    MachineDataDirectory(#[source] io::Error),

    /// Creating the client group or changing its membership failed.
    #[error("configuring the client group failed: {0}")]
    ClientGroup(#[source] io::Error),

    /// Reading or writing installer ownership metadata failed.
    #[error("updating installer ownership metadata failed: {0}")]
    InstallerMetadata(#[source] io::Error),

    /// Explicit client-group removal was refused because its safety
    /// preconditions were not met.
    #[error("refusing to purge the client group: {0}")]
    UnsafeClientGroupPurge(String),

    /// An existing service with this name is not one this installer may
    /// refresh.
    #[error("refusing to replace an existing {SERVICE_NAME} service: {0}")]
    ExistingService(String),

    /// Configuring a newly-created service failed, and rolling it back also
    /// failed.
    #[error(
        "configuring the new service failed ({configure}); rolling it back also failed ({rollback})"
    )]
    Rollback {
        /// The original configuration failure.
        configure: WindowsServiceError,
        /// The rollback failure.
        rollback: WindowsServiceError,
    },

    /// Registering the service control handler failed.
    #[error("registering service control handler failed: {0}")]
    RegisterHandler(#[source] io::Error),

    /// Reporting service status failed.
    #[error("reporting service status failed: {0}")]
    ReportStatus(#[source] io::Error),

    /// A service runner was registered more than once in one process.
    #[error("service runner was already registered")]
    RunnerAlreadyRegistered,
}

pub(super) fn raw_error_code(error: &ServiceError) -> u32 {
    match error {
        ServiceError::RegisterHandler(error) | ServiceError::ReportStatus(error) => {
            error.raw_os_error().unwrap_or(1).cast_unsigned()
        }
        ServiceError::Dispatcher(_)
        | ServiceError::Manager(_)
        | ServiceError::ExecutablePath(_)
        | ServiceError::MachineDataDirectory(_)
        | ServiceError::ClientGroup(_)
        | ServiceError::InstallerMetadata(_)
        | ServiceError::UnsafeClientGroupPurge(_)
        | ServiceError::ExistingService(_)
        | ServiceError::Rollback { .. }
        | ServiceError::RunnerAlreadyRegistered => 1,
    }
}
