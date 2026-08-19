// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Push-link provisioning for one locally discovered Hue Bridge.

use std::collections::{BTreeMap, HashSet};

use luminate_plugin_api::{
    PluginError, PluginSetupChoice, PluginSetupInteraction, PluginSetupRequest,
    PluginSetupResponse, PluginSetupSettingValue, PluginSetupStep, decode_cbor, encode_cbor,
};
use serde::{Deserialize, Serialize};

use crate::configuration::{BridgeId, Endpoint};
use crate::{api, http, mdns};

pub(crate) const WORKFLOW_ID: &str = "push-link";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SetupBridge {
    bridge_id: String,
    endpoint: String,
    model_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Continuation {
    Choose(Vec<SetupBridge>),
    Pair(SetupBridge),
}

pub(crate) fn run(request: PluginSetupRequest) -> Result<PluginSetupStep, PluginError> {
    if request.workflow != WORKFLOW_ID {
        return Err(PluginError::InvalidArgument(format!(
            "unknown Hue setup workflow: {}",
            request.workflow
        )));
    }
    match (request.continuation.is_empty(), request.response) {
        (true, None) => discover(),
        (false, Some(response)) => advance(&request.continuation, response),
        _ => Err(PluginError::InvalidArgument(
            "Hue setup request is out of sequence".to_owned(),
        )),
    }
}

fn discover() -> Result<PluginSetupStep, PluginError> {
    let discovered = mdns::discover_all()
        .map_err(|error| PluginError::Io(format!("Hue Bridge discovery failed: {error}")))?;
    let mut ids = HashSet::new();
    let bridges = discovered
        .into_iter()
        .filter(|bridge| ids.insert(bridge.bridge_id.as_str().to_owned()))
        .map(|bridge| SetupBridge {
            bridge_id: bridge.bridge_id.as_str().to_owned(),
            endpoint: bridge.endpoint.authority(),
            model_id: bridge.model_id,
        })
        .collect::<Vec<_>>();
    match bridges.as_slice() {
        [] => Err(PluginError::Unavailable(
            "no Hue Bridges were discovered on the local network".to_owned(),
        )),
        [bridge] => physical_action(bridge.clone()),
        _ => {
            let choices = bridges
                .iter()
                .map(|bridge| PluginSetupChoice {
                    id: bridge.bridge_id.clone(),
                    label: bridge.model_id.as_ref().map_or_else(
                        || bridge.bridge_id.clone(),
                        |model| format!("{model} ({})", bridge.bridge_id),
                    ),
                    description: Some(format!("Hue Bridge at {}", bridge.endpoint)),
                })
                .collect();
            Ok(PluginSetupStep::Interaction {
                continuation: encode(&Continuation::Choose(bridges))?,
                interaction: PluginSetupInteraction::Choice {
                    prompt: "Choose the Hue Bridge to connect.".to_owned(),
                    choices,
                },
            })
        }
    }
}

fn advance(
    continuation: &[u8],
    response: PluginSetupResponse,
) -> Result<PluginSetupStep, PluginError> {
    let continuation: Continuation = decode_cbor(continuation).map_err(|error| {
        PluginError::InvalidArgument(format!("invalid Hue setup state: {error}"))
    })?;
    match (continuation, response) {
        (Continuation::Choose(bridges), PluginSetupResponse::Choice(choice)) => {
            let bridge = bridges
                .into_iter()
                .find(|bridge| bridge.bridge_id == choice)
                .ok_or_else(|| {
                    PluginError::InvalidArgument("unknown Hue Bridge choice".to_owned())
                })?;
            physical_action(bridge)
        }
        (Continuation::Pair(bridge), PluginSetupResponse::Confirmed) => pair(&bridge),
        _ => Err(PluginError::InvalidArgument(
            "response does not match the Hue setup step".to_owned(),
        )),
    }
}

fn physical_action(bridge: SetupBridge) -> Result<PluginSetupStep, PluginError> {
    Ok(PluginSetupStep::Interaction {
        continuation: encode(&Continuation::Pair(bridge))?,
        interaction: PluginSetupInteraction::PhysicalAction {
            instruction: "Press the round link button on the selected Hue Bridge, then confirm within 30 seconds."
                .to_owned(),
        },
    })
}

fn pair(bridge: &SetupBridge) -> Result<PluginSetupStep, PluginError> {
    let bridge_id = BridgeId::parse(&bridge.bridge_id)
        .map_err(|error| PluginError::InvalidArgument(error.to_string()))?;
    let endpoint = Endpoint::parse(&bridge.endpoint)
        .map_err(|error| PluginError::InvalidArgument(error.to_string()))?;
    let application_key = http::create_application_key(&bridge_id, &endpoint)
        .map_err(|error| PluginError::Unavailable(error.to_string()))?;
    let client = http::HttpClient::new(bridge_id.clone(), application_key.clone())
        .map_err(|error| PluginError::Internal(error.to_string()))?;
    api::enumerate(&client, &endpoint, &bridge_id).map_err(|error| {
        PluginError::Unavailable(format!("paired key verification failed: {error}"))
    })?;

    let mut settings = BTreeMap::new();
    settings.insert(
        "bridge_id".to_owned(),
        PluginSetupSettingValue::String(bridge_id.as_str().to_owned()),
    );
    settings.insert(
        "application_key".to_owned(),
        PluginSetupSettingValue::String(application_key.expose().to_owned()),
    );
    settings.insert("mdns".to_owned(), PluginSetupSettingValue::Boolean(true));
    Ok(PluginSetupStep::Complete {
        settings,
        summary: completion_summary(&bridge_id),
    })
}

fn completion_summary(bridge_id: &BridgeId) -> String {
    format!(
        "Connected Hue Bridge {}. If you later disconnect or replace this credential, \
        revoke the old Luminate authorization manually through Philips Hue \
        application management.",
        bridge_id.as_str()
    )
}

fn encode(value: &Continuation) -> Result<Vec<u8>, PluginError> {
    encode_cbor(value)
        .map_err(|error| PluginError::Internal(format!("encoding Hue setup state: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_choice_advances_to_a_physical_action() {
        let bridges = vec![
            SetupBridge {
                bridge_id: "001788fffe111111".to_owned(),
                endpoint: "192.0.2.10:443".to_owned(),
                model_id: Some("BSB002".to_owned()),
            },
            SetupBridge {
                bridge_id: "001788fffe222222".to_owned(),
                endpoint: "192.0.2.20:443".to_owned(),
                model_id: Some("BSB002".to_owned()),
            },
        ];
        let step = advance(
            &encode(&Continuation::Choose(bridges)).expect("encode continuation"),
            PluginSetupResponse::Choice("001788fffe222222".to_owned()),
        )
        .expect("select discovered bridge");
        let PluginSetupStep::Interaction {
            continuation,
            interaction: PluginSetupInteraction::PhysicalAction { instruction },
        } = step
        else {
            panic!("choice should request a physical action");
        };
        assert!(instruction.contains("link button"));
        let continuation: Continuation = decode_cbor(&continuation).expect("decode continuation");
        assert!(matches!(
            continuation,
            Continuation::Pair(bridge) if bridge.bridge_id == "001788fffe222222"
        ));
    }

    #[test]
    fn rejects_a_choice_that_was_not_discovered() {
        let continuation = Continuation::Choose(vec![SetupBridge {
            bridge_id: "001788fffe111111".to_owned(),
            endpoint: "192.0.2.10:443".to_owned(),
            model_id: None,
        }]);
        assert!(
            advance(
                &encode(&continuation).expect("encode continuation"),
                PluginSetupResponse::Choice("001788fffe222222".to_owned()),
            )
            .is_err()
        );
    }

    #[test]
    fn completion_explains_manual_credential_revocation() {
        let bridge_id = BridgeId::parse("001788fffe111111").expect("valid bridge ID");
        let summary = completion_summary(&bridge_id);

        assert!(summary.contains(bridge_id.as_str()));
        assert!(summary.contains("revoke the old Luminate authorization manually"));
        assert!(summary.contains("Philips Hue application management"));
    }
}
