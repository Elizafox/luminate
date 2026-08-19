// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Correlation and status validation for decoded Razer responses.

use std::error::Error;
use std::fmt;

use super::report::{Report, ReportStatus};

pub(crate) fn validate<'a>(
    request: &Report,
    response: &'a Report,
) -> Result<&'a [u8], ResponseError> {
    if response.transaction_id() != request.transaction_id() {
        return Err(ResponseError::TransactionMismatch);
    }
    if response.command_class() != request.command_class()
        || response.command_id().base() != request.command_id().base()
    {
        return Err(ResponseError::CommandMismatch);
    }
    if response.remaining_packets() != 0 {
        return Err(ResponseError::RemainingPackets(
            response.remaining_packets(),
        ));
    }

    match response.status() {
        ReportStatus::Successful => Ok(response.arguments()),
        ReportStatus::Busy => Err(ResponseError::Busy),
        ReportStatus::Failure => Err(ResponseError::Failure),
        ReportStatus::Timeout => Err(ResponseError::Timeout),
        ReportStatus::NotSupported => Err(ResponseError::NotSupported),
        ReportStatus::NewCommand => Err(ResponseError::InvalidStatus),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResponseError {
    TransactionMismatch,
    CommandMismatch,
    RemainingPackets(u16),
    Busy,
    Failure,
    Timeout,
    NotSupported,
    InvalidStatus,
}

impl fmt::Display for ResponseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TransactionMismatch => {
                formatter.write_str("Razer response transaction does not match request")
            }
            Self::CommandMismatch => {
                formatter.write_str("Razer response command does not match request")
            }
            Self::RemainingPackets(count) => write!(
                formatter,
                "Razer response unexpectedly has {count} packets remaining"
            ),
            Self::Busy => formatter.write_str("Razer device is busy"),
            Self::Failure => formatter.write_str("Razer device reported command failure"),
            Self::Timeout => formatter.write_str("Razer device reported command timeout"),
            Self::NotSupported => formatter.write_str("Razer device does not support the command"),
            Self::InvalidStatus => formatter.write_str("Razer response retained request status"),
        }
    }
}

impl Error for ResponseError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::report::{CommandId, REPORT_LEN};

    fn response(status: u8, transaction: u8, class: u8, id: u8) -> Report {
        let request = Report::request(
            transaction,
            class,
            CommandId::new(id, true).expect("valid ID"),
            &[1, 2],
        )
        .expect("valid report");
        let mut bytes = request.encode();
        bytes[0] = status;
        bytes[88] = bytes[2..88].iter().fold(0, |crc, byte| crc ^ byte);
        Report::decode(&bytes[..REPORT_LEN]).expect("valid response")
    }

    #[test]
    fn accepts_only_successful_correlated_terminal_response() {
        let request = Report::request(0x1f, 0, CommandId::new(2, true).expect("valid ID"), &[0; 2])
            .expect("valid report");
        let successful = response(2, 0x1f, 0, 2);
        assert_eq!(validate(&request, &successful), Ok(&[1, 2][..]));

        assert_eq!(
            validate(&request, &response(1, 0x1f, 0, 2)),
            Err(ResponseError::Busy)
        );
        assert_eq!(
            validate(&request, &response(2, 0x3f, 0, 2)),
            Err(ResponseError::TransactionMismatch)
        );
        assert_eq!(
            validate(&request, &response(2, 0x1f, 3, 2)),
            Err(ResponseError::CommandMismatch)
        );
    }
}
