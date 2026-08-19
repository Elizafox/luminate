// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Installing, configuring, and uninstalling the daemon's SCM registration,
//! including client-group and Event Log source ownership lifecycle.

use std::env;
use std::ffi::{OsString, c_void};
use std::fs::canonicalize;
use std::io;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::Duration;

use windows_service::Error;
use windows_service::service::{
    Service, ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl,
    ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceSidType,
    ServiceStartType, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_sys::Win32::System::Services::{
    ChangeServiceConfig2W, SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO,
    SERVICE_REQUIRED_PRIVILEGES_INFOW,
};

use crate::windows::default_path;
use crate::windows::event_log::{
    EnsureEventSourceOutcome, SOURCE_NAME, ensure_event_source, remove_event_source,
};
use crate::windows::installer_metadata::{
    owns_client_group, owns_event_log_source, record_owned_client_group,
    record_owned_event_log_source, remove_owned_client_group, remove_owned_event_log_source,
};
use crate::windows::local_group::{
    AddMemberOutcome, EnsureGroupOutcome, add_local_group_member, current_account_name,
    delete_local_group, ensure_local_group, local_group_is_empty, remove_local_group_member,
};
use crate::windows::security_descriptor::protect_machine_data_root;

use super::error::ServiceError;
use super::{
    CLIENT_GROUP_DESCRIPTION, CLIENT_GROUP_NAME, ERROR_SERVICE_DOES_NOT_EXIST,
    ERROR_SERVICE_EXISTS, ERROR_SERVICE_NOT_ACTIVE, FAILURE_COUNT_RESET_PERIOD,
    FAILURE_RESTART_DELAYS, PRESHUTDOWN_TIMEOUT, REQUIRED_PRIVILEGES, SERVICE_DESCRIPTION,
    SERVICE_DISPLAY_NAME, SERVICE_NAME,
};

/// Installs or refreshes the daemon's basic SCM registration.
///
/// The normal path must reside under the machine's Program Files directory.
/// `allow_development_path` is an explicit escape hatch for throwaway
/// development machines and must not be used for production installation.
///
/// Returns whether this invocation added at least one client-group member.
///
/// # Errors
///
/// Returns an error if the executable path cannot be canonicalized, is outside
/// Program Files without the development override, the machine data root
/// cannot be created or hardened, an unrelated service already owns the
/// name, or SCM rejects the registration.
pub fn install(
    allow_development_path: bool,
    add_users: &[String],
    add_current_user: bool,
) -> Result<bool, ServiceError> {
    let executable_path = env::current_exe()
        .and_then(canonicalize)
        .map_err(ServiceError::ExecutablePath)?;
    validate_install_path(&executable_path, allow_development_path)?;
    let service_info = service_info(executable_path.clone());

    let data_root =
        default_path::program_data_root().map_err(ServiceError::MachineDataDirectory)?;
    protect_machine_data_root(&data_root).map_err(ServiceError::MachineDataDirectory)?;

    let group_created = ensure_local_group(CLIENT_GROUP_NAME, CLIENT_GROUP_DESCRIPTION)
        .map_err(ServiceError::ClientGroup)?
        == EnsureGroupOutcome::Created;

    let mut accounts = add_users.to_vec();
    if add_current_user {
        match current_account_name() {
            Ok(account) => accounts.push(account),
            Err(error) => {
                rollback_group_changes(group_created, &[]).map_err(ServiceError::ClientGroup)?;
                return Err(ServiceError::ClientGroup(error));
            }
        }
    }

    let mut added_accounts = Vec::new();
    for account in &accounts {
        match add_local_group_member(CLIENT_GROUP_NAME, account) {
            Ok(AddMemberOutcome::Added) => added_accounts.push(account.clone()),
            Ok(AddMemberOutcome::AlreadyMember) => {}
            Err(error) => {
                rollback_group_changes(group_created, &added_accounts)
                    .map_err(ServiceError::ClientGroup)?;
                return Err(ServiceError::ClientGroup(error));
            }
        }
    }

    if group_created && let Err(error) = record_owned_client_group(CLIENT_GROUP_NAME) {
        rollback_group_changes(true, &added_accounts).map_err(ServiceError::ClientGroup)?;
        return Err(ServiceError::InstallerMetadata(error));
    }

    let event_source_created = match ensure_event_source() {
        Ok(EnsureEventSourceOutcome::Created) => true,
        Ok(EnsureEventSourceOutcome::AlreadyRegistered) => false,
        Err(error) => {
            rollback_group_install(group_created, &added_accounts)?;
            return Err(ServiceError::InstallerMetadata(error));
        }
    };
    if event_source_created && let Err(error) = record_owned_event_log_source(SOURCE_NAME) {
        remove_event_source().map_err(|rollback| {
            ServiceError::InstallerMetadata(io::Error::other(format!(
                "recording Event Log source ownership failed ({error}); rolling the source back \
                 also failed ({rollback})"
            )))
        })?;
        rollback_group_install(group_created, &added_accounts)?;
        return Err(ServiceError::InstallerMetadata(error));
    }

    if let Err(error) = install_service(&service_info) {
        rollback_event_source_install(event_source_created)?;
        if group_created {
            remove_owned_client_group(CLIENT_GROUP_NAME)
                .map_err(ServiceError::InstallerMetadata)?;
        }
        rollback_group_changes(group_created, &added_accounts)
            .map_err(ServiceError::ClientGroup)?;
        return Err(error);
    }

    Ok(!added_accounts.is_empty())
}

fn rollback_group_install(
    group_created: bool,
    added_accounts: &[String],
) -> Result<(), ServiceError> {
    if group_created {
        remove_owned_client_group(CLIENT_GROUP_NAME).map_err(ServiceError::InstallerMetadata)?;
    }
    rollback_group_changes(group_created, added_accounts).map_err(ServiceError::ClientGroup)
}

fn rollback_event_source_install(created: bool) -> Result<(), ServiceError> {
    if !created {
        return Ok(());
    }
    remove_event_source().map_err(ServiceError::InstallerMetadata)?;
    remove_owned_event_log_source(SOURCE_NAME).map_err(ServiceError::InstallerMetadata)
}

fn rollback_group_changes(group_created: bool, added_accounts: &[String]) -> io::Result<()> {
    if group_created {
        return delete_local_group(CLIENT_GROUP_NAME);
    }

    for account in added_accounts.iter().rev() {
        remove_local_group_member(CLIENT_GROUP_NAME, account)?;
    }
    Ok(())
}

/// Strips the quoting `windows_service::ServiceInfo` adds around a binary
/// path containing a space.
///
/// `ServiceConfig::from_raw` (`query_config`'s return type) copies
/// `lpBinaryPathName` verbatim into a `PathBuf`, without undoing the
/// quoting `windows-service`'s own `escape_wide` applied when the service was
/// registered. Comparing that against a freshly canonicalized path would
/// then spuriously fail for every production install, since `%ProgramFiles%`
/// always contains a space. A canonical filesystem path can never itself
/// contain `"` (NTFS forbids it) or end in a bare `\`, so stripping one
/// matching pair of surrounding quotes is an exact inverse for every path
/// this installer registers. This is not a general command-line unescaper,
/// which would be the wrong tool for a value that is always a plain path.
fn unquote_binary_path(path: &Path) -> &Path {
    let Some(text) = path.to_str() else {
        return path;
    };
    match text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    {
        Some(unquoted) => Path::new(unquoted),
        None => path,
    }
}

fn install_service(service_info: &ServiceInfo) -> Result<(), ServiceError> {
    let executable_path = service_info.executable_path.clone();
    let manager_access = ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE;
    let manager = ServiceManager::local_computer(None::<&str>, manager_access)
        .map_err(ServiceError::Manager)?;
    // SCM requires START access when a configured failure action restarts the
    // service, even though setting the policy itself uses CHANGE_CONFIG.
    let service_access = ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::QUERY_CONFIG
        | ServiceAccess::DELETE
        | ServiceAccess::START;
    let (service, created) = match manager.create_service(service_info, service_access) {
        Ok(service) => (service, true),
        Err(Error::Winapi(error)) if error.raw_os_error() == Some(ERROR_SERVICE_EXISTS) => {
            let service = manager
                .open_service(SERVICE_NAME, service_access)
                .map_err(ServiceError::Manager)?;
            let existing = service.query_config().map_err(ServiceError::Manager)?;
            if unquote_binary_path(&existing.executable_path) != executable_path {
                return Err(ServiceError::ExistingService(format!(
                    "it is registered to {}",
                    existing.executable_path.display()
                )));
            }
            let is_local_system = existing
                .account_name
                .as_deref()
                .is_some_and(|account| account.eq_ignore_ascii_case("LocalSystem"));
            if !is_local_system {
                return Err(ServiceError::ExistingService(
                    "it is not configured to run as LocalSystem".to_owned(),
                ));
            }
            service
                .change_config(service_info)
                .map_err(ServiceError::Manager)?;
            (service, false)
        }
        Err(error) => return Err(ServiceError::Manager(error)),
    };
    let configure = service
        .set_description(SERVICE_DESCRIPTION)
        .and_then(|()| service.set_config_service_sid_info(ServiceSidType::Unrestricted))
        .and_then(|()| service.set_preshutdown_timeout(PRESHUTDOWN_TIMEOUT))
        .and_then(|()| set_required_privileges(&service, &REQUIRED_PRIVILEGES))
        .and_then(|()| service.update_failure_actions(failure_actions()))
        .and_then(|()| service.set_failure_actions_on_non_crash_failures(true));
    if let Err(configure) = configure {
        return configuration_error(&service, created, configure);
    }

    Ok(())
}

fn failure_actions() -> ServiceFailureActions {
    let mut actions = FAILURE_RESTART_DELAYS
        .into_iter()
        .map(|delay| ServiceAction {
            action_type: ServiceActionType::Restart,
            delay,
        })
        .collect::<Vec<_>>();
    // SCM repeats the final action for subsequent failures. A terminal no-op
    // therefore bounds recovery to the three restart attempts above.
    actions.push(ServiceAction {
        action_type: ServiceActionType::None,
        delay: Duration::ZERO,
    });

    ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(FAILURE_COUNT_RESET_PERIOD),
        reboot_msg: Some(OsString::new()),
        command: Some(OsString::new()),
        actions: Some(actions),
    }
}

#[allow(
    unsafe_code,
    reason = "windows-service does not wrap SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO; the service \
              handle and mutable MULTI_SZ remain valid for the duration of ChangeServiceConfig2W."
)]
fn set_required_privileges(service: &Service, privileges: &[&str]) -> Result<(), Error> {
    let mut privileges = encode_multi_sz(privileges);
    let info = SERVICE_REQUIRED_PRIVILEGES_INFOW {
        pmszRequiredPrivileges: privileges.as_mut_ptr(),
    };

    // SAFETY: `service.raw_handle()` is a live SCM service handle with
    // CHANGE_CONFIG access. `info` points to a writable, double-NUL-terminated
    // MULTI_SZ retained until the synchronous call returns.
    let changed = unsafe {
        ChangeServiceConfig2W(
            service.raw_handle(),
            SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO,
            ptr::from_ref(&info).cast::<c_void>(),
        )
    };
    if changed == 0 {
        return Err(Error::Winapi(io::Error::last_os_error()));
    }

    Ok(())
}

fn encode_multi_sz(values: &[&str]) -> Vec<u16> {
    let mut encoded = Vec::new();
    for value in values {
        encoded.extend(value.encode_utf16());
        encoded.push(0);
    }
    encoded.push(0);
    encoded
}

fn configuration_error(
    service: &Service,
    created: bool,
    configure: Error,
) -> Result<(), ServiceError> {
    if !created {
        return Err(ServiceError::Manager(configure));
    }

    match service.delete() {
        Ok(()) => Err(ServiceError::Manager(configure)),
        Err(rollback) => Err(ServiceError::Rollback {
            configure,
            rollback,
        }),
    }
}

/// Stops and removes the daemon's SCM registration.
///
/// The operation is idempotent. SCM may retain the registration briefly after
/// this function returns while another process still holds a service handle.
/// When `purge_client_group` is true, the client group is also removed if
/// installer metadata proves ownership and the group has no members.
///
/// # Errors
///
/// Returns an error if SCM cannot be opened, the running service cannot be
/// stopped, or SCM rejects the deletion request.
pub fn uninstall(purge_client_group: bool) -> Result<(), ServiceError> {
    if purge_client_group {
        validate_client_group_purge()?;
    }

    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(ServiceError::Manager)?;
    let service = match manager
        .open_service(SERVICE_NAME, ServiceAccess::STOP | ServiceAccess::DELETE)
    {
        Ok(service) => service,
        Err(Error::Winapi(error)) if error.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST) => {
            remove_event_source_owned_by_installer()?;
            if purge_client_group {
                purge_client_group_owned_by_installer()?;
            }
            return Ok(());
        }
        Err(error) => return Err(ServiceError::Manager(error)),
    };

    match service.stop() {
        Ok(_) => {}
        Err(Error::Winapi(error)) if error.raw_os_error() == Some(ERROR_SERVICE_NOT_ACTIVE) => {}
        Err(error) => return Err(ServiceError::Manager(error)),
    }

    service.delete().map_err(ServiceError::Manager)?;
    remove_event_source_owned_by_installer()?;
    if purge_client_group {
        purge_client_group_owned_by_installer()?;
    }
    Ok(())
}

fn remove_event_source_owned_by_installer() -> Result<(), ServiceError> {
    if !owns_event_log_source(SOURCE_NAME).map_err(ServiceError::InstallerMetadata)? {
        return Ok(());
    }

    remove_event_source().map_err(ServiceError::InstallerMetadata)?;
    remove_owned_event_log_source(SOURCE_NAME).map_err(ServiceError::InstallerMetadata)
}

fn validate_client_group_purge() -> Result<(), ServiceError> {
    let owned = owns_client_group(CLIENT_GROUP_NAME).map_err(ServiceError::InstallerMetadata)?;
    if let Some(reason) = client_group_purge_error(owned, true) {
        return Err(ServiceError::UnsafeClientGroupPurge(reason.to_owned()));
    }

    let empty = local_group_is_empty(CLIENT_GROUP_NAME).map_err(ServiceError::ClientGroup)?;
    if let Some(reason) = client_group_purge_error(owned, empty) {
        return Err(ServiceError::UnsafeClientGroupPurge(reason.to_owned()));
    }
    Ok(())
}

const fn client_group_purge_error(owned: bool, empty: bool) -> Option<&'static str> {
    if !owned {
        Some("installer ownership metadata is absent or does not match")
    } else if !empty {
        Some("the group still has members")
    } else {
        None
    }
}

fn purge_client_group_owned_by_installer() -> Result<(), ServiceError> {
    // Recheck immediately before deletion so a membership change during SCM
    // shutdown cannot silently weaken the explicit purge guard.
    validate_client_group_purge()?;
    delete_local_group(CLIENT_GROUP_NAME).map_err(ServiceError::ClientGroup)?;
    remove_owned_client_group(CLIENT_GROUP_NAME).map_err(ServiceError::InstallerMetadata)
}

fn service_info(executable_path: PathBuf) -> ServiceInfo {
    ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path,
        launch_arguments: Vec::new(),
        dependencies: Vec::new(),
        account_name: None,
        account_password: None,
    }
}

fn validate_install_path(
    executable_path: &Path,
    allow_development_path: bool,
) -> Result<(), ServiceError> {
    if allow_development_path {
        return Ok(());
    }

    let program_files = default_path::program_files()
        .and_then(canonicalize)
        .map_err(ServiceError::ExecutablePath)?;
    if executable_path.starts_with(program_files) {
        return Ok(());
    }

    Err(ServiceError::ExecutablePath(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "service executable is outside Program Files; use --allow-development-path only on a \
         throwaway development machine",
    )))
}

#[cfg(test)]
#[path = "install_tests.rs"]
mod tests;
