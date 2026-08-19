// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Querying and controlling the already-registered daemon service's SCM
//! state.

use windows_service::service::{ServiceAccess, ServiceState as WindowsServiceState};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use super::SERVICE_NAME;
use super::error::ServiceError;

/// Current SCM state of the Luminate daemon service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceState {
    /// The service is not running.
    Stopped,

    /// The service is starting.
    StartPending,

    /// The service is stopping.
    StopPending,

    /// The service is running.
    Running,

    /// The service is resuming.
    ContinuePending,

    /// The service is pausing.
    PausePending,

    /// The service is paused.
    Paused,
}

impl From<WindowsServiceState> for ServiceState {
    fn from(state: WindowsServiceState) -> Self {
        match state {
            WindowsServiceState::Stopped => Self::Stopped,
            WindowsServiceState::StartPending => Self::StartPending,
            WindowsServiceState::StopPending => Self::StopPending,
            WindowsServiceState::Running => Self::Running,
            WindowsServiceState::ContinuePending => Self::ContinuePending,
            WindowsServiceState::PausePending => Self::PausePending,
            WindowsServiceState::Paused => Self::Paused,
        }
    }
}

/// Queries the registered daemon service's current SCM state.
///
/// # Errors
///
/// Returns an error if SCM cannot be opened, the service is not registered, or
/// its status cannot be queried.
pub fn query_state() -> Result<ServiceState, ServiceError> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(ServiceError::Manager)?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS)
        .map_err(ServiceError::Manager)?;
    let status = service.query_status().map_err(ServiceError::Manager)?;

    Ok(status.current_state.into())
}

/// Requests that SCM start the registered daemon service.
///
/// # Errors
///
/// Returns an error if SCM cannot be opened, the service is not registered, or
/// SCM rejects the start request.
pub fn start() -> Result<(), ServiceError> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(ServiceError::Manager)?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::START)
        .map_err(ServiceError::Manager)?;

    service.start::<&str>(&[]).map_err(ServiceError::Manager)
}

/// Requests that SCM stop the registered daemon service.
///
/// # Errors
///
/// Returns an error if SCM cannot be opened, the service is not registered, or
/// SCM rejects the stop request.
pub fn stop() -> Result<(), ServiceError> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(ServiceError::Manager)?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::STOP)
        .map_err(ServiceError::Manager)?;

    service.stop().map(|_| ()).map_err(ServiceError::Manager)
}
