// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Bounded CLIP v2 light-state readback.

use std::collections::HashMap;

use luminate_core::colour::Colour;
use luminate_core::state::{AppearanceState, EmissionState, FacetValue, StateFacetKind};
use luminate_plugin_api::{
    PluginFacetObservation, PluginReadError, PluginReadRequest, PluginStateSnapshot, PluginTarget,
};

use crate::api::{self, Light, Transport};
use crate::colour::{mirek_to_kelvin, xy_to_rgb};
use crate::topology::HueDevice;

const SURFACE_ID: &str = "light";

pub(crate) fn read(
    transport: &impl Transport,
    devices: &[HueDevice],
    request: &PluginReadRequest,
) -> PluginStateSnapshot {
    let devices = devices
        .iter()
        .map(|device| (device.id.as_str(), device))
        .collect::<HashMap<_, _>>();
    let mut lights = HashMap::<String, Result<Light, String>>::new();
    let mut snapshot = PluginStateSnapshot::default();

    for requested in &request.targets {
        let device_id = requested.target.device_id();
        let Some(device) = devices.get(device_id) else {
            push_error(&mut snapshot, &requested.target, "Hue light is unavailable");
            continue;
        };
        if let Err(diagnostic) = validate_target(device, &requested.target) {
            push_error(&mut snapshot, &requested.target, diagnostic);
            continue;
        }
        let light = lights.entry(device.id.clone()).or_insert_with(|| {
            api::get_light(transport, &device.endpoint, &device.light.id)
                .map_err(|error| format!("Hue light readback failed: {error}"))
        });
        match light {
            Ok(light) => match observations(light, &requested.target, &requested.facets) {
                Ok(observations) => snapshot.observations.extend(observations),
                Err(diagnostic) => push_error(&mut snapshot, &requested.target, diagnostic),
            },
            Err(diagnostic) => push_error(&mut snapshot, &requested.target, diagnostic.clone()),
        }
    }
    snapshot
}

fn validate_target<'a>(device: &HueDevice, target: &'a PluginTarget) -> Result<(), &'a str> {
    match target {
        PluginTarget::Device { device: target } if target == &device.id => Ok(()),
        PluginTarget::Surface {
            device: target,
            surface,
        } if target == &device.id && surface == SURFACE_ID => Ok(()),
        PluginTarget::Group { .. } => Err("Hue groups are not physical readback scopes"),
        PluginTarget::Device { .. }
        | PluginTarget::Surface { .. }
        | PluginTarget::Element { .. } => Err("target is not this Hue light"),
    }
}

fn observations(
    light: &Light,
    target: &PluginTarget,
    facets: &[StateFacetKind],
) -> Result<Vec<PluginFacetObservation>, String> {
    let mut observations = Vec::new();
    for facet in facets {
        let value = match facet {
            StateFacetKind::Appearance => appearance(light)?.map(FacetValue::Appearance),
            StateFacetKind::Brightness => light
                .dimming
                .as_ref()
                .map(|dimming| FacetValue::Brightness(brightness_value(dimming.brightness))),
            StateFacetKind::Emission => Some(FacetValue::Emission(if light.on.on {
                EmissionState::Emitting
            } else {
                EmissionState::Dark
            })),
            StateFacetKind::PhysicalPower
            | StateFacetKind::EffectiveAppearance
            | StateFacetKind::AppearanceSlots => None,
        };
        if let Some(value) = value {
            observations.push(PluginFacetObservation {
                target: target.clone(),
                value,
            });
        }
    }
    Ok(observations)
}

fn appearance(light: &Light) -> Result<Option<AppearanceState>, String> {
    if let Some(temperature) = &light.color_temperature
        && temperature.mirek_valid
    {
        let mirek = temperature
            .mirek
            .ok_or_else(|| "Hue marked a missing colour temperature valid".to_owned())?;
        let kelvin = mirek_to_kelvin(mirek).map_err(|error| error.to_string())?;
        return Ok(Some(AppearanceState::Static(Colour::cct(kelvin))));
    }
    light
        .color
        .as_ref()
        .map(|colour| {
            xy_to_rgb(&colour.xy, colour.gamut.as_ref())
                .map(Colour::rgb)
                .map(AppearanceState::Static)
                .map_err(|error| error.to_string())
        })
        .transpose()
}

fn brightness_value(brightness: f64) -> u32 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "validated Hue brightness is rounded and clamped to 0..=100"
    )]
    let value = brightness.round().clamp(0.0, 100.0) as u32;
    value
}

fn push_error(
    snapshot: &mut PluginStateSnapshot,
    target: &PluginTarget,
    diagnostic: impl Into<String>,
) {
    snapshot.errors.push(PluginReadError {
        target: target.clone(),
        diagnostic: diagnostic.into(),
    });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use luminate_core::state::FacetValue;
    use serde_json::json;

    use super::*;
    use crate::api::{Device, Light};
    use crate::configuration::{BridgeId, Endpoint};
    use crate::http::HttpError;

    struct FakeTransport {
        calls: RefCell<Vec<String>>,
        response: RefCell<Result<Vec<u8>, HttpError>>,
    }

    impl Transport for FakeTransport {
        fn get_json(&self, _endpoint: &Endpoint, path: &str) -> Result<Vec<u8>, HttpError> {
            self.calls.borrow_mut().push(path.to_owned());
            self.response
                .borrow()
                .as_ref()
                .map(Clone::clone)
                .map_err(|error| HttpError::Protocol(error.to_string()))
        }

        fn put_json(
            &self,
            _endpoint: &Endpoint,
            _path: &str,
            _body: &[u8],
        ) -> Result<Vec<u8>, HttpError> {
            Err(HttpError::Protocol("unexpected test PUT".to_owned()))
        }
    }

    fn hue_device() -> HueDevice {
        let device = serde_json::from_value::<Device>(json!({
            "id": "22222222-2222-4222-8222-222222222222",
            "type": "device",
            "product_data": {
                "model_id": "test-model",
                "manufacturer_name": "Test Vendor",
                "product_name": "Test Lamp",
                "certified": true,
                "software_version": "1.2.3"
            },
            "metadata": { "name": "Test light" },
            "services": [{
                "rid": "33333333-3333-4333-8333-333333333333",
                "rtype": "light"
            }]
        }))
        .expect("device");
        let light = light(&json!({
            "on": { "on": true },
            "dimming": { "brightness": 42.5 },
            "color_temperature": {
                "mirek": 250,
                "mirek_valid": true,
                "mirek_schema": { "mirek_minimum": 153, "mirek_maximum": 500 }
            },
            "color": { "xy": { "x": 0.3, "y": 0.4 } }
        }));
        HueDevice {
            id: "philips-hue:001788fffe123456:33333333-3333-4333-8333-333333333333".to_owned(),
            bridge_id: BridgeId::parse("001788fffe123456").expect("bridge ID"),
            endpoint: Endpoint::from_ip("192.0.2.1".parse().expect("IP"), 443).expect("endpoint"),
            device,
            light,
        }
    }

    fn light(state: &serde_json::Value) -> Light {
        serde_json::from_value(light_value(state)).expect("light")
    }

    fn light_value(state: &serde_json::Value) -> serde_json::Value {
        let mut value = json!({
            "id": "33333333-3333-4333-8333-333333333333",
            "owner": {
                "rid": "22222222-2222-4222-8222-222222222222",
                "rtype": "device"
            },
            "type": "light"
        });
        value
            .as_object_mut()
            .expect("light object")
            .extend(state.as_object().expect("state object").clone());
        value
    }

    fn response(state: &serde_json::Value) -> Vec<u8> {
        let light = light_value(state);
        serde_json::to_vec(&json!({ "errors": [], "data": [light] })).expect("response")
    }

    fn all_facets() -> Vec<StateFacetKind> {
        vec![
            StateFacetKind::Appearance,
            StateFacetKind::Brightness,
            StateFacetKind::Emission,
        ]
    }

    #[test]
    fn reads_each_light_once_and_preserves_temperature_mode() {
        let device = hue_device();
        let transport = FakeTransport {
            calls: RefCell::new(Vec::new()),
            response: RefCell::new(Ok(response(&json!({
                "on": { "on": true },
                "dimming": { "brightness": 42.5 },
                "color_temperature": {
                    "mirek": 250,
                    "mirek_valid": true,
                    "mirek_schema": { "mirek_minimum": 153, "mirek_maximum": 500 }
                },
                "color": { "xy": { "x": 0.3, "y": 0.4 } }
            })))),
        };
        let request = PluginReadRequest {
            targets: vec![
                luminate_plugin_api::PluginReadTarget {
                    target: PluginTarget::Device {
                        device: device.id.clone(),
                    },
                    facets: all_facets(),
                },
                luminate_plugin_api::PluginReadTarget {
                    target: PluginTarget::Surface {
                        device: device.id.clone(),
                        surface: SURFACE_ID.to_owned(),
                    },
                    facets: vec![StateFacetKind::Emission],
                },
            ],
        };

        let snapshot = read(&transport, &[device], &request);

        assert_eq!(transport.calls.borrow().len(), 1);
        assert!(snapshot.errors.is_empty());
        assert_eq!(snapshot.observations.len(), 4);
        assert!(snapshot.observations.iter().any(|observation| matches!(
            observation.value,
            FacetValue::Appearance(AppearanceState::Static(Colour::Cct { kelvin: 4_000 }))
        )));
        assert!(
            snapshot
                .observations
                .iter()
                .any(|observation| { observation.value == FacetValue::Brightness(43) })
        );
    }

    #[test]
    fn uses_xy_when_temperature_is_invalid_and_reports_target_scoped_failures() {
        let device = hue_device();
        let transport = FakeTransport {
            calls: RefCell::new(Vec::new()),
            response: RefCell::new(Ok(response(&json!({
                "on": { "on": false },
                "color_temperature": {
                    "mirek": null,
                    "mirek_valid": false,
                    "mirek_schema": { "mirek_minimum": 153, "mirek_maximum": 500 }
                },
                "color": { "xy": { "x": 0.7, "y": 0.3 } }
            })))),
        };
        let request = PluginReadRequest {
            targets: vec![
                luminate_plugin_api::PluginReadTarget {
                    target: PluginTarget::Device {
                        device: device.id.clone(),
                    },
                    facets: vec![StateFacetKind::Appearance, StateFacetKind::Emission],
                },
                luminate_plugin_api::PluginReadTarget {
                    target: PluginTarget::Surface {
                        device: device.id.clone(),
                        surface: "wrong".to_owned(),
                    },
                    facets: vec![StateFacetKind::Emission],
                },
            ],
        };

        let snapshot = read(&transport, &[device], &request);

        assert_eq!(snapshot.errors.len(), 1);
        assert_eq!(snapshot.observations.len(), 2);
        assert!(snapshot.observations.iter().any(|observation| matches!(
            observation.value,
            FacetValue::Appearance(AppearanceState::Static(Colour::Additive(_)))
        )));
        assert!(
            snapshot.observations.iter().any(|observation| {
                observation.value == FacetValue::Emission(EmissionState::Dark)
            })
        );
    }
}
