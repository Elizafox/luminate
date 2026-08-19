// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Service control handler registration, SCM status reporting, and
//! power/device-event forwarding to the daemon.

use std::ffi::c_void;
use std::io;
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::mpsc;
use windows_sys::Win32::Devices::HumanInterfaceDevice::GUID_DEVINTERFACE_HID;
use windows_sys::Win32::Devices::Usb::GUID_DEVINTERFACE_USB_DEVICE;
use windows_sys::Win32::Foundation::{ERROR_CALL_NOT_IMPLEMENTED, NO_ERROR};
use windows_sys::Win32::System::Services::{
    RegisterServiceCtrlHandlerExW, SERVICE_ACCEPT_POWEREVENT, SERVICE_ACCEPT_PRESHUTDOWN,
    SERVICE_ACCEPT_STOP, SERVICE_CONTROL_DEVICEEVENT, SERVICE_CONTROL_INTERROGATE,
    SERVICE_CONTROL_POWEREVENT, SERVICE_CONTROL_PRESHUTDOWN, SERVICE_CONTROL_STOP, SERVICE_RUNNING,
    SERVICE_START_PENDING, SERVICE_STATUS, SERVICE_STATUS_HANDLE, SERVICE_STOP_PENDING,
    SERVICE_STOPPED, SERVICE_WIN32_OWN_PROCESS, SetServiceStatus,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DBT_DEVTYP_DEVICEINTERFACE, DEV_BROADCAST_DEVICEINTERFACE_W, DEVICE_NOTIFY_SERVICE_HANDLE,
    HDEVNOTIFY, RegisterDeviceNotificationW, UnregisterDeviceNotification,
};
use windows_sys::core::GUID;

use crate::power::{PowerEventReceiver, PowerEventSender};
use crate::windows::power::{interpret as interpret_power_event, interpret_device_event};

use super::error::{ServiceError, raw_error_code};
use super::{ERROR_SERVICE_SPECIFIC_ERROR, SERVICE_NAME, START_WAIT_HINT, STOP_WAIT_HINT};

/// Inputs supplied to the daemon when SCM starts the service.
pub struct ServiceContext {
    /// Receives exactly one shutdown request after the first STOP or
    /// PRESHUTDOWN control.
    pub shutdown: mpsc::Receiver<ServiceShutdown>,

    /// Receives a [`SystemPowerEvent`](crate::power::SystemPowerEvent) each
    /// time the control handler observes and interprets a
    /// `SERVICE_CONTROL_POWEREVENT`.
    pub power_events: PowerEventReceiver,

    /// Reports daemon lifecycle changes to SCM.
    pub status: StatusReporter,
}

/// The terminal SCM control that requested graceful service shutdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceShutdown {
    /// An administrator or service-management tool requested an ordinary stop.
    Stop,

    /// Windows requested shutdown during the system preshutdown phase.
    Preshutdown,
}

/// Cloneable service-status reporter used by the daemon lifecycle adapter.
#[derive(Clone)]
pub struct StatusReporter {
    shared: &'static HandlerState,
}

impl StatusReporter {
    /// Advances the genuine startup-progress checkpoint.
    ///
    /// # Errors
    ///
    /// Returns an error if the cached status is unavailable or SCM rejects
    /// the status update.
    pub fn startup_progress(&self) -> Result<(), ServiceError> {
        self.shared.update(advance_startup)
    }

    /// Reports that the daemon is ready to accept requests.
    ///
    /// # Errors
    ///
    /// Returns an error if the cached status is unavailable or SCM rejects
    /// the status update.
    pub fn running(&self) -> Result<(), ServiceError> {
        self.shared.update(mark_running)
    }

    /// Reports terminal service status with a Win32 exit code.
    ///
    /// # Errors
    ///
    /// Returns an error if the cached status is unavailable or SCM rejects
    /// the status update.
    pub fn stopped(&self, exit_code: u32) -> Result<(), ServiceError> {
        self.shared.update(|status| mark_stopped(status, exit_code))
    }

    /// Reports terminal service status with a service-specific exit code.
    ///
    /// # Errors
    ///
    /// Returns an error if the cached status is unavailable or SCM rejects
    /// the status update.
    pub fn stopped_with_error(&self, exit_code: u32) -> Result<(), ServiceError> {
        self.shared
            .update(|status| mark_stopped_with_error(status, exit_code))
    }
}

struct HandlerState {
    status_handle: AtomicUsize,
    status: Mutex<SERVICE_STATUS>,
    shutdown_sent: AtomicBool,
    shutdown: mpsc::Sender<ServiceShutdown>,
    power_events: PowerEventSender,

    /// Handles from [`register_device_interface_notification`], zero for a
    /// class that failed to register. Unregistered exactly once, when
    /// shutdown begins (see [`HandlerState::request_shutdown`]).
    device_notify_hid: AtomicUsize,
    device_notify_usb: AtomicUsize,
}

impl HandlerState {
    fn update(&self, update: impl FnOnce(&mut SERVICE_STATUS)) -> Result<(), ServiceError> {
        let mut status = self.status.lock().expect("service status lock poisoned");
        update(&mut status);
        self.report(&status)
    }

    #[allow(
        unsafe_code,
        reason = "SetServiceStatus has no safe standard-library wrapper; the registered handle \
                  remains live and the cached status is locked for the duration of the call."
    )]
    fn report(&self, status: &SERVICE_STATUS) -> Result<(), ServiceError> {
        // SAFETY: `status_handle` comes from successful handler registration
        // and remains valid until SCM observes SERVICE_STOPPED. `status` is a
        // fully initialized value retained for the duration of the call.
        let reported = unsafe {
            SetServiceStatus(
                self.status_handle.load(Ordering::Acquire) as SERVICE_STATUS_HANDLE,
                ptr::from_ref(status),
            )
        };
        if reported == 0 {
            return Err(ServiceError::ReportStatus(io::Error::last_os_error()));
        }

        Ok(())
    }

    /// Handles a `SERVICE_CONTROL_POWEREVENT`, forwarding it to the daemon if
    /// it interprets as a [`SystemPowerEvent`](crate::power::SystemPowerEvent).
    ///
    /// Windows has offered services no way to veto a suspend since Vista, so
    /// unlike [`Self::request_shutdown`] this always returns `NO_ERROR`: a
    /// dropped send means the daemon side of the channel is already gone,
    /// which happens naturally during shutdown and is not itself an error.
    fn request_powerevent(&self, event_type: u32) -> u32 {
        if let Some(event) = interpret_power_event(event_type) {
            let _ = self.power_events.send(event);
        }

        NO_ERROR
    }

    /// Handles a `SERVICE_CONTROL_DEVICEEVENT`, forwarding it to the daemon
    /// if it interprets as a [`SystemPowerEvent`](crate::power::SystemPowerEvent).
    ///
    /// Only arrives for the interface classes
    /// [`register_device_interface_notification`] registered; always
    /// returns `NO_ERROR` for the same reason [`Self::request_powerevent`]
    /// does.
    fn request_deviceevent(&self, event_type: u32) -> u32 {
        if let Some(event) = interpret_device_event(event_type) {
            let _ = self.power_events.send(event);
        }

        NO_ERROR
    }

    fn request_shutdown(&self, request: ServiceShutdown) -> u32 {
        if !begin_shutdown(&self.shutdown_sent) {
            return NO_ERROR;
        }

        unregister_device_interface_notification(&self.device_notify_hid);
        unregister_device_interface_notification(&self.device_notify_usb);

        let status_code = self
            .update(mark_stop_pending)
            .map_or_else(|error| raw_error_code(&error), |()| NO_ERROR);

        match self.shutdown.try_send(request) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => status_code,
            Err(mpsc::error::TrySendError::Closed(_)) => 1,
        }
    }

    fn interrogate(&self) -> u32 {
        let status = self.status.lock().expect("service status lock poisoned");

        self.report(&status)
            .map_or_else(|error| raw_error_code(&error), |()| NO_ERROR)
    }
}

#[allow(
    unsafe_code,
    reason = "RegisterServiceCtrlHandlerExW has no safe wrapper with a callback lifetime that \
              survives repeated terminal controls; the context is validated, initialized, and \
              deliberately retained for process lifetime."
)]
pub(super) fn register_handler() -> Result<ServiceContext, ServiceError> {
    let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
    let (power_tx, power_rx) = mpsc::unbounded_channel();
    let initial_status = initial_status();

    let mut service_name: Vec<u16> = SERVICE_NAME.encode_utf16().chain(Some(0)).collect();
    let provisional = Box::new(HandlerState {
        status_handle: AtomicUsize::new(0),
        status: Mutex::new(initial_status),
        shutdown_sent: AtomicBool::new(false),
        shutdown: shutdown_tx,
        power_events: power_tx,
        device_notify_hid: AtomicUsize::new(0),
        device_notify_usb: AtomicUsize::new(0),
    });
    let context = Box::into_raw(provisional);
    // SAFETY: the UTF-16 service name is NUL-terminated, the callback has the
    // required ABI, and `context` remains allocated for the process lifetime.
    let status_handle = unsafe {
        RegisterServiceCtrlHandlerExW(
            service_name.as_mut_ptr(),
            Some(control_handler),
            context.cast::<c_void>(),
        )
    };
    if status_handle.is_null() {
        // SAFETY: registration failed, so SCM cannot have retained or invoked
        // the context pointer and this allocation is still uniquely owned.
        let _ = unsafe { Box::from_raw(context) };
        return Err(ServiceError::RegisterHandler(io::Error::last_os_error()));
    }

    // SAFETY: `context` points to the allocation deliberately retained for
    // process lifetime. Publishing the handle completes its initialization.
    let shared = unsafe { &*context };
    shared
        .status_handle
        .store(status_handle as usize, Ordering::Release);
    shared.report(&initial_status)?;

    // Best-effort, like every other automatic rescan trigger: a class that
    // fails to register is logged and does not contribute a `DevicesChanged`
    // of its own, as on a Linux machine with no reachable udev.
    shared.device_notify_hid.store(
        register_device_interface_notification(status_handle, GUID_DEVINTERFACE_HID) as usize,
        Ordering::Release,
    );
    shared.device_notify_usb.store(
        register_device_interface_notification(status_handle, GUID_DEVINTERFACE_USB_DEVICE)
            as usize,
        Ordering::Release,
    );

    Ok(ServiceContext {
        shutdown: shutdown_rx,
        power_events: power_rx,
        status: StatusReporter { shared },
    })
}

/// Registers for arrival/removal notifications of device interfaces in
/// `class_guid`, delivered to this service's control handler as
/// `SERVICE_CONTROL_DEVICEEVENT`. Returns a null handle and logs on failure.
/// See [`register_handler`]'s call site for why this is not fatal.
#[allow(
    unsafe_code,
    reason = "RegisterDeviceNotificationW has no safe wrapper; `status_handle` is a live handle \
              from a successful RegisterServiceCtrlHandlerExW call and `filter` is a live, fully \
              initialized DEV_BROADCAST_DEVICEINTERFACE_W for the duration of the call."
)]
fn register_device_interface_notification(
    status_handle: SERVICE_STATUS_HANDLE,
    class_guid: GUID,
) -> HDEVNOTIFY {
    let filter = DEV_BROADCAST_DEVICEINTERFACE_W {
        dbcc_size: device_interface_filter_size(),
        dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE,
        dbcc_reserved: 0,
        dbcc_classguid: class_guid,
        dbcc_name: [0],
    };

    // SAFETY: see the function-level `reason` above.
    let handle = unsafe {
        RegisterDeviceNotificationW(
            status_handle,
            (&raw const filter).cast::<c_void>(),
            DEVICE_NOTIFY_SERVICE_HANDLE,
        )
    };
    if handle.is_null() {
        tracing::info!(
            error = %io::Error::last_os_error(),
            "could not register for device-interface notifications; hotplug for this device \
             class will not trigger an automatic rescan"
        );
    }

    handle
}

/// `DEV_BROADCAST_DEVICEINTERFACE_W`'s size as `dbcc_size` expects it.
///
/// The struct is a few dozen bytes, nowhere near `u32::MAX`, so narrowing
/// here cannot lose anything.
#[allow(
    clippy::cast_possible_truncation,
    reason = "DEV_BROADCAST_DEVICEINTERFACE_W is a few dozen bytes, far inside u32"
)]
const fn device_interface_filter_size() -> u32 {
    size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32
}

/// Unregisters a device-interface notification handle stored by
/// [`register_device_interface_notification`], if it registered
/// successfully. Swaps the stored value to zero first so a handle is never
/// unregistered twice, even if called concurrently.
#[allow(
    unsafe_code,
    reason = "UnregisterDeviceNotification has no safe wrapper; `raw` is a handle returned by a \
              prior successful RegisterDeviceNotificationW call, and the swap above ensures it is \
              passed here at most once."
)]
fn unregister_device_interface_notification(handle: &AtomicUsize) {
    let raw = handle.swap(0, Ordering::AcqRel);
    if raw == 0 {
        return;
    }

    // SAFETY: see the function-level `reason` above.
    let unregistered = unsafe { UnregisterDeviceNotification(raw as HDEVNOTIFY) };
    if unregistered == 0 {
        tracing::warn!(
            error = %io::Error::last_os_error(),
            "failed to unregister a device-interface notification"
        );
    }
}

fn initial_status() -> SERVICE_STATUS {
    SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: SERVICE_START_PENDING,
        dwControlsAccepted: 0,
        dwWin32ExitCode: 0,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 1,
        dwWaitHint: duration_millis(START_WAIT_HINT),
    }
}

fn advance_startup(status: &mut SERVICE_STATUS) {
    if status.dwCurrentState == SERVICE_START_PENDING {
        status.dwCheckPoint = status.dwCheckPoint.saturating_add(1);
    }
}

fn mark_running(status: &mut SERVICE_STATUS) {
    if status.dwCurrentState != SERVICE_START_PENDING {
        return;
    }

    status.dwCurrentState = SERVICE_RUNNING;
    status.dwControlsAccepted =
        SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_PRESHUTDOWN | SERVICE_ACCEPT_POWEREVENT;
    status.dwCheckPoint = 0;
    status.dwWaitHint = 0;
}

fn mark_stop_pending(status: &mut SERVICE_STATUS) {
    status.dwCurrentState = SERVICE_STOP_PENDING;
    status.dwControlsAccepted = 0;
    status.dwCheckPoint = 1;
    status.dwWaitHint = duration_millis(STOP_WAIT_HINT);
}

fn mark_stopped(status: &mut SERVICE_STATUS, exit_code: u32) {
    status.dwCurrentState = SERVICE_STOPPED;
    status.dwControlsAccepted = 0;
    status.dwWin32ExitCode = exit_code;
    status.dwServiceSpecificExitCode = 0;
    status.dwCheckPoint = 0;
    status.dwWaitHint = 0;
}

fn mark_stopped_with_error(status: &mut SERVICE_STATUS, exit_code: u32) {
    mark_stopped(status, ERROR_SERVICE_SPECIFIC_ERROR);
    status.dwServiceSpecificExitCode = exit_code;
}

fn begin_shutdown(shutdown_sent: &AtomicBool) -> bool {
    shutdown_sent
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

#[allow(
    unsafe_code,
    reason = "SCM invokes this FFI callback with the process-lifetime context pointer registered \
              by register_handler."
)]
extern "system" fn control_handler(
    control: u32,
    event_type: u32,
    _event_data: *mut c_void,
    context: *mut c_void,
) -> u32 {
    if context.is_null() {
        return ERROR_CALL_NOT_IMPLEMENTED;
    }
    // SAFETY: registration receives a process-lifetime `HandlerState` pointer.
    let state = unsafe { &*context.cast::<HandlerState>() };
    match control {
        SERVICE_CONTROL_STOP => state.request_shutdown(ServiceShutdown::Stop),
        SERVICE_CONTROL_PRESHUTDOWN => state.request_shutdown(ServiceShutdown::Preshutdown),
        SERVICE_CONTROL_POWEREVENT => state.request_powerevent(event_type),
        SERVICE_CONTROL_DEVICEEVENT => state.request_deviceevent(event_type),
        SERVICE_CONTROL_INTERROGATE => state.interrogate(),
        _ => ERROR_CALL_NOT_IMPLEMENTED,
    }
}

fn duration_millis(duration: Duration) -> u32 {
    u32::try_from(duration.as_millis()).unwrap_or(u32::MAX)
}

#[cfg(test)]
#[path = "handler_tests.rs"]
mod tests;
