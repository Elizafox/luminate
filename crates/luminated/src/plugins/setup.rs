// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Daemon-owned state for interactive plugin setup sessions.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use luminate_platform::secure_random::fill_bytes;
use luminate_plugin_api::{PluginSetupInteraction, PluginSetupRequest, PluginSetupResponse};
use luminate_protocol::{
    PluginSetupChoice, PluginSetupInteractionResponse, PluginSetupSession, PluginSetupSessionId,
    PluginSetupSessionState,
};

use super::PluginManager;
use crate::authorization::RateLimitKey;

const SESSION_LIFETIME: Duration = Duration::from_secs(10 * 60);
const MAX_ACTIVE_SESSIONS: usize = 64;

pub(super) struct SetupSessions {
    entries: HashMap<PluginSetupSessionId, SetupSessionRecord>,
}

#[derive(Clone)]
struct SetupSessionRecord {
    snapshot: PluginSetupSession,
    owner: RateLimitKey,
    path: PathBuf,
    continuation: Vec<u8>,
    expected: ExpectedResponse,
    expires_at: Instant,
    in_flight: bool,
    baseline_settings: toml::Table,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpectedResponse {
    Choice,
    Confirmation,
}

pub(crate) struct PreparedSetupStep {
    pub(crate) session: PluginSetupSessionId,
    pub(crate) path: PathBuf,
    pub(crate) request: PluginSetupRequest,
}

impl SetupSessions {
    pub(super) fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    fn expire(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, entry| entry.expires_at > now);
    }
}

impl SetupSessionRecord {
    fn is_active(&self) -> bool {
        self.in_flight
            || matches!(
                self.snapshot.state,
                PluginSetupSessionState::Choice { .. }
                    | PluginSetupSessionState::PhysicalAction { .. }
                    | PluginSetupSessionState::Applying
            )
    }
}

#[allow(
    clippy::expect_used,
    clippy::unwrap_in_result,
    reason = "a poisoned plugin catalogue or setup-session lock is an existing daemon process invariant"
)]
impl PluginManager {
    /// Prepares the initial isolated step for an advertised setup workflow.
    pub fn prepare_setup_start(
        &self,
        plugin: &str,
        workflow: &str,
        owner: RateLimitKey,
        baseline_settings: toml::Table,
    ) -> Result<PreparedSetupStep> {
        let path = {
            #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
            let catalogue = self
                .catalogue
                .read()
                .expect("plugin catalogue lock poisoned");
            let entry = catalogue
                .iter()
                .find(|entry| entry.name == plugin)
                .with_context(|| format!("installed plugin not found: {plugin}"))?;
            anyhow::ensure!(
                entry
                    .setup_workflows
                    .iter()
                    .any(|entry| entry.id == workflow),
                "plugin {plugin} does not advertise setup workflow {workflow}"
            );
            entry.path.clone()
        };
        #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
        let mut sessions = self
            .setup_sessions
            .lock()
            .expect("setup session lock poisoned");
        sessions.expire();
        anyhow::ensure!(
            sessions
                .entries
                .values()
                .filter(|entry| entry.is_active())
                .count()
                < MAX_ACTIVE_SESSIONS,
            "too many active plugin setup sessions"
        );
        anyhow::ensure!(
            sessions
                .entries
                .values()
                .all(|entry| entry.snapshot.plugin != plugin || !entry.is_active()),
            "plugin {plugin} already has an active setup session"
        );
        let session = loop {
            let candidate = new_session_id()?;
            if !sessions.entries.contains_key(&candidate) {
                break candidate;
            }
        };
        let snapshot = PluginSetupSession {
            id: session.clone(),
            plugin: plugin.to_owned(),
            workflow: workflow.to_owned(),
            generation: 0,
            state: PluginSetupSessionState::Failed {
                diagnostic: "setup has not produced its initial interaction".to_owned(),
            },
        };
        sessions.entries.insert(
            session.clone(),
            SetupSessionRecord {
                snapshot,
                owner,
                path: path.clone(),
                continuation: Vec::new(),
                expected: ExpectedResponse::Confirmation,
                expires_at: Instant::now() + SESSION_LIFETIME,
                in_flight: true,
                baseline_settings,
            },
        );
        Ok(PreparedSetupStep {
            session,
            path,
            request: PluginSetupRequest {
                workflow: workflow.to_owned(),
                continuation: Vec::new(),
                response: None,
            },
        })
    }

    /// Records an interaction returned by an isolated setup step.
    pub fn finish_setup_interaction(
        &self,
        session: &PluginSetupSessionId,
        continuation: Vec<u8>,
        interaction: PluginSetupInteraction,
    ) -> Result<PluginSetupSession> {
        #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
        let mut sessions = self
            .setup_sessions
            .lock()
            .expect("setup session lock poisoned");
        let entry = sessions
            .entries
            .get_mut(session)
            .context("setup session expired")?;
        if matches!(entry.snapshot.state, PluginSetupSessionState::Cancelled) {
            return Ok(entry.snapshot.clone());
        }
        let (state, expected) = public_interaction(interaction)?;
        entry.snapshot.generation = entry.snapshot.generation.saturating_add(1);
        entry.snapshot.state = state;
        entry.continuation = continuation;
        entry.expected = expected;
        entry.expires_at = Instant::now() + SESSION_LIFETIME;
        entry.in_flight = false;
        Ok(entry.snapshot.clone())
    }

    /// Validates and prepares a response to the current interaction.
    pub fn prepare_setup_response(
        &self,
        session: &PluginSetupSessionId,
        generation: u64,
        response: PluginSetupInteractionResponse,
        owner: &RateLimitKey,
    ) -> Result<PreparedSetupStep> {
        #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
        let mut sessions = self
            .setup_sessions
            .lock()
            .expect("setup session lock poisoned");
        sessions.expire();
        let entry = sessions
            .entries
            .get_mut(session)
            .context("setup session not found")?;
        anyhow::ensure!(
            &entry.owner == owner,
            "setup session belongs to another actor"
        );
        anyhow::ensure!(
            !entry.in_flight,
            "setup session already has a step in progress"
        );
        anyhow::ensure!(
            entry.snapshot.generation == generation,
            "stale setup generation"
        );
        let response = match (entry.expected, response) {
            (ExpectedResponse::Choice, PluginSetupInteractionResponse::Choice(choice)) => {
                PluginSetupResponse::Choice(choice)
            }
            (ExpectedResponse::Confirmation, PluginSetupInteractionResponse::Confirmed) => {
                PluginSetupResponse::Confirmed
            }
            _ => anyhow::bail!("response does not match the current setup interaction"),
        };
        anyhow::ensure!(
            matches!(
                entry.snapshot.state,
                PluginSetupSessionState::Choice { .. }
                    | PluginSetupSessionState::PhysicalAction { .. }
            ),
            "setup session is already terminal"
        );
        entry.in_flight = true;
        Ok(PreparedSetupStep {
            session: session.clone(),
            path: entry.path.clone(),
            request: PluginSetupRequest {
                workflow: entry.snapshot.workflow.clone(),
                continuation: entry.continuation.clone(),
                response: Some(response),
            },
        })
    }

    /// Returns an actor-owned setup session snapshot.
    pub fn setup_session(
        &self,
        session: &PluginSetupSessionId,
        owner: &RateLimitKey,
    ) -> Result<PluginSetupSession> {
        #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
        let mut sessions = self
            .setup_sessions
            .lock()
            .expect("setup session lock poisoned");
        sessions.expire();
        let entry = sessions
            .entries
            .get(session)
            .context("setup session not found")?;
        anyhow::ensure!(
            &entry.owner == owner,
            "setup session belongs to another actor"
        );
        Ok(entry.snapshot.clone())
    }

    /// Cancels an actor-owned setup session.
    pub fn cancel_setup(
        &self,
        session: &PluginSetupSessionId,
        owner: &RateLimitKey,
    ) -> Result<PluginSetupSession> {
        #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
        let mut sessions = self
            .setup_sessions
            .lock()
            .expect("setup session lock poisoned");
        sessions.expire();
        let entry = sessions
            .entries
            .get_mut(session)
            .context("setup session not found")?;
        anyhow::ensure!(
            &entry.owner == owner,
            "setup session belongs to another actor"
        );
        if !matches!(
            entry.snapshot.state,
            PluginSetupSessionState::Applying
                | PluginSetupSessionState::Completed { .. }
                | PluginSetupSessionState::Failed { .. }
        ) {
            entry.snapshot.generation = entry.snapshot.generation.saturating_add(1);
            entry.snapshot.state = PluginSetupSessionState::Cancelled;
            entry.continuation.clear();
        }
        Ok(entry.snapshot.clone())
    }

    pub(crate) fn begin_setup_commit(
        &self,
        session: &PluginSetupSessionId,
    ) -> Result<Option<(PluginSetupSession, toml::Table)>> {
        #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
        let mut sessions = self
            .setup_sessions
            .lock()
            .expect("setup session lock poisoned");
        let entry = sessions
            .entries
            .get_mut(session)
            .context("setup session not found")?;
        if matches!(entry.snapshot.state, PluginSetupSessionState::Cancelled) {
            entry.in_flight = false;
            return Ok(None);
        }
        anyhow::ensure!(entry.in_flight, "setup session has no completed step");
        entry.snapshot.generation = entry.snapshot.generation.saturating_add(1);
        entry.snapshot.state = PluginSetupSessionState::Applying;
        Ok(Some((
            entry.snapshot.clone(),
            entry.baseline_settings.clone(),
        )))
    }

    pub(crate) fn finish_setup_terminal(
        &self,
        session: &PluginSetupSessionId,
        state: PluginSetupSessionState,
    ) -> Result<PluginSetupSession> {
        #[allow(clippy::expect_used, reason = "Acceptable to panic on poisoned locks")]
        let mut sessions = self
            .setup_sessions
            .lock()
            .expect("setup session lock poisoned");
        let entry = sessions
            .entries
            .get_mut(session)
            .context("setup session not found")?;
        if !matches!(entry.snapshot.state, PluginSetupSessionState::Cancelled) {
            entry.snapshot.generation = entry.snapshot.generation.saturating_add(1);
            entry.snapshot.state = state;
        }
        entry.continuation.clear();
        entry.in_flight = false;
        Ok(entry.snapshot.clone())
    }
}

fn public_interaction(
    interaction: PluginSetupInteraction,
) -> Result<(PluginSetupSessionState, ExpectedResponse)> {
    match interaction {
        PluginSetupInteraction::Choice { prompt, choices } => {
            anyhow::ensure!(
                valid_public_text(&prompt, 2_048),
                "setup choice prompt is invalid"
            );
            anyhow::ensure!(
                !choices.is_empty() && choices.len() <= 64,
                "invalid setup choices"
            );
            let mut ids = HashSet::new();
            let choices = choices
                .into_iter()
                .map(|choice| {
                    anyhow::ensure!(
                        !choice.id.is_empty()
                            && choice.id.len() <= 128
                            && choice.id.bytes().all(|byte| {
                                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                            }),
                        "invalid setup choice ID"
                    );
                    anyhow::ensure!(ids.insert(choice.id.clone()), "duplicate setup choice ID");
                    anyhow::ensure!(
                        valid_public_text(&choice.label, 256),
                        "setup choice label is invalid"
                    );
                    anyhow::ensure!(
                        choice
                            .description
                            .as_deref()
                            .is_none_or(|value| valid_public_text(value, 2_048)),
                        "setup choice description is invalid"
                    );
                    Ok(PluginSetupChoice {
                        id: choice.id,
                        label: choice.label,
                        description: choice.description,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((
                PluginSetupSessionState::Choice { prompt, choices },
                ExpectedResponse::Choice,
            ))
        }
        PluginSetupInteraction::PhysicalAction { instruction } => {
            anyhow::ensure!(
                valid_public_text(&instruction, 2_048),
                "setup instruction is invalid"
            );
            Ok((
                PluginSetupSessionState::PhysicalAction { instruction },
                ExpectedResponse::Confirmation,
            ))
        }
    }
}

pub(crate) fn valid_public_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}

fn new_session_id() -> Result<PluginSetupSessionId> {
    let mut random = [0_u8; 16];
    fill_bytes(&mut random).context("generating plugin setup session identifier")?;
    let mut value = String::with_capacity(32);
    for byte in random {
        write!(&mut value, "{byte:02x}").context("formatting plugin setup session identifier")?;
    }
    PluginSetupSessionId::parse(value).map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_choice_interactions_remain_typed_and_bounded() {
        let (state, expected) = public_interaction(PluginSetupInteraction::Choice {
            prompt: "Choose hardware.".to_owned(),
            choices: vec![luminate_plugin_api::PluginSetupChoice {
                id: "first".to_owned(),
                label: "First device".to_owned(),
                description: None,
            }],
        })
        .expect("valid choice interaction");
        assert_eq!(expected, ExpectedResponse::Choice);
        assert!(matches!(
            state,
            PluginSetupSessionState::Choice { choices, .. } if choices[0].id == "first"
        ));

        assert!(
            public_interaction(PluginSetupInteraction::Choice {
                prompt: "Choose hardware.".to_owned(),
                choices: Vec::new(),
            })
            .is_err()
        );
    }

    #[test]
    fn generated_session_identifiers_are_canonical() {
        let first = new_session_id().expect("generate first session ID");
        let second = new_session_id().expect("generate second session ID");
        assert_eq!(first.as_str().len(), 32);
        assert_ne!(first, second);
        assert!(first.as_str().bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}
