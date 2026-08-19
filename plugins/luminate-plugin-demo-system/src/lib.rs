// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Reference implementation of a loadable Luminate system plugin.
//!
//! [`luminate_plugin_api::luminate_export_plugin!`] takes care of the ABI
//! boundary. The code below can therefore focus on the plugin lifecycle:
//! finding devices, describing them, and applying updates. To keep the example
//! approachable, these devices are fixed and updates are recorded in memory.

use luminate_core::control::ReconciliationPolicy;
use std::env;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::process;
use std::thread;
use std::time;

use std::ffi::CStr;

use luminate_core::appearance_slot::{
    AppearanceCapability, AppearanceSlotDescriptor, AppearanceSlotId, AppearanceSlotUpdatePolicy,
    AppearanceSlotsCapability,
};
use luminate_core::capability::{
    BrightnessCapability, CapabilityScope, CapabilitySet, CctEmulation, ColourCapability,
    ColourChannel, ColourChannelCapability, EffectChoice, EffectParameter,
    HardwareEffectDescriptor, HardwareEffectId, HardwareEffectsCapability, PersistenceCapability,
    PersistenceRequirement,
};
use luminate_core::device::{DeviceCategory, device_category};
use luminate_core::effect::Effect;
use luminate_core::element::{ElementGeometry, ElementKind};
use luminate_core::group::GroupKind;
use luminate_core::surface::SurfaceKind;
use luminate_core::util::DiscreteRange;
use luminate_plugin_api::sdk::LuminatePlugin;
use luminate_plugin_api::{
    DeviceDescriptor, ElementDescriptor, GroupDescriptor, GroupMemberDescriptor, PluginBus,
    PluginError, PluginProbeHint, PluginRequestContext, PluginUpdate, PluginUpdateOperation,
    PluginVendorId, ProbeOutcome, ShadowState, SurfaceDescriptor, luminate_export_plugin,
};

const NAME: &CStr = c"luminate-plugin-demo-system";
const VERSION: &CStr = c"0.1.0";

static BUSES: &[PluginBus] = &[PluginBus::Platform];
static VENDORS: &[PluginVendorId] = &[];
static HINTS: &[PluginProbeHint] = &[];

// Real hardware plugins use vendor IDs and probe hints to limit when the daemon
// tries them. These synthetic platform devices do not need either.

struct DemoSystem;

impl LuminatePlugin for DemoSystem {
    fn new() -> Result<Self, PluginError> {
        Ok(Self)
    }

    fn probe(&self) -> ProbeOutcome {
        tracing::info!(
            vendor = "Moonbeam Systems",
            product = "GoblinGlow Fabric",
            "probe accepted"
        );
        ProbeOutcome::Ready
    }

    fn topology(&self) -> Result<Vec<DeviceDescriptor>, PluginError> {
        Ok(demo_topology())
    }

    fn apply(
        &self,
        _context: &PluginRequestContext,
        update: &PluginUpdate,
    ) -> Result<(), PluginError> {
        inject_test_fault();
        ensure_owned_device(update.target.device_id())?;
        if log_update(update) {
            Ok(())
        } else {
            Err(PluginError::Unsupported(
                "demo target rejected the update".to_owned(),
            ))
        }
    }
}

fn ensure_owned_device(device_id: &str) -> Result<(), PluginError> {
    if demo_topology().iter().any(|device| device.id == device_id) {
        Ok(())
    } else {
        Err(PluginError::InvalidTarget(format!(
            "target is not owned by the demo system plugin: {device_id}"
        )))
    }
}

fn inject_test_fault() {
    match env::var("LUMINATE_DEMO_PLUGIN_TEST_FAULT").as_deref() {
        Ok("hang") => thread::sleep(time::Duration::from_secs(30)),
        Ok("abort") => process::abort(),
        Ok("abort-once") => {
            let marker = env::var_os("LUMINATE_DEMO_PLUGIN_TEST_FAULT_MARKER");
            let Some(marker) = marker else {
                process::abort();
            };
            match OpenOptions::new().write(true).create_new(true).open(marker) {
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Ok(_) | Err(_) => process::abort(),
            }
        }
        _ => {}
    }
}

/// Builds the complete set of devices shown by this demo.
///
/// A hardware plugin would usually build this list from its probe results.
fn demo_topology() -> Vec<DeviceDescriptor> {
    vec![
        controller_device(),
        keyboard_device(),
        mouse_device(),
        power_button_device(),
        cpu_cooler_device(),
        ram_device("demo-ram-a", "RAM Stick A"),
        ram_device("demo-ram-b", "RAM Stick B"),
        power_supply_device(),
        case_lights_device(),
        status_led_device(),
        rgbw_strip_device(),
        cct_fan_device(),
        addressable_strip_device(),
        streaming_keypad_device(),
        gaming_mousepad_device(),
        monitor_device(),
    ]
}

/// A simple RGB controller with no smaller addressable parts.
///
/// Devices only need surfaces and elements when those subdivisions are useful
/// for display or control.
fn controller_device() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: "demo-controller".to_owned(),
        name: "Moonbeam GoblinGlow Fabric".to_owned(),
        vendor: Some("Moonbeam Systems".to_owned()),
        model: Some("GoblinGlow Fabric".to_owned()),
        surfaces: Vec::new(),
        groups: Vec::new(),
        capabilities: rgb_device_capabilities(),
        category: Some(DeviceCategory::new("led-controller")),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// A keyboard with a surface, addressable zones, and named groups.
///
/// Surfaces describe physical areas, elements identify parts within them, and
/// groups provide convenient logical targets. Their IDs are local to this
/// device.
fn keyboard_device() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: "demo-keyboard".to_owned(),
        name: "TypeWyrm 100".to_owned(),
        vendor: Some("Moonbeam Systems".to_owned()),
        model: Some("TypeWyrm 100".to_owned()),
        surfaces: vec![SurfaceDescriptor {
            id: "zones".to_owned(),
            name: "Zones".to_owned(),
            kind: SurfaceKind::Opaque,
            physical_tags: Vec::new(),
            elements: vec![
                zone_element("function-row", "Function Row"),
                zone_element("alpha-block", "Alpha Block"),
                zone_element("numpad", "Numpad"),
                zone_element("underglow", "Underglow"),
                key_element("g1", "G1"),
                key_element("g2", "G2"),
                key_element("g3", "G3"),
                key_element("turbo", "Turbo"),
                // These keys appear in the layout but are not independently
                // controllable. An empty capability set expresses that
                // distinction without hiding the keys altogether.
                topology_only_key_element("key-a", "A"),
                topology_only_key_element("key-b", "B"),
            ],
            capabilities: rgb_surface_capabilities(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: vec![
            GroupDescriptor {
                id: "all".to_owned(),
                name: "All".to_owned(),
                description: Some("All keyboard zones".to_owned()),
                kind: GroupKind::Topology,
                members: vec![
                    GroupMemberDescriptor::Surface("zones".to_owned()),
                    GroupMemberDescriptor::Group("reactive-zones".to_owned()),
                ],
                capabilities: rgb_group_capabilities(),
                notes: Vec::new(),
                warnings: Vec::new(),
            },
            GroupDescriptor {
                id: "reactive-zones".to_owned(),
                name: "Reactive Zones".to_owned(),
                description: Some("Zones that react to typing".to_owned()),
                kind: GroupKind::Topology,
                members: vec![
                    GroupMemberDescriptor::Element {
                        surface: "zones".to_owned(),
                        element: "alpha-block".to_owned(),
                    },
                    GroupMemberDescriptor::Element {
                        surface: "zones".to_owned(),
                        element: "numpad".to_owned(),
                    },
                ],
                capabilities: rgb_group_capabilities(),
                notes: Vec::new(),
                warnings: Vec::new(),
            },
            GroupDescriptor {
                id: "gamer-keys".to_owned(),
                name: "Gamer Keys".to_owned(),
                description: Some("Dedicated macro and gamer keys".to_owned()),
                kind: GroupKind::Topology,
                members: vec![
                    GroupMemberDescriptor::Element {
                        surface: "zones".to_owned(),
                        element: "g1".to_owned(),
                    },
                    GroupMemberDescriptor::Element {
                        surface: "zones".to_owned(),
                        element: "g2".to_owned(),
                    },
                    GroupMemberDescriptor::Element {
                        surface: "zones".to_owned(),
                        element: "g3".to_owned(),
                    },
                    GroupMemberDescriptor::Element {
                        surface: "zones".to_owned(),
                        element: "turbo".to_owned(),
                    },
                ],
                capabilities: rgb_group_capabilities(),
                notes: Vec::new(),
                warnings: Vec::new(),
            },
        ],
        capabilities: rgb_device_capabilities(),
        category: Some(DeviceCategory::new(device_category::KEYBOARD)),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// A mouse whose lighting areas offer different controls.
///
/// Putting capabilities on each element helps clients show only the controls
/// that work for that part of the mouse.
fn mouse_device() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: "demo-mouse".to_owned(),
        name: "Clickwyrm Pro".to_owned(),
        vendor: Some("Moonbeam Systems".to_owned()),
        model: Some("Clickwyrm Pro".to_owned()),
        surfaces: vec![SurfaceDescriptor {
            id: "lighting".to_owned(),
            name: "Lighting".to_owned(),
            kind: SurfaceKind::Opaque,
            physical_tags: Vec::new(),
            elements: vec![
                ElementDescriptor {
                    id: "logo".to_owned(),
                    name: Some("Logo".to_owned()),
                    kind: ElementKind::Logo,
                    geometry: None,
                    physical_tags: Vec::new(),
                    capabilities: monochrome_two_level_capabilities(CapabilityScope::Element),
                    notes: Vec::new(),
                    warnings: Vec::new(),
                },
                ElementDescriptor {
                    id: "wheel".to_owned(),
                    name: Some("Wheel".to_owned()),
                    kind: ElementKind::Led,
                    geometry: None,
                    physical_tags: Vec::new(),
                    capabilities: rgb_element_capabilities(),
                    notes: Vec::new(),
                    warnings: Vec::new(),
                },
                ElementDescriptor {
                    id: "side-strip".to_owned(),
                    name: Some("Side Strip".to_owned()),
                    kind: ElementKind::Zone,
                    geometry: None,
                    physical_tags: Vec::new(),
                    capabilities: rgb_element_capabilities(),
                    notes: Vec::new(),
                    warnings: Vec::new(),
                },
            ],
            capabilities: rgb_surface_capabilities(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: Vec::new(),
        capabilities: rgb_device_capabilities(),
        category: Some(DeviceCategory::new(device_category::MOUSE)),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// A fixed-colour ring that is visible in the topology but not adjustable.
fn power_button_device() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: "demo-power-button".to_owned(),
        name: "Power Button".to_owned(),
        vendor: Some("Moonbeam Systems".to_owned()),
        model: Some("WakeSigil".to_owned()),
        surfaces: vec![SurfaceDescriptor {
            id: "button".to_owned(),
            name: "Button".to_owned(),
            kind: SurfaceKind::Zone,
            physical_tags: Vec::new(),
            elements: vec![ElementDescriptor {
                id: "ring".to_owned(),
                name: Some("Ring".to_owned()),
                kind: ElementKind::RingSegment,
                geometry: None,
                physical_tags: Vec::new(),
                capabilities: fixed_colour_capabilities(),
                notes: Vec::new(),
                warnings: Vec::new(),
            }],
            capabilities: fixed_colour_capabilities(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: Vec::new(),
        capabilities: fixed_colour_capabilities(),
        category: Some(DeviceCategory::new(device_category::BUTTON)),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// A cooler with one continuous RGB ring.
///
/// `SurfaceKind::Linear` describes its shape; it does not imply that each LED
/// can be controlled separately.
fn cpu_cooler_device() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: "demo-cpu-cooler".to_owned(),
        name: "Cyclone 120".to_owned(),
        vendor: Some("Moonbeam Systems".to_owned()),
        model: Some("Cyclone 120".to_owned()),
        surfaces: vec![SurfaceDescriptor {
            id: "ring".to_owned(),
            name: "Ring".to_owned(),
            kind: SurfaceKind::Linear { length: 12.0 },
            physical_tags: Vec::new(),
            elements: Vec::new(),
            capabilities: rgb_surface_capabilities(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: vec![GroupDescriptor {
            id: "cooling".to_owned(),
            name: "Cooling".to_owned(),
            description: Some("Cooling hardware".to_owned()),
            kind: GroupKind::Topology,
            members: vec![GroupMemberDescriptor::Surface("ring".to_owned())],
            capabilities: rgb_group_capabilities(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        capabilities: rgb_device_capabilities(),
        category: Some(DeviceCategory::new(device_category::COOLER)),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// Builds one of the otherwise identical RAM-stick devices.
///
/// Each physical unit still needs its own stable ID.
fn ram_device(id: &str, name: &str) -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: id.to_owned(),
        name: name.to_owned(),
        vendor: Some("Moonbeam Systems".to_owned()),
        model: Some("MemoryMirth".to_owned()),
        surfaces: vec![SurfaceDescriptor {
            id: "bar".to_owned(),
            name: "Bar".to_owned(),
            kind: SurfaceKind::Linear { length: 10.0 },
            physical_tags: Vec::new(),
            elements: Vec::new(),
            capabilities: rgb_surface_capabilities(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: vec![GroupDescriptor {
            id: "memory".to_owned(),
            name: "Memory".to_owned(),
            description: Some("Memory lighting".to_owned()),
            kind: GroupKind::Topology,
            members: vec![GroupMemberDescriptor::Surface("bar".to_owned())],
            capabilities: rgb_group_capabilities(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        capabilities: rgb_device_capabilities(),
        category: Some(DeviceCategory::new(device_category::RAM)),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// A power-supply badge with both an informational note and a warning.
fn power_supply_device() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: "demo-power-supply".to_owned(),
        name: "Power Supply".to_owned(),
        vendor: Some("Moonbeam Systems".to_owned()),
        model: Some("Sparkheap 850".to_owned()),
        surfaces: vec![SurfaceDescriptor {
            id: "logo".to_owned(),
            name: "Logo".to_owned(),
            kind: SurfaceKind::Zone,
            physical_tags: Vec::new(),
            elements: vec![ElementDescriptor {
                id: "badge".to_owned(),
                name: Some("Badge".to_owned()),
                kind: ElementKind::Logo,
                geometry: None,
                physical_tags: Vec::new(),
                capabilities: monochrome_two_level_capabilities(CapabilityScope::Element),
                notes: Vec::new(),
                warnings: Vec::new(),
            }],
            capabilities: monochrome_two_level_capabilities(CapabilityScope::Surface),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: Vec::new(),
        capabilities: monochrome_two_level_capabilities(CapabilityScope::Device),
        category: Some(DeviceCategory::new(device_category::POWER_SUPPLY)),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: vec!["Badge brightness has two discrete levels only (off/on).".to_owned()],
        warnings: vec![
            "Badge LED is prone to burn-in under a static colour for extended periods.".to_owned(),
        ],
    }
}

/// A case strip controlled as one unit, so it has no individual LED elements.
fn case_lights_device() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: "demo-case-lights".to_owned(),
        name: "Case Lights".to_owned(),
        vendor: Some("Moonbeam Systems".to_owned()),
        model: Some("ChromaBurrow 24".to_owned()),
        surfaces: vec![SurfaceDescriptor {
            id: "strip".to_owned(),
            name: "Strip".to_owned(),
            kind: SurfaceKind::Linear { length: 24.0 },
            physical_tags: Vec::new(),
            elements: Vec::new(),
            capabilities: rgb_surface_capabilities(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: vec![GroupDescriptor {
            id: "chassis".to_owned(),
            name: "Chassis".to_owned(),
            description: Some("Chassis lighting".to_owned()),
            kind: GroupKind::Topology,
            members: vec![GroupMemberDescriptor::Surface("strip".to_owned())],
            capabilities: rgb_group_capabilities(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        capabilities: rgb_device_capabilities(),
        category: Some(DeviceCategory::new(device_category::CASE_LIGHT)),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// A status light controlled entirely through firmware effects.
fn status_led_device() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: "demo-status-led".to_owned(),
        name: "Status LED".to_owned(),
        vendor: Some("Moonbeam Systems".to_owned()),
        model: Some("Pinlight".to_owned()),
        surfaces: Vec::new(),
        groups: Vec::new(),
        capabilities: no_colour_effects_capabilities(),
        category: Some(DeviceCategory::new(device_category::LED_STRIP)),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// A strip with separate red, green, blue, and white emitters.
///
/// Describing the real channels lets clients use the dedicated white emitter
/// instead of approximating white with RGB.
fn rgbw_strip_device() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: "demo-rgbw-strip".to_owned(),
        name: "RGBW Strip".to_owned(),
        vendor: Some("Moonbeam Systems".to_owned()),
        model: Some("Fourglow 16".to_owned()),
        surfaces: vec![SurfaceDescriptor {
            id: "strip".to_owned(),
            name: "Strip".to_owned(),
            kind: SurfaceKind::Linear { length: 16.0 },
            physical_tags: Vec::new(),
            elements: Vec::new(),
            capabilities: rgbw_capabilities(CapabilityScope::Surface),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: Vec::new(),
        capabilities: rgbw_capabilities(CapabilityScope::Device),
        category: Some(DeviceCategory::new(device_category::LED_STRIP)),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// A tunable-white fan with independent temperature and brightness controls.
fn cct_fan_device() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: "demo-cct-fan".to_owned(),
        name: "CCT Fan".to_owned(),
        vendor: Some("Moonbeam Systems".to_owned()),
        model: Some("Driftvane 140".to_owned()),
        surfaces: Vec::new(),
        groups: Vec::new(),
        capabilities: cct_capabilities(),
        category: Some(DeviceCategory::new(device_category::FAN)),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// An addressable strip with one element for each physical LED.
#[allow(
    clippy::cast_precision_loss,
    reason = "The demo sampler intentionally maps integer coordinates into normalized floating-point colour space."
)]
fn addressable_strip_device() -> DeviceDescriptor {
    const LED_COUNT: usize = 16;
    let elements = (0..LED_COUNT)
        .map(|index| {
            let position = index as f32 / (LED_COUNT - 1) as f32;
            ElementDescriptor {
                id: format!("led-{index}"),
                name: Some(format!("LED {index}")),
                kind: ElementKind::Led,
                geometry: Some(ElementGeometry::Linear { position }),
                physical_tags: Vec::new(),
                capabilities: rgb_element_capabilities(),
                notes: Vec::new(),
                warnings: Vec::new(),
            }
        })
        .collect();

    DeviceDescriptor {
        claims: Vec::new(),
        id: "demo-addressable-strip".to_owned(),
        name: "Addressable Strip".to_owned(),
        vendor: Some("Moonbeam Systems".to_owned()),
        model: Some("Pixelvine 16".to_owned()),
        surfaces: vec![SurfaceDescriptor {
            id: "strip".to_owned(),
            name: "Strip".to_owned(),
            kind: SurfaceKind::Linear {
                length: LED_COUNT as f32,
            },
            physical_tags: vec!["shape:flexible-strip".to_owned()],
            elements,
            capabilities: addressable_strip_capabilities(),
            notes: Vec::new(),
            warnings: vec![
                "Hardware accepts updates no faster than roughly 60 times per second; \
                 rapid successive mutations may be coalesced or dropped."
                    .to_owned(),
            ],
        }],
        groups: Vec::new(),
        capabilities: rgb_device_capabilities(),
        category: Some(DeviceCategory::new(device_category::LED_STRIP)),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// A regular matrix of individually addressable keypad keys.
///
/// Each cell carries its row and column, while its capabilities describe what
/// can be changed there.
fn streaming_keypad_device() -> DeviceDescriptor {
    const ROWS: u16 = 3;
    const COLS: u16 = 5;

    let elements = (0..ROWS)
        .flat_map(|row| (0..COLS).map(move |col| (row, col)))
        .map(|(row, col)| ElementDescriptor {
            id: format!("key-{row}-{col}"),
            name: Some(format!("Key {row}.{col}")),
            kind: ElementKind::Key,
            geometry: Some(ElementGeometry::MatrixCell { row, col }),
            physical_tags: Vec::new(),
            capabilities: hsv_capabilities(CapabilityScope::Element),
            notes: Vec::new(),
            warnings: Vec::new(),
        })
        .collect();

    DeviceDescriptor {
        claims: Vec::new(),
        id: "demo-streaming-keypad".to_owned(),
        name: "Streaming Keypad".to_owned(),
        vendor: Some("Moonbeam Systems".to_owned()),
        model: Some("HighElf Streamkey".to_owned()),
        surfaces: vec![SurfaceDescriptor {
            id: "grid".to_owned(),
            name: "Grid".to_owned(),
            kind: SurfaceKind::Matrix {
                rows: ROWS,
                cols: COLS,
            },
            physical_tags: Vec::new(),
            elements,
            capabilities: streaming_keypad_capabilities(CapabilityScope::Surface),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: Vec::new(),
        capabilities: streaming_keypad_capabilities(CapabilityScope::Device),
        category: Some(DeviceCategory::new("keypad")),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// Builds HSV and brightness controls for the requested target scope.
fn hsv_capabilities(scope: CapabilityScope) -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::Hsv {
            hue_bits: 16,
            saturation_bits: 8,
            value_bits: 8,
        }],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: u8::MAX.into(),
            scope,
        },
        ..CapabilitySet::default()
    }
}

/// Adds the keypad's firmware effect to its direct colour controls.
fn streaming_keypad_capabilities(scope: CapabilityScope) -> CapabilitySet {
    CapabilitySet {
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![rainbow_wave_effect(), scene_show_effect()],
            scope,
            concurrent_with_streaming: true,
        }),
        ..hsv_capabilities(scope)
    }
}

/// Describes a vendor scene with a fixed choice list and speed range.
fn scene_show_effect() -> HardwareEffectDescriptor {
    HardwareEffectDescriptor {
        id: HardwareEffectId::new("demo-scene-show"),
        name: "Scene Show".to_owned(),
        parameters: vec![
            EffectParameter::Choice {
                options: vec![
                    EffectChoice {
                        id: "aurora".to_owned(),
                        name: "Aurora".to_owned(),
                    },
                    EffectChoice {
                        id: "campfire".to_owned(),
                        name: "Campfire".to_owned(),
                    },
                    EffectChoice {
                        id: "nebula".to_owned(),
                        name: "Nebula".to_owned(),
                    },
                ],
            },
            EffectParameter::Speed {
                range: DiscreteRange::new(1, 10, 1),
            },
        ],
    }
}

/// Describes a firmware rainbow effect and its adjustable speed.
///
/// Declaring the range lets the daemon reject bad values before they reach the
/// device.
fn rainbow_wave_effect() -> HardwareEffectDescriptor {
    HardwareEffectDescriptor {
        id: HardwareEffectId::new("rainbow"),
        name: "Rainbow Wave".to_owned(),
        parameters: vec![EffectParameter::Duration {
            milliseconds: DiscreteRange::new(100, 10_000, 100),
        }],
    }
}

/// A mousepad whose LEDs form an irregular two-dimensional outline.
///
/// `Sparse2d` is useful when positions do not fit a regular row-and-column grid.
fn gaming_mousepad_device() -> DeviceDescriptor {
    const POSITIONS: &[(f32, f32)] = &[
        (0.03, 0.15),
        (0.02, 0.55),
        (0.08, 0.92),
        (0.35, 0.97),
        (0.68, 0.94),
        (0.97, 0.85),
        (0.98, 0.45),
        (0.90, 0.08),
        (0.55, 0.03),
        (0.22, 0.05),
    ];

    let elements = POSITIONS
        .iter()
        .enumerate()
        .map(|(index, &(x, y))| ElementDescriptor {
            id: format!("led-{index}"),
            name: Some(format!("Edge LED {index}")),
            kind: ElementKind::Led,
            geometry: Some(ElementGeometry::Point { x, y }),
            physical_tags: Vec::new(),
            capabilities: hsl_capabilities(CapabilityScope::Element),
            notes: Vec::new(),
            warnings: Vec::new(),
        })
        .collect();

    DeviceDescriptor {
        claims: Vec::new(),
        id: "demo-mousepad".to_owned(),
        name: "Gaming Mousepad".to_owned(),
        vendor: Some("Moonbeam Systems".to_owned()),
        model: Some("Balrog Battlemat".to_owned()),
        surfaces: vec![SurfaceDescriptor {
            id: "perimeter".to_owned(),
            name: "Perimeter".to_owned(),
            kind: SurfaceKind::Sparse2d {
                width: 36.0,
                height: 12.0,
            },
            physical_tags: Vec::new(),
            elements,
            capabilities: hsl_capabilities(CapabilityScope::Surface),
            notes: Vec::new(),
            warnings: Vec::new(),
        }],
        groups: Vec::new(),
        capabilities: hsl_capabilities(CapabilityScope::Device),
        category: Some(DeviceCategory::new("mousepad")),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// Builds HSL and brightness controls for the requested target scope.
fn hsl_capabilities(scope: CapabilityScope) -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::Hsl {
            hue_bits: 16,
            saturation_bits: 8,
            lightness_bits: 8,
        }],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: u8::MAX.into(),
            scope,
        },
        ..CapabilitySet::default()
    }
}

/// A monitor with two lighting areas that can be targeted independently.
fn monitor_device() -> DeviceDescriptor {
    DeviceDescriptor {
        claims: Vec::new(),
        id: "demo-monitor".to_owned(),
        name: "Monitor".to_owned(),
        vendor: Some("Moonbeam Systems".to_owned()),
        model: Some("Bifröst 27".to_owned()),
        surfaces: vec![
            SurfaceDescriptor {
                id: "backlight".to_owned(),
                name: "Backlight".to_owned(),
                kind: SurfaceKind::Linear { length: 27.0 },
                physical_tags: Vec::new(),
                elements: Vec::new(),
                capabilities: monitor_backlight_capabilities(),
                notes: Vec::new(),
                warnings: Vec::new(),
            },
            SurfaceDescriptor {
                id: "logo".to_owned(),
                name: "Logo".to_owned(),
                kind: SurfaceKind::Zone,
                physical_tags: Vec::new(),
                elements: vec![ElementDescriptor {
                    id: "badge".to_owned(),
                    name: Some("Badge".to_owned()),
                    kind: ElementKind::Logo,
                    geometry: Some(ElementGeometry::Rect {
                        x: 0.42,
                        y: 0.90,
                        w: 0.16,
                        h: 0.06,
                    }),
                    physical_tags: Vec::new(),
                    capabilities: monochrome_two_level_capabilities(CapabilityScope::Element),
                    notes: Vec::new(),
                    warnings: Vec::new(),
                }],
                capabilities: monochrome_two_level_capabilities(CapabilityScope::Surface),
                notes: Vec::new(),
                warnings: Vec::new(),
            },
            SurfaceDescriptor {
                id: "power-indicator".to_owned(),
                name: "Power indicator".to_owned(),
                kind: SurfaceKind::Zone,
                physical_tags: Vec::new(),
                elements: Vec::new(),
                capabilities: stored_power_indicator_capabilities(),
                notes: vec![
                    "Firmware selects the active appearance from the monitor's power state."
                        .to_owned(),
                ],
                warnings: Vec::new(),
            },
        ],
        groups: Vec::new(),
        capabilities: rgb_device_capabilities(),
        category: Some(DeviceCategory::new(device_category::MONITOR)),
        physical_tags: Vec::new(),
        host_attached: true,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// Two coupled appearances stored by the monitor firmware.
fn stored_power_indicator_capabilities() -> CapabilitySet {
    let slot = |id: &str, name: &str| AppearanceSlotDescriptor {
        id: AppearanceSlotId::new(id),
        name: name.to_owned(),
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
        notes: Vec::new(),
        warnings: vec!["Updates write through to persistent firmware storage.".to_owned()],
    };

    CapabilitySet {
        appearance_slots: Some(AppearanceSlotsCapability {
            slots: vec![slot("active", "Active"), slot("standby", "Standby")],
            update_policy: AppearanceSlotUpdatePolicy::PartialIfKnown,
        }),
        ..CapabilitySet::default()
    }
}

/// Five additive emitter channels for the monitor backlight.
fn monitor_backlight_capabilities() -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::Additive(vec![
            ColourChannelCapability::new(ColourChannel::Red, 8),
            ColourChannelCapability::new(ColourChannel::Green, 8),
            ColourChannelCapability::new(ColourChannel::Blue, 8),
            ColourChannelCapability::new(ColourChannel::Amber, 8),
            ColourChannelCapability::new(ColourChannel::Ultraviolet, 8),
        ])],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: u8::MAX.into(),
            scope: CapabilityScope::Surface,
        },
        ..CapabilitySet::default()
    }
}

/// Builds a named keyboard zone with its own RGB controls.
fn zone_element(id: &str, name: &str) -> ElementDescriptor {
    ElementDescriptor {
        id: id.to_owned(),
        name: Some(name.to_owned()),
        kind: ElementKind::Zone,
        geometry: None,
        physical_tags: Vec::new(),
        capabilities: rgb_element_capabilities(),
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// Builds a key that can be targeted and coloured independently.
fn key_element(id: &str, name: &str) -> ElementDescriptor {
    ElementDescriptor {
        id: id.to_owned(),
        name: Some(name.to_owned()),
        kind: ElementKind::Key,
        geometry: None,
        physical_tags: Vec::new(),
        capabilities: rgb_element_capabilities(),
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// Builds a key that appears in the layout but cannot be targeted directly.
fn topology_only_key_element(id: &str, name: &str) -> ElementDescriptor {
    ElementDescriptor {
        id: id.to_owned(),
        name: Some(name.to_owned()),
        kind: ElementKind::Key,
        geometry: None,
        physical_tags: Vec::new(),
        capabilities: CapabilitySet::default(),
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// Ordinary 8-bit RGB and brightness controls for a whole device.
fn rgb_device_capabilities() -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: u8::MAX.into(),
            scope: CapabilityScope::Device,
        },
        emission: true,
        ..CapabilitySet::default()
    }
}

/// Ordinary 8-bit RGB and brightness controls for one surface.
fn rgb_surface_capabilities() -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: u8::MAX.into(),
            scope: CapabilityScope::Surface,
        },
        ..CapabilitySet::default()
    }
}

/// RGB controls exposed through a logical group target.
fn rgb_group_capabilities() -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: u8::MAX.into(),
            scope: CapabilityScope::Device,
        },
        ..CapabilitySet::default()
    }
}

/// Ordinary 8-bit RGB and brightness controls for one element.
fn rgb_element_capabilities() -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: u8::MAX.into(),
            scope: CapabilityScope::Element,
        },
        ..CapabilitySet::default()
    }
}

/// One fixed monochrome channel with no brightness control.
fn fixed_colour_capabilities() -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::monochrome(1)],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::None,
        ..CapabilitySet::default()
    }
}

/// A monochrome light with just two brightness states: off and on.
fn monochrome_two_level_capabilities(scope: CapabilityScope) -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::monochrome(1)],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 1,
            maximum: 1,
            scope,
        },
        ..CapabilitySet::default()
    }
}

/// Firmware “on” and “off” effects with no direct colour controls.
fn no_colour_effects_capabilities() -> CapabilitySet {
    CapabilitySet {
        colour: vec![],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::None,
        hardware_effects: Some(HardwareEffectsCapability {
            effects: vec![
                HardwareEffectDescriptor {
                    id: HardwareEffectId::new("on"),
                    name: "On".to_owned(),
                    parameters: Vec::new(),
                },
                HardwareEffectDescriptor {
                    id: HardwareEffectId::new("off"),
                    name: "Off".to_owned(),
                    parameters: Vec::new(),
                },
            ],
            scope: CapabilityScope::Device,
            concurrent_with_streaming: false,
        }),
        ..CapabilitySet::default()
    }
}

/// An additive RGBW colour model with target-scoped brightness.
fn rgbw_capabilities(scope: CapabilityScope) -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::Additive(vec![
            ColourChannelCapability::new(ColourChannel::Red, 8),
            ColourChannelCapability::new(ColourChannel::Green, 8),
            ColourChannelCapability::new(ColourChannel::Blue, 8),
            ColourChannelCapability::new(ColourChannel::White, 8),
        ])],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: u8::MAX.into(),
            scope,
        },
        ..CapabilitySet::default()
    }
}

/// A 16-bit colour-temperature channel with brightness.
fn cct_capabilities() -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::Cct { bits: 16 }],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: u8::MAX.into(),
            scope: CapabilityScope::Device,
        },
        ..CapabilitySet::default()
    }
}

/// Whole-surface controls for the addressable strip.
fn addressable_strip_capabilities() -> CapabilitySet {
    CapabilitySet {
        colour: vec![ColourCapability::rgb8()],
        cct_emulation: CctEmulation::Auto,
        brightness: BrightnessCapability::Independent {
            bits: 8,
            maximum: u8::MAX.into(),
            scope: CapabilityScope::Surface,
        },
        ..CapabilitySet::default()
    }
}

/// Records updates so tests and diagnostics can inspect what the demo received.
///
/// This is only a demo shadow; it is not daemon state or durable device state.
fn shadow_state() -> &'static ShadowState<String, String> {
    static SHADOW_STATE: ShadowState<String, String> = ShadowState::new();
    &SHADOW_STATE
}

fn record_shadow_state(target: &str, state: String) {
    shadow_state()
        .lock()
        .expect("lock poisoned")
        .insert(target.to_owned(), state);
}

fn log_update(update: &PluginUpdate) -> bool {
    let target = update.target.to_string();

    match &update.operation {
        PluginUpdateOperation::SetBrightness { value } => {
            tracing::info!(target, value, "brightness updated");
            record_shadow_state(&target, format!("brightness={value}"));
            true
        }
        PluginUpdateOperation::SetEffect {
            effect: Effect::Static { colour },
        } => {
            if update.target.device_id() == "demo-cpu-cooler" && colour.is_dark() {
                // Models firmware that reserves pure black as a sentinel.
                tracing::warn!(
                    target,
                    "colour update rejected: pure black is reserved as a sentinel on this device, use clear instead"
                );
                return false;
            }
            tracing::info!(target, ?colour, "colour updated");
            record_shadow_state(&target, format!("{colour:?}"));
            true
        }
        PluginUpdateOperation::SetEffect { effect } => {
            tracing::info!(target, ?effect, "lighting effect updated");
            record_shadow_state(&target, format!("{effect:?}"));
            true
        }
        PluginUpdateOperation::Clear => {
            tracing::info!(target, "lighting cleared");
            shadow_state()
                .lock()
                .expect("lock poisoned")
                .remove(&target);
            true
        }
        PluginUpdateOperation::SaveCurrent => {
            // None of this plugin's devices advertise persistence.
            tracing::warn!(
                target,
                "save-current rejected: demo plugin does not model firmware persistence for this target"
            );
            false
        }
        PluginUpdateOperation::SetAppearanceSlots { .. } => {
            tracing::warn!(
                target,
                "appearance-slot update rejected: target has no slots"
            );
            false
        }
    }
}

luminate_export_plugin! {
    plugin: DemoSystem,
    name: NAME,
    version: VERSION,
    priority: 200,
    recommended_reconciliation: Some(ReconciliationPolicy::Restore),
    buses: BUSES,
    vendors: VENDORS,
    probe_hints: HINTS,
    start: none,
    rescan: none,
    batch: default,
    read_state: none,
    frame_upload: none,
    shm_frame: none,
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
