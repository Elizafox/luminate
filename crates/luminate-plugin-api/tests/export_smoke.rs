// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    unsafe_code,
    reason = "This integration test exercises the plugin's generated native ABI callbacks."
)]

use std::ffi::{CStr, c_char};
use std::ptr;
use std::slice;
use std::time::Duration;

use luminate_core::control::ReconciliationPolicy;
use luminate_core::frame::FrameEnvelope;
use luminate_core::shm_frame::{SHM_FRAME_HEADER_VERSION, ShmFrameHeader, ShmPixelFormat};
use luminate_plugin_api::sdk::{
    BatchPlugin, FrameStreamingPlugin, LuminatePlugin, ReadablePlugin, RescanPlugin, SetupPlugin,
    ShmFrameStreamingPlugin, StartPlugin,
};
use luminate_plugin_api::{
    DeviceDescriptor, PluginApplyResult, PluginBus, PluginError, PluginRequestContext,
    PluginSetupInteraction, PluginSetupRequest, PluginSetupStep, PluginSetupWorkflowDescriptor,
    PluginSetupWorkflowKind, PluginTarget, PluginUpdate, PluginUpdateOperation, ProbeOutcome,
    decode_cbor, encode_cbor, luminate_export_plugin,
};

struct ExportedPlugin;

impl LuminatePlugin for ExportedPlugin {
    fn new() -> Result<Self, PluginError> {
        Ok(Self)
    }

    fn probe(&self) -> ProbeOutcome {
        ProbeOutcome::Ready
    }

    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
        Ok(Vec::new())
    }

    fn apply(
        &self,
        _context: &PluginRequestContext,
        _update: &PluginUpdate,
    ) -> Result<(), PluginError> {
        Ok(())
    }
}

impl StartPlugin for ExportedPlugin {
    fn start(&self) {}
}

impl RescanPlugin for ExportedPlugin {
    fn rescan(&self, _reason: luminate_plugin_api::RescanReason) {}
}

impl SetupPlugin for ExportedPlugin {
    fn setup(request: PluginSetupRequest) -> Result<PluginSetupStep, PluginError> {
        if request.workflow != "pair" {
            return Err(PluginError::InvalidArgument("unknown workflow".to_owned()));
        }
        Ok(PluginSetupStep::Interaction {
            continuation: vec![1, 2, 3],
            interaction: PluginSetupInteraction::PhysicalAction {
                instruction: "Press the button.".to_owned(),
            },
        })
    }
}

impl BatchPlugin for ExportedPlugin {
    fn apply_batch(
        &self,
        _context: &PluginRequestContext,
        updates: &[PluginUpdate],
    ) -> Vec<Result<(), PluginError>> {
        updates.iter().map(|_| Ok(())).collect()
    }
}

impl ReadablePlugin for ExportedPlugin {
    fn read_state(
        &self,
        _context: &PluginRequestContext,
        _request: &luminate_plugin_api::PluginReadRequest,
    ) -> Result<luminate_plugin_api::PluginStateSnapshot, PluginError> {
        Ok(luminate_plugin_api::PluginStateSnapshot::default())
    }
}

impl FrameStreamingPlugin for ExportedPlugin {
    fn upload_frame(
        &self,
        _context: &PluginRequestContext,
        _target: &PluginTarget,
        _envelope: &FrameEnvelope,
    ) -> Result<(), PluginError> {
        Ok(())
    }
}

impl ShmFrameStreamingPlugin for ExportedPlugin {
    type Stream = u32;

    fn shm_stream_begin(
        &self,
        _context: &PluginRequestContext,
        _target: &PluginTarget,
        _format: ShmPixelFormat,
        _pixel_count: u32,
        _generation: u32,
    ) -> Result<Self::Stream, PluginError> {
        Ok(42)
    }

    fn shm_frame(
        &self,
        _context: &PluginRequestContext,
        stream: &mut Self::Stream,
        _header: &ShmFrameHeader,
        _pixels: &[u8],
    ) -> Result<(), PluginError> {
        *stream += 1;
        Ok(())
    }

    fn shm_stream_end(&self, _stream: Self::Stream, _generation: u32) {}
}

static NAME: &CStr = c"export-smoke";
static VERSION: &CStr = c"1.0";
static BUSES: &[PluginBus] = &[];
static VENDORS: &[luminate_plugin_api::PluginVendorId] = &[];
static HINTS: &[luminate_plugin_api::PluginProbeHint] = &[];
static SETUP_WORKFLOWS: &[PluginSetupWorkflowDescriptor] = &[PluginSetupWorkflowDescriptor::new(
    c"pair",
    c"Pair hardware",
    c"Connect nearby hardware.",
    PluginSetupWorkflowKind::Provision,
)];

luminate_export_plugin! {
    plugin: ExportedPlugin,
    name: NAME,
    version: VERSION,
    priority: 1,
    recommended_reconciliation: Some(ReconciliationPolicy::Restore),
    buses: BUSES,
    vendors: VENDORS,
    probe_hints: HINTS,
    settings: &[] as &'static [luminate_plugin_api::PluginSettingDescriptor],
    setup_workflows: SETUP_WORKFLOWS,
    setup: native,
    start: native,
    rescan: native,
    batch: native,
    read_state: native,
    frame_upload: native,
    shm_frame: native,
}

unsafe extern "C" fn noop_log(_plugin: *const c_char, _level: u8, _message: *const c_char) {}

unsafe extern "C" fn noop_notify(_plugin: *const c_char) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "The test intentionally exercises the complete generated descriptor surface."
    )]
    fn exported_descriptor_callbacks_round_trip_through_the_native_boundary() {
        // SAFETY: the callbacks and empty configuration remain valid for the call.
        // SAFETY: the generated callback is called with valid no-op callbacks.
        unsafe { (LUMINATE_PLUGIN_DESCRIPTOR.init)(noop_log, 0, noop_notify, ptr::null(), 0) };
        assert_eq!(
            // SAFETY: the descriptor supplies a valid generated callback.
            unsafe { (LUMINATE_PLUGIN_DESCRIPTOR.probe.expect("probe callback"))() },
            ProbeOutcome::Ready.to_abi()
        );
        assert_eq!(LUMINATE_PLUGIN_DESCRIPTOR.setup_workflow_count, 1);
        let request = encode_cbor(&PluginSetupRequest {
            workflow: "pair".to_owned(),
            continuation: Vec::new(),
            response: None,
        })
        .expect("encode setup request");
        let mut setup_len = 0;
        // SAFETY: request storage and writable response length remain valid for the call.
        let setup = unsafe {
            (LUMINATE_PLUGIN_DESCRIPTOR
                .setup_cbor
                .expect("setup callback"))(
                request.as_ptr(), request.len(), &raw mut setup_len
            )
        };
        assert!(!setup.is_null());
        // SAFETY: the generated callback reports readable cache bytes until its next call.
        let setup = unsafe { slice::from_raw_parts(setup, setup_len) };
        let setup: Result<PluginSetupStep, String> =
            decode_cbor(setup).expect("decode setup callback result");
        assert!(matches!(
            setup,
            Ok(PluginSetupStep::Interaction {
                interaction: PluginSetupInteraction::PhysicalAction { .. },
                ..
            })
        ));

        let mut topology_len = 0;
        let topology =
        // SAFETY: the descriptor supplies a valid generated callback and writable length.
        unsafe { (LUMINATE_PLUGIN_DESCRIPTOR.topology_cbor.expect("topology callback"))(&raw mut topology_len) };
        assert!(!topology.is_null());
        assert!(topology_len > 0);

        let update = PluginUpdate {
            target: PluginTarget::Device {
                device: "example".to_owned(),
            },
            operation: PluginUpdateOperation::Clear,
        };
        let encoded = encode_cbor(&update).expect("encode update");
        let mut result = PluginApplyResult::internal("unwritten");
        // SAFETY: the encoded update and result remain valid for this callback.
        let accepted = unsafe {
            (LUMINATE_PLUGIN_DESCRIPTOR
                .apply_update_cbor
                .expect("apply callback"))(
                PluginRequestContext::new(Duration::from_secs(1)),
                encoded.as_ptr(),
                encoded.len(),
                &raw mut result,
            )
        };
        assert_eq!(accepted, 1);

        let mut batch_results = [PluginApplyResult::internal("unwritten")];
        let batch = encode_cbor(&luminate_plugin_api::PluginUpdateBatch {
            updates: vec![update],
        })
        .expect("encode batch");
        // SAFETY: the encoded batch and writable result array remain valid for this callback.
        let accepted = unsafe {
            (LUMINATE_PLUGIN_DESCRIPTOR
                .apply_batch_cbor
                .expect("batch callback"))(
                PluginRequestContext::new(Duration::from_secs(1)),
                batch.as_ptr(),
                batch.len(),
                batch_results.as_mut_ptr(),
                batch_results.len(),
            )
        };
        assert_eq!(accepted, 1);

        // Exercise the optional generated lifecycle and malformed-payload paths.
        // SAFETY: these callbacks only read their scalar arguments.
        // SAFETY: the generated start callback takes no arguments.
        unsafe { (LUMINATE_PLUGIN_DESCRIPTOR.start.expect("start callback"))() };
        // SAFETY: the generated rescan callback accepts every ABI byte.
        unsafe { (LUMINATE_PLUGIN_DESCRIPTOR.rescan.expect("rescan callback"))(u8::MAX) };
        let mut output = [0_u8; 1];
        // SAFETY: null input is an explicitly handled ABI validation case.
        let required = unsafe {
            (LUMINATE_PLUGIN_DESCRIPTOR
                .read_state_cbor
                .expect("read callback"))(
                PluginRequestContext::new(Duration::from_secs(1)),
                ptr::null(),
                0,
                output.as_mut_ptr(),
                output.len(),
            )
        };
        assert_eq!(required, usize::MAX);
        // SAFETY: null input and output are explicitly handled by the adapter.
        let accepted = unsafe {
            (LUMINATE_PLUGIN_DESCRIPTOR
                .frame_upload_cbor
                .expect("frame callback"))(
                PluginRequestContext::new(Duration::from_secs(1)),
                ptr::null(),
                0,
                ptr::null_mut(),
            )
        };
        assert_eq!(accepted, 0);

        let target = encode_cbor(&PluginTarget::Device {
            device: "example".to_owned(),
        })
        .expect("encode stream target");
        let mut handle = 0;
        let mut stream_result = PluginApplyResult::internal("unwritten");
        // SAFETY: target, handle, and result remain valid for this callback.
        let accepted = unsafe {
            (LUMINATE_PLUGIN_DESCRIPTOR
                .shm_stream_begin
                .expect("SHM begin callback"))(
                PluginRequestContext::new(Duration::from_secs(1)),
                target.as_ptr(),
                target.len(),
                ShmPixelFormat::Rgb8.to_abi(),
                1,
                2,
                &raw mut handle,
                &raw mut stream_result,
            )
        };
        assert_eq!(accepted, 1);
        assert_ne!(handle, 0);

        let header = ShmFrameHeader {
            sequence: 1,
            generation: 2,
            pixel_count: 1,
            pixel_format: ShmPixelFormat::Rgb8.to_abi(),
            header_version: SHM_FRAME_HEADER_VERSION,
            flags: 0,
            reserved: 0,
        };
        let mut frame_result = PluginApplyResult::internal("unwritten");
        let pixels = [0_u8, 1, 2];
        // SAFETY: handle was returned by the matching begin callback; header and pixels remain valid.
        let accepted = unsafe {
            (LUMINATE_PLUGIN_DESCRIPTOR
                .shm_frame_apply
                .expect("SHM frame callback"))(
                PluginRequestContext::new(Duration::from_secs(1)),
                handle,
                header,
                pixels.as_ptr(),
                pixels.len(),
                &raw mut frame_result,
            )
        };
        assert_eq!(accepted, 1);
        // SAFETY: the handle is ended exactly once after the frame callback.
        unsafe {
            (LUMINATE_PLUGIN_DESCRIPTOR
                .shm_stream_end
                .expect("SHM end callback"))(
                PluginRequestContext::new(Duration::from_secs(1)),
                handle,
                2,
            );
        }
    }
}
