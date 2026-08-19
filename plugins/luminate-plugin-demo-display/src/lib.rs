// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Reference plugin for frame streaming to a 16×16 RGB matrix.
//!
//! It supports both ordinary CBOR frame upload and the zero-copy shared-memory
//! path, making the differences between the two implementations easy to compare.

use luminate_core::control::ReconciliationPolicy;

use std::ffi::CStr;

use luminate_core::capability::{
    BufferingMode, CapabilityScope, CapabilitySet, CctEmulation, ColourCapability, FrameUpdateMode,
    FrameUploadCapability, ShmFrameCapability, ShmFrameShape,
};
use luminate_core::device::{DeviceCategory, device_category};
use luminate_core::frame::{FrameEnvelope, FramePayload};
use luminate_core::shm_frame::{ShmFrameHeader, ShmPixelFormat};
use luminate_plugin_api::{
    DeviceDescriptor, PluginBus, PluginError, PluginProbeHint, PluginRequestContext, PluginTarget,
    PluginUpdate, PluginUpdateOperation, PluginVendorId, ProbeOutcome, ShadowState,
    luminate_export_plugin,
    sdk::{FrameStreamingPlugin, LuminatePlugin, ShmFrameStreamingPlugin},
};

const NAME: &CStr = c"luminate-plugin-demo-display";
const VERSION: &CStr = c"0.1.0";
const DEVICE_ID: &str = "demo-led-display";
const WIDTH: u32 = 16;
const HEIGHT: u32 = 16;
const PIXEL_COUNT: u32 = WIDTH * HEIGHT;

static BUSES: &[PluginBus] = &[PluginBus::Platform];
static VENDORS: &[PluginVendorId] = &[];
static HINTS: &[PluginProbeHint] = &[];

struct DemoDisplay;

/// Remembers where a negotiated stream writes and how many frames it has seen.
///
/// Keeping the validated target here means later frames cannot quietly switch
/// destinations; changing target requires a new stream.
#[derive(Debug)]
struct DisplayStream {
    target: String,
    frames_applied: u64,
}

impl LuminatePlugin for DemoDisplay {
    fn new() -> Result<Self, PluginError> {
        Ok(Self)
    }

    fn probe(&self) -> ProbeOutcome {
        tracing::info!(
            vendor = "Nebula Forge",
            product = "Aurora Panel",
            "demo display probe accepted"
        );
        ProbeOutcome::Ready
    }

    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
        Ok(vec![display_device()])
    }

    fn apply(
        &self,
        _context: &PluginRequestContext,
        update: &PluginUpdate,
    ) -> Result<(), PluginError> {
        apply_update(update).map_err(PluginError::Unsupported)
    }
}

impl FrameStreamingPlugin for DemoDisplay {
    fn upload_frame(
        &self,
        _context: &PluginRequestContext,
        target: &PluginTarget,
        envelope: &FrameEnvelope,
    ) -> Result<(), PluginError> {
        apply_frame_payload(target, &envelope.payload).map_err(PluginError::Unsupported)
    }
}

impl ShmFrameStreamingPlugin for DemoDisplay {
    type Stream = DisplayStream;

    fn shm_stream_begin(
        &self,
        _context: &PluginRequestContext,
        target: &PluginTarget,
        format: ShmPixelFormat,
        pixel_count: u32,
        generation: u32,
    ) -> Result<Self::Stream, PluginError> {
        ensure_device_target(target).map_err(PluginError::Unsupported)?;
        if format != ShmPixelFormat::Rgb8 {
            return Err(PluginError::Unsupported(format!(
                "demo display only accepts Rgb8 over the shared-memory path, got {format:?}"
            )));
        }
        if pixel_count != PIXEL_COUNT {
            return Err(PluginError::Unsupported(format!(
                "demo display expects exactly {PIXEL_COUNT} pixels, got {pixel_count}"
            )));
        }
        tracing::info!(
            target = %target,
            generation,
            "demo display shared-memory stream began"
        );
        Ok(DisplayStream {
            target: target.to_string(),
            frames_applied: 0,
        })
    }

    fn shm_frame(
        &self,
        _context: &PluginRequestContext,
        stream: &mut Self::Stream,
        header: &ShmFrameHeader,
        pixels: &[u8],
    ) -> Result<(), PluginError> {
        // The SDK has already decoded the header. The plugin still checks the
        // byte count before treating the sample as RGB8 pixel data.
        let expected_len = usize::try_from(PIXEL_COUNT).unwrap_or(usize::MAX)
            * ShmPixelFormat::Rgb8.bytes_per_pixel();
        if pixels.len() != expected_len {
            return Err(PluginError::Unsupported(format!(
                "demo display expects {expected_len} shared-memory pixel bytes, got {}",
                pixels.len()
            )));
        }
        stream.frames_applied += 1;
        record_shadow_state(
            &stream.target,
            &format!(
                "shm-frame:generation={}:sequence={}:applied={}",
                header.generation, header.sequence, stream.frames_applied
            ),
        );
        Ok(())
    }

    fn shm_stream_end(&self, stream: Self::Stream, generation: u32) {
        // Stream teardown cannot return an ABI error. A real transport would
        // log any cleanup failure and leave the device in a safe state.
        tracing::info!(
            target = %stream.target,
            generation,
            frames_applied = stream.frames_applied,
            "demo display shared-memory stream ended"
        );
    }
}

fn display_device() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: DEVICE_ID.to_owned(),
        name: "Demo LED Display".to_owned(),
        vendor: Some("Nebula Forge".to_owned()),
        model: Some("Aurora Panel".to_owned()),
        surfaces: Vec::new(),
        groups: Vec::new(),
        capabilities: display_capabilities(),
        category: Some(DeviceCategory::new(device_category::LED_STRIP)),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: vec!["Validates the zero-copy shared-memory frame fast path end to end.".to_owned()],
        warnings: Vec::new(),
    }
}

/// Describes the frame formats accepted by both upload paths.
///
/// The matrix shape tells renderers how the linear pixels form rows and columns.
fn display_capabilities() -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        frame_upload: Some(FrameUploadCapability {
            scope: CapabilityScope::Device,
            update_mode: FrameUpdateMode::FullFrameOnly,
            max_rate_hz: None,
            atomic: true,
            buffering: BufferingMode::Immediate,
            shm: Some(ShmFrameCapability {
                pixel_formats: vec![ShmPixelFormat::Rgb8],
                shape: ShmFrameShape::Matrix {
                    width: WIDTH,
                    height: HEIGHT,
                },
                max_rate_hz: None,
            }),
        }),
        emission: true,
        ..CapabilitySet::default()
    }
}

fn apply_update(update: &PluginUpdate) -> Result<(), String> {
    ensure_device_target(&update.target)?;

    match &update.operation {
        PluginUpdateOperation::SaveCurrent => {
            tracing::info!(
                target = %update.target,
                "demo display save-current accepted as a no-op"
            );
        }
        PluginUpdateOperation::SetAppearanceSlots { .. } => {
            return Err("demo display does not advertise appearance slots".to_owned());
        }
        operation @ (PluginUpdateOperation::SetEffect { .. }
        | PluginUpdateOperation::SetBrightness { .. }
        | PluginUpdateOperation::Clear) => {
            record_shadow_state(&update.target.to_string(), &operation.to_string());
            tracing::info!(
                target = %update.target,
                operation = %operation.name(),
                "demo display update applied"
            );
        }
    }
    Ok(())
}

fn apply_frame_payload(target: &PluginTarget, payload: &FramePayload) -> Result<(), String> {
    ensure_device_target(target)?;
    let FramePayload::Full(pixels) = payload else {
        return Err("demo display frame streaming only supports full frames".to_owned());
    };
    let count = u32::try_from(pixels.len()).unwrap_or(u32::MAX);
    if count != PIXEL_COUNT {
        return Err(format!(
            "demo display frame expects exactly {PIXEL_COUNT} pixels, got {count}"
        ));
    }
    record_shadow_state(&target.to_string(), &format!("pipe-frame:count={count}"));
    Ok(())
}

fn ensure_device_target(target: &PluginTarget) -> Result<(), String> {
    match target {
        PluginTarget::Device { device } if device == DEVICE_ID => Ok(()),
        PluginTarget::Device { device } => Err(format!(
            "target is not owned by the demo display plugin: {device}"
        )),
        PluginTarget::Surface { .. }
        | PluginTarget::Element { .. }
        | PluginTarget::Group { .. } => {
            Err("demo display only accepts device-scoped targets".to_owned())
        }
    }
}

fn record_shadow_state(target: &str, entry: &str) {
    shadow_state()
        .lock()
        .expect("lock poisoned")
        .insert(target.to_owned(), entry.to_owned());
}

fn shadow_state() -> &'static ShadowState<String, String> {
    static SHADOW: ShadowState<String, String> = ShadowState::new();
    &SHADOW
}

// The demo implements both frame paths itself. Batch updates use the SDK's
// ordinary one-at-a-time fallback, and unsupported callbacks are left out.
luminate_export_plugin! {
    plugin: DemoDisplay,
    name: NAME,
    version: VERSION,
    priority: 50,
    recommended_reconciliation: Some(ReconciliationPolicy::Leave),
    buses: BUSES,
    vendors: VENDORS,
    probe_hints: HINTS,
    start: none,
    rescan: none,
    batch: default,
    read_state: none,
    frame_upload: native,
    shm_frame: native,
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
