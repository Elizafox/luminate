// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::{
    Client, Duration, Error, Request, ResponseStatus, Result, SceneId, StartTransitionRequest,
    TransitionDestination, TransitionId, TransitionOptions, TransitionSource, TransitionStatus,
    TransitionTargetState, sleep_for, unexpected_response,
};

/// Transition operations associated with one connected client.
#[derive(Debug, Clone, Copy)]
pub struct Transitions<'client> {
    pub(super) client: &'client Client,
}

impl Transitions<'_> {
    async fn start(
        self,
        source: TransitionSource,
        destination: TransitionDestination,
        options: TransitionOptions,
    ) -> Result<TransitionStatus> {
        self.expect_status(Request::StartTransition(StartTransitionRequest {
            source,
            destination,
            options,
            authorized_targets: None,
            renewable_lease_ms: None,
        }))
        .await
    }

    /// Starts a scene-to-scene transition.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TransitionImpossible`] before hardware mutation when
    /// endpoint preflight fails, or another daemon or transport error.
    pub async fn scene_to_scene(
        self,
        source: SceneId,
        destination: SceneId,
        options: TransitionOptions,
    ) -> Result<TransitionStatus> {
        self.start(
            TransitionSource::Scene(source),
            TransitionDestination::Scene(destination),
            options,
        )
        .await
    }

    /// Starts a transition from exact fresh current state to a scene.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::scene_to_scene`].
    pub async fn current_to_scene(
        self,
        destination: SceneId,
        options: TransitionOptions,
    ) -> Result<TransitionStatus> {
        self.start(
            TransitionSource::Current,
            TransitionDestination::Scene(destination),
            options,
        )
        .await
    }

    /// Starts a scene-to-ephemeral-state transition.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::scene_to_scene`].
    pub async fn scene_to_states(
        self,
        source: SceneId,
        destination: Vec<TransitionTargetState>,
        options: TransitionOptions,
    ) -> Result<TransitionStatus> {
        self.start(
            TransitionSource::Scene(source),
            TransitionDestination::TargetStates(destination),
            options,
        )
        .await
    }

    /// Starts a current-to-ephemeral-state transition.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::scene_to_scene`].
    pub async fn current_to_states(
        self,
        destination: Vec<TransitionTargetState>,
        options: TransitionOptions,
    ) -> Result<TransitionStatus> {
        self.start(
            TransitionSource::Current,
            TransitionDestination::TargetStates(destination),
            options,
        )
        .await
    }

    /// Fetches an active or retained terminal transition.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] after a daemon restart or terminal-status
    /// eviction, or another daemon or transport error.
    pub async fn get(self, id: TransitionId) -> Result<TransitionStatus> {
        self.expect_status(Request::GetTransition { id }).await
    }

    /// Aborts a transition and waits until no later step can write.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] for an unknown transition, or another
    /// daemon or transport error.
    pub async fn abort(self, id: TransitionId) -> Result<TransitionStatus> {
        self.expect_status(Request::AbortTransition { id }).await
    }

    /// Waits for a terminal transition snapshot.
    ///
    /// The polling interval is intentionally modest; transition dirty-bit
    /// events remain the efficient way to coordinate many transitions.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::get`].
    pub async fn wait(self, id: TransitionId) -> Result<TransitionStatus> {
        loop {
            let status = self.get(id.clone()).await?;
            if status.is_terminal() {
                return Ok(status);
            }
            sleep_for(Duration::from_millis(25)).await;
        }
    }

    #[allow(
        clippy::wildcard_enum_match_arm,
        reason = "every non-transition response is the same protocol-shape error"
    )]
    async fn expect_status(self, request: Request) -> Result<TransitionStatus> {
        match self.client.request(request).await?.status {
            ResponseStatus::Transition(status) => Ok(*status),
            ResponseStatus::Error(error) => Err(Error::from_operation_error(error)),
            other => Err(unexpected_response("transition status", &other)),
        }
    }
}
