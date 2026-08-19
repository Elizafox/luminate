// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Immediate lighting control REST resource.

use super::schemas::OutcomeSchema;
use super::{
    AuthenticatedClient, ControlRequest, Json, Response, Selector, empty_response, outcome_response,
};

#[utoipa::path(post, path = "/api/v0/control", request_body = ControlRequest, responses((status = 200, body = OutcomeSchema), (status = 204), (status = 401, body = crate::ProblemResponse), (status = 403, body = crate::ProblemResponse), (status = 422, body = crate::ProblemResponse)))]
pub(super) async fn control(
    session: AuthenticatedClient,
    Json(p): Json<ControlRequest>,
) -> Response {
    match p {
        ControlRequest::AppearanceSlots { target, values } => {
            empty_response(session.set_appearance_slots(target, values).await)
        }
        ControlRequest::Effect {
            selector,
            effect,
            on_unsupported,
        } => match selector {
            Selector::Target(target) => empty_response(session.set_effect(target, effect).await),
            selector @ (Selector::Collection(_) | Selector::Targets(_)) => outcome_response(
                session
                    .set_effect_selector(selector, effect, on_unsupported)
                    .await,
            ),
        },
        ControlRequest::Colour {
            selector,
            colour,
            on_unsupported,
        } => match selector {
            Selector::Target(target) => empty_response(session.set_colour(target, colour).await),
            selector @ (Selector::Collection(_) | Selector::Targets(_)) => outcome_response(
                session
                    .set_colour_selector(selector, colour, on_unsupported)
                    .await,
            ),
        },
        ControlRequest::Rgb {
            selector,
            rgb,
            on_unsupported,
        } => match selector {
            Selector::Target(target) => {
                empty_response(session.set_rgb(target, rgb.r, rgb.g, rgb.b).await)
            }
            selector @ (Selector::Collection(_) | Selector::Targets(_)) => outcome_response(
                session
                    .set_rgb_selector(selector, rgb.r, rgb.g, rgb.b, on_unsupported)
                    .await,
            ),
        },
        ControlRequest::Cct {
            selector,
            kelvin,
            on_unsupported,
        } => match selector {
            Selector::Target(target) => empty_response(session.set_cct(target, kelvin).await),
            selector @ (Selector::Collection(_) | Selector::Targets(_)) => outcome_response(
                session
                    .set_cct_selector(selector, kelvin, on_unsupported)
                    .await,
            ),
        },
        ControlRequest::Brightness {
            selector,
            value,
            on_unsupported,
        } => match selector {
            Selector::Target(target) => empty_response(session.set_brightness(target, value).await),
            selector @ (Selector::Collection(_) | Selector::Targets(_)) => outcome_response(
                session
                    .set_brightness_selector(selector, value, on_unsupported)
                    .await,
            ),
        },
        ControlRequest::Clear { selector } => match selector {
            Selector::Target(target) => empty_response(session.clear_target(target).await),
            selector @ (Selector::Collection(_) | Selector::Targets(_)) => {
                outcome_response(session.clear_target_selector(selector).await)
            }
        },
        ControlRequest::SaveCurrent { selector } => match selector {
            Selector::Target(target) => empty_response(session.save_current(target).await),
            selector @ (Selector::Collection(_) | Selector::Targets(_)) => {
                outcome_response(session.save_current_selector(selector).await)
            }
        },
        ControlRequest::Off { target } => empty_response(session.set_off(target).await),
        ControlRequest::RestoreAppearance { selector } => match selector {
            Selector::Target(target) => empty_response(session.restore_appearance(target).await),
            selector @ (Selector::Collection(_) | Selector::Targets(_)) => {
                outcome_response(session.restore_appearance_selector(selector).await)
            }
        },
        ControlRequest::Emission { selector, state } => match selector {
            Selector::Target(target) => empty_response(session.set_emission(target, state).await),
            selector @ (Selector::Collection(_) | Selector::Targets(_)) => {
                outcome_response(session.set_emission_selector(selector, state).await)
            }
        },
    }
}
