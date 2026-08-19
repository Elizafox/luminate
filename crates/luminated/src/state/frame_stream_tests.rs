// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::slice;

use luminate_core::capability::{self, CapabilityScope, CapabilitySet, CctEmulation};
use luminate_core::capability::{
    ColourCapability, HardwareEffectDescriptor, HardwareEffectId, HardwareEffectsCapability,
};
use luminate_core::colour::Colour;
use luminate_core::effect::{Effect, EffectArguments};
use luminate_core::frame::FramePayload;
use luminate_core::rgb::Rgb;
use luminate_core::state::{EffectiveAppearanceState, FacetValue, StateFacetKind};
use luminate_core::target::TargetId;

use crate::error::DaemonError;
use crate::state::DaemonState;
use crate::state::target_state::{TargetState, TargetStateEntry};

use super::super::tests_support::*;
use super::FrameEnvelope;

/// A demo device whose device-scope target advertises frame-upload
/// capability, with hardware effects that are not marked
/// `concurrent_with_streaming` (matching every bundled plugin today).
fn frame_streaming_state() -> DaemonState {
    let mut descriptor = demo_device_descriptor();
    descriptor.capabilities = CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        frame_upload: Some(capability::FrameUploadCapability {
            scope: CapabilityScope::Device,
            update_mode: capability::FrameUpdateMode::FullFrameOnly,
            max_rate_hz: Some(10),
            atomic: true,
            buffering: capability::BufferingMode::Immediate,
            shm: None,
        }),
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![HardwareEffectDescriptor {
                id: HardwareEffectId::new("morph"),
                name: "Morph".to_owned(),
                parameters: Vec::new(),
            }],
            scope: CapabilityScope::Device,
            concurrent_with_streaming: false,
        }),
        ..CapabilitySet::default()
    };
    DaemonState::from_descriptors(&[descriptor]).expect("frame-streaming state should build")
}

fn full_frame(generation: u32, sequence: u64) -> FrameEnvelope {
    FrameEnvelope {
        generation,
        sequence,
        payload: FramePayload::Full(Vec::new()),
        commit: false,
    }
}

#[test]
fn begin_frame_stream_rejects_target_without_frame_upload_capability() {
    let mut state = demo_state();
    let target = TargetId::device("demo-kbd");

    let error = state
        .begin_frame_stream(&target)
        .expect_err("target does not advertise frame-upload capability");
    assert!(matches!(error, DaemonError::UnsupportedCapability { .. }));
}

#[test]
fn begin_frame_stream_rejects_a_non_concurrent_active_effect() {
    let mut state = frame_streaming_state();
    let target = TargetId::device("demo-kbd");
    state.target_state.push(TargetStateEntry {
        target: target.clone(),
        state: TargetState::Effect(Effect::Hardware {
            id: HardwareEffectId::new("morph"),
            arguments: EffectArguments::default(),
        }),
    });

    let error = state
        .begin_frame_stream(&target)
        .expect_err("active effect is not concurrent_with_streaming");
    assert!(matches!(error, DaemonError::Conflict { .. }));
}

#[test]
fn begin_frame_stream_allows_off_and_static_effects() {
    let target = TargetId::device("demo-kbd");
    let effects = [
        Effect::Off,
        Effect::Static {
            colour: Colour::rgb(Rgb::new(255, 0, 0)),
        },
    ];

    for effect in effects {
        let mut state = frame_streaming_state();
        state.target_state.push(TargetStateEntry {
            target: target.clone(),
            state: TargetState::Effect(effect),
        });

        state
            .begin_frame_stream(&target)
            .expect("non-animated effects should not conflict with frame streaming");
    }
}

#[test]
fn begin_frame_stream_rejects_overlapping_streams() {
    let mut state = frame_streaming_state();
    let target = TargetId::device("demo-kbd");
    state
        .begin_frame_stream(&target)
        .expect("first stream should start");

    let error = state
        .begin_frame_stream(&target)
        .expect_err("a second concurrent stream on the same target is a conflict");
    assert!(matches!(error, DaemonError::Conflict { .. }));
}

#[test]
fn record_frame_upload_rejects_a_stale_generation() {
    let mut state = frame_streaming_state();
    let target = TargetId::device("demo-kbd");
    let generation = state
        .begin_frame_stream(&target)
        .expect("stream should start");

    let error = state
        .record_frame_upload(&target, &full_frame(generation.wrapping_add(1), 0))
        .expect_err("mismatched generation should be rejected");
    assert!(matches!(error, DaemonError::Conflict { .. }));
}

#[test]
fn record_frame_upload_terminates_a_stream_after_topology_replacement() {
    let mut state = frame_streaming_state();
    let target = TargetId::device("demo-kbd");
    let generation = state
        .begin_frame_stream(&target)
        .expect("begin frame stream");
    state.replace_devices_preserving_withdrawn_state(Vec::new());

    let error = state
        .record_frame_upload(&target, &full_frame(generation, 0))
        .expect_err("stale stream topology must be rejected");
    assert!(matches!(error, DaemonError::AuthorizationConflict { .. }));
    assert!(state.streaming_targets().is_empty());
}

#[test]
fn record_frame_upload_rejects_a_non_increasing_sequence() {
    let mut state = frame_streaming_state();
    let target = TargetId::device("demo-kbd");
    let generation = state
        .begin_frame_stream(&target)
        .expect("stream should start");
    state
        .record_frame_upload(&target, &full_frame(generation, 5))
        .expect("first frame should be accepted");

    let error = state
        .record_frame_upload(&target, &full_frame(generation, 5))
        .expect_err("a stale/duplicate sequence should be rejected");
    assert!(matches!(error, DaemonError::InvalidArgument { .. }));
}

#[test]
fn record_frame_upload_drops_frames_over_the_rate_limit_without_erroring() {
    let mut state = frame_streaming_state();
    let target = TargetId::device("demo-kbd");
    let generation = state
        .begin_frame_stream(&target)
        .expect("stream should start");

    let first = state
        .record_frame_upload(&target, &full_frame(generation, 0))
        .expect("first frame should be accepted");
    assert!(first, "the first frame on a stream is always forwarded");

    let second = state
        .record_frame_upload(&target, &full_frame(generation, 1))
        .expect("second frame should be accepted, just rate-limited");
    assert!(
        !second,
        "a frame arriving well within the 10Hz budget should be dropped, not forwarded"
    );
}

#[test]
fn end_frame_stream_releases_the_target_for_a_new_stream() {
    let mut state = frame_streaming_state();
    let target = TargetId::device("demo-kbd");
    let generation = state
        .begin_frame_stream(&target)
        .expect("stream should start");

    state.end_frame_stream(&target, generation);

    state
        .begin_frame_stream(&target)
        .expect("target should be free for a new stream once the old one ended");
}

#[test]
fn end_all_frame_streams_releases_every_owned_target_on_disconnect() {
    let mut state = frame_streaming_state();
    let target = TargetId::device("demo-kbd");
    state
        .begin_frame_stream(&target)
        .expect("stream should start");

    state.end_all_frame_streams(slice::from_ref(&target));

    state
        .begin_frame_stream(&target)
        .expect("disconnect cleanup should have released the target");
}

#[test]
fn begin_frame_stream_overrides_an_existing_appearance_derived_effective_appearance() {
    let mut state = frame_streaming_state();
    let target = TargetId::device("demo-kbd");
    state
        .set_static_colour_for_test(target.clone(), Colour::rgb(Rgb::new(200, 0, 0)))
        .expect("set colour before streaming");
    let status = state
        .device_state_status(target.device_id())
        .expect("device should have status");
    assert_eq!(
        status
            .observation(&target, StateFacetKind::EffectiveAppearance)
            .expect("colour should have produced an effective-appearance observation")
            .value,
        FacetValue::EffectiveAppearance(EffectiveAppearanceState::Static(Colour::rgb(Rgb::new(
            200, 0, 0
        ))))
    );

    state
        .begin_frame_stream(&target)
        .expect("stream should start");

    let status = state
        .device_state_status(target.device_id())
        .expect("device should have status");
    assert_eq!(
        status
            .observation(&target, StateFacetKind::EffectiveAppearance)
            .expect("streaming target should still report an effective-appearance observation")
            .value,
        FacetValue::EffectiveAppearance(EffectiveAppearanceState::Streaming)
    );
}

#[test]
fn begin_frame_stream_reports_streaming_effective_appearance_without_readback() {
    let mut state = frame_streaming_state();
    let target = TargetId::device("demo-kbd");
    state
        .begin_frame_stream(&target)
        .expect("stream should start");

    let status = state
        .device_state_status(target.device_id())
        .expect("device should have status");
    let observation = status
        .observation(&target, StateFacetKind::EffectiveAppearance)
        .expect("streaming target should report an effective-appearance observation");
    assert_eq!(
        observation.value,
        FacetValue::EffectiveAppearance(EffectiveAppearanceState::Streaming)
    );
}

#[test]
fn end_frame_stream_removes_the_streaming_effective_appearance() {
    let mut state = frame_streaming_state();
    let target = TargetId::device("demo-kbd");
    let generation = state
        .begin_frame_stream(&target)
        .expect("stream should start");

    state.end_frame_stream(&target, generation);

    let status = state
        .device_state_status(target.device_id())
        .expect("device should have status");
    assert!(
        status
            .observation(&target, StateFacetKind::EffectiveAppearance)
            .is_none(),
        "ending the stream should stop reporting a streaming effective appearance"
    );
}
