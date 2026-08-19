// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Complete native D-Bus projection of target capabilities.

use luminate::appearance_slot::{AppearanceCapability, AppearanceSlotsCapability};
use luminate::capability::{
    BrightnessCapability, BufferingMode, CapabilityScope, CctEmulation, FrameUpdateMode,
    FrameUploadCapability, HardwareEffectsCapability, PersistenceCapability,
    PersistenceRequirement, PhysicalPowerCapability, PowerDomainRef, ReadbackFidelity,
    ShmFrameCapability, ShmFrameShape, StateReadbackCapability,
};
use luminate::{AppearanceSlotUpdatePolicy, CapabilitySet, ShmPixelFormat, StateFacetKind};

use crate::convert::{effect_descriptor, static_colour_capability};
use crate::error::MethodError;
use crate::management::{Dictionary, dictionary, owned};

pub(crate) fn capabilities(value: &CapabilitySet) -> Result<Dictionary, MethodError> {
    let mut result = dictionary([
        (
            "Colour",
            owned(
                value
                    .colour
                    .iter()
                    .map(static_colour_capability)
                    .collect::<Vec<_>>(),
            )?,
        ),
        ("CctEmulation", owned(cct_emulation(value.cct_emulation))?),
        ("Brightness", owned(brightness(&value.brightness)?)?),
        ("Persistence", owned(persistence(&value.persistence)?)?),
        (
            "StateReadback",
            owned(state_readback(&value.state_readback)?)?,
        ),
        ("Emission", owned(value.emission)?),
        ("OffIsWearSafe", owned(value.off_is_wear_safe)?),
    ]);
    insert_optional(
        &mut result,
        "FrameUpload",
        value.frame_upload.as_ref().map(frame_upload).transpose()?,
    )?;
    insert_optional(
        &mut result,
        "HardwareEffects",
        value
            .hardware_effects
            .as_ref()
            .map(hardware_effects)
            .transpose()?,
    )?;
    insert_optional(
        &mut result,
        "AppearanceSlots",
        value
            .appearance_slots
            .as_ref()
            .map(appearance_slots)
            .transpose()?,
    )?;
    insert_optional(
        &mut result,
        "PhysicalPower",
        value.physical_power.map(physical_power).transpose()?,
    )?;
    insert_optional(
        &mut result,
        "PowerDomain",
        value.power_domain.as_ref().map(power_domain).transpose()?,
    )?;
    Ok(result)
}

fn insert_optional(
    result: &mut Dictionary,
    field: &str,
    value: Option<Dictionary>,
) -> Result<(), MethodError> {
    result.insert(format!("Has{field}"), owned(value.is_some())?);
    if let Some(value) = value {
        result.insert(field.to_owned(), owned(value)?);
    }
    Ok(())
}

fn brightness(value: &BrightnessCapability) -> Result<Dictionary, MethodError> {
    match value {
        BrightnessCapability::None => Ok(dictionary([("Kind", owned("none")?)])),
        BrightnessCapability::Independent {
            bits,
            maximum,
            scope: value_scope,
        } => Ok(dictionary([
            ("Kind", owned("independent")?),
            ("Bits", owned(*bits)?),
            ("Maximum", owned(*maximum)?),
            ("Scope", owned(scope(*value_scope))?),
        ])),
    }
}

fn frame_upload(value: &FrameUploadCapability) -> Result<Dictionary, MethodError> {
    let mut result = dictionary([
        ("Scope", owned(scope(value.scope))?),
        ("UpdateMode", owned(update_mode(value.update_mode))?),
        ("HasMaximumRateHz", owned(value.max_rate_hz.is_some())?),
        ("Atomic", owned(value.atomic)?),
        ("Buffering", owned(buffering(value.buffering))?),
        ("HasSharedMemory", owned(value.shm.is_some())?),
    ]);
    if let Some(rate) = value.max_rate_hz {
        result.insert("MaximumRateHz".into(), owned(rate)?);
    }
    if let Some(shm) = &value.shm {
        result.insert("SharedMemory".into(), owned(shm_frame(shm)?)?);
    }
    Ok(result)
}

fn shm_frame(value: &ShmFrameCapability) -> Result<Dictionary, MethodError> {
    let mut result = dictionary([
        (
            "PixelFormats",
            owned(
                value
                    .pixel_formats
                    .iter()
                    .copied()
                    .map(pixel_format)
                    .collect::<Vec<_>>(),
            )?,
        ),
        ("Shape", owned(shm_shape(value.shape)?)?),
        ("HasMaximumRateHz", owned(value.max_rate_hz.is_some())?),
    ]);
    if let Some(rate) = value.max_rate_hz {
        result.insert("MaximumRateHz".into(), owned(rate)?);
    }
    Ok(result)
}

fn shm_shape(value: ShmFrameShape) -> Result<Dictionary, MethodError> {
    match value {
        ShmFrameShape::Linear { pixel_count } => Ok(dictionary([
            ("Kind", owned("linear")?),
            ("PixelCount", owned(pixel_count)?),
        ])),
        ShmFrameShape::Matrix { width, height } => Ok(dictionary([
            ("Kind", owned("matrix")?),
            ("Width", owned(width)?),
            ("Height", owned(height)?),
        ])),
    }
}

fn hardware_effects(value: &HardwareEffectsCapability) -> Result<Dictionary, MethodError> {
    Ok(dictionary([
        ("Scope", owned(scope(value.scope))?),
        (
            "ConcurrentWithStreaming",
            owned(value.concurrent_with_streaming)?,
        ),
        (
            "Effects",
            owned(
                value
                    .effects
                    .iter()
                    .map(effect_descriptor)
                    .collect::<Vec<_>>(),
            )?,
        ),
    ]))
}

fn appearance_slots(value: &AppearanceSlotsCapability) -> Result<Dictionary, MethodError> {
    Ok(dictionary([
        ("UpdatePolicy", owned(slot_policy(value.update_policy))?),
        (
            "Slots",
            owned(
                value
                    .slots
                    .iter()
                    .map(|slot| {
                        Ok(dictionary([
                            ("Id", owned(slot.id.as_str().to_owned())?),
                            ("Name", owned(slot.name.clone())?),
                            ("Appearance", owned(appearance(&slot.appearance)?)?),
                            ("Persistence", owned(persistence(&slot.persistence)?)?),
                            ("Notes", owned(slot.notes.clone())?),
                            ("Warnings", owned(slot.warnings.clone())?),
                        ]))
                    })
                    .collect::<Result<Vec<_>, MethodError>>()?,
            )?,
        ),
    ]))
}

fn appearance(value: &AppearanceCapability) -> Result<Dictionary, MethodError> {
    let mut result = dictionary([
        (
            "Colour",
            owned(
                value
                    .colour
                    .iter()
                    .map(static_colour_capability)
                    .collect::<Vec<_>>(),
            )?,
        ),
        ("CctEmulation", owned(cct_emulation(value.cct_emulation))?),
        (
            "HasHardwareEffects",
            owned(value.hardware_effects.is_some())?,
        ),
    ]);
    if let Some(effects) = &value.hardware_effects {
        result.insert("HardwareEffects".into(), owned(hardware_effects(effects)?)?);
    }
    Ok(result)
}

fn persistence(value: &PersistenceCapability) -> Result<Dictionary, MethodError> {
    match value {
        PersistenceCapability::None => Ok(dictionary([("Kind", owned("none")?)])),
        PersistenceCapability::CurrentState {
            requirement,
            explicit_commit,
            readback,
        } => Ok(dictionary([
            ("Kind", owned("current-state")?),
            ("Requirement", owned(persistence_requirement(*requirement))?),
            ("ExplicitCommit", owned(*explicit_commit)?),
            ("Readback", owned(*readback)?),
        ])),
        PersistenceCapability::Profiles {
            requirement,
            slots,
            explicit_commit,
            readback,
        } => Ok(dictionary([
            ("Kind", owned("profiles")?),
            ("Requirement", owned(persistence_requirement(*requirement))?),
            ("Slots", owned(*slots)?),
            ("ExplicitCommit", owned(*explicit_commit)?),
            ("Readback", owned(*readback)?),
        ])),
    }
}

fn state_readback(value: &StateReadbackCapability) -> Result<Dictionary, MethodError> {
    match value {
        StateReadbackCapability::None => Ok(dictionary([("Kind", owned("none")?)])),
        StateReadbackCapability::Readable {
            facets,
            read_disturbs_output,
            notifies_external_changes,
        } => Ok(dictionary([
            ("Kind", owned("readable")?),
            (
                "Facets",
                owned(
                    facets
                        .iter()
                        .map(|facet| (facet_name(facet.facet), fidelity(facet.fidelity)))
                        .collect::<Vec<_>>(),
                )?,
            ),
            ("ReadDisturbsOutput", owned(*read_disturbs_output)?),
            (
                "NotifiesExternalChanges",
                owned(*notifies_external_changes)?,
            ),
        ])),
    }
}

fn power_domain(value: &PowerDomainRef) -> Result<Dictionary, MethodError> {
    match value {
        PowerDomainRef::Device => Ok(dictionary([("Kind", owned("device")?)])),
        PowerDomainRef::Surface { surface } => Ok(dictionary([
            ("Kind", owned("surface")?),
            ("Surface", owned(surface.clone())?),
        ])),
    }
}

fn physical_power(value: PhysicalPowerCapability) -> Result<Dictionary, MethodError> {
    Ok(dictionary([("Scope", owned(scope(value.scope))?)]))
}

fn scope(value: CapabilityScope) -> &'static str {
    match value {
        CapabilityScope::Element => "element",
        CapabilityScope::Surface => "surface",
        CapabilityScope::Device => "device",
        CapabilityScope::Controller => "controller",
    }
}

fn cct_emulation(value: CctEmulation) -> &'static str {
    match value {
        CctEmulation::Auto => "auto",
        CctEmulation::Disabled => "disabled",
    }
}

fn update_mode(value: FrameUpdateMode) -> &'static str {
    match value {
        FrameUpdateMode::FullFrameOnly => "full-frame-only",
        FrameUpdateMode::Partial => "partial",
        FrameUpdateMode::Both => "both",
    }
}

fn buffering(value: BufferingMode) -> &'static str {
    match value {
        BufferingMode::Immediate => "immediate",
        BufferingMode::ExplicitCommit => "explicit-commit",
        BufferingMode::DoubleBuffered => "double-buffered",
    }
}

fn pixel_format(value: ShmPixelFormat) -> &'static str {
    match value {
        ShmPixelFormat::Rgb8 => "rgb8",
        ShmPixelFormat::Rgbw8 => "rgbw8",
        ShmPixelFormat::Mono8 => "mono8",
        ShmPixelFormat::Rgbx8 => "rgbx8",
    }
}

fn slot_policy(value: AppearanceSlotUpdatePolicy) -> &'static str {
    match value {
        AppearanceSlotUpdatePolicy::Independent => "independent",
        AppearanceSlotUpdatePolicy::PartialIfKnown => "partial-if-known",
        AppearanceSlotUpdatePolicy::CompleteSet => "complete-set",
    }
}

fn persistence_requirement(value: PersistenceRequirement) -> &'static str {
    match value {
        PersistenceRequirement::Optional => "optional",
        PersistenceRequirement::Required => "required",
    }
}

fn fidelity(value: ReadbackFidelity) -> &'static str {
    match value {
        ReadbackFidelity::BestEffort => "best-effort",
        ReadbackFidelity::Exact => "exact",
    }
}

fn facet_name(value: StateFacetKind) -> &'static str {
    match value {
        StateFacetKind::Appearance => "appearance",
        StateFacetKind::Brightness => "brightness",
        StateFacetKind::Emission => "emission",
        StateFacetKind::PhysicalPower => "physical-power",
        StateFacetKind::EffectiveAppearance => "effective-appearance",
        StateFacetKind::AppearanceSlots => "appearance-slots",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use luminate::appearance_slot::{
        AppearanceSlotDescriptor, AppearanceSlotId, AppearanceSlotsCapability,
    };
    use luminate::capability::{ReadableFacet, ShmFrameCapability};

    fn boolean(value: &Dictionary, key: &str) -> bool {
        bool::try_from(value[key].try_clone().expect("clone D-Bus value"))
            .expect("field should be boolean")
    }

    #[test]
    fn default_capabilities_preserve_explicit_absence() {
        let encoded = capabilities(&CapabilitySet::default()).expect("encode defaults");

        assert!(!boolean(&encoded, "HasFrameUpload"));
        assert!(!boolean(&encoded, "HasHardwareEffects"));
        assert!(!boolean(&encoded, "HasAppearanceSlots"));
        assert!(!boolean(&encoded, "HasPhysicalPower"));
        assert!(!boolean(&encoded, "HasPowerDomain"));
        assert!(!encoded.contains_key("FrameUpload"));
        assert!(!encoded.contains_key("PowerDomain"));
    }

    #[test]
    fn broad_capability_families_have_native_dictionary_values() {
        let value = CapabilitySet {
            colour: vec![luminate::ColourCapability::rgb8()],
            cct_emulation: CctEmulation::Disabled,
            brightness: BrightnessCapability::Independent {
                bits: 8,
                maximum: 255,
                scope: CapabilityScope::Device,
            },
            frame_upload: Some(FrameUploadCapability {
                scope: CapabilityScope::Surface,
                update_mode: FrameUpdateMode::Both,
                max_rate_hz: Some(60),
                atomic: true,
                buffering: BufferingMode::DoubleBuffered,
                shm: Some(ShmFrameCapability {
                    pixel_formats: vec![ShmPixelFormat::Rgb8, ShmPixelFormat::Rgbx8],
                    shape: ShmFrameShape::Matrix {
                        width: 22,
                        height: 6,
                    },
                    max_rate_hz: None,
                }),
            }),
            hardware_effects: Some(HardwareEffectsCapability {
                effects: Vec::new(),
                scope: CapabilityScope::Controller,
                concurrent_with_streaming: true,
            }),
            appearance_slots: Some(AppearanceSlotsCapability {
                slots: vec![AppearanceSlotDescriptor {
                    id: AppearanceSlotId::new("battery"),
                    name: "Battery".into(),
                    appearance: AppearanceCapability::default(),
                    persistence: PersistenceCapability::CurrentState {
                        requirement: PersistenceRequirement::Optional,
                        explicit_commit: true,
                        readback: true,
                    },
                    notes: vec!["note".into()],
                    warnings: vec!["warning".into()],
                }],
                update_policy: AppearanceSlotUpdatePolicy::CompleteSet,
            }),
            persistence: PersistenceCapability::Profiles {
                requirement: PersistenceRequirement::Required,
                slots: 4,
                explicit_commit: true,
                readback: false,
            },
            state_readback: StateReadbackCapability::Readable {
                facets: vec![ReadableFacet {
                    facet: StateFacetKind::AppearanceSlots,
                    fidelity: ReadbackFidelity::Exact,
                }],
                read_disturbs_output: false,
                notifies_external_changes: true,
            },
            emission: true,
            off_is_wear_safe: true,
            physical_power: Some(PhysicalPowerCapability {
                scope: CapabilityScope::Device,
            }),
            power_domain: Some(PowerDomainRef::Surface {
                surface: "power".into(),
            }),
        };

        let encoded = capabilities(&value).expect("encode capabilities");
        for field in [
            "FrameUpload",
            "HardwareEffects",
            "AppearanceSlots",
            "PhysicalPower",
            "PowerDomain",
        ] {
            assert!(boolean(&encoded, &format!("Has{field}")));
            assert!(encoded.contains_key(field));
        }
        assert!(boolean(&encoded, "Emission"));
        assert!(boolean(&encoded, "OffIsWearSafe"));
    }

    #[test]
    fn leaf_enum_names_and_alternate_shapes_are_stable() {
        assert_eq!(
            [
                CapabilityScope::Element,
                CapabilityScope::Surface,
                CapabilityScope::Device,
                CapabilityScope::Controller,
            ]
            .map(scope),
            ["element", "surface", "device", "controller"]
        );
        assert_eq!(
            [
                FrameUpdateMode::FullFrameOnly,
                FrameUpdateMode::Partial,
                FrameUpdateMode::Both,
            ]
            .map(update_mode),
            ["full-frame-only", "partial", "both"]
        );
        assert_eq!(
            [
                BufferingMode::Immediate,
                BufferingMode::ExplicitCommit,
                BufferingMode::DoubleBuffered,
            ]
            .map(buffering),
            ["immediate", "explicit-commit", "double-buffered"]
        );
        assert_eq!(
            [
                ShmPixelFormat::Rgb8,
                ShmPixelFormat::Rgbw8,
                ShmPixelFormat::Mono8,
                ShmPixelFormat::Rgbx8,
            ]
            .map(pixel_format),
            ["rgb8", "rgbw8", "mono8", "rgbx8"]
        );
        assert_eq!(
            [
                AppearanceSlotUpdatePolicy::Independent,
                AppearanceSlotUpdatePolicy::PartialIfKnown,
                AppearanceSlotUpdatePolicy::CompleteSet,
            ]
            .map(slot_policy),
            ["independent", "partial-if-known", "complete-set"]
        );

        let linear =
            shm_shape(ShmFrameShape::Linear { pixel_count: 12 }).expect("encode linear shape");
        let device_domain = power_domain(&PowerDomainRef::Device).expect("encode device domain");
        assert!(linear.contains_key("PixelCount"));
        assert!(!linear.contains_key("Width"));
        assert!(!device_domain.contains_key("Surface"));
    }
}
