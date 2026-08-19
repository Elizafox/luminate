// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! The `luminate_export_plugin!` macro.
//!
//! Expands into the `extern "C"` entry points and the two `no_mangle`
//! statics the daemon's plugin host looks for. `#[macro_export]` places the
//! macro at the crate root regardless of which file defines it, so plugin
//! crates invoke it unqualified and are unaffected by this module's
//! existence.

/// Declares a safe Rust plugin whose batch behaviour is ordered application of
/// individual updates and which has no start or readback callback.
#[macro_export]
macro_rules! luminate_export_plugin {
    (
        plugin: $plugin:ty,
        name: $name:expr,
        version: $version:expr,
        priority: $priority:expr,
        recommended_reconciliation: $recommended_reconciliation:expr,
        buses: $buses:expr,
        vendors: $vendors:expr,
        probe_hints: $hints:expr,
        start: $start:ident,
        rescan: $rescan:ident,
        batch: $batch:ident,
        read_state: $read_state:ident,
        frame_upload: $frame_upload:ident,
        shm_frame: $shm_frame:ident $(,)?
    ) => {
        $crate::luminate_export_plugin! {
            plugin: $plugin,
            name: $name,
            version: $version,
            priority: $priority,
            recommended_reconciliation: $recommended_reconciliation,
            buses: $buses,
            vendors: $vendors,
            probe_hints: $hints,
            settings: &[] as &'static [$crate::PluginSettingDescriptor],
            start: $start,
            rescan: $rescan,
            batch: $batch,
            read_state: $read_state,
            frame_upload: $frame_upload,
            shm_frame: $shm_frame,
        }
    };

    (
        plugin: $plugin:ty,
        name: $name:expr,
        version: $version:expr,
        priority: $priority:expr,
        recommended_reconciliation: $recommended_reconciliation:expr,
        buses: $buses:expr,
        vendors: $vendors:expr,
        probe_hints: $hints:expr,
        settings: $settings:expr,
        start: $start:ident,
        rescan: $rescan:ident,
        batch: $batch:ident,
        read_state: $read_state:ident,
        frame_upload: $frame_upload:ident,
        shm_frame: $shm_frame:ident $(,)?
    ) => {
        $crate::luminate_export_plugin! {
            plugin: $plugin,
            name: $name,
            version: $version,
            priority: $priority,
            recommended_reconciliation: $recommended_reconciliation,
            buses: $buses,
            vendors: $vendors,
            probe_hints: $hints,
            settings: $settings,
            setup_workflows: &[] as &'static [$crate::PluginSetupWorkflowDescriptor],
            setup: none,
            start: $start,
            rescan: $rescan,
            batch: $batch,
            read_state: $read_state,
            frame_upload: $frame_upload,
            shm_frame: $shm_frame,
        }
    };

    (
        plugin: $plugin:ty,
        name: $name:expr,
        version: $version:expr,
        priority: $priority:expr,
        recommended_reconciliation: $recommended_reconciliation:expr,
        buses: $buses:expr,
        vendors: $vendors:expr,
        probe_hints: $hints:expr,
        settings: $settings:expr,
        setup_workflows: $setup_workflows:expr,
        setup: $setup:ident,
        start: $start:ident,
        rescan: $rescan:ident,
        batch: $batch:ident,
        read_state: $read_state:ident,
        frame_upload: $frame_upload:ident,
        shm_frame: $shm_frame:ident $(,)?
    ) => {
        static LUMINATE_PLUGIN_INSTANCE: ::std::sync::OnceLock<
            Result<$plugin, $crate::PluginError>,
        > = ::std::sync::OnceLock::new();
        static LUMINATE_PLUGIN_TOPOLOGY: ::std::sync::OnceLock<
            ::std::sync::Mutex<Vec<u8>>,
        > = ::std::sync::OnceLock::new();
        static LUMINATE_PLUGIN_SETUP: ::std::sync::OnceLock<
            ::std::sync::Mutex<Vec<u8>>,
        > = ::std::sync::OnceLock::new();

        unsafe extern "C" fn luminate_plugin_init(
            log: $crate::PluginLogFn,
            max_level: u8,
            notify_topology_changed: $crate::PluginNotifyFn,
            configuration_cbor: *const u8,
            configuration_len: usize,
        ) {
            // SAFETY: the host upholds the descriptor initialization contract.
            unsafe {
                $crate::sdk::initialize::<$plugin>(
                    &LUMINATE_PLUGIN_INSTANCE,
                    $name,
                    log,
                    max_level,
                    notify_topology_changed,
                    configuration_cbor,
                    configuration_len,
                )
            };
        }

        unsafe extern "C" fn luminate_plugin_probe() -> u8 {
            $crate::sdk::probe(&LUMINATE_PLUGIN_INSTANCE)
        }

        $crate::luminate_export_plugin!(@start_item $start, $plugin);
        $crate::luminate_export_plugin!(@rescan_item $rescan, $plugin);

        unsafe extern "C" fn luminate_plugin_topology(length: *mut usize) -> *const u8 {
            // SAFETY: the host upholds the descriptor callback contract.
            unsafe {
                $crate::sdk::topology(
                    &LUMINATE_PLUGIN_INSTANCE,
                    &LUMINATE_PLUGIN_TOPOLOGY,
                    length,
                )
            }
        }

        unsafe extern "C" fn luminate_plugin_apply(
            context: $crate::PluginRequestContext,
            update_cbor: *const u8,
            update_len: usize,
            result: *mut $crate::PluginApplyResult,
        ) -> u8 {
            // SAFETY: the host upholds the descriptor callback contract.
            unsafe { $crate::sdk::apply(&LUMINATE_PLUGIN_INSTANCE, context, update_cbor, update_len, result) }
        }

        $crate::luminate_export_plugin!(@batch_item $batch, $plugin);
        $crate::luminate_export_plugin!(@read_item $read_state, $plugin);
        $crate::luminate_export_plugin!(@frame_item $frame_upload, $plugin);
        $crate::luminate_export_plugin!(@shm_item $shm_frame, $plugin);
        $crate::luminate_export_plugin!(@setup_item $setup, $plugin);

        #[unsafe(no_mangle)]
        pub static LUMINATE_PLUGIN_ABI_VERSION: $crate::PluginAbiVersion =
            $crate::PLUGIN_ABI_VERSION;

        #[unsafe(no_mangle)]
        pub static LUMINATE_PLUGIN_DESCRIPTOR: $crate::PluginDescriptor =
            $crate::PluginDescriptor {
                name: $name.as_ptr().cast(),
                version: $version.as_ptr().cast(),
                priority: $priority,
                recommended_reconciliation: $crate::reconciliation_policy_to_abi(
                    $recommended_reconciliation,
                ),
                buses: $buses.as_ptr(),
                bus_count: $buses.len(),
                vendors: $vendors.as_ptr(),
                vendor_count: $vendors.len(),
                probe_hints: $hints.as_ptr(),
                probe_hint_count: $hints.len(),
                settings: $settings.as_ptr(),
                setting_count: $settings.len(),
                setup_workflows: $setup_workflows.as_ptr(),
                setup_workflow_count: $setup_workflows.len(),
                init: luminate_plugin_init,
                probe: Some(luminate_plugin_probe),
                start: $crate::luminate_export_plugin!(@start_expr $start),
                rescan: $crate::luminate_export_plugin!(@rescan_expr $rescan),
                topology_cbor: Some(luminate_plugin_topology),
                apply_update_cbor: Some(luminate_plugin_apply),
                apply_batch_cbor: Some(luminate_plugin_apply_batch),
                read_state_cbor: $crate::luminate_export_plugin!(@read_expr $read_state),
                frame_upload_cbor: $crate::luminate_export_plugin!(@frame_expr $frame_upload),
                shm_stream_begin: $crate::luminate_export_plugin!(@shm_begin_expr $shm_frame),
                shm_frame_apply: $crate::luminate_export_plugin!(@shm_apply_expr $shm_frame),
                shm_stream_end: $crate::luminate_export_plugin!(@shm_end_expr $shm_frame),
                setup_cbor: $crate::luminate_export_plugin!(@setup_expr $setup),
            };
    };

    (@setup_item none, $plugin:ty) => {};
    (@setup_item native, $plugin:ty) => {
        unsafe extern "C" fn luminate_plugin_setup(
            request_cbor: *const u8,
            request_len: usize,
            response_len: *mut usize,
        ) -> *const u8 {
            // SAFETY: the host upholds the setup callback contract.
            unsafe {
                $crate::sdk::setup::<$plugin>(
                    &LUMINATE_PLUGIN_SETUP,
                    request_cbor,
                    request_len,
                    response_len,
                )
            }
        }
    };
    (@setup_expr none) => { None };
    (@setup_expr native) => { Some(luminate_plugin_setup) };

    (@start_item none, $plugin:ty) => {};
    (@start_item native, $plugin:ty) => {
        unsafe extern "C" fn luminate_plugin_start() {
            $crate::sdk::start::<$plugin>(&LUMINATE_PLUGIN_INSTANCE);
        }
    };
    (@start_expr none) => { None };
    (@start_expr native) => { Some(luminate_plugin_start) };

    (@rescan_item none, $plugin:ty) => {};
    (@rescan_item native, $plugin:ty) => {
        unsafe extern "C" fn luminate_plugin_rescan(reason: u8) {
            $crate::sdk::rescan::<$plugin>(&LUMINATE_PLUGIN_INSTANCE, reason);
        }
    };
    (@rescan_expr none) => { None };
    (@rescan_expr native) => { Some(luminate_plugin_rescan) };

    (@batch_item default, $plugin:ty) => {
        unsafe extern "C" fn luminate_plugin_apply_batch(
            context: $crate::PluginRequestContext,
            batch_cbor: *const u8,
            batch_len: usize,
            results: *mut $crate::PluginApplyResult,
            results_len: usize,
        ) -> u8 {
            // SAFETY: the host upholds the descriptor callback contract.
            unsafe {
                $crate::sdk::apply_batch(
                    &LUMINATE_PLUGIN_INSTANCE,
                    context,
                    batch_cbor,
                    batch_len,
                    results,
                    results_len,
                )
            }
        }
    };
    (@batch_item native, $plugin:ty) => {
        unsafe extern "C" fn luminate_plugin_apply_batch(
            context: $crate::PluginRequestContext,
            batch_cbor: *const u8,
            batch_len: usize,
            results: *mut $crate::PluginApplyResult,
            results_len: usize,
        ) -> u8 {
            // SAFETY: the host upholds the descriptor callback contract.
            unsafe {
                $crate::sdk::apply_native_batch::<$plugin>(
                    &LUMINATE_PLUGIN_INSTANCE,
                    context,
                    batch_cbor,
                    batch_len,
                    results,
                    results_len,
                )
            }
        }
    };

    (@read_item none, $plugin:ty) => {};
    (@read_item native, $plugin:ty) => {
        unsafe extern "C" fn luminate_plugin_read_state(
            context: $crate::PluginRequestContext,
            request_cbor: *const u8,
            request_len: usize,
            output: *mut u8,
            output_capacity: usize,
        ) -> usize {
            // SAFETY: the host upholds the descriptor callback contract.
            unsafe {
                $crate::sdk::read_state::<$plugin>(
                    &LUMINATE_PLUGIN_INSTANCE,
                    context,
                    request_cbor,
                    request_len,
                    output,
                    output_capacity,
                )
            }
        }
    };
    (@read_expr none) => { None };
    (@read_expr native) => { Some(luminate_plugin_read_state) };

    (@frame_item none, $plugin:ty) => {};
    (@frame_item native, $plugin:ty) => {
        unsafe extern "C" fn luminate_plugin_frame_upload(
            context: $crate::PluginRequestContext,
            envelope_cbor: *const u8,
            envelope_len: usize,
            result: *mut $crate::PluginApplyResult,
        ) -> u8 {
            // SAFETY: the host upholds the descriptor callback contract.
            unsafe {
                $crate::sdk::upload_frame::<$plugin>(
                    &LUMINATE_PLUGIN_INSTANCE,
                    context,
                    envelope_cbor,
                    envelope_len,
                    result,
                )
            }
        }
    };
    (@frame_expr none) => { None };
    (@frame_expr native) => { Some(luminate_plugin_frame_upload) };

    (@shm_item none, $plugin:ty) => {};
    (@shm_item native, $plugin:ty) => {
        unsafe extern "C" fn luminate_plugin_shm_stream_begin(
            context: $crate::PluginRequestContext,
            target_cbor: *const u8,
            target_len: usize,
            pixel_format: u32,
            pixel_count: u32,
            generation: u32,
            handle_out: *mut u64,
            result: *mut $crate::PluginApplyResult,
        ) -> u8 {
            // SAFETY: the host upholds the descriptor callback contract.
            unsafe {
                $crate::sdk::shm_stream_begin::<$plugin>(
                    &LUMINATE_PLUGIN_INSTANCE,
                    context,
                    target_cbor,
                    target_len,
                    pixel_format,
                    pixel_count,
                    generation,
                    handle_out,
                    result,
                )
            }
        }

        unsafe extern "C" fn luminate_plugin_shm_frame_apply(
            context: $crate::PluginRequestContext,
            handle: u64,
            header: ::luminate_core::shm_frame::ShmFrameHeader,
            pixels: *const u8,
            pixels_len: usize,
            result: *mut $crate::PluginApplyResult,
        ) -> u8 {
            // SAFETY: the host upholds the descriptor callback contract,
            // which is also `sdk::shm_frame`'s own safety contract for `handle`.
            unsafe {
                $crate::sdk::shm_frame::<$plugin>(
                    &LUMINATE_PLUGIN_INSTANCE,
                    context,
                    handle,
                    header,
                    pixels,
                    pixels_len,
                    result,
                )
            }
        }

        unsafe extern "C" fn luminate_plugin_shm_stream_end(
            context: $crate::PluginRequestContext,
            handle: u64,
            generation: u32,
        ) {
            // SAFETY: the host upholds the descriptor callback contract,
            // which is also `sdk::shm_stream_end`'s own safety contract for
            // `handle`.
            unsafe {
                $crate::sdk::shm_stream_end::<$plugin>(&LUMINATE_PLUGIN_INSTANCE, context, handle, generation)
            }
        }
    };
    (@shm_begin_expr none) => { None };
    (@shm_begin_expr native) => { Some(luminate_plugin_shm_stream_begin) };
    (@shm_apply_expr none) => { None };
    (@shm_apply_expr native) => { Some(luminate_plugin_shm_frame_apply) };
    (@shm_end_expr none) => { None };
    (@shm_end_expr native) => { Some(luminate_plugin_shm_stream_end) };
}
