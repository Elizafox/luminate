// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford
// SPDX-FileCopyrightText: 2015 Tim Theede <pez2001@voyagerproject.de>
// SPDX-FileCopyrightText: 2015 Terri Cain <terri@dolphincorp.co.uk>

//! Validated representation of the common Razer 90-byte HID report.
//!
//! The wire layout and checksum are derived from `OpenRazer`'s `razercommon.h`
//! and `razercommon.c` at revision
//! `6820f9da169d354bc7e6e93a0aa8683a6bb75792`. The Rust types and validation
//! boundaries are Luminate-specific.

use std::error::Error;
use std::fmt;

pub(crate) const REPORT_LEN: usize = 90;
pub(crate) const ARGUMENT_CAPACITY: usize = 80;

const STATUS_OFFSET: usize = 0;
const TRANSACTION_ID_OFFSET: usize = 1;
const REMAINING_PACKETS_OFFSET: usize = 2;
const PROTOCOL_TYPE_OFFSET: usize = 4;
const DATA_LEN_OFFSET: usize = 5;
const COMMAND_CLASS_OFFSET: usize = 6;
const COMMAND_ID_OFFSET: usize = 7;
const ARGUMENTS_OFFSET: usize = 8;
const CRC_OFFSET: usize = 88;
const RESERVED_OFFSET: usize = 89;
const CRC_START: usize = REMAINING_PACKETS_OFFSET;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReportStatus {
    NewCommand,
    Busy,
    Successful,
    Failure,
    Timeout,
    NotSupported,
}

impl ReportStatus {
    const fn to_wire(self) -> u8 {
        match self {
            Self::NewCommand => 0x00,
            Self::Busy => 0x01,
            Self::Successful => 0x02,
            Self::Failure => 0x03,
            Self::Timeout => 0x04,
            Self::NotSupported => 0x05,
        }
    }

    const fn from_wire(value: u8) -> Result<Self, ReportError> {
        match value {
            0x00 => Ok(Self::NewCommand),
            0x01 => Ok(Self::Busy),
            0x02 => Ok(Self::Successful),
            0x03 => Ok(Self::Failure),
            0x04 => Ok(Self::Timeout),
            0x05 => Ok(Self::NotSupported),
            value => Err(ReportError::UnknownStatus(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommandId(u8);

impl CommandId {
    pub(crate) const fn new(id: u8, response: bool) -> Result<Self, ReportError> {
        if id > 0x7f {
            return Err(ReportError::CommandIdOutOfRange(id));
        }

        Ok(Self(id | if response { 0x80 } else { 0x00 }))
    }

    const fn from_wire(value: u8) -> Self {
        Self(value)
    }

    const fn to_wire(self) -> u8 {
        self.0
    }

    pub(crate) const fn base(self) -> u8 {
        self.0 & 0x7f
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Report {
    status: ReportStatus,
    transaction_id: u8,
    remaining_packets: u16,
    command_class: u8,
    command_id: CommandId,
    arguments: [u8; ARGUMENT_CAPACITY],
    data_len: u8,
}

impl Report {
    pub(crate) fn request(
        transaction_id: u8,
        command_class: u8,
        command_id: CommandId,
        arguments: &[u8],
    ) -> Result<Self, ReportError> {
        let data_len = u8::try_from(arguments.len())
            .map_err(|_| ReportError::ArgumentsTooLong(arguments.len()))?;
        Self::request_with_data_len(
            transaction_id,
            command_class,
            command_id,
            arguments,
            data_len,
        )
    }

    pub(crate) fn request_with_data_len(
        transaction_id: u8,
        command_class: u8,
        command_id: CommandId,
        arguments: &[u8],
        data_len: u8,
    ) -> Result<Self, ReportError> {
        if arguments.len() > ARGUMENT_CAPACITY {
            return Err(ReportError::ArgumentsTooLong(arguments.len()));
        }
        if usize::from(data_len) > ARGUMENT_CAPACITY {
            return Err(ReportError::ArgumentsTooLong(usize::from(data_len)));
        }

        let mut argument_buffer = [0; ARGUMENT_CAPACITY];
        let destination = argument_buffer
            .get_mut(..arguments.len())
            .ok_or(ReportError::ArgumentsTooLong(arguments.len()))?;
        destination.copy_from_slice(arguments);

        Ok(Self {
            status: ReportStatus::NewCommand,
            transaction_id,
            remaining_packets: 0,
            command_class,
            command_id,
            arguments: argument_buffer,
            data_len,
        })
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, ReportError> {
        let bytes: &[u8; REPORT_LEN] = bytes
            .try_into()
            .map_err(|_| ReportError::InvalidLength(bytes.len()))?;
        if bytes[PROTOCOL_TYPE_OFFSET] != 0 {
            return Err(ReportError::UnsupportedProtocol(
                bytes[PROTOCOL_TYPE_OFFSET],
            ));
        }
        if bytes[RESERVED_OFFSET] != 0 {
            return Err(ReportError::NonZeroReserved(bytes[RESERVED_OFFSET]));
        }

        let data_len = bytes[DATA_LEN_OFFSET];
        if usize::from(data_len) > ARGUMENT_CAPACITY {
            return Err(ReportError::ArgumentsTooLong(usize::from(data_len)));
        }

        let expected_crc = calculate_crc(bytes);
        if bytes[CRC_OFFSET] != expected_crc {
            return Err(ReportError::CrcMismatch {
                expected: expected_crc,
                actual: bytes[CRC_OFFSET],
            });
        }

        let mut arguments = [0; ARGUMENT_CAPACITY];
        arguments.copy_from_slice(
            bytes
                .get(ARGUMENTS_OFFSET..CRC_OFFSET)
                .ok_or(ReportError::InvalidLength(bytes.len()))?,
        );

        Ok(Self {
            status: ReportStatus::from_wire(bytes[STATUS_OFFSET])?,
            transaction_id: bytes[TRANSACTION_ID_OFFSET],
            remaining_packets: u16::from_be_bytes([
                bytes[REMAINING_PACKETS_OFFSET],
                bytes[REMAINING_PACKETS_OFFSET + 1],
            ]),
            command_class: bytes[COMMAND_CLASS_OFFSET],
            command_id: CommandId::from_wire(bytes[COMMAND_ID_OFFSET]),
            arguments,
            data_len,
        })
    }

    pub(crate) fn encode(&self) -> [u8; REPORT_LEN] {
        let mut bytes = [0; REPORT_LEN];
        bytes[STATUS_OFFSET] = self.status.to_wire();
        bytes[TRANSACTION_ID_OFFSET] = self.transaction_id;
        bytes[REMAINING_PACKETS_OFFSET..PROTOCOL_TYPE_OFFSET]
            .copy_from_slice(&self.remaining_packets.to_be_bytes());
        bytes[DATA_LEN_OFFSET] = self.data_len;
        bytes[COMMAND_CLASS_OFFSET] = self.command_class;
        bytes[COMMAND_ID_OFFSET] = self.command_id.to_wire();
        bytes[ARGUMENTS_OFFSET..CRC_OFFSET].copy_from_slice(&self.arguments);
        bytes[CRC_OFFSET] = calculate_crc(&bytes);
        bytes
    }

    pub(crate) fn arguments(&self) -> &[u8] {
        self.arguments
            .get(..usize::from(self.data_len))
            .unwrap_or_default()
    }

    pub(crate) const fn status(&self) -> ReportStatus {
        self.status
    }

    pub(crate) const fn transaction_id(&self) -> u8 {
        self.transaction_id
    }

    pub(crate) const fn remaining_packets(&self) -> u16 {
        self.remaining_packets
    }

    pub(crate) const fn command_class(&self) -> u8 {
        self.command_class
    }

    pub(crate) const fn command_id(&self) -> CommandId {
        self.command_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReportError {
    InvalidLength(usize),
    ArgumentsTooLong(usize),
    CommandIdOutOfRange(u8),
    UnknownStatus(u8),
    UnsupportedProtocol(u8),
    NonZeroReserved(u8),
    CrcMismatch { expected: u8, actual: u8 },
}

impl fmt::Display for ReportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength(length) => {
                write!(
                    formatter,
                    "Razer report length {length} is not {REPORT_LEN}"
                )
            }
            Self::ArgumentsTooLong(length) => write!(
                formatter,
                "Razer report argument length {length} exceeds {ARGUMENT_CAPACITY}"
            ),
            Self::CommandIdOutOfRange(id) => {
                write!(formatter, "Razer command ID {id:#04x} exceeds 7 bits")
            }
            Self::UnknownStatus(status) => {
                write!(formatter, "unknown Razer report status {status:#04x}")
            }
            Self::UnsupportedProtocol(protocol) => {
                write!(formatter, "unsupported Razer protocol type {protocol:#04x}")
            }
            Self::NonZeroReserved(value) => {
                write!(formatter, "Razer report reserved byte is {value:#04x}")
            }
            Self::CrcMismatch { expected, actual } => write!(
                formatter,
                "Razer report CRC is {actual:#04x}, expected {expected:#04x}"
            ),
        }
    }
}

impl Error for ReportError {}

fn calculate_crc(bytes: &[u8; REPORT_LEN]) -> u8 {
    bytes[CRC_START..CRC_OFFSET]
        .iter()
        .fold(0, |crc, byte| crc ^ byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_common_report_layout_and_crc() {
        let command_id = CommandId::new(0x03, false).expect("command ID should be valid");
        let report = Report::request(0x1f, 0x0f, command_id, &[0x01, 0x02, 0x03])
            .expect("arguments should fit");

        let bytes = report.encode();

        assert_eq!(bytes.len(), REPORT_LEN);
        assert_eq!(&bytes[..11], &[0x00, 0x1f, 0, 0, 0, 3, 0x0f, 3, 1, 2, 3]);
        assert!(bytes[11..CRC_OFFSET].iter().all(|byte| *byte == 0));
        assert_eq!(bytes[CRC_OFFSET], 0x0f);
        assert_eq!(bytes[RESERVED_OFFSET], 0);
    }

    #[test]
    fn round_trips_maximum_argument_payload() {
        let arguments = [0xa5; ARGUMENT_CAPACITY];
        let report = Report::request(
            0x3f,
            0x0f,
            CommandId::new(0x02, true).expect("command ID should be valid"),
            &arguments,
        )
        .expect("maximum-sized arguments should fit");

        let decoded = Report::decode(&report.encode()).expect("encoded report should decode");

        assert_eq!(decoded, report);
        assert_eq!(decoded.arguments(), arguments);
    }

    #[test]
    fn rejects_oversized_arguments() {
        let arguments = [0; ARGUMENT_CAPACITY + 1];
        let error = Report::request(
            1,
            2,
            CommandId::new(3, false).expect("command ID should be valid"),
            &arguments,
        )
        .expect_err("oversized arguments must be rejected");

        assert_eq!(error, ReportError::ArgumentsTooLong(81));
    }

    #[test]
    fn rejects_invalid_envelope_fields() {
        let report = Report::request(
            1,
            2,
            CommandId::new(3, false).expect("command ID should be valid"),
            &[],
        )
        .expect("empty arguments should fit");
        let valid = report.encode();

        let mut bad_crc = valid;
        bad_crc[CRC_OFFSET] ^= 1;
        assert!(matches!(
            Report::decode(&bad_crc),
            Err(ReportError::CrcMismatch { .. })
        ));

        let mut bad_protocol = valid;
        bad_protocol[PROTOCOL_TYPE_OFFSET] = 1;
        assert_eq!(
            Report::decode(&bad_protocol),
            Err(ReportError::UnsupportedProtocol(1))
        );

        let mut bad_reserved = valid;
        bad_reserved[RESERVED_OFFSET] = 1;
        assert_eq!(
            Report::decode(&bad_reserved),
            Err(ReportError::NonZeroReserved(1))
        );
    }

    #[test]
    fn rejects_invalid_command_id_and_report_length() {
        assert_eq!(
            CommandId::new(0x80, false),
            Err(ReportError::CommandIdOutOfRange(0x80))
        );
        assert_eq!(
            Report::decode(&[0; REPORT_LEN - 1]),
            Err(ReportError::InvalidLength(REPORT_LEN - 1))
        );
    }
}
