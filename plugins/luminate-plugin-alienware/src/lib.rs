// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Alienware HID plugin entry points and update routing.

use luminate_core::control::ReconciliationPolicy;
use luminate_core::frame::FrameEnvelope;
use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::Duration;

use std::ffi::CStr;

use luminate_plugin_api::sdk::{BatchPlugin, FrameStreamingPlugin, LuminatePlugin};
use luminate_plugin_api::{
    DeviceDescriptor, PluginBus, PluginError, PluginProbeHint, PluginRequestContext, PluginTarget,
    PluginUpdate, PluginVendorId, ProbeHintKind, ProbeOutcome, luminate_export_plugin,
};

mod aw_elc_profile;
mod keyboard_layout;
mod protocol;
mod topology;

const NAME: &CStr = c"luminate-plugin-alienware";
const VERSION: &CStr = c"0.1.0";

const KEYBOARD_VID: u16 = 0x0d62;
const KEYBOARD_PID: u16 = 0xd2b1;
const AW_ELC_VID: u16 = 0x187c;
const AW_ELC_LEGACY_PID: u16 = 0x0550;
const AW_ELC_M16_R2_PID: u16 = 0x0551;

const KEYBOARD_HINT: &CStr = c"0d62:d2b1";
const AW_ELC_LEGACY_HINT: &CStr = c"187c:0550";
const AW_ELC_M16_R2_HINT: &CStr = c"187c:0551";

static BUSES: &[PluginBus] = &[PluginBus::Hid];
static VENDORS: &[PluginVendorId] = &[
    PluginVendorId {
        vendor: KEYBOARD_VID as u32,
        product: KEYBOARD_PID as u32,
    },
    PluginVendorId {
        vendor: AW_ELC_VID as u32,
        product: AW_ELC_LEGACY_PID as u32,
    },
    PluginVendorId {
        vendor: AW_ELC_VID as u32,
        product: AW_ELC_M16_R2_PID as u32,
    },
];
static HINTS: &[PluginProbeHint] = &[
    PluginProbeHint {
        kind: ProbeHintKind::HidVidPid,
        value: KEYBOARD_HINT.as_ptr().cast(),
    },
    PluginProbeHint {
        kind: ProbeHintKind::HidVidPid,
        value: AW_ELC_LEGACY_HINT.as_ptr().cast(),
    },
    PluginProbeHint {
        kind: ProbeHintKind::HidVidPid,
        value: AW_ELC_M16_R2_HINT.as_ptr().cast(),
    },
];

static WARNED_IMPORTED_PROFILES: Mutex<BTreeSet<(u16, u16)>> = Mutex::new(BTreeSet::new());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Discovery {
    keyboard: bool,
    keyboard_layout: Option<keyboard_layout::KeyboardLayoutId>,
    aw_elc: Option<aw_elc_profile::AwElcIdentity>,
}

impl Discovery {
    const fn empty() -> Self {
        Self {
            keyboard: false,
            keyboard_layout: None,
            aw_elc: None,
        }
    }

    const fn any(self) -> bool {
        self.keyboard || self.aw_elc.is_some()
    }
}

/// Caches the most recent live discovery so [`Alienware::upload_frame`] can
/// read the keyboard layout without re-probing hardware on every frame.
/// [`LuminatePlugin::probe`] and [`LuminatePlugin::topology`] both refresh it
/// from a new HID enumeration on every call, which is what lets the daemon's
/// ordinary topology re-pull (on resume, on a udev device-change event, or on
/// an operator-requested rescan) notice hardware that was unplugged or
/// plugged in since the plugin started; see the `rescan: none` note on the
/// `luminate_export_plugin!` block below.
struct Alienware {
    discovery: Mutex<Discovery>,
}

impl Alienware {
    fn refresh_discovery(&self) -> Discovery {
        refresh_discovery_cache(
            &self.discovery,
            &HidApiEnumerator,
            &protocol::HidApiTransport,
        )
    }

    #[allow(
        clippy::unwrap_used,
        reason = "A poisoned mutex means a panic elsewhere and possible violated invariants."
    )]
    fn cached_discovery(&self) -> Discovery {
        *self.discovery.lock().unwrap()
    }
}

/// Re-scans live and overwrites `cache` with the result, returning it too.
///
/// Split out from [`Alienware::refresh_discovery`] so the "a live rescan must
/// replace whatever was cached, not layer on top of it" behaviour can be tested
/// against a fake enumerator instead of real hidapi state.
#[allow(
    clippy::unwrap_used,
    reason = "A poisoned mutex means a panic elsewhere and possible violated invariants."
)]
fn refresh_discovery_cache(
    cache: &Mutex<Discovery>,
    enumerator: &dyn HidDeviceEnumerator,
    transport: &dyn protocol::HidTransport,
) -> Discovery {
    let discovery = discover_with(enumerator, transport);
    protocol::aw_elc::retain_live_shadow_for(discovery.aw_elc);
    let mut cached = cache.lock().unwrap();
    if cached.keyboard != discovery.keyboard || cached.keyboard_layout != discovery.keyboard_layout
    {
        protocol::keyboard::invalidate_per_key_shadow_for_discovery_change();
    }
    *cached = discovery;
    discovery
}

impl LuminatePlugin for Alienware {
    fn new() -> Result<Self, PluginError> {
        Ok(Self {
            discovery: Mutex::new(Discovery::empty()),
        })
    }

    fn probe(&self) -> ProbeOutcome {
        let discovery = self.refresh_discovery();
        tracing::info!(
            keyboard = discovery.keyboard,
            keyboard_layout = discovery
                .keyboard_layout
                .map_or("not-detected", keyboard_layout::KeyboardLayoutId::name),
            aw_elc = discovery
                .aw_elc
                .map_or("not-detected", |identity| identity.profile.model),
            "alienware hid probe complete"
        );
        if discovery.any() {
            ProbeOutcome::Ready
        } else {
            ProbeOutcome::Unsupported
        }
    }

    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
        let discovery = self.refresh_discovery();
        let mut devices = Vec::new();
        if discovery.keyboard {
            devices.push(topology::keyboard_device(discovery.keyboard_layout));
        }
        if let Some(identity) = discovery.aw_elc {
            devices.push(topology::aw_elc_device(identity));
        }
        Ok(devices)
    }

    fn apply(&self, _: &PluginRequestContext, update: &PluginUpdate) -> Result<(), PluginError> {
        let outcome = apply_update_with_transport(
            update,
            &protocol::HidApiTransport,
            self.cached_discovery().aw_elc,
        );
        log_outcome(
            update,
            &outcome,
            "alienware update applied",
            "alienware update rejected",
        );
        outcome
    }
}

impl BatchPlugin for Alienware {
    fn apply_batch(
        &self,
        _: &PluginRequestContext,
        updates: &[PluginUpdate],
    ) -> Vec<Result<(), PluginError>> {
        let outcomes = apply_batch_with_transport(
            updates,
            &protocol::HidApiTransport,
            self.cached_discovery().aw_elc,
        );
        for (update, outcome) in updates.iter().zip(&outcomes) {
            log_outcome(
                update,
                outcome,
                "alienware batch update applied",
                "alienware batch update rejected",
            );
        }
        outcomes
    }
}

impl FrameStreamingPlugin for Alienware {
    fn upload_frame(
        &self,
        _: &PluginRequestContext,
        target: &PluginTarget,
        envelope: &FrameEnvelope,
    ) -> Result<(), PluginError> {
        let PluginTarget::Surface { device, .. } = target else {
            return Err(PluginError::InvalidTarget(
                "Alienware frame streaming requires a surface target".to_owned(),
            ));
        };
        if device != topology::KEYBOARD_DEVICE_ID {
            return Err(PluginError::InvalidTarget(
                "target is not the Alienware keyboard surface".to_owned(),
            ));
        }

        #[allow(
            clippy::unwrap_used,
            clippy::unwrap_in_result,
            reason = "A poisoned mutex means a panic elsewhere and possible violated invariants."
        )]
        let layout = self
            .discovery
            .lock()
            .unwrap()
            .keyboard_layout
            .ok_or_else(|| {
                PluginError::Unsupported(
                    "keyboard layout not identified; frame streaming is unavailable".to_owned(),
                )
            })?;

        protocol::keyboard::apply_frame(
            &protocol::HidApiTransport,
            KEYBOARD_VID,
            KEYBOARD_PID,
            layout,
            target,
            envelope,
        )
        .map_err(classify_protocol_error)
    }
}

fn log_outcome(
    update: &PluginUpdate,
    outcome: &Result<(), PluginError>,
    success: &'static str,
    failure: &'static str,
) {
    match outcome {
        Ok(()) => {
            tracing::info!(target = %update.target, operation = %update.operation.name(), success);
        }
        Err(error) => {
            tracing::warn!(target = %update.target, operation = %update.operation.name(), error = %error, failure);
        }
    }
}

/// Applies many updates together, pulling out keyboard per-key `Element`
/// targets so they can share one hardware transaction
/// (`protocol::keyboard::apply_per_key_batch`) instead of one per key.
/// Everything else falls back to the existing single-update path unchanged,
/// one call per entry.
fn apply_batch_with_transport(
    updates: &[PluginUpdate],
    transport: &dyn protocol::HidTransport,
    aw_elc: Option<aw_elc_profile::AwElcIdentity>,
) -> Vec<Result<(), PluginError>> {
    let mut results: Vec<Option<Result<(), PluginError>>> = updates.iter().map(|_| None).collect();

    let mut per_key_indices = Vec::new();
    let mut per_key_entries = Vec::new();
    let mut aw_elc_indices = Vec::new();
    let mut aw_elc_entries = Vec::new();

    for (index, update) in updates.iter().enumerate() {
        if results.get(index).is_some_and(Option::is_some) {
            continue;
        }

        if let PluginTarget::Element {
            device,
            surface,
            element,
        } = &update.target
            && device == topology::KEYBOARD_DEVICE_ID
        {
            per_key_indices.push(index);
            per_key_entries.push((surface.as_str(), element.as_str(), &update.operation));
            continue;
        }

        if update.target.device_id() == topology::AW_ELC_DEVICE_ID
            && aw_elc.is_some_and(|identity| identity.profile != &aw_elc_profile::M16_R2)
        {
            aw_elc_indices.push(index);
            aw_elc_entries.push((&update.target, &update.operation));
            continue;
        }

        if let Some(slot) = results.get_mut(index) {
            *slot = Some(apply_update_with_transport(update, transport, aw_elc));
        }
    }

    if !per_key_entries.is_empty() {
        let outcomes = protocol::keyboard::apply_per_key_batch(
            transport,
            KEYBOARD_VID,
            KEYBOARD_PID,
            &per_key_entries,
        );
        for (index, outcome) in per_key_indices.into_iter().zip(outcomes) {
            if let Some(slot) = results.get_mut(index) {
                *slot = Some(match (outcome, updates.get(index)) {
                    (Ok(()), _) => Ok(()),
                    (Err(diagnostic), Some(_update)) => Err(classify_protocol_error(diagnostic)),
                    (Err(_), None) => Err(PluginError::Internal(
                        "batch update index is out of range".to_owned(),
                    )),
                });
            }
        }
    }

    if let Some(identity) = aw_elc
        && let Some(outcomes) =
            protocol::aw_elc::apply_live_batch(transport, identity, &aw_elc_entries)
    {
        for (index, outcome) in aw_elc_indices.into_iter().zip(outcomes) {
            if let Some(slot) = results.get_mut(index) {
                *slot = Some(outcome.map_err(classify_protocol_error));
            }
        }
    }

    results
        .into_iter()
        .map(|result| {
            result.unwrap_or_else(|| {
                Err(PluginError::Internal(
                    "batch result missing for an index".to_owned(),
                ))
            })
        })
        .collect()
}

/// Lists the vendor/product pairs of every HID device currently attached.
///
/// Abstracted out of [`discover`] purely so the live-rescan behaviour can be
/// exercised in tests without touching real hidapi state (see
/// `protocol::HidTransport` for the equivalent seam used for HID I/O).
trait HidDeviceEnumerator {
    fn vendor_product_pairs(&self) -> Vec<(u16, u16)>;
}

struct HidApiEnumerator;

impl HidDeviceEnumerator for HidApiEnumerator {
    fn vendor_product_pairs(&self) -> Vec<(u16, u16)> {
        match hidapi::HidApi::new() {
            Ok(api) => api
                .device_list()
                .map(|device| (device.vendor_id(), device.product_id()))
                .collect(),
            Err(error) => {
                tracing::warn!(error = %error, "failed to initialize hidapi");
                Vec::new()
            }
        }
    }
}

fn discover_with(
    enumerator: &dyn HidDeviceEnumerator,
    transport: &dyn protocol::HidTransport,
) -> Discovery {
    let mut discovery = Discovery::empty();
    let vendor_product_pairs = enumerator.vendor_product_pairs();
    discovery.keyboard = vendor_product_pairs.contains(&(KEYBOARD_VID, KEYBOARD_PID));

    let aw_elc_candidates = vendor_product_pairs
        .iter()
        .copied()
        .filter(|(vendor_id, product_id)| {
            *vendor_id == AW_ELC_VID
                && matches!(
                    *product_id,
                    aw_elc_profile::AW_ELC_LEGACY_PRODUCT_ID
                        | aw_elc_profile::AW_ELC_M16_R2_PRODUCT_ID
                )
        })
        .collect::<Vec<_>>();
    match aw_elc_candidates.as_slice() {
        [] => {}
        &[(vendor_id, product_id)] => {
            match protocol::aw_elc::read_identity(transport, vendor_id, product_id) {
                Ok(identity) => {
                    tracing::info!(
                        vendor_id = format_args!("{:#06x}", identity.vendor_id),
                        product_id = format_args!("{:#06x}", identity.product_id),
                        platform_id = format_args!("{:#06x}", identity.platform_id),
                        raw_zone_count = identity.reported_zone_count,
                        model = identity.profile.model,
                        "AW-ELC controller identity resolved"
                    );
                    warn_if_imported_profile(identity);
                    discovery.aw_elc = Some(identity);
                }
                Err(error) => {
                    tracing::warn!(error = %error, "AW-ELC controller identity unavailable");
                }
            }
        }
        candidates => {
            tracing::warn!(
                candidate_count = candidates.len(),
                "multiple AW-ELC controllers found; withholding all AW-ELC topology"
            );
        }
    }
    if discovery.keyboard {
        discovery.keyboard_layout = match protocol::keyboard::read_layout_identity(
            transport,
            KEYBOARD_VID,
            KEYBOARD_PID,
        ) {
            Ok(identity) => {
                let layout = identity.layout_id();
                tracing::info!(
                    hardware_variant = ?identity.hardware_variant,
                    layout = identity.layout,
                    chassis_colour = identity.chassis_colour,
                    layout_id = layout.name(),
                    "alienware keyboard layout identity read"
                );
                Some(layout)
            }
            Err(error) => {
                tracing::debug!(error = %error, "alienware keyboard layout identity unavailable");
                None
            }
        };
    }
    discovery
}

fn warn_if_imported_profile(identity: aw_elc_profile::AwElcIdentity) {
    if identity.profile.validation != aw_elc_profile::ValidationStatus::OpenRgbCorroborated {
        return;
    }

    let key = (identity.product_id, identity.platform_id);
    let mut warned = WARNED_IMPORTED_PROFILES
        .lock()
        .expect("AW-ELC profile warning lock poisoned");
    if warned.insert(key) {
        tracing::warn!(
            product_id = format_args!("{:#06x}", identity.product_id),
            platform_id = format_args!("{:#06x}", identity.platform_id),
            model = identity.profile.model,
            "AW-ELC profile is corroborated by OpenRGB but not hardware-validated in Luminate"
        );
    }
}

fn apply_update_with_transport(
    update: &PluginUpdate,
    transport: &dyn protocol::HidTransport,
    aw_elc: Option<aw_elc_profile::AwElcIdentity>,
) -> Result<(), PluginError> {
    let result = match &update.target {
        PluginTarget::Device { device }
        | PluginTarget::Surface { device, .. }
        | PluginTarget::Element { device, .. }
        | PluginTarget::Group { device, .. }
            if device == topology::KEYBOARD_DEVICE_ID =>
        {
            protocol::keyboard::apply(
                transport,
                KEYBOARD_VID,
                KEYBOARD_PID,
                &update.target,
                &update.operation,
            )
        }
        PluginTarget::Device { device }
        | PluginTarget::Surface { device, .. }
        | PluginTarget::Element { device, .. }
        | PluginTarget::Group { device, .. }
            if device == topology::AW_ELC_DEVICE_ID =>
        {
            let identity = aw_elc.ok_or_else(|| {
                PluginError::Unavailable(
                    "no resolved AW-ELC controller is currently available".to_owned(),
                )
            })?;
            protocol::aw_elc::apply(transport, identity, &update.target, &update.operation)
        }
        PluginTarget::Device { .. }
        | PluginTarget::Surface { .. }
        | PluginTarget::Element { .. }
        | PluginTarget::Group { .. } => {
            return Err(PluginError::InvalidTarget(
                "target is not owned by the Alienware plugin".to_owned(),
            ));
        }
    };
    result.map_err(classify_protocol_error)
}

fn classify_protocol_error(diagnostic: String) -> PluginError {
    if diagnostic.contains("updated less than") {
        return PluginError::RateLimited {
            diagnostic,
            retry_after: Duration::from_secs(5),
        };
    }
    if diagnostic.starts_with("failed to initialize hidapi")
        || diagnostic.starts_with("failed to open HID device")
    {
        return PluginError::Unavailable(diagnostic);
    }
    if diagnostic.starts_with("failed to send HID")
        || diagnostic.starts_with("failed to read keyboard")
    {
        return PluginError::Io(diagnostic);
    }
    if diagnostic.contains("poisoned") {
        return PluginError::Internal(diagnostic);
    }
    if diagnostic.contains("not implemented")
        || diagnostic.contains("does not expose")
        || diagnostic.contains("supports only")
        || diagnostic.contains("exposed as Alienware hardware effects")
    {
        return PluginError::Unsupported(diagnostic);
    }
    PluginError::InvalidArgument(diagnostic)
}

luminate_export_plugin! {
    plugin: Alienware,
    name: NAME,
    version: VERSION,
    priority: 200,
    recommended_reconciliation: Some(ReconciliationPolicy::Restore),
    buses: BUSES,
    vendors: VENDORS,
    probe_hints: HINTS,
    start: none,
    // Probe and topology enumerate live devices, and apply opens a fresh HID
    // path. The daemon's topology re-pull needs no extra invalidation.
    rescan: none,
    batch: native,
    read_state: none,
    frame_upload: native,
    shm_frame: none,
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
