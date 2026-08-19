// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use windows_sys::Win32::UI::WindowsAndMessaging::PBT_APMRESUMESUSPEND;

use super::*;

#[test]
fn suspend_interprets_as_a_suspending_event() {
    assert!(matches!(
        interpret(PBT_APMSUSPEND),
        Some(SystemPowerEvent::Suspending(_))
    ));
}

#[test]
fn automatic_resume_interprets_as_resumed() {
    assert!(matches!(
        interpret(PBT_APMRESUMEAUTOMATIC),
        Some(SystemPowerEvent::Resumed)
    ));
}

#[test]
fn a_resume_following_the_automatic_one_is_ignored() {
    assert!(interpret(PBT_APMRESUMESUSPEND).is_none());
}

#[test]
fn an_unrecognised_event_type_is_ignored() {
    assert!(interpret(0).is_none());
}

#[test]
fn device_arrival_interprets_as_devices_changed() {
    assert!(matches!(
        interpret_device_event(DBT_DEVICEARRIVAL),
        Some(SystemPowerEvent::DevicesChanged)
    ));
}

#[test]
fn device_removal_interprets_as_devices_changed() {
    assert!(matches!(
        interpret_device_event(DBT_DEVICEREMOVECOMPLETE),
        Some(SystemPowerEvent::DevicesChanged)
    ));
}

#[test]
fn an_unrecognised_device_event_is_ignored() {
    assert!(interpret_device_event(0).is_none());
}
