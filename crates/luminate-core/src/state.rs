// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Canonical observed lighting state and reconciliation status.
//!
//! A facet is one independently knowable part of device state. An observation
//! records what the daemon knows about one facet; it is not desired state.

use serde::{Deserialize, Serialize};

use crate::appearance_slot::AppearanceSlotValue;
use crate::collection::CollectionId;
use crate::colour::Colour;
use crate::device;
use crate::effect::Effect;
use crate::target::TargetId;

/// Identifies an independently reportable part of lighting state.
///
/// Facets split along a configured/instantaneous line: `Appearance` and
/// `Brightness` report what the target is *configured* to show (its colour,
/// effect, and level), independent of whether that configuration is
/// currently visible. `Emission` and `PhysicalPower` report the
/// instantaneous fact of whether it's currently visible, at the target and
/// at the ancestor power domain respectively. A dimmer set to 50% while off
/// still reports `Brightness(50)`, the same way an off light still reports
/// its last configured colour; see [`AppearanceState`] for why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
pub enum StateFacetKind {
    /// Static colour or an active effect.
    Appearance = 0,

    /// Target-local brightness.
    Brightness = 1,

    /// Whether the target is locally dark or emitting light.
    Emission = 2,

    /// Whether an independently controlled hardware power domain is on.
    PhysicalPower = 3,

    /// What a target actually looks like right now, combining `Appearance`,
    /// `Emission`, and live frame-stream occupancy. Daemon-synthesized only;
    /// see [`EffectiveAppearanceState`].
    EffectiveAppearance = 4,

    /// Firmware-stored named appearance programs on one surface.
    AppearanceSlots = 5,
}

/// Known firmware-stored appearance-slot values for one surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppearanceSlotsState {
    /// Slot values known independently of any omitted values.
    pub values: Vec<AppearanceSlotValue>,

    /// Whether `values` covers every slot advertised by the surface.
    pub complete: bool,
}

/// The appearance currently rendered by a target.
///
/// Static colour has one canonical representation. Plugins report
/// `Static(Colour)` after applying or reading back [`Effect::Static`];
/// `Effect` is reserved for animated and hardware-defined effects.
///
/// `AppearanceState` answers "what is this target configured to display,"
/// not "is it currently visible right now" or "is it powered." Those are
/// [`EmissionState`] and [`PhysicalPowerState`] respectively, and `Emission`
/// is the sole authority for on/off. Report the configured colour or effect
/// regardless of power, the same way hardware that keeps a colour register
/// while powered off does. That is what lets a consumer show "red, but off"
/// instead of losing the colour the moment power drops.
///
/// Consequently, `Off` is never a valid observed `Appearance` value: `Off`
/// is an instruction to stop emitting, not a configuration, so it answers
/// `Emission`'s question, not this one. `Effect::Off` remains legitimate as
/// a *requested* `SetEffect` operation; a plugin acting on it should update
/// `Emission`, not overwrite the target's configured `Appearance`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AppearanceState {
    /// A fixed colour.
    Static(Colour),

    /// An animated or hardware effect.
    Effect(Effect),

    /// Constituents are configured with different appearances.
    ///
    /// This value is daemon-synthesized for aggregate targets and is never
    /// reported by a plugin for a canonical hardware target.
    Mixed,
}

/// Target-local light emission, independent of an ancestor's physical power.
///
/// This is the one place on/off state is reported; see [`AppearanceState`].
/// Derive it statelessly from underlying state on every read (for example
/// `power && brightness != 0`) rather than maintaining it as an
/// independently mutated field. A cached flag that isn't recomputed on
/// every relevant transition (brightness restored, power restored) will
/// drift out of sync with the state it's supposed to describe.
///
/// Prefer a genuine power/enable signal when a target has one. When it
/// doesn't (a target that is only ever "set to a colour," with no separate
/// enable register), deriving `Dark` from the configured colour being black
/// is an acceptable approximation: whether a target is currently emitting
/// visible light is a question worth answering even at the cost of
/// conflating "off" with "deliberately configured to black." Only skip
/// advertising `Emission` entirely for a target with no meaningful "on or
/// off" answer at all, for example one that is always emitting and can only
/// be recoloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum EmissionState {
    /// The target is not emitting light.
    Dark = 0,

    /// The target is emitting light.
    Emitting = 1,
}

/// State of a target that owns a physical power domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum PhysicalPowerState {
    /// The power domain is off.
    Off = 0,

    /// The power domain is on.
    On = 1,
}

/// What a target actually looks like right now: `Appearance` folded through
/// `Emission`, with an active frame stream taking precedence over both.
///
/// Unlike [`AppearanceState`], `Off` is a valid value here: this facet
/// answers "what is currently visible," not "what is configured." It is
/// synthesized by the daemon from a target's `Appearance` and `Emission`
/// observations, and its live frame-stream occupancy, and is never reported
/// by a plugin directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EffectiveAppearanceState {
    /// The target is not currently emitting light.
    Off,

    /// A fixed colour is currently visible.
    Static(Colour),

    /// An animated or hardware effect is currently visible.
    Effect(Effect),

    /// A client is actively pushing raw frames to this target. The daemon
    /// doesn't see pixel content, so this deliberately does not report a
    /// colour or effect: the last-configured `Appearance` is unrelated to
    /// what's actually on screen while a stream owns the target.
    Streaming,

    /// Constituents currently have different effective appearances.
    Mixed,
}

/// A typed value for one [`StateFacetKind`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FacetValue {
    /// Colour or effect appearance.
    Appearance(AppearanceState),

    /// A separate brightness value.
    Brightness(u32),

    /// Local light-emission state.
    Emission(EmissionState),

    /// Physical power-domain state.
    PhysicalPower(PhysicalPowerState),

    /// Daemon-synthesized combination of `Appearance` and `Emission`.
    EffectiveAppearance(EffectiveAppearanceState),

    /// Firmware-stored appearance programs, with explicit completeness.
    AppearanceSlots(AppearanceSlotsState),
}

impl FacetValue {
    /// Returns the facet represented by this value.
    #[must_use]
    pub const fn kind(&self) -> StateFacetKind {
        match self {
            Self::Appearance(_) => StateFacetKind::Appearance,
            Self::Brightness(_) => StateFacetKind::Brightness,
            Self::Emission(_) => StateFacetKind::Emission,
            Self::PhysicalPower(_) => StateFacetKind::PhysicalPower,
            Self::EffectiveAppearance(_) => StateFacetKind::EffectiveAppearance,
            Self::AppearanceSlots(_) => StateFacetKind::AppearanceSlots,
        }
    }
}

/// How strongly an observation is supported.
///
/// Ordered weakest to strongest so combining facets derived from more than
/// one observation (see [`FacetValue::EffectiveAppearance`]) can take the
/// minimum of their confidences: derivation never increases confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u32)]
pub enum ObservationConfidence {
    /// Projected from a successful write but not read back.
    Assumed = 0,

    /// Readback or derivation whose fidelity is insufficient to treat as
    /// exact current hardware state.
    BestEffort = 1,

    /// Sufficiently faithful hardware readback, or a sound derivation from
    /// confirmed inputs. Derivation never increases confidence: a value
    /// derived from a `BestEffort` input remains at most `BestEffort`.
    Confirmed = 2,
}

/// How the daemon obtained an observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum ObservationSource {
    /// Inferred from a successful hardware write.
    SuccessfulApply = 0,

    /// Reported by hardware readback.
    Readback = 1,

    /// Derived from another known state.
    Derived = 2,

    /// Loaded from a durable adopted baseline.
    AdoptedBaseline = 3,
}

/// Last-known knowledge of one facet at one canonical physical target.
///
/// `stale` is independent of confidence: a previously confirmed value may be
/// retained as useful history after a failed refresh or daemon restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FacetObservation {
    /// Target to which the value applies.
    pub target: TargetId,

    /// Observed or requested value.
    pub value: FacetValue,

    /// Confidence assigned to the observation.
    pub confidence: ObservationConfidence,

    /// How the observation was obtained.
    pub source: ObservationSource,

    /// Milliseconds since the Unix epoch.
    pub observed_at_ms: u64,

    /// Whether newer hardware state may exist.
    pub stale: bool,
}

/// Current availability of the device's state-reading path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum Reachability {
    /// Reachability has not been established.
    Unknown = 0,

    /// The device can currently be queried.
    Reachable = 1,

    /// The device cannot currently be queried.
    Unavailable = 2,
}

/// Progress or outcome of the most recent reconciliation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum ReconciliationStatus {
    /// No reconciliation has started.
    Idle = 0,

    /// Reconciliation is in progress.
    Reconciling = 1,

    /// Reconciliation completed successfully.
    Complete = 2,

    /// Observed hardware differs from desired state.
    Drifted = 3,

    /// The latest reconciliation attempt failed.
    Failed = 4,
}

/// Durability state for a facet selected by the `Adopt` policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum AdoptionStatus {
    /// Adoption does not apply to this facet.
    NotApplicable = 0,

    /// Eligible adoption has not yet become durable.
    Pending = 1,

    /// The adopted value was persisted.
    Durable = 2,

    /// Readback was not exact enough to adopt.
    IneligibleFidelity = 3,

    /// The value was read but could not be persisted.
    PersistenceFailed = 4,
}

/// Consumer-facing state knowledge and diagnostics for one device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceStateStatus {
    /// Device described by this status.
    pub device: device::DeviceId,

    /// Last-known facet observations.
    pub observations: Vec<FacetObservation>,

    /// Current readback availability.
    pub reachability: Reachability,

    /// Outcome of the latest reconciliation attempt.
    pub reconciliation: ReconciliationStatus,

    /// Durability status of adopted facets.
    pub adoption: Vec<(TargetId, StateFacetKind, AdoptionStatus)>,

    /// Most recent reconciliation error, if any.
    pub latest_error: Option<String>,

    /// Unix time of the latest reconciliation attempt, in milliseconds.
    pub latest_attempt_ms: Option<u64>,
}

/// Consumer-facing aggregate appearance state for one collection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionStateStatus {
    /// Collection described by this status.
    pub collection: CollectionId,

    /// Configured appearance synthesized from the collection's members.
    pub appearance: Option<AggregateAppearanceObservation>,

    /// Currently visible appearance synthesized from the collection's
    /// members.
    pub effective_appearance: Option<AggregateEffectiveAppearanceObservation>,
}

/// A collection's synthesized configured appearance and its supporting
/// observation metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AggregateAppearanceObservation {
    /// Homogeneous appearance, or [`AppearanceState::Mixed`].
    pub value: AppearanceState,

    /// Weakest confidence among the constituent observations.
    pub confidence: ObservationConfidence,

    /// Oldest constituent observation time, in milliseconds since the Unix
    /// epoch.
    pub observed_at_ms: u64,

    /// Whether any constituent observation may be stale.
    pub stale: bool,
}

/// A collection's synthesized effective appearance and its supporting
/// observation metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AggregateEffectiveAppearanceObservation {
    /// Homogeneous effective appearance, or
    /// [`EffectiveAppearanceState::Mixed`].
    pub value: EffectiveAppearanceState,

    /// Weakest confidence among the constituent observations.
    pub confidence: ObservationConfidence,

    /// Oldest constituent observation time, in milliseconds since the Unix
    /// epoch.
    pub observed_at_ms: u64,

    /// Whether any constituent observation may be stale.
    pub stale: bool,
}

impl DeviceStateStatus {
    /// Looks up the observation for one target and facet kind.
    ///
    /// `observations` carries no ordering contract beyond what a given
    /// producer happens to sort by for stable display; this is the one
    /// supported way to find a specific facet rather than relying on
    /// position (for example assuming the first entry is `Appearance`).
    #[must_use]
    pub fn observation(
        &self,
        target: &TargetId,
        kind: StateFacetKind,
    ) -> Option<&FacetObservation> {
        self.observations
            .iter()
            .find(|observation| &observation.target == target && observation.value.kind() == kind)
    }
}

/// A confirmed facet durably promoted beneath explicit desired overlays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdoptedFacet {
    /// Target to which the value applies.
    pub target: TargetId,

    /// Observed or requested value.
    pub value: FacetValue,

    /// Unix time at which the value was confirmed, in milliseconds.
    pub confirmed_at_ms: u64,
}
