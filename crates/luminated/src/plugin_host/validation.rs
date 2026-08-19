// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Validation of topology claims and advertised callback contracts.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use luminate_core::capability::{
    BrightnessCapability, CapabilityScope, CapabilitySet, PersistenceCapability,
    StateReadbackCapability,
};
#[cfg(test)]
use luminate_core::capability::{ColourCapability, EffectParameter, HardwareEffectsCapability};
use luminate_plugin_api::{ClaimExclusivity, DeviceDescriptor};

/// Which optional callbacks a plugin exposes, bundled together (rather than
/// four separate `bool` parameters) purely to stay under clippy's
/// excessive-bool-parameters threshold.
#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "four independent, orthogonal yes/no callback flags; not state-machine states \
              or mutually exclusive choices, so an enum wouldn't fit any better than a struct"
)]
pub(super) struct PluginCallbacks {
    pub(super) apply: bool,
    pub(super) read_state: bool,
    pub(super) frame_upload: bool,
    pub(super) shm_frame: bool,
}

pub(super) fn validate_hardware_claims(descriptors: &[DeviceDescriptor]) -> Result<()> {
    let mut topology_claims = HashMap::new();
    for descriptor in descriptors {
        let mut claims = HashSet::new();
        for claim in &descriptor.claims {
            anyhow::ensure!(
                !claim.physical_identity.is_empty()
                    && claim.physical_identity.trim() == claim.physical_identity,
                "device {} has an empty or non-normalized physical claim identity",
                descriptor.id
            );
            anyhow::ensure!(
                !claim.control_domain.is_empty()
                    && claim.control_domain.trim() == claim.control_domain,
                "device {} has an empty or non-normalized claim control domain",
                descriptor.id
            );
            anyhow::ensure!(
                claims.insert((claim.bus, &claim.physical_identity, &claim.control_domain)),
                "device {} repeats a hardware claim",
                descriptor.id
            );
            let key = (
                claim.bus,
                claim.physical_identity.as_str(),
                claim.control_domain.as_str(),
            );
            if let Some(existing) = topology_claims.insert(key, claim.exclusivity) {
                anyhow::ensure!(
                    existing != ClaimExclusivity::Exclusive
                        && claim.exclusivity != ClaimExclusivity::Exclusive,
                    "plugin repeats an exclusive hardware claim across logical devices"
                );
            }
        }
    }
    Ok(())
}

pub(super) fn validate_topology_contract(
    descriptors: &[DeviceDescriptor],
    callbacks: PluginCallbacks,
) -> Result<()> {
    for device in descriptors {
        validate_physical_tags(&device.physical_tags, &format!("device {}", device.id))?;
        validate_capability_contract(
            &device.capabilities,
            &format!("device {}", device.id),
            CapabilityScope::Device,
            callbacks,
        )?;
        for surface in &device.surfaces {
            let location = format!("device {} surface {}", device.id, surface.id);
            validate_physical_tags(&surface.physical_tags, &location)?;
            validate_capability_contract(
                &surface.capabilities,
                &location,
                CapabilityScope::Surface,
                callbacks,
            )?;
            for element in &surface.elements {
                let element_location = format!("{location} element {}", element.id);
                validate_physical_tags(&element.physical_tags, &element_location)?;
                validate_capability_contract(
                    &element.capabilities,
                    &element_location,
                    CapabilityScope::Element,
                    callbacks,
                )?;
            }
        }
        for group in &device.groups {
            validate_capability_contract(
                &group.capabilities,
                &format!("device {} group {}", device.id, group.id),
                CapabilityScope::Device,
                callbacks,
            )?;
        }
    }
    Ok(())
}

fn validate_physical_tags(tags: &[String], location: &str) -> Result<()> {
    let mut seen = HashSet::new();

    for tag in tags {
        anyhow::ensure!(
            !tag.is_empty() && tag.trim() == tag,
            "{location} has an empty or non-normalized physical tag"
        );
        anyhow::ensure!(seen.insert(tag), "{location} repeats physical tag {tag}");
    }

    Ok(())
}

fn validate_capability_contract(
    capabilities: &CapabilitySet,
    location: &str,
    expected_scope: CapabilityScope,
    callbacks: PluginCallbacks,
) -> Result<()> {
    capabilities
        .validate()
        .map_err(|error| anyhow::anyhow!("{location} advertises invalid capabilities: {error}"))?;

    for scope in capability_scopes(capabilities) {
        anyhow::ensure!(
            scope == expected_scope,
            "{location} advertises a capability at {scope:?} scope instead of {expected_scope:?}"
        );
    }

    let readable = matches!(
        capabilities.state_readback,
        StateReadbackCapability::Readable { .. }
    ) || match capabilities.persistence {
        PersistenceCapability::CurrentState { readback, .. }
        | PersistenceCapability::Profiles { readback, .. } => readback,
        PersistenceCapability::None => false,
    };
    anyhow::ensure!(
        !readable || callbacks.read_state,
        "{location} advertises readback but the plugin exposes no state-readback callback"
    );

    let explicit_persistence_commit = match capabilities.persistence {
        PersistenceCapability::CurrentState {
            explicit_commit, ..
        }
        | PersistenceCapability::Profiles {
            explicit_commit, ..
        } => explicit_commit,
        PersistenceCapability::None => false,
    };
    anyhow::ensure!(
        !explicit_persistence_commit || callbacks.apply,
        "{location} requires explicit persistence commit but the plugin exposes no update callback"
    );
    let mutable = !capabilities.colour.is_empty()
        || !matches!(capabilities.brightness, BrightnessCapability::None)
        || capabilities.hardware_effects.is_some()
        || capabilities.appearance_slots.is_some()
        || capabilities.emission
        || capabilities.physical_power.is_some()
        || explicit_persistence_commit;
    anyhow::ensure!(
        !mutable || callbacks.apply,
        "{location} advertises mutable capabilities but the plugin exposes no update callback"
    );
    anyhow::ensure!(
        capabilities.frame_upload.is_none() || callbacks.frame_upload,
        "{location} advertises frame-upload capability but the plugin exposes no frame-upload callback"
    );

    if let Some(frame_upload) = &capabilities.frame_upload
        && let Some(shm) = &frame_upload.shm
    {
        anyhow::ensure!(
            callbacks.shm_frame,
            "{location} advertises shared-memory frame capability but the plugin exposes no \
             shared-memory streaming callbacks"
        );
        anyhow::ensure!(
            !shm.pixel_formats.is_empty(),
            "{location} advertises shared-memory frame capability with no accepted pixel formats"
        );
    }
    Ok(())
}

fn capability_scopes(capabilities: &CapabilitySet) -> impl Iterator<Item = CapabilityScope> + '_ {
    let brightness = match capabilities.brightness {
        BrightnessCapability::Independent { scope, .. } => Some(scope),
        BrightnessCapability::None => None,
    };
    brightness
        .into_iter()
        .chain(capabilities.frame_upload.as_ref().map(|value| value.scope))
        .chain(
            capabilities
                .hardware_effects
                .as_ref()
                .map(|value| value.scope),
        )
        .chain(
            capabilities
                .physical_power
                .as_ref()
                .map(|value| value.scope),
        )
        .chain(
            capabilities
                .appearance_slots
                .iter()
                .flat_map(|slots| slots.slots.iter())
                .filter_map(|slot| slot.appearance.hardware_effects.as_ref())
                .map(|effects| effects.scope),
        )
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;
