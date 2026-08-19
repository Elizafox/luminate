// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Sanitized plugin-setup dictionaries and interaction requests.

use luminate::{
    PluginSetupInteractionResponse, PluginSetupSession, PluginSetupSessionState,
    PluginSetupWorkflow, PluginSetupWorkflowKind,
};
use zbus::zvariant::{DeserializeDict, Type};

use crate::error::MethodError;
use crate::management::{Dictionary, dictionary, owned};

#[derive(Debug, DeserializeDict, Type)]
#[zvariant(signature = "a{sv}", rename_all = "PascalCase", deny_unknown_fields)]
pub(crate) struct InteractionResponse {
    kind: String,
    choice: Option<String>,
}

impl InteractionResponse {
    pub(crate) fn into_response(self) -> Result<PluginSetupInteractionResponse, MethodError> {
        match self.kind.as_str() {
            "choice" => Ok(PluginSetupInteractionResponse::Choice(
                self.choice
                    .ok_or_else(|| invalid("choice response is missing Choice"))?,
            )),
            "confirmed" => {
                if self.choice.is_some() {
                    return Err(invalid("confirmed response must not contain Choice"));
                }
                Ok(PluginSetupInteractionResponse::Confirmed)
            }
            value => Err(invalid(format!("unknown setup response kind {value:?}"))),
        }
    }
}

pub(crate) fn workflows(values: Vec<PluginSetupWorkflow>) -> Result<Vec<Dictionary>, MethodError> {
    values.into_iter().map(workflow).collect()
}

fn workflow(value: PluginSetupWorkflow) -> Result<Dictionary, MethodError> {
    let kind = match value.kind {
        PluginSetupWorkflowKind::Provision => "provision",
        PluginSetupWorkflowKind::Repair => "repair",
        PluginSetupWorkflowKind::Discover => "discover",
        PluginSetupWorkflowKind::Import => "import",
        PluginSetupWorkflowKind::FactoryProvision => "factory-provision",
        _ => return Err(invalid("unknown plugin setup workflow kind")),
    };
    Ok(dictionary([
        ("Plugin", owned(value.plugin)?),
        ("Id", owned(value.id)?),
        ("Label", owned(value.label)?),
        ("Description", owned(value.description)?),
        ("Kind", owned(kind)?),
    ]))
}

pub(crate) fn session(value: PluginSetupSession) -> Result<Dictionary, MethodError> {
    let mut result = dictionary([
        ("Id", owned(value.id.as_str().to_owned())?),
        ("Plugin", owned(value.plugin)?),
        ("Workflow", owned(value.workflow)?),
        ("Generation", owned(value.generation)?),
    ]);
    let state = match value.state {
        PluginSetupSessionState::Choice { prompt, choices } => dictionary([
            ("Kind", owned("choice")?),
            ("Prompt", owned(prompt)?),
            (
                "Choices",
                owned(
                    choices
                        .into_iter()
                        .map(|choice| {
                            let mut value = dictionary([
                                ("Id", owned(choice.id)?),
                                ("Label", owned(choice.label)?),
                                ("HasDescription", owned(choice.description.is_some())?),
                            ]);
                            if let Some(description) = choice.description {
                                value.insert("Description".into(), owned(description)?);
                            }
                            Ok(value)
                        })
                        .collect::<Result<Vec<_>, MethodError>>()?,
                )?,
            ),
        ]),
        PluginSetupSessionState::PhysicalAction { instruction } => dictionary([
            ("Kind", owned("physical-action")?),
            ("Instruction", owned(instruction)?),
        ]),
        PluginSetupSessionState::Applying => dictionary([("Kind", owned("applying")?)]),
        PluginSetupSessionState::Completed { summary, revision } => dictionary([
            ("Kind", owned("completed")?),
            ("Summary", owned(summary)?),
            ("Revision", owned(revision)?),
        ]),
        PluginSetupSessionState::Failed { diagnostic } => dictionary([
            ("Kind", owned("failed")?),
            ("Diagnostic", owned(diagnostic)?),
        ]),
        PluginSetupSessionState::Cancelled => dictionary([("Kind", owned("cancelled")?)]),
    };
    result.insert("State".into(), owned(state)?);
    Ok(result)
}

fn invalid(message: impl Into<String>) -> MethodError {
    MethodError::InvalidArgument(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use luminate::{PluginSetupChoice, PluginSetupSessionId};

    #[test]
    fn interaction_responses_are_strict() {
        assert!(matches!(
            InteractionResponse {
                kind: "choice".into(),
                choice: Some("bridge".into())
            }
            .into_response()
            .expect("choice response"),
            PluginSetupInteractionResponse::Choice(value) if value == "bridge"
        ));
        assert!(
            InteractionResponse {
                kind: "confirmed".into(),
                choice: Some("unexpected".into())
            }
            .into_response()
            .is_err()
        );
    }

    #[test]
    fn workflow_and_session_outputs_cover_every_current_variant() {
        let workflows = workflows(vec![
            PluginSetupWorkflow::new(
                "plugin",
                "provision",
                "Provision",
                "",
                PluginSetupWorkflowKind::Provision,
            ),
            PluginSetupWorkflow::new(
                "plugin",
                "repair",
                "Repair",
                "",
                PluginSetupWorkflowKind::Repair,
            ),
            PluginSetupWorkflow::new(
                "plugin",
                "discover",
                "Discover",
                "",
                PluginSetupWorkflowKind::Discover,
            ),
            PluginSetupWorkflow::new(
                "plugin",
                "import",
                "Import",
                "",
                PluginSetupWorkflowKind::Import,
            ),
            PluginSetupWorkflow::new(
                "plugin",
                "factory",
                "Factory",
                "",
                PluginSetupWorkflowKind::FactoryProvision,
            ),
        ])
        .expect("encode workflows");
        assert_eq!(workflows.len(), 5);

        let id = || PluginSetupSessionId::parse("0123456789abcdef0123456789abcdef").expect("id");
        let states = vec![
            PluginSetupSessionState::Choice {
                prompt: "Choose".into(),
                choices: vec![PluginSetupChoice {
                    id: "one".into(),
                    label: "One".into(),
                    description: None,
                }],
            },
            PluginSetupSessionState::PhysicalAction {
                instruction: "Press the button".into(),
            },
            PluginSetupSessionState::Applying,
            PluginSetupSessionState::Completed {
                summary: "Done".into(),
                revision: 7,
            },
            PluginSetupSessionState::Failed {
                diagnostic: "No bridge".into(),
            },
            PluginSetupSessionState::Cancelled,
        ];
        for state in states {
            let encoded = session(PluginSetupSession {
                id: id(),
                plugin: "plugin".into(),
                workflow: "workflow".into(),
                generation: 2,
                state,
            })
            .expect("encode session");
            assert!(encoded.contains_key("State"));
        }
    }
}
