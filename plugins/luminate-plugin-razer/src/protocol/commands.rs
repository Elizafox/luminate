// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford
// SPDX-FileCopyrightText: 2015 Terri Cain <terri@dolphincorp.co.uk>

//! Typed construction of common Razer protocol commands.

use std::error::Error;
use std::fmt;

use super::report::{CommandId, Report, ReportError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResponsePolicy {
    Required,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Command {
    report: Report,
    response_policy: ResponsePolicy,
    idempotent: bool,
}

impl Command {
    #[cfg(test)]
    pub(crate) fn serial(transaction_id: u8) -> Result<Self, ReportError> {
        Self::get(transaction_id, 0x00, 0x02, 0x16)
    }

    #[cfg(test)]
    pub(crate) fn firmware_version(transaction_id: u8) -> Result<Self, ReportError> {
        Self::get(transaction_id, 0x00, 0x01, 0x02)
    }

    pub(crate) fn extended_matrix_effect(
        transaction_id: u8,
        storage: Storage,
        target: LedTarget,
        effect: MatrixEffect,
    ) -> Result<Self, ReportError> {
        let arguments = match effect {
            MatrixEffect::Static(colour) => vec![
                storage.to_wire(),
                target.to_wire(),
                effect.to_wire(),
                0,
                0,
                1,
                colour.red,
                colour.green,
                colour.blue,
            ],
            MatrixEffect::Wave(direction) => vec![
                storage.to_wire(),
                target.to_wire(),
                0x04,
                direction.to_wire(),
                0x28,
                0,
            ],
            MatrixEffect::Wheel(direction) => vec![
                storage.to_wire(),
                target.to_wire(),
                0x0a,
                direction.to_wire(),
                0x28,
                0,
            ],
            MatrixEffect::Reactive { speed, colour } => vec![
                storage.to_wire(),
                target.to_wire(),
                0x05,
                0,
                speed.to_wire(),
                1,
                colour.red,
                colour.green,
                colour.blue,
            ],
            MatrixEffect::Breathing(colours) => {
                colours.arguments(storage.to_wire(), target.to_wire(), 0x02, None)
            }
            MatrixEffect::Starlight { speed, colours } => colours.arguments(
                storage.to_wire(),
                target.to_wire(),
                0x07,
                Some(speed.to_wire()),
            ),
            MatrixEffect::Off | MatrixEffect::Spectrum => {
                vec![
                    storage.to_wire(),
                    target.to_wire(),
                    effect.to_wire(),
                    0,
                    0,
                    0,
                ]
            }
        };

        Self::set(
            transaction_id,
            0x0f,
            0x02,
            &arguments,
            ResponsePolicy::Required,
        )
    }

    pub(crate) fn set_brightness(
        transaction_id: u8,
        storage: Storage,
        target: LedTarget,
        brightness: u8,
    ) -> Result<Self, ReportError> {
        Self::set(
            transaction_id,
            0x0f,
            0x04,
            &[storage.to_wire(), target.to_wire(), brightness],
            ResponsePolicy::Required,
        )
    }

    #[cfg(test)]
    pub(crate) fn get_brightness(
        transaction_id: u8,
        storage: Storage,
        target: LedTarget,
    ) -> Result<Self, ReportError> {
        Ok(Self {
            report: Report::request(
                transaction_id,
                0x0f,
                CommandId::new(0x04, true)?,
                &[storage.to_wire(), target.to_wire(), 0],
            )?,
            response_policy: ResponsePolicy::Required,
            idempotent: true,
        })
    }

    pub(crate) fn custom_mode(transaction_id: u8) -> Result<Self, ReportError> {
        Self::custom_mode_with_policy(transaction_id, ResponsePolicy::None)
    }

    pub(crate) fn custom_mode_required(transaction_id: u8) -> Result<Self, ReportError> {
        Self::custom_mode_with_policy(transaction_id, ResponsePolicy::Required)
    }

    fn custom_mode_with_policy(
        transaction_id: u8,
        response_policy: ResponsePolicy,
    ) -> Result<Self, ReportError> {
        Self::set(
            transaction_id,
            0x0f,
            0x02,
            &[0, 0, 0x08, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            response_policy,
        )
    }

    pub(crate) fn custom_frame_row(
        transaction_id: u8,
        row: u8,
        start_column: u8,
        colours: &[Rgb],
    ) -> Result<Self, CommandError> {
        Self::custom_frame_row_with_policy(
            transaction_id,
            row,
            start_column,
            colours,
            ResponsePolicy::None,
        )
    }

    pub(crate) fn custom_frame_row_required(
        transaction_id: u8,
        row: u8,
        start_column: u8,
        colours: &[Rgb],
    ) -> Result<Self, CommandError> {
        Self::custom_frame_row_with_policy(
            transaction_id,
            row,
            start_column,
            colours,
            ResponsePolicy::Required,
        )
    }

    fn custom_frame_row_with_policy(
        transaction_id: u8,
        row: u8,
        start_column: u8,
        colours: &[Rgb],
        response_policy: ResponsePolicy,
    ) -> Result<Self, CommandError> {
        if row >= 8 {
            return Err(CommandError::RowOutOfRange(row));
        }
        if colours.is_empty() {
            return Err(CommandError::EmptySpan);
        }
        let colour_count =
            u8::try_from(colours.len()).map_err(|_| CommandError::SpanOutOfRange {
                start_column,
                count: colours.len(),
            })?;
        let stop_column =
            start_column
                .checked_add(colour_count - 1)
                .ok_or(CommandError::SpanOutOfRange {
                    start_column,
                    count: colours.len(),
                })?;
        if stop_column >= 23 {
            return Err(CommandError::SpanOutOfRange {
                start_column,
                count: colours.len(),
            });
        }

        let mut arguments = Vec::with_capacity(5 + colours.len() * 3);
        arguments.extend_from_slice(&[0, 0, row, start_column, stop_column]);
        for colour in colours {
            arguments.extend_from_slice(&[colour.red, colour.green, colour.blue]);
        }
        Ok(Self {
            report: Report::request_with_data_len(
                transaction_id,
                0x0f,
                CommandId::new(0x03, false).map_err(CommandError::Report)?,
                &arguments,
                0x47,
            )
            .map_err(CommandError::Report)?,
            response_policy,
            idempotent: true,
        })
    }

    #[cfg(test)]
    fn get(
        transaction_id: u8,
        command_class: u8,
        command_id: u8,
        response_length: u8,
    ) -> Result<Self, ReportError> {
        Ok(Self {
            report: Report::request(
                transaction_id,
                command_class,
                CommandId::new(command_id, true)?,
                &vec![0; usize::from(response_length)],
            )?,
            response_policy: ResponsePolicy::Required,
            idempotent: true,
        })
    }

    fn set(
        transaction_id: u8,
        command_class: u8,
        command_id: u8,
        arguments: &[u8],
        response_policy: ResponsePolicy,
    ) -> Result<Self, ReportError> {
        Ok(Self {
            report: Report::request(
                transaction_id,
                command_class,
                CommandId::new(command_id, false)?,
                arguments,
            )?,
            response_policy,
            idempotent: true,
        })
    }

    pub(crate) const fn report(&self) -> &Report {
        &self.report
    }

    pub(crate) const fn response_policy(&self) -> ResponsePolicy {
        self.response_policy
    }

    pub(crate) const fn idempotent(&self) -> bool {
        self.idempotent
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommandError {
    Report(ReportError),
    RowOutOfRange(u8),
    EmptySpan,
    SpanOutOfRange { start_column: u8, count: usize },
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Report(error) => error.fmt(formatter),
            Self::RowOutOfRange(row) => write!(formatter, "Razer matrix row {row} exceeds 7"),
            Self::EmptySpan => formatter.write_str("Razer matrix span must contain a colour"),
            Self::SpanOutOfRange {
                start_column,
                count,
            } => write!(
                formatter,
                "Razer matrix span starting at {start_column} with {count} colours exceeds column 22"
            ),
        }
    }
}

impl Error for CommandError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Report(error) => Some(error),
            Self::RowOutOfRange(_) | Self::EmptySpan | Self::SpanOutOfRange { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Storage {
    /// Apply without replacing the state restored after a power cycle.
    Volatile,
    /// `OpenRazer`'s `VARSTORE`; live validation shows that state persists.
    Variable,
}

impl Storage {
    const fn to_wire(self) -> u8 {
        match self {
            Self::Volatile => 0x00,
            Self::Variable => 0x01,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LedTarget {
    Backlight,
}

impl LedTarget {
    const fn to_wire(self) -> u8 {
        match self {
            Self::Backlight => 0x05,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rgb {
    pub(crate) red: u8,
    pub(crate) green: u8,
    pub(crate) blue: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MatrixEffect {
    Off,
    Static(Rgb),
    Spectrum,
    Wave(Direction),
    Wheel(Direction),
    Reactive {
        speed: ReactiveSpeed,
        colour: Rgb,
    },
    Breathing(ColourMode),
    Starlight {
        speed: StarlightSpeed,
        colours: ColourMode,
    },
}

impl MatrixEffect {
    const fn to_wire(self) -> u8 {
        match self {
            Self::Off => 0x00,
            Self::Static(_) => 0x01,
            Self::Spectrum => 0x03,
            Self::Wave(_) => 0x04,
            Self::Wheel(_) => 0x0a,
            Self::Reactive { .. } => 0x05,
            Self::Breathing(_) => 0x02,
            Self::Starlight { .. } => 0x07,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    Forward,
    Reverse,
}

impl Direction {
    const fn to_wire(self) -> u8 {
        match self {
            Self::Forward => 1,
            Self::Reverse => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReactiveSpeed {
    Fast,
    Medium,
    Slow,
    Slowest,
}

impl ReactiveSpeed {
    const fn to_wire(self) -> u8 {
        match self {
            Self::Fast => 1,
            Self::Medium => 2,
            Self::Slow => 3,
            Self::Slowest => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StarlightSpeed {
    Fast,
    Medium,
    Slow,
}

impl StarlightSpeed {
    const fn to_wire(self) -> u8 {
        match self {
            Self::Fast => 1,
            Self::Medium => 2,
            Self::Slow => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColourMode {
    Single(Rgb),
    Dual(Rgb, Rgb),
}

impl ColourMode {
    fn arguments(self, storage: u8, target: u8, effect: u8, speed: Option<u8>) -> Vec<u8> {
        let speed = speed.unwrap_or(0);
        match self {
            Self::Single(colour) => vec![
                storage,
                target,
                effect,
                1,
                speed,
                1,
                colour.red,
                colour.green,
                colour.blue,
            ],
            Self::Dual(first, second) => vec![
                storage,
                target,
                effect,
                2,
                speed,
                2,
                first.red,
                first.green,
                first.blue,
                second.red,
                second.green,
                second.blue,
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructs_identity_queries_from_upstream_fixtures() {
        let serial = Command::serial(0x1f).expect("serial query should be valid");
        let firmware = Command::firmware_version(0x1f).expect("firmware query should be valid");

        assert_eq!(
            &serial.report().encode()[..8],
            &[0, 0x1f, 0, 0, 0, 0x16, 0, 0x82]
        );
        assert_eq!(serial.report().arguments(), &[0; 0x16]);
        assert_eq!(serial.response_policy(), ResponsePolicy::Required);
        assert_eq!(
            &firmware.report().encode()[..8],
            &[0, 0x1f, 0, 0, 0, 2, 0, 0x81]
        );
    }

    #[test]
    fn constructs_extended_matrix_effect_fixtures() {
        let static_effect = Command::extended_matrix_effect(
            0x1f,
            Storage::Variable,
            LedTarget::Backlight,
            MatrixEffect::Static(Rgb {
                red: 0xff,
                green: 0,
                blue: 0,
            }),
        )
        .expect("static command should be valid");
        let spectrum = Command::extended_matrix_effect(
            0x1f,
            Storage::Variable,
            LedTarget::Backlight,
            MatrixEffect::Spectrum,
        )
        .expect("spectrum command should be valid");

        assert_eq!(
            static_effect.report().arguments(),
            &[1, 5, 1, 0, 0, 1, 0xff, 0, 0]
        );
        assert_eq!(spectrum.report().arguments(), &[1, 5, 3, 0, 0, 0]);
    }

    #[test]
    fn distinguishes_volatile_and_variable_storage() {
        let volatile = Command::set_brightness(0x1f, Storage::Volatile, LedTarget::Backlight, 91)
            .expect("volatile brightness command should be valid");
        let variable = Command::set_brightness(0x1f, Storage::Variable, LedTarget::Backlight, 173)
            .expect("variable brightness command should be valid");

        assert_eq!(volatile.report().arguments(), &[0x00, 0x05, 91]);
        assert_eq!(variable.report().arguments(), &[0x01, 0x05, 173]);
    }

    #[test]
    fn constructs_and_bounds_blackwidow_custom_rows() {
        let colours = [Rgb {
            red: 1,
            green: 2,
            blue: 3,
        }; 23];
        let command = Command::custom_frame_row(0x1f, 7, 0, &colours)
            .expect("full BlackWidow row should fit");
        let encoded = command.report().encode();

        assert_eq!(&encoded[..8], &[0, 0x1f, 0, 0, 0, 0x47, 0x0f, 0x03]);
        assert_eq!(&encoded[8..13], &[0, 0, 7, 0, 22]);
        assert_eq!(command.response_policy(), ResponsePolicy::None);
        assert!(matches!(
            Command::custom_frame_row(0x1f, 8, 0, &colours),
            Err(CommandError::RowOutOfRange(8))
        ));
        assert!(matches!(
            Command::custom_frame_row(0x1f, 0, 22, &colours[..2]),
            Err(CommandError::SpanOutOfRange { .. })
        ));
    }

    #[test]
    fn custom_commands_preserve_profile_response_policy() {
        assert_eq!(
            Command::custom_mode(0x1f)
                .expect("no-response custom mode")
                .response_policy(),
            ResponsePolicy::None
        );
        assert_eq!(
            Command::custom_mode_required(0x1f)
                .expect("response-bearing custom mode")
                .response_policy(),
            ResponsePolicy::Required
        );
        assert_eq!(
            Command::custom_frame_row_required(
                0x1f,
                0,
                0,
                &[Rgb {
                    red: 1,
                    green: 2,
                    blue: 3,
                }],
            )
            .expect("response-bearing custom row")
            .response_policy(),
            ResponsePolicy::Required
        );
    }
}
