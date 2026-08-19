// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Cross-checks C enum constants against their canonical Rust discriminants.

#![allow(
    clippy::expect_used,
    clippy::tests_outside_test_module,
    reason = "Integration tests are crate roots and intentionally fail loudly when ABI declarations drift."
)]

use luminate_core::capability::{
    BufferingMode, CapabilityScope, ColourChannel, ColourEncoding, EffectDirection,
    FrameUpdateMode, PersistenceRequirement, ReadbackFidelity,
};
use luminate_core::policy::{Operation, RuleEffect};
use luminate_core::state::{
    AdoptionStatus, EmissionState, ObservationConfidence, ObservationSource, PhysicalPowerState,
    Reachability, ReconciliationStatus, StateFacetKind,
};

const CBINDGEN_CONFIG: &str = include_str!("../cbindgen.toml");

fn c_constant(name: &str) -> u32 {
    let prefix = format!("#define {name} UINT32_C(");
    let line = CBINDGEN_CONFIG
        .lines()
        .find(|line| line.starts_with(&prefix))
        .expect("C constant should be present");
    let value = line
        .strip_prefix(&prefix)
        .and_then(|remainder| remainder.split_once(')'))
        .map(|(value, _)| value)
        .expect("C constant should have a UINT32_C value");

    value
        .parse()
        .expect("C constant should contain a valid u32")
}

macro_rules! assert_discriminants {
    ($($variant:path => $constant:literal),+ $(,)?) => {
        $(assert_eq!($variant as u32, c_constant($constant), "{}", $constant);)+
    };
}

#[test]
fn rust_discriminants_match_c_constants() {
    assert_discriminants! {
        CapabilityScope::Element => "LUMINATE_SCOPE_ELEMENT",
        CapabilityScope::Surface => "LUMINATE_SCOPE_SURFACE",
        CapabilityScope::Device => "LUMINATE_SCOPE_DEVICE",
        CapabilityScope::Controller => "LUMINATE_SCOPE_CONTROLLER",
        ColourEncoding::Additive => "LUMINATE_COLOUR_ENCODING_ADDITIVE",
        ColourEncoding::Hsv => "LUMINATE_COLOUR_ENCODING_HSV",
        ColourEncoding::Hsl => "LUMINATE_COLOUR_ENCODING_HSL",
        ColourEncoding::Cct => "LUMINATE_COLOUR_ENCODING_CCT",
        ColourEncoding::Monochrome => "LUMINATE_COLOUR_ENCODING_MONOCHROME",
        ColourChannel::Red => "LUMINATE_COLOUR_CHANNEL_RED",
        ColourChannel::Green => "LUMINATE_COLOUR_CHANNEL_GREEN",
        ColourChannel::Blue => "LUMINATE_COLOUR_CHANNEL_BLUE",
        ColourChannel::White => "LUMINATE_COLOUR_CHANNEL_WHITE",
        ColourChannel::WarmWhite => "LUMINATE_COLOUR_CHANNEL_WARM_WHITE",
        ColourChannel::CoolWhite => "LUMINATE_COLOUR_CHANNEL_COOL_WHITE",
        ColourChannel::Amber => "LUMINATE_COLOUR_CHANNEL_AMBER",
        ColourChannel::Ultraviolet => "LUMINATE_COLOUR_CHANNEL_ULTRAVIOLET",
        ColourChannel::Hue => "LUMINATE_COLOUR_CHANNEL_HUE",
        ColourChannel::Saturation => "LUMINATE_COLOUR_CHANNEL_SATURATION",
        ColourChannel::Value => "LUMINATE_COLOUR_CHANNEL_VALUE",
        ColourChannel::Lightness => "LUMINATE_COLOUR_CHANNEL_LIGHTNESS",
        ColourChannel::Temperature => "LUMINATE_COLOUR_CHANNEL_TEMPERATURE",
        ColourChannel::Intensity => "LUMINATE_COLOUR_CHANNEL_INTENSITY",
        FrameUpdateMode::FullFrameOnly => "LUMINATE_FRAME_UPDATE_FULL_ONLY",
        FrameUpdateMode::Partial => "LUMINATE_FRAME_UPDATE_PARTIAL",
        FrameUpdateMode::Both => "LUMINATE_FRAME_UPDATE_BOTH",
        BufferingMode::Immediate => "LUMINATE_BUFFERING_IMMEDIATE",
        BufferingMode::ExplicitCommit => "LUMINATE_BUFFERING_EXPLICIT_COMMIT",
        BufferingMode::DoubleBuffered => "LUMINATE_BUFFERING_DOUBLE",
        PersistenceRequirement::Optional => "LUMINATE_PERSISTENCE_OPTIONAL",
        PersistenceRequirement::Required => "LUMINATE_PERSISTENCE_REQUIRED",
        ReadbackFidelity::BestEffort => "LUMINATE_READBACK_BEST_EFFORT",
        ReadbackFidelity::Exact => "LUMINATE_READBACK_EXACT",
        EffectDirection::Forward => "LUMINATE_DIRECTION_FORWARD",
        EffectDirection::Reverse => "LUMINATE_DIRECTION_REVERSE",
        EffectDirection::Clockwise => "LUMINATE_DIRECTION_CLOCKWISE",
        EffectDirection::CounterClockwise => "LUMINATE_DIRECTION_COUNTER_CLOCKWISE",
        EffectDirection::Inward => "LUMINATE_DIRECTION_INWARD",
        EffectDirection::Outward => "LUMINATE_DIRECTION_OUTWARD",
        EffectDirection::Random => "LUMINATE_DIRECTION_RANDOM",
        StateFacetKind::Appearance => "LUMINATE_FACET_APPEARANCE",
        StateFacetKind::Brightness => "LUMINATE_FACET_BRIGHTNESS",
        StateFacetKind::Emission => "LUMINATE_FACET_EMISSION",
        StateFacetKind::PhysicalPower => "LUMINATE_FACET_PHYSICAL_POWER",
        StateFacetKind::EffectiveAppearance => "LUMINATE_FACET_EFFECTIVE_APPEARANCE",
        Reachability::Unknown => "LUMINATE_REACHABILITY_UNKNOWN",
        Reachability::Reachable => "LUMINATE_REACHABILITY_REACHABLE",
        Reachability::Unavailable => "LUMINATE_REACHABILITY_UNAVAILABLE",
        ReconciliationStatus::Idle => "LUMINATE_RECONCILIATION_IDLE",
        ReconciliationStatus::Reconciling => "LUMINATE_RECONCILIATION_RECONCILING",
        ReconciliationStatus::Complete => "LUMINATE_RECONCILIATION_COMPLETE",
        ReconciliationStatus::Drifted => "LUMINATE_RECONCILIATION_DRIFTED",
        ReconciliationStatus::Failed => "LUMINATE_RECONCILIATION_FAILED",
        ObservationConfidence::Assumed => "LUMINATE_CONFIDENCE_ASSUMED",
        ObservationConfidence::BestEffort => "LUMINATE_CONFIDENCE_BEST_EFFORT",
        ObservationConfidence::Confirmed => "LUMINATE_CONFIDENCE_CONFIRMED",
        ObservationSource::SuccessfulApply => "LUMINATE_SOURCE_SUCCESSFUL_APPLY",
        ObservationSource::Readback => "LUMINATE_SOURCE_READBACK",
        ObservationSource::Derived => "LUMINATE_SOURCE_DERIVED",
        ObservationSource::AdoptedBaseline => "LUMINATE_SOURCE_ADOPTED_BASELINE",
        AdoptionStatus::NotApplicable => "LUMINATE_ADOPTION_NOT_APPLICABLE",
        AdoptionStatus::Pending => "LUMINATE_ADOPTION_PENDING",
        AdoptionStatus::Durable => "LUMINATE_ADOPTION_DURABLE",
        AdoptionStatus::IneligibleFidelity => "LUMINATE_ADOPTION_INELIGIBLE_FIDELITY",
        AdoptionStatus::PersistenceFailed => "LUMINATE_ADOPTION_PERSISTENCE_FAILED",
        EmissionState::Dark => "LUMINATE_EMISSION_DARK",
        EmissionState::Emitting => "LUMINATE_EMISSION_EMITTING",
        PhysicalPowerState::Off => "LUMINATE_PHYSICAL_POWER_OFF",
        PhysicalPowerState::On => "LUMINATE_PHYSICAL_POWER_ON",
        Operation::Observe => "LUMINATE_POLICY_OP_OBSERVE",
        Operation::Refresh => "LUMINATE_POLICY_OP_REFRESH",
        Operation::Control => "LUMINATE_POLICY_OP_CONTROL",
        Operation::HardwareAdministration => "LUMINATE_POLICY_OP_HARDWARE_ADMINISTRATION",
        Operation::DaemonAdministration => "LUMINATE_POLICY_OP_DAEMON_ADMINISTRATION",
        Operation::ManagePlugins => "LUMINATE_POLICY_OP_MANAGE_PLUGINS",
        Operation::CreateCollection => "LUMINATE_POLICY_OP_CREATE_COLLECTION",
        Operation::DestroyCollection => "LUMINATE_POLICY_OP_DESTROY_COLLECTION",
        Operation::ModifyCollection => "LUMINATE_POLICY_OP_MODIFY_COLLECTION",
        Operation::AdministerCollections => "LUMINATE_POLICY_OP_ADMINISTER_COLLECTIONS",
        Operation::ManagePolicy => "LUMINATE_POLICY_OP_MANAGE_POLICY",
        Operation::ManageAuthentication => "LUMINATE_POLICY_OP_MANAGE_AUTHENTICATION",
        Operation::AdministerFrontend => "LUMINATE_POLICY_OP_ADMINISTER_FRONTEND",
        Operation::CreateScene => "LUMINATE_POLICY_OP_CREATE_SCENE",
        Operation::ModifyScene => "LUMINATE_POLICY_OP_MODIFY_SCENE",
        Operation::DestroyScene => "LUMINATE_POLICY_OP_DESTROY_SCENE",
        Operation::AdministerScenes => "LUMINATE_POLICY_OP_ADMINISTER_SCENES",
        RuleEffect::Allow => "LUMINATE_RULE_EFFECT_ALLOW",
        RuleEffect::Deny => "LUMINATE_RULE_EFFECT_DENY",
    }
}
