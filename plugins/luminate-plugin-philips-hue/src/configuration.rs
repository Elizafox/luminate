// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Typed, validated configuration for one Hue Bridge.

use std::fmt;
use std::net::IpAddr;
use std::str::FromStr as _;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use thiserror::Error;
use zeroize::Zeroizing;

const BRIDGE_ID_LENGTH: usize = 16;
const MAX_APPLICATION_KEY_LENGTH: usize = 1_024;
const MAX_ENDPOINT_LENGTH: usize = 512;
const DEFAULT_HTTPS_PORT: u16 = 443;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ApplicationKey(Zeroizing<String>);

impl ApplicationKey {
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    pub(crate) fn parse(value: String) -> Result<Self, ConfigurationError> {
        let value = Self(Zeroizing::new(value));
        validate_application_key(&value)?;
        Ok(value)
    }
}

impl fmt::Debug for ApplicationKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApplicationKey([REDACTED])")
    }
}

impl<'de> Deserialize<'de> for ApplicationKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(|value| Self(Zeroizing::new(value)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct BridgeId(String);

impl BridgeId {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn parse(value: &str) -> Result<Self, ConfigurationError> {
        if value.len() != BRIDGE_ID_LENGTH || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ConfigurationError::InvalidBridgeId);
        }

        Ok(Self(value.to_ascii_lowercase()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Endpoint {
    host: String,
    port: u16,
}

impl Endpoint {
    pub(crate) fn from_ip(address: IpAddr, port: u16) -> Result<Self, ConfigurationError> {
        if port == 0 {
            return Err(ConfigurationError::InvalidEndpoint);
        }
        Ok(Self {
            host: address.to_string(),
            port,
        })
    }

    pub(crate) fn host(&self) -> &str {
        &self.host
    }

    pub(crate) const fn port(&self) -> u16 {
        self.port
    }

    pub(crate) fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, ConfigurationError> {
        if value.is_empty()
            || value.len() > MAX_ENDPOINT_LENGTH
            || value.chars().any(char::is_whitespace)
            || value.contains('/')
            || value.contains("//")
        {
            return Err(ConfigurationError::InvalidEndpoint);
        }

        if let Ok(address) = IpAddr::from_str(value) {
            return Ok(Self {
                host: address.to_string(),
                port: DEFAULT_HTTPS_PORT,
            });
        }

        if let Some(rest) = value.strip_prefix('[') {
            let (host, suffix) = rest
                .split_once(']')
                .ok_or(ConfigurationError::InvalidEndpoint)?;
            let address =
                IpAddr::from_str(host).map_err(|_| ConfigurationError::InvalidEndpoint)?;
            if !address.is_ipv6() {
                return Err(ConfigurationError::InvalidEndpoint);
            }
            let port = parse_optional_port(suffix)?;
            return Ok(Self {
                host: address.to_string(),
                port,
            });
        }

        let (host, port) = match value.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => (host, parse_port(port)?),
            Some(_) => return Err(ConfigurationError::InvalidEndpoint),
            None => (value, DEFAULT_HTTPS_PORT),
        };
        if host.is_empty()
            || host.len() > 253
            || host.starts_with('.')
            || host.ends_with('.')
            || host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
        {
            return Err(ConfigurationError::InvalidEndpoint);
        }

        Ok(Self {
            host: host.to_ascii_lowercase(),
            port,
        })
    }
}

fn parse_optional_port(suffix: &str) -> Result<u16, ConfigurationError> {
    if suffix.is_empty() {
        Ok(DEFAULT_HTTPS_PORT)
    } else {
        parse_port(
            suffix
                .strip_prefix(':')
                .ok_or(ConfigurationError::InvalidEndpoint)?,
        )
    }
}

fn parse_port(value: &str) -> Result<u16, ConfigurationError> {
    value
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or(ConfigurationError::InvalidEndpoint)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfiguration {
    bridge_id: String,
    application_key: ApplicationKey,
    #[serde(default = "default_mdns")]
    mdns: bool,
    #[serde(default)]
    endpoint: Option<String>,
}

const fn default_mdns() -> bool {
    true
}

#[derive(Debug, Clone)]
pub(crate) struct HueConfiguration {
    pub(crate) bridge_id: BridgeId,
    pub(crate) application_key: ApplicationKey,
    pub(crate) mdns: bool,
    pub(crate) endpoint: Option<Endpoint>,
}

impl<'de> Deserialize<'de> for HueConfiguration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawConfiguration::deserialize(deserializer)?;
        Self::try_from(raw).map_err(D::Error::custom)
    }
}

impl TryFrom<RawConfiguration> for HueConfiguration {
    type Error = ConfigurationError;

    fn try_from(raw: RawConfiguration) -> Result<Self, Self::Error> {
        let bridge_id = BridgeId::parse(&raw.bridge_id)?;
        validate_application_key(&raw.application_key)?;
        let endpoint = raw.endpoint.as_deref().map(Endpoint::parse).transpose()?;
        if !raw.mdns && endpoint.is_none() {
            return Err(ConfigurationError::NoEndpointOrDiscovery);
        }

        Ok(Self {
            bridge_id,
            application_key: raw.application_key,
            mdns: raw.mdns,
            endpoint,
        })
    }
}

fn validate_application_key(key: &ApplicationKey) -> Result<(), ConfigurationError> {
    let value = key.expose();
    if value.is_empty()
        || value.len() > MAX_APPLICATION_KEY_LENGTH
        || value.chars().any(char::is_control)
    {
        Err(ConfigurationError::InvalidApplicationKey)
    } else {
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ConfigurationError {
    #[error("bridge_id must contain exactly 16 hexadecimal digits")]
    InvalidBridgeId,
    #[error("application_key must be non-empty, bounded, and contain no control characters")]
    InvalidApplicationKey,
    #[error("endpoint must be a hostname or IP address with an optional non-zero port")]
    InvalidEndpoint,
    #[error("endpoint is required when mDNS discovery is disabled")]
    NoEndpointOrDiscovery,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(value: serde_json::Value) -> Result<HueConfiguration, serde_json::Error> {
        serde_json::from_value(value)
    }

    #[test]
    fn validates_and_normalizes_single_bridge_configuration() {
        let configuration = parse(json!({
            "bridge_id": "001788FFFE012345",
            "application_key": "secret-value",
            "endpoint": "[2001:db8::1]:8443"
        }))
        .expect("valid Hue configuration");

        assert_eq!(configuration.bridge_id.as_str(), "001788fffe012345");
        assert!(configuration.mdns);
        let endpoint = configuration.endpoint.expect("configured endpoint");
        assert_eq!(endpoint.host(), "2001:db8::1");
        assert_eq!(endpoint.port(), 8443);
    }

    #[test]
    fn application_key_debug_output_is_redacted() {
        let configuration = parse(json!({
            "bridge_id": "001788fffe012345",
            "application_key": "do-not-print-this"
        }))
        .expect("valid Hue configuration");
        let debug = format!("{configuration:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("do-not-print-this"));
    }

    #[test]
    fn rejects_unknown_missing_and_invalid_values() {
        assert!(
            parse(json!({
                "bridge_id": "001788fffe012345",
                "application_key": "key",
                "mdns": false,
                "mdsn": true
            }))
            .is_err()
        );
        assert!(parse(json!({"bridge_id": "001788fffe012345"})).is_err());
        assert!(
            parse(json!({
                "bridge_id": "not-a-bridge-id",
                "application_key": "key"
            }))
            .is_err()
        );
        assert!(
            parse(json!({
                "bridge_id": "001788fffe012345",
                "application_key": ""
            }))
            .is_err()
        );
        assert!(
            parse(json!({
                "bridge_id": "001788fffe012345",
                "application_key": "key",
                "mdns": false
            }))
            .is_err()
        );
    }

    #[test]
    fn parses_supported_endpoint_forms_and_rejects_urls() {
        let cases = [
            ("192.0.2.10", "192.0.2.10", 443),
            ("192.0.2.10:8443", "192.0.2.10", 8443),
            ("bridge.example", "bridge.example", 443),
            ("bridge.example:9443", "bridge.example", 9443),
            ("2001:db8::1", "2001:db8::1", 443),
        ];
        for (value, expected_host, expected_port) in cases {
            let endpoint = Endpoint::parse(value).expect("valid endpoint");
            assert_eq!(endpoint.host(), expected_host);
            assert_eq!(endpoint.port(), expected_port);
        }

        for value in [
            "https://bridge.example",
            "bridge.example:0",
            "[bad]",
            "bad host",
        ] {
            assert!(Endpoint::parse(value).is_err(), "accepted {value}");
        }
    }
}
