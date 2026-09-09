// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned lock means another thread panicked and may have violated invariants"
)]

//! Validated CLIP v2 light mutations.

use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use luminate_core::colour::Colour;
use luminate_core::effect::Effect;
use luminate_core::rgb::Rgb;
use luminate_plugin_api::{
    PluginError, PluginTarget, PluginUpdate, PluginUpdateOperation, current_request_deadline,
};
use serde_json::{Value, json};

use crate::api::{self, ApiError, Transport};
use crate::colour::{ColourError, kelvin_to_mirek, rgb_to_xy};
use crate::http::HttpError;
use crate::topology::HueDevice;

const RATE_LIMIT_RETRY: Duration = Duration::from_millis(100);
const SURFACE_ID: &str = "light";

pub(crate) struct RateLimiter {
    interval: Duration,
    next: Mutex<Instant>,
}

impl RateLimiter {
    pub(crate) fn hue_lights() -> Self {
        Self::new(RATE_LIMIT_RETRY)
    }

    fn new(interval: Duration) -> Self {
        Self {
            interval,
            next: Mutex::new(Instant::now()),
        }
    }

    fn wait(&self) -> Result<(), PluginError> {
        let now = Instant::now();
        let remaining =
            current_request_deadline().and_then(luminate_plugin_api::PluginDeadline::remaining);
        let delay = self.reserve(now, remaining)?;

        if !delay.is_zero() {
            thread::sleep(delay);
        }
        Ok(())
    }

    fn reserve(&self, now: Instant, remaining: Option<Duration>) -> Result<Duration, PluginError> {
        let mut next = self.next.lock().expect("Hue rate limiter lock poisoned");
        let scheduled = (*next).max(now);
        let delay = scheduled.saturating_duration_since(now);
        if remaining.is_some_and(|remaining| remaining <= delay) {
            return Err(PluginError::RateLimited {
                diagnostic: "Hue light update would exceed the request deadline".to_owned(),
                retry_after: delay,
            });
        }
        *next = scheduled.checked_add(self.interval).unwrap_or(scheduled);
        Ok(delay)
    }
}

pub(crate) fn apply(
    transport: &impl Transport,
    rate_limiter: &RateLimiter,
    device: &HueDevice,
    update: &PluginUpdate,
) -> Result<(), PluginError> {
    validate_target(device, &update.target)?;
    let body = mutation_body(device, &update.operation)?;
    let body = serde_json::to_vec(&body).map_err(|error| {
        PluginError::Internal(format!("failed to serialize Hue update: {error}"))
    })?;
    rate_limiter.wait()?;

    api::put_light(transport, &device.endpoint, &device.light.id, &body).map_err(classify_api_error)
}

fn validate_target(device: &HueDevice, target: &PluginTarget) -> Result<(), PluginError> {
    match target {
        PluginTarget::Device { device: target } if target == &device.id => Ok(()),
        PluginTarget::Surface {
            device: target,
            surface,
        } if target == &device.id && surface == SURFACE_ID => Ok(()),
        PluginTarget::Device { .. }
        | PluginTarget::Surface { .. }
        | PluginTarget::Element { .. }
        | PluginTarget::Group { .. } => Err(PluginError::InvalidTarget(
            "target is not this Hue light or its whole-light surface".to_owned(),
        )),
    }
}

fn mutation_body(
    device: &HueDevice,
    operation: &PluginUpdateOperation,
) -> Result<Value, PluginError> {
    match operation {
        PluginUpdateOperation::Clear
        | PluginUpdateOperation::SetEffect {
            effect: Effect::Off,
        } => Ok(json!({ "on": { "on": false } })),
        PluginUpdateOperation::SetBrightness { value } => {
            if device.light.dimming.is_none() {
                return Err(PluginError::Unsupported(
                    "Hue light does not advertise dimming".to_owned(),
                ));
            }
            if *value > 100 {
                return Err(PluginError::InvalidArgument(
                    "Hue brightness must be between 0 and 100".to_owned(),
                ));
            }
            Ok(json!({ "dimming": { "brightness": value } }))
        }
        PluginUpdateOperation::SetEffect {
            effect: Effect::Static { colour },
        } => static_colour_body(device, colour),
        PluginUpdateOperation::SetEffect { .. } => Err(PluginError::Unsupported(
            "Hue REST lights do not support client-timed animations".to_owned(),
        )),
        PluginUpdateOperation::SaveCurrent => Err(PluginError::Unsupported(
            "Hue light state has no separate save operation".to_owned(),
        )),
        PluginUpdateOperation::SetAppearanceSlots { .. } => Err(PluginError::Unsupported(
            "Hue lights do not advertise appearance slots".to_owned(),
        )),
    }
}

fn static_colour_body(device: &HueDevice, colour: &Colour) -> Result<Value, PluginError> {
    match colour {
        Colour::Additive(_) => {
            if device.light.color.is_none() {
                return Err(PluginError::Unsupported(
                    "Hue light does not advertise colour control".to_owned(),
                ));
            }
            let rgb = colour
                .try_as_rgb()
                .map_err(|error| PluginError::InvalidArgument(error.to_string()))?;
            if rgb == Rgb::BLACK {
                return Ok(json!({ "on": { "on": false } }));
            }
            let hue_colour = device.light.color.as_ref().ok_or_else(|| {
                PluginError::Internal("validated Hue colour capability disappeared".to_owned())
            })?;
            let xy = rgb_to_xy(rgb, hue_colour.gamut.as_ref()).map_err(classify_colour_error)?;
            Ok(json!({
                "on": { "on": true },
                "color": { "xy": { "x": xy.x, "y": xy.y } }
            }))
        }
        Colour::Cct { kelvin } => {
            let temperature = device.light.color_temperature.as_ref().ok_or_else(|| {
                PluginError::Unsupported(
                    "Hue light does not advertise colour-temperature control".to_owned(),
                )
            })?;
            let mirek = kelvin_to_mirek(*kelvin, &temperature.mirek_schema)
                .map_err(classify_colour_error)?;
            Ok(json!({
                "on": { "on": true },
                "color_temperature": { "mirek": mirek }
            }))
        }
        Colour::Hsv { .. } | Colour::Hsl { .. } | Colour::Monochrome { .. } => {
            Err(PluginError::Unsupported(
                "Hue topology accepts only advertised RGB or colour-temperature values".to_owned(),
            ))
        }
    }
}

fn classify_colour_error(error: ColourError) -> PluginError {
    PluginError::InvalidArgument(error.to_string())
}

fn classify_api_error(error: ApiError) -> PluginError {
    match error {
        ApiError::Transport(HttpError::Status { code: 429, .. }) => PluginError::RateLimited {
            diagnostic: "Hue bridge rate-limited the light update".to_owned(),
            retry_after: RATE_LIMIT_RETRY,
        },
        ApiError::Transport(HttpError::Status {
            code: 401 | 403, ..
        }) => PluginError::Unavailable("Hue application key was rejected".to_owned()),
        ApiError::Transport(
            HttpError::Status {
                code: 404 | 503, ..
            }
            | HttpError::TimedOut,
        ) => PluginError::Unavailable("Hue light or bridge is unavailable".to_owned()),
        ApiError::Reported(codes) if codes.iter().any(|code| code == "client_error") => {
            PluginError::InvalidArgument("Hue rejected the light update".to_owned())
        }
        ApiError::Reported(codes)
            if codes.iter().any(|code| {
                code == "communication_error" || code == "attribute_may_have_no_effect"
            }) =>
        {
            PluginError::Unavailable("Hue could not confirm the light update".to_owned())
        }
        ApiError::Transport(HttpError::Tls(_) | HttpError::Io(_)) => {
            PluginError::Io("Hue HTTPS transport failed".to_owned())
        }
        ApiError::Transport(HttpError::Redirect(_)) => {
            PluginError::Io("Hue redirect was rejected".to_owned())
        }
        ApiError::Transport(HttpError::Status { code, .. }) => {
            PluginError::Io(format!("Hue returned HTTP {code}"))
        }
        ApiError::Transport(HttpError::Protocol(_))
        | ApiError::Malformed(_)
        | ApiError::ExcessiveErrors
        | ApiError::InvalidReportedError
        | ApiError::ExcessiveResources
        | ApiError::AmbiguousBridge
        | ApiError::BridgeIdentityMismatch
        | ApiError::DuplicateResourceId
        | ApiError::DuplicateServiceReference
        | ApiError::InvalidMutationResponse
        | ApiError::InvalidResource(_)
        | ApiError::Reported(_) => {
            PluginError::Io("Hue returned an invalid API response".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use luminate_core::colour::Colour;
    use luminate_core::effect::Effect;
    use luminate_core::rgb::Rgb;

    use super::*;
    use crate::api::{Device, Light};
    use crate::configuration::{BridgeId, Endpoint};

    struct FakeTransport {
        calls: RefCell<Vec<(String, Vec<u8>)>>,
        response: RefCell<Result<Vec<u8>, HttpError>>,
    }

    impl FakeTransport {
        fn successful() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                response: RefCell::new(Ok(br#"{"errors":[],"data":[{"rid":"33333333-3333-4333-8333-333333333333","rtype":"light"}]}"#.to_vec())),
            }
        }
    }

    impl Transport for FakeTransport {
        fn get_json(&self, _endpoint: &Endpoint, _path: &str) -> Result<Vec<u8>, HttpError> {
            Err(HttpError::Protocol("unexpected test GET".to_owned()))
        }

        fn put_json(
            &self,
            _endpoint: &Endpoint,
            path: &str,
            body: &[u8],
        ) -> Result<Vec<u8>, HttpError> {
            self.calls
                .borrow_mut()
                .push((path.to_owned(), body.to_vec()));
            self.response
                .borrow()
                .as_ref()
                .map(Clone::clone)
                .map_err(|error| HttpError::Protocol(error.to_string()))
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
        let light = serde_json::from_value::<Light>(json!({
            "id": "33333333-3333-4333-8333-333333333333",
            "owner": {
                "rid": "22222222-2222-4222-8222-222222222222",
                "rtype": "device"
            },
            "type": "light",
            "on": { "on": true },
            "dimming": { "brightness": 50.0 },
            "color_temperature": {
                "mirek": 250,
                "mirek_valid": true,
                "mirek_schema": { "mirek_minimum": 153, "mirek_maximum": 500 }
            },
            "color": {
                "xy": { "x": 0.3, "y": 0.3 },
                "gamut": {
                    "red": { "x": 0.6915, "y": 0.3083 },
                    "green": { "x": 0.17, "y": 0.7 },
                    "blue": { "x": 0.1532, "y": 0.0475 }
                }
            }
        }))
        .expect("light");
        HueDevice {
            id: "philips-hue:001788fffe123456:33333333-3333-4333-8333-333333333333".to_owned(),
            bridge_id: BridgeId::parse("001788fffe123456").expect("bridge ID"),
            endpoint: Endpoint::from_ip("192.0.2.1".parse().expect("IP"), 443).expect("endpoint"),
            device,
            light,
        }
    }

    fn update(operation: PluginUpdateOperation) -> PluginUpdate {
        PluginUpdate {
            target: PluginTarget::Device {
                device: hue_device().id,
            },
            operation,
        }
    }

    #[test]
    fn writes_minimal_brightness_off_rgb_and_temperature_payloads() {
        let transport = FakeTransport::successful();
        let device = hue_device();
        let operations = [
            PluginUpdateOperation::SetBrightness { value: 42 },
            PluginUpdateOperation::Clear,
            PluginUpdateOperation::SetEffect {
                effect: Effect::Static {
                    colour: Colour::rgb(Rgb::new(255, 0, 0)),
                },
            },
            PluginUpdateOperation::SetEffect {
                effect: Effect::Static {
                    colour: Colour::cct(4_000),
                },
            },
        ];
        for operation in operations {
            apply(
                &transport,
                &RateLimiter::new(Duration::ZERO),
                &device,
                &update(operation),
            )
            .expect("Hue update");
        }

        let calls = transport.calls.borrow();
        assert_eq!(calls.len(), 4);
        assert_eq!(
            serde_json::from_slice::<Value>(&calls[0].1).expect("brightness JSON"),
            json!({ "dimming": { "brightness": 42 } })
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&calls[1].1).expect("off JSON"),
            json!({ "on": { "on": false } })
        );
        let rgb = serde_json::from_slice::<Value>(&calls[2].1).expect("RGB JSON");
        assert_eq!(rgb["on"]["on"], true);
        assert!(rgb["color"]["xy"]["x"].as_f64().is_some());
        assert_eq!(
            serde_json::from_slice::<Value>(&calls[3].1).expect("CCT JSON"),
            json!({ "on": { "on": true }, "color_temperature": { "mirek": 250 } })
        );
    }

    #[test]
    fn rejects_invalid_targets_values_and_effects_before_io() {
        let transport = FakeTransport::successful();
        let device = hue_device();
        let mut invalid_target = update(PluginUpdateOperation::Clear);
        invalid_target.target = PluginTarget::Surface {
            device: device.id.clone(),
            surface: "not-light".to_owned(),
        };
        assert!(matches!(
            apply(
                &transport,
                &RateLimiter::new(Duration::ZERO),
                &device,
                &invalid_target
            ),
            Err(PluginError::InvalidTarget(_))
        ));
        assert!(matches!(
            apply(
                &transport,
                &RateLimiter::new(Duration::ZERO),
                &device,
                &update(PluginUpdateOperation::SetBrightness { value: 101 })
            ),
            Err(PluginError::InvalidArgument(_))
        ));
        assert!(matches!(
            apply(
                &transport,
                &RateLimiter::new(Duration::ZERO),
                &device,
                &update(PluginUpdateOperation::SetEffect {
                    effect: Effect::Spectrum { period_ms: 1_000 }
                })
            ),
            Err(PluginError::Unsupported(_))
        ));
        let mut no_dimming = device.clone();
        no_dimming.light.dimming = None;
        assert!(matches!(
            apply(
                &transport,
                &RateLimiter::new(Duration::ZERO),
                &no_dimming,
                &update(PluginUpdateOperation::SetBrightness { value: 50 })
            ),
            Err(PluginError::Unsupported(_))
        ));
        let mut no_colour = device.clone();
        no_colour.light.color = None;
        assert!(matches!(
            apply(
                &transport,
                &RateLimiter::new(Duration::ZERO),
                &no_colour,
                &update(PluginUpdateOperation::SetEffect {
                    effect: Effect::Static {
                        colour: Colour::rgb(Rgb::WHITE)
                    }
                })
            ),
            Err(PluginError::Unsupported(_))
        ));
        assert!(transport.calls.borrow().is_empty());
    }

    #[test]
    fn rejects_api_error_envelopes_and_mismatched_success_identifiers() {
        let transport = FakeTransport::successful();
        *transport.response.borrow_mut() = Ok(
            br#"{"errors":[{"description":"busy","error_code":"communication_error"}],"data":[]}"#
                .to_vec(),
        );
        assert!(matches!(
            apply(
                &transport,
                &RateLimiter::new(Duration::ZERO),
                &hue_device(),
                &update(PluginUpdateOperation::Clear)
            ),
            Err(PluginError::Unavailable(_))
        ));

        *transport.response.borrow_mut() = Ok(br#"{"errors":[],"data":[{"rid":"44444444-4444-4444-8444-444444444444","rtype":"light"}]}"#.to_vec());
        assert!(matches!(
            apply(
                &transport,
                &RateLimiter::new(Duration::ZERO),
                &hue_device(),
                &update(PluginUpdateOperation::Clear)
            ),
            Err(PluginError::Io(_))
        ));
    }

    #[test]
    fn classifies_authentication_rate_limit_and_transport_failures() {
        assert!(matches!(
            classify_api_error(ApiError::Transport(HttpError::Status {
                code: 401,
                reason: "Unauthorized".to_owned()
            })),
            PluginError::Unavailable(_)
        ));
        assert!(matches!(
            classify_api_error(ApiError::Transport(HttpError::Status {
                code: 429,
                reason: "Too Many Requests".to_owned()
            })),
            PluginError::RateLimited { retry_after, .. }
                if retry_after == Duration::from_millis(100)
        ));
        assert!(matches!(
            classify_api_error(ApiError::Reported(vec!["client_error".to_owned()])),
            PluginError::InvalidArgument(_)
        ));
        assert!(matches!(
            classify_api_error(ApiError::Transport(HttpError::TimedOut)),
            PluginError::Unavailable(_)
        ));
    }

    #[test]
    fn spaces_light_updates_and_preserves_a_rejected_reservation() {
        let limiter = RateLimiter::new(Duration::from_millis(100));
        let start = Instant::now();
        *limiter.next.lock().expect("rate limiter") = start;

        assert_eq!(
            limiter.reserve(start, None).expect("first slot"),
            Duration::ZERO
        );
        assert_eq!(
            limiter.reserve(start, None).expect("second slot"),
            Duration::from_millis(100)
        );
        assert!(matches!(
            limiter.reserve(start, Some(Duration::from_millis(150))),
            Err(PluginError::RateLimited { retry_after, .. })
                if retry_after == Duration::from_millis(200)
        ));
        assert_eq!(
            limiter
                .reserve(start, Some(Duration::from_millis(250)))
                .expect("preserved third slot"),
            Duration::from_millis(200)
        );
    }
}
