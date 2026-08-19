// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};

use luminate_core::rgb::Rgb;

static NEXT_TEST_ID: AtomicUsize = AtomicUsize::new(0);

fn test_target() -> PluginTarget {
    PluginTarget::Device {
        device: format!(
            "shm-registry-{}-{}",
            process::id(),
            NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed)
        ),
    }
}

#[test]
fn registry_applies_only_matching_full_frames_and_epochs() {
    let registry = ShmPublisherRegistry::default();
    let target = test_target();
    let envelope = FrameEnvelope {
        generation: 7,
        sequence: 1,
        payload: FramePayload::Full(vec![Colour::rgb(Rgb::new(1, 2, 3))]),
        commit: true,
    };

    assert!(!registry.apply(&target, &envelope, 1));
    registry
        .open_and_insert(
            "test-plugin",
            &target.to_string(),
            ShmPixelFormat::Rgb8,
            u32::try_from(size_of::<ShmFrameHeader>() + 3).expect("small test segment"),
            7,
            1,
        )
        .expect("open test shared-memory services");
    assert!(registry.has_active_stream(&target));
    assert!(!registry.apply(
        &target,
        &FrameEnvelope {
            generation: 6,
            ..envelope.clone()
        },
        1,
    ));
    assert!(registry.apply(&target, &envelope, 1));

    assert!(!registry.apply(&target, &envelope, 2));
    assert!(!registry.has_active_stream(&target));
}

#[test]
fn registry_rejects_partial_frames_and_generation_mismatches_without_state_changes() {
    let registry = ShmPublisherRegistry::default();
    let target = test_target();
    registry
        .open_and_insert(
            "test-plugin",
            &target.to_string(),
            ShmPixelFormat::Rgb8,
            u32::try_from(size_of::<ShmFrameHeader>() + 3).expect("small test segment"),
            9,
            3,
        )
        .expect("open test shared-memory services");
    let partial = FrameEnvelope {
        generation: 9,
        sequence: 1,
        payload: FramePayload::Partial(Vec::new()),
        commit: false,
    };

    assert!(!registry.apply(&target, &partial, 3));
    assert!(registry.has_active_stream(&target));
    assert!(!registry.apply(
        &target,
        &FrameEnvelope {
            generation: 10,
            ..partial
        },
        3,
    ));
    assert!(registry.has_active_stream(&target));
}
