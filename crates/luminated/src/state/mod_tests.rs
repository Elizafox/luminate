// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use luminate_core::colour::Colour;
use luminate_core::device::DeviceId;
use luminate_core::element::ElementId;
use luminate_core::group::GroupId;
use luminate_core::rgb::Rgb;
use luminate_core::surface::SurfaceId;
use luminate_core::target::TargetId;

use crate::error::DaemonError;

use super::tests_support::*;

#[test]
fn capabilities_for_target_resolves_each_scope() {
    let state = demo_state();
    let device = DeviceId::new("demo-kbd");

    assert!(
        state
            .capabilities_for_target(&TargetId::Device(device.clone()))
            .is_some()
    );
    assert!(
        state
            .capabilities_for_target(&TargetId::Surface {
                device: device.clone(),
                surface: SurfaceId::new("main"),
            })
            .is_some()
    );
    assert!(
        state
            .capabilities_for_target(&TargetId::Element {
                device: device.clone(),
                surface: SurfaceId::new("main"),
                element: ElementId::new("logo"),
            })
            .is_some()
    );
    assert!(
        state
            .capabilities_for_target(&TargetId::Group {
                device: device.clone(),
                group: GroupId::new("all"),
            })
            .is_some()
    );
    assert!(
        state
            .capabilities_for_target(&TargetId::Surface {
                device,
                surface: SurfaceId::new("missing"),
            })
            .is_none()
    );
}

#[test]
fn topology_withdrawal_retains_device_state_for_reappearance() {
    let mut state = demo_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));
    state
        .set_static_colour_for_test(target.clone(), Colour::rgb(Rgb::new(1, 2, 3)))
        .expect("set device colour");

    assert_eq!(
        state.replace_devices_preserving_withdrawn_state(Vec::new()),
        0
    );
    assert_eq!(state.target_states().len(), 1);
    assert_eq!(state.target_states()[0].target, target);

    let devices = demo_state().devices();
    assert_eq!(state.replace_devices_preserving_withdrawn_state(devices), 0);
    assert_eq!(state.target_states().len(), 1);
    assert!(state.ensure_target_state_known(&target).is_ok());
}

#[test]
fn withdrawn_device_state_can_be_purged_but_active_state_cannot() {
    let device = DeviceId::new("demo-kbd");
    let target = TargetId::Device(device.clone());
    let mut state = demo_state();
    state
        .set_static_colour_for_test(target.clone(), Colour::rgb(Rgb::new(3, 4, 5)))
        .expect("set device colour");

    assert!(matches!(
        state.purge_withdrawn_device(&device),
        Err(DaemonError::InvalidArgument { .. })
    ));

    assert_eq!(
        state.replace_devices_preserving_withdrawn_state(Vec::new()),
        0
    );
    assert_eq!(
        state
            .purge_withdrawn_device(&device)
            .expect("purge withdrawn state"),
        1
    );
    assert!(state.target_states().is_empty());
    assert!(matches!(
        state.purge_withdrawn_device(&device),
        Err(DaemonError::TargetNotFound(_))
    ));
}

#[test]
fn withdrawn_device_ids_are_unique_sorted_and_rechecked_before_purge() {
    let device = DeviceId::new("demo-kbd");
    let target = TargetId::Device(device.clone());
    let mut state = demo_state();
    state
        .set_static_colour_for_test(target, Colour::rgb(Rgb::new(3, 4, 5)))
        .expect("set device colour");

    state.replace_devices_preserving_withdrawn_state(Vec::new());
    assert_eq!(state.withdrawn_device_ids(), vec![device.clone()]);

    state.replace_devices_preserving_withdrawn_state(demo_state().devices());
    assert!(state.withdrawn_device_ids().is_empty());
    assert!(matches!(
        state.purge_withdrawn_device(&device),
        Err(DaemonError::InvalidArgument { .. })
    ));
    assert_eq!(state.target_states().len(), 1);
}
