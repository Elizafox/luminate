// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Named firmware-stored appearances selected by conditions outside direct
//! Luminate control.

use serde::{Deserialize, Serialize};

use crate::capability::{
    CctEmulation, ColourCapability, HardwareEffectsCapability, PersistenceCapability,
};
use crate::effect::Effect;
use crate::util::declare_opaque_id;

/// Opaque stable identifier for an appearance slot within one surface.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AppearanceSlotId(String);

declare_opaque_id!(AppearanceSlotId, "Creates an appearance-slot identifier.");

/// Appearance operations supported by an ordinary target or one appearance
/// slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppearanceCapability {
    /// Complete colour encodings accepted as independent alternatives.
    pub colour: Vec<ColourCapability>,

    /// Whether colour-temperature requests may be mapped onto other channels.
    pub cct_emulation: CctEmulation,

    /// Hardware-defined effects accepted by this appearance.
    pub hardware_effects: Option<HardwareEffectsCapability>,
}

impl Default for AppearanceCapability {
    fn default() -> Self {
        Self {
            colour: Vec::new(),
            cct_emulation: CctEmulation::Auto,
            hardware_effects: None,
        }
    }
}

/// Whether callers may omit values when updating a surface's appearance
/// slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppearanceSlotUpdatePolicy {
    /// Every slot can be updated without knowing or rewriting the others.
    Independent,

    /// A partial request is safe only when every omitted coupled value is
    /// already known and can be supplied to the provider.
    PartialIfKnown,

    /// Every mutation must explicitly contain every advertised slot.
    CompleteSet,
}

/// One firmware-stored appearance or effect program on a surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppearanceSlotDescriptor {
    /// Stable machine identifier within the owning surface.
    pub id: AppearanceSlotId,

    /// Human-readable presentation name.
    pub name: String,

    /// Effects and colours accepted by this slot.
    pub appearance: AppearanceCapability,

    /// Non-volatile storage behaviour of writes to this slot.
    pub persistence: PersistenceCapability,

    /// Cosmetic explanatory text. Empty means none.
    pub notes: Vec<String>,

    /// Cosmetic cautionary text. Empty means none.
    pub warnings: Vec<String>,
}

/// Named appearance slots exposed by one physical surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppearanceSlotsCapability {
    /// Slots in provider-defined presentation order.
    pub slots: Vec<AppearanceSlotDescriptor>,

    /// Completeness rule for one logical slot mutation.
    pub update_policy: AppearanceSlotUpdatePolicy,
}

/// One requested effect associated with a named appearance slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppearanceSlotValue {
    /// Slot to update.
    pub slot: AppearanceSlotId,

    /// Appearance or effect program to store.
    pub effect: Effect,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{CapabilitySet, ColourCapability, PersistenceRequirement};

    #[test]
    fn update_policy_has_stable_kebab_case_serialization() {
        let encoded = serde_json::to_string(&AppearanceSlotUpdatePolicy::PartialIfKnown)
            .expect("serialize appearance-slot update policy");

        assert_eq!(encoded, "\"partial-if-known\"");
        assert_eq!(
            serde_json::from_str::<AppearanceSlotUpdatePolicy>(&encoded)
                .expect("deserialize appearance-slot update policy"),
            AppearanceSlotUpdatePolicy::PartialIfKnown
        );
    }

    #[test]
    fn slot_capability_round_trips_without_losing_order_or_metadata() {
        let capability = AppearanceSlotsCapability {
            slots: vec![AppearanceSlotDescriptor {
                id: AppearanceSlotId::new("battery"),
                name: "Battery".to_owned(),
                appearance: AppearanceCapability {
                    colour: vec![ColourCapability::rgb8()],
                    cct_emulation: CctEmulation::Disabled,
                    hardware_effects: None,
                },
                persistence: PersistenceCapability::CurrentState {
                    requirement: PersistenceRequirement::Required,
                    explicit_commit: false,
                    readback: false,
                },
                notes: vec!["Selected while unplugged".to_owned()],
                warnings: vec!["Writes persistent storage".to_owned()],
            }],
            update_policy: AppearanceSlotUpdatePolicy::CompleteSet,
        };

        let encoded = serde_json::to_string(&capability).expect("serialize slot capability");
        let decoded: AppearanceSlotsCapability =
            serde_json::from_str(&encoded).expect("deserialize slot capability");

        assert_eq!(decoded, capability);
    }

    #[test]
    fn capability_set_without_slot_field_deserializes_as_no_slots() {
        let mut encoded =
            serde_json::to_value(CapabilitySet::default()).expect("serialize default capabilities");
        encoded
            .as_object_mut()
            .expect("capability set serializes as an object")
            .remove("appearance_slots");

        let decoded: CapabilitySet =
            serde_json::from_value(encoded).expect("deserialize older capability set");

        assert!(decoded.appearance_slots.is_none());
    }
}
