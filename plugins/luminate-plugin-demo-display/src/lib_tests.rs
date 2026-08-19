// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Pipe and shared-memory frame negotiation tests for the matrix display.

use std::time::Duration;

use luminate_core::colour::Colour;
use luminate_core::effect::Effect;
use luminate_core::rgb::Rgb;
use luminate_core::shm_frame::SHM_FRAME_HEADER_VERSION;

use super::*;

fn device_target() -> PluginTarget {
    PluginTarget::Device {
        device: DEVICE_ID.to_owned(),
    }
}

fn matrix_pixels() -> Vec<Colour> {
    (0..PIXEL_COUNT)
        .map(|index| Colour::rgb(Rgb::new(u8::try_from(index).unwrap_or(u8::MAX), 0, 0)))
        .collect()
}

#[test]
fn topology_advertises_a_matrix_shaped_shared_memory_capability() {
    let device = display_device();

    assert_eq!(device.id, DEVICE_ID);
    assert!(device.surfaces.is_empty());
    let shm = device
        .capabilities
        .frame_upload
        .as_ref()
        .and_then(|frame_upload| frame_upload.shm.as_ref())
        .expect("display should advertise the shared-memory fast path");
    assert_eq!(shm.pixel_formats, vec![ShmPixelFormat::Rgb8]);
    assert_eq!(
        shm.shape,
        ShmFrameShape::Matrix {
            width: WIDTH,
            height: HEIGHT
        }
    );
    assert_eq!(shm.shape.pixel_count(), Some(PIXEL_COUNT));
}

#[test]
fn accepts_device_colour_update() {
    let update = PluginUpdate {
        target: device_target(),
        operation: PluginUpdateOperation::SetEffect {
            effect: Effect::Static {
                colour: Colour::rgb(Rgb::new(10, 20, 30)),
            },
        },
    };

    apply_update(&update).expect("device colour update should be accepted");
}

#[test]
fn rejects_foreign_device() {
    let update = PluginUpdate {
        target: PluginTarget::Device {
            device: "demo-keyboard".to_owned(),
        },
        operation: PluginUpdateOperation::Clear,
    };

    let error = apply_update(&update).expect_err("foreign device should be rejected");
    assert!(error.contains("not owned"));
}

#[test]
fn pipe_path_accepts_a_full_matrix_frame() {
    let payload = FramePayload::Full(matrix_pixels());

    apply_frame_payload(&device_target(), &payload)
        .expect("a full 256-pixel frame should apply over the pipe");
}

#[test]
fn pipe_path_rejects_the_wrong_pixel_count() {
    let payload = FramePayload::Full(vec![Colour::rgb(Rgb::new(1, 2, 3))]);

    let error = apply_frame_payload(&device_target(), &payload)
        .expect_err("a single-pixel frame should be rejected");
    assert!(error.contains("expects exactly 256 pixels"));
}

#[test]
fn pipe_path_rejects_a_partial_frame() {
    let payload = FramePayload::Partial(vec![(0, Colour::rgb(Rgb::new(1, 2, 3)))]);

    let error = apply_frame_payload(&device_target(), &payload)
        .expect_err("partial frames should be rejected");
    assert!(error.contains("full frames"));
}

fn plugin() -> DemoDisplay {
    DemoDisplay
}

fn request_context() -> PluginRequestContext {
    PluginRequestContext::new(Duration::from_secs(1))
}

#[test]
fn shm_stream_begin_rejects_a_non_rgb8_format() {
    let error = plugin()
        .shm_stream_begin(
            &request_context(),
            &device_target(),
            ShmPixelFormat::Mono8,
            PIXEL_COUNT,
            1,
        )
        .expect_err("a non-Rgb8 format should be rejected");
    assert!(matches!(error, PluginError::Unsupported(message) if message.contains("Rgb8")));
}

#[test]
fn shm_stream_begin_rejects_the_wrong_pixel_count() {
    let error = plugin()
        .shm_stream_begin(
            &request_context(),
            &device_target(),
            ShmPixelFormat::Rgb8,
            4,
            1,
        )
        .expect_err("a mismatched pixel count should be rejected");
    assert!(
        matches!(error, PluginError::Unsupported(message) if message.contains("expects exactly 256 pixels"))
    );
}

#[test]
fn shm_frame_round_trips_through_begin_apply_end() {
    let plugin = plugin();
    let mut stream = plugin
        .shm_stream_begin(
            &request_context(),
            &device_target(),
            ShmPixelFormat::Rgb8,
            PIXEL_COUNT,
            5,
        )
        .expect("begin should accept a matching negotiation");

    let header = ShmFrameHeader {
        sequence: 1,
        generation: 5,
        pixel_count: PIXEL_COUNT,
        pixel_format: ShmPixelFormat::Rgb8.to_abi(),
        header_version: SHM_FRAME_HEADER_VERSION,
        flags: 0,
        reserved: 0,
    };
    let pixels = vec![0_u8; usize::try_from(PIXEL_COUNT).unwrap() * 3];
    plugin
        .shm_frame(&request_context(), &mut stream, &header, &pixels)
        .expect("a correctly sized sample should apply");
    assert_eq!(stream.frames_applied, 1);

    plugin.shm_stream_end(stream, 5);
}

#[test]
fn shm_frame_rejects_a_mismatched_pixel_buffer_length() {
    let plugin = plugin();
    let mut stream = plugin
        .shm_stream_begin(
            &request_context(),
            &device_target(),
            ShmPixelFormat::Rgb8,
            PIXEL_COUNT,
            1,
        )
        .expect("begin should accept a matching negotiation");

    let header = ShmFrameHeader {
        sequence: 1,
        generation: 1,
        pixel_count: PIXEL_COUNT,
        pixel_format: ShmPixelFormat::Rgb8.to_abi(),
        header_version: SHM_FRAME_HEADER_VERSION,
        flags: 0,
        reserved: 0,
    };
    let error = plugin
        .shm_frame(&request_context(), &mut stream, &header, &[0_u8; 3])
        .expect_err("an undersized sample should be rejected");
    assert!(matches!(error, PluginError::Unsupported(_)));
}
