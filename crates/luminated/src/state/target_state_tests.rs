// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use luminate_core::colour::Colour;
use luminate_core::device::DeviceId;
use luminate_core::effect::Effect;
use luminate_core::element::ElementId;
use luminate_core::group::GroupId;
use luminate_core::rgb::Rgb;
use luminate_core::surface::SurfaceId;
use luminate_core::target::TargetId;

use crate::error::DaemonError;

use super::super::tests_support::*;
use super::{TargetState, TargetStateEntry};

impl super::DaemonState {
    pub(crate) fn set_static_colour_for_test(
        &mut self,
        target: TargetId,
        colour: Colour,
    ) -> Result<(), DaemonError> {
        self.set_effect(target, Effect::Static { colour })
    }
}

#[test]
fn restore_persisted_keeps_targets_that_still_resolve() {
    let mut state = demo_state();
    let device = DeviceId::new("demo-kbd");

    let persisted = vec![
        TargetStateEntry {
            target: TargetId::Device(device.clone()),
            state: TargetState::Effect(Effect::Static {
                colour: Colour::rgb(Rgb::new(1, 2, 3)),
            }),
        },
        TargetStateEntry {
            target: TargetId::Device(DeviceId::new("missing-device")),
            state: TargetState::Effect(Effect::Off),
        },
    ];

    let (restored, dropped) = state.restore_persisted(persisted, true);
    assert_eq!(restored, 1);
    assert_eq!(dropped, 1);
    assert!(has_target(&state, &TargetId::Device(device)));
}

#[test]
fn static_effect_rejects_target_with_unknown_surface_on_a_real_device() {
    let mut state = demo_state();
    let target = TargetId::Surface {
        device: DeviceId::new("demo-kbd"),
        surface: SurfaceId::new("missing"),
    };

    let result = state.set_static_colour_for_test(target, Colour::rgb(Rgb::new(1, 2, 3)));

    assert!(
        result.is_err(),
        "a nonexistent surface under a real device must not be silently accepted"
    );
    assert!(state.target_states().is_empty());
}

#[test]
fn save_current_requires_exact_known_target_state() {
    let mut state = demo_state();
    let device = DeviceId::new("demo-kbd");
    let device_target = TargetId::Device(device.clone());
    let surface_target = TargetId::Surface {
        device,
        surface: SurfaceId::new("main"),
    };

    let error = state
        .ensure_target_state_known(&device_target)
        .expect_err("missing cached state should be unknown");
    assert!(matches!(error, DaemonError::UnknownState { .. }));

    state
        .set_static_colour_for_test(surface_target, Colour::rgb(Rgb::new(1, 2, 3)))
        .expect("surface colour should set");

    let error = state
        .ensure_target_state_known(&device_target)
        .expect_err("sibling/broader state is still unknown for exact save target");
    assert!(matches!(error, DaemonError::UnknownState { .. }));

    state
        .set_effect(device_target.clone(), Effect::Spectrum { period_ms: 1000 })
        .expect("device effect should set");
    state
        .ensure_target_state_known(&device_target)
        .expect("exact cached state should be known");
}

#[test]
fn narrower_surface_retains_broad_device_overlay() {
    let mut state = demo_state();
    let device = DeviceId::new("demo-kbd");
    let device_target = TargetId::Device(device.clone());
    let surface = TargetId::Surface {
        device,
        surface: SurfaceId::new("main"),
    };

    state
        .set_static_colour_for_test(device_target.clone(), Colour::rgb(Rgb::new(4, 5, 6)))
        .expect("device colour should set");
    state
        .set_static_colour_for_test(surface.clone(), Colour::rgb(Rgb::new(1, 2, 3)))
        .expect("surface colour should set");

    assert_eq!(state.target_states().len(), 2);
    assert_eq!(state.target_states()[0].target, device_target);
    assert_eq!(state.target_states()[1].target, surface);
}

#[test]
fn independent_brightness_preserves_same_target_appearance_across_restart() {
    let mut state = validation_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));
    let colour = Colour::rgb(Rgb::new(1, 2, 3));

    state
        .set_static_colour_for_test(target.clone(), colour.clone())
        .expect("device colour should set");
    state
        .set_brightness(target.clone(), 17)
        .expect("device brightness should set");

    assert_eq!(state.target_states().len(), 2);
    assert!(matches!(
        &state.target_states()[0].state,
        TargetState::Effect(Effect::Static { colour: cached }) if cached == &colour
    ));
    assert!(matches!(
        state.target_states()[1].state,
        TargetState::Brightness(17)
    ));

    let mut restarted = validation_state();
    assert_eq!(
        restarted.restore_persisted(state.target_states().to_vec(), true),
        (2, 0)
    );
    assert_eq!(restarted.target_states().len(), 2);
}

#[test]
fn broad_brightness_preserves_per_element_appearance() {
    let mut state = demo_state();
    let device = DeviceId::new("demo-kbd");
    let element = TargetId::Element {
        device: device.clone(),
        surface: SurfaceId::new("main"),
        element: ElementId::new("logo"),
    };
    let colour = Colour::rgb(Rgb::new(4, 5, 6));

    state
        .set_static_colour_for_test(element.clone(), colour)
        .expect("element colour should set");
    state
        .set_brightness(TargetId::Device(device), 23)
        .expect("device brightness should set");

    assert_eq!(state.target_states().len(), 2);
    assert!(has_target(&state, &element));
    assert!(matches!(
        state.target_states()[1].state,
        TargetState::Brightness(23)
    ));
}

#[test]
fn clear_remains_an_all_facet_replay_barrier() {
    let mut state = demo_state();
    let target = TargetId::Device(DeviceId::new("demo-kbd"));

    state
        .set_static_colour_for_test(target.clone(), Colour::rgb(Rgb::new(1, 2, 3)))
        .expect("device colour should set");
    state
        .set_brightness(target.clone(), 17)
        .expect("device brightness should set");
    state
        .clear_target(&target)
        .expect("device clear should set");
    state
        .set_static_colour_for_test(target, Colour::rgb(Rgb::new(7, 8, 9)))
        .expect("post-clear colour should set");

    assert_eq!(state.target_states().len(), 2);
    assert!(matches!(state.target_states()[0].state, TargetState::Clear));
    assert!(matches!(
        state.target_states()[1].state,
        TargetState::Effect(Effect::Static { .. })
    ));
}

#[test]
fn setting_group_removes_overlapping_surface_state() {
    let mut state = demo_state();
    let device = DeviceId::new("demo-kbd");
    let surface = TargetId::Surface {
        device: device.clone(),
        surface: SurfaceId::new("main"),
    };
    let group = TargetId::Group {
        device,
        group: GroupId::new("all"),
    };

    state
        .set_static_colour_for_test(surface.clone(), Colour::rgb(Rgb::new(1, 2, 3)))
        .expect("surface colour should set");
    state
        .set_effect(group.clone(), Effect::Spectrum { period_ms: 1000 })
        .expect("group effect should set");

    assert!(!has_target(&state, &surface));
    assert!(has_target(&state, &group));
}

#[test]
fn clearing_group_supersedes_surface_with_clear_overlay() {
    let mut state = demo_state();
    let device = DeviceId::new("demo-kbd");
    let surface = TargetId::Surface {
        device: device.clone(),
        surface: SurfaceId::new("main"),
    };
    let group = TargetId::Group {
        device,
        group: GroupId::new("all"),
    };

    state
        .set_static_colour_for_test(surface.clone(), Colour::rgb(Rgb::new(1, 2, 3)))
        .expect("surface colour should set");
    state.clear_target(&group).expect("group clear should set");

    assert!(!has_target(&state, &surface));
    assert_eq!(state.target_states().len(), 1);
    assert_eq!(state.target_states()[0].target, group);
    assert!(matches!(state.target_states()[0].state, TargetState::Clear));
}

#[test]
fn restore_persisted_broad_targets_win_old_conflicts() {
    let mut state = demo_state();
    let device = DeviceId::new("demo-kbd");
    let surface = TargetId::Surface {
        device: device.clone(),
        surface: SurfaceId::new("main"),
    };
    let group = TargetId::Group {
        device,
        group: GroupId::new("all"),
    };

    let persisted = vec![
        TargetStateEntry {
            target: surface.clone(),
            state: TargetState::Effect(Effect::Static {
                colour: Colour::rgb(Rgb::new(1, 2, 3)),
            }),
        },
        TargetStateEntry {
            target: group.clone(),
            state: TargetState::Effect(Effect::Static {
                colour: Colour::rgb(Rgb::new(4, 5, 6)),
            }),
        },
    ];

    let (restored, dropped) = state.restore_persisted(persisted, false);

    assert_eq!(restored, 2);
    assert_eq!(dropped, 0);
    assert!(!has_target(&state, &surface));
    assert!(has_target(&state, &group));
}

#[test]
fn restart_preserves_unaffected_sibling_after_narrow_mutation() {
    let mut state = demo_state();
    let device_id = DeviceId::new("demo-kbd");
    let device = TargetId::Device(device_id.clone());
    let changed = TargetId::Element {
        device: device_id.clone(),
        surface: SurfaceId::new("main"),
        element: ElementId::new("logo"),
    };
    let sibling = TargetId::Element {
        device: device_id,
        surface: SurfaceId::new("main"),
        element: ElementId::new("topology-only"),
    };
    let red = Colour::rgb(Rgb::new(255, 0, 0));
    let blue = Colour::rgb(Rgb::new(0, 0, 255));

    state
        .set_static_colour_for_test(device, red.clone())
        .expect("set broad colour");
    state
        .set_static_colour_for_test(changed.clone(), blue.clone())
        .expect("set child colour");

    let mut restarted = demo_state();
    let (restored, dropped) = restarted.restore_persisted(state.target_states().to_vec(), true);
    assert_eq!((restored, dropped), (2, 0));
    assert!(matches!(
        effective_state_for_target(&restarted, &sibling),
        Some(TargetState::Effect(Effect::Static { colour })) if colour == &red
    ));
    assert!(matches!(
        effective_state_for_target(&restarted, &changed),
        Some(TargetState::Effect(Effect::Static { colour })) if colour == &blue
    ));
}

#[test]
fn restart_preserves_unaffected_sibling_after_narrow_clear() {
    let mut state = demo_state();
    let device_id = DeviceId::new("demo-kbd");
    let device = TargetId::Device(device_id.clone());
    let cleared = TargetId::Element {
        device: device_id.clone(),
        surface: SurfaceId::new("main"),
        element: ElementId::new("logo"),
    };
    let sibling = TargetId::Element {
        device: device_id,
        surface: SurfaceId::new("main"),
        element: ElementId::new("topology-only"),
    };
    let red = Colour::rgb(Rgb::new(255, 0, 0));

    state
        .set_static_colour_for_test(device, red.clone())
        .expect("set broad colour");
    state.clear_target(&cleared).expect("clear child");

    let mut restarted = demo_state();
    let (restored, dropped) = restarted.restore_persisted(state.target_states().to_vec(), true);
    assert_eq!((restored, dropped), (2, 0));
    assert!(matches!(
        effective_state_for_target(&restarted, &sibling),
        Some(TargetState::Effect(Effect::Static { colour })) if colour == &red
    ));
    assert!(matches!(
        effective_state_for_target(&restarted, &cleared),
        Some(TargetState::Clear)
    ));
    assert!(matches!(
        restarted.ensure_target_state_known(&cleared),
        Err(DaemonError::UnknownState { .. })
    ));
}

#[test]
fn startup_retains_state_for_absent_dynamic_device() {
    let target = TargetId::Device(DeviceId::new("offline-bulb"));
    let persisted = vec![TargetStateEntry {
        target: target.clone(),
        state: TargetState::Effect(Effect::Static {
            colour: Colour::rgb(Rgb::new(3, 4, 5)),
        }),
    }];
    let mut state = demo_state();

    let counts = state.restore_persisted_retaining_withdrawn(persisted, true);
    assert_eq!(counts, (0, 1, 0));
    assert_eq!(state.target_states()[0].target, target);
}
