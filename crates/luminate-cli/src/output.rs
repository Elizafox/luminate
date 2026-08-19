// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Human-readable topology and capability output.

#![allow(
    clippy::print_stdout,
    reason = "The CLI intentionally writes command output to stdout."
)]

use luminate_core::capability::{
    BrightnessCapability, BufferingMode, CapabilityScope, CapabilitySet, ColourCapability,
    ColourChannel, EffectParameter, FrameUpdateMode, PersistenceCapability, PersistenceRequirement,
    StateReadbackCapability,
};
use luminate_core::device::Device;
use luminate_core::element::{Element, ElementGeometry, ElementKind};
use luminate_core::group::{Group, GroupKind, GroupMember};
use luminate_core::surface::{Surface, SurfaceKind};
use luminate_platform::terminal::{escape, escape_json};
use serde::Serialize;

/// Escapes control characters in human-readable output so untrusted daemon or
/// plugin text cannot inject terminal escape sequences. JSON uses
/// [`terminal_json_pretty`] so its decoded values remain unchanged.
pub(crate) fn terminal_safe(input: &str) -> String {
    escape(input).into_owned()
}

/// Serializes JSON without allowing raw terminal controls in its byte form.
///
/// Escaping C1 and DEL this way does not change the value recovered by a JSON
/// parser, so machine-readable output retains its existing data contract.
pub(crate) fn terminal_json_pretty(value: &impl Serialize) -> serde_json::Result<String> {
    let serialized = serde_json::to_string_pretty(value)?;
    Ok(escape_json(&serialized).into_owned())
}

pub(crate) fn print_device(device: &Device) {
    println!("{}", terminal_safe(&device.name));
    println!("  id: {}", terminal_safe(&device.id.to_string()));
    if let Some(vendor) = &device.vendor {
        println!("  vendor: {}", terminal_safe(vendor));
    }
    if let Some(model) = &device.model {
        println!("  model: {}", terminal_safe(model));
    }
    if let Some(category) = &device.category {
        println!("  category: {}", terminal_safe(category.as_str()));
    }
    if !device.physical_tags.is_empty() {
        println!(
            "  physical tags: {}",
            device
                .physical_tags
                .iter()
                .map(|tag| terminal_safe(tag))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!(
        "  capabilities: {}",
        format_capabilities(&device.capabilities)
    );
    print_notes_and_warnings("  ", &device.notes, &device.warnings);
    println!("  surfaces:");
    if device.surfaces.is_empty() {
        println!("    (none)");
    } else {
        for surface in &device.surfaces {
            print_surface(surface);
        }
    }
    println!("  groups:");
    if device.groups.is_empty() {
        println!("    (none)");
    } else {
        for group in &device.groups {
            print_group(group);
        }
    }
}

pub(crate) fn print_surface(surface: &Surface) {
    println!(
        "    - {} ({}) [{}]",
        terminal_safe(surface.id.as_str()),
        terminal_safe(&surface.name),
        format_surface_kind(&surface.kind)
    );
    println!(
        "      capabilities: {}",
        format_capabilities(&surface.capabilities)
    );
    if !surface.physical_tags.is_empty() {
        println!(
            "      physical tags: {}",
            surface
                .physical_tags
                .iter()
                .map(|tag| terminal_safe(tag))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    print_notes_and_warnings("      ", &surface.notes, &surface.warnings);
    println!("      elements:");
    if surface.elements.is_empty() {
        println!("        (none)");
    } else {
        for element in &surface.elements {
            print_element(element);
        }
    }
}

pub(crate) fn print_element(element: &Element) {
    let name = element.name.as_deref().unwrap_or("(unnamed)");
    println!(
        "        - {} ({}) [{}]",
        terminal_safe(element.id.as_str()),
        terminal_safe(name),
        format_element_kind(&element.kind)
    );
    if let Some(geometry) = &element.geometry {
        println!("          geometry: {}", format_geometry(geometry));
    }
    println!(
        "          capabilities: {}",
        format_capabilities(&element.capabilities)
    );
    print_notes_and_warnings("          ", &element.notes, &element.warnings);
}

pub(crate) fn print_group(group: &Group) {
    println!(
        "    - {} ({}) [{}]",
        terminal_safe(group.id.as_str()),
        terminal_safe(&group.name),
        format_group_kind(group.kind)
    );
    if let Some(description) = &group.description {
        println!("      description: {}", terminal_safe(description));
    }
    println!(
        "      capabilities: {}",
        format_capabilities(&group.capabilities)
    );
    print_notes_and_warnings("      ", &group.notes, &group.warnings);
    println!("      members:");
    if group.members.is_empty() {
        println!("        (none)");
    } else {
        for member in &group.members {
            println!("        - {}", terminal_safe(&format_group_member(member)));
        }
    }
}

fn print_notes_and_warnings(indent: &str, notes: &[String], warnings: &[String]) {
    for note in notes {
        println!("{indent}note: {}", terminal_safe(note));
    }
    for warning in warnings {
        println!("{indent}warning: {}", terminal_safe(warning));
    }
}

pub(crate) fn format_capabilities(capabilities: &CapabilitySet) -> String {
    let mut parts = vec![
        format!(
            "colour={}",
            if capabilities.colour.is_empty() {
                "none".to_owned()
            } else {
                capabilities
                    .colour
                    .iter()
                    .map(format_colour_capability)
                    .collect::<Vec<_>>()
                    .join("|")
            }
        ),
        format!(
            "brightness={}",
            format_brightness_capability(&capabilities.brightness)
        ),
        format!("emission={}", capabilities.emission),
        format!(
            "persistence={}",
            format_persistence_capability(&capabilities.persistence)
        ),
        format!(
            "readback={}",
            format_state_readback_capability(&capabilities.state_readback)
        ),
    ];

    if let Some(frame_upload) = &capabilities.frame_upload {
        parts.push(format!(
            "frame-upload={} {} {}{}",
            format_scope(frame_upload.scope),
            format_frame_update_mode(frame_upload.update_mode),
            format_buffering_mode(frame_upload.buffering),
            frame_upload
                .max_rate_hz
                .map_or_else(String::new, |rate| format!(" {rate}Hz"))
        ));
    }

    if let Some(hardware_effects) = &capabilities.hardware_effects {
        let effects = hardware_effects
            .effects
            .iter()
            .map(|effect| {
                if effect.parameters.is_empty() {
                    terminal_safe(effect.id.as_str())
                } else {
                    format!(
                        "{}({})",
                        terminal_safe(effect.id.as_str()),
                        effect
                            .parameters
                            .iter()
                            .map(format_effect_parameter)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!(
            "hw-effects={} [{}]{}",
            format_scope(hardware_effects.scope),
            effects,
            if hardware_effects.concurrent_with_streaming {
                ", concurrent"
            } else {
                ""
            }
        ));
    }

    parts.join("; ")
}

fn format_colour_capability(capability: &ColourCapability) -> String {
    match capability {
        ColourCapability::Additive(channels) => format!(
            "additive[{}]",
            channels
                .iter()
                .map(|channel| format!(
                    "{}:{}",
                    format_colour_channel(channel.channel),
                    channel.bits
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
        ColourCapability::Hsv {
            hue_bits,
            saturation_bits,
            value_bits,
        } => {
            format!("hsv[hue:{hue_bits},saturation:{saturation_bits},value:{value_bits}]")
        }
        ColourCapability::Hsl {
            hue_bits,
            saturation_bits,
            lightness_bits,
        } => {
            format!("hsl[hue:{hue_bits},saturation:{saturation_bits},lightness:{lightness_bits}]")
        }
        ColourCapability::Cct { bits } => format!("cct[temperature:{bits}]"),
        ColourCapability::Monochrome { bits } => format!("monochrome[intensity:{bits}]"),
    }
}

fn format_brightness_capability(capability: &BrightnessCapability) -> String {
    match capability {
        BrightnessCapability::None => "none".to_owned(),
        BrightnessCapability::Independent {
            bits,
            maximum,
            scope,
        } => {
            format!("independent:{bits}bit:0-{maximum}@{}", format_scope(*scope))
        }
    }
}

fn format_persistence_capability(capability: &PersistenceCapability) -> String {
    match capability {
        PersistenceCapability::None => "none".to_owned(),
        PersistenceCapability::CurrentState {
            requirement,
            explicit_commit,
            readback,
        } => format!(
            "state({}, commit={explicit_commit}, readback={readback})",
            format_persistence_requirement(*requirement)
        ),
        PersistenceCapability::Profiles {
            requirement,
            slots,
            explicit_commit,
            readback,
        } => format!(
            "profiles({}, slots={slots}, commit={explicit_commit}, readback={readback})",
            format_persistence_requirement(*requirement)
        ),
    }
}

fn format_persistence_requirement(requirement: PersistenceRequirement) -> &'static str {
    match requirement {
        PersistenceRequirement::Optional => "optional",
        PersistenceRequirement::Required => "required",
    }
}

fn format_state_readback_capability(capability: &StateReadbackCapability) -> String {
    match capability {
        StateReadbackCapability::None => "none".to_owned(),
        StateReadbackCapability::Readable {
            facets,
            read_disturbs_output,
            notifies_external_changes,
        } => {
            let facets = facets
                .iter()
                .map(|facet| format!("{:?}:{:?}", facet.facet, facet.fidelity))
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "readable(facets=[{facets}], disturbs={read_disturbs_output}, notifies={notifies_external_changes})"
            )
        }
    }
}

fn format_scope(scope: CapabilityScope) -> &'static str {
    match scope {
        CapabilityScope::Element => "element",
        CapabilityScope::Surface => "surface",
        CapabilityScope::Device => "device",
        CapabilityScope::Controller => "controller",
    }
}

fn format_frame_update_mode(mode: FrameUpdateMode) -> &'static str {
    match mode {
        FrameUpdateMode::FullFrameOnly => "full-frame",
        FrameUpdateMode::Partial => "partial",
        FrameUpdateMode::Both => "full-or-partial",
    }
}

fn format_buffering_mode(mode: BufferingMode) -> &'static str {
    match mode {
        BufferingMode::Immediate => "immediate",
        BufferingMode::ExplicitCommit => "explicit-commit",
        BufferingMode::DoubleBuffered => "double-buffered",
    }
}

fn format_effect_parameter(parameter: &EffectParameter) -> String {
    match parameter {
        EffectParameter::Colour {
            minimum_colours,
            maximum_colours,
        } => format!("colour:{minimum_colours}-{maximum_colours}"),
        EffectParameter::Speed { range } => {
            format!("speed:{}-{} step {}", range.min, range.max, range.step)
        }
        EffectParameter::Direction { values } => format!(
            "direction:{}",
            values
                .iter()
                .map(|value| format!("{value:?}").to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join("|")
        ),
        EffectParameter::Duration { milliseconds } => {
            format!(
                "duration:{}-{}ms step {}",
                milliseconds.min, milliseconds.max, milliseconds.step
            )
        }
        EffectParameter::Brightness { bits } => format!("brightness:{bits}bit"),
        EffectParameter::Choice { options } => format!(
            "choice:{}",
            options
                .iter()
                .map(|option| terminal_safe(&option.id))
                .collect::<Vec<_>>()
                .join("|")
        ),
    }
}

fn format_colour_channel(channel: ColourChannel) -> &'static str {
    match channel {
        ColourChannel::Red => "red",
        ColourChannel::Green => "green",
        ColourChannel::Blue => "blue",
        ColourChannel::White => "white",
        ColourChannel::WarmWhite => "warm-white",
        ColourChannel::CoolWhite => "cool-white",
        ColourChannel::Amber => "amber",
        ColourChannel::Ultraviolet => "uv",
        ColourChannel::Hue => "hue",
        ColourChannel::Saturation => "saturation",
        ColourChannel::Value => "value",
        ColourChannel::Lightness => "lightness",
        ColourChannel::Temperature => "temperature",
        ColourChannel::Intensity => "intensity",
    }
}

pub(crate) fn format_surface_kind(kind: &SurfaceKind) -> String {
    match kind {
        SurfaceKind::Opaque => "opaque".to_owned(),
        SurfaceKind::Zone => "zone".to_owned(),
        SurfaceKind::Linear { length } => format!("linear length={length}"),
        SurfaceKind::Sparse2d { width, height } => format!("sparse2d {width}x{height}"),
        SurfaceKind::Matrix { rows, cols } => format!("matrix {rows}x{cols}"),
    }
}

pub(crate) fn format_element_kind(kind: &ElementKind) -> &'static str {
    match kind {
        ElementKind::Key => "key",
        ElementKind::Led => "led",
        ElementKind::Zone => "zone",
        ElementKind::Logo => "logo",
        ElementKind::RingSegment => "ring-segment",
    }
}

fn format_geometry(geometry: &ElementGeometry) -> String {
    match geometry {
        ElementGeometry::Rect { x, y, w, h } => format!("rect x={x} y={y} w={w} h={h}"),
        ElementGeometry::Point { x, y } => format!("point x={x} y={y}"),
        ElementGeometry::Linear { position } => format!("linear position={position}"),
        ElementGeometry::MatrixCell { row, col } => format!("matrix-cell row={row} col={col}"),
    }
}

pub(crate) fn format_group_kind(kind: GroupKind) -> &'static str {
    match kind {
        GroupKind::BuiltIn => "built-in",
        GroupKind::Topology => "topology",
        GroupKind::Driver => "driver",
        GroupKind::User => "user",
        GroupKind::Application => "application",
    }
}

pub(crate) fn format_group_member(member: &GroupMember) -> String {
    match member {
        GroupMember::Surface(surface) => format!("surface:{}", surface.as_str()),
        GroupMember::Element { surface, element } => {
            format!("element:{}/{}", surface.as_str(), element.as_str())
        }
        GroupMember::Group(group) => format!("group:{}", group.as_str()),
    }
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
