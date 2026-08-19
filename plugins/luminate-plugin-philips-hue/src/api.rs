// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Typed and validated Hue CLIP v2 resource snapshots.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::mem;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use thiserror::Error;

use crate::configuration::{BridgeId, Endpoint};
use crate::http::{HttpClient, HttpError};

const MAX_RESOURCES: usize = 4_096;
const MAX_SERVICES_PER_DEVICE: usize = 128;
const MAX_API_ERRORS: usize = 64;
const MAX_RESOURCE_TYPE_LENGTH: usize = 64;
const MAX_PRODUCT_FIELD_LENGTH: usize = 128;
const MAX_NAME_LENGTH: usize = 32;

pub(crate) trait Transport {
    fn get_json(&self, endpoint: &Endpoint, path: &str) -> Result<Vec<u8>, HttpError>;

    fn put_json(&self, endpoint: &Endpoint, path: &str, body: &[u8]) -> Result<Vec<u8>, HttpError>;
}

impl Transport for HttpClient {
    fn get_json(&self, endpoint: &Endpoint, path: &str) -> Result<Vec<u8>, HttpError> {
        self.get(endpoint, path)
    }

    fn put_json(&self, endpoint: &Endpoint, path: &str, body: &[u8]) -> Result<Vec<u8>, HttpError> {
        self.put(endpoint, path, body)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub(crate) devices: Vec<Device>,
    pub(crate) lights: Vec<Light>,
}

impl Snapshot {
    pub(crate) fn owned_lights(&self) -> Vec<OwnedLight<'_>> {
        let devices = self
            .devices
            .iter()
            .map(|device| (&device.id, device))
            .collect::<HashMap<_, _>>();

        self.lights
            .iter()
            .filter_map(|light| {
                let device = devices.get(&light.owner.rid)?;
                (light.owner.rtype.is("device")
                    && device
                        .services
                        .iter()
                        .any(|service| service.rtype.is("light") && service.rid == light.id))
                .then_some(OwnedLight { device, light })
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OwnedLight<'a> {
    pub(crate) device: &'a Device,
    pub(crate) light: &'a Light,
}

pub(crate) fn enumerate(
    transport: &impl Transport,
    endpoint: &Endpoint,
    expected_bridge_id: &BridgeId,
) -> Result<Snapshot, ApiError> {
    let bridges = get_resources::<Bridge>(transport, endpoint, "/clip/v2/resource/bridge")?;
    validate_bridge(bridges, expected_bridge_id)?;
    let devices = get_resources::<Device>(transport, endpoint, "/clip/v2/resource/device")?;
    let lights = get_resources::<Light>(transport, endpoint, "/clip/v2/resource/light")?;

    validate_unique_ids(&devices)?;
    validate_unique_ids(&lights)?;
    for device in &devices {
        device.validate()?;
    }
    for light in &lights {
        light.validate()?;
    }

    Ok(Snapshot { devices, lights })
}

pub(crate) fn put_light(
    transport: &impl Transport,
    endpoint: &Endpoint,
    light_id: &ResourceId,
    body: &[u8],
) -> Result<(), ApiError> {
    let path = format!("/clip/v2/resource/light/{}", light_id.as_str());
    let response = transport.put_json(endpoint, &path, body)?;
    let envelope = serde_json::from_slice::<Envelope<ResourceReference>>(&response)
        .map_err(|error| ApiError::Malformed(error.to_string()))?;
    validate_envelope_errors(&envelope.errors)?;
    let [updated] =
        <Vec<ResourceReference> as TryInto<[ResourceReference; 1]>>::try_into(envelope.data)
            .map_err(|_| ApiError::InvalidMutationResponse)?;
    if updated.rid != *light_id || !updated.rtype.is("light") {
        return Err(ApiError::InvalidMutationResponse);
    }
    Ok(())
}

pub(crate) fn get_light(
    transport: &impl Transport,
    endpoint: &Endpoint,
    light_id: &ResourceId,
) -> Result<Light, ApiError> {
    let path = format!("/clip/v2/resource/light/{}", light_id.as_str());
    let mut lights = get_resources::<Light>(transport, endpoint, &path)?;
    let [light] = <Vec<Light> as TryInto<[Light; 1]>>::try_into(mem::take(&mut lights))
        .map_err(|_| ApiError::InvalidResource("individual light response"))?;
    light.validate()?;
    if light.id != *light_id {
        return Err(ApiError::InvalidResource("individual light identity"));
    }
    Ok(light)
}

fn get_resources<T>(
    transport: &impl Transport,
    endpoint: &Endpoint,
    path: &str,
) -> Result<Vec<T>, ApiError>
where
    T: for<'de> Deserialize<'de>,
{
    let response = transport.get_json(endpoint, path)?;
    let envelope = serde_json::from_slice::<Envelope<T>>(&response)
        .map_err(|error| ApiError::Malformed(error.to_string()))?;
    validate_envelope_errors(&envelope.errors)?;
    if envelope.data.len() > MAX_RESOURCES {
        return Err(ApiError::ExcessiveResources);
    }
    Ok(envelope.data)
}

fn validate_envelope_errors(errors: &[ReportedError]) -> Result<(), ApiError> {
    if errors.len() > MAX_API_ERRORS {
        return Err(ApiError::ExcessiveErrors);
    }
    if errors.iter().any(|error| {
        !valid_text(&error.error_code, MAX_RESOURCE_TYPE_LENGTH, false)
            || !valid_text(&error.description, MAX_PRODUCT_FIELD_LENGTH, false)
    }) {
        return Err(ApiError::InvalidReportedError);
    }
    if !errors.is_empty() {
        return Err(ApiError::Reported(
            errors
                .iter()
                .map(|error| error.error_code.clone())
                .collect(),
        ));
    }
    Ok(())
}

fn validate_bridge(
    bridges: Vec<Bridge>,
    expected_bridge_id: &BridgeId,
) -> Result<Bridge, ApiError> {
    let [bridge] = <Vec<Bridge> as TryInto<[Bridge; 1]>>::try_into(bridges)
        .map_err(|_| ApiError::AmbiguousBridge)?;
    if !bridge.resource_type.is("bridge")
        || BridgeId::parse(&bridge.configured_identity).as_ref() != Ok(expected_bridge_id)
    {
        return Err(ApiError::BridgeIdentityMismatch);
    }
    Ok(bridge)
}

fn validate_unique_ids<T: Identified>(resources: &[T]) -> Result<(), ApiError> {
    let mut ids = HashSet::with_capacity(resources.len());
    if resources.iter().all(|resource| ids.insert(resource.id())) {
        Ok(())
    } else {
        Err(ApiError::DuplicateResourceId)
    }
}

trait Identified {
    fn id(&self) -> &ResourceId;
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    errors: Vec<ReportedError>,
    data: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct ReportedError {
    error_code: String,
    description: String,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct ResourceId(String);

impl ResourceId {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    fn parse(value: String) -> Result<Self, &'static str> {
        let bytes = value.as_bytes();
        if bytes.len() != 36
            || bytes.get(8) != Some(&b'-')
            || bytes.get(13) != Some(&b'-')
            || bytes.get(18) != Some(&b'-')
            || bytes.get(23) != Some(&b'-')
            || bytes.iter().enumerate().any(|(index, byte)| {
                !matches!(index, 8 | 13 | 18 | 23) && !matches!(byte, b'0'..=b'9' | b'a'..=b'f')
            })
        {
            return Err("invalid Hue resource UUID");
        }
        Ok(Self(value))
    }
}

impl fmt::Debug for ResourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("ResourceId").field(&self.0).finish()
    }
}

impl<'de> Deserialize<'de> for ResourceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResourceType(String);

impl ResourceType {
    pub(crate) fn is(&self, expected: &str) -> bool {
        self.0 == expected
    }
}

impl<'de> Deserialize<'de> for ResourceType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.is_empty()
            || value.len() > MAX_RESOURCE_TYPE_LENGTH
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        {
            return Err(D::Error::custom("invalid Hue resource type"));
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct ResourceReference {
    pub(crate) rid: ResourceId,
    pub(crate) rtype: ResourceType,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Bridge {
    #[serde(rename = "id")]
    pub(crate) _id: ResourceId,
    #[serde(rename = "type")]
    pub(crate) resource_type: ResourceType,
    #[serde(rename = "bridge_id")]
    pub(crate) configured_identity: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Device {
    pub(crate) id: ResourceId,
    #[serde(rename = "type")]
    pub(crate) resource_type: ResourceType,
    pub(crate) product_data: ProductData,
    pub(crate) metadata: Metadata,
    pub(crate) services: Vec<ResourceReference>,
}

impl Device {
    fn validate(&self) -> Result<(), ApiError> {
        if !self.resource_type.is("device")
            || self.services.len() > MAX_SERVICES_PER_DEVICE
            || !valid_text(&self.metadata.name, MAX_NAME_LENGTH, false)
            || !valid_text(&self.product_data.model_id, MAX_PRODUCT_FIELD_LENGTH, false)
            || !valid_text(
                &self.product_data.manufacturer_name,
                MAX_PRODUCT_FIELD_LENGTH,
                false,
            )
            || !valid_text(
                &self.product_data.product_name,
                MAX_PRODUCT_FIELD_LENGTH,
                false,
            )
        {
            return Err(ApiError::InvalidResource("device"));
        }
        let mut services = HashSet::with_capacity(self.services.len());
        if !self
            .services
            .iter()
            .all(|service| services.insert((service.rid.clone(), service.rtype.0.clone())))
        {
            return Err(ApiError::DuplicateServiceReference);
        }
        Ok(())
    }
}

impl Identified for Device {
    fn id(&self) -> &ResourceId {
        &self.id
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ProductData {
    pub(crate) model_id: String,
    pub(crate) manufacturer_name: String,
    pub(crate) product_name: String,
    #[serde(rename = "certified")]
    pub(crate) _certified: bool,
    #[serde(rename = "software_version")]
    pub(crate) _software_version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Metadata {
    pub(crate) name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Light {
    pub(crate) id: ResourceId,
    pub(crate) owner: ResourceReference,
    #[serde(rename = "type")]
    pub(crate) resource_type: ResourceType,
    #[serde(rename = "on")]
    pub(crate) on: OnState,
    pub(crate) dimming: Option<Dimming>,
    pub(crate) color_temperature: Option<ColorTemperature>,
    pub(crate) color: Option<Color>,
}

impl Light {
    fn validate(&self) -> Result<(), ApiError> {
        if !self.resource_type.is("light") || !self.owner.rtype.is("device") {
            return Err(ApiError::InvalidResource("light"));
        }
        if let Some(dimming) = &self.dimming {
            validate_percentage(dimming.brightness)?;
            if let Some(minimum) = dimming.min_dim_level {
                validate_percentage(minimum)?;
            }
        }
        if let Some(temperature) = &self.color_temperature
            && (temperature
                .mirek
                .is_some_and(|mirek| !(50..=1_000).contains(&mirek))
                || !(50..=1_000).contains(&temperature.mirek_schema.mirek_minimum)
                || !(50..=1_000).contains(&temperature.mirek_schema.mirek_maximum)
                || temperature.mirek_schema.mirek_minimum > temperature.mirek_schema.mirek_maximum
                || (temperature.mirek_valid && temperature.mirek.is_none()))
        {
            return Err(ApiError::InvalidResource("light colour temperature"));
        }
        if let Some(color) = &self.color {
            color.xy.validate()?;
            if let Some(gamut) = &color.gamut {
                gamut.red.validate()?;
                gamut.green.validate()?;
                gamut.blue.validate()?;
                let area = (gamut.green.x - gamut.red.x).mul_add(
                    gamut.blue.y - gamut.red.y,
                    -(gamut.green.y - gamut.red.y) * (gamut.blue.x - gamut.red.x),
                );
                if !area.is_finite() || area.abs() <= f64::EPSILON {
                    return Err(ApiError::InvalidResource("light colour gamut"));
                }
            }
        }
        Ok(())
    }

    pub(crate) const fn has_dimming(&self) -> bool {
        self.dimming.is_some()
    }

    pub(crate) const fn has_colour_temperature(&self) -> bool {
        self.color_temperature.is_some()
    }

    pub(crate) const fn has_colour(&self) -> bool {
        self.color.is_some()
    }
}

impl Identified for Light {
    fn id(&self) -> &ResourceId {
        &self.id
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct OnState {
    #[serde(rename = "on")]
    pub(crate) on: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Dimming {
    pub(crate) brightness: f64,
    pub(crate) min_dim_level: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ColorTemperature {
    pub(crate) mirek: Option<u16>,
    pub(crate) mirek_valid: bool,
    pub(crate) mirek_schema: MirekSchema,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MirekSchema {
    pub(crate) mirek_minimum: u16,
    pub(crate) mirek_maximum: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Color {
    pub(crate) xy: Xy,
    pub(crate) gamut: Option<Gamut>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Gamut {
    pub(crate) red: Xy,
    pub(crate) green: Xy,
    pub(crate) blue: Xy,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Xy {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

impl Xy {
    fn validate(&self) -> Result<(), ApiError> {
        if self.x.is_finite()
            && self.y.is_finite()
            && (0.0..=1.0).contains(&self.x)
            && (0.0..=1.0).contains(&self.y)
        {
            Ok(())
        } else {
            Err(ApiError::InvalidResource("light colour"))
        }
    }
}

fn validate_percentage(value: f64) -> Result<(), ApiError> {
    if value.is_finite() && (0.0..=100.0).contains(&value) {
        Ok(())
    } else {
        Err(ApiError::InvalidResource("light dimming"))
    }
}

fn valid_text(value: &str, maximum: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.len() <= maximum
        && !value.chars().any(char::is_control)
}

#[derive(Debug, Error)]
pub(crate) enum ApiError {
    #[error(transparent)]
    Transport(#[from] HttpError),
    #[error("malformed Hue API response: {0}")]
    Malformed(String),
    #[error("Hue returned too many API errors")]
    ExcessiveErrors,
    #[error("Hue returned a malformed API error")]
    InvalidReportedError,
    #[error("Hue returned API errors: {0:?}")]
    Reported(Vec<String>),
    #[error("Hue returned too many resources")]
    ExcessiveResources,
    #[error("Hue returned an ambiguous bridge resource set")]
    AmbiguousBridge,
    #[error("authenticated Hue bridge identity does not match configuration")]
    BridgeIdentityMismatch,
    #[error("Hue returned duplicate resource IDs")]
    DuplicateResourceId,
    #[error("Hue returned duplicate device service references")]
    DuplicateServiceReference,
    #[error("Hue returned an invalid light mutation response")]
    InvalidMutationResponse,
    #[error("Hue returned an invalid {0} resource")]
    InvalidResource(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::ApplicationKey;
    use crate::topology;
    use luminate_core::capability::{
        BrightnessCapability, CapabilityScope, ColourCapability, ReadableFacet, ReadbackFidelity,
        StateReadbackCapability,
    };
    use luminate_core::state::StateFacetKind;

    const DEVICE_UUID: &str = "22222222-2222-4222-8222-222222222222";
    const LIGHT_UUID: &str = "33333333-3333-4333-8333-333333333333";

    struct FakeTransport {
        bridge: &'static [u8],
        devices: &'static [u8],
        lights: &'static [u8],
    }

    impl Transport for FakeTransport {
        fn get_json(&self, _endpoint: &Endpoint, path: &str) -> Result<Vec<u8>, HttpError> {
            match path {
                "/clip/v2/resource/bridge" => Ok(self.bridge.to_vec()),
                "/clip/v2/resource/device" => Ok(self.devices.to_vec()),
                "/clip/v2/resource/light" => Ok(self.lights.to_vec()),
                _ => Err(HttpError::Protocol("unexpected test path".to_owned())),
            }
        }

        fn put_json(
            &self,
            _endpoint: &Endpoint,
            _path: &str,
            _body: &[u8],
        ) -> Result<Vec<u8>, HttpError> {
            Err(HttpError::Protocol("unexpected test mutation".to_owned()))
        }
    }

    fn fixture() -> FakeTransport {
        FakeTransport {
            bridge: br#"{"errors":[],"data":[{"id":"11111111-1111-4111-8111-111111111111","type":"bridge","bridge_id":"001788fffe123456","future":true}]}"#,
            devices: br#"{"errors":[],"data":[{"id":"22222222-2222-4222-8222-222222222222","type":"device","product_data":{"model_id":"test-model","manufacturer_name":"Test Vendor","product_name":"Test Lamp","product_archetype":"classic_bulb","certified":true,"software_version":"1.2.3"},"metadata":{"name":"Test light","archetype":"classic_bulb"},"services":[{"rid":"33333333-3333-4333-8333-333333333333","rtype":"light"}]}]}"#,
            lights: br#"{"errors":[],"data":[{"id":"33333333-3333-4333-8333-333333333333","owner":{"rid":"22222222-2222-4222-8222-222222222222","rtype":"device"},"type":"light","on":{"on":true},"dimming":{"brightness":42.5,"min_dim_level":0.2},"color_temperature":{"mirek":250,"mirek_valid":true,"mirek_schema":{"mirek_minimum":153,"mirek_maximum":500}},"color":{"xy":{"x":0.3,"y":0.4},"gamut":{"red":{"x":0.7,"y":0.3},"green":{"x":0.2,"y":0.7},"blue":{"x":0.1,"y":0.1}}},"unknown_future_field":{}}]}"#,
        }
    }

    fn endpoint() -> Endpoint {
        Endpoint::from_ip("192.0.2.1".parse().expect("IP address"), 443).expect("endpoint")
    }

    #[test]
    fn enumerates_validates_and_joins_supported_resources() {
        let bridge_id = BridgeId::parse("001788fffe123456").expect("bridge ID");
        let snapshot = enumerate(&fixture(), &endpoint(), &bridge_id).expect("snapshot");

        assert_eq!(snapshot.devices.len(), 1);
        assert_eq!(snapshot.lights.len(), 1);
        let owned = snapshot.owned_lights();
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].device.id.as_str(), DEVICE_UUID);
        assert_eq!(owned[0].light.id.as_str(), LIGHT_UUID);
        assert!(owned[0].light.has_dimming());
        assert!(owned[0].light.has_colour_temperature());
        assert!(owned[0].light.has_colour());
    }

    #[test]
    fn rejects_wrong_bridge_and_api_level_errors() {
        let wrong_id = BridgeId::parse("001788fffe654321").expect("bridge ID");
        assert!(matches!(
            enumerate(&fixture(), &endpoint(), &wrong_id),
            Err(ApiError::BridgeIdentityMismatch)
        ));

        let mut fixture = fixture();
        fixture.bridge = br#"{"errors":[{"description":"not authorized","error_code":"client_error"}],"data":[]}"#;
        assert!(matches!(
            enumerate(&fixture, &endpoint(), &wrong_id),
            Err(ApiError::Reported(_))
        ));
    }

    #[test]
    fn rejects_malformed_ids_invalid_capabilities_and_duplicates() {
        assert!(serde_json::from_str::<ResourceId>(r#""not-a-uuid""#).is_err());

        let mut fixture = fixture();
        fixture.lights = br#"{"errors":[],"data":[{"id":"33333333-3333-4333-8333-333333333333","owner":{"rid":"22222222-2222-4222-8222-222222222222","rtype":"device"},"type":"light","on":{"on":true},"dimming":{"brightness":101.0}},{"id":"33333333-3333-4333-8333-333333333333","owner":{"rid":"22222222-2222-4222-8222-222222222222","rtype":"device"},"type":"light","on":{"on":false}}]}"#;
        let bridge_id = BridgeId::parse("001788fffe123456").expect("bridge ID");
        assert!(matches!(
            enumerate(&fixture, &endpoint(), &bridge_id),
            Err(ApiError::DuplicateResourceId)
        ));

        fixture.lights = br#"{"errors":[],"data":[{"id":"33333333-3333-4333-8333-333333333333","owner":{"rid":"22222222-2222-4222-8222-222222222222","rtype":"device"},"type":"light","on":{"on":true},"dimming":{"brightness":101.0}}]}"#;
        assert!(matches!(
            enumerate(&fixture, &endpoint(), &bridge_id),
            Err(ApiError::InvalidResource("light dimming"))
        ));
    }

    #[test]
    fn omits_lights_without_an_unambiguous_ownership_join() {
        let mut fixture = fixture();
        fixture.devices = br#"{"errors":[],"data":[{"id":"22222222-2222-4222-8222-222222222222","type":"device","product_data":{"model_id":"test-model","manufacturer_name":"Test Vendor","product_name":"Test Lamp","certified":true,"software_version":"1.2.3"},"metadata":{"name":"Test light"},"services":[]}]}"#;
        let bridge_id = BridgeId::parse("001788fffe123456").expect("bridge ID");
        let snapshot = enumerate(&fixture, &endpoint(), &bridge_id).expect("snapshot");

        assert!(snapshot.owned_lights().is_empty());
    }

    #[test]
    fn maps_structural_capabilities_and_stable_identity_into_topology() {
        let bridge_id = BridgeId::parse("001788fffe123456").expect("bridge ID");
        let endpoint = endpoint();
        let snapshot = enumerate(&fixture(), &endpoint, &bridge_id).expect("snapshot");
        let (_, device, _) = topology::registry_entries(&snapshot, &endpoint, &bridge_id)
            .into_iter()
            .next()
            .expect("registry entry");
        let descriptor = topology::descriptor(&device);

        assert_eq!(
            descriptor.id,
            "philips-hue:001788fffe123456:33333333-3333-4333-8333-333333333333"
        );
        assert_eq!(descriptor.name, "Test light (333333333333)");
        assert_eq!(descriptor.vendor.as_deref(), Some("Test Vendor"));
        assert!(descriptor.capabilities.emission);
        assert!(descriptor.capabilities.physical_power.is_none());
        assert_eq!(
            descriptor.capabilities.brightness,
            BrightnessCapability::Independent {
                bits: 7,
                maximum: 100,
                scope: CapabilityScope::Device,
            }
        );
        assert_eq!(
            descriptor.capabilities.colour,
            vec![ColourCapability::rgb8(), ColourCapability::cct(16)]
        );
        assert_eq!(
            descriptor.capabilities.state_readback,
            StateReadbackCapability::Readable {
                facets: vec![
                    ReadableFacet {
                        facet: StateFacetKind::Appearance,
                        fidelity: ReadbackFidelity::BestEffort,
                    },
                    ReadableFacet {
                        facet: StateFacetKind::Brightness,
                        fidelity: ReadbackFidelity::BestEffort,
                    },
                    ReadableFacet {
                        facet: StateFacetKind::Emission,
                        fidelity: ReadbackFidelity::Exact,
                    },
                ],
                read_disturbs_output: false,
                notifies_external_changes: false,
            }
        );
        assert_eq!(descriptor.surfaces.len(), 1);
        assert_eq!(descriptor.surfaces[0].id, "light");
    }

    #[test]
    fn topology_is_exact_for_on_only_and_tunable_white_lights() {
        let bridge_id = BridgeId::parse("001788fffe123456").expect("bridge ID");
        let endpoint = endpoint();
        let mut on_only = fixture();
        on_only.lights = br#"{"errors":[],"data":[{"id":"33333333-3333-4333-8333-333333333333","owner":{"rid":"22222222-2222-4222-8222-222222222222","rtype":"device"},"type":"light","on":{"on":false}}]}"#;
        let snapshot = enumerate(&on_only, &endpoint, &bridge_id).expect("on-only snapshot");
        let (_, device, _) = topology::registry_entries(&snapshot, &endpoint, &bridge_id)
            .into_iter()
            .next()
            .expect("registry entry");
        let descriptor = topology::descriptor(&device);
        assert!(descriptor.capabilities.emission);
        assert_eq!(
            descriptor.capabilities.brightness,
            BrightnessCapability::None
        );
        assert!(descriptor.capabilities.colour.is_empty());

        let mut tunable_white = fixture();
        tunable_white.lights = br#"{"errors":[],"data":[{"id":"33333333-3333-4333-8333-333333333333","owner":{"rid":"22222222-2222-4222-8222-222222222222","rtype":"device"},"type":"light","on":{"on":true},"dimming":{"brightness":50.0},"color_temperature":{"mirek":null,"mirek_valid":false,"mirek_schema":{"mirek_minimum":153,"mirek_maximum":500}}}]}"#;
        let snapshot =
            enumerate(&tunable_white, &endpoint, &bridge_id).expect("tunable-white snapshot");
        let (_, device, _) = topology::registry_entries(&snapshot, &endpoint, &bridge_id)
            .into_iter()
            .next()
            .expect("registry entry");
        let descriptor = topology::descriptor(&device);
        assert_eq!(
            descriptor.capabilities.colour,
            vec![ColourCapability::cct(16)]
        );
    }

    #[test]
    fn topology_fingerprint_ignores_state_but_tracks_descriptor_inputs() {
        let bridge_id = BridgeId::parse("001788fffe123456").expect("bridge ID");
        let endpoint = endpoint();
        let original = enumerate(&fixture(), &endpoint, &bridge_id).expect("snapshot");
        let mut changed_state = original.clone();
        changed_state.lights[0]
            .dimming
            .as_mut()
            .expect("dimming")
            .brightness = 5.0;
        let mut renamed = original.clone();
        renamed.devices[0].metadata.name = "Renamed light".to_owned();

        let fingerprint = |snapshot: &Snapshot| {
            topology::registry_entries(snapshot, &endpoint, &bridge_id)
                .into_iter()
                .next()
                .expect("registry entry")
                .2
        };
        assert_eq!(fingerprint(&original), fingerprint(&changed_state));
        assert_ne!(fingerprint(&original), fingerprint(&renamed));
    }

    #[test]
    fn fixture_credentials_are_not_needed_by_the_api_parser() {
        let key = serde_json::from_str::<ApplicationKey>(r#""test-only-key""#).expect("key");
        assert_eq!(format!("{key:?}"), "ApplicationKey([REDACTED])");
    }
}
