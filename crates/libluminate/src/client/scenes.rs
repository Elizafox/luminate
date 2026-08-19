// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::{
    Client, Request, ResponseStatus, Result, Scene, SceneBinding, SceneCaptureMode, SceneId,
    SceneOutcome, TargetId, unexpected_response,
};

impl Client {
    /// Creates an explicitly-authored persistent scene.
    ///
    /// # Errors
    ///
    /// Returns a transport, authorization, validation, or persistence error.
    pub async fn create_scene(
        &self,
        name: String,
        description: Option<String>,
        bindings: Vec<SceneBinding>,
    ) -> Result<Scene> {
        self.expect_scene(Request::CreateScene {
            name,
            description,
            bindings,
        })
        .await
    }

    /// Captures intended state for selected concrete targets.
    ///
    /// # Errors
    ///
    /// Returns a transport, authorization, unknown-state, validation, or
    /// persistence error.
    pub async fn capture_scene(
        &self,
        name: String,
        description: Option<String>,
        mode: SceneCaptureMode,
        targets: Vec<TargetId>,
    ) -> Result<Scene> {
        self.expect_scene(Request::CaptureScene {
            name,
            description,
            mode,
            targets,
        })
        .await
    }

    /// Replaces an explicitly-authored scene at an expected revision.
    ///
    /// # Errors
    ///
    /// Returns a transport, authorization, revision, validation, or
    /// persistence error.
    pub async fn replace_scene(
        &self,
        id: SceneId,
        expected_revision: u64,
        name: String,
        description: Option<String>,
        bindings: Vec<SceneBinding>,
    ) -> Result<Scene> {
        self.expect_scene(Request::ReplaceScene {
            id,
            expected_revision,
            name,
            description,
            bindings,
        })
        .await
    }

    /// Recaptures intended state at an expected revision.
    ///
    /// # Errors
    ///
    /// Returns a transport, authorization, revision, unknown-state,
    /// validation, or persistence error.
    pub async fn recapture_scene(
        &self,
        id: SceneId,
        expected_revision: u64,
        mode: SceneCaptureMode,
        targets: Vec<TargetId>,
    ) -> Result<Scene> {
        self.expect_scene(Request::RecaptureScene {
            id,
            expected_revision,
            mode,
            targets,
        })
        .await
    }

    /// Deletes a scene at an expected revision.
    ///
    /// # Errors
    ///
    /// Returns a transport, authorization, revision, or persistence error.
    pub async fn delete_scene(&self, id: SceneId, expected_revision: u64) -> Result<()> {
        self.expect_ack(Request::DeleteScene {
            id,
            expected_revision,
        })
        .await
    }

    /// Lists every scene visible to the caller.
    ///
    /// # Errors
    ///
    /// Returns a transport or daemon error.
    #[allow(
        clippy::wildcard_enum_match_arm,
        reason = "every non-scene-list response is rejected identically"
    )]
    pub async fn list_scenes(&self) -> Result<Vec<Scene>> {
        match self.request(Request::ListScenes).await?.status {
            ResponseStatus::Scenes(scenes) => Ok(scenes),
            other => Err(unexpected_response("scene list", &other)),
        }
    }

    /// Looks up a visible scene by id.
    ///
    /// # Errors
    ///
    /// Returns a transport or daemon error.
    #[allow(
        clippy::wildcard_enum_match_arm,
        reason = "every non-scene-info response is rejected identically"
    )]
    pub async fn get_scene(&self, id: SceneId) -> Result<Option<Scene>> {
        match self.request(Request::GetScene { id }).await?.status {
            ResponseStatus::SceneInfo(scene) => Ok(scene.map(|scene| *scene)),
            other => Err(unexpected_response("scene lookup", &other)),
        }
    }

    /// Applies a scene immediately.
    ///
    /// # Errors
    ///
    /// Returns a transport, validation, hardware, or daemon error.
    #[allow(
        clippy::wildcard_enum_match_arm,
        reason = "every non-scene-application response is rejected identically"
    )]
    pub async fn apply_scene(&self, id: SceneId) -> Result<SceneOutcome> {
        self.apply_scene_authorized(id, None).await
    }

    #[allow(
        clippy::wildcard_enum_match_arm,
        reason = "every non-scene-application response is rejected identically"
    )]
    pub(crate) async fn apply_scene_authorized(
        &self,
        id: SceneId,
        authorized_targets: Option<Vec<TargetId>>,
    ) -> Result<SceneOutcome> {
        match self
            .request(Request::ApplyScene {
                id,
                authorized_targets,
            })
            .await?
            .status
        {
            ResponseStatus::SceneApplied { applied, denied } => {
                Ok(SceneOutcome { applied, denied })
            }
            other => Err(unexpected_response("scene application", &other)),
        }
    }

    #[allow(
        clippy::wildcard_enum_match_arm,
        reason = "every non-scene-mutation response is rejected identically"
    )]
    async fn expect_scene(&self, request: Request) -> Result<Scene> {
        match self.request(request).await?.status {
            ResponseStatus::Scene(scene) => Ok(*scene),
            other => Err(unexpected_response("scene mutation", &other)),
        }
    }
}
