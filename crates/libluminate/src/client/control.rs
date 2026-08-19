// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::{
    AppearanceSlotValue, Client, CollectionOutcome, Colour, Effect, EmissionState, Request, Result,
    Rgb, Selector, SetAppearanceSlotsRequest, SetBrightnessRequest, SetEffectRequest, TargetId,
    UnsupportedPolicy,
};

impl Client {
    /// Stores named appearance programs on one concrete surface.
    ///
    /// # Errors
    ///
    /// Returns an error if the target is not a surface, a slot is duplicated
    /// or unsupported, the mutation is incomplete, an effect is invalid, or
    /// the daemon or plugin cannot apply the operation.
    pub async fn set_appearance_slots(
        &self,
        target: TargetId,
        values: Vec<AppearanceSlotValue>,
    ) -> Result<()> {
        self.expect_ack(Request::SetAppearanceSlots(SetAppearanceSlotsRequest {
            target,
            values,
        }))
        .await
    }

    /// Apply an effect to `target`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed,
    /// [`Error::Unsupported`] or
    /// [`Error::InvalidArgument`] if the target rejects this effect, or
    /// another [`Error`] variant for an I/O/protocol failure.
    pub async fn set_effect(&self, target: TargetId, effect: Effect) -> Result<()> {
        self.set_effect_selector(Selector::Target(target), effect, None)
            .await
            .map(drop)
    }

    /// Apply an effect to a target or every device a collection covers.
    /// `on_unsupported` is meaningful only for a fan-out selector
    /// ([`Selector::Collection`]).
    ///
    /// # Errors
    ///
    /// Returns the same connection, validation, capability, and protocol
    /// errors as [`Self::set_effect`].
    pub async fn set_effect_selector(
        &self,
        selector: Selector,
        effect: Effect,
        on_unsupported: Option<UnsupportedPolicy>,
    ) -> Result<CollectionOutcome> {
        self.expect_collection_outcome(Request::SetEffect(SetEffectRequest {
            selector,
            effect,
            on_unsupported,
        }))
        .await
    }

    /// Sets `target` to one static colour.
    ///
    /// This is a convenience over [`Self::set_effect`], with the same static
    /// effect semantics and capability validation.
    ///
    /// # Errors
    ///
    /// Returns the same connection, validation, capability, and protocol
    /// errors as [`Self::set_effect`].
    pub async fn set_colour(&self, target: TargetId, colour: Colour) -> Result<()> {
        self.set_effect(target, Effect::Static { colour }).await
    }

    /// Sets a target or collection to one static colour.
    ///
    /// This is a convenience over [`Self::set_effect_selector`], with the same
    /// static effect semantics and capability validation.
    ///
    /// # Errors
    ///
    /// Returns the same connection, validation, capability, and protocol
    /// errors as [`Self::set_effect_selector`].
    pub async fn set_colour_selector(
        &self,
        selector: Selector,
        colour: Colour,
        on_unsupported: Option<UnsupportedPolicy>,
    ) -> Result<CollectionOutcome> {
        self.set_effect_selector(selector, Effect::Static { colour }, on_unsupported)
            .await
    }

    /// Sets `target` to one 8-bit RGB static colour.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::set_colour`].
    pub async fn set_rgb(&self, target: TargetId, r: u8, g: u8, b: u8) -> Result<()> {
        self.set_colour(target, Colour::rgb(Rgb::new(r, g, b)))
            .await
    }

    /// Sets a target or collection to one 8-bit RGB static colour.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::set_colour_selector`].
    pub async fn set_rgb_selector(
        &self,
        selector: Selector,
        r: u8,
        g: u8,
        b: u8,
        on_unsupported: Option<UnsupportedPolicy>,
    ) -> Result<CollectionOutcome> {
        self.set_colour_selector(selector, Colour::rgb(Rgb::new(r, g, b)), on_unsupported)
            .await
    }

    /// Sets `target` to one correlated colour temperature.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::set_colour`].
    pub async fn set_cct(&self, target: TargetId, kelvin: u32) -> Result<()> {
        self.set_colour(target, Colour::cct(kelvin)).await
    }

    /// Sets a target or collection to one correlated colour temperature.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::set_colour_selector`].
    pub async fn set_cct_selector(
        &self,
        selector: Selector,
        kelvin: u32,
        on_unsupported: Option<UnsupportedPolicy>,
    ) -> Result<CollectionOutcome> {
        self.set_colour_selector(selector, Colour::cct(kelvin), on_unsupported)
            .await
    }

    /// Set `target`'s brightness.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed,
    /// [`Error::Unsupported`] or
    /// [`Error::InvalidArgument`] if the target rejects this brightness
    /// value, or another [`Error`] variant for an I/O/protocol failure.
    pub async fn set_brightness(&self, target: TargetId, value: u32) -> Result<()> {
        self.set_brightness_selector(Selector::Target(target), value, None)
            .await
            .map(drop)
    }

    /// Set brightness on a target or every device a collection covers.
    /// `on_unsupported` is meaningful only for a fan-out selector
    /// ([`Selector::Collection`]).
    ///
    /// # Errors
    ///
    /// Returns the same connection, validation, capability, and protocol
    /// errors as [`Self::set_brightness`].
    pub async fn set_brightness_selector(
        &self,
        target: Selector,
        value: u32,
        on_unsupported: Option<UnsupportedPolicy>,
    ) -> Result<CollectionOutcome> {
        self.expect_collection_outcome(Request::SetBrightness(SetBrightnessRequest {
            target,
            value,
            on_unsupported,
        }))
        .await
    }

    /// Clear any state the daemon holds for `target`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed, or another [`Error`]
    /// variant if the request itself fails or the daemon responds with an
    /// error.
    pub async fn clear_target(&self, target: TargetId) -> Result<()> {
        self.clear_target_selector(Selector::Target(target))
            .await
            .map(drop)
    }

    /// Clear any state the daemon holds for a target or every device a
    /// collection covers.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::clear_target`].
    pub async fn clear_target_selector(&self, target: Selector) -> Result<CollectionOutcome> {
        self.expect_collection_outcome(Request::ClearTarget { target })
            .await
    }

    /// Persist `target`'s current hardware state so it can be restored on
    /// daemon startup.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed,
    /// [`Error::UnknownState`] if
    /// the target's current state can't be determined precisely enough to
    /// save, or another [`Error`] variant for an I/O/protocol failure.
    pub async fn save_current(&self, target: TargetId) -> Result<()> {
        self.save_current_selector(Selector::Target(target))
            .await
            .map(drop)
    }

    /// Persist the current hardware state of a target or every device a
    /// collection covers.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::save_current`].
    pub async fn save_current_selector(&self, target: Selector) -> Result<CollectionOutcome> {
        self.expect_collection_outcome(Request::SaveCurrent { target })
            .await
    }

    /// Turn `target` off. Shorthand for [`Self::set_effect`] with
    /// [`Effect::Off`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] if the connection has closed, or another [`Error`]
    /// variant if the request itself fails or the daemon responds with an
    /// error.
    pub async fn set_off(&self, target: TargetId) -> Result<()> {
        self.set_effect(target, Effect::Off).await
    }

    /// Reapply `target`'s last-known configured appearance.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownState`] if no usable appearance is known, or
    /// the same connection, validation, capability, conflict, and protocol
    /// errors as the corresponding colour or effect operation.
    pub async fn restore_appearance(&self, target: TargetId) -> Result<()> {
        self.restore_appearance_selector(Selector::Target(target))
            .await
            .map(drop)
    }

    /// Reapply the last-known configured appearance of a target or every
    /// authorized target a collection covers.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::restore_appearance`].
    pub async fn restore_appearance_selector(&self, target: Selector) -> Result<CollectionOutcome> {
        self.expect_collection_outcome(Request::RestoreAppearance { target })
            .await
    }

    /// Set whether `target` emits light.
    ///
    /// `Dark` turns the target off. `Emitting` reapplies its last-known
    /// configured appearance.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::set_off`] for `Dark` and
    /// [`Self::restore_appearance`] for `Emitting`.
    pub async fn set_emission(&self, target: TargetId, state: EmissionState) -> Result<()> {
        self.set_emission_selector(Selector::Target(target), state)
            .await
            .map(drop)
    }

    /// Set whether a target or every authorized target a collection covers
    /// emits light.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::set_emission`].
    pub async fn set_emission_selector(
        &self,
        target: Selector,
        state: EmissionState,
    ) -> Result<CollectionOutcome> {
        match state {
            EmissionState::Dark => self.set_effect_selector(target, Effect::Off, None).await,
            EmissionState::Emitting => self.restore_appearance_selector(target).await,
        }
    }
}
